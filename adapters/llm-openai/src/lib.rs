//! One adapter for every OpenAI-compatible `/v1/chat/completions` server. Ollama, LM Studio,
//! llama.cpp's `llama-server`, vLLM, OpenAI and OpenRouter all speak this protocol, so they differ
//! only by base URL, model name, API key and a few request extras.

use async_trait::async_trait;
use capri_application::ports::{LanguageModel, LlmError, Piece, PieceStream, TokenStream, ToolSpec};
use capri_domain::{Message, Role, ToolCall};
use futures::StreamExt;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use futures::stream::BoxStream;
use tokio::sync::OnceCell;

mod sse;
mod think;

pub use think::ThinkFilter;

#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    /// e.g. `http://localhost:11434/v1` for Ollama, `http://localhost:8080/v1` for llama-server.
    pub base_url: String,
    /// `None` asks the server (`GET /models`) and uses the first model it serves. That is what you
    /// want for llama-server, which serves exactly one model whatever name you send.
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Let reasoning models think before answering. Off by default: she answers in a sentence or two,
    /// and thinking only adds seconds before the first word (and burns the token budget).
    pub thinking: bool,
    /// Extra JSON merged into every request body, for server-specific options.
    pub extra: Option<serde_json::Value>,
    /// Offer her tools to the model (native tool calling). Servers that refuse them are noticed
    /// and get plain chat from then on.
    pub tools: bool,
}

impl OpenAiConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: None,
            api_key: None,
            temperature: Some(0.8),
            max_tokens: Some(400),
            thinking: false,
            extra: None,
            tools: true,
        }
    }

    /// llama.cpp's `llama-server`: thinking is switched off through the chat template when asked.
    pub fn llama_server(base_url: impl Into<String>) -> Self {
        let mut c = Self::new(base_url);
        c.extra = Some(serde_json::json!({ "chat_template_kwargs": { "enable_thinking": false } }));
        c
    }
}

/// Extra tokens allowed for a model's hidden reasoning, on top of the reply budget.
pub const THINKING_BUDGET: u32 = 3072;

#[derive(Debug, Clone)]
pub struct OpenAiCompatible {
    cfg: OpenAiConfig,
    http: reqwest::Client,
    resolved: Arc<OnceCell<String>>,
    /// Set once the model has been seen thinking although thinking was turned off (some servers
    /// ignore the switch): from then on every request leaves room for it.
    thinks: Arc<AtomicBool>,
    /// Set once the server refused the `tools` field (llama-server without `--jinja`...).
    no_tools: Arc<AtomicBool>,
}

#[derive(Deserialize)]
struct Models {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    /// Some servers report a failure mid-stream as a JSON error object.
    #[serde(default)]
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    /// llama-server, vLLM and DeepSeek put a reasoning model's thinking here.
    #[serde(default, alias = "reasoning")]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<CallDelta>>,
}

/// A piece of a tool call; the arguments arrive a few characters at a time.
#[derive(Deserialize, Default)]
struct CallDelta {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FnDelta>,
}

#[derive(Deserialize, Default)]
struct FnDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// What the stream carried besides visible text, to explain an empty answer.
#[derive(Debug, Default)]
struct Tally {
    said: bool,
    reasoning_chars: usize,
    /// Text that arrived as content but sat inside `<think>` tags.
    hidden_chars: usize,
    /// SSE `data:` payloads seen.
    events: usize,
    finish: Option<String>,
    error: Option<String>,
    /// The start of the raw body, for a server that ignored `stream: true` and for diagnostics.
    head: String,
    /// Tool calls being assembled, by index.
    calls: Vec<ToolCall>,
}

impl Tally {
    /// The error for a stream that ended without a visible word.
    fn empty_reply(&self, max_tokens: Option<u32>) -> LlmError {
        if let Some(e) = &self.error {
            return LlmError::Backend(e.clone());
        }
        let budget = max_tokens.map_or_else(String::new, |m| format!(" ({m} tokens)"));
        match (self.reasoning_chars + self.hidden_chars, self.finish.as_deref()) {
            (r, Some("length")) if r > 0 => LlmError::Backend(format!(
                "the model spent its whole budget{budget} thinking and never answered: raise the max reply length, \
                 or use a model or server option that turns thinking off"
            )),
            (r, _) if r > 0 => LlmError::Backend("the model only thought and gave no answer".into()),
            (_, Some("length")) => LlmError::Backend(format!("the model hit the length limit{budget} before any text")),
            _ if self.events == 0 => LlmError::Backend(format!(
                "the server did not stream a reply; it answered: {}",
                self.head.chars().take(200).collect::<String>()
            )),
            _ => LlmError::Backend(format!("empty reply (finish: {})", self.finish.as_deref().unwrap_or("none"))),
        }
    }

