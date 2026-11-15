//! The winit event loop: one borderless, always-on-top, transparent window that follows her around.
//!
//! Mouse: drag her to pick her up (let go fast to toss her), click to talk, double-click to poke.
//! Keyboard while the chat box is open: type, Enter to send, Esc to close, `/quit` to leave.

use std::sync::Arc;
use std::time::{Duration, Instant};

use capri_application::ports::{Skin, SkinRepository};
use capri_application::services::{ChatEvent, ChatService, DeskService, FailureKind, Heard, VoiceService};
use capri_domain::{CharacterConfig, Language, Point, Rect};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId, WindowLevel};

use crate::bubble::Bubble;
use crate::present::{self, Presenter};
use crate::scene::{Layout, Painter};

/// Builds the desk once the window exists: it gets the DPI scale and the monitors (screen px).
pub type DeskFactory = Box<dyn FnOnce(f32, Vec<Rect>) -> DeskService>;

#[derive(Debug, Clone)]
pub struct DeskOptions {
    pub skin: String,
    /// Her size at 100% scaling.
    pub size: f32,
    pub fps: u32,
    /// Resolved interface language.
    pub lang: Language,
    /// Push-to-talk key, e.g. `F8` or `Ctrl+Alt+Space`.
    pub push_to_talk: String,
}

/// New settings to apply live (the brain is swapped by the caller before returning this).
#[derive(Debug, Clone)]
pub struct Reload {
    pub options: DeskOptions,
    /// Behaviour at 100% scaling; the desk scales the size.
    pub character: CharacterConfig,
}

/// What the desk asks of the outside world, besides chatting.
pub struct DeskHooks {
    /// Opens the settings panel (right-click on her, or `/settings`).
    pub open_settings: Box<dyn Fn()>,
    /// Polled about once a second; returns new settings when the config file changed.
    pub reload: Box<dyn FnMut() -> Option<Reload>>,
}

impl std::fmt::Debug for DeskHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeskHooks")
    }
}

#[derive(Debug)]
pub enum UiEvent {
    Chat(ChatEvent),
    /// What push-to-talk heard.
    Heard(Result<Heard, String>),
}

#[derive(Debug, thiserror::Error)]
pub enum DeskError {
    #[error("window system: {0}")]
    EventLoop(String),
    #[error("skin: {0}")]
    Skin(String),
}

