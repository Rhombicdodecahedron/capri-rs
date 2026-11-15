//! Renders animation frames with the real renderer, for the README media (`tools/readme_media.py`).
//! Run from the repo root: `cargo run --release --example readme_frames -- <out-dir>`.

use std::path::PathBuf;

use capri_application::ports::SkinRepository;
use capri_desk_winit::bubble::Bubble;
use capri_desk_winit::scene::{Layout, Painter};
use capri_domain::{Animator, ClipId, Language, Mode, Point, Pose, Rng};
use capri_skin_fs::FsSkins;

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "target/readme-frames".into()));
    std::fs::create_dir_all(&out).expect("out dir");
    let skins = FsSkins::new(vec![PathBuf::from("assets/skins")]);
    let size = 200.0;
    let skin = skins.load("capri", size, 1.0).expect("capri skin in assets/skins/capri");
    let layout = Layout::for_size(size);

    // (name, clip, mode, talking line, frames, fps)
    let shots: [(&str, ClipId, Mode, &str, u32, f32); 3] = [
        ("perch", ClipId::Perch, Mode::Perching, "", 48, 12.0),
        ("talk", ClipId::Idle, Mode::Standing, "Salut choom. Ton build est passé, je m'occupe du reste.", 48, 12.0),
        ("idle", ClipId::Idle, Mode::Standing, "", 48, 12.0),
    ];
    for (name, clip, mode, line, frames, fps) in shots {
        let mut animator = Animator::new(clip, 0.0);
        let mut rng = Rng::seeded(11);
        let mut bubble = Bubble::default();
        if !line.is_empty() {
            bubble.finish(line.to_string(), 0.0);
        }
        let mut painter = Painter::new(layout);
        painter.lang = Language::Fr;
        for i in 0..frames {
            let t = i as f32 / fps;
            let talking = !line.is_empty();
            let frame = animator.frame(t, talking, false, &mut rng);
            let pose = Pose { mode, anchor: layout.anchor, frame, visible: true, asleep: false };
            let head_up = if mode == Mode::Perching { 0.75 } else { 1.05 };
            let head = Point::new(layout.anchor.x, layout.anchor.y - size * head_up);
            painter.paint(&pose, head, &skin, &bubble, t);
            painter.pixmap().save_png(out.join(format!("{name}-{i:03}.png"))).expect("png");
        }
    }
    println!("anchor {} {} canvas {}x{}", layout.anchor.x, layout.anchor.y, layout.width, layout.height);
}