    /// The tool calls the reply carried (streamed, or in a whole completion).
    fn tool_calls(&self) -> Vec<ToolCall> {
        let mut calls: Vec<ToolCall> = self.calls.iter().filter(|c| !c.name.trim().is_empty()).cloned().collect();
        if calls.is_empty() && self.events == 0 {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(self.head.trim()) {
                if let Some(list) = v["choices"][0]["message"]["tool_calls"].as_array() {
                    for (i, c) in list.iter().enumerate() {
                        let args = &c["function"]["arguments"];
                        calls.push(ToolCall {
                            id: c["id"].as_str().map_or_else(|| format!("call_{i}"), str::to_string),
                            name: c["function"]["name"].as_str().unwrap_or_default().to_string(),
                            arguments: args.as_str().map_or_else(|| args.to_string(), str::to_string),
                        });
                    }
                }
            }
        }
        for (i, c) in calls.iter_mut().enumerate() {
            if c.id.is_empty() {
                c.id = format!("call_{i}");
            }
        }
        calls
    }

    /// A server that ignored `stream: true` sends one plain JSON completion: take its text.
    fn whole_reply(&self) -> Option<String> {
        if self.events > 0 {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(self.head.trim()).ok()?;
        let text = v["choices"][0]["message"]["content"].as_str()?;
        let mut think = ThinkFilter::default();
        let visible = think.feed(text);
        (!visible.trim().is_empty()).then(|| visible.trim().to_string())
    }
}

/// One SSE payload: the visible text it carries, after updating the tally.
fn absorb(data: &str, tally: &mut Tally, think: &mut ThinkFilter) -> Option<String> {
    if data == "[DONE]" {
        return None;
    }
    tally.events += 1;
    let parsed: Chunk = serde_json::from_str(data).ok()?;
    if let Some(e) = parsed.error {
        tally.error = Some(e.get("message").and_then(|m| m.as_str()).map_or_else(|| e.to_string(), str::to_string));
    }
    let mut text = String::new();
    for c in parsed.choices {
        if let Some(r) = c.delta.reasoning_content {
            tally.reasoning_chars += r.len();
        }
        if let Some(t) = c.delta.content {
            text.push_str(&t);
        }
        for d in c.delta.tool_calls.unwrap_or_default() {
            while tally.calls.len() <= d.index {
                tally.calls.push(ToolCall { id: String::new(), name: String::new(), arguments: String::new() });
            }
            let call = &mut tally.calls[d.index];
            if let Some(id) = d.id.filter(|i| !i.is_empty()) {
                call.id = id;
            }
            if let Some(f) = d.function {
                if let Some(n) = f.name {
                    call.name.push_str(&n);
                }
                if let Some(a) = f.arguments {
                    call.arguments.push_str(&a);
                }
            }
        }
        if c.finish_reason.is_some() {
            tally.finish = c.finish_reason;
        }
    }
    let visible = think.feed(&text);
    tally.hidden_chars += text.len().saturating_sub(visible.len());
    if visible.trim().is_empty() {
        return None;
    }
    tally.said = true;
    Some(visible)
}

impl OpenAiCompatible {
    pub fn new(cfg: OpenAiConfig) -> Self {
        let http =
            reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(5)).build().unwrap_or_default();
        Self {
            cfg,
            http,
            resolved: Arc::new(OnceCell::new()),
            thinks: Arc::new(AtomicBool::new(false)),
            no_tools: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn config(&self) -> &OpenAiConfig {
        &self.cfg
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.cfg.base_url.trim_end_matches('/'))
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.cfg.api_key {
            Some(key) => req.bearer_auth(key),
            None => req,
        }
    }

