//! Her brain as the Claude Code CLI. Each reply is one `claude -p` run with tools switched off and
//! streamed JSON on stdout. The user's own Claude subscription or API key is used; no key is stored here.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use capri_application::ports::{LanguageModel, LlmError, TokenStream};
use capri_domain::{Message, Role};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    /// Path to the `claude` executable; `None` looks it up on `PATH`.
    pub executable: Option<PathBuf>,
    /// e.g. `sonnet`, `opus`, `haiku`; `None` uses the CLI default.
    pub model: Option<String>,
    pub timeout_secs: u64,
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self { executable: None, model: None, timeout_secs: 120 }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeCode {
    cfg: ClaudeCodeConfig,
}

impl ClaudeCode {
    pub fn new(cfg: ClaudeCodeConfig) -> Self {
        Self { cfg }
    }

    fn exe(&self) -> PathBuf {
        self.cfg
            .executable
            .clone()
            .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "claude.cmd" } else { "claude" }))
    }

    fn spawn(&self, system_file: &std::path::Path) -> std::io::Result<Child> {
        let mut cmd = Command::new(self.exe());
        cmd.args(["-p", "--no-session-persistence", "--strict-mcp-config", "--disable-slash-commands"])
            .args(["--tools", "", "--output-format", "stream-json", "--verbose", "--include-partial-messages"])
            .arg("--system-prompt-file")
            .arg(system_file);
        if let Some(m) = &self.cfg.model {
            cmd.args(["--model", m]);
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.spawn()
    }
}

/// Splits a chat into the CLI's two inputs: a system prompt and one user prompt holding the transcript.
pub fn render(messages: &[Message]) -> (String, String) {
    let system: Vec<&str> = messages.iter().filter(|m| m.role == Role::System).map(|m| m.content.as_str()).collect();
    let turns: Vec<&Message> = messages.iter().filter(|m| m.role != Role::System).collect();
    let system = system.join("\n\n");
    let Some((last, earlier)) = turns.split_last() else {
        return (system, String::new());
    };
    if earlier.is_empty() {
        return (system, last.content.clone());
    }
    let mut prompt = String::from("The chat so far:\n\n");
    for m in earlier {
        let who = if m.role == Role::Assistant { "YOU" } else { "USER" };
        let _ = write!(prompt, "{who}: {}\n\n", m.content);
    }
    let _ = write!(prompt, "USER: {}\n\nWrite only your next reply.", last.content);
    (system, prompt)
}

/// One line of `stream-json` output.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Text(String),
    Result { ok: bool, text: String },
    Other,
}

pub fn parse_event(line: &str) -> Event {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
        return Event::Other;
    };
    match v["type"].as_str() {
        Some("stream_event") => {
            let e = &v["event"];
            if e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta" {
                return Event::Text(e["delta"]["text"].as_str().unwrap_or_default().to_string());
            }
            Event::Other
        }
        Some("result") => Event::Result {
            ok: !v["is_error"].as_bool().unwrap_or(false),
            text: v["result"].as_str().or_else(|| v["subtype"].as_str()).unwrap_or_default().to_string(),
        },
        _ => Event::Other,
    }
}

struct Run {
    lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    child: Child,
    said: bool,
    done: bool,
    deadline: tokio::time::Instant,
    _system: TempFile,
}

/// Deleted when the run ends.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[async_trait]
impl LanguageModel for ClaudeCode {
    fn name(&self) -> String {
        format!("claude-code{}", self.cfg.model.as_ref().map(|m| format!("/{m}")).unwrap_or_default())
    }

    async fn stream(&self, messages: &[Message]) -> Result<TokenStream, LlmError> {
        let (system, prompt) = render(messages);
        let path = std::env::temp_dir().join(format!("capri-system-{}-{}.txt", std::process::id(), unique()));
        tokio::fs::write(&path, system).await.map_err(|e| LlmError::Unreachable(e.to_string()))?;
        let system_file = TempFile(path);

        let mut child = self.spawn(&system_file.0).map_err(|e| {
            LlmError::Unreachable(format!(
                "could not start `{}` ({e}). Is Claude Code installed?",
                self.exe().display()
            ))
        })?;
        let mut stdin = child.stdin.take().ok_or_else(|| LlmError::Unreachable("no stdin".into()))?;
        stdin.write_all(prompt.as_bytes()).await.map_err(|e| LlmError::Unreachable(e.to_string()))?;
        drop(stdin);
        let stdout = child.stdout.take().ok_or_else(|| LlmError::Unreachable("no stdout".into()))?;

        let run = Run {
            lines: BufReader::new(stdout).lines(),
            child,
            said: false,
            done: false,
            deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(self.cfg.timeout_secs),
            _system: system_file,
        };
        let stream = futures::stream::unfold(run, |mut run| async move {
            loop {
                if run.done {
                    return None;
                }
                let line = match tokio::time::timeout_at(run.deadline, run.lines.next_line()).await {
                    Err(_) => {
                        run.done = true;
                        let _ = run.child.start_kill();
                        return Some((Err(LlmError::Interrupted("Claude Code timed out".into())), run));
                    }
                    Ok(Err(e)) => {
                        run.done = true;
                        return Some((Err(LlmError::Interrupted(e.to_string())), run));
                    }
                    Ok(Ok(None)) => {
                        run.done = true;
                        let err = stderr_tail(&mut run.child).await;
                        return Some((
                            Err(LlmError::Backend(format!("Claude Code stopped without an answer. {err}"))),
                            run,
                        ));
                    }
                    Ok(Ok(Some(l))) => l,
                };
                match parse_event(&line) {
                    Event::Text(t) if !t.is_empty() => {
                        run.said = true;
                        return Some((Ok(t), run));
                    }
                    Event::Result { ok: false, text } => {
                        run.done = true;
                        let lower = text.to_lowercase();
                        let err = if lower.contains("login") || lower.contains("auth") {
                            LlmError::SignedOut(text)
                        } else {
                            LlmError::Backend(text)
                        };
                        return Some((Err(err), run));
                    }
                    Event::Result { ok: true, text } => {
                        run.done = true;
                        // an older CLI without partial messages sends the whole reply at the end
                        if !run.said && !text.is_empty() {
                            return Some((Ok(text), run));
                        }
                        return None;
                    }
                    _ => {}
                }
            }
        });
        Ok(Box::pin(stream))
    }
}

async fn stderr_tail(child: &mut Child) -> String {
    use tokio::io::AsyncReadExt;
    let mut s = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut s).await;
    }
    let s = s.trim();
    s.char_indices().rev().nth(300).map_or(s, |(i, _)| &s[i..]).to_string()
}

fn unique() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lone_question_goes_as_is() {
        let (sys, prompt) = render(&[Message::system("be Capri"), Message::user("hi")]);
        assert_eq!(sys, "be Capri");
        assert_eq!(prompt, "hi");
    }

    #[test]
    fn history_becomes_a_transcript() {
        let (_, prompt) =
            render(&[Message::system("s"), Message::user("a"), Message::assistant("b"), Message::user("c")]);
        assert!(
            prompt.contains("USER: a") && prompt.contains("YOU: b") && prompt.ends_with("Write only your next reply.")
        );
    }

    #[test]
    fn parses_stream_events() {
        let delta = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hey"}}}"#;
        assert_eq!(parse_event(delta), Event::Text("Hey".into()));
        let fail = r#"{"type":"result","is_error":true,"result":"Invalid API key"}"#;
        assert_eq!(parse_event(fail), Event::Result { ok: false, text: "Invalid API key".into() });
        assert_eq!(parse_event("not json"), Event::Other);
    }
}
