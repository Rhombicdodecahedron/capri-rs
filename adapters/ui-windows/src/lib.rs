//! Her hands on other apps, behind the [`UiDriver`] port.
//!
//! On Windows this is UI Automation, the accessibility API screen readers use: it sees the buttons,
//! fields and lists of native apps, WinUI, WPF, Electron and Chromium-based apps (Spotify, Discord,
//! browsers) alike. Controls are pressed through their accessibility action when they have one
//! (Invoke, Toggle, Select), which works behind other windows and never moves the mouse; a real
//! click is the fallback. Keys go to the window after bringing it to the front.
//!
//! Elsewhere every call answers [`UiError::Unsupported`] for now.

use async_trait::async_trait;
use capri_application::ports::{UiAction, UiDriver, UiError};
use capri_domain::ui::{KeyChord, Locator, Snapshot, WindowInfo, WindowQuery};

#[cfg(windows)]
mod uia;
#[cfg(windows)]
mod win32;

/// Whether this build can drive other apps.
pub const AVAILABLE: bool = cfg!(windows);

/// The platform's UI automation.
#[derive(Debug, Default, Clone, Copy)]
pub struct Automation;

#[cfg(windows)]
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, UiError> + Send + 'static) -> Result<T, UiError> {
    tokio::task::spawn_blocking(f).await.map_err(|e| UiError::Failed(format!("ui thread: {e}")))?
}

#[cfg(windows)]
#[async_trait]
impl UiDriver for Automation {
    async fn windows(&self) -> Result<Vec<WindowInfo>, UiError> {
        blocking(uia::windows).await
    }

    async fn snapshot(&self, window: &WindowQuery) -> Result<Snapshot, UiError> {
        let q = window.clone();
        blocking(move || uia::snapshot(&q)).await
    }

    async fn act(&self, window: &WindowQuery, target: &Locator, action: &UiAction) -> Result<String, UiError> {
        let (q, t, a) = (window.clone(), target.clone(), action.clone());
        blocking(move || uia::act(&q, &t, &a)).await
    }

    async fn press(&self, window: &WindowQuery, keys: &KeyChord) -> Result<(), UiError> {
        let (q, k) = (window.clone(), keys.clone());
        blocking(move || uia::press_keys(&q, &k)).await
    }
}

#[cfg(not(windows))]
#[async_trait]
impl UiDriver for Automation {
    async fn windows(&self) -> Result<Vec<WindowInfo>, UiError> {
        Err(UiError::Unsupported)
    }

    async fn snapshot(&self, _: &WindowQuery) -> Result<Snapshot, UiError> {
        Err(UiError::Unsupported)
    }

    async fn act(&self, _: &WindowQuery, _: &Locator, _: &UiAction) -> Result<String, UiError> {
        Err(UiError::Unsupported)
    }

    async fn press(&self, _: &WindowQuery, _: &KeyChord) -> Result<(), UiError> {
        Err(UiError::Unsupported)
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn other_systems_say_unsupported() {
        const { assert!(!AVAILABLE) };
        assert_eq!(Automation.windows().await, Err(UiError::Unsupported));
    }
}
