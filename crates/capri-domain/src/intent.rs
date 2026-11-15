//! Commands recognised without asking the model (instant and free). Everything else goes to the model.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Remember(String),
    Forget(String),
    NewChat,
    Chat,
}

const REMEMBER: &[&str] = &["remember that ", "remember ", "souviens-toi que ", "souviens-toi de ", "retiens que "];
const FORGET: &[&str] = &["forget that ", "forget ", "oublie que ", "oublie "];
const NEW_CHAT: &[&str] = &["new chat", "nouvelle conversation", "/new", "/reset"];

pub fn parse(line: &str) -> Intent {
    let trimmed = line.trim();
    let lower = trimmed.to_lowercase();
    if NEW_CHAT.iter().any(|c| lower == *c) {
        return Intent::NewChat;
    }
    for p in REMEMBER {
        if let Some(rest) = strip_prefix_ci(trimmed, &lower, p) {
            return Intent::Remember(rest);
        }
    }
    for p in FORGET {
        if let Some(rest) = strip_prefix_ci(trimmed, &lower, p) {
            return Intent::Forget(rest);
        }
    }
    Intent::Chat
}

fn strip_prefix_ci(original: &str, lower: &str, prefix: &str) -> Option<String> {
    if !lower.starts_with(prefix) {
        return None;
    }
    // lowercase can change byte lengths for some scripts: walk the original by chars instead
    let n = prefix.chars().count();
    let rest: String = original.chars().skip(n).collect();
    let rest = rest.trim().trim_end_matches(['.', '!']).trim().to_string();
    (!rest.is_empty()).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_memory_commands_in_english_and_french() {
        assert_eq!(parse("Remember that I like synthwave."), Intent::Remember("I like synthwave".into()));
        assert_eq!(parse("souviens-toi que j'ai un chat"), Intent::Remember("j'ai un chat".into()));
        assert_eq!(parse("oublie mon chat"), Intent::Forget("mon chat".into()));
    }

    #[test]
    fn everything_else_is_chat() {
        assert_eq!(parse("what's the weather?"), Intent::Chat);
        assert_eq!(parse("remember"), Intent::Chat);
        assert_eq!(parse("/new"), Intent::NewChat);
    }
}
