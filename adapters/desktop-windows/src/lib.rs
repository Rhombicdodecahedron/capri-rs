//! Desktop probes.
//!
//! - [`Win32Desktop`] (Windows): visible top edges of real windows, in z-order, minus the parts hidden
//!   behind windows above them, plus each monitor's taskbar line as a floor.
//! - [`StaticDesktop`] (everywhere): just the floors of the monitors it is given. Used on macOS and
//!   Linux until their own adapters exist, and in tests.
//!
//! The occlusion maths lives in [`edges`] and is pure, so it is tested on any OS.

use std::sync::RwLock;

use capri_application::ports::DesktopProbe;
use capri_domain::{Ledge, LedgeId, LedgeKind, Rect};

pub mod edges;

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use win32::Win32Desktop;

/// Monitors only: she can stand on each taskbar line, nothing else.
#[derive(Debug, Default)]
pub struct StaticDesktop {
    areas: RwLock<Vec<Rect>>,
}

impl StaticDesktop {
    pub fn new(areas: Vec<Rect>) -> Self {
        Self { areas: RwLock::new(areas) }
    }

    /// Monitors were plugged, unplugged or rescaled.
    pub fn set_areas(&self, areas: Vec<Rect>) {
        if let Ok(mut a) = self.areas.write() {
            *a = areas;
        }
    }
}

impl DesktopProbe for StaticDesktop {
    fn work_areas(&self) -> Vec<Rect> {
        self.areas.read().map(|a| a.clone()).unwrap_or_default()
    }

    fn ledges(&self, _ignore: &[u64]) -> Vec<Ledge> {
        floors(&self.work_areas())
    }
}

pub fn floors(areas: &[Rect]) -> Vec<Ledge> {
    areas
        .iter()
        .enumerate()
        .map(|(i, a)| Ledge {
            id: LedgeId::Floor(i as u32),
            kind: LedgeKind::Floor,
            left: a.x,
            right: a.right(),
            y: a.bottom(),
        })
        .collect()
}

/// The best probe for this OS. `fallback_areas` is used where no native adapter exists yet.
#[allow(clippy::needless_pass_by_value)] // consumed on every OS but Windows
pub fn native(fallback_areas: Vec<Rect>) -> Box<dyn DesktopProbe> {
    #[cfg(windows)]
    {
        let _ = fallback_areas;
        Box::new(Win32Desktop::default())
    }
    #[cfg(not(windows))]
    {
        Box::new(StaticDesktop::new(fallback_areas))
    }
}
