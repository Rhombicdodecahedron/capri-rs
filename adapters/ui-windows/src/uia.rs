//! UI Automation calls. Each runs on a blocking thread with its own COM apartment: elements never
//! leave the call that found them, and every action looks its control up again (locators, not handles).

use std::collections::HashMap;
use std::time::Duration;

use capri_application::ports::{UiAction, UiError};
use capri_domain::geometry::Rect;
use capri_domain::ui::{Element, KeyChord, Locator, Role, Snapshot, WindowInfo, WindowQuery};
use uiautomation::patterns::{
    UIExpandCollapsePattern, UIInvokePattern, UILegacyIAccessiblePattern, UISelectionItemPattern, UITogglePattern,
    UIValuePattern,
};
use uiautomation::types::{ControlType, TreeScope, UIProperty};
use uiautomation::variants::Variant;
use uiautomation::{UIAutomation, UIElement};

use crate::win32;

/// A window with thousands of controls is cut here.
const MAX_ELEMENTS: usize = 4000;

fn failed(e: &uiautomation::Error) -> UiError {
    UiError::Failed(e.to_string())
}

fn automation() -> Result<UIAutomation, UiError> {
    UIAutomation::new().map_err(|e| failed(&e))
}

fn int(v: &Variant) -> Option<i32> {
    TryInto::<i32>::try_into(v).ok()
}

fn hwnd_of(el: &UIElement) -> u64 {
    el.get_property_value(UIProperty::NativeWindowHandle).ok().and_then(|v| int(&v)).map_or(0, |h| u64::from(h as u32))
}

/// Top-level windows with their element, front to back.
fn top_level(auto: &UIAutomation) -> Result<Vec<(WindowInfo, UIElement)>, UiError> {
    let root = auto.get_root_element().map_err(|e| failed(&e))?;
    let all = auto.create_true_condition().map_err(|e| failed(&e))?;
    let children = root.find_all(TreeScope::Children, &all).map_err(|e| failed(&e))?;
    let mut names: HashMap<u32, String> = HashMap::new();
    let mut out = Vec::with_capacity(children.len());
    for el in children {
        let id = hwnd_of(&el);
        if id == 0 {
            continue;
        }
        let pid = el.get_process_id().unwrap_or_default();
        let process = names.entry(pid).or_insert_with(|| win32::process_name(pid)).clone();
        let title = el.get_name().unwrap_or_default();
        out.push((WindowInfo { id, title, process, pid }, el));
    }
    Ok(out)
}

fn find_window(auto: &UIAutomation, q: &WindowQuery) -> Result<(WindowInfo, UIElement), UiError> {
    top_level(auto)?.into_iter().find(|(w, _)| q.matches(w)).ok_or_else(|| UiError::NoWindow(describe(q)))
}

fn describe(q: &WindowQuery) -> String {
    match (&q.process, &q.title, q.id) {
        (Some(p), _, _) => p.clone(),
        (_, Some(t), _) => format!("\"{t}\""),
        (_, _, Some(id)) => format!("window {id:#x}"),
        _ => "any window".into(),
    }
}

pub fn windows() -> Result<Vec<WindowInfo>, UiError> {
    let auto = automation()?;
    Ok(top_level(&auto)?.into_iter().map(|(w, _)| w).collect())
}

fn role_of(t: ControlType) -> Role {
    match t {
        ControlType::Button | ControlType::SplitButton => Role::Button,
        ControlType::CheckBox => Role::CheckBox,
        ControlType::ComboBox => Role::ComboBox,
        ControlType::Edit => Role::Edit,
        ControlType::Hyperlink => Role::Link,
        ControlType::List => Role::List,
        ControlType::ListItem => Role::ListItem,
        ControlType::Menu | ControlType::MenuBar => Role::Menu,
        ControlType::MenuItem => Role::MenuItem,
        ControlType::RadioButton => Role::RadioButton,
        ControlType::Slider | ControlType::Spinner => Role::Slider,
        ControlType::Tab => Role::Tab,
        ControlType::TabItem => Role::TabItem,
        ControlType::Text => Role::Text,
        ControlType::Tree => Role::Tree,
        ControlType::TreeItem => Role::TreeItem,
        ControlType::Table | ControlType::DataGrid => Role::Table,
        ControlType::DataItem => Role::Row,
        ControlType::Document => Role::Document,
        ControlType::Group => Role::Group,
        ControlType::Image => Role::Image,
        ControlType::Window => Role::Window,
        _ => Role::Other,
    }
}

