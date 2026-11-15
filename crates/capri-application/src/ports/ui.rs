//! Driven port: her hands on other apps, the way Playwright has hands on a web page. The adapter reads
//! a window's accessibility tree and presses controls through it; which control to press is decided
//! here and in the domain ([`Locator`]), so every adapter finds the same button.

use async_trait::async_trait;
use capri_domain::ui::{KeyChord, Locator, Snapshot, WindowInfo, WindowQuery};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UiError {
    #[error("driving apps isn't supported on this system yet")]
    Unsupported,
    #[error("no window matches {0}")]
    NoWindow(String),
    #[error("no control matches {0}")]
    NotFound(String),
    #[error("{0}")]
    Failed(String),
}

/// What to do with the control a locator points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAction {
    /// Press it: the accessibility action when the control has one (no mouse move, works behind
    /// other windows), a real click otherwise.
    Click,
    DoubleClick,
    /// Replace the text in a field.
    Fill(String),
    /// Give it the keyboard focus.
    Focus,
}

#[async_trait]
pub trait UiDriver: Send + Sync {
    /// Visible top-level windows, front to back.
    async fn windows(&self) -> Result<Vec<WindowInfo>, UiError>;

    /// The controls of the first window matching `window`, in tree order.
    async fn snapshot(&self, window: &WindowQuery) -> Result<Snapshot, UiError>;

    /// Finds the control `target` points at (looked up again now, like a Playwright locator) and acts on it.
    /// Returns the control's name.
    async fn act(&self, window: &WindowQuery, target: &Locator, action: &UiAction) -> Result<String, UiError>;

    /// Brings the window to the front and presses a key chord in it.
    async fn press(&self, window: &WindowQuery, keys: &KeyChord) -> Result<(), UiError>;
}

/// Driven port: hands a link, file or `spotify:` URI to the system, like double-clicking it.
pub trait Opener: Send + Sync {
    fn open(&self, target: &str) -> Result<(), String>;
}
