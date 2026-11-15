//! Service tests with in-memory fakes: no network, no disk, no windows.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use capri_domain::{
    ApprovalMode, Character, CharacterConfig, Ledge, LedgeId, LedgeKind, Message, Mode, Origin, Rect, Risk,
};
use tokio::sync::Mutex;

use crate::ports::*;
use crate::services::*;

struct ScriptedModel {
    pieces: Vec<&'static str>,
    seen: Mutex<Vec<Vec<Message>>>,
}

#[async_trait]
impl LanguageModel for ScriptedModel {
    fn name(&self) -> String {
        "scripted".into()
    }

    async fn stream(&self, messages: &[Message]) -> Result<TokenStream, LlmError> {
        self.seen.lock().await.push(messages.to_vec());
        let items: Vec<Result<String, LlmError>> = self.pieces.iter().map(|p| Ok((*p).to_string())).collect();
        Ok(Box::pin(futures::stream::iter(items)))
    }
}

struct FailingModel;

#[async_trait]
impl LanguageModel for FailingModel {
    fn name(&self) -> String {
        "down".into()
    }

    async fn stream(&self, _: &[Message]) -> Result<TokenStream, LlmError> {
        Err(LlmError::Unreachable("connection refused".into()))
    }
}

#[derive(Default)]
struct VecMemory {
    facts: Mutex<Vec<String>>,
    log: Mutex<Vec<Message>>,
}

#[async_trait]
impl MemoryStore for VecMemory {
    async fn remember(&self, fact: &str) -> Result<(), MemoryError> {
        self.facts.lock().await.push(fact.into());
        Ok(())
    }

    async fn recall(&self, _: &str, limit: usize) -> Result<Vec<String>, MemoryError> {
        Ok(self.facts.lock().await.iter().take(limit).cloned().collect())
    }

    async fn forget(&self, needle: &str) -> Result<usize, MemoryError> {
        let mut f = self.facts.lock().await;
        let before = f.len();
        f.retain(|x| !x.contains(needle));
        Ok(before - f.len())
    }

    async fn log_turn(&self, turn: &Message) -> Result<(), MemoryError> {
        self.log.lock().await.push(turn.clone());
        Ok(())
    }

    async fn recent_turns(&self, limit: usize) -> Result<Vec<Message>, MemoryError> {
        let l = self.log.lock().await;
        Ok(l.iter().rev().take(limit).rev().cloned().collect())
    }
}

async fn run(chat: &ChatService, line: &str) -> Vec<ChatEvent> {
    let mut events = Vec::new();
    chat.send(line, |e| events.push(e)).await;
    events
}

#[tokio::test]
async fn streams_reply_and_logs_both_turns() {
    let model = Arc::new(ScriptedModel { pieces: vec!["Hey ", "choom."], seen: Mutex::default() });
    let memory = Arc::new(VecMemory::default());
    let chat = ChatService::new(model.clone(), memory.clone(), ChatSettings::default());

    let events = run(&chat, "hi").await;
    assert_eq!(events.first(), Some(&ChatEvent::Thinking));
    assert_eq!(events.last(), Some(&ChatEvent::Done("Hey choom.".into())));
    assert_eq!(memory.log.lock().await.len(), 2);
}

#[tokio::test]
async fn remembered_facts_reach_the_prompt_without_calling_the_model_for_the_command() {
    let model = Arc::new(ScriptedModel { pieces: vec!["ok"], seen: Mutex::default() });
    let memory = Arc::new(VecMemory::default());
    let chat = ChatService::new(model.clone(), memory.clone(), ChatSettings::default());

    run(&chat, "remember that I drive a 1989 E30").await;
    assert!(model.seen.lock().await.is_empty(), "memory commands are handled locally");

    run(&chat, "what do I drive?").await;
    let seen = model.seen.lock().await;
    assert!(seen[0][0].content.contains("1989 E30"));
}

#[tokio::test]
async fn model_failure_is_reported_not_logged() {
    let memory = Arc::new(VecMemory::default());
    let chat = ChatService::new(Arc::new(FailingModel), memory.clone(), ChatSettings::default());
    let events = run(&chat, "hi").await;
    assert!(matches!(events.last(), Some(ChatEvent::Failed(f)) if f.kind == FailureKind::Unreachable));
    assert_eq!(memory.log.lock().await.len(), 0);
}

struct Echo(Risk);

#[async_trait]
impl Tool for Echo {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "echo".into(), description: "echo".into(), parameters: serde_json::json!({}), risk: self.0 }
    }

    async fn run(&self, args: serde_json::Value) -> Result<String, ToolError> {
        Ok(args.to_string())
    }
}

struct Answer(bool, AtomicBool);

#[async_trait]
impl ApprovalPrompt for Answer {
    async fn ask(&self, _: &ActionRequest) -> bool {
        self.1.store(true, Ordering::SeqCst);
        self.0
    }
}

