//! Voice ports: ears (speech to text), mouth (text to speech), and the sound card on both sides.

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SpeechError(pub String);

/// Mono PCM samples in `[-1, 1]` at `rate` Hz.
#[derive(Debug, Clone, Default)]
pub struct Audio {
    pub rate: u32,
    pub samples: Vec<f32>,
}

impl Audio {
    pub fn seconds(&self) -> f32 {
        if self.rate == 0 { 0.0 } else { self.samples.len() as f32 / self.rate as f32 }
    }

    /// Root-mean-square level, to tell a real phrase from a silent key press.
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        (self.samples.iter().map(|s| s * s).sum::<f32>() / self.samples.len() as f32).sqrt()
    }
}

/// Driven port: turns recorded speech into text.
/// Adapters: whisper.cpp on this PC, any OpenAI-compatible `/audio/transcriptions` server, ElevenLabs.
#[async_trait]
pub trait SpeechToText: Send + Sync {
    async fn transcribe(&self, audio: &Audio) -> Result<String, SpeechError>;
}

/// Driven port: turns a sentence into her voice.
/// Adapters: ElevenLabs, any OpenAI-compatible `/audio/speech` server (OpenAI, Kokoro, ...).
#[async_trait]
pub trait TextToSpeech: Send + Sync {
    async fn synthesize(&self, text: &str) -> Result<Audio, SpeechError>;
}

/// Driven port: the microphone, recorded while the push-to-talk key is held.
pub trait Microphone: Send + Sync {
    fn start(&self) -> Result<(), SpeechError>;

    /// Stops recording and returns what was heard, mono.
    fn stop(&self) -> Result<Audio, SpeechError>;
}

/// Driven port: the speakers. `play` queues and returns at once.
pub trait Speaker: Send + Sync {
    fn play(&self, audio: Audio) -> Result<(), SpeechError>;

    /// Drops everything queued (she was interrupted).
    fn stop(&self);

    fn is_playing(&self) -> bool;
}
