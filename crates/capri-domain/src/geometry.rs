//! Screen geometry in physical screen pixels, the unit the OS reports window rects in. The app scales
//! the character size by the monitor DPI factor, so she looks the same size on a 100% or 150% screen.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// Clamps `p` so it stays inside the rect.
    pub fn clamp(&self, p: Point) -> Point {
        Point::new(p.x.clamp(self.x, self.right()), p.y.clamp(self.y, self.bottom()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_is_half_open() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(Point::new(0.0, 0.0)));
        assert!(!r.contains(Point::new(10.0, 5.0)));
    }

    #[test]
    fn clamp_keeps_point_inside() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(r.clamp(Point::new(-5.0, 20.0)), Point::new(0.0, 10.0));
    }
}
