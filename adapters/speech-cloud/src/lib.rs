//! Speech over HTTP.
//!
//! - [`OpenAiStt`]: `POST {base}/audio/transcriptions`, spoken by OpenAI, faster-whisper servers
//!   (speaches), LocalAI and most self-hosted Whisper servers.
//! - [`OpenAiTts`]: `POST {base}/audio/speech`, spoken by OpenAI, Kokoro-FastAPI, speaches...
//! - [`ElevenLabsStt`] and [`ElevenLabsTts`]: ElevenLabs' own API.

use std::time::Duration;

use async_trait::async_trait;
use capri_application::ports::{Audio, SpeechError, SpeechToText, TextToSpeech};
use serde::Deserialize;

pub mod wav;

pub const ELEVENLABS_URL: &str = "https://api.elevenlabs.io/v1";

fn err(e: impl std::fmt::Display) -> SpeechError {
    SpeechError(e.to_string())
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_default()
}

async fn ok(resp: reqwest::Response, what: &str) -> Result<reqwest::Response, SpeechError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body: String = resp.text().await.unwrap_or_default().chars().take(300).collect();
    Err(SpeechError(match status.as_u16() {
        401 | 403 => format!("{what}: the key was refused ({status})"),
        _ => format!("{what}: {status} {body}"),
    }))
}

#[derive(Deserialize)]
struct Transcript {
    text: String,
}

#[derive(Debug, Clone)]
pub struct OpenAiStt {
    pub base_url: String,
    pub api_key: Option<String>,
    /// `whisper-1` on OpenAI; the model name your server expects otherwise.
    pub model: String,
    /// ISO 639-1 code (`fr`, `en`), `None` to let the model detect it.
    pub language: Option<String>,
    http: reqwest::Client,
}

impl OpenAiStt {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), api_key: None, model: model.into(), language: None, http: client() }
    }
}

#[async_trait]
impl SpeechToText for OpenAiStt {
    async fn transcribe(&self, audio: &Audio) -> Result<String, SpeechError> {
        let file = reqwest::multipart::Part::bytes(wav::encode(audio))
            .file_name("speech.wav")
            .mime_str("audio/wav")
            .map_err(err)?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", file)
            .text("model", self.model.clone())
            .text("response_format", "json");
        if let Some(l) = self.language.as_ref().filter(|l| !l.is_empty()) {
            form = form.text("language", l.clone());
        }
        let url = format!("{}/audio/transcriptions", self.base_url.trim_end_matches('/'));
        let mut req = self.http.post(url).multipart(form);
        if let Some(k) = &self.api_key {
            req = req.bearer_auth(k);
        }
        let resp = ok(req.send().await.map_err(err)?, "speech recognition").await?;
        Ok(resp.json::<Transcript>().await.map_err(err)?.text)
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiTts {
    pub base_url: String,
    pub api_key: Option<String>,
    /// `gpt-4o-mini-tts` or `tts-1` on OpenAI, `kokoro` on Kokoro-FastAPI...
    pub model: String,
    pub voice: String,
    pub speed: f32,
    http: reqwest::Client,
}

impl OpenAiTts {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, voice: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: None,
            model: model.into(),
            voice: voice.into(),
            speed: 1.0,
            http: client(),
        }
    }
}

#[async_trait]
impl TextToSpeech for OpenAiTts {
    async fn synthesize(&self, text: &str) -> Result<Audio, SpeechError> {
        let body = serde_json::json!({
            "model": self.model,
            "input": text,
            "voice": self.voice,
            "response_format": "wav",
            "speed": self.speed,
        });
        let url = format!("{}/audio/speech", self.base_url.trim_end_matches('/'));
        let mut req = self.http.post(url).json(&body);
        if let Some(k) = &self.api_key {
            req = req.bearer_auth(k);
        }
        let resp = ok(req.send().await.map_err(err)?, "text to speech").await?;
        let bytes = resp.bytes().await.map_err(err)?;
        wav::decode(&bytes)
    }
}

#[derive(Debug, Clone)]
pub struct ElevenLabsStt {
    pub api_key: String,
    /// `scribe_v1`.
    pub model: String,
    pub language: Option<String>,
    pub base_url: String,
    http: reqwest::Client,
}

impl ElevenLabsStt {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "scribe_v1".into(),
            language: None,
            base_url: ELEVENLABS_URL.into(),
            http: client(),
        }
    }
}

#[async_trait]
impl SpeechToText for ElevenLabsStt {
    async fn transcribe(&self, audio: &Audio) -> Result<String, SpeechError> {
        let file = reqwest::multipart::Part::bytes(wav::encode(audio))
            .file_name("speech.wav")
            .mime_str("audio/wav")
            .map_err(err)?;
        let mut form = reqwest::multipart::Form::new().part("file", file).text("model_id", self.model.clone());
        if let Some(l) = self.language.as_ref().filter(|l| !l.is_empty()) {
            form = form.text("language_code", l.clone());
        }
        let url = format!("{}/speech-to-text", self.base_url.trim_end_matches('/'));
        let req = self.http.post(url).header("xi-api-key", &self.api_key).multipart(form);
        let resp = ok(req.send().await.map_err(err)?, "ElevenLabs speech recognition").await?;
        Ok(resp.json::<Transcript>().await.map_err(err)?.text)
    }
}

/// ElevenLabs returns raw 16-bit PCM at this rate with `output_format=pcm_22050`.
const ELEVEN_RATE: u32 = 22_050;

#[derive(Debug, Clone)]
pub struct ElevenLabsTts {
    pub api_key: String,
    pub voice_id: String,
    /// `eleven_flash_v2_5` is the fast one; `eleven_multilingual_v2` sounds richer.
    pub model: String,
    pub base_url: String,
    http: reqwest::Client,
}

impl ElevenLabsTts {
    pub fn new(api_key: impl Into<String>, voice_id: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            voice_id: voice_id.into(),
            model: "eleven_flash_v2_5".into(),
            base_url: ELEVENLABS_URL.into(),
            http: client(),
        }
    }
}

#[async_trait]
impl TextToSpeech for ElevenLabsTts {
    async fn synthesize(&self, text: &str) -> Result<Audio, SpeechError> {
        let url = format!(
            "{}/text-to-speech/{}?output_format=pcm_{ELEVEN_RATE}",
            self.base_url.trim_end_matches('/'),
            self.voice_id
        );
        let body = serde_json::json!({ "text": text, "model_id": self.model });
        let req = self.http.post(url).header("xi-api-key", &self.api_key).json(&body);
        let resp = ok(req.send().await.map_err(err)?, "ElevenLabs voice").await?;
        let bytes = resp.bytes().await.map_err(err)?;
        Ok(Audio { rate: ELEVEN_RATE, samples: wav::pcm16_le(&bytes) })
    }
}

/// A voice offered by the account, for the settings list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voice {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize)]
struct Voices {
    voices: Vec<VoiceEntry>,
}

#[derive(Deserialize)]
struct VoiceEntry {
    voice_id: String,
    name: String,
}

/// The voices available to this ElevenLabs key.
pub async fn elevenlabs_voices(api_key: &str) -> Result<Vec<Voice>, SpeechError> {
    let req = client().get(format!("{ELEVENLABS_URL}/voices")).header("xi-api-key", api_key);
    let resp = ok(req.send().await.map_err(err)?, "ElevenLabs voices").await?;
    let v: Voices = resp.json().await.map_err(err)?;
    Ok(v.voices.into_iter().map(|v| Voice { id: v.voice_id, name: v.name }).collect())
}
