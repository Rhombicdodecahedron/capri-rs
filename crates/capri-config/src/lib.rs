//! `capri.toml`: everything the user can set. An API key can be read from an environment variable
//! (preferred) or written in the file, which lives in the user's own profile folder.

use std::path::{Path, PathBuf};

use anyhow::Context;
use capri_domain::{ApprovalMode, Language, Persona};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub brain: Brain,
    pub claude_code: ClaudeCodeSection,
    pub desk: Desk,
    pub persona: PersonaSection,
    pub safety: Safety,
    pub memory: MemorySection,
    pub ui: Ui,
    pub voice: Voice,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Voice {
    /// Hold this key anywhere to talk to her, e.g. `F8`, `Ctrl+Alt+Space`.
    pub push_to_talk: String,
    /// Read her replies out loud (needs a text-to-speech engine).
    pub speak_replies: bool,
    pub volume: f32,
    pub stt: SpeechToTextSection,
    pub tts: TextToSpeechSection,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            push_to_talk: "F8".into(),
            speak_replies: true,
            volume: 1.0,
            stt: SpeechToTextSection::default(),
            tts: TextToSpeechSection::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SttEngine {
    #[default]
    Off,
    /// whisper.cpp on this PC (needs a build with `--features whisper-local`).
    WhisperLocal,
    /// Any `/v1/audio/transcriptions` server: a self-hosted Whisper, OpenAI...
    Openai,
    Elevenlabs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TtsEngine {
    #[default]
    Off,
    Elevenlabs,
    /// Any `/v1/audio/speech` server: OpenAI, Kokoro-FastAPI...
    Openai,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeechToTextSection {
    pub engine: SttEngine,
    /// `fr`, `en`... Empty: detect it.
    pub language: String,
    /// whisper-local: path to a ggml model file (empty: the downloaded one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_path: Option<PathBuf>,
    /// whisper-local: tiny | base | small | medium, used to find or download the model.
    pub model_size: String,
    /// openai: server URL with /v1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Model name on the server (`whisper-1`, `scribe_v1`...); empty: the engine's default.
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextToSpeechSection {
    pub engine: TtsEngine,
    /// ElevenLabs voice id, or the OpenAI-style voice name (`nova`, `af_bella`...).
    pub voice: String,
    /// Empty: the engine's default (`eleven_flash_v2_5`, `gpt-4o-mini-tts`).
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

/// An API key from its environment variable first, then from the file.
pub fn resolve_key(env: Option<&str>, inline: Option<&str>) -> Result<Option<String>, String> {
    if let Some(var) = env.filter(|v| !v.trim().is_empty()) {
        return std::env::var(var).map(Some).map_err(|_| format!("the environment variable {var} is not set"));
    }
    Ok(inline.filter(|k| !k.trim().is_empty()).map(str::to_string))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    /// auto | en | fr
    pub language: Language,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Engine {
    Ollama,
    LmStudio,
    #[serde(rename = "llama-cpp", alias = "llama-server", alias = "llamacpp")]
    LlamaServer,
    Openai,
    Openrouter,
    /// Any other OpenAI-compatible server: set `base_url`.
    Custom,
    ClaudeCode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Brain {
    pub engine: Engine,
    /// Empty: ask the server which model it serves (the right choice for llama-server).
    pub model: String,
    /// Overrides the engine's default URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Name of the environment variable holding the API key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// The API key itself, if you prefer it in the file. `api_key_env` wins when both are set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Let reasoning models (Qwen3, DeepSeek-R1...) think before answering. Slower first word.
    pub thinking: bool,
    /// Let the model call her tools (music, apps, the web, driving windows). llama-server needs
    /// `--jinja` for this; servers that refuse are detected and get plain chat.
    pub tools: bool,
}

impl Default for Brain {
    fn default() -> Self {
        Self {
            engine: Engine::Ollama,
            model: "qwen3:8b".into(),
            base_url: None,
            api_key_env: None,
            api_key: None,
            temperature: Some(0.8),
            max_tokens: Some(400),
            thinking: false,
            tools: true,
        }
    }
}

impl Brain {
    pub fn default_url(&self) -> &'static str {
        match self.engine {
            Engine::Ollama => "http://localhost:11434/v1",
            Engine::LmStudio => "http://localhost:1234/v1",
            Engine::LlamaServer => "http://localhost:8080/v1",
            Engine::Openai => "https://api.openai.com/v1",
            Engine::Openrouter => "https://openrouter.ai/api/v1",
            Engine::Custom | Engine::ClaudeCode => "",
        }
    }

    pub fn url(&self) -> String {
        self.base_url.clone().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| self.default_url().to_string())
    }

    pub fn default_key_env(&self) -> Option<&'static str> {
        match self.engine {
            Engine::Openai => Some("OPENAI_API_KEY"),
            Engine::Openrouter => Some("OPENROUTER_API_KEY"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeCodeSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Desk {
    pub skin: String,
    pub size: f32,
    pub roaming: bool,
    pub roam_every_secs: (f32, f32),
    pub sleep_after_secs: f32,
    pub fps: u32,
}

impl Default for Desk {
    fn default() -> Self {
        Self {
            skin: "capri".into(),
            size: 240.0,
            roaming: true,
            roam_every_secs: (18.0, 40.0),
            sleep_after_secs: 300.0,
            fps: 60,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PersonaSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Replaces her default character prompt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

impl PersonaSection {
    pub fn persona(&self) -> Persona {
        let mut p = Persona::default();
        if let Some(n) = &self.name {
            p.name.clone_from(n);
        }
        if let Some(s) = &self.prompt {
            p.system_prompt.clone_from(s);
        }
        p
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Safety {
    pub approval: ApprovalMode,
    /// Let her drive other apps' windows (read their buttons, click, type, press keys).
    pub app_control: bool,
    /// Words in a window's title or program name that keep her eyes and hands off it, on top of the
    /// built-in list (password managers, banking, private browsing, sign-in prompts).
    pub never_touch: Vec<String>,
}

impl Default for Safety {
    fn default() -> Self {
        Self { approval: ApprovalMode::Ask, app_control: true, never_touch: Vec::new() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MemorySection {
    /// Defaults to `memory.sqlite` in the data folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

impl Config {
    /// Writes the settings to `path`, keeping the comments and layout of the existing file: only
    /// values change, keys that were cleared are removed, new keys are appended to their table.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let existing = std::fs::read_to_string(path).unwrap_or_else(|_| DEFAULT_CONFIG.to_string());
        let text = self.merge_into(&existing)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// The text of `existing` with this config's values merged in.
    pub fn merge_into(&self, existing: &str) -> anyhow::Result<String> {
        let mut doc: toml_edit::DocumentMut = existing.parse().context("the current capri.toml is not valid TOML")?;
        let new = toml_edit::ser::to_document(self).context("serialising settings")?;
        merge_table(doc.as_table_mut(), new.as_table());
        Ok(doc.to_string())
    }

    /// Reads `path`, or writes a commented default there if it does not exist yet.
    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        if path.is_file() {
            let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            return toml::from_str(&text).with_context(|| format!("parsing {}", path.display()));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, DEFAULT_CONFIG).with_context(|| format!("writing {}", path.display()))?;
        tracing::info!(path = %path.display(), "wrote a default config");
        Ok(toml::from_str(DEFAULT_CONFIG)?)
    }
}

fn as_table(item: &toml_edit::Item) -> Option<toml_edit::Table> {
    match item {
        toml_edit::Item::Table(t) => Some(t.clone()),
        toml_edit::Item::Value(toml_edit::Value::InlineTable(t)) => Some(t.clone().into_table()),
        _ => None,
    }
}

fn merge_table(dst: &mut toml_edit::Table, src: &toml_edit::Table) {
    let stale: Vec<String> = dst.iter().map(|(k, _)| k.to_string()).filter(|k| !src.contains_key(k)).collect();
    for k in stale {
        dst.remove(&k);
    }
    for (k, item) in src {
        if let Some(mut s) = as_table(item) {
            match dst.get_mut(k) {
                Some(toml_edit::Item::Table(d)) => merge_table(d, &s),
                _ if s.is_empty() => {}
                _ => {
                    for (_, v) in s.iter_mut() {
                        if let Some(v) = v.as_value_mut() {
                            v.decor_mut().clear();
                        }
                    }
                    dst.insert(k, toml_edit::Item::Table(s));
                }
            }
            continue;
        }
        if let (Some(toml_edit::Item::Value(d)), toml_edit::Item::Value(s)) = (dst.get_mut(k), item) {
            let decor = d.decor().clone();
            *d = s.clone();
            *d.decor_mut() = decor;
        } else {
            let mut item = item.clone();
            if let toml_edit::Item::Value(v) = &mut item {
                v.decor_mut().clear();
            }
            dst.insert(k, item);
        }
    }
}

pub const DEFAULT_CONFIG: &str = r#"# Capri settings. She picks up changes as soon as the file is saved.

# Right-click Capri (or type /settings) to change all of this from her settings panel.
[brain]
# ollama | lm-studio | llama-cpp | openai | openrouter | custom | claude-code
engine = "ollama"
# empty = ask the server which model it serves (best for llama.cpp)
model = "qwen3:8b"
# base_url = "https://llamacpp.example.com/v1"   # your server, with /v1
# api_key_env = "LLAMA_API_KEY"                  # read the key from this environment variable
# api_key = "..."                                # or put it here (stored in plain text)
temperature = 0.8
max_tokens = 400
thinking = false
# let the model use her tools (llama-server needs --jinja)
tools = true

[claude_code]
# executable = "C:/Users/you/AppData/Roaming/npm/claude.cmd"
# model = "sonnet"

[desk]
# folder name under assets/skins; falls back to "demo" if it is missing
skin = "capri"
size = 240
roaming = true
roam_every_secs = [18, 40]
sleep_after_secs = 300
fps = 60

[persona]
# name = "Capri"
# prompt = "You are ..."

[safety]
# ask | auto | read_only
approval = "ask"
# drive other apps: read their buttons, click, type (never password managers, banking, private tabs)
app_control = true
# more windows to keep off limits, by words in their title or program name
never_touch = []

[ui]
# auto | en | fr  (she always answers in the language you write in)
language = "auto"

[voice]
# hold this key anywhere to talk to her
push_to_talk = "F8"
speak_replies = true
volume = 1.0

[voice.stt]
# off | whisper-local | openai | elevenlabs
engine = "off"
language = "fr"
model_size = "base"
model = ""

[voice.tts]
# off | elevenlabs | openai
engine = "off"
voice = ""
model = ""
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_parses_and_matches_defaults() {
        let c: Config = toml::from_str(DEFAULT_CONFIG).unwrap();
        assert_eq!(c.brain.engine, Engine::Ollama);
        assert_eq!(c.desk.skin, "capri");
        assert_eq!(c.safety, Safety::default());
        assert!(c.brain.tools);
    }

    #[test]
    fn save_keeps_comments_and_changes_values() {
        let mut c: Config = toml::from_str(DEFAULT_CONFIG).unwrap();
        c.brain.engine = Engine::LlamaServer;
        c.brain.base_url = Some("https://llama.example.com/v1".into());
        c.brain.model = String::new();
        c.ui.language = Language::Fr;
        let out = c.merge_into(DEFAULT_CONFIG).unwrap();
        assert!(out.contains("# Capri settings") && out.contains("# Right-click Capri"));
        assert!(out.contains("engine = \"llama-cpp\""));
        assert!(out.contains("base_url = \"https://llama.example.com/v1\""));
        assert!(out.contains("language = \"fr\""));
        let back: Config = toml::from_str(&out).unwrap();
        assert_eq!(back.brain.engine, Engine::LlamaServer);
        assert_eq!(back.brain.model, "");
    }

    #[test]
    fn cleared_keys_are_removed() {
        let with_key = "[brain]\napi_key = \"secret\" # mine\nengine = \"ollama\"\n";
        let c: Config = toml::from_str(with_key).unwrap();
        let mut c2 = c.clone();
        c2.brain.api_key = None;
        let out = c2.merge_into(with_key).unwrap();
        assert!(!out.contains("secret"));
    }

    #[test]
    fn empty_file_is_all_defaults() {
        let c: Config = toml::from_str("").unwrap();
        assert!((c.desk.size - 240.0).abs() < f32::EPSILON);
    }
}
