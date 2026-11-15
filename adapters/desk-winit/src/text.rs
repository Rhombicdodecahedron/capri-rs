//! Text drawing with ab_glyph straight into a tiny-skia pixmap (no system font stack needed).

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use tiny_skia::{Color, Pixmap};

pub static SHARE_TECH_MONO: &[u8] = include_bytes!("../../../assets/fonts/ShareTechMono-Regular.ttf");

#[derive(Debug, Clone)]
pub struct Typeface {
    font: FontRef<'static>,
}

impl Default for Typeface {
    fn default() -> Self {
        Self { font: FontRef::try_from_slice(SHARE_TECH_MONO).expect("bundled font is valid") }
    }
}

impl Typeface {
    pub fn line_height(&self, px: f32) -> f32 {
        let f = self.font.as_scaled(PxScale::from(px));
        f.ascent() - f.descent() + f.line_gap()
    }

    pub fn width(&self, text: &str, px: f32) -> f32 {
        let f = self.font.as_scaled(PxScale::from(px));
        let mut w = 0.0;
        let mut prev = None;
        for c in text.chars() {
            let id = f.glyph_id(c);
            if let Some(p) = prev {
                w += f.kern(p, id);
            }
            w += f.h_advance(id);
            prev = Some(id);
        }
        w
    }

    /// Greedy word wrap; words longer than a line are cut by characters.
    pub fn wrap(&self, text: &str, px: f32, max_width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for word in paragraph.split_whitespace() {
                let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                if self.width(&candidate, px) <= max_width {
                    line = candidate;
                    continue;
                }
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                let mut piece = String::new();
                for c in word.chars() {
                    piece.push(c);
                    if self.width(&piece, px) > max_width {
                        piece.pop();
                        lines.push(std::mem::take(&mut piece));
                        piece.push(c);
                    }
                }
                line = piece;
            }
            lines.push(line);
        }
        lines
    }

    /// Draws one line with its baseline at `y`.
    pub fn draw(&self, pm: &mut Pixmap, text: &str, px: f32, x: f32, y: f32, color: Color) {
        let f = self.font.as_scaled(PxScale::from(px));
        let (w, h) = (pm.width() as i32, pm.height() as i32);
        let c = color.premultiply().to_color_u8();
        let mut caret = x;
        let mut prev = None;
        for ch in text.chars() {
            let id = f.glyph_id(ch);
            if let Some(p) = prev {
                caret += f.kern(p, id);
            }
            let glyph = id.with_scale_and_position(PxScale::from(px), point(caret, y));
            caret += f.h_advance(id);
            prev = Some(id);
            let Some(outlined) = self.font.outline_glyph(glyph) else { continue };
            let b = outlined.px_bounds();
            let data = pm.pixels_mut();
            outlined.draw(|gx, gy, cov| {
                let px_x = b.min.x as i32 + gx as i32;
                let px_y = b.min.y as i32 + gy as i32;
                if px_x < 0 || px_y < 0 || px_x >= w || px_y >= h {
                    return;
                }
                let i = (px_y * w + px_x) as usize;
                let a = cov.clamp(0.0, 1.0);
                let dst = data[i];
                let blend = |s: u8, d: u8| -> u8 {
                    (f32::from(s) * a + f32::from(d) * (1.0 - a * f32::from(c.alpha()) / 255.0)).round().min(255.0)
                        as u8
                };
                let r = blend(c.red(), dst.red());
                let g = blend(c.green(), dst.green());
                let bl = blend(c.blue(), dst.blue());
                let al = blend(c.alpha(), dst.alpha());
                if let Some(p) = tiny_skia::PremultipliedColorU8::from_rgba(r.min(al), g.min(al), bl.min(al), al) {
                    data[i] = p;
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_to_width() {
        let t = Typeface::default();
        let lines = t.wrap("the quick brown fox jumps over the lazy dog", 16.0, 120.0);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| t.width(l, 16.0) <= 120.0));
    }

    #[test]
    fn cuts_overlong_words() {
        let t = Typeface::default();
        let lines = t.wrap(&"x".repeat(200), 16.0, 100.0);
        assert!(lines.len() > 1);
    }
}
