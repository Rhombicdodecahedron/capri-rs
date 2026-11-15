//! Speech recognition on this PC, with whisper.cpp. Nothing leaves the machine.
//!
//! The adapter is compiled only with the `whisper` feature (it builds whisper.cpp, which needs CMake
//! and a C++ compiler). Without it, [`WhisperLocal::load`] explains how to get it, so the rest of the
//! app builds anywhere.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use capri_application::ports::{Audio, SpeechError, SpeechToText};

/// Whether this build can transcribe locally.
pub const AVAILABLE: bool = cfg!(feature = "whisper");

/// Model files from the whisper.cpp project, by size. `base` is a good start: fast on any CPU and
/// decent in French; `small` is noticeably better and still fine on a recent CPU.
pub const MODELS: [(&str, &str, u32); 4] = [
    ("tiny", "ggml-tiny.bin", 75),
    ("base", "ggml-base.bin", 142),
    ("small", "ggml-small.bin", 466),
    ("medium", "ggml-medium.bin", 1500),
];

pub fn model_url(file: &str) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{file}")
}

pub struct WhisperLocal {
    #[cfg(feature = "whisper")]
    ctx: std::sync::Arc<whisper_rs::WhisperContext>,
    language: Option<String>,
    path: PathBuf,
}

impl std::fmt::Debug for WhisperLocal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhisperLocal")
            .field("model", &self.path)
            .field("language", &self.language)
            .finish_non_exhaustive()
    }
}

impl WhisperLocal {
    /// Loads a ggml model file. `language`: `fr`, `en`... or `None` to detect it on every phrase.
    #[allow(clippy::needless_pass_by_value)] // kept by the real adapter, dropped by the stub
    pub fn load(model: &Path, language: Option<String>) -> Result<Self, SpeechError> {
        if !model.is_file() {
            return Err(SpeechError(format!("Whisper model not found: {}", model.display())));
        }
        #[cfg(feature = "whisper")]
        {
            let params = whisper_rs::WhisperContextParameters::default();
            let ctx = whisper_rs::WhisperContext::new_with_params(model, params)
                .map_err(|e| SpeechError(format!("could not load {}: {e}", model.display())))?;
            tracing::info!(model = %model.display(), "local Whisper ready");
            Ok(Self { ctx: std::sync::Arc::new(ctx), language, path: model.to_path_buf() })
        }
        #[cfg(not(feature = "whisper"))]
        {
            let _ = language;
            Err(SpeechError(
                "this build has no local Whisper: rebuild with `cargo run --release --features whisper-local` \
                 (needs CMake), or pick a Whisper server in the settings"
                    .into(),
            ))
        }
    }
}

#[async_trait]
impl SpeechToText for WhisperLocal {
    async fn transcribe(&self, audio: &Audio) -> Result<String, SpeechError> {
        #[cfg(feature = "whisper")]
        {
            let ctx = self.ctx.clone();
            let language = self.language.clone();
            let samples = audio.samples.clone();
            // whisper.cpp is CPU-bound and blocking
            tokio::task::spawn_blocking(move || run(&ctx, language.as_deref(), &samples))
                .await
                .map_err(|e| SpeechError(e.to_string()))?
        }
        #[cfg(not(feature = "whisper"))]
        {
            let _ = audio;
            Err(SpeechError("this build has no local Whisper".into()))
        }
    }
}

#[cfg(feature = "whisper")]
fn run(ctx: &whisper_rs::WhisperContext, language: Option<&str>, samples: &[f32]) -> Result<String, SpeechError> {
    use whisper_rs::{FullParams, SamplingStrategy};
    let mut state = ctx.create_state().map_err(|e| SpeechError(e.to_string()))?;
    let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    p.set_language(Some(language.filter(|l| !l.is_empty()).unwrap_or("auto")));
    p.set_n_threads(std::thread::available_parallelism().map_or(4, |n| n.get().min(8)) as i32);
    // push-to-talk phrases are short: no context carried over, no timestamps, no console output
    p.set_no_context(true);
    p.set_no_timestamps(true);
    p.set_single_segment(true);
    p.set_suppress_blank(true);
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_special(false);
    p.set_print_timestamps(false);
    state.full(p, samples).map_err(|e| SpeechError(e.to_string()))?;
    let text: Vec<String> =
        state.as_iter().filter_map(|s| s.to_str_lossy().ok().map(|t| t.trim().to_string())).collect();
    Ok(text.join(" ").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_model_is_a_clear_error() {
        let e = WhisperLocal::load(Path::new("/nope/ggml-base.bin"), None).unwrap_err();
        assert!(e.0.contains("not found"));
    }

    #[test]
    fn model_urls_point_at_whisper_cpp() {
        assert!(model_url("ggml-base.bin").ends_with("/ggml-base.bin"));
    }
}
