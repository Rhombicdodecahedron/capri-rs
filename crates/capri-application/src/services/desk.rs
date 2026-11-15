//! Use case: keep her alive on the desktop. Polls the desktop for ledges at a modest rate and steps the
//! domain character every frame. Synchronous on purpose: the UI thread calls `tick` each frame.

use std::sync::Arc;

use capri_domain::{Character, Happening, Ledge, Point, Pose};

use crate::ports::DesktopProbe;

pub struct DeskService {
    character: Character,
    probe: Arc<dyn DesktopProbe>,
    ledges: Vec<Ledge>,
    ledges_at: f32,
    refresh_every: f32,
    own_windows: Vec<u64>,
}

impl std::fmt::Debug for DeskService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeskService")
            .field("mode", &self.character.mode())
            .field("ledges", &self.ledges.len())
            .finish_non_exhaustive()
    }
}

impl DeskService {
    pub fn new(character: Character, probe: Arc<dyn DesktopProbe>) -> Self {
        Self {
            character,
            probe,
            ledges: Vec::new(),
            ledges_at: f32::NEG_INFINITY,
            refresh_every: 0.25,
            own_windows: Vec::new(),
        }
    }

    /// Her own windows must never count as ledges, or she would sit on herself.
    pub fn set_own_windows(&mut self, ids: Vec<u64>) {
        self.own_windows = ids;
    }

    pub fn character(&self) -> &Character {
        &self.character
    }

    pub fn character_mut(&mut self) -> &mut Character {
        &mut self.character
    }

    pub fn ledges(&self) -> &[Ledge] {
        &self.ledges
    }

    /// Drops her in from the top of the main screen, right of centre, like the original intro.
    pub fn start(&mut self, now: f32) {
        self.refresh(now, true);
        let area = self.probe.work_areas().first().copied().unwrap_or_default();
        let x = area.x + area.w * 0.7;
        self.character.appear(Point::new(x, area.y - self.character.config().size), now);
    }

    pub fn tick(&mut self, now: f32, dt: f32) -> (Pose, Vec<Happening>) {
        self.refresh(now, false);
        self.character.tick(now, dt, &self.ledges)
    }

    fn refresh(&mut self, now: f32, force: bool) {
        if force || now - self.ledges_at >= self.refresh_every {
            self.ledges = self.probe.ledges(&self.own_windows);
            self.ledges_at = now;
        }
    }
}
