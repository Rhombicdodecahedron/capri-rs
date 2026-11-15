//! Builds the voice adapters from `[voice]`, and the voice tests of the settings panel.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use capri_application::ports::{Microphone, Speaker, SpeechToText, TextToSpeech};
use capri_application::services::VoiceParts;
use capri_audio_cpal::{CpalMicrophone, CpalSpeaker};
use capri_config::{SttEngine, TtsEngine, Voice, resolve_key};
use capri_speech_cloud::{ElevenLabsStt, ElevenLabsTts, OpenAiStt, OpenAiTts};
use capri_speech_whisper::WhisperLocal;

/// The sound card, opened once and shared by every configuration.
#[derive(Clone)]
pub struct Devices {
    pub microphone: Arc<CpalMicrophone>,
    pub speaker: Arc<CpalSpeaker>,
}

impl std::fmt::Debug for Devices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Devices")
    }
}

impl Devices {
    pub fn new(volume: f32) -> Self {
        Self { microphone: Arc::new(CpalMicrophone::default()), speaker: Arc::new(CpalSpeaker::new(volume)) }
    }
}

/// Where downloaded Whisper models go.
pub fn models_dir(data: &Path) -> PathBuf {
    data.join("models")
}

fn whisper_file(size: &str) -> &'static str {
    capri_speech_whisper::MODELS.iter().find(|m| m.0 == size).map_or("ggml-base.bin", |m| m.1)
}

pub fn whisper_model_path(v: &Voice, data: &Path) -> PathBuf {
    v.stt
        .model_path
        .clone()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| models_dir(data).join(whisper_file(&v.stt.model_size)))
}

fn language(v: &Voice) -> Option<String> {
    Some(v.stt.language.trim().to_lowercase()).filter(|l| !l.is_empty() && l != "auto")
}

fn or(s: &str, default: &str) -> String {
    if s.trim().is_empty() { default.to_string() } else { s.trim().to_string() }
}

pub fn stt(v: &Voice, data: &Path) -> Result<Option<Arc<dyn SpeechToText>>, String> {
    let s = &v.stt;
    let key = || resolve_key(s.api_key_env.as_deref(), s.api_key.as_deref());
    Ok(match s.engine {
        SttEngine::Off => None,
        SttEngine::WhisperLocal => {
            Some(Arc::new(WhisperLocal::load(&whisper_model_path(v, data), language(v)).map_err(|e| e.0)?))
        }
        SttEngine::Openai => {
            let url = s
                .base_url
                .clone()
                .filter(|u| !u.trim().is_empty())
                .unwrap_or_else(|| "https://api.openai.com/v1".into());
            let mut a = OpenAiStt::new(url, or(&s.model, "whisper-1"));
            a.api_key = key()?.or_else(|| std::env::var("OPENAI_API_KEY").ok());
            a.language = language(v);
            Some(Arc::new(a))
        }
        SttEngine::Elevenlabs => {
            let k = key()?.ok_or("ElevenLabs needs an API key")?;
            let mut a = ElevenLabsStt::new(k);
            a.model = or(&s.model, "scribe_v1");
            a.language = language(v);
            Some(Arc::new(a))
        }
    })
}

pub fn tts(v: &Voice) -> Result<Option<Arc<dyn TextToSpeech>>, String> {
    let t = &v.tts;
    let key = || resolve_key(t.api_key_env.as_deref(), t.api_key.as_deref());
    Ok(match t.engine {
        TtsEngine::Off => None,
        TtsEngine::Elevenlabs => {
            let k = key()?.ok_or("ElevenLabs needs an API key")?;
            // "Rachel", a stock voice every account has, until one is picked
            let mut a = ElevenLabsTts::new(k, or(&t.voice, "21m00Tcm4TlvDq8ikWAM"));
            a.model = or(&t.model, "eleven_flash_v2_5");
            Some(Arc::new(a))
        }
        TtsEngine::Openai => {
            let url = t
                .base_url
                .clone()
                .filter(|u| !u.trim().is_empty())
                .unwrap_or_else(|| "https://api.openai.com/v1".into());
            let mut a = OpenAiTts::new(url, or(&t.model, "gpt-4o-mini-tts"), or(&t.voice, "nova"));
            a.api_key = key()?.or_else(|| std::env::var("OPENAI_API_KEY").ok());
            Some(Arc::new(a))
        }
    })
}

/// Everything the voice service needs. A part that fails to build is left out and logged, so a bad
/// voice setting never stops her from chatting.
pub fn parts(v: &Voice, data: &Path, devices: &Devices) -> VoiceParts {
    devices.speaker.set_volume(v.volume);
    let stt = stt(v, data).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "speech recognition unavailable");
        None
    });
    let tts = tts(v).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "text to speech unavailable");
        None
    });
    VoiceParts {
        stt,
        tts,
        microphone: Some(devices.microphone.clone() as Arc<dyn Microphone>),
        speaker: Some(devices.speaker.clone() as Arc<dyn Speaker>),
        speak_replies: v.speak_replies,
    }
}

/// Records `seconds` from the microphone and transcribes it.
pub async fn test_hearing(v: &Voice, data: &Path, devices: &Devices, seconds: f32) -> Result<String, String> {
    let stt = stt(v, data)?.ok_or("speech recognition is off")?;
    devices.microphone.start().map_err(|e| e.0)?;
    tokio::time::sleep(std::time::Duration::from_secs_f32(seconds)).await;
    let audio = devices.microphone.stop().map_err(|e| e.0)?;
    if audio.rms() < 0.002 {
        return Err("the microphone heard only silence: check the input device in Windows".into());
    }
    let started = Instant::now();
    let text = stt.transcribe(&audio).await.map_err(|e| e.0)?;
    tracing::info!(ms = started.elapsed().as_millis(), "test transcription");
    Ok(text.trim().to_string())
}

/// Speaks a sample line; returns the time to the first sound, in ms.
pub async fn test_voice(v: &Voice, devices: &Devices, line: &str) -> Result<u128, String> {
    let tts = tts(v)?.ok_or("her voice is off")?;
    let started = Instant::now();
    let audio = tts.synthesize(line).await.map_err(|e| e.0)?;
    let ms = started.elapsed().as_millis();
    devices.speaker.set_volume(v.volume);
    devices.speaker.play(audio).map_err(|e| e.0)?;
    while devices.speaker.is_playing() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Ok(ms)
}

/// Downloads a Whisper model into the data folder; returns its path.
pub async fn download_whisper(size: &str, data: &Path) -> Result<PathBuf, String> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;
    let file = whisper_file(size);
    let dir = models_dir(data);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    let dest = dir.join(file);
    if dest.is_file() {
        return Ok(dest);
    }
    let resp = reqwest::get(capri_speech_whisper::model_url(file)).await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("download failed: {}", resp.status()));
    }
    let part = dest.with_extension("part");
    let mut out = tokio::fs::File::create(&part).await.map_err(|e| e.to_string())?;
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        out.write_all(&chunk.map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())?;
    }
    out.flush().await.map_err(|e| e.to_string())?;
    drop(out);
    tokio::fs::rename(&part, &dest).await.map_err(|e| e.to_string())?;
    tracing::info!(model = %dest.display(), "Whisper model downloaded");
    Ok(dest)
}
