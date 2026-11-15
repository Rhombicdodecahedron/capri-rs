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

    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|t| t.spec()).collect()
    }

    pub async fn run(&self, name: &str, args: serde_json::Value, origin: Origin) -> Result<String, ToolError> {
        let tool = self.tools.get(name).ok_or_else(|| ToolError::Unknown(name.into()))?;
        let spec = tool.spec();
        match approval::decide_for(self.mode(), spec.risk, origin) {
            Decision::Allow => {}
            Decision::Deny => return Err(ToolError::Blocked),
            Decision::AskUser => {
                let request =
                    ActionRequest { tool: spec.name.clone(), summary: summarize(&spec, &args), risk: spec.risk };
                if !self.prompt.ask(&request).await {
                    return Err(ToolError::Declined);
                }
            }
        }
        tracing::info!(tool = %spec.name, ?origin, "running tool");
        tool.run(args).await
    }
}

fn summarize(spec: &ToolSpec, args: &serde_json::Value) -> String {
    let args = args.to_string();
    let args =
        if args.chars().count() > 160 { format!("{}…", args.chars().take(160).collect::<String>()) } else { args };
    format!("{} {}", spec.name, args)
}