    /// The models the server offers (`GET /models`).
    pub async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let resp = self
            .auth(self.http.get(self.url("models")))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| LlmError::Unreachable(e.to_string()))?;
        let resp = check(resp).await?;
        let models: Models =
            resp.json().await.map_err(|e| LlmError::Backend(format!("unexpected /models answer: {e}")))?;
        Ok(models.data.into_iter().map(|m| m.id).collect())
    }

    /// The configured model, or the first one the server lists.
    pub async fn model(&self) -> Result<String, LlmError> {
        if let Some(m) = self.cfg.model.as_ref().filter(|m| !m.trim().is_empty()) {
            return Ok(m.clone());
        }
        self.resolved
            .get_or_try_init(|| async {
                let models = self.list_models().await?;
                let first =
                    models.into_iter().next().ok_or_else(|| LlmError::Backend("the server lists no model".into()))?;
                tracing::info!(model = %first, url = %self.cfg.base_url, "using the server's model");
                Ok(first)
            })
            .await
            .cloned()
    }
}

async fn check(resp: reqwest::Response) -> Result<reqwest::Response, LlmError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let text = resp.text().await.unwrap_or_default();
    let text: String = text.chars().take(300).collect();
    Err(match status.as_u16() {
        401 | 403 => LlmError::SignedOut(format!("{status}: {text}")),
        _ => LlmError::Backend(format!("{status}: {text}")),
    })
}

/// One message as the chat completions API wants it.
fn wire(m: &Message) -> serde_json::Value {
    let role = match m.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut v = serde_json::json!({ "role": role, "content": m.content });
    if !m.tool_calls.is_empty() {
        v["tool_calls"] = m
            .tool_calls
            .iter()
            .map(|c| {
                serde_json::json!({ "id": c.id, "type": "function", "function": { "name": c.name, "arguments": c.arguments } })
            })
            .collect();
    }
    if let Some(id) = &m.tool_call_id {
        v["tool_call_id"] = id.clone().into();
    }
    v
}

/// The request body. Pure, so the exact shape sent to each server is unit-tested.
pub fn build_body(cfg: &OpenAiConfig, model: &str, messages: &[Message]) -> serde_json::Value {
    build_body_with_tools(cfg, model, messages, &[])
}

/// [`build_body`] offering `tools` to the model.
pub fn build_body_with_tools(
    cfg: &OpenAiConfig,
    model: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> serde_json::Value {
    let mut messages: Vec<Message> = messages.to_vec();
    // Qwen3 reads a `/no_think` switch in the user turn, on any server: the portable way to skip
    // its reasoning when the server has no option for it.
    if !cfg.thinking && model.to_lowercase().contains("qwen3") {
        if let Some(last) = messages.iter_mut().rev().find(|m| m.role == Role::User) {
            last.content.push_str("\n\n/no_think");
        }
    }
    let wired: Vec<serde_json::Value> = messages.iter().map(wire).collect();
    let mut body = serde_json::json!({ "model": model, "messages": wired, "stream": true });
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(|t| {
                serde_json::json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.parameters } })
            })
            .collect();
    }
    if let Some(t) = cfg.temperature {
        body["temperature"] = t.into();
    }
    if let Some(m) = cfg.max_tokens {
        body["max_tokens"] = m.into();
    }
    if let (Some(serde_json::Value::Object(extra)), Some(obj)) = (&cfg.extra, body.as_object_mut()) {
        let thinking_switch = extra.contains_key("chat_template_kwargs");
        for (k, v) in extra {
            obj.insert(k.clone(), v.clone());
        }
        if thinking_switch {
            obj["chat_template_kwargs"]["enable_thinking"] = cfg.thinking.into();
        }
    }
    body
}

#[async_trait]
impl LanguageModel for OpenAiCompatible {
    fn name(&self) -> String {
        let model = self.cfg.model.clone().or_else(|| self.resolved.get().cloned()).unwrap_or_else(|| "auto".into());
        format!("{model} @ {}", self.cfg.base_url)
    }

