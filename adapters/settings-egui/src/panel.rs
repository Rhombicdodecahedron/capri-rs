//! The panel itself: a header, numbered tabs on the left, the section in the middle, and a status bar
//! with SAVE at the bottom.

use std::sync::{Arc, Mutex};

use capri_config::{Config, Engine};
use capri_domain::{ApprovalMode, Language, Persona};
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke};

use crate::theme::{CYAN, CYAN_DIM, DEEP, GREEN, MUTED, NIGHT, PANEL, RED, TEXT};
use crate::{SettingsInput, TestReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Brain,
    Body,
    Persona,
    Safety,
    Interface,
}

const TABS: [Tab; 5] = [Tab::Brain, Tab::Body, Tab::Persona, Tab::Safety, Tab::Interface];

#[derive(Debug, Clone)]
enum TestState {
    Running,
    Ok(TestReport),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeySource {
    None,
    Env,
    File,
}

pub struct Panel {
    input: SettingsInput,
    saved: Config,
    cfg: Config,
    tab: Tab,
    key_source: KeySource,
    show_key: bool,
    custom_prompt: bool,
    test: Arc<Mutex<Option<TestState>>>,
    status: Option<(String, bool)>,
    load_error: Option<String>,
    auto_test: bool,
}

impl std::fmt::Debug for Panel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Panel").field("tab", &self.tab).finish_non_exhaustive()
    }
}

impl Panel {
    pub fn new(input: SettingsInput) -> Self {
        let (cfg, load_error) = match Config::load_or_create(&input.config_path) {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(format!("{e:#}"))),
        };
        let key_source = if cfg.brain.api_key_env.as_deref().is_some_and(|v| !v.is_empty()) {
            KeySource::Env
        } else if cfg.brain.api_key.as_deref().is_some_and(|v| !v.is_empty()) {
            KeySource::File
        } else {
            KeySource::None
        };
        let custom_prompt = cfg.persona.prompt.is_some();
        Self {
            input,
            saved: cfg.clone(),
            cfg,
            tab: Tab::Brain,
            key_source,
            show_key: false,
            custom_prompt,
            test: Arc::new(Mutex::new(None)),
            status: None,
            load_error,
            auto_test: false,
        }
    }

    /// Runs the link test as soon as the panel opens.
    #[must_use]
    pub fn with_auto_test(mut self) -> Self {
        self.auto_test = true;
        self
    }

    fn lang(&self) -> Language {
        self.cfg.ui.language.resolve(self.input.system_locale.as_deref())
    }

    fn t<'a>(&self, en: &'a str, fr: &'a str) -> &'a str {
        self.lang().pick(en, fr)
    }

    fn dirty(&self) -> bool {
        // compare through TOML so f32 noise or field order never counts as a change
        toml_eq(&self.cfg, &self.saved).is_none_or(|eq| !eq)
    }

    fn tab_label(&self, tab: Tab) -> &'static str {
        match tab {
            Tab::Brain => self.t("BRAIN", "CERVEAU"),
            Tab::Body => self.t("BODY", "CORPS"),
            Tab::Persona => "PERSONA",
            Tab::Safety => self.t("SAFETY", "SÉCURITÉ"),
            Tab::Interface => "INTERFACE",
        }
    }

    fn save(&mut self) {
        self.normalise();
        match self.cfg.save(&self.input.config_path) {
            Ok(()) => {
                self.saved = self.cfg.clone();
                self.status = Some((
                    self.t("SAVED. Capri applies it now.", "ENREGISTRÉ. Capri l'applique tout de suite.").into(),
                    true,
                ));
            }
            Err(e) => self.status = Some((format!("{e:#}"), false)),
        }
    }

    /// Turns the panel's choices back into the file's representation.
    fn normalise(&mut self) {
        let b = &mut self.cfg.brain;
        match self.key_source {
            KeySource::None => {
                b.api_key = None;
                b.api_key_env = None;
            }
            KeySource::Env => b.api_key = None,
            KeySource::File => b.api_key_env = None,
        }
        for v in [&mut b.api_key, &mut b.api_key_env, &mut b.base_url] {
            if v.as_deref().is_some_and(|s| s.trim().is_empty()) {
                *v = None;
            }
        }
        if !self.custom_prompt {
            self.cfg.persona.prompt = None;
        }
        if self.cfg.persona.name.as_deref().is_some_and(|s| s.trim().is_empty()) {
            self.cfg.persona.name = None;
        }
        if self.cfg.claude_code.model.as_deref().is_some_and(|s| s.trim().is_empty()) {
            self.cfg.claude_code.model = None;
        }
    }

    fn start_test(&mut self, ctx: &egui::Context) {
        self.normalise();
        *self.test.lock().expect("test lock") = Some(TestState::Running);
        let slot = self.test.clone();
        let tester = self.input.tester.clone();
        let (brain, cc) = (self.cfg.brain.clone(), self.cfg.claude_code.clone());
        let ctx = ctx.clone();
        self.input.runtime.spawn(async move {
            let result = tester.test(brain, cc).await;
            *slot.lock().expect("test lock") = Some(match result {
                Ok(r) => TestState::Ok(r),
                Err(e) => TestState::Failed(e),
            });
            ctx.request_repaint();
        });
    }
}

