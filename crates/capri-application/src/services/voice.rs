//! Use case: talk to her and hear her answer.
//!
//! Hold the push-to-talk key: the microphone records; let go: the recording is transcribed and the
//! text goes to the chat like a typed line. Her reply is cut into sentences as it streams in, and each
//! sentence is synthesised and queued on the speakers while the next one is still being written.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use capri_domain::SentenceSplitter;
use tokio::sync::mpsc;

use crate::ports::{Microphone, Speaker, SpeechError, SpeechToText, TextToSpeech};

/// The voice adapters in use; any of them may be missing (voice off, no sound card).
#[derive(Clone, Default)]
pub struct VoiceParts {
    pub stt: Option<Arc<dyn SpeechToText>>,
    pub tts: Option<Arc<dyn TextToSpeech>>,
    pub microphone: Option<Arc<dyn Microphone>>,
    pub speaker: Option<Arc<dyn Speaker>>,
    /// Speak replies out loud (with a working text-to-speech).
    pub speak_replies: bool,
}

impl std::fmt::Debug for VoiceParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceParts")
            .field("stt", &self.stt.is_some())
            .field("tts", &self.tts.is_some())
            .field("microphone", &self.microphone.is_some())
            .field("speaker", &self.speaker.is_some())
            .field("speak_replies", &self.speak_replies)
            .finish()
    }
}

/// What came out of a push-to-talk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heard {
    Text(String),
    /// Too short or too quiet to be speech.
    Nothing,
}

/// Recordings shorter or quieter than this are a key tap, not a phrase.
const MIN_SECONDS: f32 = 0.35;
const MIN_RMS: f32 = 0.004;

pub struct VoiceService {
    parts: RwLock<VoiceParts>,
    splitter: Mutex<SentenceSplitter>,
    lines: mpsc::UnboundedSender<(u64, String)>,
    /// Bumped on every interruption: queued sentences from an older reply are dropped.
    generation: Arc<AtomicUsize>,
    /// Sentences handed to the worker and not yet sent to the speakers.
    pending: Arc<AtomicUsize>,
}

impl std::fmt::Debug for VoiceService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceService")
            .field("parts", &*self.parts.read().unwrap_or_else(std::sync::PoisonError::into_inner))
            .finish_non_exhaustive()
    }
}

