// Compila os runners (sort + rastreador) e executa, lendo a saída.

use crate::detectar::{Assinatura, Lang};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const VT_TRACE_C: &str = include_str!("../runtime/vt_trace.c");
const RUNNER_C: &str = include_str!("../runtime/runner_c.c");
const RUNNER_RS: &str = include_str!("../runtime/runner_rs.rs");

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Modo {
    /// sem otimização e com rastreamento: para a animação
    Rastro,
    /// otimizado (-O2 / opt-level=3): para medir tempo
    Tempo,
}

#[derive(Clone, Copy, Debug)]
pub struct Evento {
    pub escrita: bool,
    pub idx: i64,
    pub val: i64,
}

#[derive(Clone, Debug, Default)]
pub struct Rastro {
    pub inicial: Vec<i64>,
    pub eventos: Vec<Evento>,
    pub final_: Vec<i64>,
    pub ordenado: bool,
    pub truncado: bool,
    pub problema: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Copy, Debug)]
pub struct Medida {
    pub n: usize,
    pub mediana_ns: i64,
    pub ok: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Medicao {
    pub pontos: Vec<Medida>,
    pub problema: Option<String>,
}

pub struct Compilador {
    dir: PathBuf,
}

fn rodar(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("não consegui executar {:?}: {e}", cmd.get_program()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

impl Compilador {
    /// Prepara `target/vt/` com o rastreador já compilado.
    pub fn novo(dir: PathBuf) -> Result<Self, String> {
        let rt = dir.join("runtime");
        std::fs::create_dir_all(&rt).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(dir.join("bin")).map_err(|e| e.to_string())?;
        let escrever = |nome: &str, conteudo: &str| -> Result<bool, String> {
            let p = rt.join(nome);
            if std::fs::read_to_string(&p).ok().as_deref() == Some(conteudo) {
                return Ok(false);
            }
            std::fs::write(&p, conteudo).map_err(|e| e.to_string())?;
            Ok(true)
        };
        let mudou_vt = escrever("vt_trace.c", VT_TRACE_C)?;
        let mudou_runner = escrever("runner_c.c", RUNNER_C)?;
        escrever("runner_rs.rs", RUNNER_RS)?;
        if mudou_vt || !rt.join("libvt.a").exists() {
            rodar(Command::new("gcc").args(["-O2", "-c", "vt_trace.c", "-o", "vt.o"]).current_dir(&rt))?;
            let _ = std::fs::remove_file(rt.join("libvt.a"));
            rodar(Command::new("ar").args(["rcs", "libvt.a", "vt.o"]).current_dir(&rt))?;
        }
        if mudou_runner || !rt.join("runner_c.o").exists() {
            rodar(Command::new("gcc").args(["-O2", "-c", "runner_c.c", "-o", "runner_c.o"]).current_dir(&rt))?;
        }
        Ok(Self { dir })
    }

    /// Compila (ou reaproveita do cache) o runner de um sort.
    pub fn compilar(&self, lang: Lang, arquivo: &Path, a: &Assinatura, fim_inclusivo: bool, modo: Modo) -> Result<PathBuf, String> {
        let fonte = std::fs::read_to_string(arquivo).map_err(|e| e.to_string())?;
        let chamada = a.chamada(lang, fim_inclusivo);
        let mut h = DefaultHasher::new();
        (lang, &fonte, &chamada, &a.tipo_elem, modo, VT_TRACE_C, RUNNER_C, RUNNER_RS).hash(&mut h);
        let stem = arquivo.file_stem().unwrap_or_default().to_string_lossy();
        let bin = self.dir.join("bin").join(format!("{}-{}-{:?}-{:016x}", lang.nome(), stem, modo, h.finish()));
        if bin.exists() {
            return Ok(bin);
        }
        let rt = self.dir.join("runtime");
        let abs = arquivo.canonicalize().map_err(|e| e.to_string())?;
        let tmp = self.dir.join("bin").join(format!("{}.{}", bin.file_name().unwrap().to_string_lossy(), lang.ext()));
        match lang {
            Lang::C => {
                let entry = format!(
                    "#define main vt_user_main\n#include \"{}\"\n#undef main\n\
                     typedef {} vt_elem;\n\
                     const int vt_elem_size = sizeof(vt_elem);\n\
                     const int vt_elem_signed = ((vt_elem)-1) < 0;\n\
                     void vt_entry(void *v, long n) {{ {}; }}\n",
                    abs.display(),
                    a.tipo_elem,
                    chamada
                );
                std::fs::write(&tmp, entry).map_err(|e| e.to_string())?;
                let opt = if modo == Modo::Rastro { "-O0" } else { "-O2" };
                let mut cmd = Command::new("gcc");
                cmd.args(["-std=gnu17", opt, "-g", "-w"]);
                for h in ["stdio.h", "stdlib.h", "string.h", "stdbool.h", "limits.h", "math.h"] {
                    cmd.args(["-include", h]);
                }
                cmd.arg(&tmp).arg(rt.join("runner_c.o")).arg(rt.join("vt.o")).arg("-lm").arg("-o").arg(&bin);
                rodar(&mut cmd).map_err(|e| limpar_caminhos(&e, &tmp))?;
            }
            Lang::Rust => {
                let src = RUNNER_RS
                    .replace("__VT_ELEM__", &a.tipo_elem)
                    .replace("__VT_SORT_PATH__", &abs.display().to_string())
                    .replace("__VT_CALL__", &chamada);
                std::fs::write(&tmp, src).map_err(|e| e.to_string())?;
                let opt = if modo == Modo::Rastro { "opt-level=0" } else { "opt-level=3" };
                let mut cmd = Command::new("rustc");
                cmd.args(["--edition", "2024", "-C", opt, "-A", "warnings"])
                    .arg(&tmp)
                    .arg("-L")
                    .arg(&rt)
                    .args(["-l", "static=vt", "-o"])
                    .arg(&bin);
                rodar(&mut cmd).map_err(|e| limpar_caminhos(&e, &tmp))?;
            }
        }
        Ok(bin)
    }
}

/// O erro do compilador cita o arquivo gerado; troca pelo nome do runner.
fn limpar_caminhos(err: &str, tmp: &Path) -> String {
    err.replace(&tmp.display().to_string(), "<runner gerado>")
}

/// Executa o binário com um teto de tempo externo (o runner já tem o seu, via alarm).
fn executar(bin: &Path, args: &[String], teto: Duration) -> Result<(String, String, Option<i32>), String> {
    let mut filho = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut out = filho.stdout.take().unwrap();
    let mut err = filho.stderr.take().unwrap();
    let t_out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let ini = Instant::now();
    let status = loop {
        if let Some(st) = filho.try_wait().map_err(|e| e.to_string())? {
            break Some(st);
        }
        if ini.elapsed() > teto {
            let _ = filho.kill();
            let _ = filho.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = t_out.join().unwrap_or_default();
    let stderr = t_err.join().unwrap_or_default();
    Ok((stdout, stderr, status.and_then(|s| s.code())))
}

fn problema_da_saida(linha: &str) -> Option<String> {
    if let Some(r) = linha.strip_prefix("CRASH ") {
        return Some(if r == "TIMEOUT" {
            "passou do tempo limite (loop infinito?)".to_string()
        } else {
            format!("o programa quebrou: {r}")
        });
    }
    linha.strip_prefix("PANIC ").map(|r| format!("panic: {r}"))
}

pub fn rastrear(bin: &Path, n: usize, seed: u32, padrao: usize, timeout_s: u32) -> Result<Rastro, String> {
    let args = ["trace".to_string(), n.to_string(), seed.to_string(), padrao.to_string(), timeout_s.to_string()];
    let (out, stderr, code) = executar(bin, &args, Duration::from_secs(timeout_s as u64 + 10))?;
    let mut r = Rastro { stderr, ..Default::default() };
    let mut fim = false;
    let nums = |s: &str| s.split_whitespace().filter_map(|x| x.parse().ok()).collect::<Vec<i64>>();
    for linha in out.lines() {
        let b = linha.as_bytes();
        if b.len() > 2 && (b[0] == b'R' || b[0] == b'W') && b[1] == b' ' {
            let mut it = linha[2..].split(' ');
            let idx = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
            let val = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
            r.eventos.push(Evento { escrita: b[0] == b'W', idx, val });
        } else if let Some(x) = linha.strip_prefix("INIT") {
            r.inicial = nums(x);
        } else if let Some(x) = linha.strip_prefix("FINAL") {
            r.final_ = nums(x);
        } else if linha == "SORTED 1" {
            r.ordenado = true;
        } else if linha == "TRUNC" {
            r.truncado = true;
        } else if linha == "END" {
            fim = true;
        } else if let Some(p) = problema_da_saida(linha) {
            r.problema = Some(p);
        }
    }
    if r.inicial.len() != n {
        // nem chegou a imprimir o vetor inicial: algo deu muito errado
        return Err(format!("o runner não respondeu (código {:?})\n{}", code, r.stderr));
    }
    if !fim && r.problema.is_none() {
        r.problema = Some(match code {
            None => "passou do tempo limite (loop infinito?)".into(),
            Some(c) => format!("o programa terminou sem concluir (código {c})"),
        });
    }
    if r.final_.is_empty() {
        // quebrou no meio: o estado final é o que os eventos deixaram
        let mut v = r.inicial.clone();
        for e in &r.eventos {
            if e.escrita && e.idx >= 0 && (e.idx as usize) < v.len() {
                v[e.idx as usize] = e.val;
            }
        }
        r.final_ = v;
    }
    Ok(r)
}

pub fn medir(bin: &Path, tamanhos: &[usize], seed: u32, padrao: usize, reps: usize, timeout_s: u32) -> Result<Medicao, String> {
    let mut args = vec!["bench".to_string(), seed.to_string(), padrao.to_string(), reps.to_string(), timeout_s.to_string()];
    args.extend(tamanhos.iter().map(|t| t.to_string()));
    let (out, stderr, code) = executar(bin, &args, Duration::from_secs(timeout_s as u64 + 10))?;
    let mut m = Medicao::default();
    let mut fim = false;
    for linha in out.lines() {
        if let Some(x) = linha.strip_prefix("BENCH ") {
            let v: Vec<i64> = x.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            if v.len() == 4 {
                m.pontos.push(Medida { n: v[0] as usize, mediana_ns: v[1], ok: v[3] == 1 });
            }
        } else if linha == "END" {
            fim = true;
        } else if let Some(p) = problema_da_saida(linha) {
            m.problema = Some(p);
        }
    }
    if !fim && m.problema.is_none() {
        m.problema = Some(match code {
            None => "passou do tempo limite".into(),
            Some(c) => format!("terminou sem concluir (código {c}) {}", stderr.trim()),
        });
    }
    Ok(m)
}
