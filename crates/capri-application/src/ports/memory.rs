use async_trait::async_trait;
use capri_domain::Message;

#[derive(Debug, thiserror::Error)]
#[error("memory store: {0}")]
pub struct MemoryError(pub String);

/// Driven port: what she keeps about the user between sessions, stored on the user's own PC.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn remember(&self, fact: &str) -> Result<(), MemoryError>;

    /// Up to `limit` facts relevant to `query`, most relevant first.
    async fn recall(&self, query: &str, limit: usize) -> Result<Vec<String>, MemoryError>;

    async fn forget(&self, fact_contains: &str) -> Result<usize, MemoryError>;

    /// Appends a turn to the chat log so a later session can pick up where this one left off.
    async fn log_turn(&self, turn: &Message) -> Result<(), MemoryError>;

    async fn recent_turns(&self, limit: usize) -> Result<Vec<Message>, MemoryError>;
}