/// Runs until she is told to quit. Blocks the calling (main) thread, as every OS requires for UI.
pub fn run(
    options: DeskOptions,
    make_desk: DeskFactory,
    skins: Arc<dyn SkinRepository>,
    chat: Arc<ChatService>,
    runtime: tokio::runtime::Handle,
    hooks: DeskHooks,
    voice: Arc<VoiceService>,
) -> Result<(), DeskError> {
    let event_loop =
        EventLoop::<UiEvent>::with_user_event().build().map_err(|e| DeskError::EventLoop(e.to_string()))?;
    let proxy = event_loop.create_proxy();
    let mut app = App {
        options,
        make_desk: Some(make_desk),
        skins,
        chat,
        runtime,
        proxy,
        hooks,
        voice,
        hotkeys: None,
        ptt: None,
        listening: false,
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app).map_err(|e| DeskError::EventLoop(e.to_string()))?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

struct App {
    options: DeskOptions,
    make_desk: Option<DeskFactory>,
    skins: Arc<dyn SkinRepository>,
    chat: Arc<ChatService>,
    runtime: tokio::runtime::Handle,
    proxy: EventLoopProxy<UiEvent>,
    hooks: DeskHooks,
    voice: Arc<VoiceService>,
    hotkeys: Option<GlobalHotKeyManager>,
    ptt: Option<HotKey>,
    listening: bool,
    state: Option<Live>,
    error: Option<DeskError>,
}

impl App {
    /// Sends a line (typed or heard) to the brain; the reply streams back as events and, sentence by
    /// sentence, to her voice.
    fn send_line(&self, line: String) {
        let (chat, proxy, voice) = (self.chat.clone(), self.proxy.clone(), self.voice.clone());
        self.runtime.spawn(async move {
            chat.send(&line, |e| {
                match &e {
                    ChatEvent::Delta(d) => voice.say_streamed(d),
                    ChatEvent::Done(_) => voice.say_done(),
                    _ => {}
                }
                let _ = proxy.send_event(UiEvent::Chat(e));
            })
            .await;
        });
    }

    /// (Re)registers the push-to-talk key; a bad key name is logged, not fatal.
    fn register_ptt(&mut self) {
        let Some(manager) = self.hotkeys.as_ref() else { return };
        if let Some(old) = self.ptt.take() {
            let _ = manager.unregister(old);
        }
        let name = self.options.push_to_talk.trim();
        if name.is_empty() {
            return;
        }
        match name.parse::<HotKey>() {
            Ok(key) => match manager.register(key) {
                Ok(()) => {
                    tracing::info!(key = name, "push-to-talk ready");
                    self.ptt = Some(key);
                }
                Err(e) => tracing::warn!(key = name, error = %e, "push-to-talk key is taken by another app"),
            },
            Err(e) => tracing::warn!(key = name, error = %e, "push-to-talk key not understood"),
        }
    }

    fn poll_ptt(&mut self) {
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if self.ptt.is_none_or(|k| k.id() != event.id()) {
                continue;
            }
            let Some(s) = self.state.as_mut() else { continue };
            match event.state() {
                HotKeyState::Pressed if !self.listening => {
                    if !self.voice.can_listen() {
                        let now = s.now();
                        let msg = s.painter.lang.pick(
                            "Speech recognition is off. Set it up in the settings, VOICE tab.",
                            "La reconnaissance vocale est désactivée. Règle-la dans les réglages, onglet VOIX.",
                        );
                        s.bubble.fail(msg.to_string(), now);
                        continue;
                    }
                    match self.voice.listen() {
                        Ok(()) => {
                            self.listening = true;
                            s.bubble.listen();
                        }
                        Err(e) => {
                            let now = s.now();
                            s.bubble.fail(e.0, now);
                        }
                    }
                }
                HotKeyState::Released if self.listening => {
                    self.listening = false;
                    s.bubble.heard();
                    let (voice, proxy) = (self.voice.clone(), self.proxy.clone());
                    self.runtime.spawn(async move {
                        let heard = voice.heard().await.map_err(|e| e.0);
                        let _ = proxy.send_event(UiEvent::Heard(heard));
                    });
                }
                _ => {}
            }
        }
    }
}

/// Everything that exists once the window is up.
struct Live {
    window: Arc<Window>,
    presenter: Box<dyn Presenter>,
    painter: Painter,
    skin: Skin,
    desk: DeskService,
    bubble: Bubble,
    clock: Instant,
    last_tick: f32,
    origin: (i32, i32),
    cursor: PhysicalPosition<f64>,
    press: Option<Press>,
    last_click: f32,
    trail: Vec<(f32, Point)>,
    scale: f32,
    reload_at: f32,
    voice: Arc<VoiceService>,
    /// Last mouse event her window received, and since when the pointer has been moving over her
    /// without any: the sign that Windows stopped sending her clicks.
    last_mouse_event: f32,
    unheard_since: Option<f32>,
    last_os_cursor: Option<Point>,
}

#[derive(Clone, Copy)]
struct Press {
    at_screen: Point,
    grab_offset: Point,
    dragging: bool,
    on_bubble: bool,
}

impl Live {
    fn now(&self) -> f32 {
        self.clock.elapsed().as_secs_f32()
    }

    /// The pointer in screen pixels. Asked from the OS when it can tell: the window moves under the
    /// pointer while she is dragged, so window-relative positions lag one frame behind and jitter.
    fn cursor_screen(&self) -> Point {
        present::os_cursor().map_or_else(
            || Point::new(self.origin.0 as f32 + self.cursor.x as f32, self.origin.1 as f32 + self.cursor.y as f32),
            |(x, y)| Point::new(x, y),
        )
    }

    /// The pointer on her canvas.
    fn cursor_local(&self) -> (f32, f32) {
        let p = self.cursor_screen();
        (p.x - self.origin.0 as f32, p.y - self.origin.1 as f32)
    }

