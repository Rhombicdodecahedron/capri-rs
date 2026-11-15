//! Capri's body on the desktop as a state machine: falling, standing, perching on windows, being held,
//! teleporting to another ledge, dozing off. It reacts to events and to `tick`, and never touches the OS:
//! ledges come in as plain data, frames go out as plain data.

use crate::animator::{Animator, Frame};
use crate::body::BodyCommand;
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
    /// Whether she lies down and naps (mini, on the taskbar) by herself now and then.
    pub rests: bool,
}

impl Default for CharacterConfig {
    fn default() -> Self {
        Self { size: 240.0, gravity: 2600.0, roam_every: (18.0, 40.0), roaming: true, sleep_after: 300.0, rests: true }
    }
}

impl CharacterConfig {
    /// A ledge narrower than this is ignored.
    pub fn min_seat_width(&self) -> f32 {
        self.size * 0.6
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    /// Lying on the taskbar or on top of a window: eyes drift shut after a while, then she gets up.
    Lying,
    /// Her mini form, a little screen with her eyes, sitting where she was: napping, or small on purpose.
    Mini,
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
    /// Eyes shut and sleeping: the renderer lets z's drift up from her head.
    pub asleep: bool,
}

/// Which rest poses the skin draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Abilities {
    pub lie_down: bool,
    pub mini: bool,
}