/// The window's controls, as domain elements and as live UI Automation elements, index for index.
fn controls(auto: &UIAutomation, win: &UIElement, id: u64) -> Result<(Vec<Element>, Vec<UIElement>), UiError> {
    let cache = auto.create_cache_request().map_err(|e| failed(&e))?;
    for p in [
        UIProperty::Name,
        UIProperty::ControlType,
        UIProperty::IsEnabled,
        UIProperty::IsOffscreen,
        UIProperty::IsPassword,
        UIProperty::BoundingRectangle,
        UIProperty::ValueValue,
        UIProperty::ToggleToggleState,
    ] {
        cache.add_property(p).map_err(|e| failed(&e))?;
    }
    let condition = auto.get_control_view_condition().map_err(|e| failed(&e))?;
    let found = win.find_all_build_cache(TreeScope::Descendants, &condition, &cache).map_err(|e| failed(&e))?;
    // a minimized window reports everything offscreen: keep it all then
    let minimized = win32::is_minimized(id);
    let (mut elements, mut live) = (Vec::new(), Vec::new());
    for el in found.into_iter().take(MAX_ELEMENTS) {
        if !minimized && el.is_cached_offscreen().unwrap_or(false) {
            continue;
        }
        let role = el.get_cached_control_type().map_or(Role::Other, role_of);
        let password = el.is_cached_password().unwrap_or(false);
        let value = (!password)
            .then(|| el.get_cached_property_value(UIProperty::ValueValue).ok())
            .flatten()
            .and_then(|v| TryInto::<String>::try_into(&v).ok())
            .filter(|v| !v.trim().is_empty());
        let checked = el.get_cached_property_value(UIProperty::ToggleToggleState).ok().and_then(|v| int(&v)).and_then(
            |s| match s {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            },
        );
        let bounds = el.get_cached_bounding_rectangle().ok().and_then(|r| {
            let (w, h) = (r.get_right() - r.get_left(), r.get_bottom() - r.get_top());
            (w > 0 && h > 0).then(|| Rect::new(r.get_left() as f32, r.get_top() as f32, w as f32, h as f32))
        });
        elements.push(Element {
            role,
            name: el.get_cached_name().unwrap_or_default(),
            value,
            enabled: el.is_cached_enabled().unwrap_or(true),
            checked,
            bounds,
        });
        live.push(el);
    }
    Ok((elements, live))
}

/// Chromium and Electron apps (Spotify, Discord, browsers) build their accessibility tree the first
/// time a reader asks: give them a moment before calling the window empty.
fn controls_patiently(
    auto: &UIAutomation,
    win: &UIElement,
    id: u64,
) -> Result<(Vec<Element>, Vec<UIElement>), UiError> {
    let chromium = win.get_classname().is_ok_and(|c| c.starts_with("Chrome_WidgetWin"));
    let mut got = controls(auto, win, id)?;
    let mut tries = 0;
    while chromium && got.0.len() < 8 && tries < 6 {
        std::thread::sleep(Duration::from_millis(350));
        got = controls(auto, win, id)?;
        tries += 1;
    }
    Ok(got)
}

pub fn snapshot(q: &WindowQuery) -> Result<Snapshot, UiError> {
    let auto = automation()?;
    let (window, el) = find_window(&auto, q)?;
    let (elements, _) = controls_patiently(&auto, &el, window.id)?;
    tracing::debug!(window = %window.title, controls = elements.len(), "snapshot");
    Ok(Snapshot { window, elements })
}

/// Presses a control the accessible way when it can (no mouse, works in the background), else clicks it.
fn press(el: &UIElement, window: u64) -> Result<(), UiError> {
    if el.get_pattern::<UIInvokePattern>().is_ok_and(|p| p.invoke().is_ok()) {
        return Ok(());
    }
    if el.get_pattern::<UITogglePattern>().is_ok_and(|p| p.toggle().is_ok()) {
        return Ok(());
    }
    if el.get_pattern::<UISelectionItemPattern>().is_ok_and(|p| p.select().is_ok()) {
        return Ok(());
    }
    if el.get_pattern::<UIExpandCollapsePattern>().is_ok_and(|p| p.expand().is_ok()) {
        return Ok(());
    }
    // Chromium and Electron controls reached through the older MSAA bridge: their default action
    if el.get_pattern::<UILegacyIAccessiblePattern>().is_ok_and(|p| p.do_default_action().is_ok()) {
        return Ok(());
    }
    win32::bring_to_front(window);
    el.click().map_err(|e| failed(&e))
}

fn fill(el: &UIElement, window: u64, text: &str) -> Result<(), UiError> {
    if el.get_pattern::<UIValuePattern>().is_ok_and(|p| p.set_value(text).is_ok()) {
        return Ok(());
    }
    // fields without a value pattern (rich editors, web inputs): select all and type over it
    win32::bring_to_front(window);
    el.set_focus().map_err(|e| failed(&e))?;
    let select_all = KeyChord::parse("ctrl+a").ok_or_else(|| UiError::Failed("no ctrl+a".into()))?;
    win32::send_chord(&select_all).map_err(UiError::Failed)?;
    el.send_text(text, 5).map_err(|e| failed(&e))
}

pub fn act(q: &WindowQuery, target: &Locator, action: &UiAction) -> Result<String, UiError> {
    let auto = automation()?;
    let (window, win) = find_window(&auto, q)?;
    let (elements, live) = controls_patiently(&auto, &win, window.id)?;
    let i = target.pick(&elements).ok_or_else(|| UiError::NotFound(target.describe()))?;
    let el = &live[i];
    tracing::info!(window = %window.process, control = %elements[i].name, ?action, "ui action");
    match action {
        UiAction::Click => press(el, window.id)?,
        UiAction::DoubleClick => {
            win32::bring_to_front(window.id);
            el.double_click().map_err(|e| failed(&e))?;
        }
        UiAction::Fill(text) => fill(el, window.id, text)?,
        UiAction::Focus => el.set_focus().map_err(|e| failed(&e))?,
    }
    Ok(elements[i].name.clone())
}

pub fn press_keys(q: &WindowQuery, keys: &KeyChord) -> Result<(), UiError> {
    let auto = automation()?;
    let (window, _) = find_window(&auto, q)?;
    win32::bring_to_front(window.id);
    win32::send_chord(keys).map_err(UiError::Failed)
}
