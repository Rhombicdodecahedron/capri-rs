//! Minimal server-sent-events line splitter: network chunks can cut a line anywhere, even inside a
//! UTF-8 character, so bytes are buffered until a full line arrives.

#[derive(Debug, Default)]
pub struct Lines {
    buf: Vec<u8>,
}

impl Lines {
    /// Feeds raw bytes, returns the payload of every complete `data:` line.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(data) = line.strip_prefix("data:") {
                out.push(data.trim_start().to_string());
            } else if let Some(err) = line.strip_prefix("error:") {
                // llama-server reports failures inside a stream on an `error:` line
                let err = err.trim_start();
                let payload =
                    if err.starts_with('{') { err.to_string() } else { serde_json::Value::from(err).to_string() };
                out.push(format!("{{\"error\":{payload}}}"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_lines_split_across_chunks() {
        let mut l = Lines::default();
        assert_eq!(l.push(b"data: {\"a\""), Vec::<String>::new());
        assert_eq!(l.push(b":1}\r\n\r\ndata: [DONE]\n"), vec!["{\"a\":1}", "[DONE]"]);
    }

    #[test]
    fn keeps_multibyte_chars_split_across_chunks() {
        let mut l = Lines::default();
        let bytes = "data: é\n".as_bytes();
        assert_eq!(l.push(&bytes[..7]), Vec::<String>::new());
        assert_eq!(l.push(&bytes[7..]), vec!["é"]);
    }

    #[test]
    fn keeps_llama_server_error_lines() {
        let mut l = Lines::default();
        let out = l.push(b"error: {\"code\":500,\"message\":\"roles must alternate\"}\n");
        assert_eq!(out, vec![r#"{"error":{"code":500,"message":"roles must alternate"}}"#]);
    }

    #[test]
    fn ignores_comments_and_event_lines() {
        let mut l = Lines::default();
        assert_eq!(l.push(b": keep-alive\nevent: ping\n"), Vec::<String>::new());
    }
}
