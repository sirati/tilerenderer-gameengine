//! Button snapping: while the mouse is captured, holding Alt makes the
//! pointer go from one thing of the driver's interface that can be clicked
//! (`scene::Interactable`: buttons, list entries, lists) to the next rather
//! than move freely.
//!
//! Three pointers take part:
//!
//! - A hidden one ([`Glide`]), moved by the mouse's raw motion, with
//!   momentum and friction. It starts on the element snapped to.
//! - The target: [`Settings::input`]'s `snap_updates_per_second` times a
//!   second, where the hidden pointer would come to rest under friction is
//!   worked out, and the element nearest to that point becomes the target.
//!   A translucent ghost pointer shows at the element the next update would
//!   pick, `snap_ghost_fps` times a second.
//! - The visible pointer ([`Spring`]) flies to the target on a damped
//!   spring (accelerating smoothly, overshooting a little and settling
//!   back), keeping its speed when the target changes in flight. Only once
//!   it rests on an element does the driver get the pointer there (the
//!   element shows its hover), and the hidden pointer starts from there.
//!
//! In a list that scrolls, the pointer stays on its entry and the list
//! scrolls instead ([`Scroller`]: wheel notches spread out in time), until
//! an update lands outside the list. A click clicks the element snapped to.
//! Releasing Alt (or pressing Ctrl, which lets go of the mouse) leaves the
//! pointer where it is: clicks go there until the mouse moves again.
//!
//! The constants are tuned for the default of 3 updates a second; the
//! spring's speed follows the configured rate.
//!
//! [`Settings::input`]: viewer_settings::Settings

use scene::{Button, Input, Interactable, InteractableKind};

use crate::layout::Layout;

/// A window position in pixels (fractional while moving).
pub type Point = (f64, f64);

/// The hidden pointer's friction, per second: once the mouse stops, its
/// speed falls as `e^(−FRICTION·t)`, so it comes to rest `v / FRICTION`
/// further on. 6/s: a glide's time constant (1/6 s) is half an update
/// interval at 3 updates a second, so by the next update a throw has gone
/// 86 % of its way; a flick of 1200 px/s throws the target 200 px ahead,
/// a slow, careful move (60 px/s) only 10 px: precise when slow, far when
/// fast.
pub const FRICTION: f64 = 6.0;

/// How quickly the hidden pointer's speed follows the mouse's (time
/// constant, seconds). 50 ms: a few mouse reports (125-1000 Hz), so one
/// jittery report does not throw the target, short against the 333 ms
/// update interval.
pub const VELOCITY_SMOOTHING: f64 = 0.05;

/// Window pixels per raw mouse count (as an unaccelerated system pointer).
pub const MOTION_GAIN: f64 = 1.0;

/// The visible pointer's damping fraction (ζ; SwiftUI's `dampingFraction`,
/// Apple's springs are this model): from rest it overshoots by
/// `e^(−πζ/√(1−ζ²))`, 2.8 % at 0.75 (9 px on a 300 px jump), and settles
/// back without a second visible swing.
pub const DAMPING: f64 = 0.75;

/// When the visible pointer first reaches its target, starting from rest,
/// in update intervals: 4/3 (444 ms at 3 updates a second). At the next
/// update it is still moving at over 40 % of its top speed, so a new target
/// steers it along rather than starting it again.
pub const ARRIVAL: f64 = 4.0 / 3.0;

/// The visible pointer is at rest within this distance of its target
/// (pixels) and below [`REST_SPEED`] (pixels a second); it is then put on
/// the target exactly.
pub const REST_DISTANCE: f64 = 0.5;
pub const REST_SPEED: f64 = 10.0;

/// The longest step simulated at once (seconds): a stalled frame does not
/// make the pointer jump.
pub const MAX_STEP: f64 = 0.1;

/// The spring's integration step (seconds), well below its period (0.7 s
/// at 3 updates a second) for semi-implicit Euler to be accurate.
const SUBSTEP: f64 = 1.0 / 480.0;

/// Wheel notches for a list's scroll are spread out with this time
/// constant (seconds): what an update asks for is 95 % sent by the next
/// update at 3 a second (3 time constants).
pub const SCROLL_SMOOTHING: f64 = 0.1;

/// At most this many wheel notches a second (a game takes each as a step).
pub const MAX_NOTCHES_PER_SECOND: f64 = 30.0;

