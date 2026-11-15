//! Composes one frame of her window: the sprite at her anchor, the mouth overlay, and the chat box.
//! Pure drawing into a pixmap, so it can be rendered to a PNG in tests.

use capri_application::ports::Skin;
use capri_domain::{ClipId, Language, Point, Pose};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, PixmapRef, Stroke, Transform};

use crate::bubble::{Bubble, BubbleState};
use crate::text::Typeface;

pub const NEON_RED: (u8, u8, u8) = (0xff, 0x00, 0x3c);
pub const NEON_CYAN: (u8, u8, u8) = (0x00, 0xf0, 0xff);
pub const NIGHT: (u8, u8, u8) = (0x0a, 0x0a, 0x12);

fn rgb(c: (u8, u8, u8), a: u8) -> Color {
    Color::from_rgba8(c.0, c.1, c.2, a)
}

/// Fixed canvas around her, in multiples of her size: room for the chat box above her head and for
/// her legs dangling below a window edge.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub size: f32,
    pub width: u32,
    pub height: u32,
    /// Where her anchor sits on the canvas.
    pub anchor: Point,
}

impl Layout {
    pub fn for_size(size: f32) -> Self {
        let width = (size * 2.4).round() as u32;
        let height = (size * 2.8).round() as u32;
        Self { size, width, height, anchor: Point::new(size * 1.2, size * 2.3) }
    }

    /// Screen position of the canvas' top-left corner for her anchor at `anchor`.
    pub fn origin(&self, anchor: Point) -> (i32, i32) {
        ((anchor.x - self.anchor.x).round() as i32, (anchor.y - self.anchor.y).round() as i32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Debug)]
pub struct Painter {
    pub layout: Layout,
    pixmap: Pixmap,
    font: Typeface,
    /// Where the chat box was drawn last frame (canvas px), for click handling.
    pub bubble_rect: Option<Rect>,
    pub lang: Language,
    /// Opaque bounds of each frame, measured once: the hit area and where her head really is.
    bounds: std::collections::HashMap<(ClipId, usize), Option<Rect>>,
}

impl Painter {
    pub fn new(layout: Layout) -> Self {
        let pixmap = Pixmap::new(layout.width.max(1), layout.height.max(1)).expect("canvas size is non-zero");
        Self {
            layout,
            pixmap,
            font: Typeface::default(),
            bubble_rect: None,
            lang: Language::En,
            bounds: std::collections::HashMap::new(),
        }
    }

    pub fn pixmap(&self) -> &Pixmap {
        &self.pixmap
    }

    /// Alpha of the last frame at canvas pixel (x, y): used to tell a click on her from a click through.
    pub fn alpha_at(&self, x: f32, y: f32) -> u8 {
        if x < 0.0 || y < 0.0 {
            return 0;
        }
        self.pixmap.pixel(x as u32, y as u32).map_or(0, tiny_skia::PremultipliedColorU8::alpha)
    }

    pub fn paint(&mut self, pose: &Pose, head: Point, skin: &Skin, bubble: &Bubble, now: f32) {
        self.pixmap.fill(Color::TRANSPARENT);
        self.bubble_rect = None;
        if !pose.visible {
            return;
        }
        let a = self.layout.anchor;
        let mut head = Point::new(a.x + head.x - pose.anchor.x, a.y + head.y - pose.anchor.y);
        if let Some(strip) = skin.strip_or_fallback(pose.frame.clip) {
            let i = (pose.frame.index as usize).min(strip.frames.len() - 1);
            let f = &strip.frames[i];
            let x = a.x - strip.anchor.0 * f.width as f32;
            let y = a.y - strip.anchor.1 * f.height as f32;
            let b = *self.bounds.entry((strip.clip, i)).or_insert_with(|| opaque_bounds(f));
            if let Some(b) = b {
                // the hit area: her whole silhouette box, at alpha 1/255 (invisible) so a click
                // anywhere on her lands on her window and not on what is behind her thin legs
                let pad = 6.0 * (self.layout.size / 240.0).max(0.5);
                if let Some(r) =
                    tiny_skia::Rect::from_xywh(x + b.x - pad, y + b.y - pad, b.w + 2.0 * pad, b.h + 2.0 * pad)
                {
                    let mut hit = Paint::default();
                    hit.set_color(Color::from_rgba8(0, 0, 0, 1));
                    self.pixmap.fill_rect(r, &hit, Transform::identity(), None);
                }
                // the chat box points at the top of what is actually drawn, standing or seated
                head = Point::new(x + b.x + b.w / 2.0, y + b.y + b.h * 0.04);
            }
            draw_sprite(&mut self.pixmap, f, x, y);
            if let (Some(m), Some(mouth)) = (pose.frame.mouth, skin.strip(ClipId::Mouth)) {
                let mf = &mouth.frames[(m as usize).min(mouth.frames.len() - 1)];
                draw_sprite(&mut self.pixmap, mf, x, y);
            }
        }
        self.paint_bubble(bubble, head, now);
    }

