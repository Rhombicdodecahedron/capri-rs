use capri_domain::{Ledge, Rect};

/// Driven port: what is on the user's screens right now.
/// Adapters: Win32 (window top edges, desktop icons, taskbar), and a monitor-only fallback elsewhere.
pub trait DesktopProbe: Send + Sync {
    /// Usable area of each monitor (screen minus taskbar), screen px.
    fn work_areas(&self) -> Vec<Rect>;

    /// Every ledge she could land on, highest priority first. `ignore` lists Capri's own window ids.
    fn ledges(&self, ignore: &[u64]) -> Vec<Ledge>;
}
