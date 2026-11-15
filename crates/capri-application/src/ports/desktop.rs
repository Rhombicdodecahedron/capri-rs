use capri_domain::{Ledge, Rect};

/// Where one window is right now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tracking {
    /// This probe cannot follow single windows; the periodic refresh is all there is.
    Unsupported,
    /// Closed, minimised or hidden.
    Gone,
    At(Rect),
}

/// Driven port: what is on the user's screens right now.
/// Adapters: Win32 (window top edges, desktop icons, taskbar), and a monitor-only fallback elsewhere.
pub trait DesktopProbe: Send + Sync {
    /// Usable area of each monitor (screen minus taskbar), screen px.
    fn work_areas(&self) -> Vec<Rect>;

    /// Every ledge she could land on, highest priority first. `ignore` lists Capri's own window ids.
    fn ledges(&self, ignore: &[u64]) -> Vec<Ledge>;

    /// The current frame of one window, cheap enough to call every frame: she follows the window she
    /// sits on without waiting for the next full refresh.
    fn track(&self, _window: u64) -> Tracking {
        Tracking::Unsupported
    }
}
