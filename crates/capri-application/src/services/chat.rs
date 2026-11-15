//! Use case: the user says something, she answers (streamed), and memory is kept up to date.

use std::sync::Arc;

use capri_domain::intent::{self, Intent};
use capri_domain::{Conversation, Language, MediaCommand, Message, Origin, Persona, media};
use futures::StreamExt;
use tokio::sync::Mutex;

use crate::ports::{LanguageModel, LlmError, MemoryStore, ToolError};
use crate::services::ToolRunner;

/// Name of the music tool in the [`ToolRunner`].
pub const MEDIA_TOOL: &str = "media";

/// What the UI receives while a reply is being produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatEvent {
    Thinking,
    Delta(String),
    Done(String),
    Failed(ChatFailure),
}

/// Why a reply failed, so each front end can say it in its own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatFailure {
    pub kind: FailureKind,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Nothing answered at the configured address.
    Unreachable,
    /// The server refused the key or the sign-in.
    SignedOut,
    /// The server answered with an error, or the reply broke off.
    Backend,
}

impl From<&LlmError> for ChatFailure {
    fn from(e: &LlmError) -> Self {
        let kind = match e {
            LlmError::Unreachable(_) => FailureKind::Unreachable,
            LlmError::SignedOut(_) => FailureKind::SignedOut,
            LlmError::Backend(_) | LlmError::Interrupted(_) => FailureKind::Backend,
        };
        Self { kind, detail: e.to_string() }
    }
}

#[derive(Debug, Clone)]
pub struct ChatSettings {
    pub persona: Persona,
    pub history_turns: usize,
    pub recall_limit: usize,
    /// Language of the replies she gives without the model (memory, music commands).
    pub lang: Language,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self { persona: Persona::default(), history_turns: 24, recall_limit: 6, lang: Language::En }
    }
}

pub struct ChatService {
    model: std::sync::RwLock<Arc<dyn LanguageModel>>,
    memory: Arc<dyn MemoryStore>,
    conversation: Mutex<Conversation>,
    settings: ChatSettings,
    lang: std::sync::RwLock<Language>,
    tools: Option<Arc<ToolRunner>>,
}

impl std::fmt::Debug for ChatService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatService").field("model", &self.model_name()).finish_non_exhaustive()
    }
}

impl ChatService {
    pub fn new(model: Arc<dyn LanguageModel>, memory: Arc<dyn MemoryStore>, settings: ChatSettings) -> Self {
        let conversation = Conversation::new(settings.persona.clone(), settings.history_turns);
        let lang = std::sync::RwLock::new(settings.lang);
        Self {
            model: std::sync::RwLock::new(model),
            memory,
            conversation: Mutex::new(conversation),
            settings,
            lang,
            tools: None,
        }
    }

    /// Abilities she can use (music...). Every call goes through the runner's approval policy.
    #[must_use]
    pub fn with_tools(mut self, tools: Arc<ToolRunner>) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn set_language(&self, lang: Language) {
        if let Ok(mut l) = self.lang.write() {
            *l = lang;
        }
    }

    fn lang(&self) -> Language {
        self.lang.read().map_or(Language::En, |l| *l)
    }

    fn current_model(&self) -> Arc<dyn LanguageModel> {
        self.model.read().map_or_else(|e| e.into_inner().clone(), |m| m.clone())
    }

    pub fn model_name(&self) -> String {
        self.current_model().name()
    }

    /// Swaps the brain live (new settings saved); the conversation carries on.
    pub fn set_model(&self, model: Arc<dyn LanguageModel>) {
        match self.model.write() {
            Ok(mut m) => *m = model,
            Err(e) => *e.into_inner() = model,
        }
    }

    pub async fn set_persona(&self, persona: Persona) {
        self.conversation.lock().await.set_persona(persona);
    }

    /// Picks up the previous session's last turns so she does not start from a blank slate.
    pub async fn resume(&self) {
        let Ok(turns) = self.memory.recent_turns(self.settings.history_turns).await else {
            return;
        };
        let mut c = self.conversation.lock().await;
        for t in turns {
            c.push(t);
        }
    }

