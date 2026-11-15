//! Reasoning models (Qwen3, DeepSeek-R1...) stream their thinking inside `<think>...</think>`.
//! She should never say that out loud, so it is cut from the stream, even when a tag is split
//! across two chunks.

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

#[derive(Debug, Default)]
pub struct ThinkFilter {
    inside: bool,
    pending: String,
}

impl ThinkFilter {
    pub fn feed(&mut self, text: &str) -> String {
        self.pending.push_str(text);
        let mut out = String::new();
        loop {
            let tag = if self.inside { CLOSE } else { OPEN };
            if let Some(i) = self.pending.find(tag) {
                if !self.inside {
                    out.push_str(&self.pending[..i]);
                }
                self.pending.drain(..i + tag.len());
                self.inside = !self.inside;
                continue;
            }
            // keep a possible partial tag at the end for the next chunk
            let keep = partial_suffix(&self.pending, tag);
            let cut = self.pending.len() - keep;
            if !self.inside {
                out.push_str(&self.pending[..cut]);
            }
            self.pending.drain(..cut);
            return out;
        }
    }
}

/// Length of the longest suffix of `s` that is a proper prefix of `tag`.
fn partial_suffix(s: &str, tag: &str) -> usize {
    (1..tag.len())
        .rev()
        .find(|&n| s.len() >= n && s.is_char_boundary(s.len() - n) && tag.starts_with(&s[s.len() - n..]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&str]) -> String {
        let mut f = ThinkFilter::default();
        chunks.iter().map(|c| f.feed(c)).collect()
    }

    #[test]
    fn drops_thinking() {
        assert_eq!(run(&["<think>hmm</think>Hello"]), "Hello");
    }

    #[test]
    fn handles_tags_split_across_chunks() {
        assert_eq!(run(&["<thi", "nk>secret</th", "ink>Yo", " choom"]), "Yo choom");
    }

    #[test]
    fn passes_plain_text_and_lone_angle_brackets() {
        assert_eq!(run(&["a < b", " and c > d"]), "a < b and c > d");
    }
}
