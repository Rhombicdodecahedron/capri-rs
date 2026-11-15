//! Capri's body on the desktop as a state machine: falling, standing, perching on windows, being held,
//! teleporting to another ledge, dozing off. It reacts to events and to `tick`, and never touches the OS:
//! ledges come in as plain data, frames go out as plain data.

use crate::animator::{Animator, Frame};
use crate::clip::ClipId;
use crate::geometry::Point;
use crate::ledge::{self, Ledge, LedgeKind};
use crate::rng::Rng;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterConfig {
    /// On-screen size in screen pixels at 100% scaling (240 matches the original default).
    pub size: f32,
    /// Pixels per second squared.
    pub gravity: f32,
    /// Seconds between two hops to another ledge, picked at random in this range.
    pub roam_every: (f32, f32),
    /// Whether she hops around by herself.
    pub roaming: bool,
    /// Idle seconds on the floor before she falls asleep.
    pub sleep_after: f32,
}

impl Default for CharacterConfig {
    fn default() -> Self {
        Self { size: 240.0, gravity: 2600.0, roam_every: (18.0, 40.0), roaming: true, sleep_after: 300.0 }
    }
}

impl CharacterConfig {
    /// A ledge narrower than this is ignored.
    pub fn min_seat_width(&self) -> f32 {
        self.size * 0.6
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Hidden,
    Falling,
    Held,
    Standing,
    Perching,
    Sleeping,
    Waking,
    /// Glitching out before reappearing on `Character::target`.
    Teleporting,
}

/// Things the application may want to react to (sound effects, logs, the chat box following her).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Happening {
    Landed(LedgeKind),
    SeatLost,
    FellAsleep,
    Woke,
    Teleported,
}

/// What the renderer needs this tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub mode: Mode,
    /// Her feet when standing, her seat when perching, in screen pixels.
    pub anchor: Point,
    pub frame: Frame,
    pub visible: bool,
}

#[derive(Debug, Clone)]
pub struct Character {
    cfg: CharacterConfig,
    mode: Mode,
    pos: Point,
    vel: Point,
    seat: Option<Ledge>,
    /// Horizontal offset from the seat's left edge, so she rides along when a window moves.
    seat_offset: f32,
    target: Option<(Ledge, f32)>,
    animator: Animator,
    talking: bool,
    chat_open: bool,
    last_activity: f32,
    next_roam: f32,
    rng: Rng,
}

