//! Use case: the model saves a lasting fact about the user, like "souviens-toi que ..." does.

use std::sync::Arc;

use async_trait::async_trait;
use capri_domain::Risk;

use crate::ports::{MemoryStore, Tool, ToolError, ToolSpec};

pub struct RememberTool {
    memory: Arc<dyn MemoryStore>,
}

impl std::fmt::Debug for RememberTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RememberTool")
    }
}

impl RememberTool {
    pub fn new(memory: Arc<dyn MemoryStore>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for RememberTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "remember".into(),
            description: "Save a lasting fact about the user to your memory (a preference, a name, a plan), \
                written in the third person: 'Alex drives a 1989 BMW E30'. Only when they tell you something \
                worth keeping or ask you to remember."
                .into(),
            parameters: serde_json::json!({ "type": "object", "properties": { "fact": { "type": "string" } }, "required": ["fact"] }),
            risk: Risk::Reversible,
        }
    }

    fn describe(&self, args: &serde_json::Value) -> String {
        format!("remember \"{}\"", args.get("fact").and_then(|f| f.as_str()).unwrap_or("?"))
    }

    async fn run(&self, args: serde_json::Value) -> Result<String, ToolError> {
        let fact = args.get("fact").and_then(|f| f.as_str()).map(str::trim).unwrap_or_default();
        if fact.is_empty() {
            return Err(ToolError::BadArguments("no fact given".into()));
        }
        self.memory.remember(fact).await.map_err(|e| ToolError::Failed(e.to_string()))?;
        Ok(format!("saved: {fact}"))
    }
}