impl VoiceService {
    /// Must be called inside a tokio runtime: it starts the worker that voices sentences in order.
    pub fn new(parts: VoiceParts) -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        let service = Arc::new(Self {
            parts: RwLock::new(parts),
            splitter: Mutex::new(SentenceSplitter::default()),
            lines: tx,
            generation: Arc::new(AtomicUsize::new(0)),
            pending: Arc::new(AtomicUsize::new(0)),
        });
        tokio::spawn(speak_worker(Arc::downgrade(&service), rx));
        service
    }

    fn parts(&self) -> VoiceParts {
        self.parts.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// New settings saved: swap the adapters live.
    pub fn reconfigure(&self, parts: VoiceParts) {
        self.interrupt();
        *self.parts.write().unwrap_or_else(std::sync::PoisonError::into_inner) = parts;
    }

    pub fn can_listen(&self) -> bool {
        let p = self.parts();
        p.stt.is_some() && p.microphone.is_some()
    }

    /// Push-to-talk pressed: she stops talking and listens.
    pub fn listen(&self) -> Result<(), SpeechError> {
        self.interrupt();
        let mic = self.parts().microphone.ok_or_else(|| SpeechError("no microphone".into()))?;
        mic.start()
    }

    /// Push-to-talk released: what did the user say?
    pub async fn heard(&self) -> Result<Heard, SpeechError> {
        let p = self.parts();
        let mic = p.microphone.ok_or_else(|| SpeechError("no microphone".into()))?;
        let audio = mic.stop()?;
        if audio.seconds() < MIN_SECONDS || audio.rms() < MIN_RMS {
            return Ok(Heard::Nothing);
        }
        let stt = p.stt.ok_or_else(|| SpeechError("speech recognition is off".into()))?;
        let text = stt.transcribe(&audio).await?;
        let text = text.trim();
        Ok(if text.is_empty() || is_noise(text) { Heard::Nothing } else { Heard::Text(text.to_string()) })
    }

    /// A piece of her streamed reply: complete sentences go to the voice queue.
    pub fn say_streamed(&self, delta: &str) {
        if !self.will_speak() {
            return;
        }
        let sentences = self.splitter.lock().unwrap_or_else(std::sync::PoisonError::into_inner).feed(delta);
        for s in sentences {
            self.queue(s);
        }
    }

    /// The reply is complete: voice whatever is left.
    pub fn say_done(&self) {
        let rest = self.splitter.lock().unwrap_or_else(std::sync::PoisonError::into_inner).finish();
        if let (true, Some(rest)) = (self.will_speak(), rest) {
            self.queue(rest);
        }
    }

    /// Shuts her up: drops queued sentences and what is playing.
    pub fn interrupt(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.splitter.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
        if let Some(sp) = self.parts().speaker {
            sp.stop();
        }
    }

    /// True while she is (about to be) speaking out loud: drives the mouth.
    pub fn speaking(&self) -> bool {
        self.pending.load(Ordering::SeqCst) > 0 || self.parts().speaker.is_some_and(|s| s.is_playing())
    }

    fn will_speak(&self) -> bool {
        let p = self.parts();
        p.speak_replies && p.tts.is_some() && p.speaker.is_some()
    }

    fn queue(&self, sentence: String) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        let generation = self.generation.load(Ordering::SeqCst) as u64;
        if self.lines.send((generation, sentence)).is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Voices queued sentences one at a time, in order, so the speakers get them in sequence.
async fn speak_worker(service: std::sync::Weak<VoiceService>, mut rx: mpsc::UnboundedReceiver<(u64, String)>) {
    while let Some((generation, sentence)) = rx.recv().await {
        let Some(s) = service.upgrade() else { return };
        let current = s.generation.load(Ordering::SeqCst) as u64 == generation;
        let p = s.parts();
        if let (true, Some(tts), Some(speaker)) = (current, p.tts, p.speaker) {
            match tts.synthesize(&sentence).await {
                // interrupted while synthesising: drop it
                Ok(audio) if s.generation.load(Ordering::SeqCst) as u64 == generation => {
                    if let Err(e) = speaker.play(audio) {
                        tracing::warn!(error = %e, "could not play her voice");
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "text to speech failed"),
            }
        }
        s.pending.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Whisper's usual hallucinations on silence or breathing.
fn is_noise(text: &str) -> bool {
    let t = text.trim().trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    t.is_empty()
        || matches!(
            t.as_str(),
            "you"
                | "thank you"
                | "thanks for watching"
                | "merci"
                | "sous-titrage st' 501"
                | "sous-titres réalisés par la communauté d'amara.org"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::Audio;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicBool;

    struct Echo;

    #[async_trait]
    impl TextToSpeech for Echo {
        async fn synthesize(&self, text: &str) -> Result<Audio, SpeechError> {
            Ok(Audio { rate: 1000, samples: vec![0.1; text.len()] })
        }
    }

    #[async_trait]
    impl SpeechToText for Echo {
        async fn transcribe(&self, audio: &Audio) -> Result<String, SpeechError> {
            Ok(format!("{} samples", audio.samples.len()))
        }
    }

    #[derive(Default)]
    struct Recorder {
        played: Mutex<Vec<usize>>,
        stopped: AtomicBool,
    }

    impl Speaker for Recorder {
        fn play(&self, audio: Audio) -> Result<(), SpeechError> {
            self.played.lock().unwrap().push(audio.samples.len());
            Ok(())
        }

        fn stop(&self) {
            self.stopped.store(true, Ordering::SeqCst);
        }

        fn is_playing(&self) -> bool {
            false
        }
    }

    struct FakeMic(Audio);

    impl Microphone for FakeMic {
        fn start(&self) -> Result<(), SpeechError> {
            Ok(())
        }

        fn stop(&self) -> Result<Audio, SpeechError> {
            Ok(self.0.clone())
        }
    }

    fn parts(speaker: Arc<Recorder>, mic: Audio) -> VoiceParts {
        VoiceParts {
            stt: Some(Arc::new(Echo)),
            tts: Some(Arc::new(Echo)),
            microphone: Some(Arc::new(FakeMic(mic))),
            speaker: Some(speaker),
            speak_replies: true,
        }
    }

    async fn settle(v: &VoiceService) {
        for _ in 0..100 {
            if v.pending.load(Ordering::SeqCst) == 0 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn speaks_each_sentence_in_order_as_it_streams() {
        let sp = Arc::new(Recorder::default());
        let v = VoiceService::new(parts(sp.clone(), Audio::default()));
        v.say_streamed("First sentence here. Sec");
        v.say_streamed("ond one is longer! Tail");
        v.say_done();
        settle(&v).await;
        assert_eq!(
            *sp.played.lock().unwrap(),
            ["First sentence here.".len(), "Second one is longer!".len(), "Tail".len()]
        );
    }

    #[tokio::test]
    async fn interrupt_drops_queued_sentences() {
        let sp = Arc::new(Recorder::default());
        let v = VoiceService::new(parts(sp.clone(), Audio::default()));
        v.interrupt();
        v.say_streamed("This one is spoken. ");
        settle(&v).await;
        assert!(sp.stopped.load(Ordering::SeqCst));
        assert_eq!(sp.played.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_tap_on_the_key_is_not_speech() {
        let sp = Arc::new(Recorder::default());
        let tap = Audio { rate: 16_000, samples: vec![0.2; 1_000] };
        let v = VoiceService::new(parts(sp, tap));
        assert_eq!(v.heard().await.unwrap(), Heard::Nothing);
    }

    #[tokio::test]
    async fn a_phrase_is_transcribed() {
        let sp = Arc::new(Recorder::default());
        let phrase = Audio { rate: 16_000, samples: vec![0.2; 16_000] };
        let v = VoiceService::new(parts(sp, phrase));
        assert_eq!(v.heard().await.unwrap(), Heard::Text("16000 samples".into()));
    }

    #[test]
    fn whisper_silence_hallucinations_are_noise() {
        assert!(is_noise(" Thank you. "));
        assert!(!is_noise("Ouvre Spotify"));
    }
}
