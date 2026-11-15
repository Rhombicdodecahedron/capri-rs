//! The chat box above her head: what she is saying, or what the user is typing.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BubbleState {
    Hidden,
    /// The user is typing a line to her.
    Typing {
        draft: String,
    },
    /// Waiting for the first words of the reply.
    Thinking,
    /// Her reply (possibly still streaming in).
    Saying {
        text: String,
        done: bool,
    },
    /// The model failed: shown in red so it is never mistaken for her answer.
    Error(String),
}

#[derive(Debug, Clone)]
pub struct Bubble {
    pub state: BubbleState,
    /// When a finished reply or error should disappear (seconds on the app clock).
    pub hide_at: Option<f32>,
}

impl Default for Bubble {
    fn default() -> Self {
        Self { state: BubbleState::Hidden, hide_at: None }
    }
}

impl Bubble {
    pub fn is_open(&self) -> bool {
        self.state != BubbleState::Hidden
    }

    pub fn is_typing(&self) -> bool {
        matches!(self.state, BubbleState::Typing { .. })
    }

    pub fn open_typing(&mut self) {
        self.state = BubbleState::Typing { draft: String::new() };
        self.hide_at = None;
    }

    pub fn close(&mut self) {
        self.state = BubbleState::Hidden;
        self.hide_at = None;
    }

    pub fn type_text(&mut self, s: &str) {
        if let BubbleState::Typing { draft } = &mut self.state {
            for c in s.chars().filter(|c| !c.is_control()) {
                if draft.chars().count() < 500 {
                    draft.push(c);
                }
            }
        }
    }

    pub fn backspace(&mut self) {
        if let BubbleState::Typing { draft } = &mut self.state {
            draft.pop();
        }
    }

    /// Takes the typed line (Enter) and switches to the thinking state.
    pub fn submit(&mut self) -> Option<String> {
        let BubbleState::Typing { draft } = &mut self.state else { return None };
        let line = std::mem::take(draft).trim().to_string();
        if line.is_empty() {
            return None;
        }
        self.state = BubbleState::Thinking;
        Some(line)
    }

    pub fn push_reply(&mut self, delta: &str) {
        match &mut self.state {
            BubbleState::Saying { text, done: false } => text.push_str(delta),
            _ => self.state = BubbleState::Saying { text: delta.to_string(), done: false },
        }
    }

    /// The reply is complete: keep it up long enough to read (about 3 words a second, 4 s minimum).
    pub fn finish(&mut self, full: String, now: f32) {
        let words = full.split_whitespace().count() as f32;
        self.hide_at = Some(now + (words / 3.0).max(4.0));
        self.state = BubbleState::Saying { text: full, done: true };
    }

    pub fn fail(&mut self, error: String, now: f32) {
        self.hide_at = Some(now + 8.0);
        self.state = BubbleState::Error(error);
    }

    /// True while she should look like she is talking (mouth flaps).
    pub fn talking(&self, now: f32) -> bool {
        match &self.state {
            BubbleState::Saying { done: false, .. } => true,
            BubbleState::Saying { done: true, text } => {
                // flap for roughly the time it takes to say the line, then just hold it on screen
                let words = text.split_whitespace().count() as f32;
                self.hide_at.is_some_and(|h| now < h - (words / 3.0).max(4.0) + words / 2.5)
            }
            _ => false,
        }
    }

    pub fn tick(&mut self, now: f32) {
        if self.hide_at.is_some_and(|h| now >= h) {
            self.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_flow() {
        let mut b = Bubble::default();
        b.open_typing();
        b.type_text("hi\u{8}");
        b.type_text("!");
        b.backspace();
        assert_eq!(b.submit(), Some("hi".into()));
        assert_eq!(b.state, BubbleState::Thinking);
        b.push_reply("Hey");
        b.push_reply(" you");
        b.finish("Hey you".into(), 10.0);
        assert!(b.is_open());
        b.tick(14.1);
        assert!(!b.is_open());
    }

    #[test]
    fn empty_lines_are_not_sent() {
        let mut b = Bubble::default();
        b.open_typing();
        b.type_text("   ");
        assert_eq!(b.submit(), None);
        assert!(b.is_typing());
    }
}
