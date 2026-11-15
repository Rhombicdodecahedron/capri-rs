//! One adapter for every OpenAI-compatible `/v1/chat/completions` server. Ollama, LM Studio,
//! llama.cpp's `llama-server`, vLLM, OpenAI and OpenRouter all speak this protocol, so they differ
//! only by base URL, model name, API key and a few request extras.

use async_trait::async_trait;
use capri_application::ports::{LanguageModel, LlmError, TokenStream};
use capri_domain::{Message, Role};
use futures::StreamExt;
use serde::Deserialize;
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
        }
    }

    /// llama.cpp's `llama-server`: thinking is switched off through the chat template when asked.
    pub fn llama_server(base_url: impl Into<String>) -> Self {
        let mut c = Self::new(base_url);
        c.extra = Some(serde_json::json!({ "chat_template_kwargs": { "enable_thinking": false } }));
        c
    }
}

#[derive(Debug)]
pub struct OpenAiCompatible {
    cfg: OpenAiConfig,
    http: reqwest::Client,
    resolved: OnceCell<String>,
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
}

/// What the stream carried besides visible text, to explain an empty answer.
#[derive(Debug, Default)]
struct Tally {
    said: bool,
    reasoning_chars: usize,
    finish: Option<String>,
    error: Option<String>,
}

impl Tally {
    /// The error for a stream that ended without a visible word.
    fn empty_reply(&self, max_tokens: Option<u32>) -> LlmError {
        if let Some(e) = &self.error {
            return LlmError::Backend(e.clone());
        }
        let budget = max_tokens.map_or_else(String::new, |m| format!(" ({m} tokens)"));
        match (self.reasoning_chars, self.finish.as_deref()) {
            (r, Some("length")) if r > 0 => LlmError::Backend(format!(
                "the model spent its whole budget{budget} thinking and never answered: raise the max reply length, \
                 or use a model or server option that turns thinking off"
            )),
            (r, _) if r > 0 => LlmError::Backend("the model only thought and gave no answer".into()),
            (_, Some("length")) => LlmError::Backend(format!("the model hit the length limit{budget} before any text")),
            _ => LlmError::Backend("empty reply".into()),
        }
    }
}

/// One SSE payload: the visible text it carries, after updating the tally.
fn absorb(data: &str, tally: &mut Tally, think: &mut ThinkFilter) -> Option<String> {
    if data == "[DONE]" {
        return None;
    }
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
        if c.finish_reason.is_some() {
            tally.finish = c.finish_reason;
        }
    }
    let visible = think.feed(&text);
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
        Self { cfg, http, resolved: OnceCell::new() }
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

/// The request body. Pure, so the exact shape sent to each server is unit-tested.
pub fn build_body(cfg: &OpenAiConfig, model: &str, messages: &[Message]) -> serde_json::Value {
    let mut messages: Vec<Message> = messages.to_vec();
    // Qwen3 reads a `/no_think` switch in the user turn, on any server: the portable way to skip
    // its reasoning when the server has no option for it.
    if !cfg.thinking && model.to_lowercase().contains("qwen3") {
        if let Some(last) = messages.iter_mut().rev().find(|m| m.role == Role::User) {
            last.content.push_str("\n\n/no_think");
        }
    }
    let mut body = serde_json::json!({ "model": model, "messages": messages, "stream": true });
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
        let body = build_body(&self.cfg, &model, messages);
        let resp = self
            .auth(self.http.post(self.url("chat/completions")).json(&body))
            .send()
            .await
            .map_err(|e| LlmError::Unreachable(e.to_string()))?;
        let resp = check(resp).await?;

        let bytes = resp.bytes_stream();
        let mut lines = sse::Lines::default();
        let mut think = ThinkFilter::default();
        let tally = std::sync::Arc::new(std::sync::Mutex::new(Tally::default()));
        let seen = tally.clone();
        let max_tokens = self.cfg.max_tokens;
        let body = bytes
            .map(move |chunk| -> Vec<Result<String, LlmError>> {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => return vec![Err(LlmError::Interrupted(e.to_string()))],
                };
                let mut t = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                lines.push(&chunk).into_iter().filter_map(|data| absorb(&data, &mut t, &mut think).map(Ok)).collect()
            })
            .flat_map(futures::stream::iter);
        // when the stream ends without a visible word, say why instead of returning nothing
        let end = futures::stream::once(async move {
            let t = tally.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if t.said {
                None
            } else {
                tracing::warn!(reasoning_chars = t.reasoning_chars, finish = ?t.finish, error = ?t.error, "empty reply");
                Some(Err(t.empty_reply(max_tokens)))
            }
        })
        .filter_map(futures::future::ready);
        Ok(Box::pin(body.chain(end)))
    }
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
    fn plain_servers_get_a_plain_body() {
        let body = build_body(&OpenAiConfig::new("https://api.openai.com/v1"), "gpt-4.1-mini", &chat());
        let keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys.len(), 5, "{keys:?}");
    }
}
