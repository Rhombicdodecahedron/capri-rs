//! Loads skins from folders on disk.
//!
//! Layout (identical to the original capri-ai-desktop `assets/desk/skins/<id>/`, so a folder made for
//! it can be copied in unchanged):
//!
//! ```text
//! assets/skins/<id>/
//!   skin.toml        optional: name = "...", credits = "..."
//!   idle.png         required; every other clip is optional and falls back to a close one
//!   blink.png  blink_glitch.png  grin.png  glitch.png  dangle.png  sleep.png  wake.png  mouth.png
//!   perch.png  perch_blink.png  perch_blink_glitch.png  perch_glitch.png
//! ```
//!
//! Each PNG is a horizontal strip of equally wide frames; the frame count per clip is fixed by
//! [`capri_domain::ClipId::spec`]. Any height works: frames are scaled to the character's size.
//!
//! Her rest poses are single pictures, also optional:
//!
//! ```text
//!   lay_awake.png  lay.png  lay_talk.png  lay_talk2.png      lying down (eyes open, shut, talking)
//!   mini/idle.png  mini/blink.png  mini/talk.png  mini/talk2.png  mini/annoyed.png
//!   mini/annoyed_talk.png  mini/look_left.png  mini/look_right.png  mini/glitch.png
//! ```
//!
//! Each family is cropped to the area its pictures draw on together, so switching looks never moves
//! her, then scaled: lying, her body is about as long as she is tall; mini, a small screen.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use capri_application::ports::{Skin, SkinError, SkinInfo, SkinRepository, Sprite, Strip};
use capri_domain::{ClipId, ClipKind};
use image::RgbaImage;
use image::imageops::FilterType;
use serde::Deserialize;

/// Where she sits on a seated frame (her seat on the window edge), as fractions of the frame,
/// matching the original sheets.
pub const SEAT_ANCHOR: (f32, f32) = (0.30, 0.71);

/// Lying down, her body is this many times her standing size long (the original's 1.15).
pub const LAY_LENGTH: f32 = 1.15;
/// Her mini screen is this many times her size wide (96 px chip at size 240, art at 84%, minus margins).
pub const MINI_WIDTH: f32 = 0.31;

#[derive(Debug, Clone)]
pub struct FsSkins {
    roots: Vec<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct Manifest {
    name: Option<String>,
    credits: Option<String>,
}

impl FsSkins {
    /// Later roots win, so a user folder can override a bundled skin of the same id.
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    fn folder(&self, id: &str) -> Option<PathBuf> {
        self.roots.iter().rev().map(|r| r.join(id)).find(|p| p.join("idle.png").is_file())
    }

