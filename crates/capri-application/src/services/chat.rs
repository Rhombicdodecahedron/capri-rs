//! Use case: the user says something, she answers (streamed), and memory is kept up to date.

use std::sync::{Arc, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use capri_domain::approval;
use capri_domain::intent::{self, Intent};
use capri_domain::{Conversation, Language, MediaCommand, Message, Origin, Persona, SpotifyAction, ToolCall, media};
use futures::StreamExt;
use tokio::sync::{Mutex, oneshot};

use crate::ports::{ActionRequest, ApprovalPrompt, LanguageModel, LlmError, MemoryStore, Piece, ToolError, ToolSpec};
use crate::services::ToolRunner;
use crate::services::spotify::SPOTIFY_TOOL;

/// Name of the music tool in the [`ToolRunner`].
pub const MEDIA_TOOL: &str = "media";

/// What the UI receives while a reply is being produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatEvent {
    Thinking,
    /// She needs a yes or no before doing something; the user's next line is the answer.
    Confirm(String),
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
    /// Tool rounds allowed for one message (look, then act, then check...).
    pub max_tool_rounds: usize,
    /// How long a yes/no question waits for an answer before it counts as no.
    pub confirm_timeout: Duration,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            persona: Persona::default(),
            history_turns: 24,
            recall_limit: 6,
            lang: Language::En,
            max_tool_rounds: 6,
            confirm_timeout: Duration::from_secs(90),
        }
    }
}

/// Added to the system prompt when the model gets tools.
const TOOLS_NOTE: &str = "\n\nYou have hands on the user's PC through the tools you are given. When they ask for \
something on the PC, call the tool now instead of describing it. To work in an app, read it with ui_snapshot, \
then act by ref with ui_click / ui_fill / ui_press, and snapshot again to check when it matters. For Spotify, \
use the spotify tool. The app asks the user to confirm risky actions itself: don't ask them first, and if a \
tool result says they said no, drop it. After the tools, answer in one short sentence, in their language, \
saying only what the results confirm.";

/// The yes/no slot: set while a question waits, taken by the next line the user sends.
type Pending = std::sync::Mutex<Option<oneshot::Sender<bool>>>;

pub struct ChatService {
    model: std::sync::RwLock<Arc<dyn LanguageModel>>,
    memory: Arc<dyn MemoryStore>,
    conversation: Mutex<Conversation>,
    settings: ChatSettings,
    lang: std::sync::RwLock<Language>,
    tools: Option<Arc<ToolRunner>>,
    pending: Pending,
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
            pending: std::sync::Mutex::new(None),
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
        // a question is waiting: this line answers it (anything but a clear yes is a no)
        let waiting = self.pending.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(tx) = waiting {
            let answer = approval::answer(line);
            let _ = tx.send(answer == Some(true));
            if answer.is_some() {
                return;
            }
        }
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

