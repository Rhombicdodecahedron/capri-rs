//! Her everyday powers on the PC, the ones the original Capri had: open an app by name, open a
//! website, search the web (Google, YouTube, Wikipedia, Maps), tell the time, lock the screen. Plus
//! the [`Opener`] the Spotify remote uses for `spotify:` links.
//!
//! - Windows: apps are the Start menu and desktop shortcuts plus the built-in ones (calculator,
//!   settings...), started through the shell like a double-click.
//! - Linux: `.desktop` entries, started with `gtk-launch`; links with `xdg-open`.
//! - macOS: `/Applications`, started with `open -a`; links with `open`.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use capri_application::ports::{Opener, Tool, ToolError, ToolSpec};
use capri_domain::Risk;
use capri_domain::launcher::{self, SearchSite};
use serde::Deserialize;

#[cfg(windows)]
mod windows;

/// Opens links, files and app URIs with whatever the system has registered for them.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, target: &str) -> Result<(), String> {
        open(target)
    }
}

#[cfg(windows)]
fn open(target: &str) -> Result<(), String> {
    windows::shell_open(target)
}

#[cfg(not(windows))]
fn open(target: &str) -> Result<(), String> {
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(program).arg(target).spawn().map(|_| ()).map_err(|e| format!("{program}: {e}"))
}

/// An app she can start: what it is called, and how to start it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub name: String,
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A shortcut, program or URI handed to the shell.
    Open(String),
    /// A Linux desktop entry id, for `gtk-launch`.
    Desktop(String),
    /// A macOS app name, for `open -a`.
    MacApp(String),
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 && path.extension().is_none_or(|x| x != "app") {
                walk(&path, depth - 1, out);
            } else if path.extension().is_some_and(|x| x == "app") {
                out.push(path);
            }
        } else {
            out.push(path);
        }
    }
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The apps installed for this user.
pub fn installed_apps() -> Vec<App> {
    let mut files = Vec::new();
    let mut apps = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramData", "APPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                walk(&PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs"), 4, &mut files);
            }
        }
        for var in ["USERPROFILE", "PUBLIC"] {
            if let Some(base) = std::env::var_os(var) {
                walk(&PathBuf::from(base).join("Desktop"), 0, &mut files);
            }
        }
        for f in &files {
            let ext = f.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if matches!(ext.as_str(), "lnk" | "url" | "appref-ms") {
                apps.push(App { name: stem(f), target: Target::Open(f.to_string_lossy().into_owned()) });
            }
        }
        for (name, target) in [
            ("Calculator", "calc.exe"),
            ("Notepad", "notepad.exe"),
            ("Paint", "mspaint.exe"),
            ("File Explorer", "explorer.exe"),
            ("Settings", "ms-settings:"),
            ("Task Manager", "taskmgr.exe"),
            ("Terminal", "wt.exe"),
            ("Clock", "ms-clock:"),
            ("Camera", "microsoft.windows.camera:"),
            ("Microsoft Store", "ms-windows-store:"),
            ("Spotify", "spotify:"),
            ("Browser", "https://www.google.com"),
        ] {
            apps.push(App { name: name.into(), target: Target::Open(target.into()) });
        }
    } else if cfg!(target_os = "macos") {
        for dir in ["/Applications", "/System/Applications"] {
            walk(Path::new(dir), 1, &mut files);
        }
        if let Some(home) = std::env::var_os("HOME") {
            walk(&PathBuf::from(home).join("Applications"), 1, &mut files);
        }
        for f in files.iter().filter(|f| f.extension().is_some_and(|e| e == "app")) {
            apps.push(App { name: stem(f), target: Target::MacApp(stem(f)) });
        }
    } else {
        let mut dirs = vec![
            PathBuf::from("/usr/share/applications"),
            PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".local/share/applications"));
        }
        for d in dirs {
            walk(&d, 1, &mut files);
        }
        for f in files.iter().filter(|f| f.extension().is_some_and(|e| e == "desktop")) {
            let Ok(text) = std::fs::read_to_string(f) else { continue };
            if text.lines().any(|l| l.trim() == "NoDisplay=true") {
                continue;
            }
            if let Some(name) = text.lines().find_map(|l| l.strip_prefix("Name=")) {
                apps.push(App { name: name.trim().to_string(), target: Target::Desktop(stem(f)) });
            }
        }
    }
    apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    apps
}

