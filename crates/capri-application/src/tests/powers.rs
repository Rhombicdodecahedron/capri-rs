//! Her hands: Spotify through its buttons, the generic `ui_*` tools, and tool calls from the model
//! with the yes/no asked in the chat. A fake desktop stands in for UI Automation.

use std::sync::Mutex as StdMutex;
use std::time::Duration;

use capri_domain::ui::{Element, KeyChord, Locator, Role as UiRole, Snapshot, WindowInfo, WindowQuery};
use capri_domain::{Language, SpotifyAction, ToolCall};

use super::*;

/// A desktop of windows whose controls the test scripts. Every snapshot pops the next scripted
/// state of a window, so a page can "load" after a few looks.
#[derive(Default)]
struct FakeDesk {
    windows: StdMutex<Vec<WindowInfo>>,
    /// Successive control lists per window id; the last one stays.
    pages: StdMutex<Vec<(u64, Vec<Vec<Element>>)>>,
    acts: StdMutex<Vec<(u64, String, UiAction)>>,
    keys: StdMutex<Vec<String>>,
}

impl FakeDesk {
    fn window(self, id: u64, title: &str, process: &str, pages: Vec<Vec<Element>>) -> Self {
        self.windows.lock().unwrap().push(WindowInfo { id, title: title.into(), process: process.into(), pid: 1 });
        self.pages.lock().unwrap().push((id, pages));
        self
    }

    fn elements(&self, id: u64, advance: bool) -> Option<Vec<Element>> {
        let mut pages = self.pages.lock().unwrap();
        let (_, list) = pages.iter_mut().find(|(w, _)| *w == id)?;
        let page = list.first().cloned().unwrap_or_default();
        if advance && list.len() > 1 {
            list.remove(0);
        }
        Some(page)
    }

    fn find(&self, q: &WindowQuery) -> Result<WindowInfo, UiError> {
        self.windows.lock().unwrap().iter().find(|w| q.matches(w)).cloned().ok_or(UiError::NoWindow(format!("{q:?}")))
    }

    fn acted(&self) -> Vec<String> {
        self.acts.lock().unwrap().iter().map(|(_, n, _)| n.clone()).collect()
    }
}

#[async_trait]
impl UiDriver for FakeDesk {
    async fn windows(&self) -> Result<Vec<WindowInfo>, UiError> {
        Ok(self.windows.lock().unwrap().clone())
    }

    async fn snapshot(&self, q: &WindowQuery) -> Result<Snapshot, UiError> {
        let window = self.find(q)?;
        let elements = self.elements(window.id, true).unwrap_or_default();
        Ok(Snapshot { window, elements })
    }

    async fn act(&self, q: &WindowQuery, target: &Locator, action: &UiAction) -> Result<String, UiError> {
        let w = self.find(q)?;
        let els = self.elements(w.id, false).unwrap_or_default();
        let i = target.pick(&els).ok_or_else(|| UiError::NotFound(target.describe()))?;
        self.acts.lock().unwrap().push((w.id, els[i].name.clone(), action.clone()));
        Ok(els[i].name.clone())
    }

    async fn press(&self, q: &WindowQuery, keys: &KeyChord) -> Result<(), UiError> {
        self.find(q)?;
        self.keys.lock().unwrap().push(keys.to_string());
        Ok(())
    }
}

#[derive(Default)]
struct FakeOpener(StdMutex<Vec<String>>);

impl Opener for FakeOpener {
    fn open(&self, target: &str) -> Result<(), String> {
        self.0.lock().unwrap().push(target.into());
        Ok(())
    }
}

fn button(name: &str, y: f32) -> Element {
    Element::new(UiRole::Button, name).at(Rect::new(0.0, y, 30.0, 30.0))
}

fn player_bar(playing: bool) -> Vec<Element> {
    vec![
        button("Précédent", 1000.0),
        button(if playing { "Pause" } else { "Lecture" }, 1000.0),
        button("Suivant", 1000.0),
        button("Activer la lecture aléatoire", 1000.0),
    ]
}

