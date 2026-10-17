//! Button snapping: a mode in which the pointer goes from one thing of the
//! driver's interface that can be clicked (`scene::Interactable`: buttons,
//! list entries, lists) to the next rather than move freely.
//!
//! **The mode.** Alt pressed on its own turns it on, on any screen, the
//! mouse captured or not; it stays on once Alt is let go. Alt again, Esc or
//! Ctrl (which also lets go of a captured mouse) turn it off. Alt held
//! with another key is a shortcut for the driver: the driver gets Alt and
//! the key, and the mode does not come on. While the mode is on its pointer
//! (and a translucent ghost) are the only cursor ([`SnapInput::active`],
//! `crate::cursor::SnapMode`): the system pointer and the world cursor are
//! not shown. The mouse's raw motion steers the pointer; the layout's
//! divider, its walls and the window's edges (edge scrolling) do not apply
//! to it: it reaches the interface anywhere in the window, in any view,
//! the corner's buttons and popups drawn elsewhere included
//! ([`Layout::moved_parts`]). Turned on again, it goes on from the element
//! it last rested on (if that is still there), else from the one nearest
//! the pointer.
//!
//! **Where it goes.** On Alt it snaps at once to the element nearest the
//! pointer. Then, at most [`Rates::updates_per_second`] times a second, the
//! mouse's motion since the last update picks the next element: one lying
//! within [`CONE`] (45°) of the motion's direction, seen from the element
//! snapped to, the one most nearly along that direction (the angle weighs
//! far more than the distance) at about the distance the mouse moved.
//! Motion shorter than [`DEAD_ZONE`] picks nothing (jitter does not make it
//! flip); without an element within the cone it stays. The first update
//! after the mouse rested comes at once.
//!
//! **How it moves.** Every move is a flight ([`Flight`]) from where the
//! pointer is to the element's middle, lasting exactly [`FLIGHT`] update
//! intervals (1.33: 444 ms at 3 a second): a damped spring (accelerating
//! smoothly, overshooting a little and settling back, as Apple's) eased
//! onto the target at its end, so that it lands on it exactly then. A new
//! target in flight keeps the pointer's velocity and steers it there, the
//! flight timed anew. It lands also once the mode is turned off; it rests
//! only on an element. Only at rest does the driver get the pointer there
//! (the element shows its hover).
//!
//! **Clicks** go to the element snapped to, never to the world. The view
//! a click captures the mouse in (the one following the world cursor) is
//! an element too ([`TargetKind::Capture`], from the layout, not the
//! driver): clicking it ends the mode and captures the mouse (again).
//!
//! **Lists.** On an entry of a list that scrolls, motion up or down that
//! stays inside the list scrolls it instead, smoothly ([`Scroller`]: wheel
//! notches spread out in time), the pointer staying on its entry.
//!
//! **Leaving.** The pointer lands, and the usual rules take over where it
//! is: on an element of the driver's the system pointer shows there (a
//! captured mouse is let go there); on the capture view the world cursor
//! goes on. Should the element snapped to go (a screen opened over it), the
//! pointer goes on to the one nearest it.
//!
//! [`Layout::moved_parts`]: crate::layout::Layout::moved_parts

use std::f64::consts::PI;

use scene::{Button, Input, Interactable, InteractableKind};

use crate::layout::Layout;

/// A window position in pixels (fractional while moving).
pub type Point = (f64, f64);

/// How long a flight lasts, in update intervals: 4/3 (1.33; 444 ms at 3
/// updates a second). The next update comes before it lands, so a pointer
/// kept moving is steered on rather than stopped.
pub const FLIGHT: f64 = 4.0 / 3.0;

/// The flight's spring: its damping fraction (ζ, SwiftUI's
/// `dampingFraction`), and when (a fraction of the flight) it first reaches
/// its target from rest. From rest it overshoots by about
/// `e^(−πζ/√(1−ζ²))`, 4.6 % at 0.7, a little less as it is eased in.
pub const DAMPING: f64 = 0.7;
pub const CROSSING: f64 = 0.5;

/// From this fraction of the flight on the spring is eased onto the target
/// (smoothstep), landing on it at the flight's end, still.
pub const SETTLE_FROM: f64 = 0.6;

/// The next element lies within this angle of the mouse's motion (radians).
pub const CONE: f64 = PI / 4.0;

/// Motion (pixels) an update needs to pick an element: less is jitter.
pub const DEAD_ZONE: f64 = 12.0;

/// Motion short of twice the dead zone is forgotten after the mouse rested
/// this long (seconds): slow drift adds up, a nudge long ago does not (nor
/// the rest of a nudge that picked at once).
pub const IDLE_RESET: f64 = 0.25;

/// How much an element off the motion's direction is held back: its
/// distance term times `1 + ANGLE_WEIGHT · (angle / CONE)²` (17 at the
/// cone's edge, 4.2 at 20°, 1.8 at 10°): one straight ahead wins over one
/// 20° off unless that is several times nearer.
pub const ANGLE_WEIGHT: f64 = 16.0;

