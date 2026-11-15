//! The VOICE tab: push-to-talk, her ears (speech to text) and her mouth (text to speech).

use capri_config::{SttEngine, TtsEngine};
use eframe::egui::{self, RichText};

use crate::panel::{Job, Panel, hint, label, neon_button, run_job, running, section, status_dot};
use crate::theme::{GREEN, MUTED, RED, TEXT};

fn stt_name(e: SttEngine, t: &dyn Fn(&'static str, &'static str) -> &'static str) -> &'static str {
    match e {
        SttEngine::Off => t("Off", "Désactivée"),
        SttEngine::WhisperLocal => t("Whisper on this PC", "Whisper sur ce PC"),
        SttEngine::Openai => t("Whisper server (OpenAI-compatible)", "Serveur Whisper (compatible OpenAI)"),
        SttEngine::Elevenlabs => "ElevenLabs",
    }
}

fn tts_name(e: TtsEngine, t: &dyn Fn(&'static str, &'static str) -> &'static str) -> &'static str {
    match e {
        TtsEngine::Off => t("Off", "Désactivée"),
        TtsEngine::Elevenlabs => "ElevenLabs",
        TtsEngine::Openai => t("OpenAI-compatible server", "Serveur compatible OpenAI"),
    }
}

/// Two optional key fields: an environment variable, or the key itself.
fn key_fields(
    ui: &mut egui::Ui,
    t: &dyn Fn(&'static str, &'static str) -> &'static str,
    env: &mut Option<String>,
    key: &mut Option<String>,
    id: &str,
) {
    label(ui, t("API KEY", "CLÉ D'API"));
    ui.horizontal(|ui| {
        let k = key.get_or_insert_with(String::new);
        ui.add(
            egui::TextEdit::singleline(k)
                .id_salt(format!("{id}-key"))
                .desired_width(300.0)
                .password(true)
                .hint_text(t("paste the key", "colle la clé")),
        );
        ui.label(RichText::new(t("or variable", "ou variable")).color(MUTED));
        let e = env.get_or_insert_with(String::new);
        ui.add(
            egui::TextEdit::singleline(e)
                .id_salt(format!("{id}-env"))
                .desired_width(160.0)
                .hint_text("ELEVENLABS_API_KEY"),
        );
    });
}

fn result_line<T>(ui: &mut egui::Ui, job: &Job<T>, ok: impl FnOnce(&T) -> String, idle: &str) {
    match job {
        Job::Idle => hint(ui, idle),
        Job::Running => {
            ui.spinner();
        }
        Job::Done(Ok(v)) => {
            ui.horizontal_wrapped(|ui| {
                status_dot(ui, GREEN);
                ui.label(RichText::new(ok(v)).color(GREEN));
            });
        }
        Job::Done(Err(e)) => {
            ui.horizontal_wrapped(|ui| {
                status_dot(ui, RED);
                ui.label(RichText::new(e).color(RED));
            });
        }
    }
}

impl Panel {
    pub(crate) fn voice_tab(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang();
        let t = move |en: &'static str, fr: &'static str| lang.pick(en, fr);
        section(ui, t("PUSH TO TALK", "APPUYER POUR PARLER"), |ui| {
            label(
                ui,
                t("KEY (HOLD IT ANYWHERE, LET GO TO SEND)", "TOUCHE (MAINTIENS-LA N'IMPORTE OÙ, RELÂCHE POUR ENVOYER)"),
            );
            ui.add(egui::TextEdit::singleline(&mut self.cfg.voice.push_to_talk).desired_width(220.0).hint_text("F8"));
            hint(ui, t("Examples: F8, Ctrl+Alt+Space, Shift+F9.", "Exemples : F8, Ctrl+Alt+Space, Shift+F9."));
            ui.add_space(4.0);
            ui.checkbox(
                &mut self.cfg.voice.speak_replies,
                t("Read her replies out loud", "Lire ses réponses à voix haute"),
            );
            label(ui, "VOLUME");
            ui.add(egui::Slider::new(&mut self.cfg.voice.volume, 0.0..=2.0).step_by(0.05));
        });
        self.hearing_section(ui, &t);
        self.speaking_section(ui, &t);
    }

    fn hearing_section(&mut self, ui: &mut egui::Ui, t: &dyn Fn(&'static str, &'static str) -> &'static str) {
        let whisper_ok = self.input.tester.whisper_available();
        let model_path = self.input.tester.whisper_model_path(&self.cfg.voice);
        let mut start_test = false;
        let mut start_download = false;
        section(ui, t("HEARING (SPEECH TO TEXT)", "OUÏE (RECONNAISSANCE VOCALE)"), |ui| {
            let s = &mut self.cfg.voice.stt;
            egui::ComboBox::from_id_salt("stt").width(360.0).selected_text(stt_name(s.engine, t)).show_ui(ui, |ui| {
                for e in [SttEngine::Off, SttEngine::WhisperLocal, SttEngine::Openai, SttEngine::Elevenlabs] {
                    ui.selectable_value(&mut s.engine, e, stt_name(e, t));
                }
            });
            if s.engine == SttEngine::Off {
                return;
            }
            label(ui, t("LANGUAGE (fr, en... EMPTY = DETECT)", "LANGUE (fr, en... VIDE = DÉTECTER)"));
            ui.add(egui::TextEdit::singleline(&mut s.language).desired_width(80.0).hint_text("fr"));
            match s.engine {
                SttEngine::WhisperLocal => {
                    if !whisper_ok {
                        ui.label(RichText::new(t(
                            "This build has no local Whisper. Rebuild with: cargo run --release --features whisper-local (needs CMake).",
                            "Cette version n'a pas Whisper en local. Recompile avec : cargo run --release --features whisper-local (il faut CMake).",
                        )).color(RED));
                    }
                    label(ui, t("MODEL SIZE", "TAILLE DU MODÈLE"));
                    ui.horizontal(|ui| {
                        for (size, _, mb) in capri_speech_whisper_sizes() {
                            ui.selectable_value(&mut s.model_size, size.to_string(), format!("{size} ({mb} MB)"));
                        }
                    });
                    hint(
                        ui,
                        t(
                            "base is fast on any CPU; small understands French better.",
                            "base est rapide sur n'importe quel CPU ; small comprend mieux le français.",
                        ),
                    );
                    label(ui, t("MODEL FILE", "FICHIER DU MODÈLE"));
                    let exists = model_path.is_file();
                    ui.label(
                        RichText::new(model_path.display().to_string())
                            .color(if exists { TEXT } else { MUTED })
                            .monospace()
                            .size(12.0),
                    );
                    if !exists {
                        ui.horizontal(|ui| {
                            if neon_button(ui, t("DOWNLOAD THE MODEL", "TÉLÉCHARGER LE MODÈLE"), false).clicked()
                                && !running(&self.download)
                            {
                                start_download = true;
                            }
                            result_line(ui, &self.download.lock().expect("job lock"), |p| p.display().to_string(), "");
                        });
                    }
                }
                SttEngine::Openai => {
                    label(ui, t("SERVER URL (WITH /v1)", "URL DU SERVEUR (AVEC /v1)"));
                    let url = s.base_url.get_or_insert_with(String::new);
                    ui.add(egui::TextEdit::singleline(url).desired_width(480.0).hint_text("https://api.openai.com/v1"));
                    label(ui, t("MODEL", "MODÈLE"));
                    ui.add(egui::TextEdit::singleline(&mut s.model).desired_width(260.0).hint_text("whisper-1"));
                    key_fields(ui, t, &mut s.api_key_env, &mut s.api_key, "stt");
                }
                SttEngine::Elevenlabs => key_fields(ui, t, &mut s.api_key_env, &mut s.api_key, "stt"),
                SttEngine::Off => {}
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let label = if running(&self.hearing) {
                    t("LISTENING 4 S…", "J'ÉCOUTE 4 S…")
                } else {
                    t("TALK FOR 4 SECONDS", "PARLER 4 SECONDES")
                };
                if neon_button(ui, label, false).clicked() && !running(&self.hearing) {
                    start_test = true;
                }
            });
            result_line(
                ui,
                &self.hearing.lock().expect("job lock"),
                |text| format!("« {text} »"),
                t("Press, then say something.", "Appuie, puis dis quelque chose."),
            );
        });
        let rt = self.input.runtime.clone();
        if start_test {
            self.normalise();
            let (tester, voice) = (self.input.tester.clone(), self.cfg.voice.clone());
            run_job(&rt, ui.ctx(), &self.hearing, async move { tester.test_hearing(voice).await });
        }
        if start_download {
            let (tester, size) = (self.input.tester.clone(), self.cfg.voice.stt.model_size.clone());
            run_job(&rt, ui.ctx(), &self.download, async move { tester.download_whisper(size).await });
        }
    }

    fn speaking_section(&mut self, ui: &mut egui::Ui, t: &dyn Fn(&'static str, &'static str) -> &'static str) {
        let mut start_test = false;
        let mut load_voices = false;
        section(ui, t("HER VOICE (TEXT TO SPEECH)", "SA VOIX (SYNTHÈSE VOCALE)"), |ui| {
            let v = &mut self.cfg.voice.tts;
            egui::ComboBox::from_id_salt("tts").width(360.0).selected_text(tts_name(v.engine, t)).show_ui(ui, |ui| {
                for e in [TtsEngine::Off, TtsEngine::Elevenlabs, TtsEngine::Openai] {
                    ui.selectable_value(&mut v.engine, e, tts_name(e, t));
                }
            });
            match v.engine {
                TtsEngine::Off => return,
                TtsEngine::Elevenlabs => load_voices = eleven_fields(ui, t, v, &self.voices),
                TtsEngine::Openai => {
                    label(ui, t("SERVER URL (WITH /v1)", "URL DU SERVEUR (AVEC /v1)"));
                    let url = v.base_url.get_or_insert_with(String::new);
                    ui.add(egui::TextEdit::singleline(url).desired_width(480.0).hint_text("https://api.openai.com/v1"));
                    label(ui, t("MODEL", "MODÈLE"));
                    ui.add(egui::TextEdit::singleline(&mut v.model).desired_width(260.0).hint_text("gpt-4o-mini-tts"));
                    label(ui, t("VOICE", "VOIX"));
                    ui.add(egui::TextEdit::singleline(&mut v.voice).desired_width(260.0).hint_text("nova"));
                    key_fields(ui, t, &mut v.api_key_env, &mut v.api_key, "tts");
                }
            }
            ui.add_space(6.0);
            if neon_button(
                ui,
                if running(&self.speaking) { t("SPEAKING…", "ELLE PARLE…") } else { t("HEAR HER", "L'ÉCOUTER") },
                false,
            )
            .clicked()
                && !running(&self.speaking)
            {
                start_test = true;
            }
            result_line(
                ui,
                &self.speaking.lock().expect("job lock"),
                |ms| format!("{} {ms} ms", t("first sound after", "premier son après")),
                t("Plays a short line with these settings.", "Joue une petite phrase avec ces réglages."),
            );
        });
        let rt = self.input.runtime.clone();
        if start_test {
            self.normalise();
            let (tester, voice) = (self.input.tester.clone(), self.cfg.voice.clone());
            let line = t(
                "Hey choom. Neural link up, I can hear you loud and clear.",
                "Salut choom. Lien neural établi, je t'entends cinq sur cinq.",
            )
            .to_string();
            run_job(&rt, ui.ctx(), &self.speaking, async move { tester.test_voice(voice, line).await });
        }
        if load_voices {
            self.normalise();
            let (tester, voice) = (self.input.tester.clone(), self.cfg.voice.clone());
            run_job(&rt, ui.ctx(), &self.voices, async move { tester.elevenlabs_voices(voice).await });
        }
    }
}

/// ElevenLabs key, voice and model. Returns true when "load my voices" was clicked.
fn eleven_fields(
    ui: &mut egui::Ui,
    t: &dyn Fn(&'static str, &'static str) -> &'static str,
    v: &mut capri_config::TextToSpeechSection,
    voices: &crate::panel::Slot<Vec<(String, String)>>,
) -> bool {
    let mut load = false;
    key_fields(ui, t, &mut v.api_key_env, &mut v.api_key, "tts");
    label(ui, t("VOICE", "VOIX"));
    let listed: Vec<(String, String)> = match &*voices.lock().expect("job lock") {
        Job::Done(Ok(list)) => list.clone(),
        _ => Vec::new(),
    };
    ui.horizontal(|ui| {
        let current = listed.iter().find(|(id, _)| *id == v.voice).map_or_else(
            || {
                if v.voice.is_empty() {
                    t("Rachel (default)", "Rachel (par défaut)").to_string()
                } else {
                    v.voice.clone()
                }
            },
            |(_, name)| name.clone(),
        );
        egui::ComboBox::from_id_salt("eleven-voice").width(260.0).selected_text(current).show_ui(ui, |ui| {
            for (id, name) in &listed {
                ui.selectable_value(&mut v.voice, id.clone(), name);
            }
        });
        if neon_button(ui, t("LOAD MY VOICES", "CHARGER MES VOIX"), false).clicked() && !running(voices) {
            load = true;
        }
        if let Job::Done(Err(e)) = &*voices.lock().expect("job lock") {
            ui.label(RichText::new(e).color(RED));
        }
    });
    hint(
        ui,
        t(
            "Or paste a voice id from the ElevenLabs voice library.",
            "Ou colle l'identifiant d'une voix de la bibliothèque ElevenLabs.",
        ),
    );
    ui.add(egui::TextEdit::singleline(&mut v.voice).desired_width(300.0).hint_text("voice id"));
    label(ui, t("MODEL", "MODÈLE"));
    ui.horizontal(|ui| {
        for (m, name) in [
            ("", t("fast (flash v2.5)", "rapide (flash v2.5)")),
            ("eleven_multilingual_v2", t("richer (multilingual v2)", "plus riche (multilingual v2)")),
        ] {
            ui.selectable_value(&mut v.model, m.to_string(), name);
        }
    });
    load
}

fn capri_speech_whisper_sizes() -> [(&'static str, &'static str, u32); 4] {
    [
        ("tiny", "ggml-tiny.bin", 75),
        ("base", "ggml-base.bin", 142),
        ("small", "ggml-small.bin", 466),
        ("medium", "ggml-medium.bin", 1500),
    ]
}
