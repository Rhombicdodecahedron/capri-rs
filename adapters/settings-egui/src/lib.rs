//! # capri-settings-egui
//!
//! Her settings panel. It runs as its own small process (`capri settings`), so it never fights the
//! desk window's event loop: it edits `capri.toml`, and the desk picks the file up as soon as it is
//! saved. The brain test goes through [`ConnectionTester`], which the composition root implements
//! with the real adapters.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use capri_config::{Brain, ClaudeCodeSection, Voice};

mod panel;
mod theme;
mod voice_tab;

/// What the TEST button shows.
#[derive(Debug, Clone)]
pub struct TestReport {
    pub models: Vec<String>,
    pub model: String,
    pub reply: String,
    pub first_word_ms: u128,
    pub total_ms: u128,
}

/// Why the TEST failed, with whatever the server did list, so the panel can offer the real names.
#[derive(Debug, Clone, Default)]
pub struct TestFailure {
    pub message: String,
    pub models: Vec<String>,
    /// Set when the chosen model is not among `models`.
    pub missing_model: Option<String>,
}

impl From<String> for TestFailure {
    fn from(message: String) -> Self {
        Self { message, ..Self::default() }
    }
}

#[async_trait]
pub trait ConnectionTester: Send + Sync {
    /// Lists the server's models (when it can) and runs a tiny chat, without saving anything.
    async fn test(&self, brain: Brain, claude_code: ClaudeCodeSection) -> Result<TestReport, TestFailure>;

    /// Records a few seconds from the microphone and returns what was understood.
    async fn test_hearing(&self, voice: Voice) -> Result<String, String>;

    /// Speaks a sample line; returns the time to the first sound in ms.
    async fn test_voice(&self, voice: Voice, line: String) -> Result<u128, String>;

    /// The ElevenLabs voices of this key, as (id, name).
    async fn elevenlabs_voices(&self, voice: Voice) -> Result<Vec<(String, String)>, String>;

    /// Downloads a Whisper model (`tiny`, `base`, `small`, `medium`) and returns its path.
    async fn download_whisper(&self, size: String) -> Result<PathBuf, String>;

    /// Whether this build includes local Whisper.
    fn whisper_available(&self) -> bool;

    /// Where the local Whisper model of these settings is (or would be).
    fn whisper_model_path(&self, voice: &Voice) -> PathBuf;
}

#[derive(Debug, Clone)]
pub struct SkinChoice {
    pub id: String,
    pub name: String,
}

pub struct SettingsInput {
    pub config_path: PathBuf,
    pub skins: Vec<SkinChoice>,
    pub tester: Arc<dyn ConnectionTester>,
    pub runtime: tokio::runtime::Handle,
    pub system_locale: Option<String>,
    /// Test the brain connection as soon as the panel opens.
    pub test_on_open: bool,
}

impl std::fmt::Debug for SettingsInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsInput").field("config_path", &self.config_path).finish_non_exhaustive()
    }
}

/// Opens the panel and blocks until it is closed.
pub fn run(input: SettingsInput) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Capri // settings")
            .with_inner_size([860.0, 680.0])
            .with_min_inner_size([680.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "capri-settings",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let auto = input.test_on_open;
            let panel = panel::Panel::new(input);
            Ok(Box::new(if auto { panel.with_auto_test() } else { panel }))
        }),
    )
    .map_err(|e| e.to_string())
}
