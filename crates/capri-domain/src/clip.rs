//! The animation clips a skin provides. The table matches the sprite-sheet format of the original
//! capri-ai-desktop project, so a skin folder made for it works here unchanged: each clip is one PNG,
//! a horizontal strip of equally wide frames.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipId {
    Idle,
    Blink,
    BlinkGlitch,
    Grin,
    Glitch,
    Dangle,
    Sleep,
    Wake,
    Mouth,
    Perch,
    PerchBlink,
    PerchBlinkGlitch,
    PerchGlitch,
}

/// How a clip plays once it reaches its last frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Playback {
    /// Loops forever.
    Loop,
    /// Plays `n` times, then the animator reports it finished.
    Times(u32),
    /// Drawn on top of another clip (blinks, mouth flaps), never played on its own.
    Overlay,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipSpec {
    pub id: ClipId,
    pub frames: u32,
    pub fps: f32,
    /// Drawn height as a multiple of the character's configured size.
    pub height_ratio: f32,
    pub playback: Playback,
}

impl ClipSpec {
    pub fn duration(&self) -> f32 {
        self.frames as f32 / self.fps
    }
}

impl ClipId {
    pub const ALL: [ClipId; 13] = [
        ClipId::Idle,
        ClipId::Blink,
        ClipId::BlinkGlitch,
        ClipId::Grin,
        ClipId::Glitch,
        ClipId::Dangle,
        ClipId::Sleep,
        ClipId::Wake,
        ClipId::Mouth,
        ClipId::Perch,
        ClipId::PerchBlink,
        ClipId::PerchBlinkGlitch,
        ClipId::PerchGlitch,
    ];

    /// File stem of the sprite strip, e.g. `perch_blink` for `perch_blink.png`.
    pub fn file_stem(self) -> &'static str {
        match self {
            ClipId::Idle => "idle",
            ClipId::Blink => "blink",
            ClipId::BlinkGlitch => "blink_glitch",
            ClipId::Grin => "grin",
            ClipId::Glitch => "glitch",
            ClipId::Dangle => "dangle",
            ClipId::Sleep => "sleep",
            ClipId::Wake => "wake",
            ClipId::Mouth => "mouth",
            ClipId::Perch => "perch",
            ClipId::PerchBlink => "perch_blink",
            ClipId::PerchBlinkGlitch => "perch_blink_glitch",
            ClipId::PerchGlitch => "perch_glitch",
        }
    }

    /// Seated clips share the perch frame size and the seat anchor instead of the feet.
    pub fn is_seated(self) -> bool {
        matches!(self, ClipId::Perch | ClipId::PerchBlink | ClipId::PerchBlinkGlitch | ClipId::PerchGlitch)
    }

    pub fn spec(self) -> ClipSpec {
        let (frames, fps, height_ratio, playback) = match self {
            ClipId::Idle => (16, 8.0, 1.11, Playback::Loop),
            ClipId::Blink | ClipId::BlinkGlitch => (4, 8.0, 1.11, Playback::Overlay),
            ClipId::Grin => (16, 8.0, 1.11, Playback::Times(2)),
            ClipId::Glitch => (10, 15.0, 1.11, Playback::Times(1)),
            ClipId::Dangle => (24, 10.0, 1.11, Playback::Loop),
            ClipId::Sleep => (16, 6.0, 1.11, Playback::Loop),
            ClipId::Wake => (13, 15.0, 1.11, Playback::Times(1)),
            ClipId::Mouth => (5, 12.0, 1.11, Playback::Overlay),
            ClipId::Perch => (16, 8.0, 1.0, Playback::Loop),
            ClipId::PerchBlink | ClipId::PerchBlinkGlitch => (4, 8.0, 1.0, Playback::Overlay),
            ClipId::PerchGlitch => (10, 15.0, 1.0, Playback::Times(1)),
        };
        ClipSpec { id: self, frames, fps, height_ratio, playback }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_clip_has_a_unique_file() {
        let mut stems: Vec<_> = ClipId::ALL.iter().map(|c| c.file_stem()).collect();
        stems.sort_unstable();
        stems.dedup();
        assert_eq!(stems.len(), ClipId::ALL.len());
    }

    #[test]
    fn matches_original_sheet_layout() {
        assert_eq!(ClipId::Idle.spec().frames, 16);
        assert_eq!(ClipId::Perch.spec().frames, 16);
        assert_eq!(ClipId::Dangle.spec().frames, 24);
        assert_eq!(ClipId::Wake.spec().frames, 13);
    }
}
