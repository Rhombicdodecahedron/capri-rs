//! Starts her on the desktop with the hooks the desk needs: opening the settings panel and picking up
//! a saved config without a restart.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::SystemTime;

use capri_application::ports::SkinRepository;
use capri_application::services::{ChatService, DeskService, VoiceService};
use capri_config::Config;
use capri_desk_winit::{DeskHooks, DeskOptions, Reload};
use capri_domain::{Character, CharacterConfig};

use crate::brain;

pub fn character(cfg: &Config) -> CharacterConfig {
    CharacterConfig {
        size: cfg.desk.size,
        roaming: cfg.desk.roaming,
        roam_every: cfg.desk.roam_every_secs,
        sleep_after: cfg.desk.sleep_after_secs,
        ..CharacterConfig::default()
    }
}

pub fn options(cfg: &Config, skins: &dyn SkinRepository) -> DeskOptions {
    DeskOptions {
        skin: pick_skin(skins, &cfg.desk.skin),
        size: cfg.desk.size,
        fps: cfg.desk.fps,
        lang: cfg.ui.language.resolve(sys_locale::get_locale().as_deref()),
        push_to_talk: cfg.voice.push_to_talk.clone(),
    }
}

/// The configured skin if it is installed, otherwise the bundled demo one.
fn pick_skin(skins: &dyn SkinRepository, wanted: &str) -> String {
    if skins.list().iter().any(|s| s.id == wanted) {
        return wanted.to_string();
    }
    tracing::warn!(wanted, "skin not installed, using the demo skin (see assets/skins/README.md)");
    "demo".into()
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub fn run(
    cfg: &Config,
    config_path: PathBuf,
    data: PathBuf,
    skins: Arc<dyn SkinRepository>,
    chat: Arc<ChatService>,
    rt: &tokio::runtime::Runtime,
) -> anyhow::Result<()> {
    let devices = crate::voice::Devices::new(cfg.voice.volume);
    let voice = {
        let _inside = rt.enter();
        VoiceService::new(crate::voice::parts(&cfg.voice, &data, &devices))
    };
    let character_cfg = character(cfg);
    let make_desk: capri_desk_winit::DeskFactory = Box::new(move |scale, monitors| {
        let character = CharacterConfig { size: character_cfg.size * scale, ..character_cfg };
        let probe = capri_desktop_windows::native(monitors);
        DeskService::new(Character::new(character, seed()), Arc::from(probe))
    });

    // one settings window at a time
    let panel: RefCell<Option<Child>> = RefCell::new(None);
    let settings_path = config_path.clone();
    let open_settings = Box::new(move || {
        let mut slot = panel.borrow_mut();
        if slot.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None))) {
            return;
        }
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("capri"));
        match Command::new(exe).arg("--config").arg(&settings_path).arg("settings").spawn() {
            Ok(child) => *slot = Some(child),
            Err(e) => tracing::error!(error = %e, "could not open the settings panel"),
        }
    });

    let mut seen = modified(&config_path);
    let reload_chat = chat.clone();
    let reload_skins = skins.clone();
    let reload_voice = voice.clone();
    let handle = rt.handle().clone();
    let reload = Box::new(move || -> Option<Reload> {
        let now = modified(&config_path);
        if now == seen {
            return None;
        }
        seen = now;
        let cfg = match Config::load_or_create(&config_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "capri.toml has an error, keeping the current settings");
                return None;
            }
        };
        match brain::from_config(&cfg) {
            Ok(b) => reload_chat.set_model(b.model()),
            Err(e) => tracing::warn!(error = %format!("{e:#}"), "keeping the current brain"),
        }
        let chat = reload_chat.clone();
        let persona = cfg.persona.persona();
        handle.spawn(async move { chat.set_persona(persona).await });
        // a local Whisper model takes a moment to load: rebuild the voice off the UI thread
        let (voice, voice_cfg, data, devices) =
            (reload_voice.clone(), cfg.voice.clone(), data.clone(), devices.clone());
        handle.spawn_blocking(move || voice.reconfigure(crate::voice::parts(&voice_cfg, &data, &devices)));
        Some(Reload { options: options(&cfg, reload_skins.as_ref()), character: character(&cfg) })
    });

    let hooks = DeskHooks { open_settings, reload };
    capri_desk_winit::run(options(cfg, skins.as_ref()), make_desk, skins, chat, rt.handle().clone(), hooks, voice)?;
    Ok(())
}

fn seed() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64)
}