    async fn stream(&self, messages: &[Message]) -> Result<TokenStream, LlmError> {
        let model = self.model().await?;
        let budget = self.budget();
        let (first, tally) = self.open(&model, messages, budget, &[]).await?;
        let this = self.clone();
        let messages = messages.to_vec();
        // after the stream: nothing to add if she said something; otherwise either retry with room to
        // think (a model that thinks although told not to) or explain why the reply is empty
        let tail = futures::stream::once(async move {
            let (said, thought_out, whole, error) = {
                let t = tally.lock().unwrap_or_else(PoisonError::into_inner);
                let thought_out = t.reasoning_chars + t.hidden_chars > 0 && t.finish.as_deref() == Some("length");
                if !t.said {
                    tracing::warn!(
                        events = t.events,
                        reasoning_chars = t.reasoning_chars,
                        hidden_chars = t.hidden_chars,
                        finish = ?t.finish,
                        head = %t.head.chars().take(300).collect::<String>(),
                        "no visible text in the reply"
                    );
                }
                (t.said, thought_out, t.whole_reply(), t.empty_reply(budget))
            };
            if said {
                return futures::stream::empty().boxed();
            }
            if let Some(text) = whole {
                return futures::stream::once(async move { Ok(text) }).boxed();
            }
            if thought_out && !this.cfg.thinking && !this.thinks.swap(true, Ordering::Relaxed) {
                let bigger = this.budget();
                tracing::info!(model = %model, budget = ?bigger, "the model thinks even with thinking off, retrying with room for it");
                return match this.open(&model, &messages, bigger, &[]).await {
                    Ok((retry, t2)) => retry.chain(explain_if_empty(t2, bigger)).boxed(),
                    Err(e) => futures::stream::once(async move { Err(e) }).boxed(),
                };
            }
            futures::stream::once(async move { Err(error) }).boxed()
        })
        .flatten();
        Ok(Box::pin(first.chain(tail)))
    }

    fn supports_tools(&self) -> bool {
        self.cfg.tools && !self.no_tools.load(Ordering::Relaxed)
    }

    async fn respond(&self, messages: &[Message], tools: &[ToolSpec]) -> Result<PieceStream, LlmError> {
        if tools.is_empty() || !self.supports_tools() {
            return self.plain(messages).await;
        }
        let model = self.model().await?;
        let budget = self.budget();
        let (first, tally) = match self.open(&model, messages, budget, tools).await {
            Ok(opened) => opened,
            Err(LlmError::Backend(why)) if refuses_tools(&why) => {
                tracing::warn!(model = %model, reason = %why, "the server refused tools: plain chat from now on (llama-server needs --jinja for tools)");
                self.no_tools.store(true, Ordering::Relaxed);
                return self.plain(messages).await;
            }
            Err(e) => return Err(e),
        };
        let this = self.clone();
        let (messages, tools) = (messages.to_vec(), tools.to_vec());
        let tail = futures::stream::once(async move {
            let (said, calls, thought_out, whole, error) = {
                let t = tally.lock().unwrap_or_else(PoisonError::into_inner);
                let thought_out = t.reasoning_chars + t.hidden_chars > 0 && t.finish.as_deref() == Some("length");
                (t.said, t.tool_calls(), thought_out, t.whole_reply(), t.empty_reply(budget))
            };
            if !calls.is_empty() || said {
                let calls: Vec<Result<Piece, LlmError>> = calls.into_iter().map(|c| Ok(Piece::Call(c))).collect();
                return futures::stream::iter(calls).boxed();
            }
            if let Some(text) = whole {
                return futures::stream::once(async move { Ok(Piece::Text(text)) }).boxed();
            }
            if thought_out && !this.cfg.thinking && !this.thinks.swap(true, Ordering::Relaxed) {
                let bigger = this.budget();
                return match this.open(&model, &messages, bigger, &tools).await {
                    Ok((retry, t2)) => retry.map(|r| r.map(Piece::Text)).chain(calls_or_explain(t2, bigger)).boxed(),
                    Err(e) => futures::stream::once(async move { Err(e) }).boxed(),
                };
            }
            futures::stream::once(async move { Err(error) }).boxed()
        })
        .flatten();
        Ok(Box::pin(first.map(|r| r.map(Piece::Text)).chain(tail)))
    }
}

/// An error that means "this server doesn't do tool calling".
fn refuses_tools(why: &str) -> bool {
    let w = why.to_lowercase();
    (w.starts_with("400") || w.starts_with("422") || w.starts_with("500") || w.starts_with("501"))
        && (w.contains("tool") || w.contains("jinja") || w.contains("function"))
}