/// The distance term: how far an element is from the distance the mouse
/// moved, plus this much of its distance (nearer ones first when equally
/// far off).
pub const DISTANCE_WEIGHT: f64 = 0.25;

/// Window pixels per raw mouse count (as an unaccelerated system pointer).
pub const MOTION_GAIN: f64 = 1.0;

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

/// The sprite sets of the snapping pointer and its half-transparent ghost
/// (the pointer's tip at the anchor): the driver's arrow and its ghost if
/// the description has them, else the viewer's own.
pub const ARROW_SPRITE: &str = "Viewer@cursors/0";
pub const ARROW_GHOST_SPRITE: &str = "Viewer@cursor_ghost/0";
pub const POINTER_SPRITE: &str = "Viewer@pointer/0";
pub const GHOST_SPRITE: &str = "Viewer@pointer_ghost/0";

/// The id of the capture view's target (no driver's element has it).
pub const CAPTURE_ID: u64 = u64::MAX;

const VK_CONTROL: u32 = 0x11;
const VK_MENU: u32 = 0x12;
const VK_ESCAPE: u32 = 0x1B;

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
    /// The view a click captures the mouse in (`rect`), snapped to at `at`
    /// (where the world cursor is in it): a click there captures the mouse.
    Capture { at: Point },
}

fn inside([x, y, w, h]: [f64; 4], (px, py): Point) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

impl Target {
    /// The capture view `rect` (window pixels), the world cursor at `at`.
    pub fn capture(rect: [i32; 4], at: (i32, i32)) -> Self {
        let [x, y, w, h] = rect;
        Self { rect: [x as f64, y as f64, w as f64, h as f64], kind: TargetKind::Capture { at: (at.0 as f64, at.1 as f64) }, id: CAPTURE_ID }
    }