fn quick() -> Patience {
    Patience { launch: Duration::from_secs(5), results: Duration::from_secs(4), poll: Duration::from_millis(100) }
}

fn remote(desk: &Arc<FakeDesk>, opener: &Arc<FakeOpener>) -> SpotifyRemote {
    SpotifyRemote::new(desk.clone(), opener.clone()).with_patience(quick())
}

#[tokio::test(start_paused = true)]
async fn spotify_buttons_are_pressed_by_name_in_any_language() {
    let desk = Arc::new(FakeDesk::default().window(5, "Spotify Premium", "Spotify.exe", vec![player_bar(false)]));
    let opener = Arc::new(FakeOpener::default());
    let s = remote(&desk, &opener);
    assert_eq!(s.perform(&SpotifyAction::Next).await.unwrap(), "done");
    assert_eq!(s.perform(&SpotifyAction::Play).await.unwrap(), "done");
    assert_eq!(s.perform(&SpotifyAction::Shuffle).await.unwrap(), "done");
    assert_eq!(desk.acted(), ["Suivant", "Lecture", "Activer la lecture aléatoire"]);
    assert_eq!(s.perform(&SpotifyAction::Pause).await.unwrap(), "already paused");
    assert!(opener.0.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn missing_buttons_fall_back_to_spotify_shortcuts() {
    let desk = Arc::new(FakeDesk::default().window(5, "Spotify Free", "Spotify.exe", vec![vec![]]));
    let s = remote(&desk, &Arc::new(FakeOpener::default()));
    s.perform(&SpotifyAction::Next).await.unwrap();
    s.perform(&SpotifyAction::Like).await.unwrap();
    assert_eq!(*desk.keys.lock().unwrap(), ["ctrl+right", "alt+shift+b"]);
}

#[tokio::test(start_paused = true)]
async fn play_query_searches_then_clicks_the_matching_result() {
    // the old page is still up for two looks, then the results load
    let old = vec![button("Play", 1000.0), button("Play Around the World by Daft Punk", 300.0)];
    let results =
        vec![button("Play", 1000.0), button("Play Daft Punk", 200.0), button("Play Get Lucky by Daft Punk", 400.0)];
    let desk = Arc::new(FakeDesk::default().window(5, "Spotify", "Spotify.exe", vec![old.clone(), old, results]));
    let opener = Arc::new(FakeOpener::default());
    let s = remote(&desk, &opener);
    let out = s.perform(&SpotifyAction::PlayQuery("get lucky".into())).await.unwrap();
    assert_eq!(out, "playing Get Lucky by Daft Punk");
    assert_eq!(*opener.0.lock().unwrap(), ["spotify:search:get%20lucky"]);
    assert_eq!(desk.acted(), ["Play Get Lucky by Daft Punk"]);
}

#[tokio::test(start_paused = true)]
async fn play_query_starts_spotify_when_it_is_closed() {
    struct Launches(Arc<FakeDesk>, StdMutex<Vec<String>>);
    impl Opener for Launches {
        fn open(&self, target: &str) -> Result<(), String> {
            self.1.lock().unwrap().push(target.into());
            let mut w = self.0.windows.lock().unwrap();
            if w.is_empty() {
                w.push(WindowInfo { id: 9, title: "Spotify".into(), process: "Spotify.exe".into(), pid: 2 });
                self.0.pages.lock().unwrap().push((9, vec![vec![button("Play Nightcall by Kavinsky", 300.0)]]));
            }
            Ok(())
        }
    }
    let desk = Arc::new(FakeDesk::default());
    let opener = Arc::new(Launches(desk.clone(), StdMutex::default()));
    let s = SpotifyRemote::new(desk.clone(), opener.clone()).with_patience(quick());
    assert_eq!(s.perform(&SpotifyAction::PlayQuery("kavinsky".into())).await.unwrap(), "playing Nightcall by Kavinsky");
}

#[tokio::test(start_paused = true)]
async fn the_title_tells_what_plays_and_a_closed_app_is_unavailable() {
    let desk = Arc::new(FakeDesk::default().window(5, "Daft Punk - Get Lucky", "Spotify.exe", vec![player_bar(true)]));
    let s = remote(&desk, &Arc::new(FakeOpener::default()));
    assert_eq!(s.perform(&SpotifyAction::NowPlaying).await.unwrap(), "playing Daft Punk - Get Lucky");
    let closed = remote(&Arc::new(FakeDesk::default()), &Arc::new(FakeOpener::default()));
    assert!(matches!(closed.perform(&SpotifyAction::Next).await, Err(ToolError::Unavailable(_))));
}

fn music_chat(desk: &Arc<FakeDesk>, spy: &Arc<MediaSpy>) -> ChatService {
    let model = Arc::new(ScriptedModel { pieces: vec!["nope"], seen: Mutex::default() });
    let tools = Arc::new(
        ToolRunner::new(ApprovalMode::Ask, Arc::new(Answer(false, AtomicBool::new(false))))
            .with(spy.clone())
            .with(Arc::new(remote(desk, &Arc::new(FakeOpener::default())))),
    );
    let settings = ChatSettings { lang: Language::Fr, ..ChatSettings::default() };
    ChatService::new(model, Arc::new(VecMemory::default()), settings).with_tools(tools)
}

#[tokio::test(start_paused = true)]
async fn typed_music_commands_use_spotify_buttons_then_media_keys() {
    let spy = Arc::new(MediaSpy(std::sync::Mutex::default()));
    let desk = Arc::new(FakeDesk::default().window(5, "Spotify Premium", "Spotify.exe", vec![player_bar(false)]));
    let chat = music_chat(&desk, &spy);
    assert_eq!(run(&chat, "chanson suivante").await, vec![ChatEvent::Done("Suivante.".into())]);
    assert_eq!(desk.acted(), ["Suivant"]);
    assert!(spy.0.lock().unwrap().is_empty(), "Spotify was open: its own button, not the media key");

    let closed = music_chat(&Arc::new(FakeDesk::default()), &spy);
    assert_eq!(run(&closed, "chanson suivante").await, vec![ChatEvent::Done("Suivante.".into())]);
    assert_eq!(spy.0.lock().unwrap()[0]["action"], "next", "no Spotify: any player through the media keys");
    assert_eq!(run(&closed, "like cette chanson").await, vec![ChatEvent::Done("Spotify n'est pas ouvert.".into())]);
}

#[tokio::test(start_paused = true)]
async fn mets_x_sur_spotify_plays_it() {
    let spy = Arc::new(MediaSpy(std::sync::Mutex::default()));
    let results = vec![button("Play", 1000.0), button("Play Alors on danse by Stromae", 300.0)];
    let desk = Arc::new(FakeDesk::default().window(5, "Spotify", "Spotify.exe", vec![results]));
    let chat = music_chat(&desk, &spy);
    let events = run(&chat, "mets Stromae sur Spotify").await;
    assert_eq!(events, vec![ChatEvent::Done("Je lance Alors on danse by Stromae.".into())]);
}

// -- the ui_* tools ---------------------------------------------------------------------------

fn hands_runner(desk: &Arc<FakeDesk>, mode: ApprovalMode, yes: bool) -> (ToolRunner, Arc<Answer>) {
    let answer = Arc::new(Answer(yes, AtomicBool::new(false)));
    let hands = Hands::new(desk.clone(), vec!["journal".into()]);
    (ToolRunner::new(mode, answer.clone()).with_all(hands.tools()), answer)
}

fn notepad() -> FakeDesk {
    FakeDesk::default()
        .window(
            1,
            "Sans titre - Bloc-notes",
            "notepad.exe",
            vec![vec![
                Element::new(UiRole::Edit, "Éditeur de texte"),
                Element::new(UiRole::Button, "Fermer"),
                Element::new(UiRole::Button, "Supprimer"),
            ]],
        )
        .window(2, "Bitwarden", "Bitwarden.exe", vec![vec![Element::new(UiRole::Edit, "Master password")]])
        .window(3, "Capri", "capri.exe", vec![vec![]])
}

#[tokio::test]
async fn snapshots_give_refs_the_model_can_click() {
    let desk = Arc::new(notepad());
    let (tools, _) = hands_runner(&desk, ApprovalMode::Auto, true);
    let listed = tools.run("ui_windows", serde_json::json!({}), Origin::Model).await.unwrap();
    assert!(listed.contains("notepad.exe \"Sans titre - Bloc-notes\""), "{listed}");
    assert!(listed.contains("Bitwarden.exe (private window, off limits)") && !listed.contains("capri"), "{listed}");

    let snap = tools.run("ui_snapshot", serde_json::json!({"window": "bloc-notes"}), Origin::Model).await.unwrap();
    assert!(snap.contains("- e2 button \"Fermer\""), "{snap}");
    let out = tools.run("ui_click", serde_json::json!({"ref": "e2"}), Origin::Model).await.unwrap();
    assert_eq!(out, "pressed button \"Fermer\" in notepad.exe");
    tools
        .run(
            "ui_fill",
            serde_json::json!({"window": "notepad", "role": "textbox", "name": "éditeur", "text": "salut"}),
            Origin::Model,
        )
        .await
        .unwrap();
    assert_eq!(desk.acted(), ["Fermer", "Éditeur de texte"]);
}

#[tokio::test]
async fn private_windows_and_her_own_are_off_limits() {
    let desk = Arc::new(notepad());
    let (tools, _) = hands_runner(&desk, ApprovalMode::Auto, true);
    let r = tools.run("ui_snapshot", serde_json::json!({"window": "bitwarden"}), Origin::Model).await;
    assert!(matches!(&r, Err(ToolError::Failed(m)) if m.contains("private")), "{r:?}");
    let r = tools.run("ui_press", serde_json::json!({"window": "capri", "keys": "alt+f4"}), Origin::Model).await;
    assert!(r.is_err());
    assert!(desk.keys.lock().unwrap().is_empty());
}

#[tokio::test]
async fn risky_buttons_and_keys_are_always_confirmed() {
    let desk = Arc::new(notepad());
    let (tools, asked) = hands_runner(&desk, ApprovalMode::Auto, false);
    tools.run("ui_snapshot", serde_json::json!({"window": "notepad"}), Origin::Model).await.unwrap();
    let r = tools.run("ui_click", serde_json::json!({"ref": "e3"}), Origin::Model).await;
    assert!(matches!(r, Err(ToolError::Declined)), "Supprimer needs a yes even in auto mode");
    assert!(asked.1.load(Ordering::SeqCst));
    let r = tools.run("ui_press", serde_json::json!({"window": "notepad", "keys": "ctrl+w"}), Origin::Model).await;
    assert!(matches!(r, Err(ToolError::Declined)));
    assert!(desk.acted().is_empty() && desk.keys.lock().unwrap().is_empty());
}

// -- tool calls from the model -----------------------------------------------------------------

/// Calls a tool on its first turn, then talks.
struct ToolUser {
    call: ToolCall,
    after: &'static str,
    seen: Mutex<Vec<Vec<Message>>>,
}

#[async_trait]
impl LanguageModel for ToolUser {
    fn name(&self) -> String {
        "tool-user".into()
    }

    async fn stream(&self, _: &[Message]) -> Result<TokenStream, LlmError> {
        Err(LlmError::Backend("should use respond".into()))
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn respond(&self, messages: &[Message], tools: &[ToolSpec]) -> Result<PieceStream, LlmError> {
        assert!(tools.iter().any(|t| t.name == "spotify"));
        let first = self.seen.lock().await.is_empty();
        self.seen.lock().await.push(messages.to_vec());
        let piece = if first { Piece::Call(self.call.clone()) } else { Piece::Text(self.after.into()) };
        Ok(Box::pin(futures::stream::iter(vec![Ok(piece)])))
    }
}

fn tool_chat(mode: ApprovalMode, desk: &Arc<FakeDesk>) -> (Arc<ChatService>, Arc<ToolUser>) {
    let model = Arc::new(ToolUser {
        call: ToolCall { id: "c1".into(), name: "spotify".into(), arguments: r#"{"action":"next"}"#.into() },
        after: "Suivante !",
        seen: Mutex::default(),
    });
    let tools = Arc::new(
        ToolRunner::new(mode, Arc::new(Answer(false, AtomicBool::new(false))))
            .with(Arc::new(remote(desk, &Arc::new(FakeOpener::default())))),
    );
    let settings = ChatSettings { lang: Language::Fr, ..ChatSettings::default() };
    (Arc::new(ChatService::new(model.clone(), Arc::new(VecMemory::default()), settings).with_tools(tools)), model)
}

#[tokio::test(start_paused = true)]
async fn the_model_calls_a_tool_and_reads_the_result() {
    let desk = Arc::new(FakeDesk::default().window(5, "Spotify", "Spotify.exe", vec![player_bar(true)]));
    let (chat, model) = tool_chat(ApprovalMode::Auto, &desk);
    let events = run(&chat, "passe à la chanson d'après").await;
    assert_eq!(events.last(), Some(&ChatEvent::Done("Suivante !".into())));
    assert_eq!(desk.acted(), ["Suivant"]);
    let second = &model.seen.lock().await[1];
    assert!(second[0].content.contains("hands on the user's PC"), "tools note in the system prompt");
    assert_eq!(second.last().map(|m| (m.role, m.content.as_str())), Some((capri_domain::Role::Tool, "done")));
}

#[tokio::test(start_paused = true)]
async fn she_asks_in_the_chat_and_the_next_line_answers() {
    let desk = Arc::new(FakeDesk::default().window(5, "Spotify", "Spotify.exe", vec![player_bar(true)]));
    let (chat, _) = tool_chat(ApprovalMode::Ask, &desk);
    let events = Arc::new(StdMutex::new(Vec::new()));
    let (c, e) = (chat.clone(), events.clone());
    let task =
        tokio::spawn(async move { c.send("tu peux zapper ce morceau ?", |ev| e.lock().unwrap().push(ev)).await });
    // wait for the question
    for _ in 0..100 {
        if events.lock().unwrap().iter().any(|e| matches!(e, ChatEvent::Confirm(_))) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let question = events.lock().unwrap().iter().find_map(|e| match e {
        ChatEvent::Confirm(q) => Some(q.clone()),
        _ => None,
    });
    assert!(question.as_deref().is_some_and(|q| q.contains("Spotify") && q.contains("oui / non")), "{question:?}");
    assert!(desk.acted().is_empty(), "nothing before the yes");
    assert_eq!(run(&chat, "oui vas-y").await, vec![], "the answer itself gets no reply");
    task.await.unwrap();
    assert_eq!(desk.acted(), ["Suivant"]);
    assert_eq!(events.lock().unwrap().last(), Some(&ChatEvent::Done("Suivante !".into())));
}

#[tokio::test(start_paused = true)]
async fn no_answer_or_another_request_means_no() {
    let desk = Arc::new(FakeDesk::default().window(5, "Spotify", "Spotify.exe", vec![player_bar(true)]));
    let (chat, model) = tool_chat(ApprovalMode::Ask, &desk);
    let events = run(&chat, "suivante").await; // a typed command: runs directly, no question
    assert_eq!(events, vec![ChatEvent::Done("Suivante.".into())]);
    let events = run(&chat, "passe la suivante par toi-même").await; // through the model: asks, times out
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Confirm(_))));
    assert_eq!(desk.acted(), ["Suivant"], "only the typed command ran");
    let tool_turn = model.seen.lock().await[1].last().cloned().unwrap();
    assert!(tool_turn.content.contains("said no"), "{}", tool_turn.content);
}
