use async_trait::async_trait;
use capri_domain::Risk;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON schema of the arguments, sent to models that support tool calling.
    pub parameters: serde_json::Value,
    #[serde(skip)]
    pub risk: Risk,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("unknown tool `{0}`")]
    Unknown(String),
    #[error("bad arguments: {0}")]
    BadArguments(String),
    /// This tool can't do it right now (the app isn't open...): the caller may try another way.
    #[error("unavailable: {0}")]
    Unavailable(String),
    #[error("the user said no")]
    Declined,
    #[error("blocked by the read-only setting")]
    Blocked,
    #[error("{0}")]
    Failed(String),
}

/// Driven port: one ability (open an app, play music, read the weather, an MCP server tool...).
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    /// Switched off in the settings: not offered to the model, and refused if called.
    fn enabled(&self) -> bool {
        true
    }

    /// The risk of this particular call. Defaults to the spec's; tools whose risk depends on the
    /// target (which button, which keys) raise it here.
    fn risk(&self, args: &serde_json::Value) -> Risk {
        let _ = args;
        self.spec().risk
    }

    /// One line saying what this call will do, for the approval question.
    fn describe(&self, args: &serde_json::Value) -> String {
        let args = args.to_string();
        let args = if args.chars().count() > 160 {
            format!("{}…", args.chars().take(160).collect::<String>())
        } else {
            args
        };
        format!("{} {args}", self.spec().name)
    }

    async fn run(&self, args: serde_json::Value) -> Result<String, ToolError>;
}

/// A pending action shown to the user before it runs.
#[derive(Debug, Clone)]
pub struct ActionRequest {
    pub tool: String,
    pub summary: String,
    pub risk: Risk,
}

/// Driven port: asks the user yes or no (a dialog in the chat bubble, a prompt in the terminal).
#[async_trait]
pub trait ApprovalPrompt: Send + Sync {
    async fn ask(&self, request: &ActionRequest) -> bool;
}