    /// Follows the pointer while the button is held: past a few pixels a press becomes a drag.
    fn pointer_moved(&mut self, now: f32) {
        let screen = self.cursor_screen();
        self.trail.push((now, screen));
        self.trail.retain(|(t, _)| now - t < 0.1);
        let Some(mut press) = self.press else { return };
        let moved = (screen.x - press.at_screen.x).hypot(screen.y - press.at_screen.y);
        if !press.dragging && !press.on_bubble && moved > DRAG_THRESHOLD {
            press.dragging = true;
            tracing::debug!(moved, "drag starts");
            self.desk.character_mut().grab(now);
            // measured after grab: picking her up off a window shifts her to hang from the hand
            let her = self.desk.character().position();
            press.grab_offset = Point::new(her.x - press.at_screen.x, her.y - press.at_screen.y);
        }
        if press.dragging {
            self.desk
                .character_mut()
                .drag_to(Point::new(screen.x + press.grab_offset.x, screen.y + press.grab_offset.y));
        }
        self.press = Some(press);
    }

    fn frame(&mut self) {
        let now = self.now();
        let dt = now - self.last_tick;
        self.last_tick = now;
        // while the button is held she follows the pointer every frame, not only on mouse events;
        // the release itself comes from winit, which captures the mouse during a press
        if self.press.is_some() {
            self.pointer_moved(now);
        }
        self.watch_hit_test(now);
        self.bubble.tick(now);
        let speaking = self.voice.speaking();
        if speaking {
            self.bubble.hold_until(now + 1.5);
        }
        let talking = self.bubble.talking(now) || speaking;
        let c = self.desk.character_mut();
        c.set_talking(talking, now);
        c.set_chat_open(self.bubble.is_open(), now);
        let started = Instant::now();
        let (pose, _happenings) = self.desk.tick(now, dt);
        let took = started.elapsed();
        if took > Duration::from_millis(30) {
            tracing::warn!(ms = took.as_millis(), "slow frame: reading the desktop took too long");
        }
        let head = self.desk.character().head();
        self.painter.paint(&pose, head, &self.skin, &self.bubble, now);
        self.origin = self.painter.layout.origin(pose.anchor);
        self.presenter.present(self.painter.pixmap(), self.origin);
    }

    /// Applies saved settings without restarting: behaviour and language at once, skin and size by
    /// reloading the sprites and resizing the canvas.
    fn apply(&mut self, r: &Reload, skins: &dyn SkinRepository) -> Result<(), String> {
        let size = r.options.size * self.scale;
        let reskin = r.options.skin != self.skin.info.id || (size - self.painter.layout.size).abs() > 0.5;
        if reskin {
            self.skin = skins.load(&r.options.skin, size, 1.0).map_err(|e| e.to_string())?;
            let layout = Layout::for_size(size);
            self.painter = Painter::new(layout);
            let _ = self.window.request_inner_size(PhysicalSize::new(layout.width, layout.height));
        }
        self.painter.lang = r.options.lang;
        let cfg = CharacterConfig { size, ..r.character };
        self.desk.character_mut().set_config(cfg);
        tracing::info!(skin = %r.options.skin, size, reskin, "settings applied");
        Ok(())
    }

    /// If the pointer moves over her body for a while and her window gets no mouse event at all,
    /// Windows has lost her clickable area: log it and make it recompute.
    fn watch_hit_test(&mut self, now: f32) {
        let Some((x, y)) = present::os_cursor() else { return };
        let p = Point::new(x, y);
        let moved = self.last_os_cursor.is_some_and(|q| (q.x - p.x).abs() + (q.y - p.y).abs() > 2.0);
        self.last_os_cursor = Some(p);
        let local = (x - self.origin.0 as f32, y - self.origin.1 as f32);
        let over_her = self.painter.alpha_at(local.0, local.1) > 0;
        if !(moved && over_her && now - self.last_mouse_event > 0.3) {
            if !over_her || now - self.last_mouse_event <= 0.3 {
                self.unheard_since = None;
            }
            return;
        }
        let since = *self.unheard_since.get_or_insert(now);
        if now - since > 0.5 {
            tracing::warn!(
                capture = ?self.presenter.capture_owner(),
                mode = ?self.desk.character().mode(),
                "the pointer is on her but her window gets no mouse events: repairing"
            );
            self.presenter.repair_hit_test();
            self.unheard_since = None;
            self.last_mouse_event = now;
        }
    }

    fn on_her(&self) -> bool {
        let (x, y) = self.cursor_local();
        self.painter.bubble_rect.is_some_and(|r| r.contains(x, y)) || self.painter.alpha_at(x, y) > 0
    }