/// Pixels a list scrolls for one notch when neither the list nor its
/// entries tell.
pub const DEFAULT_SCROLL_STEP: f64 = 16.0;

/// One notch, as `Input::Wheel` counts them.
const NOTCH: i32 = 120;

/// The sprite sets of the snapping pointer and its translucent ghost (the
/// pointer's tip at the anchor). Without them, the world cursor's art is
/// drawn and no ghost.
pub const POINTER_SPRITE: &str = "Viewer@pointer/0";
pub const GHOST_SPRITE: &str = "Viewer@pointer_ghost/0";

const VK_CONTROL: u32 = 0x11;
const VK_MENU: u32 = 0x12;

/// Something to snap to, in window pixels.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Target {
    /// x, y, width, height.
    pub rect: [f64; 4],
    pub kind: TargetKind,
    pub id: u64,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum TargetKind {
    Button,
    /// An entry, of the list at this index of the targets if it lies in one.
    ListItem { list: Option<usize> },
    /// A list whose entries scroll in `scroll`, `step` pixels a notch;
    /// snapped to itself only if no entry of it is known.
    List { scroll: [f64; 4], step: f64, has_items: bool },
}

fn inside([x, y, w, h]: [f64; 4], (px, py): Point) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

impl Target {
    pub fn center(&self) -> Point {
        let [x, y, w, h] = self.rect;
        // (Whole pixels: the driver gets them.)
        ((x + w / 2.0).floor(), (y + h / 2.0).floor())
    }

    /// How far `p` is from the rectangle (0 inside).
    fn distance(&self, (px, py): Point) -> f64 {
        let [x, y, w, h] = self.rect;
        let dx = (x - px).max(px - (x + w)).max(0.0);
        let dy = (y - py).max(py - (y + h)).max(0.0);
        dx.hypot(dy)
    }

    fn snappable(&self) -> bool {
        !matches!(self.kind, TargetKind::List { has_items: true, .. })
    }

    /// The list it scrolls with (an entry's list, or a list itself).
    fn list(&self, index: usize) -> Option<usize> {
        match self.kind {
            TargetKind::ListItem { list } => list,
            TargetKind::List { .. } => Some(index),
            TargetKind::Button => None,
        }
    }
}

/// The driver's interactables in window pixels: `mapping` is where its
/// screen lies (`Layout::screen_mapping`), those wholly outside the window
/// are left out. Entries are matched with the list they lie in.
pub fn targets(items: &[Interactable], mapping: ((i32, i32), (u32, u32)), window: (u32, u32)) -> Vec<Target> {
    let ((ox, oy), (n, d)) = mapping;
    let scale = n.max(1) as f64 / d.max(1) as f64;
    let map = |[x, y, w, h]: [i32; 4]| [ox as f64 + x as f64 * scale, oy as f64 + y as f64 * scale, w as f64 * scale, h as f64 * scale];
    let on_screen = |[x, y, w, h]: [f64; 4]| w > 0.0 && h > 0.0 && x + w > 0.0 && y + h > 0.0 && x < window.0 as f64 && y < window.1 as f64;
    let mut out: Vec<Target> = items
        .iter()
        .map(|i| Target {
            rect: map(i.rect),
            kind: match i.kind {
                InteractableKind::Button => TargetKind::Button,
                InteractableKind::ListItem => TargetKind::ListItem { list: None },
                InteractableKind::List { scroll, step } => TargetKind::List { scroll: map(scroll), step: step.max(0) as f64 * scale, has_items: false },
            },
            id: i.id,
        })
        .filter(|t| on_screen(t.rect))
        .collect();
    // Each entry belongs to the smallest list its middle lies in.
    let lists: Vec<(usize, [f64; 4])> = out
        .iter()
        .enumerate()
        .filter_map(|(k, t)| match t.kind {
            TargetKind::List { scroll, .. } => Some((k, scroll)),
            _ => None,
        })
        .collect();
    let mut heights: Vec<Vec<f64>> = vec![Vec::new(); out.len()];
    for t in &mut out {
        if let TargetKind::ListItem { list } = &mut t.kind {
            let c = (t.rect[0] + t.rect[2] / 2.0, t.rect[1] + t.rect[3] / 2.0);
            *list = lists.iter().filter(|(_, s)| inside(*s, c)).min_by(|a, b| (a.1[2] * a.1[3]).total_cmp(&(b.1[2] * b.1[3]))).map(|(k, _)| *k);
            if let Some(k) = *list {
                heights[k].push(t.rect[3]);
            }
        }
    }
    for (k, t) in out.iter_mut().enumerate() {
        if let TargetKind::List { step, has_items, .. } = &mut t.kind {
            *has_items = !heights[k].is_empty();
            if *step <= 0.0 {
                // The entries' usual height, else a guess.
                heights[k].sort_by(f64::total_cmp);
                *step = heights[k].get(heights[k].len() / 2).copied().unwrap_or(DEFAULT_SCROLL_STEP * scale);
            }
        }
    }
    out
}

