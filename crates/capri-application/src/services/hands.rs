//! Use case: her hands on any app, as tools the model can call, in the spirit of Playwright's MCP
//! server: list the windows, take a snapshot of one (its controls, each with a ref), then click,
//! fill or press keys by ref or by role and name.
//!
//! Safety lives here, not in the adapter: private windows (password managers, banking, private
//! browsing, the user's own list) are never read or touched, her own window is off limits, and a
//! control that deletes, sends or pays raises the call to destructive, so it is always confirmed.

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use capri_domain::Risk;
use capri_domain::ui::{self, KeyChord, Locator, Role, Snapshot, WindowInfo, WindowQuery};
use serde::Deserialize;

use crate::ports::{Tool, ToolError, ToolSpec, UiAction, UiDriver, UiError};

/// Controls listed in one snapshot, so a busy window doesn't flood the model's context.
const SNAPSHOT_LINES: usize = 160;

/// What the `ui_*` tools share: the driver, the privacy list and the last snapshot (refs point into it).
pub struct Hands {
    ui: Arc<dyn UiDriver>,
    settings: std::sync::RwLock<HandSettings>,
    last: Mutex<Option<Snapshot>>,
}

#[derive(Debug, Clone)]
struct HandSettings {
    enabled: bool,
    never_touch: Vec<String>,
}

impl std::fmt::Debug for Hands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hands").field("settings", &self.settings()).finish_non_exhaustive()
    }
}

fn ui_failed(e: UiError) -> ToolError {
    match e {
        UiError::Unsupported => ToolError::Unavailable(e.to_string()),
        e => ToolError::Failed(e.to_string()),
    }
}

/// Where a click or a fill lands: a ref from the last snapshot, or a role and a name.
#[derive(Debug, Default, Deserialize)]
struct Target {
    #[serde(default)]
    window: Option<String>,
    #[serde(default, rename = "ref")]
    reference: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

/// A target resolved against the windows that exist now.
struct Resolved {
    window: WindowInfo,
    locator: Locator,
}

impl Hands {
    pub fn new(ui: Arc<dyn UiDriver>, never_touch: Vec<String>) -> Arc<Self> {
        let settings = std::sync::RwLock::new(HandSettings { enabled: true, never_touch });
        Arc::new(Self { ui, settings, last: Mutex::new(None) })
    }

    /// The SAFETY settings changed: applies to the next call.
    pub fn configure(&self, enabled: bool, never_touch: Vec<String>) {
        if let Ok(mut s) = self.settings.write() {
            *s = HandSettings { enabled, never_touch };
        }
        if !enabled {
            *self.last.lock().unwrap_or_else(PoisonError::into_inner) = None;
        }
    }

    fn settings(&self) -> HandSettings {
        self.settings.read().map_or_else(|e| e.into_inner().clone(), |s| s.clone())
    }

    /// The five tools, ready for [`ToolRunner::with_all`](crate::services::ToolRunner::with_all).
    pub fn tools(self: &Arc<Self>) -> Vec<Arc<dyn Tool>> {
        [Kind::Windows, Kind::Snapshot, Kind::Click, Kind::Fill, Kind::Press]
            .into_iter()
            .map(|kind| Arc::new(HandTool { hands: self.clone(), kind }) as Arc<dyn Tool>)
            .collect()
    }