    /// Handles one line from the user. `emit` is called for every event, from the calling task.
    pub async fn send(&self, line: &str, mut emit: impl FnMut(ChatEvent) + Send) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let l = self.lang();
        if let Some(cmd) = media::parse(line) {
            if let Some(reply) = self.media(&cmd, l).await {
                emit(ChatEvent::Done(reply));
                return;
            }
        }
        match intent::parse(line) {
            Intent::NewChat => {
                self.conversation.lock().await.clear();
                emit(ChatEvent::Done(l.pick("Fresh slate. What's next?", "On repart de zéro. Quoi de neuf ?").into()));
            }
            Intent::Remember(fact) => {
                let reply = match self.memory.remember(&fact).await {
                    Ok(()) => format!("{} {fact}", l.pick("Got it, I'll remember:", "C'est noté, je retiens :")),
                    Err(e) => format!("{} ({e}).", l.pick("I couldn't save that", "Je n'ai pas pu l'enregistrer")),
                };
                emit(ChatEvent::Done(reply));
            }
            Intent::Forget(what) => {
                let reply = match self.memory.forget(&what).await {
                    Ok(0) => l.pick("I had nothing like that saved.", "Je n'avais rien de tel en mémoire.").to_string(),
                    Ok(n) => format!("{} ({n}).", l.pick("Forgotten", "Oublié")),
                    Err(e) => format!("{} ({e}).", l.pick("I couldn't forget that", "Je n'ai pas pu l'oublier")),
                };
                emit(ChatEvent::Done(reply));
            }
            Intent::Chat => self.chat(line, &mut emit).await,
        }
    }

    /// Runs a music command through the `media` tool. `None` when no such tool is plugged in: then
    /// the line goes to the model like any other.
    async fn media(&self, cmd: &MediaCommand, l: Language) -> Option<String> {
        let tools = self.tools.as_ref()?;
        if !tools.has(MEDIA_TOOL) {
            return None;
        }
        let args = serde_json::to_value(cmd).unwrap_or_default();
        Some(match tools.run(MEDIA_TOOL, args, Origin::User).await {
            Ok(_) => cmd.reply(l),
            Err(ToolError::Blocked) => l
                .pick(
                    "I'm in read-only mode, I can't touch your music.",
                    "Je suis en lecture seule, je ne touche pas à ta musique.",
                )
                .into(),
            Err(ToolError::Declined) => l.pick("Okay, I'll leave it.", "OK, je n'y touche pas.").into(),
            Err(e) => format!("{} {e}", l.pick("That didn't work:", "Ça n'a pas marché :")),
        })
    }

    async fn chat(&self, line: &str, emit: &mut (impl FnMut(ChatEvent) + Send)) {
        emit(ChatEvent::Thinking);
        let recalled = self.memory.recall(line, self.settings.recall_limit).await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "recall failed, answering without memory");
            Vec::new()
        });
        let user = Message::user(line);
        let prompt = {
            let mut c = self.conversation.lock().await;
            c.push(user.clone());
            c.prompt(&recalled)
        };

        let reply = match self.collect(&prompt, emit).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, model = %self.model_name(), "reply failed");
                self.conversation.lock().await.drop_unanswered();
                emit(ChatEvent::Failed(ChatFailure::from(&e)));
                return;
            }
        };

        let assistant = Message::assistant(reply.clone());
        self.conversation.lock().await.push(assistant.clone());
        for turn in [&user, &assistant] {
            if let Err(e) = self.memory.log_turn(turn).await {
                tracing::warn!(error = %e, "could not log turn");
            }
        }
        emit(ChatEvent::Done(reply));
    }

    async fn collect(&self, prompt: &[Message], emit: &mut (impl FnMut(ChatEvent) + Send)) -> Result<String, LlmError> {
        let model = self.current_model();
        let mut stream = model.stream(prompt).await?;
        let mut reply = String::new();
        while let Some(piece) = stream.next().await {
            let piece = piece?;
            if !piece.is_empty() {
                reply.push_str(&piece);
                emit(ChatEvent::Delta(piece));
            }
        }
        let reply = reply.trim().to_string();
        if reply.is_empty() {
            return Err(LlmError::Backend("empty reply".into()));
        }
        Ok(reply)
    }
}