/// The target to snap to from `p`: the nearest (by distance to its
/// rectangle, then to its middle).
pub fn nearest(targets: &[Target], p: Point) -> Option<usize> {
    let key = |t: &Target| {
        let c = t.center();
        (t.distance(p), (c.0 - p.0).hypot(c.1 - p.1))
    };
    targets
        .iter()
        .enumerate()
        .filter(|(_, t)| t.snappable())
        .min_by(|(_, a), (_, b)| {
            let (ka, kb) = (key(a), key(b));
            ka.0.total_cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        })
        .map(|(k, _)| k)
}

/// The hidden pointer: follows the mouse one to one, keeps the mouse's
/// speed, and glides on with friction once the mouse stops.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Glide {
    pub pos: Point,
    /// Pixels a second.
    pub vel: Point,
    /// Motion since the last step.
    moved: Point,
}

impl Glide {
    pub fn at(pos: Point) -> Self {
        Self { pos, ..Default::default() }
    }

    /// Raw mouse motion (counts).
    pub fn push(&mut self, (dx, dy): Point) {
        let d = (dx * MOTION_GAIN, dy * MOTION_GAIN);
        self.pos = (self.pos.0 + d.0, self.pos.1 + d.1);
        self.moved = (self.moved.0 + d.0, self.moved.1 + d.1);
    }

    /// `dt` seconds on: while the mouse moves, its speed is followed; once
    /// it stops, the pointer glides.
    pub fn step(&mut self, dt: f64) {
        if dt <= 0.0 {
            return;
        }
        let moved = std::mem::take(&mut self.moved);
        if moved != (0.0, 0.0) {
            let a = 1.0 - (-dt / VELOCITY_SMOOTHING).exp();
            let u = (moved.0 / dt, moved.1 / dt);
            self.vel = (self.vel.0 + (u.0 - self.vel.0) * a, self.vel.1 + (u.1 - self.vel.1) * a);
        } else {
            // Exactly: x(t) = x0 + v0 (1 − e^(−kt)) / k.
            let decay = (-FRICTION * dt).exp();
            let travel = (1.0 - decay) / FRICTION;
            self.pos = (self.pos.0 + self.vel.0 * travel, self.pos.1 + self.vel.1 * travel);
            self.vel = (self.vel.0 * decay, self.vel.1 * decay);
        }
    }

    /// Where it comes to rest if the mouse stays still.
    pub fn rest(&self) -> Point {
        (self.pos.0 + self.vel.0 / FRICTION, self.pos.1 + self.vel.1 / FRICTION)
    }

    /// Kept in the window, stopped at its edges.
    pub fn clamp(&mut self, (w, h): (u32, u32)) {
        let one = |p: &mut f64, v: &mut f64, size: f64| {
            let hi = (size - 1.0).max(0.0);
            if *p < 0.0 || *p > hi {
                *p = p.clamp(0.0, hi);
                *v = 0.0;
            }
        };
        one(&mut self.pos.0, &mut self.vel.0, w as f64);
        one(&mut self.pos.1, &mut self.vel.1, h as f64);
    }
}

/// The visible pointer: a damped spring pulling it to its target.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Spring {
    pub pos: Point,
    pub vel: Point,
    pub target: Point,
}

impl Spring {
    pub fn at(pos: Point) -> Self {
        Self { pos, vel: (0.0, 0.0), target: pos }
    }