    fn press(&mut self) {
        let (x, y) = self.cursor_local();
        let on_bubble = self.painter.bubble_rect.is_some_and(|r| r.contains(x, y));
        self.trail.clear();
        self.press =
            Some(Press { at_screen: self.cursor_screen(), grab_offset: Point::default(), dragging: false, on_bubble });
        tracing::debug!(x, y, on_bubble, "press");
    }

    /// Button up: a throw if she was being dragged, otherwise a click (double click pokes her).
    fn release(&mut self, now: f32) {
        let Some(press) = self.press.take() else { return };
        if press.dragging {
            let v = velocity(&self.trail);
            self.desk.character_mut().release(v, now);
            return;
        }
        if press.on_bubble {
            return;
        }
        if now - self.last_click < DOUBLE_CLICK {
            self.desk.character_mut().poke(now);
            self.last_click = f32::NEG_INFINITY;
            return;
        }
        if self.bubble.is_typing() {
            self.bubble.close();
        } else {
            self.bubble.open_typing();
            self.window.focus_window();
        }
        self.last_click = now;
    }
}

impl ApplicationHandler<UiEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let Some(make_desk) = self.make_desk.take() else { return };

        let scale = event_loop.primary_monitor().map_or(1.0, |m| m.scale_factor()) as f32;
        let size = self.options.size * scale;
        let layout = Layout::for_size(size);

        #[allow(unused_mut)]
        let mut attrs = Window::default_attributes()
            .with_title("Capri")
            .with_decorations(false)
            // on Windows the layered window gives real per-pixel alpha; winit's blur-behind would fight it
            .with_transparent(!cfg!(windows))
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(PhysicalSize::new(layout.width, layout.height));
        #[cfg(windows)]
        {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs = attrs.with_skip_taskbar(true);
        }
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(DeskError::EventLoop(e.to_string()));
                event_loop.exit();
                return;
            }
        };

        let skin = match self.skins.load(&self.options.skin, size, 1.0) {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(DeskError::Skin(e.to_string()));
                event_loop.exit();
                return;
            }
        };

        let monitors: Vec<Rect> = event_loop
            .available_monitors()
            .map(|m| {
                let p = m.position();
                let s = m.size();
                Rect::new(p.x as f32, p.y as f32, s.width as f32, s.height as f32)
            })
            .collect();
        let mut desk = make_desk(scale, monitors);
        let mut painter = Painter::new(layout);
        painter.lang = self.options.lang;
        desk.set_own_windows(vec![present::native_id(&window)]);
        let presenter = present::create(window.clone());
        let clock = Instant::now();
        desk.start(0.0);

        tracing::info!(skin = %skin.info.name, size, scale, model = %self.chat.model_name(), "Capri is on the desk");
        self.state = Some(Live {
            window,
            presenter,
            painter,
            skin,
            desk,
            bubble: Bubble::default(),
            clock,
            last_tick: 0.0,
            origin: (0, 0),
            cursor: PhysicalPosition::new(0.0, 0.0),
            press: None,
            last_click: f32::NEG_INFINITY,
            trail: Vec::new(),
            scale,
            reload_at: 1.0,
            voice: self.voice.clone(),
            last_mouse_event: 0.0,
            unheard_since: None,
            last_os_cursor: None,
        });
        match GlobalHotKeyManager::new() {
            Ok(m) => {
                self.hotkeys = Some(m);
                self.register_ptt();
            }
            Err(e) => tracing::warn!(error = %e, "no global keys on this system: push-to-talk is off"),
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: UiEvent) {
        let Some(s) = self.state.as_mut() else { return };
        let now = s.now();
        match event {
            UiEvent::Heard(Ok(Heard::Text(line))) => {
                tracing::info!(heard = %line, "push-to-talk");
                self.send_line(line);
            }
            UiEvent::Heard(Ok(Heard::Nothing)) => s.bubble.close(),
            UiEvent::Heard(Err(e)) => {
                let msg = format!("{} {e}", s.painter.lang.pick("I didn't catch that:", "Je n'ai pas compris :"));
                s.bubble.fail(msg, now);
            }
            UiEvent::Chat(ChatEvent::Thinking) => {}
            UiEvent::Chat(ChatEvent::Delta(d)) => s.bubble.push_reply(&d),
            UiEvent::Chat(ChatEvent::Done(full)) => s.bubble.finish(full, now),
            UiEvent::Chat(ChatEvent::Failed(f)) => {
                tracing::warn!(detail = %f.detail, "reply failed");
                let l = s.painter.lang;
                let text = match f.kind {
                    FailureKind::Unreachable => l.pick(
                        "I can't reach my brain. Right-click me to check the server in the settings.",
                        "Je n'arrive pas à joindre mon cerveau. Clic droit sur moi pour vérifier le serveur dans les réglages.",
                    ),
                    FailureKind::SignedOut => l.pick(
                        "My brain refused the key. Right-click me to fix it in the settings.",
                        "Mon cerveau refuse la clé. Clic droit sur moi pour la corriger dans les réglages.",
                    ),
                    FailureKind::Backend => l.pick(
                        "My brain glitched on that one. Try again?",
                        "Mon cerveau a buggé sur celle-là. On réessaie ?",
                    ),
                };
                s.bubble.fail(text.to_string(), now);
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        let now = s.now();
        if let WindowEvent::MouseInput { state, button, .. } = &event {
            s.last_mouse_event = now;
            tracing::debug!(?state, ?button, "mouse button");
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => s.frame(),
            WindowEvent::CursorMoved { position, .. } => {
                s.cursor = position;
                s.last_mouse_event = now;
                s.pointer_moved(now);
            }
            WindowEvent::CursorEntered { .. } | WindowEvent::CursorLeft { .. } | WindowEvent::Focused(_) => {
                s.last_mouse_event = now;
                tracing::debug!(?event, "window event");
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if s.presenter.clicks_pass_through_transparency() || s.on_her() {
                    s.press();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => s.release(now),
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Right, .. } => {
                if s.presenter.clicks_pass_through_transparency() || s.on_her() {
                    (self.hooks.open_settings)();
                }
            }
            WindowEvent::KeyboardInput {
                event: KeyEvent { state: ElementState::Pressed, logical_key, text, .. },
                ..
            } if s.bubble.is_typing() => match logical_key {
                Key::Named(NamedKey::Enter) => {
                    if let Some(line) = s.bubble.submit() {
                        if line == "/quit" || line == "/exit" {
                            event_loop.exit();
                            return;
                        }
                        if matches!(line.as_str(), "/settings" | "/reglages" | "/réglages") {
                            s.bubble.close();
                            (self.hooks.open_settings)();
                            return;
                        }
                        self.voice.interrupt();
                        self.send_line(line);
                    }
                }
                Key::Named(NamedKey::Escape) => s.bubble.close(),
                Key::Named(NamedKey::Backspace) => s.bubble.backspace(),
                _ => {
                    if let Some(t) = text {
                        s.bubble.type_text(&t);
                    }
                }
            },
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_ptt();
        let Some(s) = self.state.as_mut() else { return };
        let now = s.now();
        if now >= s.reload_at {
            s.reload_at = now + 1.0;
            if let Some(r) = (self.hooks.reload)() {
                if let Err(e) = s.apply(&r, self.skins.as_ref()) {
                    tracing::warn!(error = %e, "could not apply the new settings");
                }
                let new_key = r.options.push_to_talk != self.options.push_to_talk;
                self.options = r.options;
                if new_key {
                    self.register_ptt();
                }
            }
        }
        let Some(s) = self.state.as_mut() else { return };
        s.window.request_redraw();
        let frame = Duration::from_secs_f64(1.0 / f64::from(self.options.fps.clamp(10, 144)));
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + frame));
    }
}

/// Pixels the pointer must travel before a press becomes a drag (a shaky click stays a click).
const DRAG_THRESHOLD: f32 = 9.0;
const DOUBLE_CLICK: f32 = 0.35;

/// Pointer velocity over the last ~100 ms, in px/s, for tossing her.
fn velocity(trail: &[(f32, Point)]) -> Point {
    match (trail.first(), trail.last()) {
        (Some((t0, a)), Some((t1, b))) if t1 - t0 > 0.01 => {
            Point::new((b.x - a.x) / (t1 - t0), (b.y - a.y) / (t1 - t0))
        }
        _ => Point::default(),
    }
}