/// Timers of her rest poses (lying down, mini).
#[derive(Debug, Clone, Default)]
struct Rest {
    /// Lying: when she gets up. Mini nap: when she wakes.
    until: f32,
    /// Lying: when her eyes drift shut.
    doze_at: f32,
    blink_until: f32,
    next_blink: f32,
    /// Mini: eyes shut, z's.
    napping: bool,
    /// Mini on purpose: she stays small after a nap, until asked back.
    stay: bool,
    annoyed_until: f32,
    glitch_until: f32,
    next_glitch: f32,
    /// Mini: when she may doze off again if nobody comes by.
    next_nap_check: f32,
    /// Recent single clicks on her mini form (three quick ones annoy her).
    clicks: Vec<f32>,
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
    abilities: Abilities,
    rest: Rest,
    /// The mouse pointer on screen: her mini eyes follow it.
    pointer: Option<Point>,
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
            abilities: Abilities::default(),
            rest: Rest::default(),
            pointer: None,
        }
    }

    /// What the loaded skin can draw: no lying art, no lying down; no mini art, no mini form.
    pub fn set_abilities(&mut self, abilities: Abilities, now: f32) {
        self.abilities = abilities;
        if self.mode == Mode::Lying && !abilities.lie_down || self.mode == Mode::Mini && !abilities.mini {
            self.get_up(now);
        }
    }

    pub fn set_pointer(&mut self, p: Point) {
        self.pointer = Some(p);
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
        let h = match self.mode {
            Mode::Perching => 0.75,
            Mode::Lying => 0.5,
            Mode::Mini => 0.36,
            _ => 1.05,
        };
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
        if self.mode == Mode::Perching {
            // the seated anchor is her seat, the hanging one her feet: keep her head where it was
            self.pos.y += self.cfg.size * 0.38;
        }
        self.seat = None;
        self.rest.napping = false;
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
            Mode::Lying | Mode::Mini => self.get_up(now),
            _ => {}
        }
    }

    /// A single click on her. Three quick ones on her mini form and she gets fed up for a moment.
    /// Returns true when that just happened.
    pub fn clicked(&mut self, now: f32) -> bool {
        self.touch(now);
        if self.mode != Mode::Mini || self.rest.napping {
            return false;
        }
        self.rest.clicks.retain(|t| now - t < 2.5);
        self.rest.clicks.push(now);
        if self.rest.clicks.len() >= 3 {
            self.rest.clicks.clear();
            self.rest.annoyed_until = now + 3.0;
            return true;
        }
        false
    }

    /// Asked in words (typed or said). Returns false when she can't do it right now.
    pub fn command(&mut self, cmd: BodyCommand, now: f32) -> bool {
        // activity, but not `touch`: a nap must not wake her straight back up
        self.last_activity = now;
        match cmd {
            BodyCommand::Nap => self.nap(now),
            BodyCommand::Mini => self.go_mini(now),
            BodyCommand::FullSize => {
                if self.mode == Mode::Mini {
                    self.get_up(now);
                    true
                } else {
                    false
                }
            }
            BodyCommand::LieDown => self.lie_down(now, (40.0, 80.0)),
            BodyCommand::WakeUp => match self.mode {
                Mode::Sleeping => {
                    self.enter(Mode::Waking, now);
                    true
                }
                Mode::Lying => {
                    self.get_up(now);
                    true
                }
                Mode::Mini if self.rest.napping => {
                    self.wake_from_nap(now);
                    true
                }
                _ => false,
            },
        }
    }

    /// Shrinks into her mini form and dozes off; she wakes up full size after a minute or two.
    /// Already mini, she dozes off right there and stays mini. Without mini art: a nap standing up.
    pub fn nap(&mut self, now: f32) -> bool {
        if !self.abilities.mini {
            if self.mode == Mode::Standing {
                self.enter(Mode::Sleeping, now);
                return true;
            }
            return self.lie_down(now, (60.0, 150.0));
        }
        if !matches!(self.mode, Mode::Standing | Mode::Perching | Mode::Lying | Mode::Sleeping | Mode::Mini) {
            return false;
        }
        if self.mode != Mode::Mini {
            self.rest.stay = false;
            self.become_mini(now);
        }
        self.rest.napping = true;
        self.rest.until = now + self.rng.range(50.0, 110.0);
        true
    }

    /// Small and awake on purpose: she stays mini until asked back (or double-clicked).
    pub fn go_mini(&mut self, now: f32) -> bool {
        if !self.abilities.mini {
            return false;
        }
        if !matches!(self.mode, Mode::Standing | Mode::Perching | Mode::Lying | Mode::Sleeping | Mode::Mini) {
            return false;
        }
        if self.mode != Mode::Mini {
            self.become_mini(now);
        }
        self.rest.stay = true;
        self.rest.napping = false;
        self.rest.next_nap_check = now + 240.0;
        true
    }

    /// Lies down where she is, on the taskbar or on top of a window, for `seconds` (a range).
    pub fn lie_down(&mut self, now: f32, seconds: (f32, f32)) -> bool {
        if !self.abilities.lie_down || self.seat.is_none() {
            return false;
        }
        if !matches!(self.mode, Mode::Standing | Mode::Perching | Mode::Sleeping | Mode::Lying) {
            return false;
        }
        self.enter(Mode::Lying, now);
        self.rest.until = now + self.rng.range(seconds.0, seconds.1);
        self.rest.doze_at = now + self.rng.range(10.0, 30.0);
        self.rest.blink_until = 0.0;
        self.rest.next_blink = now + self.rng.range(2.5, 6.0);
        true
    }

    fn become_mini(&mut self, now: f32) {
        self.enter(Mode::Mini, now);
        self.rest.glitch_until = now + 0.25;
        self.rest.next_glitch = now + self.rng.range(6.0, 20.0);
        self.rest.next_blink = now + self.rng.range(2.0, 5.0);
        self.rest.annoyed_until = 0.0;
        self.rest.clicks.clear();
    }

    fn wake_from_nap(&mut self, now: f32) {
        self.rest.napping = false;
        if self.rest.stay {
            self.rest.glitch_until = now + 0.15;
            self.rest.next_nap_check = now + 240.0;
        } else {
            self.get_up(now);
        }
    }

    /// Back on her feet (or her seat) from lying down or her mini form, with a little glitch.
    fn get_up(&mut self, now: f32) {
        self.rest.napping = false;
        let Some(seat) = self.seat else {
            self.enter(Mode::Falling, now);
            return;
        };
        if seat.is_seat() {
            self.enter(Mode::Perching, now);
            self.animator.play(ClipId::PerchGlitch, now);
        } else {
            self.enter(Mode::Standing, now);
            self.animator.play(ClipId::Glitch, now);
        }
        self.schedule_roam(now);
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
            Mode::Lying => {
                self.follow_seat(now, ledges, &mut happenings);
                if self.mode == Mode::Lying {
                    self.lying_step(now);
                }
            }
            Mode::Mini => {
                self.follow_seat(now, ledges, &mut happenings);
                if self.mode == Mode::Mini {
                    self.mini_step(now, &mut happenings);
                }
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

        let mut frame = self.animator.frame(now, self.talking, !self.chat_open, &mut self.rng);
        let look = match self.mode {
            Mode::Lying => Some(self.lying_look(now)),
            Mode::Mini => Some(self.mini_look(now)),
            _ => None,
        };
        if let Some(clip) = look {
            frame = Frame { clip, index: 0, mouth: None };
        }
        let asleep = match self.mode {
            Mode::Sleeping => true,
            Mode::Lying => !self.talking && now > self.rest.doze_at,
            Mode::Mini => !self.talking && self.rest.napping,
            _ => false,
        };
        let pose = Pose { mode: self.mode, anchor: self.pos, frame, visible: self.mode != Mode::Hidden, asleep };
        (pose, happenings)
    }

    fn blink_now_and_then(&mut self, now: f32, length: f32) {
        if now > self.rest.next_blink {
            self.rest.blink_until = now + length;
            // now and then two quick blinks
            let gap = if self.rng.unit() < 0.15 { 0.3 } else { self.rng.range(2.5, 6.0) };
            self.rest.next_blink = now + gap;
        }
    }

    fn lying_step(&mut self, now: f32) {
        if self.talking || self.chat_open {
            // talking to her keeps her eyes open a while longer
            self.rest.doze_at = self.rest.doze_at.max(now + 10.0);
        } else if now > self.rest.until {
            self.get_up(now);
            return;
        }
        self.blink_now_and_then(now, 0.15);
    }

    fn lying_look(&self, now: f32) -> ClipId {
        if self.talking {
            return match (now * 8.0) as u32 % 3 {
                0 => ClipId::LayTalk,
                1 => ClipId::LayTalk2,
                _ => ClipId::LayAwake,
            };
        }
        if now > self.rest.doze_at || now < self.rest.blink_until { ClipId::Lay } else { ClipId::LayAwake }
    }

    fn mini_step(&mut self, now: f32, out: &mut Vec<Happening>) {
        let busy = self.talking || self.chat_open;
        self.blink_now_and_then(now, 0.14);
        if now > self.rest.next_glitch {
            self.rest.glitch_until = now + 0.12;
            self.rest.next_glitch = now + self.rng.range(6.0, 20.0);
        }
        if self.rest.napping {
            if !busy && now > self.rest.until {
                let stay = self.rest.stay;
                self.wake_from_nap(now);
                if !stay {
                    out.push(Happening::Woke);
                }
            }
            return;
        }
        // small on purpose and nobody around for a long while: she dozes off now and then
        if self.rest.stay && self.cfg.rests && !busy && now > self.rest.next_nap_check {
            self.rest.next_nap_check = now + 30.0;
            if now - self.last_activity > 240.0 && self.rng.unit() < 0.25 {
                self.rest.napping = true;
                self.rest.until = now + self.rng.range(50.0, 110.0);
                out.push(Happening::FellAsleep);
            }
        }
    }

    fn mini_look(&self, now: f32) -> ClipId {
        if now < self.rest.glitch_until {
            return ClipId::MiniGlitch;
        }
        // mouth flaps open, open (second frame), shut
        let step = (now * 7.0) as u32 % 3;
        let open = self.talking && step != 2;
        let two = step == 1;
        if self.rest.napping {
            // eyes stay shut; she mumbles back in her sleep
            return if open { ClipId::MiniTalk } else { ClipId::MiniBlink };
        }
        let blinking = now < self.rest.blink_until;
        if now < self.rest.annoyed_until {
            return if open {
                ClipId::MiniAnnoyedTalk
            } else if blinking {
                ClipId::MiniBlink
            } else {
                ClipId::MiniAnnoyed
            };
        }
        if open {
            return if two { ClipId::MiniTalk2 } else { ClipId::MiniTalk };
        }
        if blinking {
            return ClipId::MiniBlink;
        }
        // her eyes follow the pointer when it is off to one side
        if let Some(p) = self.pointer {
            let dx = p.x - self.pos.x;
            if dx.abs() >= self.cfg.size * 0.4 * 1.2 {
                return if dx < 0.0 { ClipId::MiniLookLeft } else { ClipId::MiniLookRight };
            }
        }
        ClipId::MiniIdle
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
        if self.cfg.rests {
            // like the original: now and then a nap in her mini form, often a lie-down where she is
            // (more often when nobody has talked to her in a while)
            let bored = now - self.last_activity > 120.0;
            let r = self.rng.unit();
            if r < 0.12 && self.abilities.mini {
                self.nap(now);
                out.push(Happening::FellAsleep);
                return;
            }
            let lie_chance = if bored { 0.62 } else { 0.37 };
            if r < lie_chance && self.lie_down(now, if bored { (60.0, 150.0) } else { (25.0, 60.0) }) {
                return;
            }
        }
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
            Mode::Lying => ClipId::LayAwake,
            Mode::Mini => ClipId::MiniIdle,
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

    fn rested(seed: u64) -> (Character, Vec<Ledge>, f32) {
        let mut c = Character::new(calm(), seed);
        c.set_abilities(Abilities { lie_down: true, mini: true }, 0.0);
        c.appear(Point::new(100.0, 0.0), 0.0);
        let ledges = vec![floor()];
        let (t, _) = run(&mut c, 0.0, 3.0, &ledges);
        assert_eq!(c.mode(), Mode::Standing);
        (c, ledges, t)
    }

    fn pose(c: &mut Character, t: f32, ledges: &[Ledge]) -> Pose {
        c.tick(t, 1.0 / 60.0, ledges).0
    }

    #[test]
    fn lies_down_dozes_off_talks_and_gets_up() {
        let (mut c, ledges, t) = rested(3);
        assert!(c.command(BodyCommand::LieDown, t));
        let p = pose(&mut c, t + 0.1, &ledges);
        assert_eq!((p.mode, p.asleep), (Mode::Lying, false));
        assert!(matches!(p.frame.clip, ClipId::LayAwake | ClipId::Lay));
        // eyes drift shut after 10-30 s
        let (t, _) = run(&mut c, t, 31.0, &ledges);
        let p = pose(&mut c, t, &ledges);
        assert!(p.asleep && p.frame.clip == ClipId::Lay, "{p:?}");
        // talking to her: mouth moves, eyes open
        c.set_talking(true, t);
        let looks: Vec<ClipId> = (0..6).map(|i| pose(&mut c, t + i as f32 * 0.13, &ledges).frame.clip).collect();
        assert!(looks.contains(&ClipId::LayTalk) && looks.contains(&ClipId::LayTalk2), "{looks:?}");
        assert!(!pose(&mut c, t + 1.0, &ledges).asleep);
        c.set_talking(false, t + 1.0);
        // she never gets up mid-line, but does once it's over and her time is up
        run(&mut c, t + 1.0, 90.0, &ledges);
        assert_eq!(c.mode(), Mode::Standing);
    }

    #[test]
    fn a_nap_is_mini_with_eyes_shut_then_she_wakes_full_size() {
        let (mut c, ledges, t) = rested(4);
        assert!(c.command(BodyCommand::Nap, t));
        assert_eq!(c.mode(), Mode::Mini);
        let p = pose(&mut c, t + 0.5, &ledges);
        assert!(p.asleep);
        assert_eq!(p.frame.clip, ClipId::MiniBlink);
        assert!((p.anchor.y - 1040.0).abs() < f32::EPSILON, "on the taskbar");
        c.set_talking(true, t + 1.0);
        assert!(!pose(&mut c, t + 1.0, &ledges).asleep, "mumbling back");
        c.set_talking(false, t + 2.0);
        let (_, events) = run(&mut c, t + 2.0, 115.0, &ledges);
        assert_eq!(c.mode(), Mode::Standing);
        assert!(events.contains(&Happening::Woke));
    }

    #[test]
    fn mini_on_purpose_stays_follows_the_pointer_and_gets_annoyed() {
        let (mut c, ledges, t) = rested(5);
        assert!(c.command(BodyCommand::Mini, t));
        let (t, _) = run(&mut c, t, 1.0, &ledges);
        c.set_pointer(Point::new(c.position().x + 900.0, 500.0));
        let looks: Vec<ClipId> = (0..40).map(|i| pose(&mut c, t + i as f32 * 0.05, &ledges).frame.clip).collect();
        assert!(looks.contains(&ClipId::MiniLookRight), "{looks:?}");
        let shut = looks.iter().filter(|l| **l == ClipId::MiniBlink).count();
        assert!(shut < looks.len() / 2, "awake, eyes open most of the time: {looks:?}");
        assert!(!c.clicked(t + 2.0) && !c.clicked(t + 2.6));
        assert!(c.clicked(t + 3.2), "third click in 2.5 s");
        assert_eq!(pose(&mut c, t + 3.3, &ledges).frame.clip, ClipId::MiniAnnoyed);
        run(&mut c, t + 3.3, 300.0, &ledges);
        assert_eq!(c.mode(), Mode::Mini, "small until asked back");
        assert!(c.command(BodyCommand::FullSize, t + 304.0));
        assert_eq!(c.mode(), Mode::Standing);
    }

    #[test]
    fn a_double_click_brings_her_back_from_mini_or_lying() {
        let (mut c, ledges, t) = rested(6);
        c.command(BodyCommand::Nap, t);
        c.poke(t + 1.0);
        assert_eq!(c.mode(), Mode::Standing);
        let (t, _) = run(&mut c, t + 1.0, 2.0, &ledges);
        c.command(BodyCommand::LieDown, t);
        c.poke(t + 1.0);
        assert_eq!(c.mode(), Mode::Standing);
    }

    #[test]
    fn without_the_art_she_naps_standing_and_never_lies_down() {
        let mut c = Character::new(calm(), 7);
        c.appear(Point::new(100.0, 0.0), 0.0);
        let (t, _) = run(&mut c, 0.0, 3.0, &[floor()]);
        assert!(!c.command(BodyCommand::LieDown, t));
        assert!(!c.command(BodyCommand::Mini, t));
        assert!(c.command(BodyCommand::Nap, t));
        assert_eq!(c.mode(), Mode::Sleeping);
        assert!(c.command(BodyCommand::WakeUp, t + 1.0));
        assert_eq!(c.mode(), Mode::Waking);
    }

    #[test]
    fn roaming_with_rests_lies_down_and_naps_now_and_then() {
        let cfg = CharacterConfig { roam_every: (1.0, 2.0), ..CharacterConfig::default() };
        let mut c = Character::new(cfg, 11);
        c.set_abilities(Abilities { lie_down: true, mini: true }, 0.0);
        c.appear(Point::new(100.0, 0.0), 0.0);
        let ledges = [floor(), window(7, 500.0)];
        let mut seen = std::collections::HashSet::new();
        let mut t = 0.0;
        while t < 1200.0 {
            t += 1.0 / 30.0;
            seen.insert(c.tick(t, 1.0 / 30.0, &ledges).0.mode);
            c.touch(t); // keep her from falling asleep for good
        }
        assert!(seen.contains(&Mode::Lying) && seen.contains(&Mode::Mini), "{seen:?}");

        let quiet = CharacterConfig { rests: false, ..cfg };
        let mut c = Character::new(quiet, 11);
        c.set_abilities(Abilities { lie_down: true, mini: true }, 0.0);
        c.appear(Point::new(100.0, 0.0), 0.0);
        let mut t = 0.0;
        while t < 600.0 {
            t += 1.0 / 30.0;
            let m = c.tick(t, 1.0 / 30.0, &ledges).0.mode;
            assert!(!matches!(m, Mode::Lying | Mode::Mini));
            c.touch(t);
        }
    }

    #[test]
    fn sleeping_shows_zs() {
        let (mut c, ledges, t) = rested(8);
        let cfg = CharacterConfig { sleep_after: 5.0, ..*c.config() };
        c.set_config(cfg);
        let (t, _) = run(&mut c, t, 10.0, &ledges);
        assert_eq!(c.mode(), Mode::Sleeping);
        assert!(pose(&mut c, t, &ledges).asleep);
    }
}