/// Ends a tool-calling stream with its calls, or an explanation when it carried nothing at all.
fn calls_or_explain(tally: Shared, max_tokens: Option<u32>) -> BoxStream<'static, Result<Piece, LlmError>> {
    futures::stream::once(async move {
        let t = tally.lock().unwrap_or_else(PoisonError::into_inner);
        let calls = t.tool_calls();
        if !calls.is_empty() {
            return futures::stream::iter(calls.into_iter().map(|c| Ok(Piece::Call(c))).collect::<Vec<_>>()).boxed();
        }
        if t.said {
            return futures::stream::empty().boxed();
        }
        if let Some(text) = t.whole_reply() {
            return futures::stream::once(async move { Ok(Piece::Text(text)) }).boxed();
        }
        let e = t.empty_reply(max_tokens);
        futures::stream::once(async move { Err(e) }).boxed()
    })
    .flatten()
    .boxed()
}

impl OpenAiCompatible {
    /// Text only, as pieces.
    async fn plain(&self, messages: &[Message]) -> Result<PieceStream, LlmError> {
        let text = self.stream(messages).await?;
        Ok(Box::pin(text.map(|r| r.map(Piece::Text))))
    }
}

type Shared = Arc<Mutex<Tally>>;

impl OpenAiCompatible {
    /// The reply budget, plus room for reasoning when the model thinks.
    fn budget(&self) -> Option<u32> {
        let thinks = self.cfg.thinking || self.thinks.load(Ordering::Relaxed);
        self.cfg.max_tokens.map(|m| if thinks { m.saturating_add(THINKING_BUDGET) } else { m })
    }

    /// Sends one request and returns its visible text as a stream, plus the tally of what it carried.
    async fn open(
        &self,
        model: &str,
        messages: &[Message],
        max_tokens: Option<u32>,
        tools: &[ToolSpec],
    ) -> Result<(BoxStream<'static, Result<String, LlmError>>, Shared), LlmError> {
        let cfg = OpenAiConfig { max_tokens, ..self.cfg.clone() };
        let body = build_body_with_tools(&cfg, model, messages, tools);
        let resp = self
            .auth(self.http.post(self.url("chat/completions")).json(&body))
            .send()
            .await
            .map_err(|e| LlmError::Unreachable(e.to_string()))?;
        let resp = check(resp).await?;

        let mut lines = sse::Lines::default();
        let mut think = ThinkFilter::default();
        let tally: Shared = Arc::default();
        let seen = tally.clone();
        let text = resp
            .bytes_stream()
            .map(move |chunk| -> Vec<Result<String, LlmError>> {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => return vec![Err(LlmError::Interrupted(e.to_string()))],
                };
                let mut t = seen.lock().unwrap_or_else(PoisonError::into_inner);
                if t.head.len() < 4096 {
                    t.head.push_str(&String::from_utf8_lossy(&chunk));
                }
                lines.push(&chunk).into_iter().filter_map(|data| absorb(&data, &mut t, &mut think).map(Ok)).collect()
            })
            .flat_map(futures::stream::iter)
            .boxed();
        Ok((text, tally))
    }
}

