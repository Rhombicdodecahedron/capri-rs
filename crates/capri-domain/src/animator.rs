//! Picks which frame to draw: clip timing, random blinks during idle loops, mouth flaps while talking.
//! Pure timing logic: the renderer only receives a [`Frame`].

use crate::clip::{ClipId, Playback};
use crate::rng::Rng;

/// What the renderer draws this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub clip: ClipId,
    pub index: u32,
    /// Mouth overlay frame, drawn on top of a standing face while she talks.
    pub mouth: Option<u32>,
}

/// Blinks replace frames 6..10 of an idle or perch loop, like the original sheets expect.
const BLINK_WINDOW: std::ops::Range<u32> = 6..10;
const BLINK_CHANCE: f32 = 0.45;
const GLITCH_BLINK_CHANCE: f32 = 0.08;
const GRIN_CHANCE: f32 = 0.05;

#[derive(Debug, Clone)]
pub struct Animator {
    clip: ClipId,
    started: f32,
    cycle: i64,
    blink: Option<ClipId>,
    finished: bool,
}

impl Animator {
    pub fn new(clip: ClipId, now: f32) -> Self {
        Self { clip, started: now, cycle: -1, blink: None, finished: false }
    }

    pub fn clip(&self) -> ClipId {
        self.clip
    }

    /// Starts `clip` from its first frame. Playing the clip that is already running is a no-op.
    pub fn play(&mut self, clip: ClipId, now: f32) {
        if clip != self.clip || self.finished {
            *self = Self::new(clip, now);
        }
    }

    /// True once a `Playback::Times` clip has played out. The caller picks what comes next.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Advances to `now` and returns the frame to draw.
    /// `allow_grin`: whether an idle loop may break into a grin (not while the chat box is open).
    pub fn frame(&mut self, now: f32, talking: bool, allow_grin: bool, rng: &mut Rng) -> Frame {
        let spec = self.clip.spec();
        let elapsed = (now - self.started).max(0.0);
        let raw = (elapsed * spec.fps) as u32;

        if let Playback::Times(n) = spec.playback {
            if raw >= spec.frames * n {
                self.finished = true;
                return Frame { clip: self.clip, index: spec.frames - 1, mouth: None };
            }
        }

        let index = raw % spec.frames;
        let loops_with_blinks = matches!(self.clip, ClipId::Idle | ClipId::Perch);
        if loops_with_blinks {
            let cycle = i64::from(raw / spec.frames);
            if cycle != self.cycle {
                self.cycle = cycle;
                let r = rng.unit();
                self.blink = (r < BLINK_CHANCE).then(|| blink_for(self.clip, r < GLITCH_BLINK_CHANCE));
                if self.clip == ClipId::Idle && allow_grin && !talking && rng.unit() < GRIN_CHANCE {
                    self.play(ClipId::Grin, now);
                    return self.frame(now, talking, false, rng);
                }
            }
        }

        let mut frame = Frame { clip: self.clip, index, mouth: None };
        if let Some(blink) = self.blink {
            if loops_with_blinks && BLINK_WINDOW.contains(&index) {
                frame = Frame { clip: blink, index: index - BLINK_WINDOW.start, mouth: None };
            }
        }
        if talking && matches!(frame.clip, ClipId::Idle | ClipId::Blink | ClipId::BlinkGlitch) {
            frame.mouth = Some(mouth_frame(now));
        }
        frame
    }
}

fn blink_for(clip: ClipId, glitch: bool) -> ClipId {
    match (clip, glitch) {
        (ClipId::Perch, false) => ClipId::PerchBlink,
        (ClipId::Perch, true) => ClipId::PerchBlinkGlitch,
        (_, false) => ClipId::Blink,
        (_, true) => ClipId::BlinkGlitch,
    }
}

/// Mouth flaps at 12 fps over five frames, closing briefly every fourth beat so speech reads as words.
fn mouth_frame(now: f32) -> u32 {
    let beat = (now * 12.0) as u32;
    if beat % 4 == 0 { 0 } else { beat % 5 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_clips_report_finished() {
        let mut rng = Rng::seeded(1);
        let mut a = Animator::new(ClipId::Glitch, 0.0);
        assert!(!a.finished());
        a.frame(0.5, false, false, &mut rng);
        assert!(!a.finished());
        a.frame(ClipId::Glitch.spec().duration() + 0.01, false, false, &mut rng);
        assert!(a.finished());
    }

    #[test]
    fn grin_plays_twice() {
        let mut rng = Rng::seeded(1);
        let mut a = Animator::new(ClipId::Grin, 0.0);
        a.frame(ClipId::Grin.spec().duration() * 1.5, false, false, &mut rng);
        assert!(!a.finished());
        a.frame(ClipId::Grin.spec().duration() * 2.0 + 0.01, false, false, &mut rng);
        assert!(a.finished());
    }

    #[test]
    fn blinks_only_inside_window() {
        let mut rng = Rng::seeded(7);
        let mut a = Animator::new(ClipId::Idle, 0.0);
        let fps = ClipId::Idle.spec().fps;
        for step in 0..2000u32 {
            let t = step as f32 / fps;
            let f = a.frame(t, false, false, &mut rng);
            if matches!(f.clip, ClipId::Blink | ClipId::BlinkGlitch) {
                assert!(f.index < 4);
            }
        }
    }

    #[test]
    fn mouth_only_while_talking_on_standing_face() {
        let mut rng = Rng::seeded(3);
        let mut a = Animator::new(ClipId::Idle, 0.0);
        assert!(a.frame(0.2, false, false, &mut rng).mouth.is_none());
        let talking = a.frame(0.3, true, false, &mut rng);
        if talking.clip == ClipId::Idle {
            assert!(talking.mouth.is_some());
        }
        let mut seated = Animator::new(ClipId::Perch, 0.0);
        assert!(seated.frame(0.3, true, false, &mut rng).mouth.is_none());
    }

    #[test]
    fn replaying_same_clip_keeps_timing() {
        let mut a = Animator::new(ClipId::Idle, 0.0);
        a.play(ClipId::Idle, 5.0);
        let mut rng = Rng::seeded(1);
        // still counted from 0.0: at t=0.5s we are on frame 4 (outside the blink window), not frame 0
        assert_eq!(a.frame(0.5, false, false, &mut rng).index, 4);
    }
}
