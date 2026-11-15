use std::pin::Pin;

use async_trait::async_trait;
use capri_domain::{Message, ToolCall};

use crate::ports::ToolSpec;
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

/// One piece of a reply that may call tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Call(ToolCall),
}

pub type PieceStream = Pin<Box<dyn Stream<Item = Result<Piece, LlmError>> + Send>>;

/// Driven port: anything that turns a message list into a streamed reply.
/// Adapters: any OpenAI-compatible server (Ollama, LM Studio, llama-server, `OpenAI`, `OpenRouter`...),
/// the Claude Code CLI, and later Codex or others.
#[async_trait]
pub trait LanguageModel: Send + Sync {
    /// Shown in the settings and logs, e.g. `ollama/qwen3:8b`.
    fn name(&self) -> String;

    async fn stream(&self, messages: &[Message]) -> Result<TokenStream, LlmError>;

    /// Whether [`respond`](Self::respond) can call tools at all (the server or CLI supports it).
    fn supports_tools(&self) -> bool {
        false
    }

    /// A reply that may call some of `tools`. Brains without tool calling just stream text.
    async fn respond(&self, messages: &[Message], tools: &[ToolSpec]) -> Result<PieceStream, LlmError> {
        use futures::StreamExt;
        let _ = tools;
        let text = self.stream(messages).await?;
        Ok(Box::pin(text.map(|r| r.map(Piece::Text))))
    }
}
