//! A chat with her: messages, the persona's system prompt, and a bounded history window.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    /// The result of a tool she called, answering one of the assistant's tool calls.
    Tool,
}

/// The model asking to run one of her abilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Set by the model; the matching result carries it back.
    pub id: String,
    pub name: String,
    /// JSON text, exactly as the model wrote it (it may not even parse).
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// On an assistant turn: the tools it wants run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// On a tool turn: which call this answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    fn of(role: Role, content: impl Into<String>) -> Self {
        Self { role, content: content.into(), tool_calls: Vec::new(), tool_call_id: None }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::of(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::of(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::of(Role::Assistant, content)
    }

    /// An assistant turn that calls tools (with whatever it said alongside).
    pub fn calling(content: impl Into<String>, calls: Vec<ToolCall>) -> Self {
        Self { tool_calls: calls, ..Self::of(Role::Assistant, content) }
    }

    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self { tool_call_id: Some(call_id.into()), ..Self::of(Role::Tool, content) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Persona {
    pub name: String,
    pub system_prompt: String,
}

impl Default for Persona {
    fn default() -> Self {
        Self {
            name: "Capri".into(),
            system_prompt: "You are Capri, a sharp, playful netrunner AI who lives on the user's desktop. \
                Keep replies short and spoken-sounding (one to three sentences), confident, a little cheeky, \
                always on the user's side. Answer in the language the user writes in. \
                Never claim to have done something on the computer unless a tool result says it happened. \
                The app runs music commands itself before you see them (pause, next, volume, \
                'mets X sur Spotify'). When tools are offered, use them to act on the PC; otherwise say \
                plainly that you can't do it from here."
                .into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Conversation {
    persona: Persona,
    turns: Vec<Message>,
    max_turns: usize,
}

impl Conversation {
    pub fn new(persona: Persona, max_turns: usize) -> Self {
        Self { persona, turns: Vec::new(), max_turns: max_turns.max(2) }
    }

    pub fn persona(&self) -> &Persona {
        &self.persona
    }

    pub fn set_persona(&mut self, persona: Persona) {
        self.persona = persona;
    }

    pub fn push(&mut self, m: Message) {
        debug_assert!(m.role != Role::System, "system text belongs to the persona or to context");
        self.turns.push(m);
        if self.turns.len() > self.max_turns {
            let drop = self.turns.len() - self.max_turns;
            self.turns.drain(..drop);
        }
        // a window must not open on a dangling assistant turn
        while self.turns.first().is_some_and(|m| m.role == Role::Assistant) {
            self.turns.remove(0);
        }
    }

    pub fn turns(&self) -> &[Message] {
        &self.turns
    }

    pub fn clear(&mut self) {
        self.turns.clear();
    }

    /// The full prompt: persona, then optional recalled context, then the recent turns.
    pub fn prompt(&self, context: &[String]) -> Vec<Message> {
        let mut system = self.persona.system_prompt.clone();
        if !context.is_empty() {
            system.push_str("\n\nThings you remember about the user:\n");
            for c in context {
                system.push_str("- ");
                system.push_str(c);
                system.push('\n');
            }
        }
        let mut out = Vec::with_capacity(self.turns.len() + 1);
        out.push(Message::system(system));
        // Many chat templates (Gemma, Mistral, Llama 2) reject two turns of the same role in a row:
        // merge them, so a reply that failed earlier never breaks the next one.
        for t in &self.turns {
            match out.last_mut() {
                Some(last)
                    if last.role == t.role
                        && matches!(last.role, Role::User | Role::Assistant)
                        && last.tool_calls.is_empty()
                        && t.tool_calls.is_empty() =>
                {
                    last.content.push_str("\n\n");
                    last.content.push_str(&t.content);
                }
                _ => out.push(t.clone()),
            }
        }
        out
    }

    /// Drops the last turn if it is the user's (their message got no reply).
    pub fn drop_unanswered(&mut self) {
        if self.turns.last().is_some_and(|m| m.role == Role::User) {
            self.turns.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_and_starts_with_user() {
        let mut c = Conversation::new(Persona::default(), 3);
        for i in 0..5 {
            c.push(Message::user(format!("q{i}")));
            c.push(Message::assistant(format!("a{i}")));
        }
        assert!(c.turns().len() <= 3);
        assert_eq!(c.turns()[0].role, Role::User);
    }

    #[test]
    fn prompt_alternates_roles() {
        let mut c = Conversation::new(Persona::default(), 10);
        c.push(Message::user("hi"));
        c.push(Message::user("hello?"));
        c.push(Message::assistant("hey"));
        let p = c.prompt(&[]);
        assert_eq!(p.iter().map(|m| m.role).collect::<Vec<_>>(), [Role::System, Role::User, Role::Assistant]);
        assert_eq!(p[1].content, "hi\n\nhello?");
    }

    #[test]
    fn unanswered_turn_is_dropped() {
        let mut c = Conversation::new(Persona::default(), 10);
        c.push(Message::user("hi"));
        c.drop_unanswered();
        assert_eq!(c.turns(), &[] as &[Message]);
    }

    #[test]
    fn prompt_puts_memory_in_system_message() {
        let c = Conversation::new(Persona::default(), 10);
        let p = c.prompt(&["likes synthwave".into()]);
        assert_eq!(p[0].role, Role::System);
        assert!(p[0].content.contains("likes synthwave"));
    }
}