    fn paint_bubble(&mut self, bubble: &Bubble, head: Point, now: f32) {
        let k = self.layout.size / 240.0;
        let px = (17.0 * k).max(12.0);
        let pad = 12.0 * k;
        let l = self.lang;
        let (label, body, body_color) = match &bubble.state {
            BubbleState::Hidden => return,
            BubbleState::Typing { draft } => {
                let caret = if (now * 2.0) as i32 % 2 == 0 { "_" } else { " " };
                (l.pick("YOU", "TOI"), format!("> {draft}{caret}"), rgb((0xe6, 0xf7, 0xff), 255))
            }
            BubbleState::Listening => {
                let waves = [")", "))", ")))", "))"];
                let wave = waves[(now * 6.0) as usize % waves.len()];
                (l.pick("YOU", "TOI"), format!("{} {wave}", l.pick("listening", "j'écoute")), rgb(NEON_CYAN, 255))
            }
            BubbleState::Thinking => {
                let dots = ".".repeat(1 + (now * 3.0) as usize % 3);
                ("CAPRI", format!("{}{dots}", l.pick("jacking in", "connexion")), rgb(NEON_CYAN, 220))
            }
            BubbleState::Saying { text, .. } => ("CAPRI", text.clone(), rgb((0xe6, 0xf7, 0xff), 255)),
            BubbleState::Error(e) => (l.pick("BRAIN OFFLINE", "CERVEAU HORS LIGNE"), e.clone(), rgb(NEON_RED, 255)),
        };

        let max_w = (self.layout.width as f32 - 16.0).min(360.0 * k);
        let mut lines = self.font.wrap(&body, px, max_w - 2.0 * pad);
        let lh = self.font.line_height(px);
        let label_h = lh * 0.8;
        let max_lines = (((head.y - 24.0 * k) - 2.0 * pad - label_h) / lh).floor().max(1.0) as usize;
        if lines.len() > max_lines {
            // keep the newest words in view while a long reply streams in
            lines.drain(..lines.len() - max_lines);
            lines[0] = format!("…{}", lines[0]);
        }
        let text_w = lines.iter().map(|l| self.font.width(l, px)).fold(self.font.width(label, px * 0.8), f32::max);
        let w = (text_w + 2.0 * pad).min(max_w).max(120.0 * k);
        let h = label_h + lines.len() as f32 * lh + 2.0 * pad;
        let x = (head.x - w / 2.0).clamp(8.0, self.layout.width as f32 - w - 8.0);
        let y = (head.y - 14.0 * k - h).max(4.0);
        self.bubble_rect = Some(Rect { x, y, w, h });

        let chamfer = 10.0 * k;
        if let Some(path) =
            chamfered(x, y, w, h, chamfer, head.x.clamp(x + chamfer * 2.0, x + w - chamfer * 2.0), 10.0 * k)
        {
            let mut fill = Paint::default();
            fill.set_color(rgb(NIGHT, 238));
            fill.anti_alias = true;
            self.pixmap.fill_path(&path, &fill, FillRule::Winding, Transform::identity(), None);
            let mut edge = Paint::default();
            edge.set_color(rgb(if matches!(bubble.state, BubbleState::Error(_)) { NEON_RED } else { NEON_CYAN }, 255));
            edge.anti_alias = true;
            let stroke = Stroke { width: 1.5 * k.max(1.0), ..Stroke::default() };
            self.pixmap.stroke_path(&path, &edge, &stroke, Transform::identity(), None);
        }
        // red tab on the top-left edge, the Edgerunners HUD look
        if let Some(tab) = tiny_skia::Rect::from_xywh(x + chamfer, y - 2.0 * k, 36.0 * k, 3.0 * k) {
            let mut red = Paint::default();
            red.set_color(rgb(NEON_RED, 255));
            self.pixmap.fill_rect(tab, &red, Transform::identity(), None);
        }

        let ascent = lh * 0.78;
        self.font.draw(&mut self.pixmap, label, px * 0.8, x + pad, y + pad + ascent * 0.8, rgb(NEON_RED, 255));
        for (i, line) in lines.iter().enumerate() {
            let baseline = y + pad + label_h + ascent + i as f32 * lh;
            self.font.draw(&mut self.pixmap, line, px, x + pad, baseline, body_color);
        }
    }
}

/// A box with cut corners and a small tail at the bottom pointing to `tail_x`.
fn chamfered(x: f32, y: f32, w: f32, h: f32, c: f32, tail_x: f32, tail: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(x + c, y);
    pb.line_to(x + w, y);
    pb.line_to(x + w, y + h - c);
    pb.line_to(x + w - c, y + h);
    pb.line_to(tail_x + tail * 0.6, y + h);
    pb.line_to(tail_x, y + h + tail);
    pb.line_to(tail_x - tail * 0.6, y + h);
    pb.line_to(x, y + h);
    pb.line_to(x, y + c);
    pb.close();
    pb.finish()
}

/// The box around a frame's clearly opaque pixels, in frame pixels.
fn opaque_bounds(sprite: &capri_application::ports::Sprite) -> Option<Rect> {
    let (w, h) = (sprite.width, sprite.height);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if sprite.alpha(x, y) > 24 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (x0 <= x1).then(|| Rect { x: x0 as f32, y: y0 as f32, w: (x1 - x0 + 1) as f32, h: (y1 - y0 + 1) as f32 })
}

fn draw_sprite(pm: &mut Pixmap, sprite: &capri_application::ports::Sprite, x: f32, y: f32) {
    if let Some(src) = PixmapRef::from_bytes(&sprite.pixels, sprite.width, sprite.height) {
        pm.draw_pixmap(x.round() as i32, y.round() as i32, src, &PixmapPaint::default(), Transform::identity(), None);
    }
}
