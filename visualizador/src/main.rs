mod app;
mod detectar;
mod runner;

use eframe::egui;
use std::path::PathBuf;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verificar = args.iter().any(|a| a == "--verificar");
    // pasta dos sorts: o argumento, ou a pasta acima do visualizador
    let raiz = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));
    let raiz = raiz.canonicalize().unwrap_or(raiz);
    let build = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("vt");

    if verificar {
        verificar_tudo(&raiz, build);
        return Ok(());
    }

    let opcoes = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_title("Visualizador de sorts"),
        ..Default::default()
    };
    eframe::run_native("Visualizador de sorts", opcoes, Box::new(move |cc| Ok(Box::new(app::App::new(cc, raiz, build)))))
}

/// `cargo run -- --verificar`: roda cada sort uma vez no terminal, sem abrir janela.
fn verificar_tudo(raiz: &std::path::Path, build: PathBuf) {
    use detectar::{detectar, Deteccao, Lang};
    let comp = match runner::Compilador::novo(build) {
        Ok(c) => c,
        Err(e) => return eprintln!("não consegui preparar o rastreador:\n{e}"),
    };
    for lang in [Lang::C, Lang::Rust] {
        let Ok(rd) = std::fs::read_dir(raiz.join(lang.pasta())) else { continue };
        let mut arquivos: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == lang.ext())).collect();
        arquivos.sort();
        for arq in arquivos {
            let nome = arq.file_stem().unwrap().to_string_lossy().to_string();
            let src = std::fs::read_to_string(&arq).unwrap_or_default();
            let linha = match detectar(lang, &src, &nome) {
                Deteccao::Vazio | Deteccao::NaoExiste => "vazio".to_string(),
                Deteccao::Erro(e) => format!("não reconhecido: {e}"),
                Deteccao::Ok(a) => match comp.compilar(lang, &arq, &a, true, runner::Modo::Rastro).and_then(|b| runner::rastrear(&b, 30, 42, 0, 15)) {
                    Err(e) => format!("erro: {}", e.lines().next().unwrap_or("")),
                    Ok(r) => {
                        let tempo = comp
                            .compilar(lang, &arq, &a, true, runner::Modo::Tempo)
                            .and_then(|b| runner::medir(&b, &[1000], 42, 0, 3, 20))
                            .ok()
                            .and_then(|m| m.pontos.first().copied())
                            .map(|p| format!(" · N=1000 em {:.3} ms", p.mediana_ns as f64 / 1e6))
                            .unwrap_or_default();
                        format!(
                            "{} acessos, {} · {}{}{}",
                            r.eventos.len(),
                            if r.ordenado { "ordenou" } else { "NÃO ordenou" },
                            a.chamada(lang, true),
                            tempo,
                            r.problema.map(|p| format!(" · {p}")).unwrap_or_default()
                        )
                    }
                },
            };
            println!("{:<5} {:<15} {linha}", lang.nome(), nome);
        }
    }
}
