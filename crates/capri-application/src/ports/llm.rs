use std::pin::Pin;

use async_trait::async_trait;
use capri_domain::Message;
use futures::Stream;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("the model is unreachable: {0}")]
    Unreachable(String),
    #[error("the model answered with an error: {0}")]
    Backend(String),
    #[error("not signed in: {0}")]
    SignedOut(String),
    #[error("the reply was cut off: {0}")]
    Interrupted(String),
}

/// A reply arriving piece by piece, so she can start talking before the model is done.
pub type TokenStream = Pin<Box<dyn Stream<Item = Result<String, LlmError>> + Send>>;

/// Driven port: anything that turns a message list into a streamed reply.
/// Adapters: any OpenAI-compatible server (Ollama, LM Studio, llama-server, `OpenAI`, `OpenRouter`...),
/// the Claude Code CLI, and later Codex or others.
#[async_trait]
pub trait LanguageModel: Send + Sync {
    /// Shown in the settings and logs, e.g. `ollama/qwen3:8b`.
    fn name(&self) -> String;

    async fn stream(&self, messages: &[Message]) -> Result<TokenStream, LlmError>;
}
