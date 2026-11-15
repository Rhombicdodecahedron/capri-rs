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
    /// Lying down, eyes shut (dozing, or a blink).
    Lay,
    /// Lying down, eyes open.
    LayAwake,
    /// Lying down and talking, mouth open (two frames to flap between).
    LayTalk,
    LayTalk2,
    /// Her mini form, a little screen with her eyes: looking at you.
    MiniIdle,
    MiniBlink,
    MiniTalk,
    MiniTalk2,
    MiniAnnoyed,
    MiniAnnoyedTalk,
    MiniLookLeft,
    MiniLookRight,
    MiniGlitch,
}

/// How a clip's art is laid out and sized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    /// A horizontal strip of frames, `height_ratio` times her size tall.
    Strip,
    /// One large picture of her lying down, scaled so her body is about as long as she is tall.
    LayPose,
    /// One picture of her mini face, a small screen sitting on the taskbar or a window.
    MiniFace,
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
    pub const ALL: [ClipId; 26] = [
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
        ClipId::Lay,
        ClipId::LayAwake,
        ClipId::LayTalk,
        ClipId::LayTalk2,
        ClipId::MiniIdle,
        ClipId::MiniBlink,
        ClipId::MiniTalk,
        ClipId::MiniTalk2,
        ClipId::MiniAnnoyed,
        ClipId::MiniAnnoyedTalk,
        ClipId::MiniLookLeft,
        ClipId::MiniLookRight,
        ClipId::MiniGlitch,
    ];

    /// File stem of the art, e.g. `perch_blink` for `perch_blink.png`; mini looks live in `mini/`.
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
            ClipId::Lay => "lay",
            ClipId::LayAwake => "lay_awake",
            ClipId::LayTalk => "lay_talk",
            ClipId::LayTalk2 => "lay_talk2",
            ClipId::MiniIdle => "mini/idle",
            ClipId::MiniBlink => "mini/blink",
            ClipId::MiniTalk => "mini/talk",
            ClipId::MiniTalk2 => "mini/talk2",
            ClipId::MiniAnnoyed => "mini/annoyed",
            ClipId::MiniAnnoyedTalk => "mini/annoyed_talk",
            ClipId::MiniLookLeft => "mini/look_left",
            ClipId::MiniLookRight => "mini/look_right",
            ClipId::MiniGlitch => "mini/glitch",
        }
    }

    pub fn kind(self) -> ClipKind {
        match self {
            ClipId::Lay | ClipId::LayAwake | ClipId::LayTalk | ClipId::LayTalk2 => ClipKind::LayPose,
            ClipId::MiniIdle
            | ClipId::MiniBlink
            | ClipId::MiniTalk
            | ClipId::MiniTalk2
            | ClipId::MiniAnnoyed
            | ClipId::MiniAnnoyedTalk
            | ClipId::MiniLookLeft
            | ClipId::MiniLookRight
            | ClipId::MiniGlitch => ClipKind::MiniFace,
            _ => ClipKind::Strip,
        }
    }

    /// The look to borrow when a skin doesn't draw this one. `None`: nothing close enough (a skin
    /// without `lay_awake` simply doesn't lie down, one without `mini/idle` has no mini form).
    pub fn fallback(self) -> Option<ClipId> {
        match self {
            ClipId::Idle | ClipId::Mouth | ClipId::LayAwake | ClipId::MiniIdle => None,
            ClipId::PerchBlink | ClipId::PerchBlinkGlitch | ClipId::PerchGlitch => Some(ClipId::Perch),
            ClipId::BlinkGlitch => Some(ClipId::Blink),
            ClipId::Lay | ClipId::LayTalk => Some(ClipId::LayAwake),
            ClipId::LayTalk2 => Some(ClipId::LayTalk),
            ClipId::MiniTalk2 | ClipId::MiniAnnoyedTalk => Some(ClipId::MiniTalk),
            ClipId::MiniBlink
            | ClipId::MiniTalk
            | ClipId::MiniAnnoyed
            | ClipId::MiniLookLeft
            | ClipId::MiniLookRight
            | ClipId::MiniGlitch => Some(ClipId::MiniIdle),
            _ => Some(ClipId::Idle),
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
            // single pictures, sized by their kind (see ClipKind)
            _ => (1, 1.0, 1.0, Playback::Loop),
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
    fn rest_looks_fall_back_within_their_family() {
        assert_eq!(ClipId::LayTalk2.fallback(), Some(ClipId::LayTalk));
        assert_eq!(ClipId::LayAwake.fallback(), None, "no lying art: she just doesn't lie down");
        assert_eq!(ClipId::MiniAnnoyedTalk.fallback(), Some(ClipId::MiniTalk));
        assert_eq!(ClipId::MiniLookLeft.fallback(), Some(ClipId::MiniIdle));
        assert_eq!(ClipId::MiniIdle.fallback(), None);
        assert_eq!(ClipId::Lay.kind(), ClipKind::LayPose);
        assert_eq!(ClipId::MiniGlitch.file_stem(), "mini/glitch");
    }

    #[test]
    fn matches_original_sheet_layout() {
        assert_eq!(ClipId::Idle.spec().frames, 16);
        assert_eq!(ClipId::Perch.spec().frames, 16);
        assert_eq!(ClipId::Dangle.spec().frames, 24);
        assert_eq!(ClipId::Wake.spec().frames, 13);
    }
}
