//! Cuts a reply that streams in word by word into sentences, so each one can be spoken as soon as it
//! is complete instead of waiting for the whole reply.

#[derive(Debug, Default, Clone)]
pub struct SentenceSplitter {
    pending: String,
}

/// Shorter pieces are joined to the next sentence: "Oh. Hi." should not be two voice requests.
const MIN_CHARS: usize = 12;

impl SentenceSplitter {
    /// Feeds streamed text, returns the sentences that are now complete.
    pub fn feed(&mut self, text: &str) -> Vec<String> {
        self.pending.push_str(text);
        let mut out = Vec::new();
        while let Some(end) = sentence_end(&self.pending) {
            let sentence: String = self.pending.drain(..end).collect();
            let sentence = sentence.trim();
            if !sentence.is_empty() {
                out.push(sentence.to_string());
            }
        }
        out
    }

    /// The rest, once the reply is complete.
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.pending);
        let rest = rest.trim();
        (!rest.is_empty()).then(|| rest.to_string())
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

/// Byte index just after the first complete sentence of at least `MIN_CHARS`, if any.
fn sentence_end(s: &str) -> Option<usize> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let is_stop = matches!(c, '.' | '!' | '?' | '…' | '\n');
        if !is_stop || i + c.len_utf8() < MIN_CHARS.min(s.len()) {
            continue;
        }
        // a sentence ends at the punctuation run, then whitespace; "3.5" or "e.g." mid-word do not count
        let mut j = k + 1;
        loop {
            while j < chars.len() && matches!(chars[j].1, '.' | '!' | '?' | '…' | '"' | '»' | ')') {
                j += 1;
            }
            // French puts a space before a closing guillemet: « Tu es sûr ? »
            if j + 1 < chars.len() && chars[j].1.is_whitespace() && chars[j + 1].1 == '»' {
                j += 1;
                continue;
            }
            break;
        }
        if c == '\n' || (j < chars.len() && chars[j].1.is_whitespace()) {
            let end = chars.get(j).map_or(s.len(), |&(idx, _)| idx);
            if s[..end].trim().chars().count() >= MIN_CHARS {
                return Some(end);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(pieces: &[&str]) -> Vec<String> {
        let mut s = SentenceSplitter::default();
        let mut out: Vec<String> = pieces.iter().flat_map(|p| s.feed(p)).collect();
        out.extend(s.finish());
        out
    }

    #[test]
    fn splits_streamed_text_into_sentences() {
        assert_eq!(
            stream(&["Salut choom", ", ton build", " est passé. Je m'occupe", " du reste ! Ça", " roule ?"]),
            ["Salut choom, ton build est passé.", "Je m'occupe du reste !", "Ça roule ?"]
        );
    }

    #[test]
    fn keeps_numbers_and_short_bits_together() {
        assert_eq!(stream(&["Oh. Version 3.5 is out now. Nice"]), ["Oh. Version 3.5 is out now.", "Nice"]);
    }

    #[test]
    fn waits_for_the_space_after_the_dot() {
        let mut s = SentenceSplitter::default();
        assert_eq!(s.feed("This is the end."), Vec::<String>::new());
        assert_eq!(s.feed(" Next"), ["This is the end."]);
    }

    #[test]
    fn handles_ellipsis_and_quotes() {
        assert_eq!(stream(&["Attends… « Tu es sûr ? » Oui."]), ["Attends… « Tu es sûr ? »", "Oui."]);
    }
}
