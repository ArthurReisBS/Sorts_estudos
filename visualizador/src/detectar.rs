// Lê um arquivo de sort (C ou Rust) e descobre qual função chamar e como.
// Nada aqui altera o arquivo: só leitura.

use regex::Regex;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Lang {
    C,
    Rust,
}

impl Lang {
    pub fn nome(self) -> &'static str {
        match self {
            Lang::C => "C",
            Lang::Rust => "Rust",
        }
    }
    pub fn pasta(self) -> &'static str {
        match self {
            Lang::C => "C",
            Lang::Rust => "Rust",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Lang::C => "c",
            Lang::Rust => "rs",
        }
    }
}

/// Como o vetor é passado: `(v, n)` ou `(v, inicio, fim)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Forma {
    SoVetor,
    VetorTamanho,
    VetorIntervalo,
    /// Rust: `fn f(v: &[T]) -> Vec<T>` (devolve um vetor novo)
    DevolveNovo,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Assinatura {
    pub funcao: String,
    pub tipo_elem: String,
    pub forma: Forma,
    /// a linha da assinatura como aparece no arquivo, para mostrar na tela
    pub texto: String,
}

#[derive(Clone, Debug)]
pub enum Deteccao {
    NaoExiste,
    Vazio,
    Ok(Assinatura),
    Erro(String),
}

impl Assinatura {
    /// Chamada usada dentro do runner. `fim_inclusivo` só vale para (v, inicio, fim).
    pub fn chamada(&self, lang: Lang, fim_inclusivo: bool) -> String {
        let f = &self.funcao;
        let fim = if fim_inclusivo { "n - 1" } else { "n" };
        match (lang, &self.forma) {
            (Lang::C, Forma::VetorTamanho) => format!("{f}(v, n)"),
            (Lang::C, Forma::VetorIntervalo) => format!("{f}(v, 0, {fim})"),
            (Lang::C, _) => format!("{f}(v)"),
            (Lang::Rust, Forma::SoVetor) => format!("{f}(v)"),
            (Lang::Rust, Forma::VetorTamanho) => format!("{f}(v, n as _)"),
            (Lang::Rust, Forma::VetorIntervalo) => format!("{f}(v, 0 as _, ({fim}) as _)"),
            (Lang::Rust, Forma::DevolveNovo) => format!("{{ let r = {f}(v); *v = r.into_iter().collect(); }}"),
        }
    }
}

