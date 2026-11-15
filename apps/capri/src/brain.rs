//! Builds the brain adapter from the config, and tests a connection for the settings panel and
//! `capri brain`.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use async_trait::async_trait;
use capri_application::ports::LanguageModel;
use capri_config::{Brain as BrainCfg, ClaudeCodeSection, Config, Engine, Voice};
use capri_domain::Message;
use capri_llm_claude_code::{ClaudeCode, ClaudeCodeConfig};
use capri_llm_openai::{OpenAiCompatible, OpenAiConfig};
use capri_settings_egui::{ConnectionTester, TestFailure, TestReport};
use futures::StreamExt;

/// The configured brain. The concrete OpenAI-compatible adapter is kept because listing models is
/// not part of the `LanguageModel` port: only the settings test needs it.
pub enum Brain {
    OpenAi(Arc<OpenAiCompatible>),
    ClaudeCode(Arc<ClaudeCode>),
}

impl std::fmt::Debug for Brain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.model().name())
    }
}

impl Brain {
    pub fn model(&self) -> Arc<dyn LanguageModel> {
        match self {
            Brain::OpenAi(m) => m.clone(),
            Brain::ClaudeCode(m) => m.clone(),
        }
    }
}

/// API key: an explicit environment variable first, then the key in the file, then the engine's
/// usual variable (`OPENAI_API_KEY`...) if it happens to be set.
fn api_key(b: &BrainCfg) -> anyhow::Result<Option<String>> {
    if let Some(var) = b.api_key_env.as_deref().filter(|v| !v.trim().is_empty()) {
        return std::env::var(var).map(Some).with_context(|| format!("the environment variable {var} is not set"));
    }
    if let Some(k) = b.api_key.clone().filter(|k| !k.trim().is_empty()) {
        return Ok(Some(k));
    }
    Ok(b.default_key_env().and_then(|v| std::env::var(v).ok()))
}

pub fn build(b: &BrainCfg, cc: &ClaudeCodeSection) -> anyhow::Result<Brain> {
    if b.engine == Engine::ClaudeCode {
        let cfg = ClaudeCodeConfig {
            executable: cc.executable.clone(),
            model: cc.model.clone(),
            ..ClaudeCodeConfig::default()
        };
        return Ok(Brain::ClaudeCode(Arc::new(ClaudeCode::new(cfg))));
    }
    let base_url = b.url();
    anyhow::ensure!(!base_url.is_empty(), "this engine needs a server URL");
    let mut oc = if b.engine == Engine::LlamaServer {
        OpenAiConfig::llama_server(base_url)
    } else {
        OpenAiConfig::new(base_url)
    };
    oc.model = Some(b.model.clone()).filter(|m| !m.trim().is_empty());
    oc.api_key = api_key(b)?;
    oc.temperature = b.temperature.map(|t| t as f32);
    oc.max_tokens = b.max_tokens;
    oc.thinking = b.thinking;
    oc.tools = b.tools;
    Ok(Brain::OpenAi(Arc::new(OpenAiCompatible::new(oc))))
}

pub fn from_config(cfg: &Config) -> anyhow::Result<Brain> {
    build(&cfg.brain, &cfg.claude_code)
}

/// Lists models (when the server can) and times a tiny chat.
pub async fn test(brain: &Brain) -> Result<TestReport, TestFailure> {
    let (models, model) = match brain {
        Brain::OpenAi(m) => {
            let models = m.list_models().await.map_err(|e| TestFailure::from(e.to_string()))?;
            let model = m.model().await.map_err(|e| TestFailure {
                message: e.to_string(),
                models: models.clone(),
                missing_model: None,
            })?;
            // servers that host several models (llama-server router, llama-swap, Ollama) reject unknown names
            if !models.is_empty() && !models.contains(&model) {
                return Err(TestFailure {
                    message: format!("model '{model}' is not on the server"),
                    models,
                    missing_model: Some(model),
                });
            }
            (models, model)
        }
        Brain::ClaudeCode(m) => (Vec::new(), m.name()),
    };
    let fail = |e: String| TestFailure { message: e, models: models.clone(), missing_model: None };
    let prompt = [
        Message::system("You are a terse assistant used for a connection test."),
        Message::user("Reply with one short friendly sentence, in French and English."),
    ];
    let start = Instant::now();
    let mut stream = brain.model().stream(&prompt).await.map_err(|e| fail(e.to_string()))?;
    let mut reply = String::new();
    let mut first = None;
    while let Some(piece) = stream.next().await {
        let piece = piece.map_err(|e| fail(e.to_string()))?;
        if first.is_none() && !piece.trim().is_empty() {
            first = Some(start.elapsed().as_millis());
        }
        reply.push_str(&piece);
    }
    let total_ms = start.elapsed().as_millis();
    if reply.trim().is_empty() {
        return Err(fail("the model answered with nothing".into()));
    }
    Ok(TestReport { models, model, reply, first_word_ms: first.unwrap_or(total_ms), total_ms })
}

/// What the settings panel calls to try things out with the real adapters.
#[derive(Debug)]
pub struct Tester {
    data: std::path::PathBuf,
    devices: std::sync::OnceLock<crate::voice::Devices>,
}

impl Tester {
    pub fn new(data: std::path::PathBuf) -> Self {
        Self { data, devices: std::sync::OnceLock::new() }
    }

    fn devices(&self, volume: f32) -> &crate::voice::Devices {
        self.devices.get_or_init(|| crate::voice::Devices::new(volume))
    }
}

#[async_trait]
impl ConnectionTester for Tester {
    async fn test(&self, brain: BrainCfg, claude_code: ClaudeCodeSection) -> Result<TestReport, TestFailure> {
        let b = build(&brain, &claude_code).map_err(|e| TestFailure::from(format!("{e:#}")))?;
        test(&b).await
    }

    async fn test_hearing(&self, voice: Voice) -> Result<String, String> {
        crate::voice::test_hearing(&voice, &self.data, self.devices(voice.volume), 4.0).await
    }

    async fn test_voice(&self, voice: Voice, line: String) -> Result<u128, String> {
        crate::voice::test_voice(&voice, self.devices(voice.volume), &line).await
    }

    async fn elevenlabs_voices(&self, voice: Voice) -> Result<Vec<(String, String)>, String> {
        let key = capri_config::resolve_key(voice.tts.api_key_env.as_deref(), voice.tts.api_key.as_deref())?
            .ok_or("paste your ElevenLabs key first")?;
        let list = capri_speech_cloud::elevenlabs_voices(&key).await.map_err(|e| e.0)?;
        Ok(list.into_iter().map(|v| (v.id, v.name)).collect())
    }

    async fn download_whisper(&self, size: String) -> Result<std::path::PathBuf, String> {
        crate::voice::download_whisper(&size, &self.data).await
    }

    fn whisper_available(&self) -> bool {
        capri_speech_whisper::AVAILABLE
    }

    fn whisper_model_path(&self, voice: &Voice) -> std::path::PathBuf {
        crate::voice::whisper_model_path(voice, &self.data)
    }
}
