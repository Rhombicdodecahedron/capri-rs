//! The winit event loop: one borderless, always-on-top, transparent window that follows her around.
//!
//! Mouse: drag her to pick her up (let go fast to toss her), click to talk, double-click to poke.
//! Keyboard while the chat box is open: type, Enter to send, Esc to close, `/quit` to leave.

use std::sync::Arc;
use std::time::{Duration, Instant};

use capri_application::ports::{Skin, SkinRepository};
use capri_application::services::{ChatEvent, ChatService, DeskService};
use capri_domain::{CharacterConfig, Language, Point, Rect};
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
) -> Result<(), DeskError> {
    let event_loop =
        EventLoop::<UiEvent>::with_user_event().build().map_err(|e| DeskError::EventLoop(e.to_string()))?;
    let proxy = event_loop.create_proxy();
    let mut app =
        App { options, make_desk: Some(make_desk), skins, chat, runtime, proxy, hooks, state: None, error: None };
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
    state: Option<Live>,
    error: Option<DeskError>,
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

    fn cursor_screen(&self) -> Point {
        Point::new(self.origin.0 as f32 + self.cursor.x as f32, self.origin.1 as f32 + self.cursor.y as f32)
    }

    fn frame(&mut self) {
        let now = self.now();
        let dt = now - self.last_tick;
        self.last_tick = now;
        self.bubble.tick(now);
        let talking = self.bubble.talking(now);
        let c = self.desk.character_mut();
        c.set_talking(talking, now);
        c.set_chat_open(self.bubble.is_open(), now);
        let (pose, _happenings) = self.desk.tick(now, dt);
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

    fn on_her(&self) -> bool {
        let (x, y) = (self.cursor.x as f32, self.cursor.y as f32);
        self.painter.bubble_rect.is_some_and(|r| r.contains(x, y)) || self.painter.alpha_at(x, y) > 40
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
            .with_transparent(true)
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
        });
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: UiEvent) {
        let Some(s) = self.state.as_mut() else { return };
        let now = s.now();
        match event {
            UiEvent::Chat(ChatEvent::Thinking) => {}
            UiEvent::Chat(ChatEvent::Delta(d)) => s.bubble.push_reply(&d),
            UiEvent::Chat(ChatEvent::Done(full)) => s.bubble.finish(full, now),
            UiEvent::Chat(ChatEvent::Failed(e)) => s.bubble.fail(e, now),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        let now = s.now();
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => s.frame(),
            WindowEvent::CursorMoved { position, .. } => {
                s.cursor = position;
                let screen = s.cursor_screen();
                s.trail.push((now, screen));
                s.trail.retain(|(t, _)| now - t < 0.12);
                if let Some(mut press) = s.press {
                    let moved = (screen.x - press.at_screen.x).hypot(screen.y - press.at_screen.y);
                    if !press.dragging && !press.on_bubble && moved > 6.0 {
                        press.dragging = true;
                        s.desk.character_mut().grab(now);
                    }
                    if press.dragging {
                        let p = Point::new(screen.x + press.grab_offset.x, screen.y + press.grab_offset.y);
                        s.desk.character_mut().drag_to(p);
                    }
                    s.press = Some(press);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if s.presenter.clicks_pass_through_transparency() || s.on_her() {
                    let at = s.cursor_screen();
                    let her = s.desk.character().position();
                    let on_bubble =
                        s.painter.bubble_rect.is_some_and(|r| r.contains(s.cursor.x as f32, s.cursor.y as f32));
                    s.press = Some(Press {
                        at_screen: at,
                        grab_offset: Point::new(her.x - at.x, her.y - at.y),
                        dragging: false,
                        on_bubble,
                    });
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                let Some(press) = s.press.take() else { return };
                if press.dragging {
                    let v = velocity(&s.trail);
                    s.desk.character_mut().release(v, now);
                } else if !press.on_bubble {
                    if now - s.last_click < 0.35 {
                        s.desk.character_mut().poke(now);
                    } else if s.bubble.is_typing() {
                        s.bubble.close();
                    } else {
                        s.bubble.open_typing();
                        s.window.focus_window();
                    }
                    s.last_click = now;
                }
            }
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
                        let chat = self.chat.clone();
                        let proxy = self.proxy.clone();
                        self.runtime.spawn(async move {
                            chat.send(&line, |e| {
                                let _ = proxy.send_event(UiEvent::Chat(e));
                            })
                            .await;
                        });
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
        let Some(s) = self.state.as_mut() else { return };
        let now = s.now();
        if now >= s.reload_at {
            s.reload_at = now + 1.0;
            if let Some(r) = (self.hooks.reload)() {
                if let Err(e) = s.apply(&r, self.skins.as_ref()) {
                    tracing::warn!(error = %e, "could not apply the new settings");
                }
                self.options = r.options;
            }
        }
        s.window.request_redraw();
        let frame = Duration::from_secs_f64(1.0 / f64::from(self.options.fps.clamp(10, 144)));
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + frame));
    }
}

/// Pointer velocity over the last ~120 ms, in px/s, for tossing her.
fn velocity(trail: &[(f32, Point)]) -> Point {
    match (trail.first(), trail.last()) {
        (Some((t0, a)), Some((t1, b))) if t1 - t0 > 0.01 => {
            Point::new((b.x - a.x) / (t1 - t0), (b.y - a.y) / (t1 - t0))
        }
        _ => Point::default(),
    }
}