#[tokio::test]
async fn destructive_tools_need_a_yes() {
    let no = Arc::new(Answer(false, AtomicBool::new(false)));
    let runner = ToolRunner::new(ApprovalMode::Auto, no.clone()).with(Arc::new(Echo(Risk::Destructive)));
    let r = runner.run("echo", serde_json::json!({"rm": "x"}), Origin::User).await;
    assert!(matches!(r, Err(ToolError::Declined)));
    assert!(no.1.load(Ordering::SeqCst));
}

#[tokio::test]
async fn read_only_blocks_and_reads_pass() {
    let yes = Arc::new(Answer(true, AtomicBool::new(false)));
    let runner = ToolRunner::new(ApprovalMode::ReadOnly, yes.clone()).with(Arc::new(Echo(Risk::Reversible)));
    assert!(matches!(runner.run("echo", serde_json::json!({}), Origin::User).await, Err(ToolError::Blocked)));
    let reader = ToolRunner::new(ApprovalMode::ReadOnly, yes.clone()).with(Arc::new(Echo(Risk::Read)));
    assert!(reader.run("echo", serde_json::json!(1), Origin::Model).await.is_ok());
    assert!(!yes.1.load(Ordering::SeqCst), "reads never prompt");
}

struct OneScreen;

impl DesktopProbe for OneScreen {
    fn work_areas(&self) -> Vec<Rect> {
        vec![Rect::new(0.0, 0.0, 1920.0, 1040.0)]
    }

    fn ledges(&self, ignore: &[u64]) -> Vec<Ledge> {
        let mut v = vec![Ledge { id: LedgeId::Floor(0), kind: LedgeKind::Floor, left: 0.0, right: 1920.0, y: 1040.0 }];
        if !ignore.contains(&99) {
            v.push(Ledge { id: LedgeId::Window(99), kind: LedgeKind::Window, left: 1000.0, right: 1800.0, y: 300.0 });
        }
        v
    }
}

#[test]
fn desk_ignores_her_own_window() {
    let cfg = CharacterConfig { roaming: false, ..CharacterConfig::default() };
    let mut desk = DeskService::new(Character::new(cfg, 1), Arc::new(OneScreen));
    desk.set_own_windows(vec![99]);
    desk.start(0.0);
    let mut t = 0.0;
    while t < 3.0 {
        t += 1.0 / 60.0;
        desk.tick(t, 1.0 / 60.0);
    }
    assert_eq!(desk.character().mode(), Mode::Standing, "fell through her own window onto the floor");
}

/// A window the test drags around; the full refresh is slow.
struct SlidingWindow {
    x: std::sync::Mutex<f32>,
}

impl DesktopProbe for SlidingWindow {
    fn work_areas(&self) -> Vec<Rect> {
        vec![Rect::new(0.0, 0.0, 1920.0, 1040.0)]
    }

    fn ledges(&self, _: &[u64]) -> Vec<Ledge> {
        let x = *self.x.lock().unwrap();
        vec![
            Ledge { id: LedgeId::Floor(0), kind: LedgeKind::Floor, left: 0.0, right: 1920.0, y: 1040.0 },
            Ledge { id: LedgeId::window_piece(7, 0), kind: LedgeKind::Window, left: x, right: x + 800.0, y: 300.0 },
        ]
    }

    fn track(&self, window: u64) -> Tracking {
        assert_eq!(window, 7);
        Tracking::At(Rect::new(*self.x.lock().unwrap(), 300.0, 800.0, 500.0))
    }
}

#[test]
fn desk_follows_a_dragged_window_every_frame() {
    let cfg = CharacterConfig { roaming: false, ..CharacterConfig::default() };
    let window = Arc::new(SlidingWindow { x: std::sync::Mutex::new(500.0) });
    let mut desk = DeskService::new(Character::new(cfg, 1), window.clone());
    desk.start(0.0);
    desk.character_mut().appear(capri_domain::Point::new(900.0, 100.0), 0.0);
    let mut t = 0.0;
    while desk.character().mode() != Mode::Perching {
        t += 1.0 / 60.0;
        desk.tick(t, 1.0 / 60.0);
        assert!(t < 3.0, "never landed");
    }
    // the user drags the window 10 px per frame: she must follow every frame, not every refresh
    let before = desk.character().position().x;
    for _ in 0..5 {
        *window.x.lock().unwrap() += 10.0;
        t += 1.0 / 60.0;
        desk.tick(t, 1.0 / 60.0);
        let step = desk.character().position().x - before;
        assert!(step > 0.0, "did not move this frame");
    }
    let moved = desk.character().position().x - before;
    assert!((moved - 50.0).abs() < 0.5, "moved {moved}");
}