    /// The natural frequency (rad/s) at which a spring with [`DAMPING`]
    /// starting from rest first reaches its target after [`ARRIVAL`] update
    /// intervals: the step response crosses 1 at `ω_d·t = π − acos ζ`.
    pub fn omega(updates_per_second: f64) -> f64 {
        let arrival = ARRIVAL / updates_per_second;
        (std::f64::consts::PI - DAMPING.acos()) / ((1.0 - DAMPING * DAMPING).sqrt() * arrival)
    }

    /// `dt` seconds on at natural frequency `omega`.
    pub fn step(&mut self, dt: f64, omega: f64) {
        let n = (dt / SUBSTEP).ceil().max(1.0) as u32;
        let h = dt / n as f64;
        for _ in 0..n {
            let accel = |x: f64, v: f64, t: f64| omega * omega * (t - x) - 2.0 * DAMPING * omega * v;
            self.vel.0 += accel(self.pos.0, self.vel.0, self.target.0) * h;
            self.vel.1 += accel(self.pos.1, self.vel.1, self.target.1) * h;
            self.pos.0 += self.vel.0 * h;
            self.pos.1 += self.vel.1 * h;
        }
    }

    /// Whether it is (as good as) still on its target; then it is put
    /// there exactly.
    pub fn settle(&mut self) -> bool {
        let off = (self.target.0 - self.pos.0).hypot(self.target.1 - self.pos.1);
        let speed = self.vel.0.hypot(self.vel.1);
        if off < REST_DISTANCE && speed < REST_SPEED {
            self.pos = self.target;
            self.vel = (0.0, 0.0);
            true
        } else {
            false
        }
    }

    fn at_rest(&self) -> bool {
        self.pos == self.target && self.vel == (0.0, 0.0)
    }
}

/// Wheel notches spread out in time.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Scroller {
    /// Notches still to send (positive: wheel up, towards the top).
    pending: f64,
    /// Sent in part, not yet a whole notch.
    carry: f64,
    since_notch: f64,
}

impl Scroller {
    pub fn add(&mut self, notches: f64) {
        self.pending += notches;
    }

    pub fn idle(&self) -> bool {
        self.pending == 0.0 && self.carry == 0.0
    }

    /// `dt` seconds on: the notch to send now (+1 up, −1 down, 0 none).
    pub fn step(&mut self, dt: f64) -> i32 {
        self.since_notch += dt;
        let part = self.pending * (1.0 - (-dt / SCROLL_SMOOTHING).exp());
        self.pending -= part;
        self.carry += part;
        if self.pending.abs() < 0.02 {
            self.carry += std::mem::take(&mut self.pending);
        }
        // The last part of a notch rounds.
        let whole = if self.pending == 0.0 { 0.5 } else { 1.0 };
        if self.carry.abs() >= whole && self.since_notch >= 1.0 / MAX_NOTCHES_PER_SECOND {
            let n = self.carry.signum();
            self.carry -= n;
            self.since_notch = 0.0;
            if self.pending == 0.0 && self.carry.abs() < 0.5 {
                self.carry = 0.0;
            }
            return n as i32;
        }
        if self.pending == 0.0 && self.carry.abs() < 0.5 {
            self.carry = 0.0;
        }
        0
    }
}

/// What the snapping pointer does, in window pixels.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Action {
    Move(Point),
    Button { button: Button, down: bool, at: Point },
    Wheel { delta: i32, at: Point },
}

/// The element snapped to.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Snapped {
    id: u64,
    /// The list it scrolls with (its id).
    list: Option<u64>,
    /// Whether it is a list's entry (the pointer stays while the list
    /// scrolls under it).
    entry: bool,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
enum Phase {
    #[default]
    Off,
    Snapping,
    /// Left snapping: the pointer stays here until the mouse moves.
    Parked(Point),
}

/// How often things happen (from the settings).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rates {
    pub updates_per_second: f64,
    pub ghost_fps: f64,
}

impl Rates {
    pub fn of(input: &viewer_settings::Input) -> Self {
        Self { updates_per_second: (input.snap_updates_per_second as f64).clamp(0.5, 60.0), ghost_fps: (input.snap_ghost_fps as f64).clamp(1.0, 240.0) }
    }
}

impl Default for Rates {
    fn default() -> Self {
        Self::of(&viewer_settings::Input::default())
    }
}

