//! Which parts of each window's top edge are actually visible, given the windows stacked above it.

use capri_domain::{Ledge, LedgeId, LedgeKind, Rect};

/// A top-level window, as reported by the OS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowRect {
    pub id: u64,
    pub rect: Rect,
}

/// `windows` must be in z-order, topmost first. Returns one ledge per visible piece of a top edge that
/// is at least `min_width` wide and lies inside a work area. A window whose top touches the top of its
/// screen (maximised, fullscreen) gives no ledge: she would sit off-screen.
pub fn visible_edges(windows: &[WindowRect], areas: &[Rect], min_width: f32) -> Vec<Ledge> {
    let mut out = Vec::new();
    for (i, w) in windows.iter().enumerate() {
        let y = w.rect.y;
        let Some(area) =
            areas.iter().find(|a| y > a.y + 2.0 && y < a.bottom() && w.rect.x < a.right() && w.rect.right() > a.x)
        else {
            continue;
        };
        let mut pieces = vec![(w.rect.x.max(area.x), w.rect.right().min(area.right()))];
        for above in &windows[..i] {
            let r = above.rect;
            // the edge is hidden where a higher window covers the row she would sit on, and a bit above
            // it (her body would be drawn behind that window otherwise)
            if r.y <= y && r.bottom() > y - 4.0 {
                pieces = subtract(&pieces, r.x, r.right());
            }
            if pieces.is_empty() {
                break;
            }
        }
        for (k, (l, r)) in pieces.into_iter().enumerate() {
            if r - l >= min_width {
                // pieces of one window share its id only for the first piece; later pieces get a derived id
                let id = if k == 0 { w.id } else { w.id ^ ((k as u64) << 56) };
                out.push(Ledge { id: LedgeId::Window(id), kind: LedgeKind::Window, left: l, right: r, y });
            }
        }
    }
    out
}

fn subtract(pieces: &[(f32, f32)], cut_l: f32, cut_r: f32) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(pieces.len() + 1);
    for &(l, r) in pieces {
        if cut_r <= l || cut_l >= r {
            out.push((l, r));
            continue;
        }
        if cut_l > l {
            out.push((l, cut_l));
        }
        if cut_r < r {
            out.push((cut_r, r));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0.0, 0.0, 1920.0, 1040.0);

    fn w(id: u64, x: f32, y: f32, wd: f32, h: f32) -> WindowRect {
        WindowRect { id, rect: Rect::new(x, y, wd, h) }
    }

    #[test]
    fn uncovered_window_gives_full_edge() {
        let l = visible_edges(&[w(1, 100.0, 200.0, 800.0, 600.0)], &[SCREEN], 50.0);
        assert_eq!(l.len(), 1);
        assert!((l[0].left - 100.0).abs() < f32::EPSILON && (l[0].right - 900.0).abs() < f32::EPSILON);
    }

    #[test]
    fn window_above_splits_the_edge() {
        let top = w(2, 400.0, 100.0, 200.0, 400.0);
        let below = w(1, 100.0, 200.0, 800.0, 600.0);
        let l = visible_edges(&[top, below], &[SCREEN], 50.0);
        let pieces: Vec<_> =
            l.iter().filter(|l| (l.y - 200.0).abs() < f32::EPSILON).map(|l| (l.left, l.right)).collect();
        assert_eq!(pieces, vec![(100.0, 400.0), (600.0, 900.0)]);
    }

    #[test]
    fn maximised_windows_and_slivers_are_skipped() {
        let l = visible_edges(&[w(1, 0.0, 0.0, 1920.0, 1040.0), w(2, 10.0, 300.0, 30.0, 100.0)], &[SCREEN], 50.0);
        assert_eq!(l, Vec::<Ledge>::new());
    }

    #[test]
    fn window_fully_behind_another_is_hidden() {
        let l = visible_edges(&[w(2, 0.0, 100.0, 1000.0, 800.0), w(1, 100.0, 300.0, 400.0, 200.0)], &[SCREEN], 50.0);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].id, LedgeId::Window(2));
    }
}
