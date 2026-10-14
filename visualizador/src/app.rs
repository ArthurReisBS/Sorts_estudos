use crate::detectar::{detectar, Deteccao, Lang};
use crate::runner::{self, Compilador, Medicao, Modo, Rastro};
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};
use egui_plot::{Legend, Line, Plot, PlotPoints, Points};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

const PADROES: [&str; 5] = ["aleatório", "crescente", "decrescente", "quase ordenado", "poucos valores"];
const LANGS: [Lang; 2] = [Lang::C, Lang::Rust];

const COR_BARRA: Color32 = Color32::from_rgb(110, 130, 160);
const COR_LEITURA: Color32 = Color32::from_rgb(240, 190, 60);
const COR_ESCRITA: Color32 = Color32::from_rgb(230, 80, 70);
const COR_OK: Color32 = Color32::from_rgb(90, 190, 110);
const COR_C: Color32 = Color32::from_rgb(90, 150, 230);
const COR_RUST: Color32 = Color32::from_rgb(230, 120, 60);

fn cor_lang(l: Lang) -> Color32 {
    match l {
        Lang::C => COR_C,
        Lang::Rust => COR_RUST,
    }
}

enum Msg {
    Status { lang: Lang, geracao: u64, texto: String },
    Rastro { lang: Lang, geracao: u64, res: Result<Rastro, String> },
    MedidaStatus(String),
    Medida { sort: String, lang: Lang, res: Result<Medicao, String> },
    MedidaFim,
}

#[derive(PartialEq)]
enum Aba {
    Animacao,
    Comparacao,
}

enum Estado {
    Parado,
    Trabalhando(String),
    Erro(String),
}

/// Um lado da tela (C ou Rust) na animação.
struct Painel {
    lang: Lang,
    deteccao: Deteccao,
    estado: Estado,
    rastro: Option<Rastro>,
    vetor: Vec<i64>,
    /// para cada posição: (número do último evento que tocou nela, foi escrita?)
    toque: Vec<Option<(usize, bool)>>,
    pos: usize,
    leituras: usize,
    escritas: usize,
    fora: usize,
    ultimo_fora: Option<i64>,
    max_val: i64,
}

impl Painel {
    fn novo(lang: Lang) -> Self {
        Self {
            lang,
            deteccao: Deteccao::NaoExiste,
            estado: Estado::Parado,
            rastro: None,
            vetor: vec![],
            toque: vec![],
            pos: 0,
            leituras: 0,
            escritas: 0,
            fora: 0,
            ultimo_fora: None,
            max_val: 1,
        }
    }

    fn total(&self) -> usize {
        self.rastro.as_ref().map_or(0, |r| r.eventos.len())
    }

    fn carregar(&mut self, r: Rastro) {
        let n = r.inicial.len() as i64;
        self.max_val = r
            .inicial
            .iter()
            .copied()
            .chain(r.eventos.iter().filter(|e| e.escrita && e.idx >= 0 && e.idx < n).map(|e| e.val))
            .max()
            .unwrap_or(1)
            .max(1);
        self.rastro = Some(r);
        self.reiniciar();
    }

    fn reiniciar(&mut self) {
        if let Some(r) = &self.rastro {
            self.vetor = r.inicial.clone();
            self.toque = vec![None; r.inicial.len()];
        }
        self.pos = 0;
        self.leituras = 0;
        self.escritas = 0;
        self.fora = 0;
        self.ultimo_fora = None;
    }

    fn ir_para(&mut self, alvo: usize) {
        let alvo = alvo.min(self.total());
        if alvo < self.pos {
            self.reiniciar();
        }
        let Some(r) = self.rastro.take() else { return };
        let n = self.vetor.len() as i64;
        while self.pos < alvo {
            let e = r.eventos[self.pos];
            let dentro = e.idx >= 0 && e.idx < n;
            if !dentro {
                self.fora += 1;
                self.ultimo_fora = Some(e.idx);
            }
            if e.escrita {
                self.escritas += 1;
                if dentro {
                    self.vetor[e.idx as usize] = e.val;
                }
            } else {
                self.leituras += 1;
            }
            if dentro {
                self.toque[e.idx as usize] = Some((self.pos, e.escrita));
            }
            self.pos += 1;
        }
        self.rastro = Some(r);
    }