/// The snapping pointer's state (no input handling, no layout: see
/// [`SnapInput`]). Times are seconds on any clock.
#[derive(Clone, Debug, Default)]
pub struct Snapper {
    phase: Phase,
    glide: Glide,
    spring: Spring,
    snapped: Option<Snapped>,
    ghost: Option<Point>,
    /// Whether the driver has the pointer where the visible one rests.
    hovered: bool,
    next_update: f64,
    next_ghost: f64,
    last: f64,
    scroll: Scroller,
    /// Notches were sent: once they are all out, the driver gets the
    /// pointer again (what is under it now shows its hover).
    scrolled: bool,
    /// Where each button's press went.
    pressed: [Option<Point>; 3],
}

fn button_index(b: Button) -> usize {
    match b {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
    }
}

/// What an update decides.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Choice {
    /// Snap to the target at this index.
    Snap(usize),
    /// Stay, and scroll the list by this many pixels (positive: the pointer
    /// went down the list).
    Scroll { by: f64, step: f64 },
    Nothing,
}

impl Snapper {
    pub fn snapping(&self) -> bool {
        self.phase == Phase::Snapping
    }

    /// The visible pointer, while snapping or left where snapping ended.
    pub fn pointer(&self) -> Option<Point> {
        match self.phase {
            Phase::Snapping => Some(self.spring.pos),
            Phase::Parked(p) => Some(p),
            Phase::Off => None,
        }
    }

    /// The ghost pointer, while snapping.
    pub fn ghost(&self) -> Option<Point> {
        self.ghost.filter(|_| self.snapping())
    }

    /// The point snapped to (where the visible pointer is headed).
    pub fn target(&self) -> Option<Point> {
        self.snapped.filter(|_| self.snapping()).map(|_| self.spring.target)
    }

    /// The hidden pointer.
    pub fn glide(&self) -> &Glide {
        &self.glide
    }

    pub fn spring(&self) -> &Spring {
        &self.spring
    }

    /// Starts snapping at time `now` with the pointer at `at`: it snaps to
    /// the element nearest to it at once.
    pub fn enter(&mut self, now: f64, at: Point, targets: &[Target], rates: Rates) {
        *self = Self { pressed: self.pressed, ..Self::default() };
        self.phase = Phase::Snapping;
        self.spring = Spring::at(at);
        self.glide = Glide::at(at);
        self.last = now;
        if let Some(k) = nearest(targets, at) {
            self.snap_to(targets, k);
            // The hidden pointer starts on the element snapped to.
            self.glide.pos = self.spring.target;
        }
        self.ghost = Some(self.spring.target);
        self.next_update = now + 1.0 / rates.updates_per_second;
        self.next_ghost = now + 1.0 / rates.ghost_fps;
    }

    /// Stops snapping, the pointer staying where it is: what the driver
    /// gets (the pointer there, unless it has it already).
    pub fn leave(&mut self) -> Vec<Action> {
        if !self.snapping() {
            return vec![];
        }
        let p = (self.spring.pos.0.round(), self.spring.pos.1.round());
        let had = self.hovered && self.spring.at_rest();
        self.phase = Phase::Parked(p);
        if had { vec![] } else { vec![Action::Move(p)] }
    }

    /// Nothing more to do (the mouse was let go, or moved after snapping).
    pub fn stop(&mut self) {
        self.phase = Phase::Off;
        self.ghost = None;
    }

    /// Raw mouse motion: moves the hidden pointer while snapping (true);
    /// a pointer left by snapping is taken up by the mouse again (false:
    /// the motion is the mouse's as usual).
    pub fn motion(&mut self, d: Point) -> bool {
        match self.phase {
            Phase::Snapping => {
                self.glide.push(d);
                true
            }
            Phase::Parked(_) => {
                self.phase = Phase::Off;
                false
            }
            Phase::Off => false,
        }
    }

    /// A button: while snapping it goes to the element snapped to (the
    /// pointer put there), where snapping ended to that point; a release
    /// goes where its press went. `None`: not the snapping pointer's.
    pub fn click(&mut self, button: Button, down: bool) -> Option<Vec<Action>> {
        let b = button_index(button);
        if !down {
            return self.pressed[b].take().map(|at| vec![Action::Button { button, down, at }]);
        }
        let at = match self.phase {
            Phase::Snapping => {
                let at = if self.snapped.is_some() { self.spring.target } else { (self.spring.pos.0.round(), self.spring.pos.1.round()) };
                // On the element now, and the hidden pointer from there.
                self.spring = Spring::at(at);
                self.glide = Glide::at(at);
                self.hovered = true;
                self.ghost = Some(at);
                at
            }
            Phase::Parked(p) => p,
            Phase::Off => return None,
        };
        self.pressed[b] = Some(at);
        Some(vec![Action::Move(at), Action::Button { button, down, at }])
    }