    fn last(&self) -> Option<Snapshot> {
        self.last.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn allowed(&self, w: &WindowInfo) -> Result<(), ToolError> {
        if w.app() == "capri" {
            return Err(ToolError::Failed("that's my own window, I leave it alone".into()));
        }
        if ui::private(w, &self.settings().never_touch) {
            return Err(ToolError::Failed(format!(
                "\"{}\" looks private (passwords, banking, private browsing or your never-touch list): I don't look at it or touch it",
                w.title
            )));
        }
        Ok(())
    }

    /// The window someone named, or the one of the last snapshot.
    async fn find_window(&self, name: Option<&str>) -> Result<WindowInfo, ToolError> {
        let name = name.map(str::trim).filter(|n| !n.is_empty());
        let windows = self.ui.windows().await.map_err(ui_failed)?;
        let found = match (name, self.last()) {
            (Some(n), _) => WindowQuery::pick(n, &windows).cloned(),
            (None, Some(last)) => windows.iter().find(|w| w.id == last.window.id).cloned(),
            (None, None) => None,
        };
        let w = found.ok_or_else(|| {
            ToolError::Failed(match name {
                Some(n) => format!("no open window matches \"{n}\" (ui_windows lists them)"),
                None => "say which window (ui_windows lists them)".into(),
            })
        })?;
        self.allowed(&w)?;
        Ok(w)
    }

    async fn list(&self) -> Result<String, ToolError> {
        use std::fmt::Write as _;
        let windows = self.ui.windows().await.map_err(ui_failed)?;
        let never_touch = self.settings().never_touch;
        let mut out = String::new();
        for w in windows.iter().filter(|w| !w.title.trim().is_empty()) {
            if w.app() == "capri" {
                continue;
            }
            if ui::private(w, &never_touch) {
                let _ = writeln!(out, "- {} (private window, off limits)", w.process);
            } else {
                let _ = writeln!(out, "- {} \"{}\"", w.process, w.title);
            }
        }
        Ok(if out.is_empty() { "no open windows".into() } else { out })
    }

    async fn snapshot(&self, window: Option<&str>) -> Result<String, ToolError> {
        let w = self.find_window(window).await?;
        let snap = self.ui.snapshot(&WindowQuery::id(w.id)).await.map_err(ui_failed)?;
        let text = snap.render(SNAPSHOT_LINES);
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = Some(snap);
        Ok(text)
    }

    /// Turns a target into a window and a locator. A ref needs the snapshot it came from.
    async fn resolve(&self, t: &Target) -> Result<Resolved, ToolError> {
        let window = self.find_window(t.window.as_deref()).await?;
        if let Some(r) = t.reference.as_deref().filter(|r| !r.trim().is_empty()) {
            let snap = self
                .last()
                .filter(|s| s.window.id == window.id)
                .ok_or_else(|| ToolError::Failed("take a ui_snapshot of that window first".into()))?;
            let index = ui::parse_ref(r).ok_or_else(|| ToolError::BadArguments(format!("not a ref: {r}")))?;
            let locator = snap
                .locator_for(index)
                .ok_or_else(|| ToolError::BadArguments(format!("{r} is not in the last snapshot")))?;
            return Ok(Resolved { window, locator });
        }
        let name = t.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        let role = match t.role.as_deref().filter(|r| !r.trim().is_empty()) {
            Some(r) => Some(Role::parse(r).ok_or_else(|| ToolError::BadArguments(format!("unknown role {r}")))?),
            None => None,
        };
        if name.is_none() && role.is_none() {
            return Err(ToolError::BadArguments("give a ref, or a role and a name".into()));
        }
        let mut locator = role.map_or_else(Locator::default, Locator::role);
        if let Some(n) = name {
            locator = locator.name(n);
        }
        locator.enabled_only = true;
        Ok(Resolved { window, locator })
    }

    /// Acts on the target; a name that isn't exact gets a second chance as "contains".
    async fn act(&self, t: &Target, action: &UiAction) -> Result<(Resolved, String), ToolError> {
        let r = self.resolve(t).await?;
        let q = WindowQuery::id(r.window.id);
        match self.ui.act(&q, &r.locator, action).await {
            Ok(name) => Ok((r, name)),
            Err(UiError::NotFound(_)) if t.reference.is_none() && !r.locator.names.is_empty() => {
                let loose = r.locator.clone().contains();
                let name = self.ui.act(&q, &loose, action).await.map_err(ui_failed)?;
                Ok((r, name))
            }
            Err(e) => Err(ui_failed(e)),
        }
    }

    /// The name of the control a target points at, as far as it is known before acting.
    fn target_name(&self, t: &Target) -> String {
        if let Some(index) = t.reference.as_deref().and_then(ui::parse_ref) {
            if let Some(e) = self.last().and_then(|s| s.elements.get(index).cloned()) {
                return e.name;
            }
        }
        t.name.clone().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Windows,
    Snapshot,
    Click,
    Fill,
    Press,
}

struct HandTool {
    hands: Arc<Hands>,
    kind: Kind,
}

#[derive(Debug, Deserialize)]
struct ClickArgs {
    #[serde(flatten)]
    target: Target,
    #[serde(default)]
    double: bool,
}

#[derive(Debug, Deserialize)]
struct FillArgs {
    #[serde(flatten)]
    target: Target,
    text: String,
    /// Press Enter afterwards.
    #[serde(default)]
    submit: bool,
}

#[derive(Debug, Deserialize)]
struct PressArgs {
    #[serde(default)]
    window: Option<String>,
    keys: String,
}

#[derive(Debug, Default, Deserialize)]
struct WindowArg {
    #[serde(default)]
    window: Option<String>,
}

fn parse<T: serde::de::DeserializeOwned>(args: &serde_json::Value) -> Result<T, ToolError> {
    serde_json::from_value(args.clone()).map_err(|e| ToolError::BadArguments(e.to_string()))
}

fn target_schema(extra: serde_json::Value, required: &[&str]) -> serde_json::Value {
    let mut props = serde_json::json!({
        "window": { "type": "string", "description": "app or window title, e.g. 'Spotify', 'Discord', 'Bloc-notes'; defaults to the last snapshot's window" },
        "ref": { "type": "string", "description": "ref from the last ui_snapshot, e.g. 'e12' (preferred)" },
        "role": { "type": "string", "description": "role, when there is no ref: button, textbox, link, checkbox, tab, listitem, menuitem..." },
        "name": { "type": "string", "description": "the control's name as shown in the snapshot, when there is no ref" }
    });
    if let (Some(p), serde_json::Value::Object(e)) = (props.as_object_mut(), extra) {
        p.extend(e);
    }
    serde_json::json!({ "type": "object", "properties": props, "required": required })
}

#[async_trait]
impl Tool for HandTool {
    fn spec(&self) -> ToolSpec {
        let (name, description, parameters, risk) = match self.kind {
            Kind::Windows => (
                "ui_windows",
                "List the windows open on the user's PC (app and title).",
                serde_json::json!({ "type": "object", "properties": {} }),
                Risk::Read,
            ),
            Kind::Snapshot => (
                "ui_snapshot",
                "Read the controls of a window (buttons, fields, links, list items), each with a ref to act \
                 on, like a browser's accessibility snapshot. Take one before clicking or typing.",
                serde_json::json!({ "type": "object", "properties": { "window": { "type": "string", "description": "app or window title, e.g. 'Spotify'" } }, "required": ["window"] }),
                Risk::Read,
            ),
            Kind::Click => (
                "ui_click",
                "Press a button, link, tab, checkbox or list item in a window, by ref (best) or role and name.",
                target_schema(
                    serde_json::json!({ "double": { "type": "boolean", "description": "double-click instead (open a file, play a track)" } }),
                    &[],
                ),
                Risk::Reversible,
            ),
            Kind::Fill => (
                "ui_fill",
                "Type text into a field of a window (replacing what is in it), optionally pressing Enter.",
                target_schema(
                    serde_json::json!({
                        "text": { "type": "string" },
                        "submit": { "type": "boolean", "description": "press Enter afterwards (searches; in a chat it sends)" }
                    }),
                    &["text"],
                ),
                Risk::Reversible,
            ),
            Kind::Press => (
                "ui_press",
                "Bring a window to the front and press a key or shortcut in it, e.g. 'ctrl+f', 'space', 'escape'.",
                serde_json::json!({ "type": "object", "properties": {
                    "window": { "type": "string" },
                    "keys": { "type": "string", "description": "e.g. 'ctrl+right', 'f5', 'tab'" }
                }, "required": ["window", "keys"] }),
                Risk::Reversible,
            ),
        };
        ToolSpec { name: name.into(), description: description.into(), parameters, risk }
    }

    fn enabled(&self) -> bool {
        self.hands.settings().enabled
    }

    fn risk(&self, args: &serde_json::Value) -> Risk {
        match self.kind {
            Kind::Windows | Kind::Snapshot => Risk::Read,
            Kind::Click => {
                ui::risk_of(&parse::<ClickArgs>(args).map(|a| self.hands.target_name(&a.target)).unwrap_or_default())
            }
            Kind::Fill => match parse::<FillArgs>(args) {
                Ok(a) => {
                    let name = self.hands.target_name(&a.target);
                    let field = ui::Element::new(Role::Edit, name.clone());
                    let submit = if a.submit { ui::submit_risk(&field) } else { Risk::Reversible };
                    submit.max(ui::risk_of(&name))
                }
                Err(_) => Risk::Destructive,
            },
            Kind::Press => parse::<PressArgs>(args)
                .ok()
                .and_then(|a| KeyChord::parse(&a.keys))
                .map_or(Risk::Destructive, |k| k.risk()),
        }
    }

    fn describe(&self, args: &serde_json::Value) -> String {
        let window = args.get("window").and_then(|w| w.as_str()).map(|w| format!(" in {w}")).unwrap_or_default();
        match self.kind {
            Kind::Windows => "list the open windows".into(),
            Kind::Snapshot => format!("read the controls{window}"),
            Kind::Click => {
                let name = parse::<ClickArgs>(args).map(|a| self.hands.target_name(&a.target)).unwrap_or_default();
                format!("press \"{name}\"{window}")
            }
            Kind::Fill => match parse::<FillArgs>(args) {
                Ok(a) => format!(
                    "type \"{}\" into \"{}\"{window}{}",
                    a.text,
                    self.hands.target_name(&a.target),
                    if a.submit { ", then Enter" } else { "" }
                ),
                Err(_) => format!("type into a field{window}"),
            },
            Kind::Press => format!("press {}{window}", args.get("keys").and_then(|k| k.as_str()).unwrap_or("?")),
        }
    }

    async fn run(&self, args: serde_json::Value) -> Result<String, ToolError> {
        let h = &self.hands;
        match self.kind {
            Kind::Windows => h.list().await,
            Kind::Snapshot => h.snapshot(parse::<WindowArg>(&args)?.window.as_deref()).await,
            Kind::Click => {
                let a: ClickArgs = parse(&args)?;
                let action = if a.double { UiAction::DoubleClick } else { UiAction::Click };
                let (r, name) = h.act(&a.target, &action).await?;
                Ok(format!(
                    "pressed {} \"{name}\" in {}",
                    r.locator.role.map_or("control", Role::as_str),
                    r.window.process
                ))
            }
            Kind::Fill => {
                let a: FillArgs = parse(&args)?;
                let (r, name) = h.act(&a.target, &UiAction::Fill(a.text.clone())).await?;
                if a.submit {
                    let enter = KeyChord::parse("enter").ok_or_else(|| ToolError::Failed("no enter key".into()))?;
                    h.ui.press(&WindowQuery::id(r.window.id), &enter).await.map_err(ui_failed)?;
                }
                Ok(format!("typed into \"{name}\"{}", if a.submit { " and pressed Enter" } else { "" }))
            }
            Kind::Press => {
                let a: PressArgs = parse(&args)?;
                let keys = KeyChord::parse(&a.keys)
                    .ok_or_else(|| ToolError::BadArguments(format!("unknown keys: {}", a.keys)))?;
                let w = h.find_window(a.window.as_deref()).await?;
                h.ui.press(&WindowQuery::id(w.id), &keys).await.map_err(ui_failed)?;
                Ok(format!("pressed {keys} in {}", w.process))
            }
        }
    }
}
