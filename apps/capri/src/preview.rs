//! `capri preview`: draws a few poses side by side with the real renderer, so new art can be checked
//! (anchors, sizes, mouth placement, frame counts) in a second, without launching the desk.

use std::path::Path;

use anyhow::Context;
use capri_application::ports::SkinRepository;
use capri_desk_winit::bubble::Bubble;
use capri_desk_winit::scene::{Layout, Painter};
use capri_domain::{ClipId, Frame, Mode, Point, Pose};
use tiny_skia::{Color, Paint, Pixmap, PixmapPaint, Rect, Transform};

pub fn render(skins: &dyn SkinRepository, id: &str, size: f32, out: &Path) -> anyhow::Result<()> {
    let skin = skins.load(id, size, 1.0).with_context(|| format!("loading skin `{id}`"))?;
    let layout = Layout::for_size(size);
    let all: [(ClipId, u32, Option<u32>, Mode, &str); 12] = [
        (ClipId::Idle, 0, None, Mode::Standing, ""),
        (ClipId::Idle, 3, Some(3), Mode::Standing, "Wake up, choom. Your build passed."),
        (ClipId::Blink, 1, None, Mode::Standing, ""),
        (ClipId::Perch, 4, None, Mode::Perching, "Je suis bien, là-haut."),
        (ClipId::Dangle, 6, None, Mode::Held, ""),
        (ClipId::Sleep, 2, None, Mode::Sleeping, ""),
        (ClipId::LayAwake, 0, None, Mode::Lying, ""),
        (ClipId::LayTalk, 0, None, Mode::Lying, "Five more minutes."),
        (ClipId::Lay, 0, None, Mode::Lying, ""),
        (ClipId::MiniIdle, 0, None, Mode::Mini, ""),
        (ClipId::MiniAnnoyedTalk, 0, None, Mode::Mini, "Arrête."),
        (ClipId::MiniBlink, 0, None, Mode::Mini, ""),
    ];
    // rest poses only when the skin draws them
    let shots: Vec<_> = all
        .into_iter()
        .filter(|(clip, ..)| clip.kind() == capri_domain::ClipKind::Strip || skin.strip(*clip).is_some())
        .collect();
    let mut sheet = Pixmap::new(layout.width * shots.len() as u32, layout.height).context("sheet size")?;
    sheet.fill(Color::from_rgba8(36, 38, 52, 255));
    for (i, (clip, index, mouth, mode, line)) in shots.into_iter().enumerate() {
        let mut painter = Painter::new(layout);
        let anchor = layout.anchor;
        // sleeping poses show their z's
        let asleep = matches!(clip, ClipId::Sleep | ClipId::Lay | ClipId::MiniBlink);
        let pose = Pose { mode, anchor, frame: Frame { clip, index, mouth }, visible: true, asleep };
        let mut bubble = Bubble::default();
        if !line.is_empty() {
            bubble.finish(line.to_string(), 0.0);
        }
        let h = match mode {
            Mode::Perching => 0.75,
            Mode::Lying => 0.5,
            Mode::Mini => 0.36,
            _ => 1.05,
        };
        let head = Point::new(anchor.x, anchor.y - size * h);
        painter.paint(&pose, head, &skin, &bubble, 0.9);
        let x = (i as u32 * layout.width) as i32;
        // a "window edge" under seated poses and a floor under standing ones
        let mut edge = Paint::default();
        edge.set_color(Color::from_rgba8(90, 96, 120, 255));
        if let Some(r) = Rect::from_xywh(x as f32 + 10.0, anchor.y, layout.width as f32 - 20.0, 3.0) {
            sheet.fill_rect(r, &edge, Transform::identity(), None);
        }
        sheet.draw_pixmap(x, 0, painter.pixmap().as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }
    sheet.save_png(out).context("writing png")?;
    Ok(())
}
