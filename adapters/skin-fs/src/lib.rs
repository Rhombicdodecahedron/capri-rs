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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use capri_application::ports::{Skin, SkinError, SkinInfo, SkinRepository, Sprite, Strip};
use capri_domain::ClipId;
use image::RgbaImage;
use image::imageops::FilterType;
use serde::Deserialize;

/// Where she sits on a seated frame (her seat on the window edge), as fractions of the frame,
/// matching the original sheets.
pub const SEAT_ANCHOR: (f32, f32) = (0.30, 0.71);

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
        for clip in ClipId::ALL {
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
    fn premultiplies() {
        assert_eq!(premultiply(vec![255, 255, 255, 128]), vec![128, 128, 128, 128]);
    }
}