    /// Runs a music command: Spotify's own buttons when it is open, the system media keys
    /// otherwise. `None` when no music tool is plugged in: then the line goes to the model.
    async fn media(&self, cmd: &MediaCommand, l: Language) -> Option<String> {
        let tools = self.tools.as_ref()?;
        let on_spotify = match cmd {
            MediaCommand::Search(q) => Some(SpotifyAction::PlayQuery(q.clone())),
            MediaCommand::Play => Some(SpotifyAction::Play),
            MediaCommand::Pause => Some(SpotifyAction::Pause),
            MediaCommand::PlayPause => Some(SpotifyAction::Toggle),
            MediaCommand::Next => Some(SpotifyAction::Next),
            MediaCommand::Previous => Some(SpotifyAction::Previous),
            MediaCommand::Shuffle => Some(SpotifyAction::Shuffle),
            MediaCommand::Repeat => Some(SpotifyAction::Repeat),
            MediaCommand::Like => Some(SpotifyAction::Like),
            MediaCommand::NowPlaying => Some(SpotifyAction::NowPlaying),
            MediaCommand::OpenSpotify => Some(SpotifyAction::Open),
            MediaCommand::VolumeUp | MediaCommand::VolumeDown | MediaCommand::Mute => None,
        };
        let spotify_only =
            matches!(cmd, MediaCommand::Shuffle | MediaCommand::Repeat | MediaCommand::Like | MediaCommand::NowPlaying);
        if let Some(action) = on_spotify.filter(|_| tools.has(SPOTIFY_TOOL)) {
            let args = serde_json::to_value(&action).unwrap_or_default();
            match tools.run(SPOTIFY_TOOL, args, Origin::User).await {
                Ok(out) => return Some(music_reply(cmd, &out, l)),
                Err(ToolError::Unavailable(why)) if !spotify_only => {
                    tracing::debug!(%why, "spotify not available, using the media keys");
                }
                Err(ToolError::Unavailable(_)) => {
                    return Some(l.pick("Spotify isn't open.", "Spotify n'est pas ouvert.").into());
                }
                Err(e) => return Some(tool_failure(&e, l)),
            }
        }
        if spotify_only || !tools.has(MEDIA_TOOL) {
            return None;
        }
        let args = serde_json::to_value(cmd).unwrap_or_default();
        Some(match tools.run(MEDIA_TOOL, args, Origin::User).await {
            Ok(out) => music_reply(cmd, &out, l),
            Err(e) => tool_failure(&e, l),
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

        let reply = match self.answer(prompt, emit).await {
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

    /// The reply, with tool rounds in between when the brain can call tools.
    async fn answer(
        &self,
        mut prompt: Vec<Message>,
        emit: &mut (impl FnMut(ChatEvent) + Send),
    ) -> Result<String, LlmError> {
        let model = self.current_model();
        let tools = self.tools.clone().filter(|_| model.supports_tools());
        let specs: Vec<ToolSpec> = tools.as_ref().map(|t| t.specs()).unwrap_or_default();
        let Some(tools) = tools.filter(|_| !specs.is_empty()) else {
            return self.collect(&prompt, emit).await;
        };
        if let Some(system) = prompt.first_mut() {
            system.content.push_str(TOOLS_NOTE);
        }
        let mut said = String::new();
        let mut acted = false;
        for round in 0..self.settings.max_tool_rounds.max(1) {
            let (text, calls) = self.round(model.as_ref(), &prompt, &specs, emit).await?;
            if !text.trim().is_empty() {
                if !said.is_empty() {
                    said.push(' ');
                    emit(ChatEvent::Delta(" ".into()));
                }
                said.push_str(text.trim());
            }
            if calls.is_empty() || round + 1 == self.settings.max_tool_rounds {
                break;
            }
            acted = true;
            prompt.push(Message::calling(text, calls.clone()));
            for call in calls {
                let result = self.call(&tools, &call, &mut *emit).await;
                tracing::info!(tool = %call.name, result = %result.chars().take(200).collect::<String>(), "tool result");
                prompt.push(Message::tool_result(call.id, result));
            }
            emit(ChatEvent::Thinking);
        }
        let said = said.trim().to_string();
        if said.is_empty() {
            if acted {
                return Ok(self.lang().pick("Done.", "C'est fait.").into());
            }
            return Err(LlmError::Backend("empty reply".into()));
        }
        Ok(said)
    }

    /// One model turn: streams its text, gathers the tools it calls.
    async fn round(
        &self,
        model: &dyn LanguageModel,
        prompt: &[Message],
        specs: &[ToolSpec],
        emit: &mut (impl FnMut(ChatEvent) + Send),
    ) -> Result<(String, Vec<ToolCall>), LlmError> {
        let mut stream = model.respond(prompt, specs).await?;
        let (mut text, mut calls) = (String::new(), Vec::new());
        while let Some(piece) = stream.next().await {
            match piece? {
                Piece::Text(t) if !t.is_empty() => {
                    text.push_str(&t);
                    emit(ChatEvent::Delta(t));
                }
                Piece::Text(_) => {}
                Piece::Call(c) => calls.push(c),
            }
        }
        Ok((text, calls))
    }

    /// Runs one tool the model asked for; the text is what the model reads back.
    async fn call(&self, tools: &ToolRunner, call: &ToolCall, emit: &mut (dyn FnMut(ChatEvent) + Send)) -> String {
        let raw = if call.arguments.trim().is_empty() { "{}" } else { call.arguments.as_str() };
        let Ok(args) = serde_json::from_str::<serde_json::Value>(raw) else {
            return format!("error: the arguments are not valid JSON: {raw}");
        };
        let asker = AskInChat {
            emit: std::sync::Mutex::new(emit),
            pending: &self.pending,
            lang: self.lang(),
            timeout: self.settings.confirm_timeout,
        };
        match tools.run_asking(&call.name, args, Origin::Model, &asker).await {
            Ok(out) => out,
            Err(ToolError::Declined) => "the user said no: don't do it, and don't try another way".into(),
            Err(ToolError::Blocked) => "blocked: the user set Capri to read-only, nothing on the PC can change".into(),
            Err(e) => format!("error: {e}"),
        }
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

/// Her line after a music command ran.
fn music_reply(cmd: &MediaCommand, out: &str, l: Language) -> String {
    match out {
        "already playing" => l.pick("It's already playing.", "Ça joue déjà.").into(),
        "already paused" => l.pick("It's already paused.", "C'est déjà en pause.").into(),
        _ => cmd.reply(l, out),
    }
}

fn tool_failure(e: &ToolError, l: Language) -> String {
    match e {
        ToolError::Blocked => {
            l.pick("I'm in read-only mode, I can't touch that.", "Je suis en lecture seule, je n'y touche pas.").into()
        }
        ToolError::Declined => l.pick("Okay, I'll leave it.", "OK, je n'y touche pas.").into(),
        e => format!("{} {e}", l.pick("That didn't work:", "Ça n'a pas marché :")),
    }
}

/// Asks the user in the chat itself: the question goes out as [`ChatEvent::Confirm`] and the next
/// line they send (typed or said) is the answer.
struct AskInChat<'a> {
    emit: std::sync::Mutex<&'a mut (dyn FnMut(ChatEvent) + Send)>,
    pending: &'a Pending,
    lang: Language,
    timeout: Duration,
}

#[async_trait]
impl ApprovalPrompt for AskInChat<'_> {
    async fn ask(&self, request: &ActionRequest) -> bool {
        let (tx, rx) = oneshot::channel();
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
        let l = self.lang;
        let question = match request.risk {
            capri_domain::Risk::Destructive => format!(
                "{} {} {}",
                l.pick("This one can't be undone:", "Celle-là, on ne peut pas l'annuler :"),
                request.summary,
                l.pick("Go ahead? (yes / no)", "J'y vais ? (oui / non)")
            ),
            _ => format!(
                "{} {} {}",
                l.pick("Can I do this?", "Je peux ?"),
                request.summary,
                l.pick("(yes / no)", "(oui / non)")
            ),
        };
        {
            let mut emit = self.emit.lock().unwrap_or_else(PoisonError::into_inner);
            (*emit)(ChatEvent::Confirm(question));
        }
        if let Ok(Ok(yes)) = tokio::time::timeout(self.timeout, rx).await {
            yes
        } else {
            self.pending.lock().unwrap_or_else(PoisonError::into_inner).take();
            false
        }
    }
}