    fn terminou(&self) -> bool {
        self.rastro.is_some() && self.pos >= self.total()
    }
}

struct LinhaMedida {
    sort: String,
    lang: Lang,
    res: Result<Medicao, String>,
}

pub struct App {
    raiz: PathBuf,
    compilador: Result<Arc<Compilador>, String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,

    sorts: Vec<String>,
    selecionado: String,
    ultima_varredura: Instant,
    aba: Aba,

    // animação
    n: usize,
    semente: u32,
    padrao: usize,
    fim_inclusivo: bool,
    paineis: [Painel; 2],
    geracao: u64,
    tocando: bool,
    passo: f64,
    velocidade: f64,
    eventos_por_quadro: f64,

    // comparação de tempo
    tamanhos: String,
    reps: usize,
    padrao_medida: usize,
    medidas: Vec<LinhaMedida>,
    medindo: Option<String>,
    escala_log: bool,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, raiz: PathBuf, build: PathBuf) -> Self {
        let (tx, rx) = channel();
        let mut app = Self {
            raiz,
            compilador: Compilador::novo(build).map(Arc::new),
            tx,
            rx,
            sorts: vec![],
            selecionado: String::new(),
            ultima_varredura: Instant::now(),
            aba: Aba::Animacao,
            n: 40,
            semente: 42,
            padrao: 0,
            fim_inclusivo: true,
            paineis: [Painel::novo(Lang::C), Painel::novo(Lang::Rust)],
            geracao: 0,
            tocando: false,
            passo: 0.0,
            velocidade: 60.0,
            eventos_por_quadro: 1.0,
            tamanhos: "100, 500, 1000, 2000, 5000".into(),
            reps: 5,
            padrao_medida: 0,
            medidas: vec![],
            medindo: None,
            escala_log: false,
        };
        app.varrer();
        app.selecionado = if app.sorts.iter().any(|s| s == "BubbleSort") {
            "BubbleSort".into()
        } else {
            app.sorts.first().cloned().unwrap_or_default()
        };
        app.redetectar();
        app
    }

    fn arquivo(&self, sort: &str, lang: Lang) -> PathBuf {
        self.raiz.join(lang.pasta()).join(format!("{sort}.{}", lang.ext()))
    }

    /// Lista os sorts que existem em C/ ou Rust/.
    fn varrer(&mut self) {
        let mut nomes = std::collections::BTreeSet::new();
        for l in LANGS {
            if let Ok(rd) = std::fs::read_dir(self.raiz.join(l.pasta())) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some(l.ext()) {
                        if let Some(s) = p.file_stem().and_then(|s| s.to_str()) {
                            nomes.insert(s.to_string());
                        }
                    }
                }
            }
        }
        self.sorts = nomes.into_iter().collect();
        self.ultima_varredura = Instant::now();
    }

    fn detectar_sort(&self, sort: &str, lang: Lang) -> Deteccao {
        match std::fs::read_to_string(self.arquivo(sort, lang)) {
            Ok(src) => detectar(lang, &src, sort),
            Err(_) => Deteccao::NaoExiste,
        }
    }

    fn redetectar(&mut self) {
        for i in 0..2 {
            let d = self.detectar_sort(&self.selecionado.clone(), self.paineis[i].lang);
            self.paineis[i].deteccao = d;
        }
    }

    fn total(&self) -> usize {
        self.paineis.iter().map(Painel::total).max().unwrap_or(0)
    }

    fn executar(&mut self) {
        self.redetectar();
        self.geracao += 1;
        self.tocando = false;
        self.passo = 0.0;
        let comp = match &self.compilador {
            Ok(c) => c.clone(),
            Err(e) => {
                for p in &mut self.paineis {
                    p.estado = Estado::Erro(format!("não consegui preparar o rastreador:\n{e}"));
                }
                return;
            }
        };
        for i in 0..2 {
            let p = &mut self.paineis[i];
            p.rastro = None;
            p.reiniciar();
            let Deteccao::Ok(assin) = p.deteccao.clone() else {
                p.estado = Estado::Parado;
                continue;
            };
            p.estado = Estado::Trabalhando("compilando…".into());
            let (lang, tx, geracao, comp) = (p.lang, self.tx.clone(), self.geracao, comp.clone());
            let arquivo = self.arquivo(&self.selecionado, lang);
            let (n, semente, padrao, fim) = (self.n, self.semente, self.padrao, self.fim_inclusivo);
            std::thread::spawn(move || {
                let res = comp.compilar(lang, &arquivo, &assin, fim, Modo::Rastro).and_then(|bin| {
                    let _ = tx.send(Msg::Status { lang, geracao, texto: "executando…".into() });
                    runner::rastrear(&bin, n, semente, padrao, 15)
                });
                let _ = tx.send(Msg::Rastro { lang, geracao, res });
            });
        }
    }

    fn medir(&mut self, todos: bool) {
        let Ok(comp) = self.compilador.clone() else { return };
        let tamanhos: Vec<usize> = self
            .tamanhos
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|s| s.trim().parse().ok())
            .filter(|&n| n >= 2)
            .collect();
        if tamanhos.is_empty() {
            return;
        }
        let sorts: Vec<String> = if todos { self.sorts.clone() } else { vec![self.selecionado.clone()] };
        let mut tarefas = vec![];
        for s in &sorts {
            self.medidas.retain(|m| &m.sort != s);
            for l in LANGS {
                if let Deteccao::Ok(a) = self.detectar_sort(s, l) {
                    tarefas.push((s.clone(), l, self.arquivo(s, l), a));
                }
            }
        }
        let (tx, fim, semente, padrao, reps) = (self.tx.clone(), self.fim_inclusivo, self.semente, self.padrao_medida, self.reps);
        self.medindo = Some("preparando…".into());
        // uma medida por vez: duas ao mesmo tempo disputariam a CPU e sujariam os tempos
        std::thread::spawn(move || {
            for (sort, lang, arquivo, assin) in tarefas {
                let _ = tx.send(Msg::MedidaStatus(format!("{sort} em {}: compilando…", lang.nome())));
                let res = comp.compilar(lang, &arquivo, &assin, fim, Modo::Tempo).and_then(|bin| {
                    let _ = tx.send(Msg::MedidaStatus(format!("{sort} em {}: medindo…", lang.nome())));
                    runner::medir(&bin, &tamanhos, semente, padrao, reps, 60)
                });
                let _ = tx.send(Msg::Medida { sort, lang, res });
            }
            let _ = tx.send(Msg::MedidaFim);
        });
    }

    fn receber(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Status { lang, geracao, texto } if geracao == self.geracao => {
                    self.painel_mut(lang).estado = Estado::Trabalhando(texto);
                }
                Msg::Rastro { lang, geracao, res } if geracao == self.geracao => {
                    let p = self.painel_mut(lang);
                    match res {
                        Ok(r) => {
                            p.estado = Estado::Parado;
                            p.carregar(r);
                        }
                        Err(e) => p.estado = Estado::Erro(e),
                    }
                    if self.paineis.iter().all(|p| !matches!(p.estado, Estado::Trabalhando(_))) {
                        self.tocando = self.total() > 0;
                    }
                }
                Msg::MedidaStatus(s) => self.medindo = Some(s),
                Msg::Medida { sort, lang, res } => self.medidas.push(LinhaMedida { sort, lang, res }),
                Msg::MedidaFim => self.medindo = None,
                _ => {}
            }
        }
    }

    fn painel_mut(&mut self, lang: Lang) -> &mut Painel {
        self.paineis.iter_mut().find(|p| p.lang == lang).unwrap()
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receber();
        if self.ultima_varredura.elapsed().as_secs_f32() > 1.0 {
            self.varrer();
            self.redetectar();
        }

        // avança a animação
        let dt = ui.input(|i| i.stable_dt).min(0.1) as f64;
        self.eventos_por_quadro = (self.velocidade * dt).max(1.0);
        if self.tocando {
            self.passo = (self.passo + self.velocidade * dt).min(self.total() as f64);
            if self.passo as usize >= self.total() {
                self.tocando = false;
            }
        }
        let alvo = self.passo as usize;
        for p in &mut self.paineis {
            p.ir_para(alvo);
        }

        egui::Panel::top("topo").show(ui, |ui| self.ui_topo(ui));
        match self.aba {
            Aba::Animacao => {
                egui::Panel::bottom("resumo").show(ui, |ui| self.ui_resumo(ui));
                egui::CentralPanel::default_margins().show(ui, |ui| self.ui_animacao(ui));
            }
            Aba::Comparacao => {
                egui::CentralPanel::default_margins().show(ui, |ui| self.ui_comparacao(ui));
            }
        }

        let ocupado = self.tocando
            || self.medindo.is_some()
            || self.paineis.iter().any(|p| matches!(p.estado, Estado::Trabalhando(_)));
        if ocupado {
            ui.ctx().request_repaint();
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

// ------------------------------------------------------------------ telas

impl App {
    fn ui_topo(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.aba, Aba::Animacao, RichText::new("Animação").heading());
            ui.selectable_value(&mut self.aba, Aba::Comparacao, RichText::new("Comparação de tempo").heading());
            ui.separator();
            ui.label(RichText::new(self.raiz.display().to_string()).weak().small());
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Sort:");
            let antes = self.selecionado.clone();
            egui::ComboBox::from_id_salt("sort").selected_text(&self.selecionado).show_ui(ui, |ui| {
                for s in self.sorts.clone() {
                    let estado = LANGS
                        .iter()
                        .map(|&l| match self.detectar_sort(&s, l) {
                            Deteccao::Ok(_) => format!("{} ✔", l.nome()),
                            Deteccao::Vazio => format!("{} vazio", l.nome()),
                            Deteccao::NaoExiste => format!("{} —", l.nome()),
                            Deteccao::Erro(_) => format!("{} ?", l.nome()),
                        })
                        .collect::<Vec<_>>()
                        .join(" · ");
                    ui.selectable_value(&mut self.selecionado, s.clone(), format!("{s}   ({estado})"));
                }
            });
            if self.selecionado != antes {
                self.redetectar();
                for p in &mut self.paineis {
                    p.rastro = None;
                    p.estado = Estado::Parado;
                    p.reiniciar();
                }
                self.passo = 0.0;
                self.tocando = false;
            }
            ui.separator();
            if self.aba == Aba::Animacao {
                ui.label("N:");
                ui.add(egui::Slider::new(&mut self.n, 2..=400).logarithmic(true));
                ui.label("Entrada:");
                combo_padrao(ui, "padrao", &mut self.padrao);
                ui.label("Semente:");
                ui.add(egui::DragValue::new(&mut self.semente));
            }
            ui.checkbox(&mut self.fim_inclusivo, "(v, início, fim): fim = n−1")
                .on_hover_text("Para sorts com assinatura (vetor, início, fim).\nMarcado: chama com fim = n−1 (último índice).\nDesmarcado: fim = n (uma posição depois do último).");
            if self.aba == Aba::Animacao {
                ui.separator();
                if ui.button(RichText::new("▶ Executar").strong()).clicked() {
                    self.executar();
                }
            }
        });
        ui.add_space(4.0);
    }

    fn ui_animacao(&mut self, ui: &mut egui::Ui) {
        let total = self.total();
        ui.horizontal(|ui| {
            if ui.button("⏮").on_hover_text("voltar ao início").clicked() {
                self.passo = 0.0;
                self.tocando = false;
            }
            let rotulo = if self.tocando { "⏸ pausar" } else { "▶ tocar" };
            if ui.add_enabled(total > 0, egui::Button::new(rotulo)).clicked() {
                if !self.tocando && self.passo as usize >= total {
                    self.passo = 0.0;
                }
                self.tocando = !self.tocando;
            }
            if ui.button("⏭ +1").on_hover_text("um acesso de cada vez").clicked() {
                self.tocando = false;
                self.passo = (self.passo.floor() + 1.0).min(total as f64);
            }
            if ui.button("⏩ fim").clicked() {
                self.tocando = false;
                self.passo = total as f64;
            }
            ui.separator();
            ui.label("Velocidade:");
            ui.add(egui::Slider::new(&mut self.velocidade, 1.0..=200_000.0).logarithmic(true).suffix(" acessos/s"));
            ui.separator();
            let mut p = self.passo as usize;
            ui.label("Acesso:");
            let largura = (ui.available_width() - 10.0).max(100.0);
            ui.spacing_mut().slider_width = largura - 90.0;
            if ui.add(egui::Slider::new(&mut p, 0..=total.max(1))).changed() {
                self.passo = p as f64;
                self.tocando = false;
            }
        });
        ui.horizontal(|ui| {
            legenda(ui, COR_LEITURA, "leitura");
            legenda(ui, COR_ESCRITA, "escrita");
            legenda(ui, COR_OK, "ordenado no fim");
            ui.label(RichText::new("· cada acesso ao vetor é um passo; os dois lados andam no mesmo ritmo").weak());
        });
        ui.separator();

        let janela = self.eventos_por_quadro.max(2.0) as usize;
        ui.columns(2, |cols| {
            for (i, ui) in cols.iter_mut().enumerate() {
                ui_painel(ui, &self.paineis[i], janela);
            }
        });
    }

    fn ui_resumo(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(RichText::new("C × Rust nesta execução").strong());
        egui::Grid::new("resumo").striped(true).min_col_width(110.0).show(ui, |ui| {
            ui.label("");
            for p in &self.paineis {
                ui.label(RichText::new(p.lang.nome()).color(cor_lang(p.lang)).strong());
            }
            ui.label("diferença");
            ui.end_row();
            let linhas: [(&str, fn(&Painel) -> Option<usize>); 3] = [
                ("leituras", |p| p.rastro.as_ref().map(|r| r.eventos.iter().filter(|e| !e.escrita).count())),
                ("escritas", |p| p.rastro.as_ref().map(|r| r.eventos.iter().filter(|e| e.escrita).count())),
                ("acessos no total", |p| p.rastro.as_ref().map(|r| r.eventos.len())),
            ];
            for (nome, f) in linhas {
                ui.label(nome);
                let v: Vec<Option<usize>> = self.paineis.iter().map(f).collect();
                for x in &v {
                    ui.label(x.map_or("—".to_string(), |x| x.to_string()));
                }
                match (v[0], v[1]) {
                    (Some(c), Some(r)) if c > 0 => {
                        ui.label(format!("Rust fez {:+.0}% em relação ao C", (r as f64 / c as f64 - 1.0) * 100.0))
                    }
                    _ => ui.label(""),
                };
                ui.end_row();
            }
            ui.label("resultado");
            for p in &self.paineis {
                match &p.rastro {
                    Some(r) if r.ordenado => ui.label(RichText::new("ordenou ✔").color(COR_OK)),
                    Some(_) => ui.label(RichText::new("não ordenou ✘").color(COR_ESCRITA)),
                    None => ui.label("—"),
                };
            }
            ui.end_row();
        });
        ui.label(
            RichText::new("Os acessos são contados com o código sem otimização (-O0 / opt-level=0), então mostram o que o seu código pede, não o que a CPU faria depois do compilador otimizar. Para tempo real, use a aba Comparação de tempo.")
                .weak()
                .small(),
        );
        ui.add_space(4.0);
    }

    fn ui_comparacao(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Tamanhos (N):");
            ui.add(egui::TextEdit::singleline(&mut self.tamanhos).desired_width(220.0));
            ui.label("Repetições:");
            ui.add(egui::DragValue::new(&mut self.reps).range(1..=31));
            ui.label("Entrada:");
            combo_padrao(ui, "padrao_medida", &mut self.padrao_medida);
            ui.separator();
            let livre = self.medindo.is_none() && self.compilador.is_ok();
            if ui.add_enabled(livre, egui::Button::new(format!("Medir {}", self.selecionado))).clicked() {
                self.medir(false);
            }
            if ui.add_enabled(livre, egui::Button::new("Medir todos os sorts")).clicked() {
                self.medir(true);
            }
            if ui.add_enabled(livre && !self.medidas.is_empty(), egui::Button::new("Limpar")).clicked() {
                self.medidas.clear();
            }
            ui.checkbox(&mut self.escala_log, "escala log");
        });
        if let Some(s) = &self.medindo {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(s);
            });
        }
        if let Err(e) = &self.compilador {
            ui.colored_label(COR_ESCRITA, format!("não consegui preparar o rastreador: {e}"));
        }
        ui.label(
            RichText::new("Compilado com otimização (C com -O2, Rust com opt-level=3). Cada ponto é a mediana das repetições, com a mesma entrada nas duas linguagens.")
                .weak()
                .small(),
        );
        ui.separator();

        let altura = (ui.available_height() * 0.55).max(200.0);
        let log = self.escala_log;
        let tf = move |x: f64| if log { x.max(1e-9).log10() } else { x };
        Plot::new("tempos")
            .legend(Legend::default())
            .height(altura)
            .x_axis_label(if log { "log10(N)" } else { "N" })
            .y_axis_label(if log { "log10(tempo em ms)" } else { "tempo (ms)" })
            .show(ui, |pu| {
                for m in &self.medidas {
                    let Ok(med) = &m.res else { continue };
                    let pts: Vec<[f64; 2]> =
                        med.pontos.iter().map(|p| [tf(p.n as f64), tf(p.mediana_ns as f64 / 1e6)]).collect();
                    let nome = format!("{} ({})", m.sort, m.lang.nome());
                    let cor = cor_sort(&m.sort, m.lang, &self.sorts);
                    let estilo = if m.lang == Lang::C { egui_plot::LineStyle::Solid } else { egui_plot::LineStyle::dashed_dense() };
                    pu.line(Line::new(nome.clone(), PlotPoints::from(pts.clone())).color(cor).width(2.0).style(estilo));
                    pu.points(Points::new(nome, PlotPoints::from(pts)).color(cor).radius(3.0));
                }
            });
        ui.label(RichText::new("linha cheia = C · tracejada = Rust").weak().small());
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut sorts: Vec<&String> = self.medidas.iter().map(|m| &m.sort).collect();
            sorts.dedup();
            sorts.sort();
            sorts.dedup();
            egui::Grid::new("tabela").striped(true).min_col_width(90.0).show(ui, |ui| {
                for h in ["sort", "N", "C (ms)", "Rust (ms)", "Rust ÷ C", ""] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for s in sorts {
                    let pega = |l: Lang| self.medidas.iter().find(|m| &m.sort == s && m.lang == l).map(|m| &m.res);
                    let (c, r) = (pega(Lang::C), pega(Lang::Rust));
                    let mut ns: Vec<usize> = [c, r]
                        .iter()
                        .flatten()
                        .filter_map(|x| x.as_ref().ok())
                        .flat_map(|m| m.pontos.iter().map(|p| p.n))
                        .collect();
                    ns.sort();
                    ns.dedup();
                    let ponto = |x: Option<&Result<Medicao, String>>, n: usize| {
                        x.and_then(|x| x.as_ref().ok()).and_then(|m| m.pontos.iter().find(|p| p.n == n)).copied()
                    };
                    for &n in &ns {
                        ui.label(s.as_str());
                        ui.label(n.to_string());
                        let (pc, pr) = (ponto(c, n), ponto(r, n));
                        for p in [pc, pr] {
                            match p {
                                Some(p) => ui.label(format!("{:.3}", p.mediana_ns as f64 / 1e6)),
                                None => ui.label("—"),
                            };
                        }
                        match (pc, pr) {
                            (Some(a), Some(b)) if a.mediana_ns > 0 => {
                                let x = b.mediana_ns as f64 / a.mediana_ns as f64;
                                let cor = if x <= 1.0 { COR_RUST } else { COR_C };
                                ui.colored_label(cor, format!("{x:.2}×"))
                            }
                            _ => ui.label(""),
                        };
                        let ruins: Vec<&str> = [(pc, "C"), (pr, "Rust")]
                            .iter()
                            .filter(|(p, _)| p.is_some_and(|p| !p.ok))
                            .map(|(_, l)| *l)
                            .collect();
                        if ruins.is_empty() {
                            ui.label("");
                        } else {
                            ui.colored_label(COR_ESCRITA, format!("não ordenou: {}", ruins.join(", ")));
                        }
                        ui.end_row();
                    }
                    // problemas (não compilou, panic, tempo esgotado)
                    for (x, l) in [(c, Lang::C), (r, Lang::Rust)] {
                        let msg = match x {
                            Some(Err(e)) => Some(e.lines().take(3).collect::<Vec<_>>().join(" ")),
                            Some(Ok(m)) => m.problema.clone(),
                            None => None,
                        };
                        if let Some(msg) = msg {
                            ui.label(s.as_str());
                            ui.label("");
                            ui.colored_label(COR_ESCRITA, format!("{}: {msg}", l.nome()));
                            ui.end_row();
                        }
                    }
                }
            });
        });
    }
}