    /// A wheel turn goes where the visible pointer is. `None`: not the
    /// snapping pointer's.
    pub fn wheel(&mut self, delta: i32) -> Option<Vec<Action>> {
        let at = self.pointer()?;
        Some(vec![Action::Wheel { delta, at: (at.0.round(), at.1.round()) }])
    }

    /// Time `now` (in `window`, `targets` as they are now): the pointers
    /// move on, updates and the ghost come when due, the list scrolls.
    pub fn tick(&mut self, now: f64, targets: &[Target], window: (u32, u32), rates: Rates) -> Vec<Action> {
        let mut out = Vec::new();
        if !self.snapping() {
            return out;
        }
        let dt = (now - self.last).clamp(0.0, MAX_STEP);
        self.last = now;
        self.glide.step(dt);
        self.glide.clamp(window);
        self.track(targets);
        if now >= self.next_update {
            self.update(targets);
            let interval = 1.0 / rates.updates_per_second;
            self.next_update = (self.next_update + interval).max(now);
        }
        if now >= self.next_ghost {
            self.ghost = Some(match self.choose(targets) {
                Choice::Snap(k) => targets[k].center(),
                Choice::Scroll { .. } | Choice::Nothing => self.spring.target,
            });
            let interval = 1.0 / rates.ghost_fps;
            self.next_ghost = (self.next_ghost + interval).max(now);
        }
        let was_at_rest = self.spring.at_rest();
        self.spring.step(dt, Spring::omega(rates.updates_per_second));
        if self.spring.settle() {
            if self.snapped.is_some() && !self.hovered {
                // At rest on the element: it shows its hover, and the hidden
                // pointer goes on from it.
                out.push(Action::Move(self.spring.target));
                self.hovered = true;
                self.glide.pos = self.spring.target;
            }
        } else if was_at_rest {
            self.hovered = false;
        }
        match self.scroll.step(dt) {
            0 => {}
            n => {
                out.push(Action::Wheel { delta: n * NOTCH, at: self.spring.pos });
                self.scrolled = true;
            }
        }
        if self.scrolled && self.scroll.idle() {
            self.scrolled = false;
            if self.spring.at_rest() {
                out.push(Action::Move(self.spring.pos));
            }
        }
        out
    }

    fn snap_to(&mut self, targets: &[Target], k: usize) {
        let t = &targets[k];
        let list = t.list(k).map(|l| targets[l].id);
        self.snapped = Some(Snapped { id: t.id, list, entry: matches!(t.kind, TargetKind::ListItem { .. }) });
        let c = t.center();
        if c != self.spring.target {
            self.spring.target = c;
            self.hovered = false;
        }
    }

    /// What an update would do now.
    fn choose(&self, targets: &[Target]) -> Choice {
        let landing = self.glide.rest();
        // In a list that scrolls, landing inside it scrolls it.
        let list = self.snapped.and_then(|s| s.list).and_then(|id| targets.iter().find(|t| t.id == id));
        if let Some(Target { rect, kind: TargetKind::List { step, .. }, .. }) = list {
            if inside(*rect, landing) {
                return Choice::Scroll { by: landing.1 - self.spring.target.1, step: *step };
            }
        }
        nearest(targets, landing).map_or(Choice::Nothing, Choice::Snap)
    }

    /// An update: snap to where the hidden pointer comes to rest.
    fn update(&mut self, targets: &[Target]) {
        match self.choose(targets) {
            Choice::Snap(k) => {
                if self.snapped.map(|s| s.id) != Some(targets[k].id) {
                    self.snap_to(targets, k);
                }
            }
            Choice::Scroll { by, step } => {
                // Moving down the list shows what is further down: the wheel
                // turned towards the user (negative).
                self.scroll.add(-by / step.max(1.0));
                // That way is used up: the hidden pointer stays level with
                // the visible one.
                self.glide.pos.1 = self.spring.target.1;
                self.glide.vel.1 = 0.0;
            }
            Choice::Nothing => {}
        }
    }