    /// Where the pointer goes on it.
    pub fn center(&self) -> Point {
        if let TargetKind::Capture { at } = self.kind {
            return at;
        }
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

    fn is_capture(&self) -> bool {
        matches!(self.kind, TargetKind::Capture { .. })
    }

    /// The list it scrolls with (an entry's list, or a list itself).
    fn list(&self, index: usize) -> Option<usize> {
        match self.kind {
            TargetKind::ListItem { list } => list,
            TargetKind::List { .. } => Some(index),
            TargetKind::Button | TargetKind::Capture { .. } => None,
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

/// The driver's element to snap to from `p`: the nearest (by distance to
/// its rectangle, then to its middle). Not the capture view.
pub fn nearest(targets: &[Target], p: Point) -> Option<usize> {
    let key = |t: &Target| {
        let c = t.center();
        (t.distance(p), (c.0 - p.0).hypot(c.1 - p.1))
    };
    targets
        .iter()
        .enumerate()
        .filter(|(_, t)| t.snappable() && !t.is_capture())
        .min_by(|(_, a), (_, b)| {
            let (ka, kb) = (key(a), key(b));
            ka.0.total_cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        })
        .map(|(k, _)| k)
}

/// The element the mouse's `moved` (pixels since the last update) points
/// at from `from` (the element snapped to, `current`): within [`CONE`] of
/// its direction, the angle weighing heavily, at about the distance moved.
/// None for motion within the [`DEAD_ZONE`], or nothing within the cone.
pub fn pick(targets: &[Target], from: Point, current: Option<u64>, moved: Point) -> Option<usize> {
    let reach = moved.0.hypot(moved.1);
    if reach < DEAD_ZONE {
        return None;
    }
    let dir = (moved.0 / reach, moved.1 / reach);
    targets
        .iter()
        .enumerate()
        .filter(|(_, t)| t.snappable() && Some(t.id) != current)
        .filter_map(|(k, t)| {
            let c = t.center();
            let d = (c.0 - from.0, c.1 - from.1);
            let dist = d.0.hypot(d.1);
            if dist < 1.0 {
                return None;
            }
            let angle = ((d.0 * dir.0 + d.1 * dir.1) / dist).clamp(-1.0, 1.0).acos();
            if angle > CONE + 1e-9 {
                return None;
            }
            let off = (dist - reach).abs() + DISTANCE_WEIGHT * dist + 1.0;
            Some((k, off * (1.0 + ANGLE_WEIGHT * (angle / CONE).powi(2))))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// The angle (degrees) between `moved` and the way from `from` to `to`.
fn degrees(from: Point, to: Point, moved: Point) -> f64 {
    let d = (to.0 - from.0, to.1 - from.1);
    let n = d.0.hypot(d.1) * moved.0.hypot(moved.1);
    if n == 0.0 { 0.0 } else { ((d.0 * moved.0 + d.1 * moved.1) / n).clamp(-1.0, 1.0).acos().to_degrees() }
}

/// The visible pointer's way to its target: from `from` at `vel` (pixels a
/// second) at time `start`, landing on `to` exactly `duration` seconds
/// later, still. A damped spring ([`DAMPING`], first reaching the target
/// at [`CROSSING`] of the way from rest), eased onto the target from
/// [`SETTLE_FROM`] on; the starting velocity carries on and dies out the
/// same way.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Flight {
    pub from: Point,
    pub vel: Point,
    pub to: Point,
    pub start: f64,
    pub duration: f64,
}

/// The flight's shape at `u` (0..1 of its time): how much of the way from
/// `from` is left (`F`), how much of the starting velocity's travel
/// (`G`, in flight times), and their rates (per flight time).
fn shape(u: f64) -> (f64, f64, f64, f64) {
    let root = (1.0 - DAMPING * DAMPING).sqrt();
    let omega = (PI - DAMPING.acos()) / (root * CROSSING);
    let (a, wd) = (DAMPING * omega, omega * root);
    let e = (-a * u).exp();
    let (sin, cos) = (wd * u).sin_cos();
    let s = e * (cos + a / wd * sin);
    let s_rate = -e * omega * omega / wd * sin;
    let q = e * sin / wd;
    let q_rate = e * (cos - a / wd * sin);
    let v = ((u - SETTLE_FROM) / (1.0 - SETTLE_FROM)).clamp(0.0, 1.0);
    let m = 1.0 - v * v * (3.0 - 2.0 * v);
    let m_rate = -6.0 * v * (1.0 - v) / (1.0 - SETTLE_FROM);
    (s * m, s_rate * m + s * m_rate, q * m, q_rate * m + q * m_rate)
}

impl Flight {
    /// At rest at `at`.
    pub fn rest(at: Point) -> Self {
        Self { from: at, vel: (0.0, 0.0), to: at, start: 0.0, duration: 0.0 }
    }

    /// When it lands.
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }

    pub fn done(&self, now: f64) -> bool {
        now >= self.end()
    }

    fn progress(&self, now: f64) -> Option<f64> {
        (self.duration > 0.0 && now < self.end()).then(|| ((now - self.start) / self.duration).max(0.0))
    }

    pub fn position(&self, now: f64) -> Point {
        let Some(u) = self.progress(now) else { return self.to };
        let (f, _, g, _) = shape(u);
        let d = self.duration;
        (self.to.0 + (self.from.0 - self.to.0) * f + self.vel.0 * d * g, self.to.1 + (self.from.1 - self.to.1) * f + self.vel.1 * d * g)
    }

    /// Pixels a second.
    pub fn velocity(&self, now: f64) -> Point {
        let Some(u) = self.progress(now) else { return (0.0, 0.0) };
        let (_, f_rate, _, g_rate) = shape(u);
        let d = self.duration;
        ((self.from.0 - self.to.0) * f_rate / d + self.vel.0 * g_rate, (self.from.1 - self.to.1) * f_rate / d + self.vel.1 * g_rate)
    }

    /// From where it is at `now`, at the speed it has, to `to` in
    /// `duration` seconds.
    pub fn retarget(&mut self, to: Point, now: f64, duration: f64) {
        *self = Self { from: self.position(now), vel: self.velocity(now), to, start: now, duration };
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
    /// The capture view was clicked: the mode ends, the mouse is captured
    /// at this point.
    Capture(Point),
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
    capture: bool,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
enum Phase {
    #[default]
    Off,
    /// The mode is on.
    Snapping,
    /// The mode was turned off in flight: the pointer lands.
    Landing,
    /// Landed after the mode: the pointer stays here until the mouse moves.
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

    /// How long a flight lasts (seconds).
    pub fn flight(&self) -> f64 {
        FLIGHT / self.updates_per_second
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
    flight: Flight,
    /// The visible pointer as of the last step.
    pos: Point,
    snapped: Option<Snapped>,
    ghost: Option<Point>,
    /// The flight landed and the driver has the pointer there (or a click
    /// put it there).
    arrived: bool,
    /// Mouse motion since the last update (pixels), whether some came since
    /// the last step, and how long none has come.
    moved: Point,
    moving: bool,
    idle: f64,
    next_update: f64,
    next_ghost: f64,
    last: f64,
    rates: Rates,
    scroll: Scroller,
    /// Notches moved in the list not sent yet (less than half of one).
    scroll_rest: f64,
    /// Notches were sent: once they are all out, the driver gets the
    /// pointer again (what is under it now shows its hover).
    scrolled: bool,
    /// Where each button's press went.
    pressed: [Option<Point>; 3],
    /// The driver's element the pointer last rested on (kept across modes:
    /// the next one goes on from it).
    rested: Option<u64>,
    /// Where it parked, until [`Snapper::take_parked`].
    parked: Option<Point>,
    /// When the element now snapped to was picked (for the log).
    picked: f64,
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
    /// The mode is on.
    pub fn snapping(&self) -> bool {
        self.phase == Phase::Snapping
    }

    /// The snapping pointer is the pointer: the mode is on, or it lands.
    pub fn holds_pointer(&self) -> bool {
        matches!(self.phase, Phase::Snapping | Phase::Landing)
    }

    /// The visible pointer, while snapping, landing or parked.
    pub fn pointer(&self) -> Option<Point> {
        match self.phase {
            Phase::Snapping | Phase::Landing => Some(self.pos),
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
        self.snapped.filter(|_| self.holds_pointer()).map(|_| self.flight.to)
    }

    /// The id of the element snapped to.
    pub fn target_id(&self) -> Option<u64> {
        self.snapped.filter(|_| self.holds_pointer()).map(|s| s.id)
    }

    pub fn flight(&self) -> &Flight {
        &self.flight
    }

    /// The driver's element the pointer last rested on.
    pub fn rested(&self) -> Option<u64> {
        self.rested
    }

    /// Where the pointer parked (landed after the mode), once.
    pub fn take_parked(&mut self) -> Option<Point> {
        self.parked.take()
    }

    /// Turns the mode on at time `now`, the pointer at `at`: it goes on
    /// from the element it last rested on if that is still there (from
    /// there), else it flies to the one nearest `at` at once (the capture
    /// view if there is no other).
    pub fn enter(&mut self, now: f64, at: Point, targets: &[Target], rates: Rates) {
        *self = Self { pressed: self.pressed, rested: self.rested, ..Self::default() };
        self.phase = Phase::Snapping;
        self.rates = rates;
        self.last = now;
        self.pos = at;
        self.flight = Flight::rest(at);
        let resume = self.rested.and_then(|id| targets.iter().position(|t| t.id == id && !t.is_capture() && t.snappable()));
        if let Some(k) = resume {
            self.pos = targets[k].center();
            self.flight = Flight::rest(self.pos);
        }
        if log::log_enabled!(log::Level::Debug) {
            let list: Vec<String> = targets
                .iter()
                .map(|t| {
                    let kind = match t.kind {
                        TargetKind::Button => "button",
                        TargetKind::ListItem { .. } => "entry",
                        TargetKind::List { .. } => "list",
                        TargetKind::Capture { .. } => "capture",
                    };
                    let [x, y, w, h] = t.rect;
                    format!("{kind} {:x} {x:.0},{y:.0} {w:.0}x{h:.0}", t.id)
                })
                .collect();
            log::debug!("snap: {} targets: {}", targets.len(), list.join("; "));
        }
        let k = resume.or_else(|| nearest(targets, at)).or_else(|| targets.iter().position(Target::is_capture));
        match k {
            Some(k) => {
                log::debug!("snap: on at {now:.3} s from {at:?}, {} {}", if resume.is_some() { "back on" } else { "nearest" }, targets[k].id);
                self.snap_to(targets, k, now);
            }
            None => log::debug!("snap: on at {now:.3} s from {at:?}, nothing to snap to"),
        }
        self.ghost = Some(self.flight.to);
        // (The first motion past the dead zone picks at once.)
        self.next_update = now;
        self.next_ghost = now + 1.0 / rates.ghost_fps;
    }

    /// Turns the mode off: the pointer lands (on the capture view: the
    /// world cursor is the pointer again at once). What the driver gets.
    pub fn leave(&mut self) -> Vec<Action> {
        if !self.snapping() {
            return vec![];
        }
        self.ghost = None;
        match self.snapped {
            Some(s) if !s.capture => {
                if self.flight.done(self.last) {
                    let moved = !self.arrived;
                    self.park();
                    if moved {
                        return vec![Action::Move(self.flight.to)];
                    }
                } else {
                    self.phase = Phase::Landing;
                }
            }
            _ => self.phase = Phase::Off,
        }
        vec![]
    }

    fn park(&mut self) {
        let p = self.flight.to;
        self.phase = Phase::Parked(p);
        self.pos = p;
        self.arrived = true;
        self.parked = Some(p);
        if let Some(s) = self.snapped.filter(|s| !s.capture) {
            self.rested = Some(s.id);
        }
        log::debug!("snap: off, the pointer stays at {p:?}");
    }

    /// Nothing more to do (the mouse was let go, or moved after snapping).
    pub fn stop(&mut self) {
        self.phase = Phase::Off;
        self.ghost = None;
        self.parked = None;
    }

    /// Raw mouse motion: steers the pointer while snapping (true; dropped
    /// while it lands); a pointer left by snapping is taken up by the mouse
    /// again (false: the motion is the mouse's as usual).
    pub fn motion(&mut self, d: Point) -> bool {
        match self.phase {
            Phase::Snapping => {
                self.moved = (self.moved.0 + d.0 * MOTION_GAIN, self.moved.1 + d.1 * MOTION_GAIN);
                self.moving = true;
                true
            }
            Phase::Landing => true,
            Phase::Parked(_) => {
                self.phase = Phase::Off;
                false
            }
            Phase::Off => false,
        }
    }

    /// A button: while snapping it goes to the element snapped to (the
    /// pointer put there), where the pointer parked to that point; a
    /// release goes where its press went. Nothing snapped to: nothing (never
    /// the world). `None`: not the snapping pointer's.
    pub fn click(&mut self, button: Button, down: bool) -> Option<Vec<Action>> {
        let b = button_index(button);
        if !down {
            return self.pressed[b].take().map(|at| vec![Action::Button { button, down, at }]);
        }
        let at = match self.phase {
            Phase::Snapping | Phase::Landing => match self.snapped {
                Some(s) if s.capture => return Some(vec![Action::Capture(self.flight.to)]),
                Some(s) => {
                    // On the element now.
                    let at = self.flight.to;
                    self.flight = Flight::rest(at);
                    self.pos = at;
                    self.arrived = true;
                    self.rested = Some(s.id);
                    if self.phase == Phase::Landing {
                        self.park();
                    } else {
                        self.ghost = Some(at);
                    }
                    at
                }
                None => return Some(vec![]),
            },
            Phase::Parked(p) => p,
            Phase::Off => return None,
        };
        self.pressed[b] = Some(at);
        Some(vec![Action::Move(at), Action::Button { button, down, at }])
    }

    /// A wheel turn goes to the element snapped to, or where the pointer
    /// parked. `None`: not the snapping pointer's.
    pub fn wheel(&mut self, delta: i32) -> Option<Vec<Action>> {
        match self.phase {
            Phase::Snapping | Phase::Landing => Some(match self.snapped {
                Some(s) if !s.capture => vec![Action::Wheel { delta, at: self.flight.to }],
                _ => vec![],
            }),
            Phase::Parked(p) => Some(vec![Action::Wheel { delta, at: p }]),
            Phase::Off => None,
        }
    }

    /// Time `now` (`targets` as they are now): the pointer moves on, an
    /// update and the ghost come when due, the list scrolls.
    pub fn tick(&mut self, now: f64, targets: &[Target], rates: Rates) -> Vec<Action> {
        let mut out = Vec::new();
        if !self.holds_pointer() {
            return out;
        }
        self.rates = rates;
        let dt = (now - self.last).max(0.0);
        self.last = now;
        if std::mem::take(&mut self.moving) {
            self.idle = 0.0;
        } else {
            self.idle += dt;
            // (A move on, beyond a nudge, waits for the next update.)
            if self.idle > IDLE_RESET && self.moved.0.hypot(self.moved.1) < 2.0 * DEAD_ZONE {
                self.moved = (0.0, 0.0);
            }
        }
        self.track(targets, now);
        if self.snapping() {
            let reach = self.moved.0.hypot(self.moved.1);
            if now >= self.next_update && reach >= DEAD_ZONE {
                self.update(targets, now);
                self.moved = (0.0, 0.0);
                self.next_update = now + 1.0 / rates.updates_per_second;
            }
            if now >= self.next_ghost {
                self.ghost = Some(match self.choose(targets) {
                    Choice::Snap(k) => targets[k].center(),
                    Choice::Scroll { .. } | Choice::Nothing => self.flight.to,
                });
                self.next_ghost = (self.next_ghost + 1.0 / rates.ghost_fps).max(now);
            }
        }
        self.pos = self.flight.position(now);
        if self.flight.done(now) && !self.arrived {
            self.arrived = true;
            if let Some(s) = self.snapped {
                log::debug!(
                    "snap: landed on {} at {:?} at {:.3} s, {:.0} ms after it was picked (flight {:.0} ms; seen by the step at {now:.3} s, {:.0} ms later)",
                    s.id,
                    self.flight.to,
                    self.flight.end().max(self.picked),
                    (self.flight.end() - self.picked).max(0.0) * 1000.0,
                    self.flight.duration * 1000.0,
                    (now - self.flight.end().max(self.picked)) * 1000.0
                );
                if !s.capture {
                    // At rest on the element: it shows its hover.
                    out.push(Action::Move(self.flight.to));
                    self.rested = Some(s.id);
                }
            }
        }
        if self.phase == Phase::Landing && self.flight.done(now) {
            self.park();
        }
        match self.scroll.step(dt) {
            0 => {}
            n => {
                log::debug!("snap: the list scrolls a notch {}", if n > 0 { "up" } else { "down" });
                out.push(Action::Wheel { delta: n * NOTCH, at: self.flight.to });
                self.scrolled = true;
            }
        }
        if self.scrolled && self.scroll.idle() {
            self.scrolled = false;
            if self.flight.done(now) {
                out.push(Action::Move(self.flight.to));
            }
        }
        out
    }

    /// Snaps to target `k`: the pointer flies there from where it is.
    fn snap_to(&mut self, targets: &[Target], k: usize, now: f64) {
        let t = &targets[k];
        let list = t.list(k).map(|l| targets[l].id);
        self.snapped = Some(Snapped { id: t.id, list, entry: matches!(t.kind, TargetKind::ListItem { .. }), capture: t.is_capture() });
        let c = t.center();
        self.picked = now;
        self.scroll_rest = 0.0;
        if c != self.flight.to || !self.flight.done(now) || self.pos != c {
            let duration = self.rates.flight();
            self.flight.retarget(c, now, duration);
            self.arrived = false;
            log::debug!("snap: at {now:.3} s picked {} ({:?}) at {c:?} from {:?}, lands at {:.3} s", t.id, t.kind, self.pos, now + duration);
        } else if !self.arrived {
            // Already there and still.
            self.flight = Flight::rest(c);
        }
    }

    /// What an update would do now.
    fn choose(&self, targets: &[Target]) -> Choice {
        let from = self.flight.to;
        let m = self.moved;
        // In a list that scrolls, motion up or down staying inside it (or
        // within a step past its edge: from its first or last entry shown)
        // scrolls it.
        let list = self.snapped.and_then(|s| s.list).and_then(|id| targets.iter().find(|t| t.id == id));
        if let Some(Target { rect, kind: TargetKind::List { step, .. }, .. }) = list {
            let [x, y, w, h] = *rect;
            let reach = [x, y - step, w, h + 2.0 * step];
            if m.0.hypot(m.1) >= DEAD_ZONE && m.1.abs() >= m.0.abs() && inside(reach, (from.0 + m.0, from.1 + m.1)) {
                return Choice::Scroll { by: m.1, step: *step };
            }
        }
        pick(targets, from, self.snapped.map(|s| s.id), m).map_or(Choice::Nothing, Choice::Snap)
    }

    /// An update: the motion picks the next element.
    fn update(&mut self, targets: &[Target], now: f64) {
        match self.choose(targets) {
            Choice::Snap(k) => {
                log::debug!("snap: motion {:?} picks {} at {:.0}° off it", self.moved, targets[k].id, degrees(self.flight.to, targets[k].center(), self.moved));
                self.snap_to(targets, k, now);
            }
            Choice::Scroll { by, step } => {
                // Moving down the list shows what is further down: the wheel
                // turned towards the user (negative).
                // (In whole notches, the rest kept for the next: short moves
                // add up.)
                self.scroll_rest += -by / step.max(1.0);
                if self.scroll_rest.abs() >= 0.5 {
                    let whole = self.scroll_rest.round();
                    self.scroll_rest -= whole;
                    self.scroll.add(whole);
                }
            }
            Choice::Nothing => log::debug!("snap: motion {:?}: nothing within {:.0}°, stays", self.moved, CONE.to_degrees()),
        }
    }

    /// Keeps the target on its element as the scene changes: an element
    /// that moved is followed; a list's entry stays where the pointer is
    /// (the entry there now, once the list scrolled).
    fn track(&mut self, targets: &[Target], now: f64) {
        let Some(s) = self.snapped else { return };
        let t = self.flight.to;
        if s.entry {
            if targets.iter().any(|x| x.id == s.id && inside(x.rect, t)) {
                return;
            }
            let list = s.list.and_then(|id| targets.iter().position(|x| x.id == id));
            let under = targets.iter().find(|x| matches!(x.kind, TargetKind::ListItem { list: l } if l == list) && inside(x.rect, t));
            match under {
                Some(x) => self.snapped = Some(Snapped { id: x.id, ..s }),
                None => self.gone(targets, s.id, now),
            }
            return;
        }
        match targets.iter().find(|x| x.id == s.id) {
            Some(x) => {
                let c = x.center();
                if c != t {
                    self.flight.retarget(c, now, self.rates.flight());
                    self.arrived = false;
                }
            }
            None => self.gone(targets, s.id, now),
        }
    }

    /// The element snapped to is gone (a screen opened over it, or closed):
    /// on to the one nearest the pointer (the capture view if there is no
    /// other), so that it never rests where nothing is.
    fn gone(&mut self, targets: &[Target], id: u64, now: f64) {
        let Some(k) = nearest(targets, self.pos).or_else(|| targets.iter().position(Target::is_capture)) else { return };
        log::debug!("snap: {id} is gone: on to the nearest, {}", targets[k].id);
        self.moved = (0.0, 0.0);
        self.snap_to(targets, k, now);
    }
}

/// Button snapping in the viewer: Alt turns the mode on and off; buttons
/// and the wheel go to the snapping pointer; everything else goes on to
/// the layout.
#[derive(Debug, Default)]
pub struct SnapInput {
    pub snapper: Snapper,
    ctrl: bool,
    /// Alt is held (its press repeats).
    alt: bool,
    /// This Alt press turned the mode on (another key with it undoes that).
    alt_entered: bool,
    /// Alt's press went to the driver (another key with it, or Ctrl held).
    alt_sent: bool,
    /// Esc's press turned the mode off (its release is not the driver's).
    esc: bool,
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
        Action::Capture(_) => return None,
    })
}

impl SnapInput {
    /// The snap mode is on (its pointer is the only cursor; it is the
    /// pointer while it lands too).
    pub fn active(&self) -> bool {
        self.snapper.holds_pointer()
    }

    /// The targets in `scene` as `layout` shows them, and the view a click
    /// captures the mouse in.
    fn targets(scene: &scene::Scene, layout: &Layout) -> Vec<Target> {
        let mut out = targets(&scene.interactables, layout.screen_mapping(), layout.window());
        // What lies in a part of the interface drawn elsewhere (the corner's
        // buttons, a popup moved into a view) is there.
        for t in &mut out {
            let c = t.center();
            let moved = layout.moved_parts().iter().find(|([x, y, w, h], (dx, dy))| {
                let (ox, oy) = ((x - dx) as f64, (y - dy) as f64);
                c.0 >= ox && c.1 >= oy && c.0 < ox + *w as f64 && c.1 < oy + *h as f64
            });
            if let Some((_, (dx, dy))) = moved {
                t.rect[0] += *dx as f64;
                t.rect[1] += *dy as f64;
                if let TargetKind::List { scroll, .. } = &mut t.kind {
                    scroll[0] += *dx as f64;
                    scroll[1] += *dy as f64;
                }
            }
        }
        if let Some((rect, at)) = layout.capture_view() {
            out.push(Target::capture(rect, at));
        }
        out
    }

    /// The actions as the driver gets them; a click on the capture view
    /// ends the mode and captures the mouse.
    fn perform(&mut self, actions: Vec<Action>, layout: &mut Layout) -> Vec<Input> {
        let mut out = Vec::new();
        for a in actions {
            match a {
                Action::Capture(p) => {
                    self.snapper.stop();
                    let p = (p.0.round() as i32, p.1.round() as i32);
                    if !layout.captured() {
                        layout.capture_at(p);
                    }
                    log::debug!("snap: the capture view clicked: off, the mouse captured at {p:?}");
                }
                a => out.extend(to_input(a, layout)),
            }
        }
        out.extend(self.sync(layout));
        out
    }

    /// The layout told whether the snapping pointer is the pointer, the
    /// system pointer hidden as long as it is; where it parked, the pointer
    /// is (with the mouse free the system pointer shows there).
    fn sync(&mut self, layout: &mut Layout) -> Vec<Input> {
        // Landed on an element after the mode: the pointer is there, under
        // the usual rules (the system pointer, the mouse let go if it was
        // captured).
        if let Some(p) = self.snapper.take_parked() {
            let p = (p.0.round() as i32, p.1.round() as i32);
            if layout.captured() {
                layout.let_go_at(p);
            } else {
                layout.place_pointer(p);
            }
            self.snapper.stop();
        }
        let on = self.active();
        if on == layout.snapping() {
            return vec![];
        }
        self.hide_pointer = Some(on);
        layout.set_snapping(on)
    }

    /// Turns the mode on.
    fn enter(&mut self, layout: &mut Layout, scene: &scene::Scene, now: f64, rates: Rates) -> Vec<Input> {
        let at = self.snapper.pointer().unwrap_or_else(|| {
            let (x, y) = layout.pointer_in_window();
            (x as f64, y as f64)
        });
        self.snapper.enter(now, at, &Self::targets(scene, layout), rates);
        log::debug!("viewer: button snapping on (the mouse {})", if layout.captured() { "captured" } else { "free" });
        self.sync(layout)
    }

    /// Turns the mode off (the pointer lands).
    fn leave(&mut self, layout: &mut Layout, why: &str) -> Vec<Input> {
        if !self.snapper.snapping() {
            return vec![];
        }
        let actions = self.snapper.leave();
        log::debug!("viewer: button snapping off ({why})");
        self.perform(actions, layout)
    }

    /// A window input at time `now`: what goes to the driver.
    pub fn route(&mut self, input: Input, layout: &mut Layout, scene: &scene::Scene, now: f64, rates: Rates) -> Vec<Input> {
        match input {
            // (Held, it repeats.)
            Input::Key { vk: VK_MENU, down: true, .. } if self.alt => vec![],
            Input::Key { vk: VK_MENU, down: true, .. } => {
                log::debug!("snap: Alt pressed at {now:.3} s (the mode {}, the mouse {})", if self.snapper.snapping() { "on" } else { "off" }, if layout.captured() { "captured" } else { "free" });
                self.alt = true;
                self.alt_entered = false;
                self.alt_sent = false;
                if self.snapper.snapping() {
                    return self.leave(layout, "Alt");
                }
                if self.ctrl {
                    log::debug!("viewer: Alt with Ctrl held: the driver's");
                    self.alt_sent = true;
                    return layout.route(input);
                }
                // (Nothing of the driver's to snap to, e.g. before its first
                // screen: Alt is the driver's.)
                if !Self::targets(scene, layout).iter().any(|t| !t.is_capture()) {
                    log::debug!("snap: nothing to snap to: Alt is the driver's");
                    self.alt_sent = true;
                    return layout.route(input);
                }
                self.alt_entered = true;
                self.enter(layout, scene, now, rates)
            }
            Input::Key { vk: VK_MENU, down: false, .. } => {
                self.alt = false;
                if std::mem::take(&mut self.alt_sent) { layout.route(input) } else { vec![] }
            }
            Input::Key { vk: VK_ESCAPE, down: true, .. } if self.snapper.snapping() => {
                self.esc = true;
                self.leave(layout, "Esc")
            }
            Input::Key { vk: VK_ESCAPE, down: false, .. } if self.esc => {
                self.esc = false;
                vec![]
            }
            Input::Key { vk: VK_CONTROL, down, .. } => {
                self.ctrl = down;
                // Ctrl lets go of the mouse: the mode ends first.
                let mut out = if down { self.leave(layout, "Ctrl") } else { vec![] };
                out.extend(layout.route(input));
                out
            }
            // Another key with Alt held: a shortcut. The driver gets Alt
            // first; the mode this Alt turned on is off again.
            Input::Key { down: true, .. } if self.alt && !self.alt_sent => {
                self.alt_sent = true;
                let mut out = Vec::new();
                if std::mem::take(&mut self.alt_entered) && self.snapper.snapping() {
                    self.snapper.stop();
                    log::debug!("viewer: Alt with a key: a shortcut, no button snapping");
                    out.extend(self.sync(layout));
                }
                out.extend(layout.route(Input::Key { vk: VK_MENU, down: true, text: None }));
                out.extend(layout.route(input));
                out
            }
            // (The system pointer's own motion: not while the snapping
            // pointer is the pointer.)
            Input::MouseMove { .. } if self.active() => vec![],
            Input::MouseButton { button, down, x, y } => match self.snapper.click(button, down) {
                Some(actions) => self.perform(actions, layout),
                None => layout.route(Input::MouseButton { button, down, x, y }),
            },
            Input::Wheel { delta, .. } if !self.ctrl => match self.snapper.wheel(delta) {
                Some(actions) => self.perform(actions, layout),
                None => layout.route(input),
            },
            // Keys while the snapping pointer is the pointer: the interface's
            // (never the world's, whatever the world cursor is over).
            Input::Key { .. } if self.active() => layout
                .route(input)
                .into_iter()
                .map(|i| match i {
                    Input::WorldKey { vk, down, text, .. } => Input::Key { vk, down, text },
                    i => i,
                })
                .collect(),
            other => layout.route(other),
        }
    }

    /// Raw mouse motion: `None` if it is the layout's (as without
    /// snapping), else what goes to the driver.
    pub fn motion(&mut self, d: (f64, f64)) -> Option<Vec<Input>> {
        self.snapper.motion(d).then(Vec::new)
    }

    /// Time `now`: the pointer moves on; what goes to the driver.
    pub fn tick(&mut self, layout: &mut Layout, scene: &scene::Scene, now: f64, rates: Rates) -> Vec<Input> {
        let actions = self.snapper.tick(now, &Self::targets(scene, layout), rates);
        self.perform(actions, layout)
    }

    /// The window lost the keyboard: no key is held any more, the mode ends.
    pub fn focus_lost(&mut self, layout: &mut Layout) -> Vec<Input> {
        self.snapper.stop();
        (self.ctrl, self.alt, self.alt_entered, self.alt_sent, self.esc) = (false, false, false, false, false);
        self.sync(layout)
    }

    /// Whether to hide the system pointer, when that changes.
    pub fn take_hide_pointer(&mut self) -> Option<bool> {
        self.hide_pointer.take()
    }

    /// The pointers as sprites in camera `overlay` (over the whole window
    /// at 1:1, on top of every view: window pixels): the ghost, then the
    /// pointer over it; the driver's arrow if the description has it.
    pub fn sprites(&self, compiled: &render_desc::Compiled, overlay: u32) -> Vec<tile_render::PlacedSprite> {
        let Some(pointer) = self.snapper.pointer() else { return vec![] };
        let set = |names: &[&str]| names.iter().find_map(|n| compiled.set(n).map(|(s, _)| s));
        let place = |set: Option<tile_render::SpriteSetId>, (x, y): Point| {
            Some(tile_render::PlacedSprite { screen: true, view: Some(overlay), ..tile_render::PlacedSprite::new(set?, x.round() as i32, y.round() as i32) })
        };
        let solid = place(set(&[ARROW_SPRITE, POINTER_SPRITE, crate::CURSOR_SPRITE]), pointer);
        // (Under the pointer it would not be seen.)
        let ghost = self.snapper.ghost().filter(|g| (g.0 - pointer.0).hypot(g.1 - pointer.1) >= 2.0).and_then(|g| place(set(&[ARROW_GHOST_SPRITE, GHOST_SPRITE]), g));
        ghost.into_iter().chain(solid).collect()
    }
}

#[cfg(test)]
mod tests;