fn toml_eq(a: &Config, b: &Config) -> Option<bool> {
    Some(toml::to_string(a).ok()? == toml::to_string(b).ok()?)
}

/// Small caps label above a field.
fn label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).color(CYAN).size(12.0));
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(MUTED).size(12.5));
}

/// A section box with a cut corner and a red tab, like the chat bubble on the desk.
fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    let frame = egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, CYAN_DIM))
        .inner_margin(egui::Margin::symmetric(16, 12));
    let resp = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).color(RED).size(13.0));
        ui.add_space(2.0);
        body(ui);
    });
    let r = resp.response.rect;
    let p = ui.painter();
    p.rect_filled(egui::Rect::from_min_size(r.left_top() + egui::vec2(14.0, -1.5), egui::vec2(40.0, 3.0)), 0.0, RED);
    // cut the top-right corner
    let c = 10.0;
    p.add(egui::Shape::convex_polygon(
        vec![
            r.right_top() + egui::vec2(-c, -0.5),
            r.right_top() + egui::vec2(0.5, -0.5),
            r.right_top() + egui::vec2(0.5, c),
        ],
        NIGHT,
        Stroke::NONE,
    ));
    p.line_segment(
        [r.right_top() + egui::vec2(-c, 0.0), r.right_top() + egui::vec2(0.0, c)],
        Stroke::new(1.0_f32, CYAN_DIM),
    );
    ui.add_space(10.0);
}

fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

fn neon_button(ui: &mut egui::Ui, text: &str, hot: bool) -> egui::Response {
    let (fg, stroke, fill) = if hot { (DEEP, RED, RED) } else { (CYAN, CYAN, Color32::TRANSPARENT) };
    ui.add(
        egui::Button::new(RichText::new(text).color(fg).strong())
            .fill(fill)
            .stroke(Stroke::new(1.0_f32, stroke))
            .corner_radius(CornerRadius::ZERO),
    )
}

impl eframe::App for Panel {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if std::mem::take(&mut self.auto_test) {
            self.start_test(ctx);
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) && self.dirty() {
            self.save();
        }
        self.header(ctx);
        self.footer(ctx);
        self.sidebar(ctx);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(NIGHT).inner_margin(egui::Margin::same(18))).show(
            ctx,
            |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if let Some(e) = &self.load_error {
                        ui.colored_label(RED, format!("capri.toml: {e}"));
                    }
                    match self.tab {
                        Tab::Brain => self.brain_tab(ui),
                        Tab::Body => self.body_tab(ui),
                        Tab::Persona => self.persona_tab(ui),
                        Tab::Safety => self.safety_tab(ui),
                        Tab::Interface => self.interface_tab(ui),
                    }
                });
            },
        );
    }
}

