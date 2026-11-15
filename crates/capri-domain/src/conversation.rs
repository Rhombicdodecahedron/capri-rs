//! A chat with her: messages, the persona's system prompt, and a bounded history window.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
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
                Never claim to have done something on the computer unless a tool result says it happened."
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
        out.extend(self.turns.iter().cloned());
        out
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
    fn prompt_puts_memory_in_system_message() {
        let c = Conversation::new(Persona::default(), 10);
        let p = c.prompt(&["likes synthwave".into()]);
        assert_eq!(p[0].role, Role::System);
        assert!(p[0].content.contains("likes synthwave"));
    }
}
