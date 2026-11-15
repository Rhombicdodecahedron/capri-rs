//! Use case: run an ability, with the approval policy enforced here and nowhere else.

use std::collections::BTreeMap;
use std::sync::Arc;

use capri_domain::approval::{self, ApprovalMode, Decision, Origin};

use crate::ports::{ActionRequest, ApprovalPrompt, Tool, ToolError, ToolSpec};

pub struct ToolRunner {
    tools: BTreeMap<String, Arc<dyn Tool>>,
    mode: std::sync::RwLock<ApprovalMode>,
    prompt: Arc<dyn ApprovalPrompt>,
}

impl std::fmt::Debug for ToolRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRunner")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .field("mode", &self.mode())
            .finish_non_exhaustive()
    }
}

impl ToolRunner {
    pub fn new(mode: ApprovalMode, prompt: Arc<dyn ApprovalPrompt>) -> Self {
        Self { tools: BTreeMap::new(), mode: std::sync::RwLock::new(mode), prompt }
    }

    pub fn mode(&self) -> ApprovalMode {
        self.mode.read().map_or(ApprovalMode::Ask, |m| *m)
    }

    /// The SECURITY setting changed: applies to the next action.
    pub fn set_mode(&self, mode: ApprovalMode) {
        if let Ok(mut m) = self.mode.write() {
            *m = mode;
        }
    }

    #[must_use]
    pub fn with(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.insert(tool.spec().name, tool);
        self
    }

    /// Adds several tools at once (a family that shares state, like the `ui_*` ones).
    #[must_use]
    pub fn with_all(self, tools: impl IntoIterator<Item = Arc<dyn Tool>>) -> Self {
        tools.into_iter().fold(self, Self::with)
    }

    pub fn has(&self, name: &str) -> bool {
        self.tools.get(name).is_some_and(|t| t.enabled())
    }

    /// The tools to offer the model: the ones switched on.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().filter(|t| t.enabled()).map(|t| t.spec()).collect()
    }

    /// Runs a tool, asking through the runner's own prompt when the policy says so.
    pub async fn run(&self, name: &str, args: serde_json::Value, origin: Origin) -> Result<String, ToolError> {
        self.run_asking(name, args, origin, self.prompt.as_ref()).await
    }

    /// Like [`run`](Self::run), with the question going to `prompt` (the chat the request came from).
    pub async fn run_asking(
        &self,
        name: &str,
        args: serde_json::Value,
        origin: Origin,
        prompt: &dyn ApprovalPrompt,
    ) -> Result<String, ToolError> {
        let tool = self.tools.get(name).ok_or_else(|| ToolError::Unknown(name.into()))?;
        if !tool.enabled() {
            return Err(ToolError::Unavailable(format!("{name} is switched off in the settings")));
        }
        let risk = tool.risk(&args);
        match approval::decide_for(self.mode(), risk, origin) {
            Decision::Allow => {}
            Decision::Deny => return Err(ToolError::Blocked),
            Decision::AskUser => {
                let request = ActionRequest { tool: name.to_string(), summary: tool.describe(&args), risk };
                if !prompt.ask(&request).await {
                    tracing::info!(tool = name, "declined by the user");
                    return Err(ToolError::Declined);
                }
            }
        }
        tracing::info!(tool = name, ?origin, ?risk, "running tool");
        tool.run(args).await
    }
}
