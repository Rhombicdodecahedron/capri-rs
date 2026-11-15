//! Places she can stand or sit: window top edges, desktop icons, the taskbar line (the floor).

use serde::{Deserialize, Serialize};

use crate::geometry::Point;

/// Stable identity of a ledge across refreshes (a window handle, an icon index), so she can follow a
/// window that moves and notice one that closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LedgeId {
    /// A window's top edge. The low 56 bits are the window handle; the top byte numbers the visible
    /// pieces of an edge split by windows above it. See [`LedgeId::window_piece`].
    Window(u64),
    Icon(u32),
    /// The bottom of the work area of monitor `n`.
    Floor(u32),
}

const PIECE_SHIFT: u32 = 56;

impl LedgeId {
    /// Piece `piece` of the top edge of window `handle`.
    pub fn window_piece(handle: u64, piece: u8) -> Self {
        LedgeId::Window((handle & ((1 << PIECE_SHIFT) - 1)) | (u64::from(piece) << PIECE_SHIFT))
    }

    /// The window this ledge belongs to, if it is a window edge.
    pub fn window_handle(self) -> Option<u64> {
        match self {
            LedgeId::Window(id) => Some(id & ((1 << PIECE_SHIFT) - 1)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgeKind {
    Window,
    Icon,
    Floor,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ledge {
    pub id: LedgeId,
    pub kind: LedgeKind,
    pub left: f32,
    pub right: f32,
    /// Top edge she rests on, in screen pixels.
    pub y: f32,
}

impl Ledge {
    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn spans(&self, x: f32) -> bool {
        x >= self.left && x <= self.right
    }

    /// She sits on window edges and icons, and stands on the floor.
    pub fn is_seat(&self) -> bool {
        !matches!(self.kind, LedgeKind::Floor)
    }

    /// A ledge too narrow for her to sit on is ignored.
    pub fn fits(&self, min_width: f32) -> bool {
        self.width() >= min_width
    }
}

/// The highest ledge under `x` that she would cross when moving down from `from_y` to `to_y`.
/// Used for landing while falling: the first edge she passes through catches her.
pub fn landing(ledges: &[Ledge], x: f32, from_y: f32, to_y: f32) -> Option<&Ledge> {
    ledges.iter().filter(|l| l.spans(x) && l.y >= from_y && l.y <= to_y).min_by(|a, b| a.y.total_cmp(&b.y))
}

/// The ledge directly below `p` (for dropping her where the user let go).
pub fn below(ledges: &[Ledge], p: Point) -> Option<&Ledge> {
    landing(ledges, p.x, p.y, f32::INFINITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: u64, left: f32, right: f32, y: f32) -> Ledge {
        Ledge { id: LedgeId::Window(id), kind: LedgeKind::Window, left, right, y }
    }

    #[test]
    fn landing_picks_first_edge_crossed() {
        let ledges = [window(1, 0.0, 100.0, 300.0), window(2, 0.0, 100.0, 200.0)];
        assert_eq!(landing(&ledges, 50.0, 150.0, 400.0).unwrap().id, LedgeId::Window(2));
    }

    #[test]
    fn landing_ignores_edges_not_under_her() {
        let ledges = [window(1, 200.0, 300.0, 250.0)];
        assert!(landing(&ledges, 50.0, 0.0, 400.0).is_none());
    }

    #[test]
    fn pieces_of_one_window_share_its_handle() {
        let a = LedgeId::window_piece(0x1234, 0);
        let b = LedgeId::window_piece(0x1234, 2);
        assert_ne!(a, b);
        assert_eq!(a.window_handle(), Some(0x1234));
        assert_eq!(b.window_handle(), Some(0x1234));
        assert_eq!(LedgeId::Floor(0).window_handle(), None);
    }

    #[test]
    fn floor_is_not_a_seat() {
        let floor = Ledge { id: LedgeId::Floor(0), kind: LedgeKind::Floor, left: 0.0, right: 1920.0, y: 1040.0 };
        assert!(!floor.is_seat());
        assert!(window(1, 0.0, 10.0, 0.0).is_seat());
    }
}