impl Panel {
    fn header(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .exact_height(58.0)
            .frame(egui::Frame::new().fill(DEEP).inner_margin(egui::Margin::symmetric(18, 10)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("CAPRI").color(RED).size(26.0).strong());
                    ui.label(RichText::new("//").color(CYAN_DIM).size(22.0));
                    ui.label(RichText::new(self.t("NEURAL CONFIG", "CONFIG NEURALE")).color(CYAN).size(18.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).color(MUTED).size(12.0));
                        let online = matches!(*self.test.lock().expect("test lock"), Some(TestState::Ok(_)));
                        let (dot, txt) = if online {
                            (GREEN, self.t("LINK OK", "LIEN OK"))
                        } else {
                            (MUTED, self.t("LINK ?", "LIEN ?"))
                        };
                        ui.label(RichText::new(txt).color(dot).size(12.0));
                        status_dot(ui, dot);
                    });
                });
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [r.left_bottom() + egui::vec2(-18.0, 10.0), r.right_bottom() + egui::vec2(18.0, 10.0)],
                    Stroke::new(2.0_f32, RED),
                );
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("tabs")
            .exact_width(190.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(DEEP).inner_margin(egui::Margin::symmetric(0, 16)))
            .show(ctx, |ui| {
                for (i, tab) in TABS.into_iter().enumerate() {
                    let selected = self.tab == tab;
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(ui.available_width(), 42.0), egui::Sense::click());
                    let p = ui.painter();
                    if selected {
                        p.rect_filled(rect, 0.0, PANEL);
                        p.rect_filled(
                            egui::Rect::from_min_size(rect.left_top(), egui::vec2(3.0, rect.height())),
                            0.0,
                            RED,
                        );
                    } else if resp.hovered() {
                        p.rect_filled(rect, 0.0, Color32::from_rgb(0x0e, 0x0e, 0x18));
                    }
                    let num_color = if selected { RED } else { CYAN_DIM };
                    p.text(
                        rect.left_center() + egui::vec2(18.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{:02}", i + 1),
                        egui::FontId::proportional(13.0),
                        num_color,
                    );
                    let txt_color = if selected {
                        TEXT
                    } else if resp.hovered() {
                        CYAN
                    } else {
                        MUTED
                    };
                    p.text(
                        rect.left_center() + egui::vec2(48.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        self.tab_label(tab),
                        egui::FontId::proportional(16.0),
                        txt_color,
                    );
                    if resp.clicked() {
                        self.tab = tab;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        ui.label(
                            RichText::new(self.t("art: xilia_world00", "dessins : xilia_world00"))
                                .color(MUTED)
                                .size(11.0),
                        );
                    });
                });
            });
    }

    fn footer(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("footer")
            .exact_height(56.0)
            .frame(egui::Frame::new().fill(DEEP).inner_margin(egui::Margin::symmetric(18, 10)))
            .show(ctx, |ui| {
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [r.left_top() + egui::vec2(-18.0, -10.0), r.right_top() + egui::vec2(18.0, -10.0)],
                    Stroke::new(1.0_f32, CYAN_DIM),
                );
                ui.horizontal_centered(|ui| {
                    match &self.status {
                        Some((msg, true)) => ui.label(RichText::new(format!("> {msg}")).color(GREEN)),
                        Some((msg, false)) => ui.label(RichText::new(format!("> {msg}")).color(RED)),
                        None if self.dirty() => ui.label(
                            RichText::new(self.t("> unsaved changes", "> modifications non enregistrées")).color(CYAN),
                        ),
                        None => ui
                            .label(RichText::new(self.input.config_path.display().to_string()).color(MUTED).size(12.0)),
                    };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let dirty = self.dirty();
                        if neon_button(ui, self.t("SAVE", "ENREGISTRER"), dirty).clicked() && dirty {
                            self.save();
                        }
                        if dirty && neon_button(ui, self.t("UNDO", "ANNULER"), false).clicked() {
                            self.cfg = self.saved.clone();
                            self.custom_prompt = self.cfg.persona.prompt.is_some();
                            self.status = None;
                        }
                    });
                });
            });
        if self.dirty() && self.status.as_ref().is_some_and(|(_, ok)| *ok) {
            self.status = None;
        }
    }

    fn brain_tab(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        let t = |en: &'static str, fr: &'static str| lang.pick(en, fr);
        section(ui, t("ENGINE", "MOTEUR"), |ui| {
            egui::ComboBox::from_id_salt("engine")
                .width(320.0)
                .selected_text(engine_name(self.cfg.brain.engine))
                .show_ui(ui, |ui| {
                    for e in [
                        Engine::LlamaServer,
                        Engine::Ollama,
                        Engine::LmStudio,
                        Engine::Openai,
                        Engine::Openrouter,
                        Engine::Custom,
                        Engine::ClaudeCode,
                    ] {
                        ui.selectable_value(&mut self.cfg.brain.engine, e, engine_name(e));
                    }
                });
            hint(ui, engine_hint(self.cfg.brain.engine, lang));
        });

        if self.cfg.brain.engine == Engine::ClaudeCode {
            self.claude_code_section(ui);
            self.link_test(ui);
        } else {
            self.connection_section(ui);

            self.link_test(ui);

            self.model_section(ui);
        }
    }

    fn claude_code_section(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        let t = |en: &'static str, fr: &'static str| lang.pick(en, fr);
        section(ui, "CLAUDE CODE", |ui| {
            label(ui, t("MODEL", "MODÈLE"));
            let model = self.cfg.claude_code.model.get_or_insert_with(String::new);
            egui::ComboBox::from_id_salt("cc-model")
                .width(320.0)
                .selected_text(if model.is_empty() { t("CLI default", "défaut du CLI") } else { model.as_str() })
                .show_ui(ui, |ui| {
                    for m in ["", "haiku", "sonnet", "opus"] {
                        ui.selectable_value(
                            model,
                            m.to_string(),
                            if m.is_empty() { t("CLI default", "défaut du CLI") } else { m },
                        );
                    }
                });
            label(ui, t("EXECUTABLE (EMPTY = FROM PATH)", "EXÉCUTABLE (VIDE = DANS LE PATH)"));
            let mut exe = self.cfg.claude_code.executable.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            if ui.add(egui::TextEdit::singleline(&mut exe).desired_width(480.0).hint_text("claude")).changed() {
                self.cfg.claude_code.executable = (!exe.trim().is_empty()).then(|| exe.trim().into());
            }
            hint(
                ui,
                t(
                    "Uses your own Claude sign-in. Nothing to store here.",
                    "Utilise ta propre connexion Claude. Rien à stocker ici.",
                ),
            );
        });
    }

    fn connection_section(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        let t = |en: &'static str, fr: &'static str| lang.pick(en, fr);
        section(ui, t("CONNECTION", "CONNEXION"), |ui| {
            label(ui, t("SERVER URL (WITH /v1)", "URL DU SERVEUR (AVEC /v1)"));
            let default_url = self.cfg.brain.default_url().to_string();
            let url = self.cfg.brain.base_url.get_or_insert_with(String::new);
            ui.add(egui::TextEdit::singleline(url).desired_width(480.0).hint_text(if default_url.is_empty() {
                "https://..."
            } else {
                &default_url
            }));

            label(ui, t("API KEY", "CLÉ D'API"));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.key_source, KeySource::None, t("none", "aucune"));
                ui.selectable_value(
                    &mut self.key_source,
                    KeySource::Env,
                    t("environment variable", "variable d'environnement"),
                );
                ui.selectable_value(&mut self.key_source, KeySource::File, t("in the file", "dans le fichier"));
            });
            match self.key_source {
                KeySource::None => {}
                KeySource::Env => {
                    let v = self.cfg.brain.api_key_env.get_or_insert_with(String::new);
                    ui.add(egui::TextEdit::singleline(v).desired_width(320.0).hint_text("LLAMA_API_KEY"));
                }
                KeySource::File => {
                    ui.horizontal(|ui| {
                        let v = self.cfg.brain.api_key.get_or_insert_with(String::new);
                        ui.add(egui::TextEdit::singleline(v).desired_width(380.0).password(!self.show_key));
                        ui.toggle_value(&mut self.show_key, t("show", "voir"));
                    });
                    hint(
                        ui,
                        t(
                            "Stored in plain text in your profile folder.",
                            "Stockée en clair dans ton dossier de profil.",
                        ),
                    );
                }
            }
        });
    }

    fn model_section(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        let t = |en: &'static str, fr: &'static str| lang.pick(en, fr);
        section(ui, t("MODEL", "MODÈLE"), |ui| {
            let listed: Vec<String> = match &*self.test.lock().expect("test lock") {
                Some(TestState::Ok(r)) => r.models.clone(),
                _ => Vec::new(),
            };
            let model = &mut self.cfg.brain.model;
            let auto = t("auto (what the server serves)", "auto (celui du serveur)");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(model).desired_width(320.0).hint_text(auto));
                if !listed.is_empty() {
                    egui::ComboBox::from_id_salt("models").width(220.0).selected_text(t("pick…", "choisir…")).show_ui(
                        ui,
                        |ui| {
                            ui.selectable_value(model, String::new(), auto);
                            for m in &listed {
                                ui.selectable_value(model, m.clone(), m);
                            }
                        },
                    );
                }
            });
            hint(
                ui,
                t(
                    "Leave empty for llama.cpp: it serves one model. TEST lists what the server has.",
                    "Laisse vide pour llama.cpp : il sert un seul modèle. TESTER liste ce que le serveur propose.",
                ),
            );
            ui.add_space(4.0);
            ui.checkbox(
                &mut self.cfg.brain.thinking,
                t(
                    "Let reasoning models think first (slower first word)",
                    "Laisser réfléchir les modèles à raisonnement (premier mot plus lent)",
                ),
            );
            label(ui, t("TEMPERATURE", "TEMPÉRATURE"));
            let temp = self.cfg.brain.temperature.get_or_insert(0.8);
            ui.add(egui::Slider::new(temp, 0.0..=1.5).step_by(0.05));
            label(ui, t("MAX REPLY LENGTH (TOKENS)", "LONGUEUR MAX D'UNE RÉPONSE (TOKENS)"));
            let mt = self.cfg.brain.max_tokens.get_or_insert(400);
            ui.add(egui::Slider::new(mt, 64..=2048).logarithmic(true));
        });
    }

    fn link_test(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        let t = |en: &'static str, fr: &'static str| lang.pick(en, fr);
        section(ui, t("LINK TEST", "TEST DU LIEN"), |ui| {
            let running = matches!(*self.test.lock().expect("test lock"), Some(TestState::Running));
            ui.horizontal(|ui| {
                if neon_button(
                    ui,
                    if running {
                        t("TESTING…", "TEST EN COURS…")
                    } else {
                        t("TEST CONNECTION", "TESTER LA CONNEXION")
                    },
                    false,
                )
                .clicked()
                    && !running
                {
                    self.start_test(ui.ctx());
                }
                if running {
                    ui.spinner();
                }
            });
            match &*self.test.lock().expect("test lock") {
                Some(TestState::Ok(r)) => {
                    ui.horizontal_wrapped(|ui| {
                        status_dot(ui, GREEN);
                        ui.label(
                            RichText::new(format!(
                                "{}  {}  ·  {} {} ms  ·  {} {} ms",
                                t("ONLINE", "EN LIGNE"),
                                r.model,
                                t("first word", "premier mot"),
                                r.first_word_ms,
                                t("total", "total"),
                                r.total_ms
                            ))
                            .color(GREEN),
                        );
                    });
                    if !r.models.is_empty() {
                        hint(
                            ui,
                            &format!(
                                "{}: {}",
                                t("models on the server", "modèles sur le serveur"),
                                r.models.join(", ")
                            ),
                        );
                    }
                    ui.label(RichText::new(format!("« {} »", r.reply.trim())).color(TEXT).italics());
                }
                Some(TestState::Failed(e)) => {
                    ui.horizontal_wrapped(|ui| {
                        status_dot(ui, RED);
                        ui.label(RichText::new(format!("{}  {e}", t("OFFLINE", "HORS LIGNE"))).color(RED));
                    });
                }
                _ => hint(
                    ui,
                    t(
                        "Tries the settings above without saving them.",
                        "Essaie les réglages ci-dessus sans les enregistrer.",
                    ),
                ),
            }
        });
    }

    fn body_tab(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        let t = |en: &'static str, fr: &'static str| lang.pick(en, fr);
        let d = &mut self.cfg.desk;
        section(ui, t("LOOK", "APPARENCE"), |ui| {
            label(ui, "SKIN");
            let current = self.input.skins.iter().find(|s| s.id == d.skin).map_or(d.skin.clone(), |s| s.name.clone());
            egui::ComboBox::from_id_salt("skin").width(320.0).selected_text(current).show_ui(ui, |ui| {
                for s in &self.input.skins {
                    ui.selectable_value(&mut d.skin, s.id.clone(), &s.name);
                }
            });
            label(ui, t("SIZE (PX AT 100%)", "TAILLE (PX À 100 %)"));
            ui.add(egui::Slider::new(&mut d.size, 120.0..=480.0).step_by(10.0));
            label(ui, t("FRAMES PER SECOND", "IMAGES PAR SECONDE"));
            ui.add(egui::Slider::new(&mut d.fps, 30..=144));
        });
        section(ui, t("BEHAVIOUR", "COMPORTEMENT"), |ui| {
            ui.checkbox(
                &mut d.roaming,
                t("Hops from window to window on her own", "Saute de fenêtre en fenêtre toute seule"),
            );
            ui.add_enabled_ui(d.roaming, |ui| {
                label(ui, t("BETWEEN TWO HOPS (SECONDS)", "ENTRE DEUX SAUTS (SECONDES)"));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut d.roam_every_secs.0).range(3.0..=600.0).prefix("min "));
                    ui.add(egui::DragValue::new(&mut d.roam_every_secs.1).range(3.0..=600.0).prefix("max "));
                });
                if d.roam_every_secs.1 < d.roam_every_secs.0 {
                    d.roam_every_secs.1 = d.roam_every_secs.0;
                }
            });
            label(ui, t("DOZES OFF AFTER (MINUTES IDLE)", "S'ENDORT APRÈS (MINUTES SANS RIEN)"));
            let mut minutes = d.sleep_after_secs / 60.0;
            if ui.add(egui::Slider::new(&mut minutes, 1.0..=60.0).step_by(1.0)).changed() {
                d.sleep_after_secs = minutes * 60.0;
            }
        });
    }

    fn persona_tab(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        let t = |en: &'static str, fr: &'static str| lang.pick(en, fr);
        let default = Persona::default();
        section(ui, t("IDENTITY", "IDENTITÉ"), |ui| {
            label(ui, t("NAME", "NOM"));
            let name = self.cfg.persona.name.get_or_insert_with(String::new);
            ui.add(egui::TextEdit::singleline(name).desired_width(320.0).hint_text(&default.name));
        });
        section(ui, t("CHARACTER PROMPT", "PROMPT DU PERSONNAGE"), |ui| {
            ui.checkbox(&mut self.custom_prompt, t("Use my own prompt", "Utiliser mon propre prompt"));
            if self.custom_prompt {
                let p = self.cfg.persona.prompt.get_or_insert_with(|| default.system_prompt.clone());
                ui.add(egui::TextEdit::multiline(p).desired_width(f32::INFINITY).desired_rows(12).code_editor());
            } else {
                ui.label(RichText::new(&default.system_prompt).color(MUTED).size(13.0));
            }
            hint(
                ui,
                t(
                    "She always answers in the language you write in.",
                    "Elle répond toujours dans la langue où tu lui écris.",
                ),
            );
        });
    }

    fn safety_tab(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        let t = |en: &'static str, fr: &'static str| lang.pick(en, fr);
        section(ui, t("ACTIONS ON YOUR PC", "ACTIONS SUR TON PC"), |ui| {
            for (mode, title, desc) in [
                (
                    ApprovalMode::Ask,
                    t("Ask first", "Demander d'abord"),
                    t(
                        "She asks before anything that changes your machine.",
                        "Elle demande avant tout ce qui modifie ta machine.",
                    ),
                ),
                (
                    ApprovalMode::Auto,
                    t("Auto", "Auto"),
                    t(
                        "Easy-to-undo actions run directly. Deleting, moving files or running commands still ask.",
                        "Les actions faciles à annuler passent directement. Supprimer, déplacer des fichiers ou lancer des commandes demande toujours.",
                    ),
                ),
                (
                    ApprovalMode::ReadOnly,
                    t("Read only", "Lecture seule"),
                    t("She can look, never change anything.", "Elle peut regarder, jamais rien modifier."),
                ),
            ] {
                ui.radio_value(&mut self.cfg.safety.approval, mode, RichText::new(title).color(TEXT));
                ui.indent(title, |ui| hint(ui, desc));
            }
            ui.add_space(6.0);
            hint(
                ui,
                t(
                    "Text from the model or a web page can ask for an action, never approve one.",
                    "Un texte venant du modèle ou d'une page web peut demander une action, jamais la valider.",
                ),
            );
        });
    }

    fn interface_tab(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        let t = |en: &'static str, fr: &'static str| lang.pick(en, fr);
        section(ui, t("LANGUAGE", "LANGUE"), |ui| {
            ui.radio_value(&mut self.cfg.ui.language, Language::Auto, t("Same as the system", "Comme le système"));
            ui.radio_value(&mut self.cfg.ui.language, Language::Fr, "Français");
            ui.radio_value(&mut self.cfg.ui.language, Language::En, "English");
            hint(
                ui,
                t(
                    "Labels of the chat box and this panel. Her replies follow your language.",
                    "Textes de la bulle et de ce panneau. Ses réponses suivent ta langue.",
                ),
            );
        });
        section(ui, t("FILES", "FICHIERS"), |ui| {
            label(ui, "CONFIG");
            ui.label(RichText::new(self.input.config_path.display().to_string()).color(MUTED).monospace());
            label(ui, t("SHORTCUTS", "RACCOURCIS"));
            hint(
                ui,
                t(
                    "Right-click Capri or type /settings: this panel.  Ctrl+S: save.",
                    "Clic droit sur Capri ou /settings : ce panneau.  Ctrl+S : enregistrer.",
                ),
            );
        });
    }
}