fn start(app: &App) -> Result<(), String> {
    match &app.target {
        Target::Open(t) => open(t),
        Target::Desktop(id) => {
            std::process::Command::new("gtk-launch").arg(id).spawn().map(|_| ()).map_err(|e| format!("gtk-launch: {e}"))
        }
        Target::MacApp(name) => {
            std::process::Command::new("open").args(["-a", name]).spawn().map(|_| ()).map_err(|e| format!("open: {e}"))
        }
    }
}

fn args<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<T, ToolError> {
    serde_json::from_value(v).map_err(|e| ToolError::BadArguments(e.to_string()))
}

fn spec(name: &str, description: &str, parameters: serde_json::Value, risk: Risk) -> ToolSpec {
    ToolSpec { name: name.into(), description: description.into(), parameters, risk }
}

/// `open_app`: starts an installed app by the name the user gives it.
#[derive(Debug, Default, Clone, Copy)]
pub struct AppLauncher;

#[derive(Deserialize)]
struct NameArg {
    name: String,
}

#[async_trait]
impl Tool for AppLauncher {
    fn spec(&self) -> ToolSpec {
        spec(
            "open_app",
            "Open an app installed on the user's PC by its name (Steam, Discord, the calculator, Word, \
             a game...). French or English names both work.",
            serde_json::json!({ "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] }),
            Risk::Reversible,
        )
    }

    fn describe(&self, a: &serde_json::Value) -> String {
        format!("open {}", a.get("name").and_then(|n| n.as_str()).unwrap_or("an app"))
    }

    async fn run(&self, a: serde_json::Value) -> Result<String, ToolError> {
        let NameArg { name } = args(a)?;
        let apps = installed_apps();
        let names: Vec<String> = apps.iter().map(|a| a.name.clone()).collect();
        let app = launcher::best_app(&name, &names)
            .map(|i| &apps[i])
            .ok_or_else(|| ToolError::Failed(format!("no app called \"{name}\" is installed")))?;
        tracing::info!(asked = %name, found = %app.name, "open app");
        start(app).map_err(ToolError::Failed)?;
        Ok(format!("opened {}", app.name))
    }
}

/// `open_website`: a web address in the default browser.
#[derive(Debug, Default, Clone, Copy)]
pub struct Website;

#[derive(Deserialize)]
struct UrlArg {
    url: String,
}

#[async_trait]
impl Tool for Website {
    fn spec(&self) -> ToolSpec {
        spec(
            "open_website",
            "Open a website in the user's browser.",
            serde_json::json!({ "type": "object", "properties": { "url": { "type": "string", "description": "e.g. 'youtube.com' or a full https:// link" } }, "required": ["url"] }),
            Risk::Reversible,
        )
    }

    fn describe(&self, a: &serde_json::Value) -> String {
        format!("open {}", a.get("url").and_then(|n| n.as_str()).unwrap_or("a website"))
    }

    async fn run(&self, a: serde_json::Value) -> Result<String, ToolError> {
        let UrlArg { url } = args(a)?;
        let url =
            launcher::web_url(&url).ok_or_else(|| ToolError::BadArguments(format!("not a web address: {url}")))?;
        open(&url).map_err(ToolError::Failed)?;
        Ok(format!("opened {url}"))
    }
}

/// `web_search`: a search on Google, YouTube, Wikipedia or Maps, in the browser.
#[derive(Debug, Default, Clone, Copy)]
pub struct WebSearch;

#[derive(Deserialize)]
struct SearchArg {
    query: String,
    #[serde(default)]
    site: Option<String>,
}

#[async_trait]
impl Tool for WebSearch {
    fn spec(&self) -> ToolSpec {
        spec(
            "web_search",
            "Search the web in the user's browser: Google by default, or YouTube, Wikipedia, Maps, Images.",
            serde_json::json!({ "type": "object", "properties": {
                "query": { "type": "string" },
                "site": { "type": "string", "enum": ["web", "youtube", "wikipedia", "maps", "images"] }
            }, "required": ["query"] }),
            Risk::Reversible,
        )
    }

    fn describe(&self, a: &serde_json::Value) -> String {
        let q = a.get("query").and_then(|n| n.as_str()).unwrap_or("?");
        let site = a.get("site").and_then(|n| n.as_str()).unwrap_or("the web");
        format!("search {site} for \"{q}\"")
    }

    async fn run(&self, a: serde_json::Value) -> Result<String, ToolError> {
        let SearchArg { query, site } = args(a)?;
        let site = site.as_deref().map_or(SearchSite::Web, SearchSite::parse);
        let url = launcher::search_url(&query, site);
        open(&url).map_err(ToolError::Failed)?;
        Ok(format!("opened the search for \"{query}\" in the browser"))
    }
}

/// `check_time`: the date and time on the PC.
#[derive(Debug, Default, Clone, Copy)]
pub struct Clock;

#[async_trait]
impl Tool for Clock {
    fn spec(&self) -> ToolSpec {
        spec(
            "check_time",
            "The current date and time on the user's PC.",
            serde_json::json!({ "type": "object", "properties": {} }),
            Risk::Read,
        )
    }

    async fn run(&self, _: serde_json::Value) -> Result<String, ToolError> {
        Ok(chrono::Local::now().format("%A %-d %B %Y, %H:%M").to_string())
    }
}

/// `lock_pc`: the lock screen.
#[derive(Debug, Default, Clone, Copy)]
pub struct LockScreen;

#[async_trait]
impl Tool for LockScreen {
    fn spec(&self) -> ToolSpec {
        spec(
            "lock_pc",
            "Lock the user's PC (the sign-in screen). Nothing is closed.",
            serde_json::json!({ "type": "object", "properties": {} }),
            Risk::Reversible,
        )
    }

    fn describe(&self, _: &serde_json::Value) -> String {
        "lock the PC".into()
    }

    async fn run(&self, _: serde_json::Value) -> Result<String, ToolError> {
        lock().map_err(ToolError::Failed)?;
        Ok("locked the PC".into())
    }
}

#[cfg(windows)]
fn lock() -> Result<(), String> {
    windows::lock()
}

#[cfg(not(windows))]
fn lock() -> Result<(), String> {
    let (program, args): (&str, &[&str]) =
        if cfg!(target_os = "macos") { ("pmset", &["displaysleepnow"]) } else { ("loginctl", &["lock-session"]) };
    std::process::Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("{program}: {e}"))
        .and_then(|s| if s.success() { Ok(()) } else { Err(format!("{program} failed ({s})")) })
}

/// Every tool in this crate.
pub fn tools() -> Vec<std::sync::Arc<dyn Tool>> {
    vec![
        std::sync::Arc::new(AppLauncher),
        std::sync::Arc::new(Website),
        std::sync::Arc::new(WebSearch),
        std::sync::Arc::new(Clock),
        std::sync::Arc::new(LockScreen),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn websites_refuse_anything_but_the_web() {
        let r = Website.run(serde_json::json!({"url": "file:///etc/passwd"})).await;
        assert!(matches!(r, Err(ToolError::BadArguments(_))));
    }

    #[tokio::test]
    async fn the_clock_reads_the_time() {
        let t = Clock.run(serde_json::json!({})).await.unwrap();
        assert!(t.contains(':'), "{t}");
    }

    #[test]
    fn tool_names_are_unique_and_model_safe() {
        let specs: Vec<_> = tools().iter().map(|t| t.spec().name).collect();
        let mut sorted = specs.clone();
        sorted.dedup();
        assert_eq!(specs.len(), sorted.len());
        assert!(specs.iter().all(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')));
    }
}