    fn info(id: &str, folder: &Path) -> SkinInfo {
        let manifest: Manifest = std::fs::read_to_string(folder.join("skin.toml"))
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        SkinInfo {
            id: id.to_string(),
            name: manifest.name.unwrap_or_else(|| id.to_string()),
            credits: manifest.credits,
        }
    }
}

impl SkinRepository for FsSkins {
    fn list(&self) -> Vec<SkinInfo> {
        let mut seen: HashMap<String, SkinInfo> = HashMap::new();
        for root in &self.roots {
            let Ok(entries) = std::fs::read_dir(root) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                if p.join("idle.png").is_file() {
                    let id = e.file_name().to_string_lossy().to_string();
                    seen.insert(id.clone(), Self::info(&id, &p));
                }
            }
        }
        let mut v: Vec<_> = seen.into_values().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    fn load(&self, id: &str, size: f32, scale: f32) -> Result<Skin, SkinError> {
        let folder = self.folder(id).ok_or_else(|| SkinError::NotFound(id.to_string()))?;
        let mut strips = HashMap::new();
        for kind in [ClipKind::LayPose, ClipKind::MiniFace] {
            let family: Vec<ClipId> = ClipId::ALL.into_iter().filter(|c| c.kind() == kind).collect();
            let mut images = Vec::new();
            for clip in family {
                let file = folder.join(format!("{}.png", clip.file_stem()));
                if file.is_file() {
                    let img = image::open(&file).map_err(|e| invalid(id, &file, &e.to_string()))?.into_rgba8();
                    images.push((clip, img));
                }
            }
            strips.extend(poses(kind, &images, size, scale).into_iter().map(|s| (s.clip, s)));
        }
        for clip in ClipId::ALL.into_iter().filter(|c| c.kind() == ClipKind::Strip) {
            let file = folder.join(format!("{}.png", clip.file_stem()));
            if !file.is_file() {
                if clip == ClipId::Idle {
                    return Err(invalid(id, &file, "missing (a skin needs at least idle.png)"));
                }
                continue;
            }
            let img = image::open(&file).map_err(|e| invalid(id, &file, &e.to_string()))?.into_rgba8();
            let strip = cut(clip, &img, size, scale).map_err(|reason| invalid(id, &file, &reason))?;
            strips.insert(clip, strip);
        }
        tracing::info!(skin = id, clips = strips.len(), folder = %folder.display(), "skin loaded");
        Ok(Skin { info: Self::info(id, &folder), strips })
    }
}

fn invalid(skin: &str, file: &Path, reason: &str) -> SkinError {
    SkinError::Invalid {
        skin: skin.to_string(),
        file: file.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
        reason: reason.to_string(),
    }
}

/// Cuts a strip into frames, scales them to `size * height_ratio` logical px (times the DPI `scale`),
/// and premultiplies alpha for the renderer.
pub fn cut(clip: ClipId, img: &RgbaImage, size: f32, scale: f32) -> Result<Strip, String> {
    let spec = clip.spec();
    if img.width() % spec.frames != 0 {
        return Err(format!("width {} is not a multiple of {} frames", img.width(), spec.frames));
    }
    let fw = img.width() / spec.frames;
    let fh = img.height();
    let target_h = (size * spec.height_ratio * scale).round().max(1.0) as u32;
    let target_w = ((target_h as f32) * fw as f32 / fh as f32).round().max(1.0) as u32;

    let first = image::imageops::crop_imm(img, 0, 0, fw, fh).to_image();
    let anchor = if clip.is_seated() { SEAT_ANCHOR } else { feet(&first) };

    let frames = (0..spec.frames)
        .map(|i| {
            let frame = image::imageops::crop_imm(img, i * fw, 0, fw, fh).to_image();
            let scaled = image::imageops::resize(&frame, target_w, target_h, FilterType::Lanczos3);
            Sprite { width: target_w, height: target_h, pixels: premultiply(scaled.into_raw()) }
        })
        .collect();
    Ok(Strip { clip, frames, anchor })
}

/// The box (x, y, w, h) holding every clearly drawn pixel of `images`, which share one canvas size.
fn drawn_box(images: &[&RgbaImage]) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for img in images {
        let (w, h) = img.dimensions();
        let step = (w.min(h) / 400).max(1);
        for y in (0..h).step_by(step as usize) {
            for x in (0..w).step_by(step as usize) {
                if img.get_pixel(x, y)[3] > 40 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
    }
    let (w, h) = images.first()?.dimensions();
    (x0 <= x1).then(|| {
        let x1 = (x1 + 2).min(w - 1);
        let y1 = (y1 + 2).min(h - 1);
        (x0.saturating_sub(2), y0.saturating_sub(2), x1 - x0.saturating_sub(2) + 1, y1 - y0.saturating_sub(2) + 1)
    })
}

/// Where her lying body rests: the median of where the drawn columns end, so a stray hair spike or a
/// dangling hand doesn't lift her off the taskbar. As a fraction of the height.
pub fn rest_line(img: &RgbaImage) -> f32 {
    let (w, h) = img.dimensions();
    let step = (w.min(h) / 200).max(1);
    let mut ends: Vec<u32> =
        (0..w).step_by(step as usize).filter_map(|x| (0..h).rev().find(|&y| img.get_pixel(x, y)[3] > 40)).collect();
    if ends.is_empty() {
        return 1.0;
    }
    ends.sort_unstable();
    ((ends[ends.len() / 2] + 1) as f32 / h as f32).min(1.0)
}

/// Single-picture looks of one family (lying down, or mini), cropped together and scaled.
pub fn poses(kind: ClipKind, images: &[(ClipId, RgbaImage)], size: f32, scale: f32) -> Vec<Strip> {
    let same_canvas: Vec<&RgbaImage> = match images.first() {
        Some((_, first)) => images.iter().map(|(_, i)| i).filter(|i| i.dimensions() == first.dimensions()).collect(),
        None => return Vec::new(),
    };
    let Some((bx, by, bw, bh)) = drawn_box(&same_canvas) else { return Vec::new() };
    let width = match kind {
        ClipKind::LayPose => LAY_LENGTH,
        ClipKind::MiniFace => MINI_WIDTH,
        ClipKind::Strip => return Vec::new(),
    } * size
        * scale;
    let target_w = width.round().max(1.0) as u32;
    let target_h = ((target_w as f32) * bh as f32 / bw as f32).round().max(1.0) as u32;
    let cropped = |img: &RgbaImage| image::imageops::crop_imm(img, bx, by, bw, bh).to_image();
    // lying, she rests on her body's underside (judged on the eyes-open look); mini sits on its bottom edge
    let anchor = match kind {
        ClipKind::LayPose => {
            let reference = images.iter().find(|(c, _)| *c == ClipId::LayAwake).unwrap_or(&images[0]);
            (0.5, rest_line(&cropped(&reference.1)))
        }
        _ => (0.5, 1.0),
    };
    images
        .iter()
        .filter(|(_, img)| img.dimensions() == same_canvas[0].dimensions())
        .map(|(clip, img)| {
            let scaled = image::imageops::resize(&cropped(img), target_w, target_h, FilterType::Lanczos3);
            Strip {
                clip: *clip,
                frames: vec![Sprite { width: target_w, height: target_h, pixels: premultiply(scaled.into_raw()) }],
                anchor,
            }
        })
        .collect()
}

/// Where her feet touch the ground: the centre of the lowest opaque rows, as frame fractions.
pub fn feet(frame: &RgbaImage) -> (f32, f32) {
    let (w, h) = frame.dimensions();
    let opaque = |x: u32, y: u32| frame.get_pixel(x, y)[3] > 128;
    let Some(bottom) = (0..h).rev().find(|&y| (0..w).any(|x| opaque(x, y))) else {
        return (0.5, 1.0);
    };
    // average over a few rows above the lowest one, so one stray pixel does not skew it
    let band = (h / 40).max(2);
    let (mut sum, mut n) = (0u64, 0u64);
    for y in bottom.saturating_sub(band)..=bottom {
        for x in 0..w {
            if opaque(x, y) {
                sum += u64::from(x);
                n += 1;
            }
        }
    }
    let cx = if n == 0 { w as f32 / 2.0 } else { sum as f32 / n as f32 };
    (cx / w as f32, (bottom + 1) as f32 / h as f32)
}

fn premultiply(mut px: Vec<u8>) -> Vec<u8> {
    for p in px.chunks_exact_mut(4) {
        let a = u16::from(p[3]);
        for c in &mut p[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn strip(frames: u32, fw: u32, fh: u32) -> RgbaImage {
        let mut img = RgbaImage::new(frames * fw, fh);
        for f in 0..frames {
            // a 10px-wide "body" standing on row fh-5 of each frame
            for y in 10..fh - 4 {
                for x in 0..10 {
                    img.put_pixel(f * fw + fw / 2 - 5 + x, y, Rgba([255, 0, 60, 255]));
                }
            }
        }
        img
    }

    #[test]
    fn cuts_and_scales_to_size() {
        let s = cut(ClipId::Idle, &strip(16, 240, 384), 120.0, 1.0).unwrap();
        assert_eq!(s.frames.len(), 16);
        assert_eq!(s.frames[0].height, (120.0_f32 * 1.11).round() as u32);
        assert_eq!(s.frames[0].width, 83);
    }

    #[test]
    fn rejects_wrong_frame_count() {
        assert!(cut(ClipId::Idle, &RgbaImage::new(1000, 384), 120.0, 1.0).is_err());
    }

    #[test]
    fn finds_feet() {
        let img = strip(1, 240, 384);
        let (fx, fy) = feet(&img);
        assert!((fx - 0.5).abs() < 0.02, "fx {fx}");
        assert!((fy - 380.0 / 384.0).abs() < 0.01, "fy {fy}");
    }

    #[test]
    fn seated_clips_use_seat_anchor() {
        let s = cut(ClipId::Perch, &strip(16, 384, 384), 120.0, 1.0).unwrap();
        assert_eq!(s.anchor, SEAT_ANCHOR);
    }

    #[test]
    fn rest_poses_share_one_crop_and_rest_on_the_body() {
        // two looks on a big canvas, drawn in the same spot; one has a hair spike lower down
        let mut awake = RgbaImage::new(400, 200);
        for y in 100..150 {
            for x in 50..350 {
                awake.put_pixel(x, y, Rgba([255, 0, 60, 255]));
            }
        }
        let mut shut = awake.clone();
        for y in 150..170 {
            shut.put_pixel(200, y, Rgba([255, 0, 60, 255]));
        }
        let s = poses(ClipKind::LayPose, &[(ClipId::LayAwake, awake), (ClipId::Lay, shut)], 100.0, 1.0);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].frames[0].width, 115, "as long as she is tall, times 1.15");
        assert_eq!(s[0].frames[0].height, s[1].frames[0].height, "one crop for the family");
        let (_, ay) = s[0].anchor;
        assert!(ay < 0.8, "rests on the body, not the spike: {ay}");
        let real = SkinPaths::bundled_capri();
        if let Some(dir) = real {
            let skin = FsSkins::new(vec![dir]).load("capri", 240.0, 1.0).unwrap();
            assert!(skin.can_lie_down() && skin.has_mini());
            let mini = skin.strip(ClipId::MiniTalk).unwrap();
            assert_eq!(mini.frames[0].width, (240.0_f32 * MINI_WIDTH).round() as u32);
        }
    }

    /// The bundled skins folder, when the tests run inside the repository.
    struct SkinPaths;
    impl SkinPaths {
        fn bundled_capri() -> Option<PathBuf> {
            let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/skins");
            p.join("capri/idle.png").is_file().then_some(p)
        }
    }

    #[test]
    fn premultiplies() {
        assert_eq!(premultiply(vec![255, 255, 255, 128]), vec![128, 128, 128, 128]);
    }
}