pub fn normalizar(nome: &str) -> String {
    nome.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Tira comentários `//` e `/* */`, respeitando strings e chars.
fn sem_comentarios(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == '/' && i + 1 < b.len() && b[i + 1] == '/' {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && i + 1 < b.len() && b[i + 1] == '*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                if b[i] == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i += 2;
        } else if c == '"' {
            out.push(c);
            i += 1;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\\' {
                    out.push(b[i]);
                    i += 1;
                }
                if i < b.len() {
                    out.push(b[i]);
                }
                i += 1;
            }
            if i < b.len() {
                out.push('"');
            }
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Separa parâmetros por vírgula, ignorando vírgulas dentro de <>, [] e ().
fn separar_params(s: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut prof = 0i32;
    let mut atual = String::new();
    for c in s.chars() {
        match c {
            '<' | '[' | '(' => prof += 1,
            '>' | ']' | ')' => prof -= 1,
            ',' if prof == 0 => {
                v.push(atual.trim().to_string());
                atual.clear();
                continue;
            }
            _ => {}
        }
        atual.push(c);
    }
    if !atual.trim().is_empty() {
        v.push(atual.trim().to_string());
    }
    v
}

struct Funcao {
    nome: String,
    params: Vec<String>,
    retorno: String,
    genericos: Vec<String>,
    /// onde a definição começa e onde o corpo começa (logo depois do `{`)
    def_ini: usize,
    corpo_ini: usize,
    texto: String,
}

/// Escolhe a função de entrada: a que tem o nome do arquivo; senão, entre as
/// candidatas, a única que nenhuma outra função chama.
fn escolher<'a>(funcs: &'a [Funcao], cands: Vec<&'a Funcao>, src: &str, nome_arquivo: &str) -> Option<&'a Funcao> {
    let alvo = normalizar(nome_arquivo);
    if let Some(f) = cands.iter().find(|f| normalizar(&f.nome) == alvo) {
        return Some(f);
    }
    if cands.len() == 1 {
        return Some(cands[0]);
    }
    let raiz: Vec<&&Funcao> = cands
        .iter()
        .filter(|f| {
            let chamada = Regex::new(&format!(r"\b{}\s*\(", regex::escape(&f.nome))).unwrap();
            !funcs.iter().filter(|g| g.nome != f.nome).any(|g| {
                let fim = funcs
                    .iter()
                    .map(|h| h.def_ini)
                    .filter(|&p| p > g.corpo_ini)
                    .min()
                    .unwrap_or(src.len());
                chamada.is_match(&src[g.corpo_ini..fim])
            })
        })
        .collect();
    if raiz.len() == 1 {
        Some(raiz[0])
    } else {
        None
    }
}

pub fn detectar(lang: Lang, src_original: &str, nome_arquivo: &str) -> Deteccao {
    let src = sem_comentarios(src_original);
    if src.trim().is_empty() {
        return Deteccao::Vazio;
    }
    match lang {
        Lang::C => detectar_c(&src, nome_arquivo),
        Lang::Rust => detectar_rust(&src, nome_arquivo),
    }
}

fn detectar_c(src: &str, nome_arquivo: &str) -> Deteccao {
    let re = Regex::new(r"(?m)^[ \t]*((?:static\s+|inline\s+)*)([A-Za-z_][\w \t\*]*?)\s*\b([A-Za-z_]\w*)\s*\(([^()]*)\)\s*\{").unwrap();
    let funcs: Vec<Funcao> = re
        .captures_iter(src)
        .filter(|c| !matches!(&c[3], "if" | "while" | "for" | "switch" | "main"))
        .map(|c| Funcao {
            nome: c[3].to_string(),
            params: separar_params(&c[4]),
            retorno: c[2].trim().to_string(),
            genericos: vec![],
            def_ini: c.get(0).unwrap().start(),
            corpo_ini: c.get(0).unwrap().end(),
            texto: c.get(0).unwrap().as_str().trim().trim_end_matches('{').trim().to_string(),
        })
        .collect();
    if funcs.is_empty() {
        return Deteccao::Erro("nenhuma função encontrada no arquivo".into());
    }
    let ponteiro = |p: &String| p.contains('*') || p.contains('[');
    let cands: Vec<&Funcao> = funcs
        .iter()
        .filter(|f| f.retorno == "void" && (2..=3).contains(&f.params.len()) && ponteiro(&f.params[0]))
        .collect();
    let Some(f) = escolher(&funcs, cands, src, nome_arquivo) else {
        return Deteccao::Erro(format!(
            "não achei a função de entrada. Esperado: `void {}(int *V, int N)` ou `(int *V, int inicio, int fim)`",
            nome_arquivo
        ));
    };
    // tipo do elemento: o primeiro parâmetro sem nome, `*`, `[]` e `const`
    let p0 = f.params[0].replace('*', " ").replace("[]", " ");
    let mut toks: Vec<&str> = p0.split_whitespace().filter(|t| *t != "const").collect();
    toks.pop(); // nome do parâmetro
    if toks.is_empty() {
        return Deteccao::Erro(format!("não entendi o tipo do vetor em `{}`", f.params[0]));
    }
    let tipo = toks.join(" ");
    if tipo.contains("float") || tipo.contains("double") || tipo.contains("char") && !tipo.contains("signed") {
        return Deteccao::Erro(format!("o visualizador só trabalha com vetores de inteiros (achei `{tipo}`)"));
    }
    Deteccao::Ok(Assinatura {
        funcao: f.nome.clone(),
        tipo_elem: tipo,
        forma: if f.params.len() == 2 { Forma::VetorTamanho } else { Forma::VetorIntervalo },
        texto: f.texto.clone(),
    })
}

const INTS_RUST: [&str; 12] = ["i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize"];

fn detectar_rust(src: &str, nome_arquivo: &str) -> Deteccao {
    let re = Regex::new(r"fn\s+([A-Za-z_]\w*)\s*(<[^{(]*>)?\s*\(([^)]*)\)\s*(->\s*[^{]+)?\{").unwrap();
    let funcs: Vec<Funcao> = re
        .captures_iter(src)
        .filter(|c| &c[1] != "main")
        .map(|c| {
            let genericos = c
                .get(2)
                .map(|g| {
                    separar_params(g.as_str().trim_start_matches('<').trim_end_matches('>'))
                        .iter()
                        .map(|p| p.split(':').next().unwrap_or("").trim().to_string())
                        .collect()
                })
                .unwrap_or_default();
            Funcao {
                nome: c[1].to_string(),
                params: separar_params(&c[3]),
                retorno: c.get(4).map(|r| r.as_str().trim_start_matches("->").trim().to_string()).unwrap_or_default(),
                genericos,
                def_ini: c.get(0).unwrap().start(),
                corpo_ini: c.get(0).unwrap().end(),
                texto: c.get(0).unwrap().as_str().trim().trim_end_matches('{').trim().to_string(),
            }
        })
        .collect();
    if funcs.is_empty() {
        return Deteccao::Erro("nenhuma função encontrada no arquivo".into());
    }
    let tipo_de = |p: &str| p.splitn(2, ':').nth(1).map(|t| t.trim().to_string()).unwrap_or_default();
    // extrai T de `&mut [T]`, `&mut Vec<T>` ou `&[T]`
    let elem = |t: &str| -> Option<(String, bool)> {
        let t = t.replace(' ', "");
        for (pre, suf, muta) in [("&mut[", "]", true), ("&mutVec<", ">", true), ("&[", "]", false), ("&Vec<", ">", false)] {
            if let Some(r) = t.strip_prefix(pre).and_then(|r| r.strip_suffix(suf)) {
                return Some((r.to_string(), muta));
            }
        }
        None
    };
    let cands: Vec<&Funcao> = funcs
        .iter()
        .filter(|f| (1..=3).contains(&f.params.len()) && elem(&tipo_de(&f.params[0])).is_some())
        .collect();
    let Some(f) = escolher(&funcs, cands, src, nome_arquivo) else {
        return Deteccao::Erro(format!(
            "não achei a função de entrada. Esperado algo como `fn {}(v: &mut [i32])`",
            nome_arquivo
        ));
    };
    let (t, muta) = elem(&tipo_de(&f.params[0])).unwrap();
    let tipo = if INTS_RUST.contains(&t.as_str()) {
        t
    } else if f.genericos.contains(&t) {
        "i32".to_string()
    } else {
        return Deteccao::Erro(format!("o visualizador só trabalha com vetores de inteiros (achei `{t}`)"));
    };
    let forma = if !muta {
        if f.params.len() == 1 && f.retorno.starts_with("Vec") {
            Forma::DevolveNovo
        } else {
            return Deteccao::Erro("a função recebe o vetor sem `&mut` e não devolve um `Vec` novo".into());
        }
    } else {
        match f.params.len() {
            1 => Forma::SoVetor,
            2 => Forma::VetorTamanho,
            _ => Forma::VetorIntervalo,
        }
    };
    Deteccao::Ok(Assinatura { funcao: f.nome.clone(), tipo_elem: tipo, forma, texto: f.texto.clone() })
}

#[cfg(test)]
mod testes {
    use super::*;

    fn ok(d: Deteccao) -> Assinatura {
        match d {
            Deteccao::Ok(a) => a,
            outro => panic!("{outro:?}"),
        }
    }

    #[test]
    fn c_bubble() {
        let a = ok(detectar(Lang::C, "void bubbleSort(int *V, int N) {\n}\n", "BubbleSort"));
        assert_eq!(a.funcao, "bubbleSort");
        assert_eq!(a.tipo_elem, "int");
        assert_eq!(a.forma, Forma::VetorTamanho);
    }

    #[test]
    fn c_merge_com_auxiliar() {
        let src = "void merge(int *V, int ini, int meio, int fim) { }\nvoid ordena(int V[], int ini, int fim) { merge(V, ini, 0, fim); ordena(V, ini, fim); }\n";
        let a = ok(detectar(Lang::C, src, "MergeSort"));
        assert_eq!(a.funcao, "ordena");
        assert_eq!(a.forma, Forma::VetorIntervalo);
    }

    #[test]
    fn c_auxiliar_antes_da_principal() {
        let src = "void troca(int *a, int *b){ }\nvoid ordena(int V[], int ini, int fim){ troca(&V[0], &V[1]); }\n";
        assert_eq!(ok(detectar(Lang::C, src, "Intervalo")).funcao, "ordena");
    }

    #[test]
    fn c_vazio() {
        assert!(matches!(detectar(Lang::C, "  \n// nada\n", "X"), Deteccao::Vazio));
    }

    #[test]
    fn rust_formas() {
        let a = ok(detectar(Lang::Rust, "pub fn bubble_sort(v: &mut [i32]) { }", "BubbleSort"));
        assert_eq!((a.funcao.as_str(), a.forma.clone()), ("bubble_sort", Forma::SoVetor));
        let a = ok(detectar(Lang::Rust, "fn quick<T: Ord>(v: &mut Vec<T>, lo: usize, hi: usize) { }", "QuickSort"));
        assert_eq!((a.tipo_elem.as_str(), a.forma.clone()), ("i32", Forma::VetorIntervalo));
        let a = ok(detectar(Lang::Rust, "fn counting_sort(v: &[u32]) -> Vec<u32> { vec![] }", "CountingSort"));
        assert_eq!((a.tipo_elem.as_str(), a.forma.clone()), ("u32", Forma::DevolveNovo));
    }
}
