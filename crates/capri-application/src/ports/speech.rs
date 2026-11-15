//! Voice ports. Defined now so the services can depend on them; adapters (whisper-rs, Piper, cloud
//! voices) come in a later version.

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
#[error("speech: {0}")]
pub struct SpeechError(pub String);

/// Mono PCM samples at `rate` Hz.
#[derive(Debug, Clone)]
pub struct Audio {
    pub rate: u32,
    pub samples: Vec<f32>,
}

#[async_trait]
pub trait SpeechToText: Send + Sync {
    async fn transcribe(&self, audio: &Audio) -> Result<String, SpeechError>;
}

#[async_trait]
pub trait TextToSpeech: Send + Sync {
    async fn synthesize(&self, text: &str) -> Result<Audio, SpeechError>;
}