    /// Keeps the target on its element as the scene changes: an element
    /// that moved is followed; a list's entry stays where the pointer is
    /// (the entry there now, once the list scrolled).
    fn track(&mut self, targets: &[Target]) {
        let Some(s) = self.snapped else { return };
        let t = self.spring.target;
        if s.entry {
            let still = targets.iter().any(|x| x.id == s.id && inside(x.rect, t));
            if !still {
                let list = s.list.and_then(|id| targets.iter().position(|x| x.id == id));
                let under = targets.iter().enumerate().find(|(_, x)| matches!(x.kind, TargetKind::ListItem { list: l } if l == list) && inside(x.rect, t));
                if let Some((_, x)) = under {
                    self.snapped = Some(Snapped { id: x.id, ..s });
                }
            }
            return;
        }
        if let Some(x) = targets.iter().find(|x| x.id == s.id) {
            let c = x.center();
            if c != t {
                self.spring.target = c;
                self.hovered = false;
            }
        }
    }
}

/// Button snapping in the viewer: Alt (while the mouse is captured)
/// starts it, its release ends it; buttons and the wheel go to the
/// snapping pointer; everything else goes on to the layout.
#[derive(Debug, Default)]
pub struct SnapInput {
    pub snapper: Snapper,
    ctrl: bool,
    /// Alt's press was taken for snapping (its release ends it).
    alt: bool,
    /// Alt's press went to the driver after all (another key with it).
    alt_sent: bool,
    /// Whether the system pointer is to be hidden, when that changes.
    hide_pointer: Option<bool>,
}

/// An action as the driver gets it (nothing for a point it does not take
/// input at).
fn to_input(action: Action, layout: &Layout) -> Option<Input> {
    let whole = |(x, y): Point| (x.round() as i32, y.round() as i32);
    Some(match action {
        Action::Move(p) => {
            let (x, y) = layout.route_window(whole(p))?;
            Input::MouseMove { x, y }
        }
        Action::Button { button, down, at } => {
            let (x, y) = layout.route_window(whole(at))?;
            Input::MouseButton { button, down, x, y }
        }
        Action::Wheel { delta, at } => {
            let (x, y) = layout.route_window(whole(at))?;
            Input::Wheel { delta, x, y }
        }
    })
}

fn to_inputs(actions: Vec<Action>, layout: &Layout) -> Vec<Input> {
    actions.into_iter().filter_map(|a| to_input(a, layout)).collect()
}

impl SnapInput {
    /// The targets in `scene` as `layout` shows them.
    fn targets(scene: &scene::Scene, layout: &Layout) -> Vec<Target> {
        targets(&scene.interactables, layout.screen_mapping(), layout.window())
    }

    /// A window input at time `now`: what goes to the driver.
    pub fn route(&mut self, input: Input, layout: &mut Layout, scene: &scene::Scene, now: f64, rates: Rates) -> Vec<Input> {
        match input {
            Input::Key { vk: VK_MENU, down: true, .. } if self.snapper.snapping() => vec![],
            Input::Key { vk: VK_MENU, down: true, .. } if layout.captured() && !self.ctrl => {
                let at = match self.snapper.pointer() {
                    Some(p) => p,
                    None => {
                        let (x, y) = layout.pointer_in_window();
                        (x as f64, y as f64)
                    }
                };
                self.snapper.enter(now, at, &Self::targets(scene, layout), rates);
                self.alt = true;
                self.hide_pointer = Some(true);
                log::debug!("viewer: button snapping from {at:?}");
                vec![]
            }
            Input::Key { vk: VK_MENU, down: false, .. } if self.alt => {
                self.alt = false;
                let mut out = self.leave_inputs(layout);
                if std::mem::take(&mut self.alt_sent) {
                    out.extend(layout.route(input));
                }
                out
            }
            Input::Key { vk: VK_CONTROL, down, .. } => {
                self.ctrl = down;
                // Ctrl lets go of the mouse: snapping ends first, the
                // pointer staying where it is.
                let mut out = if down { self.leave_inputs(layout) } else { vec![] };
                out.extend(layout.route(input));
                out
            }
            // Another key with Alt held: the driver gets Alt first.
            Input::Key { down: true, .. } if self.alt && !self.alt_sent => {
                self.alt_sent = true;
                let mut out = layout.route(Input::Key { vk: VK_MENU, down: true, text: None });
                out.extend(layout.route(input));
                out
            }
            Input::MouseButton { button, down, x, y } => match self.snapper.click(button, down) {
                Some(actions) => to_inputs(actions, layout),
                None => layout.route(Input::MouseButton { button, down, x, y }),
            },
            Input::Wheel { delta, .. } if !self.ctrl => match self.snapper.wheel(delta) {
                Some(actions) => to_inputs(actions, layout),
                None => layout.route(input),
            },
            other => layout.route(other),
        }
    }