fn combo_padrao(ui: &mut egui::Ui, id: &str, padrao: &mut usize) {
    egui::ComboBox::from_id_salt(id).selected_text(PADROES[*padrao]).show_ui(ui, |ui| {
        for (i, nome) in PADROES.iter().enumerate() {
            ui.selectable_value(padrao, i, *nome);
        }
    });
}

fn legenda(ui: &mut egui::Ui, cor: Color32, texto: &str) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
    ui.painter().rect_filled(r, 2.0, cor);
    ui.label(texto);
}

/// Cor estável por sort; C e Rust do mesmo sort ficam em tons próximos.
fn cor_sort(sort: &str, lang: Lang, todos: &[String]) -> Color32 {
    let i = todos.iter().position(|s| s == sort).unwrap_or(0);
    let h = (i as f32 * 0.137) % 1.0;
    let (s, v) = if lang == Lang::C { (0.65, 0.95) } else { (0.85, 0.75) };
    egui::ecolor::Hsva::new(h, s, v, 1.0).into()
}

fn ui_painel(ui: &mut egui::Ui, p: &Painel, janela: usize) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(p.lang.nome()).heading().color(cor_lang(p.lang)));
        match &p.deteccao {
            Deteccao::Ok(a) => {
                ui.label(RichText::new(&a.texto).monospace());
            }
            Deteccao::Vazio => {
                ui.label(RichText::new("arquivo vazio: ainda não implementado").weak());
            }
            Deteccao::NaoExiste => {
                ui.label(RichText::new("não existe esse arquivo").weak());
            }
            Deteccao::Erro(e) => {
                ui.colored_label(COR_ESCRITA, e);
            }
        }
    });

    match &p.estado {
        Estado::Trabalhando(t) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t);
            });
        }
        Estado::Erro(e) => {
            ui.colored_label(COR_ESCRITA, "não compilou / não rodou:");
            egui::ScrollArea::vertical().id_salt(("erro", p.lang.nome())).max_height(ui.available_height() - 10.0).show(ui, |ui| {
                ui.label(RichText::new(e).monospace().small());
            });
            return;
        }
        Estado::Parado => {}
    }

    let Some(r) = &p.rastro else {
        if matches!(p.deteccao, Deteccao::Ok(_)) && matches!(p.estado, Estado::Parado) {
            ui.label(RichText::new("aperte ▶ Executar").weak());
        }
        return;
    };

    // números
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("acesso {} de {}", p.pos, r.eventos.len()));
        ui.separator();
        ui.label(RichText::new(format!("{} leituras", p.leituras)).color(COR_LEITURA));
        ui.label(RichText::new(format!("{} escritas", p.escritas)).color(COR_ESCRITA));
        if p.fora > 0 {
            ui.separator();
            ui.colored_label(
                COR_ESCRITA,
                format!("⚠ {} acesso(s) fora do vetor (último: índice {})", p.fora, p.ultimo_fora.unwrap_or(0)),
            );
        }
    });
    if p.terminou() {
        ui.horizontal_wrapped(|ui| {
            if r.ordenado {
                ui.colored_label(COR_OK, "terminou: ordenou ✔");
            } else {
                ui.colored_label(COR_ESCRITA, "terminou, mas o vetor não ficou ordenado ✘");
            }
            if let Some(pr) = &r.problema {
                ui.colored_label(COR_ESCRITA, pr);
            }
            if r.truncado {
                ui.colored_label(COR_LEITURA, "rastro cortado em 4 milhões de acessos");
            }
        });
    }

    // barras
    let tamanho = Vec2::new(ui.available_width(), (ui.available_height() - 6.0).max(80.0));
    let (resp, painter) = ui.allocate_painter(tamanho, Sense::hover());
    let rect = resp.rect;
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
    let n = p.vetor.len().max(1);
    let area = rect.shrink(4.0);
    let w = area.width() / n as f32;
    let fim_ok = p.terminou() && r.ordenado;
    let mostrar_valor = w >= 22.0;
    for (i, &v) in p.vetor.iter().enumerate() {
        let h = (v.max(0) as f32 / p.max_val as f32) * (area.height() - if mostrar_valor { 16.0 } else { 0.0 });
        let x0 = area.left() + i as f32 * w;
        let barra = egui::Rect::from_min_max(egui::pos2(x0 + w * 0.08, area.bottom() - h), egui::pos2(x0 + w * 0.92, area.bottom()));
        let cor = match p.toque[i] {
            _ if fim_ok => COR_OK,
            Some((quando, escrita)) if p.pos.saturating_sub(quando + 1) < janela => {
                if escrita {
                    COR_ESCRITA
                } else {
                    COR_LEITURA
                }
            }
            _ => COR_BARRA,
        };
        painter.rect_filled(barra, 1.0, cor);
        if mostrar_valor {
            painter.text(
                egui::pos2(barra.center().x, barra.top() - 2.0),
                egui::Align2::CENTER_BOTTOM,
                v.to_string(),
                egui::FontId::monospace(11.0),
                ui.visuals().text_color(),
            );
        }
    }
    if let Some(i) = resp.hover_pos().map(|h| ((h.x - area.left()) / w).floor() as i64) {
        if i >= 0 && (i as usize) < p.vetor.len() {
            resp.on_hover_text(format!("V[{i}] = {}", p.vetor[i as usize]));
        }
    }
}