fn engine_name(e: Engine) -> &'static str {
    match e {
        Engine::LlamaServer => "llama.cpp",
        Engine::Ollama => "Ollama",
        Engine::LmStudio => "LM Studio",
        Engine::Openai => "OpenAI",
        Engine::Openrouter => "OpenRouter",
        Engine::Custom => "OpenAI-compatible",
        Engine::ClaudeCode => "Claude Code",
    }
}

fn engine_hint(e: Engine, lang: Language) -> &'static str {
    match e {
        Engine::LlamaServer => {
            lang.pick("llama-server, local or on your own server.", "llama-server, en local ou sur ton serveur.")
        }
        Engine::Ollama => lang.pick("Local models through Ollama.", "Modèles locaux via Ollama."),
        Engine::LmStudio => lang.pick("LM Studio's local server.", "Le serveur local de LM Studio."),
        Engine::Openai => lang.pick("OpenAI's API, needs a key.", "L'API d'OpenAI, avec une clé."),
        Engine::Openrouter => {
            lang.pick("Hundreds of models behind one key.", "Des centaines de modèles avec une seule clé.")
        }
        Engine::Custom => lang.pick(
            "Any server that speaks /v1/chat/completions.",
            "N'importe quel serveur qui parle /v1/chat/completions.",
        ),
        Engine::ClaudeCode => {
            lang.pick("The Claude Code CLI, with your Claude account.", "Le CLI Claude Code, avec ton compte Claude.")
        }
    }
}