    /// Ends snapping, the pointer staying where it is.
    fn leave(&mut self, layout: &mut Layout) -> Vec<Action> {
        if !self.snapper.snapping() {
            return vec![];
        }
        let actions = self.snapper.leave();
        if let Some((x, y)) = self.snapper.pointer() {
            layout.place_pointer((x as i32, y as i32));
            log::debug!("viewer: button snapping ends at ({x}, {y})");
        }
        self.hide_pointer = Some(false);
        actions
    }

    fn leave_inputs(&mut self, layout: &mut Layout) -> Vec<Input> {
        let actions = self.leave(layout);
        to_inputs(actions, layout)
    }

    /// Raw mouse motion: `None` if it is the layout's (as without
    /// snapping), else what goes to the driver.
    pub fn motion(&mut self, d: (f64, f64)) -> Option<Vec<Input>> {
        self.snapper.motion(d).then(Vec::new)
    }

    /// Time `now`: the pointers move on; what goes to the driver.
    pub fn tick(&mut self, layout: &mut Layout, scene: &scene::Scene, now: f64, rates: Rates) -> Vec<Input> {
        // Without the mouse captured there is nothing to snap.
        if !layout.captured() && self.snapper.pointer().is_some() {
            let out = self.leave_inputs(layout);
            self.snapper.stop();
            return out;
        }
        let actions = self.snapper.tick(now, &Self::targets(scene, layout), layout.window(), rates);
        to_inputs(actions, layout)
    }

    /// The window lost the keyboard: no key is held any more.
    pub fn focus_lost(&mut self, layout: &mut Layout) -> Vec<Input> {
        let out = self.leave_inputs(layout);
        self.snapper.stop();
        (self.ctrl, self.alt, self.alt_sent) = (false, false, false);
        out
    }

    /// Whether to hide the system pointer, when that changes.
    pub fn take_hide_pointer(&mut self) -> Option<bool> {
        self.hide_pointer.take()
    }

    /// The pointers as sprites on top of everything, in the camera of the
    /// driver's topmost interface (drawn in the driver's screen pixels):
    /// the ghost, then the pointer over it.
    pub fn sprites(&self, compiled: &render_desc::Compiled, scene: &scene::Scene, frame: &crate::layout::Frame, layout: &Layout) -> Vec<tile_render::PlacedSprite> {
        let Some(pointer) = self.snapper.pointer() else { return vec![] };
        let interface = scene.views.iter().zip(&frame.view_cameras).filter(|(v, _)| v.view.interface_only).filter_map(|(_, c)| *c).last();
        let camera = interface.or_else(|| frame.cameras.iter().position(|c| c.origin == (0, 0) && c.zoom == (1, 1)).map(|c| c as u32)).unwrap_or(0);
        let ((ox, oy), (n, d)) = layout.screen_mapping();
        let to_screen = |(x, y): Point| (((x - ox as f64) * d as f64 / n.max(1) as f64).round() as i32, ((y - oy as f64) * d as f64 / n.max(1) as f64).round() as i32);
        let place = |name: &str, p: Point| {
            let (set, _) = compiled.set(name)?;
            let (x, y) = to_screen(p);
            Some(tile_render::PlacedSprite { screen: true, view: Some(camera), ..tile_render::PlacedSprite::new(set, x, y) })
        };
        let solid = place(POINTER_SPRITE, pointer).or_else(|| place(crate::CURSOR_SPRITE, pointer));
        let ghost = self.snapper.ghost().and_then(|g| place(GHOST_SPRITE, g));
        ghost.into_iter().chain(solid).collect()
    }
}

#[cfg(test)]
mod tests;