impl Character {
    pub fn new(cfg: CharacterConfig, seed: u64) -> Self {
        Self {
            cfg,
            mode: Mode::Hidden,
            pos: Point::default(),
            vel: Point::default(),
            seat: None,
            seat_offset: 0.0,
            target: None,
            animator: Animator::new(ClipId::Idle, 0.0),
            talking: false,
            chat_open: false,
            last_activity: 0.0,
            next_roam: f32::INFINITY,
            rng: Rng::seeded(seed),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn config(&self) -> &CharacterConfig {
        &self.cfg
    }

    pub fn set_size(&mut self, size: f32) {
        self.cfg.size = size.max(48.0);
    }

    pub fn set_roaming(&mut self, roaming: bool) {
        self.cfg.roaming = roaming;
    }

    /// Applies new settings live. A new size takes effect at once; a new roaming range from the next hop.
    pub fn set_config(&mut self, cfg: CharacterConfig) {
        self.cfg = CharacterConfig { size: cfg.size.max(48.0), ..cfg };
    }

    pub fn position(&self) -> Point {
        self.pos
    }

    pub fn seat(&self) -> Option<&Ledge> {
        self.seat.as_ref()
    }

    /// Top of her head, where the chat bubble attaches.
    pub fn head(&self) -> Point {
        let h = if self.mode == Mode::Perching { 0.75 } else { 1.05 };
        Point::new(self.pos.x, self.pos.y - self.cfg.size * h)
    }

    /// She drops in from `at` (usually the top of the screen) and lands on whatever catches her.
    pub fn appear(&mut self, at: Point, now: f32) {
        self.pos = at;
        self.vel = Point::default();
        self.seat = None;
        self.enter(Mode::Falling, now);
    }

    pub fn hide(&mut self, now: f32) {
        self.seat = None;
        self.enter(Mode::Hidden, now);
    }

    pub fn grab(&mut self, now: f32) {
        if self.mode == Mode::Hidden {
            return;
        }
        self.seat = None;
        self.touch(now);
        self.enter(Mode::Held, now);
    }

    pub fn drag_to(&mut self, p: Point) {
        if self.mode == Mode::Held {
            self.pos = p;
        }
    }

    /// Let go with the pointer's velocity, so she can be tossed.
    pub fn release(&mut self, velocity: Point, now: f32) {
        if self.mode == Mode::Held {
            self.vel = Point::new(velocity.x.clamp(-3000.0, 3000.0), velocity.y.clamp(-3000.0, 3000.0));
            self.enter(Mode::Falling, now);
        }
    }

    /// A click on her: wakes her up, or makes her glitch for a moment.
    pub fn poke(&mut self, now: f32) {
        self.touch(now);
        match self.mode {
            Mode::Sleeping => self.enter(Mode::Waking, now),
            Mode::Standing => self.animator.play(ClipId::Glitch, now),
            Mode::Perching => self.animator.play(ClipId::PerchGlitch, now),
            _ => {}
        }
    }

    pub fn set_talking(&mut self, talking: bool, now: f32) {
        self.talking = talking;
        if talking {
            self.touch(now);
            if self.mode == Mode::Sleeping {
                self.enter(Mode::Waking, now);
            }
        }
    }

    /// While the chat box is open she does not wander off or break into grins.
    pub fn set_chat_open(&mut self, open: bool, now: f32) {
        self.chat_open = open;
        if open {
            self.touch(now);
        }
    }

    /// Any user activity near her resets the idle timer.
    pub fn touch(&mut self, now: f32) {
        self.last_activity = now;
        if self.mode == Mode::Sleeping {
            self.enter(Mode::Waking, now);
        }
    }

    /// Hop to a ledge right now (the brain can ask her to move).
    pub fn teleport_to(&mut self, target: Ledge, now: f32) {
        if matches!(self.mode, Mode::Standing | Mode::Perching | Mode::Sleeping) {
            let x = self.pick_spot(&target);
            self.target = Some((target, x));
            self.enter(Mode::Teleporting, now);
        }
    }

    /// Advances the simulation. `ledges` is the current list (window edges move, close and open).
    pub fn tick(&mut self, now: f32, dt: f32, ledges: &[Ledge]) -> (Pose, Vec<Happening>) {
        let mut happenings = Vec::new();
        let dt = dt.clamp(0.0, 0.1);
        match self.mode {
            Mode::Hidden | Mode::Held => {}
            Mode::Falling => self.fall(now, dt, ledges, &mut happenings),
            Mode::Standing | Mode::Perching | Mode::Sleeping | Mode::Waking => {
                self.follow_seat(now, ledges, &mut happenings);
                self.maybe_doze_or_roam(now, ledges, &mut happenings);
            }
            Mode::Teleporting => {
                if self.animator.finished() {
                    if let Some((ledge, x)) = self.target.take() {
                        self.pos = Point::new(x, ledge.y);
                        self.sit_on(ledge, now);
                        happenings.push(Happening::Teleported);
                    } else {
                        self.enter(Mode::Falling, now);
                    }
                }
            }
        }

        if self.mode == Mode::Waking && self.animator.finished() {
            self.enter(Mode::Standing, now);
            happenings.push(Happening::Woke);
        }
        self.settle_clip(now);

        let frame = self.animator.frame(now, self.talking, !self.chat_open, &mut self.rng);
        let pose = Pose { mode: self.mode, anchor: self.pos, frame, visible: self.mode != Mode::Hidden };
        (pose, happenings)
    }

    fn fall(&mut self, now: f32, dt: f32, ledges: &[Ledge], out: &mut Vec<Happening>) {
        let from = self.pos;
        self.vel.y += self.cfg.gravity * dt;
        self.vel.x *= 0.995_f32.powf(dt * 60.0);
        let to = Point::new(from.x + self.vel.x * dt, from.y + self.vel.y * dt);
        let usable: Vec<Ledge> = ledges.iter().copied().filter(|l| l.fits(self.cfg.min_seat_width())).collect();
        // only ledges she falls onto from above catch her (not ones she is thrown up through)
        let caught = if self.vel.y > 0.0 { ledge::landing(&usable, to.x, from.y, to.y).copied() } else { None };
        if let Some(l) = caught {
            self.pos = Point::new(to.x.clamp(l.left, l.right), l.y);
            self.sit_on(l, now);
            out.push(Happening::Landed(l.kind));
            return;
        }
        self.pos = to;
        // fell off every screen: put her back on the lowest floor
        let lowest_floor = ledges.iter().filter(|l| l.kind == LedgeKind::Floor).max_by(|a, b| a.y.total_cmp(&b.y));
        if let Some(floor) = lowest_floor {
            if self.pos.y > floor.y + self.cfg.size * 4.0 {
                let x = self.pos.x.clamp(floor.left, floor.right);
                self.pos = Point::new(x, floor.y);
                self.sit_on(*floor, now);
                out.push(Happening::Landed(LedgeKind::Floor));
            }
        }
    }

    fn follow_seat(&mut self, now: f32, ledges: &[Ledge], out: &mut Vec<Happening>) {
        let Some(seat) = self.seat else {
            self.enter(Mode::Falling, now);
            return;
        };
        match ledges.iter().find(|l| l.id == seat.id) {
            Some(current) if current.fits(self.cfg.min_seat_width()) => {
                let x = (current.left + self.seat_offset).clamp(current.left, current.right);
                self.pos = Point::new(x, current.y);
                self.seat = Some(*current);
            }
            _ => {
                // the window closed, shrank or went fullscreen under her
                self.seat = None;
                self.vel = Point::default();
                self.enter(Mode::Falling, now);
                out.push(Happening::SeatLost);
            }
        }
    }

    fn maybe_doze_or_roam(&mut self, now: f32, ledges: &[Ledge], out: &mut Vec<Happening>) {
        let busy = self.talking || self.chat_open;
        if self.mode == Mode::Standing && !busy && now - self.last_activity > self.cfg.sleep_after {
            self.enter(Mode::Sleeping, now);
            out.push(Happening::FellAsleep);
            return;
        }
        if !self.cfg.roaming || busy || !matches!(self.mode, Mode::Standing | Mode::Perching) {
            return;
        }
        if now < self.next_roam {
            return;
        }
        self.schedule_roam(now);
        let here = self.seat.map(|s| s.id);
        let options: Vec<Ledge> =
            ledges.iter().copied().filter(|l| l.fits(self.cfg.min_seat_width()) && Some(l.id) != here).collect();
        if !options.is_empty() {
            let pick = options[self.rng.index(options.len())];
            self.teleport_to(pick, now);
        }
    }

    fn sit_on(&mut self, l: Ledge, now: f32) {
        self.seat = Some(l);
        self.seat_offset = self.pos.x - l.left;
        self.vel = Point::default();
        self.schedule_roam(now);
        let mode = if l.is_seat() { Mode::Perching } else { Mode::Standing };
        self.enter(mode, now);
    }

    fn pick_spot(&mut self, l: &Ledge) -> f32 {
        let margin = (self.cfg.size * 0.3).min(l.width() / 2.0);
        self.rng.range(l.left + margin, l.right - margin)
    }

    fn schedule_roam(&mut self, now: f32) {
        let (lo, hi) = self.cfg.roam_every;
        self.next_roam = now + self.rng.range(lo, hi.max(lo));
    }

    fn enter(&mut self, mode: Mode, now: f32) {
        self.mode = mode;
        let clip = match mode {
            Mode::Hidden | Mode::Standing => ClipId::Idle,
            Mode::Falling | Mode::Held => ClipId::Dangle,
            Mode::Perching => ClipId::Perch,
            Mode::Sleeping => ClipId::Sleep,
            Mode::Waking => ClipId::Wake,
            Mode::Teleporting => {
                if self.seat.is_some_and(|s| s.is_seat()) {
                    ClipId::PerchGlitch
                } else {
                    ClipId::Glitch
                }
            }
        };
        self.animator.play(clip, now);
    }

    /// One-shot clips (grin, poke glitches) hand back to the resting loop when done.
    fn settle_clip(&mut self, now: f32) {
        if !self.animator.finished() {
            return;
        }
        match self.mode {
            Mode::Standing => self.animator.play(ClipId::Idle, now),
            Mode::Perching => self.animator.play(ClipId::Perch, now),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledge::LedgeId;

    fn floor() -> Ledge {
        Ledge { id: LedgeId::Floor(0), kind: LedgeKind::Floor, left: 0.0, right: 1920.0, y: 1040.0 }
    }

    fn window(id: u64, y: f32) -> Ledge {
        Ledge { id: LedgeId::Window(id), kind: LedgeKind::Window, left: 400.0, right: 1200.0, y }
    }

    fn run(c: &mut Character, from: f32, seconds: f32, ledges: &[Ledge]) -> (f32, Vec<Happening>) {
        let mut t = from;
        let mut all = Vec::new();
        while t < from + seconds {
            t += 1.0 / 60.0;
            all.extend(c.tick(t, 1.0 / 60.0, ledges).1);
        }
        (t, all)
    }

    fn calm() -> CharacterConfig {
        CharacterConfig { roaming: false, ..CharacterConfig::default() }
    }

    #[test]
    fn falls_and_perches_on_a_window() {
        let mut c = Character::new(calm(), 1);
        let ledges = [floor(), window(7, 500.0)];
        c.appear(Point::new(800.0, 0.0), 0.0);
        let (_, events) = run(&mut c, 0.0, 2.0, &ledges);
        assert_eq!(c.mode(), Mode::Perching);
        assert_eq!(c.seat().map(|s| s.id), Some(LedgeId::Window(7)));
        assert!(events.contains(&Happening::Landed(LedgeKind::Window)));
        assert!((c.position().y - 500.0).abs() < f32::EPSILON);
    }

    #[test]
    fn misses_window_and_stands_on_floor() {
        let mut c = Character::new(calm(), 1);
        c.appear(Point::new(100.0, 0.0), 0.0);
        run(&mut c, 0.0, 3.0, &[floor(), window(7, 500.0)]);
        assert_eq!(c.mode(), Mode::Standing);
    }

    #[test]
    fn rides_along_when_window_moves_and_falls_when_it_closes() {
        let mut c = Character::new(calm(), 1);
        c.appear(Point::new(800.0, 0.0), 0.0);
        let (t, _) = run(&mut c, 0.0, 2.0, &[floor(), window(7, 500.0)]);

        let mut moved = window(7, 300.0);
        moved.left += 100.0;
        moved.right += 100.0;
        let (t, _) = run(&mut c, t, 0.1, &[floor(), moved]);
        assert!((c.position().y - 300.0).abs() < f32::EPSILON);
        assert!((c.position().x - 900.0).abs() < 0.5);

        let (_, events) = run(&mut c, t, 3.0, &[floor()]);
        assert!(events.contains(&Happening::SeatLost));
        assert_eq!(c.mode(), Mode::Standing);
    }

    #[test]
    fn dozes_off_on_the_floor_and_wakes_when_poked() {
        let cfg = CharacterConfig { sleep_after: 5.0, ..calm() };
        let mut c = Character::new(cfg, 1);
        c.appear(Point::new(100.0, 900.0), 0.0);
        let (t, events) = run(&mut c, 0.0, 8.0, &[floor()]);
        assert!(events.contains(&Happening::FellAsleep));
        c.poke(t);
        assert_eq!(c.mode(), Mode::Waking);
        let (_, events) = run(&mut c, t, 2.0, &[floor()]);
        assert!(events.contains(&Happening::Woke));
        assert_eq!(c.mode(), Mode::Standing);
    }

    #[test]
    fn does_not_doze_while_talking() {
        let cfg = CharacterConfig { sleep_after: 1.0, ..calm() };
        let mut c = Character::new(cfg, 1);
        c.appear(Point::new(100.0, 900.0), 0.0);
        c.set_talking(true, 0.0);
        run(&mut c, 0.0, 5.0, &[floor()]);
        assert_eq!(c.mode(), Mode::Standing);
    }

    #[test]
    fn roams_to_another_ledge() {
        let cfg = CharacterConfig { roam_every: (1.0, 1.0), ..CharacterConfig::default() };
        let mut c = Character::new(cfg, 9);
        c.appear(Point::new(100.0, 900.0), 0.0);
        let (_, events) = run(&mut c, 0.0, 4.0, &[floor(), window(3, 400.0)]);
        assert!(events.contains(&Happening::Teleported));
    }

    #[test]
    fn can_be_held_and_tossed() {
        let mut c = Character::new(calm(), 1);
        c.appear(Point::new(100.0, 900.0), 0.0);
        let (t, _) = run(&mut c, 0.0, 1.0, &[floor()]);
        c.grab(t);
        c.drag_to(Point::new(500.0, 100.0));
        assert_eq!(c.mode(), Mode::Held);
        c.release(Point::new(200.0, 0.0), t);
        let (_, events) = run(&mut c, t, 3.0, &[floor()]);
        assert!(events.contains(&Happening::Landed(LedgeKind::Floor)));
        assert!(c.position().x > 500.0);
    }

    #[test]
    fn falling_offscreen_recovers_to_floor() {
        let mut c = Character::new(calm(), 1);
        c.appear(Point::new(5000.0, 0.0), 0.0);
        run(&mut c, 0.0, 5.0, &[floor()]);
        assert_eq!(c.mode(), Mode::Standing);
        assert!(c.position().x <= 1920.0);
    }
}