/// Ends a stream with an explanation when it carried no visible word.
fn explain_if_empty(tally: Shared, max_tokens: Option<u32>) -> BoxStream<'static, Result<String, LlmError>> {
    futures::stream::once(async move {
        let t = tally.lock().unwrap_or_else(PoisonError::into_inner);
        if t.said {
            None
        } else if let Some(text) = t.whole_reply() {
            Some(Ok(text))
        } else {
            tracing::warn!(events = t.events, reasoning_chars = t.reasoning_chars, hidden_chars = t.hidden_chars, finish = ?t.finish, "empty reply after retry");
            Some(Err(t.empty_reply(max_tokens)))
        }
    })
    .filter_map(futures::future::ready)
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat() -> Vec<Message> {
        vec![Message::system("be Capri"), Message::user("hi")]
    }

    #[test]
    fn llama_server_turns_thinking_off_in_the_template() {
        let body = build_body(&OpenAiConfig::llama_server("http://x/v1"), "gemma-3", &chat());
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn thinking_can_be_turned_back_on() {
        let cfg = OpenAiConfig { thinking: true, ..OpenAiConfig::llama_server("http://x/v1") };
        let body = build_body(&cfg, "qwen3-8b", &chat());
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], true);
        assert!(!body["messages"][1]["content"].as_str().unwrap().contains("/no_think"));
    }

    #[test]
    fn qwen3_gets_the_soft_switch_on_any_server() {
        let body = build_body(&OpenAiConfig::new("http://x/v1"), "Qwen3-14B-Q4_K_M.gguf", &chat());
        assert!(body["messages"][1]["content"].as_str().unwrap().ends_with("/no_think"));
        assert!(body.get("chat_template_kwargs").is_none(), "no unknown fields for strict servers");
    }

    fn run(payloads: &[&str]) -> (String, Tally) {
        let mut tally = Tally::default();
        let mut think = ThinkFilter::default();
        let text = payloads.iter().filter_map(|p| absorb(p, &mut tally, &mut think)).collect();
        (text, tally)
    }

    #[test]
    fn explains_a_reply_lost_to_thinking() {
        let (text, t) = run(&[
            r#"{"choices":[{"delta":{"reasoning_content":"Let me think about how to greet"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#,
            "[DONE]",
        ]);
        assert_eq!(text, "");
        assert!(t.empty_reply(Some(400)).to_string().contains("whole budget (400 tokens) thinking"));
    }

    #[test]
    fn takes_a_whole_reply_from_a_server_that_does_not_stream() {
        let t = Tally {
            head: r#"{"choices":[{"message":{"role":"assistant","content":"<think>x</think>Salut"}}]}"#.into(),
            ..Tally::default()
        };
        assert_eq!(t.whole_reply().as_deref(), Some("Salut"));
    }

    #[test]
    fn thinking_inside_content_counts_as_thinking() {
        let (text, t) = run(&[
            r#"{"choices":[{"delta":{"content":"<think>let me think about it"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#,
        ]);
        assert_eq!(text, "");
        assert!(t.hidden_chars > 0);
        assert!(t.empty_reply(Some(400)).to_string().contains("thinking"));
    }

    #[test]
    fn surfaces_errors_sent_mid_stream() {
        let (_, t) = run(&[r#"{"error":{"message":"context size exceeded"}}"#]);
        assert!(t.empty_reply(None).to_string().contains("context size exceeded"));
    }

    #[test]
    fn counts_visible_text() {
        let (text, t) = run(&[
            r#"{"choices":[{"delta":{"content":"Salut"}}]}"#,
            r#"{"choices":[{"delta":{"content":" choom"},"finish_reason":"stop"}]}"#,
        ]);
        assert_eq!(text, "Salut choom");
        assert!(t.said);
    }

    #[test]
    fn tool_calls_are_assembled_from_pieces() {
        let (text, t) = run(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"spotify","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"action\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"next\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);
        assert_eq!(text, "");
        let calls = t.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!((calls[0].id.as_str(), calls[0].name.as_str()), ("call_a", "spotify"));
        assert_eq!(calls[0].arguments, r#"{"action":"next"}"#);
    }

    #[test]
    fn tools_and_tool_turns_go_out_in_the_api_shape() {
        let spec = ToolSpec {
            name: "spotify".into(),
            description: "music".into(),
            parameters: serde_json::json!({"type": "object"}),
            risk: capri_domain::Risk::Reversible,
        };
        let call = ToolCall { id: "c1".into(), name: "spotify".into(), arguments: "{}".into() };
        let msgs = vec![
            Message::system("s"),
            Message::user("next"),
            Message::calling("", vec![call]),
            Message::tool_result("c1", "done"),
        ];
        let body = build_body_with_tools(&OpenAiConfig::new("http://x/v1"), "m", &msgs, &[spec]);
        assert_eq!(body["tools"][0]["function"]["name"], "spotify");
        assert_eq!(body["messages"][2]["tool_calls"][0]["function"]["arguments"], "{}");
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["messages"][3]["tool_call_id"], "c1");
        assert!(refuses_tools("500 Internal Server Error: tools param requires --jinja flag"));
        assert!(!refuses_tools("503: loading model"));
    }

    #[test]
    fn whole_completions_carry_tool_calls_too() {
        let t = Tally {
            head: r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"x","type":"function","function":{"name":"ui_windows","arguments":"{}"}}]}}]}"#.into(),
            ..Tally::default()
        };
        assert_eq!(t.tool_calls()[0].name, "ui_windows");
    }

    #[test]
    fn plain_servers_get_a_plain_body() {
        let body = build_body(&OpenAiConfig::new("https://api.openai.com/v1"), "gpt-4.1-mini", &chat());
        let keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys.len(), 5, "{keys:?}");
    }
}
