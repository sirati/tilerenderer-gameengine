//! Moving the map with the mouse while it is free (not captured), in every
//! view: by edge scrolling, or by grabbing the map and dragging it
//! (`viewer_settings::MapMovement`). A child of `layout`: these are
//! [`Layout`]'s methods, hooked into its pointer routing and its frame.
//!
//! # Edge scrolling
//!
//! The pointer within [`scene::EDGE_ZONE`] pixels of a window edge scrolls
//! the view it is in, if that view reaches that edge of the window; in a
//! corner both ways at once. Pushed against the divider's wall
//! (`Layout::against`) the view scrolls towards the divider as at its
//! outermost pixel. Over the driver's interface the view beneath counts
//! (the game's own screen edge works the same over its interface). Not
//! while button snapping holds the pointer.
//!
//! - **The driver's view** (A following the driver's main view, its screen
//!   not fitted into a pane, at any zoom): the driver scrolls it itself,
//!   told where the pointer is at the edge (`Input::Edge`; the Civ III
//!   bridge has the game see its pointer as far from its own edge, and the
//!   game scrolls faster nearer the edge).
//! - **The viewer's views** (B, A following anything else, insets): the
//!   viewer moves them, at [`EDGE_PAN_SPEED`] screen pixels a second at the
//!   outermost pixel down to [`EDGE_SLOWEST`] of that at the zone's inner
//!   end ([`edge_speed`]), sub-pixel steps carried over between frames.
//!
//! # Dragging
//!
//! With `MapMovement::Drag` the left button pressed on a world view is
//! held back: released within [`DRAG_THRESHOLD`] pixels it is a click
//! (press and release go on together then); moved further it grabs the
//! map, which then follows the pointer 1:1 from where it was pressed, and
//! nothing of the press reaches the driver. The middle button grabs at once,
//! in either mode. Let go, the map slides on with the pointer's velocity of
//! the last moments ([`Tracker`]) and slows with friction ([`Momentum`],
//! the deceleration of Apple's scroll views); a pointer held still before
//! it was let go does not slide, and any press on a view stops a slide. No
//! edge scrolling in this mode. Insets, A and B alike; a view following
//! the world cursor moves the world cursor (it keeps it in its middle).
//!
//! The driver's view cannot be moved by pixels from outside (Civ III
//! centres its view on a tile), so while it is dragged and slides A is
//! placed by the viewer (it shows the world itself, the driver's view
//! staying where it was), and on letting go the driver is asked to look
//! where A will come to rest (`Input::Look`, the bridge clicks the minimap
//! there). Once A rests and the driver's view moved (or after
//! [`HANDBACK_WAIT`] seconds) A follows the driver's view again: it lands
//! on the nearest place the driver can show (with smooth world movement it
//! glides the rest of the way).

use scene::{Button, EDGE_ZONE, Input};
use viewer_settings::MapMovement;

use super::{Axis, EDGE_PAN_SPEED, Follow, Hit, Layout, Side, inside};

/// The edge scrolling speed at the inner end of the edge zone, as a share
/// of [`EDGE_PAN_SPEED`] (the speed at the outermost pixel).
pub const EDGE_SLOWEST: f64 = 0.25;
/// How far (in window pixels) the pointer moves with the left button down
/// before it drags the map rather than clicks.
pub const DRAG_THRESHOLD: i32 = 4;
/// How long the pointer may rest before it is let go and the map still
/// slides on (seconds).
pub const STILL_BEFORE_RELEASE: f64 = 0.06;
/// How long A waits for the driver's view to move after a drag before it
/// follows that view again anyway (seconds).
pub const HANDBACK_WAIT: f64 = 0.5;

/// Per axis, where pointer coordinate `v` of a window `size` long is at its
/// edge, as in `Input::Edge`: 0 if not within [`EDGE_ZONE`] of an edge the
/// view reaches (`low`: the left or top one; `high`: the right or bottom
/// one), else how far in it is plus one, negative at the low edge.
pub fn edge_axis(v: i32, size: i32, low: bool, high: bool) -> i32 {
    let far = size - 1 - v;
    if v < 0 || far < 0 {
        0
    } else if low && v < EDGE_ZONE {
        -(v + 1)
    } else if high && far < EDGE_ZONE {
        far + 1
    } else {
        0
    }
}

/// How fast a view scrolls along an axis for the pointer at edge `e` of it
/// (as in `Input::Edge`), in screen pixels a second, signed: the full
/// [`EDGE_PAN_SPEED`] at the outermost pixel, falling linearly to
/// [`EDGE_SLOWEST`] of it at the zone's inner end.
pub fn edge_speed(e: i32) -> f64 {
    if e == 0 {
        return 0.0;
    }
    let from = (e.abs() - 1).min(EDGE_ZONE - 1) as f64;
    let share = 1.0 - (1.0 - EDGE_SLOWEST) * from / (EDGE_ZONE - 1).max(1) as f64;
    EDGE_PAN_SPEED * share * e.signum() as f64
}

/// Whether the pointer moved from `from` to `to` far enough to drag.
pub fn past_threshold(from: (i32, i32), to: (i32, i32)) -> bool {
    let (dx, dy) = ((to.0 - from.0) as i64, (to.1 - from.1) as i64);
    dx * dx + dy * dy > (DRAG_THRESHOLD as i64).pow(2)
}

/// The pointer's velocity while it drags (window pixels a second), as of
/// the frames shown: each frame's motion smoothed over [`Tracker::TAU`].
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Tracker {
    last: (i32, i32),
    vel: (f64, f64),
    /// How long the pointer has not moved.
    still: f64,
}

impl Tracker {
    /// The smoothing's time constant (seconds).
    pub const TAU: f64 = 0.04;
    /// The fastest slide (window pixels a second).
    pub const MAX: f64 = 8000.0;

    pub fn new(at: (i32, i32)) -> Self {
        Tracker { last: at, ..Default::default() }
    }

    /// A frame `dt` seconds after the last: the pointer is at `at`.
    pub fn sample(&mut self, at: (i32, i32), dt: f64) {
        if dt <= 0.0 {
            return;
        }
        let d = ((at.0 - self.last.0) as f64, (at.1 - self.last.1) as f64);
        let a = 1.0 - (-dt / Self::TAU).exp();
        self.vel = (self.vel.0 + (d.0 / dt - self.vel.0) * a, self.vel.1 + (d.1 / dt - self.vel.1) * a);
        self.still = if d == (0.0, 0.0) { self.still + dt } else { 0.0 };
        self.last = at;
    }

    /// The velocity the map slides on with when let go at `at`: none if
    /// the pointer rested a moment before.
    pub fn release(&self, at: (i32, i32)) -> (f64, f64) {
        if at == self.last && self.still >= STILL_BEFORE_RELEASE {
            return (0.0, 0.0);
        }
        let speed = self.vel.0.hypot(self.vel.1);
        let k = if speed > Self::MAX { Self::MAX / speed } else { 1.0 };
        (self.vel.0 * k, self.vel.1 * k)
    }
}

/// A slide after letting go: the velocity (window pixels a second, the way
/// the pointer went) decays exponentially (friction).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Momentum {
    pub vel: (f64, f64),
}

impl Momentum {
    /// The friction (1/s): Apple's normal scroll deceleration, 0.998 of the
    /// velocity kept per millisecond.
    pub const FRICTION: f64 = 2.0;
    /// Below this speed (window pixels a second) the slide stops.
    pub const STOP: f64 = 10.0;

    /// Moves on `dt` seconds: how far it went (exact for the decay).
    pub fn step(&mut self, dt: f64) -> (f64, f64) {
        let decay = (-Self::FRICTION * dt.max(0.0)).exp();
        let k = (1.0 - decay) / Self::FRICTION;
        let d = (self.vel.0 * k, self.vel.1 * k);
        self.vel = (self.vel.0 * decay, self.vel.1 * decay);
        d
    }

    pub fn moving(&self) -> bool {
        self.vel.0.hypot(self.vel.1) >= Self::STOP
    }

    /// How far it goes from here until it stops.
    pub fn distance(&self) -> (f64, f64) {
        if !self.moving() {
            return (0.0, 0.0);
        }
        (self.vel.0 / Self::FRICTION, self.vel.1 / Self::FRICTION)
    }
}

/// The left button held on a view, not yet a click or a drag.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Press {
    at: (i32, i32),
    hit: Hit,
}

/// A view grabbed.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Grab {
    hit: Hit,
    button: Button,
    /// Where it was grabbed (window), and the view's centre the pointer
    /// there holds (world, re-anchored where the world's edge stops it).
    from: (i32, i32),
    start: (f64, f64),
    /// The centre placed last.
    at: (f64, f64),
    /// World pixels per window pixel.
    scale: f64,
    tracker: Tracker,
    /// A following the driver's view: handed back to it when let go.
    look: bool,
}

/// A view sliding on after it was let go.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Coast {
    hit: Hit,
    pos: (f64, f64),
    momentum: Momentum,
    scale: f64,
}

/// A moved by hand off the driver's view, waiting to follow it again.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Handback {
    driver_at: Option<(i32, i32)>,
    waited: f64,
}

/// The state of map movement (a part of [`Layout`]).
#[derive(Clone, Debug, Default)]
pub struct Pan {
    mode: MapMovement,
    /// Whether the pointer is in the window.
    inside: bool,
    press: Option<Press>,
    grab: Option<Grab>,
    coast: Option<Coast>,
    handback: Option<Handback>,
    /// A's centre less the driver's view's while A follows it.
    main_offset: (f64, f64),
    /// The view scrolled at the edge, and the motion not yet a world pixel.
    edge_view: Option<(Hit, (i32, i32))>,
    edge_rest: (f64, f64),
    /// Replaying a held-back press as a click.
    replaying: bool,
}

fn view_name(hit: Hit) -> String {
    match hit {
        Hit::Main => "A".into(),
        Hit::Pane => "B".into(),
        Hit::Inset(k) => format!("inset {k}"),
        h => format!("{h:?}"),
    }
}

impl Layout {
    /// How the mouse moves the map (from the settings).
    pub fn set_map_movement(&mut self, m: MapMovement) {
        if self.pan.mode != m {
            log::info!("viewer: map movement: {}", if m == MapMovement::Drag { "dragging (left or middle button)" } else { "edge scrolling" });
            self.pan.mode = m;
            self.pan.press = None;
            self.pan.edge_view = None;
        }
    }

    pub fn map_movement(&self) -> MapMovement {
        self.pan.mode
    }

    /// Whether the driver scrolls the main pane itself: it follows the
    /// driver's main view (at any zoom), the driver's screen not fitted
    /// into a pane.
    fn driver_scrolls_main(&self) -> bool {
        self.main.is_some() && self.main_view.follow == Follow::Driver && self.fit.is_none() && self.shown.iter().any(|(h, _)| *h == Hit::Main)
    }

    fn driver_center(&self) -> Option<(i32, i32)> {
        self.main.map(|m| (m.x + m.viewport.0 as i32 / 2, m.y + m.viewport.1 as i32 / 2))
    }

    /// The world pixel at the middle of the view `hit` shows (last frame),
    /// and its world pixels per window pixel.
    fn shown_center(&self, hit: Hit) -> Option<((f64, f64), f64)> {
        let (_, c) = self.shown.iter().find(|(h, _)| *h == hit)?;
        let (w, h) = c.source_size();
        Some((((c.x + w as i32 / 2) as f64, (c.y + h as i32 / 2) as f64), c.zoom.1 as f64 / c.zoom.0.max(1) as f64))
    }

    /// Centre `c` of view `hit` kept off the edges of a world that does not
    /// wrap (as its camera shows it), and per axis whether it was held.
    fn clamp_view_center(&self, hit: Hit, c: (f64, f64)) -> ((f64, f64), (bool, bool)) {
        let (Some(b), Some((_, cam))) = (self.bounds, self.shown.iter().find(|(h, _)| *h == hit)) else { return (c, (false, false)) };
        let (w, h) = cam.source_size();
        let axis = |v: f64, size: i32, wrap: bool, span: u32| -> f64 {
            let (size, span) = (size as f64, span as f64);
            match wrap {
                true => v,
                false if size <= span => (size / 2.0).floor(),
                false => v.clamp((span / 2.0).floor(), size - span + (span / 2.0).floor()),
            }
        };
        let to = (axis(c.0, b.size.0, b.wrap.0, w), axis(c.1, b.size.1, b.wrap.1, h));
        (to, (to.0 != c.0, to.1 != c.1))
    }

    /// Puts the middle of view `hit` at world pixel `c` (moved by hand; a
    /// view following the world cursor moves the cursor).
    fn place_view(&mut self, hit: Hit, c: (i32, i32)) {
        match self.follow_of(hit) {
            Some(Follow::Pointer) => self.cursor = self.bounds.map_or(c, |b| b.clamp_point(c)),
            Some(_) => {
                let focus = self.focus;
                if let Some(v) = self.free_view(hit) {
                    v.moved_by_hand(c, focus);
                }
            }
            None => {}
        }
    }

    /// The view of the world the pointer at `p` is in (beneath the
    /// driver's interface and a zoom box too), if it shows one.
    fn pan_view_at(&self, p: (i32, i32)) -> Option<Hit> {
        let hit = match self.hit(p) {
            Hit::Magnifier => self.hit_below_magnifier(p),
            h => h,
        };
        let hit = match hit {
            Hit::Main | Hit::Pane | Hit::Inset(_) => hit,
            Hit::Interface | Hit::Moved(_) => {
                let (a, b) = self.panes();
                if self.split.is_some() && b.is_some_and(|r| inside(r, p)) {
                    Hit::Pane
                } else if inside(a, p) {
                    Hit::Main
                } else {
                    return None;
                }
            }
            Hit::Magnifier | Hit::Divider | Hit::Beside => return None,
        };
        self.shown.iter().any(|(h, _)| *h == hit).then_some(hit)
    }

    /// The view the pointer at `p` is in and where it is at that view's
    /// edges (as `Input::Edge`): the window's edges the view reaches, and
    /// the divider's wall it pushes against (at its outermost pixel).
    fn view_edge(&self, p: (i32, i32)) -> Option<(Hit, (i32, i32))> {
        let hit = self.pan_view_at(p)?;
        let (_, cam) = self.shown.iter().find(|(h, _)| *h == hit)?;
        let [x, y, w, h] = super::rect_of(cam);
        let (ww, wh) = (self.window.0 as i32, self.window.1 as i32);
        let mut e = (edge_axis(p.0, ww, x <= 0, x + w >= ww), edge_axis(p.1, wh, y <= 0, y + h >= wh));
        if let (Some(side), Some(split)) = (self.against, self.split) {
            if side.hit() == hit {
                let toward = if side == Side::A { 1 } else { -1 };
                match split.axis {
                    Axis::Vertical => e.0 = toward,
                    Axis::Horizontal => e.1 = toward,
                }
            }
        }
        Some((hit, e))
    }

    /// Where the pointer at `p` is at the driver's edge (`Input::Edge`):
    /// in the main pane while the driver scrolls it, edge scrolling on, the
    /// mouse free and nothing dragged.
    pub(super) fn driver_edge(&self, p: (i32, i32)) -> (i32, i32) {
        if self.pan.mode != MapMovement::EdgeScroll || self.locked || self.snapping || self.drag.is_some() || self.pan.grab.is_some() || !self.driver_scrolls_main() {
            return (0, 0);
        }
        match self.view_edge(p) {
            Some((Hit::Main, e)) => e,
            _ => (0, 0),
        }
    }

    /// The pointer left the window (or it lost the focus).
    pub(super) fn pan_pointer_gone(&mut self) {
        self.pan.inside = false;
    }

    /// A button at `p` (the mouse free): `Some` with what goes to the
    /// driver if it grabs or lets go of a view, holds back a press that may
    /// become a drag, or releases one as a click.
    pub(super) fn pan_button(&mut self, button: Button, down: bool, p: (i32, i32)) -> Option<Vec<Input>> {
        if self.pan.replaying {
            return None;
        }
        if !down {
            if self.pan.grab.is_some_and(|g| g.button == button) {
                return Some(self.let_go(p));
            }
            if button == Button::Left {
                if let Some(press) = self.pan.press.take() {
                    // A click: the press, then the release.
                    self.pan.replaying = true;
                    let mut out = self.button(button, true, press.at);
                    out.extend(self.button(button, false, p));
                    self.pan.replaying = false;
                    return Some(out);
                }
            }
            return None;
        }
        // (Button snapping clicks the interface, not the world.)
        if self.locked || self.snapping {
            return None;
        }
        let hit = self.hit(p);
        if !matches!(hit, Hit::Main | Hit::Pane | Hit::Inset(_)) {
            return None;
        }
        // A press on a view stops its slide.
        self.pan.coast = None;
        let grabbable = self.fit.map(|f| f.hit) != Some(hit) && self.shown.iter().any(|(h, _)| *h == hit);
        match button {
            Button::Middle if grabbable => {
                self.pan.press = None;
                self.grab(hit, button, p);
                Some(self.edge_change((0, 0)))
            }
            Button::Left if grabbable && self.pan.mode == MapMovement::Drag && !(self.ctrl && self.shift) && self.pan.grab.is_none() => {
                self.pan.press = Some(Press { at: p, hit });
                Some(vec![])
            }
            _ => None,
        }
    }

    /// The pointer moved to `p` (the mouse free, before the divider holds
    /// it): `Some` with what goes to the driver while a view is grabbed (a
    /// held press moved past [`DRAG_THRESHOLD`] grabs it).
    pub(super) fn pan_motion(&mut self, p: (i32, i32)) -> Option<Vec<Input>> {
        self.pan.inside = true;
        if let Some(press) = self.pan.press {
            if !past_threshold(press.at, p) {
                return None;
            }
            self.pan.press = None;
            self.grab(press.hit, Button::Left, press.at);
        }
        let g = self.pan.grab?;
        // (The divider does not hold a drag back.)
        self.pointer = p;
        self.raw_pointer = p;
        self.against = None;
        self.side = self.side_of(p).or(self.side);
        let desired = (g.start.0 - (p.0 - g.from.0) as f64 * g.scale, g.start.1 - (p.1 - g.from.1) as f64 * g.scale);
        let (at, _) = self.clamp_view_center(g.hit, desired);
        if let Some(g) = &mut self.pan.grab {
            // Held at the world's edge: the map moves back from where the
            // pointer turns.
            g.start = (g.start.0 + at.0 - desired.0, g.start.1 + at.1 - desired.1);
            g.at = at;
        }
        self.place_view(g.hit, (at.0.round() as i32, at.1.round() as i32));
        Some(self.edge_change((0, 0)))
    }

    /// Grabs view `hit` at window pixel `from`.
    fn grab(&mut self, hit: Hit, button: Button, from: (i32, i32)) {
        let Some((center, scale)) = self.shown_center(hit) else { return };
        let mut look = false;
        if hit == Hit::Main {
            if self.driver_scrolls_main() {
                // (Where A sits while it follows the driver.)
                if let Some(d) = self.driver_center() {
                    self.pan.main_offset = (center.0 - d.0 as f64, center.1 - d.1 as f64);
                }
                look = true;
            } else {
                // Grabbed again before it went back to the driver's view.
                look = self.pan.handback.take().is_some() && self.main_view.follow == Follow::Free;
            }
        }
        self.pan.coast = None;
        self.pan.edge_view = None;
        self.pan.grab = Some(Grab { hit, button, from, start: center, at: center, scale, tracker: Tracker::new(from), look });
        log::info!("viewer: {} grabbed ({button:?} button) at {from:?}", view_name(hit));
    }

    /// Lets go of the view grabbed at `p`: it slides on; A moved off the
    /// driver's view has the driver look where it comes to rest.
    fn let_go(&mut self, p: (i32, i32)) -> Vec<Input> {
        let Some(g) = self.pan.grab.take() else { return vec![] };
        let momentum = Momentum { vel: g.tracker.release(p) };
        let mut out = Vec::new();
        if momentum.moving() {
            self.pan.coast = Some(Coast { hit: g.hit, pos: g.at, momentum, scale: g.scale });
        }
        if g.look {
            let d = momentum.distance();
            let (end, _) = self.clamp_view_center(g.hit, (g.at.0 - d.0 * g.scale, g.at.1 - d.1 * g.scale));
            let (x, y) = ((end.0 - self.pan.main_offset.0).round() as i32, (end.1 - self.pan.main_offset.1).round() as i32);
            out.push(Input::Look { x, y });
            self.pan.handback = Some(Handback { driver_at: self.driver_center(), waited: 0.0 });
            log::info!("viewer: A let go, sliding at {:.0?} px/s; the driver looks at world pixel ({x}, {y})", momentum.vel);
        } else {
            log::info!("viewer: {} let go, sliding at {:.0?} px/s", view_name(g.hit), momentum.vel);
        }
        out.extend(self.edge_change(self.driver_edge(p)));
        out
    }

    /// A frame `dt` seconds after the last (the driver's screen fitted into
    /// `fitted`, if it is): the dragging pointer's velocity sampled, a
    /// sliding view moved on, A handed back to the driver's view, and the
    /// view the pointer is at the edge of scrolled.
    pub(super) fn pan_frame(&mut self, dt: f64, fitted: Option<Hit>) {
        let pointer = self.pointer;
        if let Some(g) = &mut self.pan.grab {
            g.tracker.sample(pointer, dt);
        }
        self.slide(dt);
        self.hand_back(dt);
        self.edge_scroll(dt, fitted);
    }

    fn slide(&mut self, dt: f64) {
        let Some(mut c) = self.pan.coast else { return };
        if self.locked || !self.shown.iter().any(|(h, _)| *h == c.hit) {
            self.pan.coast = None;
            return;
        }
        let d = c.momentum.step(dt);
        let (pos, held) = self.clamp_view_center(c.hit, (c.pos.0 - d.0 * c.scale, c.pos.1 - d.1 * c.scale));
        // (Stopped by the world's edge that way.)
        if held.0 {
            c.momentum.vel.0 = 0.0;
        }
        if held.1 {
            c.momentum.vel.1 = 0.0;
        }
        c.pos = pos;
        self.place_view(c.hit, (pos.0.round() as i32, pos.1.round() as i32));
        self.pan.coast = c.momentum.moving().then_some(c);
    }

    fn hand_back(&mut self, dt: f64) {
        let Some(mut h) = self.pan.handback else { return };
        if self.pan.grab.is_some_and(|g| g.hit == Hit::Main) || self.pan.coast.is_some_and(|c| c.hit == Hit::Main) {
            return;
        }
        if self.main_view.follow != Follow::Free {
            self.pan.handback = None;
            return;
        }
        h.waited += dt;
        let moved = self.driver_center() != h.driver_at;
        if moved || h.waited >= HANDBACK_WAIT {
            self.main_view.follow = Follow::Driver;
            self.main_view.hand_focus = None;
            self.pan.handback = None;
            log::info!("viewer: A follows the driver's view again ({})", if moved { "it moved there" } else { "it did not move" });
        } else {
            self.pan.handback = Some(h);
        }
    }

    fn edge_scroll(&mut self, dt: f64, fitted: Option<Hit>) {
        let active = self.pan.mode == MapMovement::EdgeScroll && self.pan.inside && !self.locked && !self.snapping && self.drag.is_none() && self.pan.grab.is_none();
        let target = self
            .view_edge(self.pointer)
            .filter(|&(hit, e)| active && e != (0, 0) && fitted != Some(hit) && !(hit == Hit::Main && self.driver_scrolls_main()));
        let sign = |(h, e): (Hit, (i32, i32))| (h, (e.0.signum(), e.1.signum()));
        if target.map(sign) != self.pan.edge_view {
            match target {
                Some((hit, e)) => log::info!("viewer: edge scrolling {} towards {:?}", view_name(hit), (e.0.signum(), e.1.signum())),
                None if self.pan.edge_view.is_some() => log::info!("viewer: edge scrolling ends"),
                None => {}
            }
            self.pan.edge_view = target.map(sign);
            self.pan.edge_rest = (0.0, 0.0);
        }
        let Some((hit, e)) = target else { return };
        if dt <= 0.0 {
            return;
        }
        let Some((center, scale)) = self.shown_center(hit) else { return };
        let rest = self.pan.edge_rest;
        let total = (rest.0 + edge_speed(e.0) * dt * scale, rest.1 + edge_speed(e.1) * dt * scale);
        let whole = (total.0.trunc(), total.1.trunc());
        self.pan.edge_rest = (total.0 - whole.0, total.1 - whole.1);
        if whole == (0.0, 0.0) {
            return;
        }
        let (to, _) = self.clamp_view_center(hit, (center.0 + whole.0, center.1 + whole.1));
        self.place_view(hit, (to.0.round() as i32, to.1.round() as i32));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_edge_zone_counts_from_the_edges_the_view_reaches() {
        // Both edges reached: 1 at the outermost pixel, 16 at the zone's end.
        assert_eq!(edge_axis(0, 800, true, true), -1);
        assert_eq!(edge_axis(15, 800, true, true), -16);
        assert_eq!(edge_axis(16, 800, true, true), 0);
        assert_eq!(edge_axis(799, 800, true, true), 1);
        assert_eq!(edge_axis(784, 800, true, true), 16);
        assert_eq!(edge_axis(783, 800, true, true), 0);
        // An edge the view does not reach (the divider's side of a pane).
        assert_eq!(edge_axis(799, 800, true, false), 0);
        assert_eq!(edge_axis(0, 800, false, true), 0);
        // Outside the window.
        assert_eq!(edge_axis(-1, 800, true, true), 0);
        assert_eq!(edge_axis(800, 800, true, true), 0);
    }

    #[test]
    fn edge_scrolling_is_faster_nearer_the_edge() {
        assert_eq!(edge_speed(0), 0.0);
        assert_eq!(edge_speed(1), EDGE_PAN_SPEED);
        assert_eq!(edge_speed(-1), -EDGE_PAN_SPEED);
        assert_eq!(edge_speed(EDGE_ZONE), EDGE_PAN_SPEED * EDGE_SLOWEST);
        assert_eq!(edge_speed(-EDGE_ZONE), -EDGE_PAN_SPEED * EDGE_SLOWEST);
        for e in 1..EDGE_ZONE {
            assert!(edge_speed(e) > edge_speed(e + 1), "{e}");
        }
    }

    #[test]
    fn a_drag_starts_past_a_few_pixels() {
        assert!(!past_threshold((100, 100), (100, 100)));
        assert!(!past_threshold((100, 100), (104, 100)));
        assert!(!past_threshold((100, 100), (97, 98)));
        assert!(past_threshold((100, 100), (105, 100)));
        assert!(past_threshold((100, 100), (100, 95)));
        assert!(past_threshold((100, 100), (104, 104)));
    }

    #[test]
    fn momentum_decays_by_friction_and_stops() {
        let mut m = Momentum { vel: (1000.0, -500.0) };
        let expected = m.distance();
        assert_eq!(expected, (500.0, -250.0));
        // A second: e^-2 of the velocity left, (1 - e^-2) of the way gone.
        let d = m.step(1.0);
        let left = (-2.0f64).exp();
        assert!((m.vel.0 - 1000.0 * left).abs() < 1e-9 && (m.vel.1 + 500.0 * left).abs() < 1e-9);
        assert!((d.0 - 500.0 * (1.0 - left)).abs() < 1e-9 && (d.1 + 250.0 * (1.0 - left)).abs() < 1e-9);
        // Frame by frame it goes the same way, and stops short of it.
        let mut m = Momentum { vel: (1000.0, -500.0) };
        let mut gone = (0.0, 0.0);
        let mut frames = 0;
        while m.moving() {
            let d = m.step(1.0 / 60.0);
            gone = (gone.0 + d.0, gone.1 + d.1);
            frames += 1;
            assert!(frames < 1000);
        }
        assert!(gone.0 < 500.0 && gone.0 > 490.0 && gone.1 > -250.0 && gone.1 < -245.0, "{gone:?}");
        // About 2.4 s at 60 frames a second.
        assert!((130..160).contains(&frames), "{frames}");
        assert_eq!(m.distance(), (0.0, 0.0));
        assert!(!Momentum { vel: (5.0, 5.0) }.moving());
    }

    #[test]
    fn the_release_velocity_is_the_pointers_of_the_last_moments() {
        let mut t = Tracker::new((0, 0));
        for k in 1..=10 {
            t.sample((10 * k, 0), 0.01);
        }
        // 10 pixels each 10 ms: close to 1000 pixels a second.
        let v = t.release((100, 0));
        assert!(v.0 > 900.0 && v.0 <= 1000.0 && v.1 == 0.0, "{v:?}");
        // Resting a moment before letting go: no slide.
        for _ in 0..7 {
            t.sample((100, 0), 0.01);
        }
        assert_eq!(t.release((100, 0)), (0.0, 0.0));
        // (Unless it moved since the last frame.)
        assert_ne!(t.release((103, 0)), (0.0, 0.0));
        // A fling has a top speed.
        let mut t = Tracker::new((0, 0));
        for k in 1..=10 {
            t.sample((1000 * k, 0), 0.01);
        }
        assert!((t.release((10000, 0)).0 - Tracker::MAX).abs() < 1e-6);
        // No time passed: no change.
        let mut t = Tracker::new((0, 0));
        t.sample((50, 0), 0.0);
        assert_eq!(t, Tracker::new((0, 0)));
    }

    // ---- In the layout ----------------------------------------------------

    use super::super::{Frame, WorldBounds};
    use scene::{Scene, View, ViewState};
    use std::time::Duration;
    use viewer_settings::{Roles, Split as S};

    const BOUNDS: Option<WorldBounds> = Some(WorldBounds { size: (8000, 4000), wrap: (true, false) });

    /// The driver's view at (`x`, 500), 800 × 600 (its middle at x + 400,
    /// 800), the interface over it.
    fn scene_at(x: i32) -> Scene {
        let main = View { camera_x: x, camera_y: 500, zoom: (1, 1), ..Default::default() };
        let ui = View { interface_only: true, ..Default::default() };
        Scene {
            views: vec![ViewState { id: 0, view: main, overlay: vec![] }, ViewState { id: 255, view: ui, overlay: vec![] }],
            focus: Some((3000, 2000)),
            shape: scene::WorldShape { cols: 62, rows: 125, wrap_x: true, wrap_y: false },
            ..Default::default()
        }
    }

    fn frame(l: &mut Layout) -> Frame {
        l.frame(&scene_at(1000), (800, 600), (0, 0, 0, 0), vec![], BOUNDS)
    }

    /// `ms` milliseconds on, a frame.
    fn tick(l: &mut Layout, ms: u64) -> Frame {
        l.advance(Duration::from_millis(ms));
        frame(l)
    }

    fn mv(l: &mut Layout, p: (i32, i32)) -> Vec<Input> {
        l.route(Input::MouseMove { x: p.0, y: p.1 })
    }

    fn button(l: &mut Layout, button: Button, down: bool, p: (i32, i32)) -> Vec<Input> {
        l.route(Input::MouseButton { button, down, x: p.0, y: p.1 })
    }

    fn left(l: &mut Layout, down: bool, p: (i32, i32)) -> Vec<Input> {
        button(l, Button::Left, down, p)
    }

    /// A and B side by side (A 0..400, B 404..800), roles `roles`.
    fn split(movement: MapMovement, roles: Roles) -> Layout {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles, ..Default::default() });
        l.set_map_movement(movement);
        frame(&mut l);
        frame(&mut l);
        l
    }

    fn b(l: &Layout) -> (i32, i32) {
        l.split.unwrap().view.center
    }

    fn edges(out: &[Input]) -> Vec<(i32, i32)> {
        out.iter().filter_map(|i| if let Input::Edge { x, y } = i { Some((*x, *y)) } else { None }).collect()
    }

    fn presses(out: &[Input]) -> usize {
        out.iter().filter(|i| matches!(i, Input::MouseButton { .. } | Input::WorldButton { .. })).count()
    }

    #[test]
    fn every_view_scrolls_at_the_window_edges_and_corners_it_reaches() {
        let mut l = split(MapMovement::EdgeScroll, Roles::Free);
        let start = b(&l);
        // B (the viewer's own) at the right edge: the viewer scrolls it,
        // 600 pixels a second at the outermost pixel; the driver hears of
        // no edge.
        assert!(edges(&mv(&mut l, (799, 300))).is_empty());
        tick(&mut l, 100);
        assert_eq!(b(&l), (start.0 + 60, start.1));
        // Its top-right corner: diagonally.
        mv(&mut l, (799, 0));
        tick(&mut l, 100);
        assert_eq!(b(&l), (start.0 + 120, start.1 - 60));
        // Slower further in: 15 pixels in, a quarter of that.
        mv(&mut l, (784, 300));
        tick(&mut l, 100);
        assert_eq!(b(&l), (start.0 + 135, start.1 - 60));
        // Its bottom edge (its left one is the divider's, no window edge).
        mv(&mut l, (406, 599));
        tick(&mut l, 100);
        assert_eq!(b(&l), (start.0 + 135, start.1));
        mv(&mut l, (600, 300));
        tick(&mut l, 100);
        assert_eq!(b(&l), (start.0 + 135, start.1));
        assert_eq!(l.split.unwrap().view.follow, Follow::Free);
        // A (the driver's view): the driver scrolls it, at its left, top and
        // bottom edges and corners; the viewer leaves it be.
        let a = l.main_view;
        mv(&mut l, (300, 300));
        assert_eq!(edges(&mv(&mut l, (0, 300))), [(-1, 0)]);
        assert_eq!(edges(&mv(&mut l, (0, 0))), [(-1, -1)]);
        assert_eq!(edges(&mv(&mut l, (3, 599))), [(-4, 1)]);
        assert_eq!(edges(&mv(&mut l, (200, 599))), [(0, 1)]);
        tick(&mut l, 100);
        assert_eq!((l.main_view.center, l.main_view.follow), (a.center, Follow::Driver));
        // Its right side is the divider's, open in the middle: no edge.
        assert_eq!(edges(&mv(&mut l, (399, 300))), [(0, 0)]);
        // Gone from the window: nothing scrolls.
        mv(&mut l, (799, 300));
        l.pointer_gone();
        let at = b(&l);
        tick(&mut l, 100);
        assert_eq!(b(&l), at);
    }

    #[test]
    fn a_view_of_the_viewers_own_in_a_scrolls_itself_and_pushing_the_divider_adds_to_an_edge() {
        // A follows the unit: the viewer scrolls it at the window's corner
        // (until the unit moves); the driver hears of no edge.
        let mut l = split(MapMovement::EdgeScroll, Roles::UnitMouse);
        assert_eq!(l.main_view.center, (3000, 2000));
        assert!(edges(&mv(&mut l, (0, 0))).iter().all(|&e| e == (0, 0)));
        tick(&mut l, 100);
        assert_eq!((l.main_view.center, l.main_view.follow), ((2940, 1940), Follow::Focus));
        // B pushed against the divider's lower wall, near the bottom edge:
        // towards A at full speed, down at 10 pixels from the edge.
        let mut l = split(MapMovement::EdgeScroll, Roles::Free);
        let start = b(&l);
        mv(&mut l, (600, 590));
        mv(&mut l, (100, 590));
        assert_eq!(l.pointer, (404, 590));
        tick(&mut l, 100);
        assert_eq!(b(&l), (start.0 - 60, start.1 + 33));
    }

    #[test]
    fn a_zoomed_view_following_the_driver_is_scrolled_by_the_driver() {
        let mut l = Layout::default();
        l.main_view.zoom = super::super::ONE + 2;
        frame(&mut l);
        frame(&mut l);
        assert_eq!(edges(&mv(&mut l, (790, 595))), [(10, 5)]);
        tick(&mut l, 100);
        assert_eq!(l.main_view.follow, Follow::Driver);
    }

    #[test]
    fn dragging_takes_no_edges_and_a_short_click_still_clicks() {
        let mut l = split(MapMovement::Drag, Roles::Free);
        let start = b(&l);
        // No edge scrolling at all.
        assert!(edges(&mv(&mut l, (799, 300))).is_empty());
        tick(&mut l, 100);
        assert_eq!(b(&l), start);
        mv(&mut l, (100, 300));
        assert!(edges(&mv(&mut l, (0, 300))).is_empty());
        // A click in A: held back until it is one, then pressed and
        // released where it was made (A shows the middle of the driver's
        // view in its middle: 200 pixels on in the driver's window); moved
        // a little, still a click there.
        mv(&mut l, (100, 100));
        assert_eq!(left(&mut l, true, (100, 100)), []);
        let press = |down, x, y| Input::MouseButton { button: Button::Left, down, x, y };
        assert_eq!(left(&mut l, false, (100, 100)), [press(true, 300, 100), press(false, 300, 100)]);
        assert_eq!(left(&mut l, true, (100, 100)), []);
        assert_eq!(mv(&mut l, (103, 102)), [Input::MouseMove { x: 303, y: 102 }]);
        assert_eq!(left(&mut l, false, (103, 102)), [press(true, 300, 100), press(false, 303, 102)]);
        // In B: where the driver's view shows the same world pixel.
        mv(&mut l, (600, 300));
        assert_eq!(left(&mut l, true, (600, 300)), []);
        let out = left(&mut l, false, (600, 300));
        let at = |down| Input::MouseButton { button: Button::Left, down, x: 398, y: 300 };
        assert_eq!(out, [at(true), at(false)]);
        // Further off, where it does not: looked at, and the press and
        // release on that world pixel.
        let v = &mut l.split.as_mut().unwrap().view;
        (v.center, v.settling) = ((5000, 2000), false);
        frame(&mut l);
        assert_eq!(left(&mut l, true, (600, 300)), []);
        let out = left(&mut l, false, (600, 300));
        assert!(matches!(out[..], [Input::Look { .. }, Input::WorldButton { down: true, .. }, Input::WorldButton { down: false, .. }]), "{out:?}");
        assert_eq!(l.map_movement(), MapMovement::Drag);
    }

    #[test]
    fn a_dragged_view_follows_the_pointer_one_to_one_and_slides_on_when_let_go() {
        let mut l = split(MapMovement::Drag, Roles::Free);
        let start = b(&l);
        let mut out = mv(&mut l, (600, 300));
        out.extend(left(&mut l, true, (600, 300)));
        // Past the threshold: grabbed where it was pressed.
        out.extend(mv(&mut l, (590, 300)));
        assert_eq!(b(&l), (start.0 + 10, start.1));
        // Pulled left 10 pixels a frame (16 ms): the map goes right with it.
        for k in 2..=12 {
            out.extend(mv(&mut l, (600 - 10 * k, 300 + k)));
            assert_eq!(b(&l), (start.0 + 10 * k, start.1 - k));
            tick(&mut l, 16);
        }
        out.extend(left(&mut l, false, (480, 312)));
        assert_eq!(presses(&out), 0, "{out:?}");
        // It slides on (about 600 pixels a second), slower and slower.
        let mut at = b(&l);
        let mut last_step = i32::MAX;
        let mut frames = 0;
        loop {
            tick(&mut l, 16);
            let now = b(&l);
            let step = now.0 - at.0;
            // (Rounded to whole pixels: a pixel more now and then.)
            assert!(step <= last_step.saturating_add(1) && step >= 0 && now.1 <= at.1, "{at:?} -> {now:?}");
            at = now;
            last_step = step;
            frames += 1;
            if l.pan.coast.is_none() {
                break;
            }
            assert!(frames < 500);
        }
        let slid = at.0 - (start.0 + 120);
        assert!((200..=320).contains(&slid), "{slid}");
        assert!(frames > 60, "{frames}");
        // A press on it stops a slide.
        left(&mut l, true, (600, 300));
        for k in 1..=5 {
            mv(&mut l, (600 - 20 * k, 300));
            tick(&mut l, 16);
        }
        left(&mut l, false, (500, 300));
        assert!(l.pan.coast.is_some());
        tick(&mut l, 16);
        let out = left(&mut l, true, (500, 300));
        assert_eq!(out, []);
        assert!(l.pan.coast.is_none());
        let at = b(&l);
        tick(&mut l, 16);
        assert_eq!(b(&l), at);
        left(&mut l, false, (500, 300));
        // Held still before letting go: no slide.
        left(&mut l, true, (500, 300));
        mv(&mut l, (450, 300));
        tick(&mut l, 16);
        for _ in 0..6 {
            tick(&mut l, 16);
        }
        left(&mut l, false, (450, 300));
        assert!(l.pan.coast.is_none());
    }

    #[test]
    fn world_scaling_keeps_dragging_and_edge_scrolling_in_window_pixels() {
        for scale in [(2, 1), (3, 2), (1, 2)] {
            let mut l = split(MapMovement::Drag, Roles::Free);
            l.world_scale = scale;
            frame(&mut l);
            let start = b(&l);
            left(&mut l, true, (600, 300));
            mv(&mut l, (450, 225));
            let world_per_pixel = scale.1 as f64 / scale.0 as f64;
            let dragged = ((150.0 * world_per_pixel).round() as i32, (75.0 * world_per_pixel).round() as i32);
            assert_eq!(b(&l), (start.0 + dragged.0, start.1 + dragged.1), "{scale:?}");
            left(&mut l, false, (450, 225));

            let mut l = split(MapMovement::EdgeScroll, Roles::Free);
            l.world_scale = scale;
            frame(&mut l);
            let start = b(&l);
            mv(&mut l, (799, 300));
            tick(&mut l, 100);
            assert_eq!(b(&l), (start.0 + (60.0 * world_per_pixel).round() as i32, start.1), "{scale:?}");
            // The driver receives edge distances in window pixels even
            // when its world is shown at a different scale. Cross the
            // divider through its open middle before entering A's corner.
            mv(&mut l, (100, 300));
            assert_eq!(edges(&mv(&mut l, (3, 0))), [(-4, -1)], "{scale:?}");
        }
    }

    #[test]
    fn dragging_the_drivers_view_has_the_driver_look_where_it_comes_to_rest() {
        let mut l = Layout::default();
        l.set_map_movement(MapMovement::Drag);
        frame(&mut l);
        frame(&mut l);
        assert_eq!(l.main_view.center, (1400, 800));
        left(&mut l, true, (400, 300));
        mv(&mut l, (300, 280));
        // A moves 1:1, placed by the viewer meanwhile.
        assert_eq!((l.main_view.center, l.main_view.follow), ((1500, 820), Follow::Free));
        assert_eq!(left(&mut l, false, (300, 280)), [Input::Look { x: 1500, y: 820 }]);
        // It stays there until the driver's view moved there, then follows
        // that again.
        tick(&mut l, 100);
        assert_eq!(l.main_view.center, (1500, 820));
        l.advance(Duration::from_millis(16));
        l.frame(&scene_at(1128), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.main_view.follow, Follow::Driver);
        l.frame(&scene_at(1128), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.main_view.center, (1528, 800));
        // The driver does not move (e.g. a menu is open): back to its view
        // after a moment.
        let mut l = Layout::default();
        l.set_map_movement(MapMovement::Drag);
        frame(&mut l);
        left(&mut l, true, (400, 300));
        mv(&mut l, (390, 300));
        left(&mut l, false, (390, 300));
        for _ in 0..4 {
            tick(&mut l, 100);
        }
        assert_eq!((l.main_view.center, l.main_view.follow), ((1410, 800), Follow::Free));
        tick(&mut l, 100);
        tick(&mut l, 16);
        assert_eq!((l.main_view.center, l.main_view.follow), ((1400, 800), Follow::Driver));
    }

    #[test]
    fn the_middle_button_drags_in_either_mode() {
        for m in [MapMovement::EdgeScroll, MapMovement::Drag] {
            let mut l = split(m, Roles::Free);
            let start = b(&l);
            assert_eq!(button(&mut l, Button::Middle, true, (600, 300)), []);
            assert!(mv(&mut l, (601, 300)).iter().all(|i| matches!(i, Input::Edge { .. })));
            assert_eq!(b(&l), (start.0 - 1, start.1));
            button(&mut l, Button::Middle, false, (601, 300));
        }
    }

    #[test]
    fn a_view_following_the_world_cursor_drags_the_cursor_and_a_click_still_captures() {
        let mut l = split(MapMovement::Drag, Roles::UnitMouse);
        // (Once the driver's view settled, the world cursor stays put.)
        for _ in 0..=super::super::SETTLE_FRAMES {
            frame(&mut l);
        }
        let cursor = l.cursor;
        mv(&mut l, (600, 300));
        left(&mut l, true, (600, 300));
        mv(&mut l, (650, 290));
        left(&mut l, false, (650, 290));
        assert!(!l.captured());
        tick(&mut l, 16);
        assert_eq!(b(&l), (cursor.0 - 50, cursor.1 + 10));
        // A click captures the mouse; captured, nothing is dragged.
        mv(&mut l, (600, 300));
        left(&mut l, true, (600, 300));
        left(&mut l, false, (600, 300));
        assert!(l.captured());
        tick(&mut l, 16);
        left(&mut l, true, (600, 300));
        assert!(l.pan.press.is_none() && l.pan.grab.is_none());
        assert!(mv(&mut l, (700, 300)).is_empty());
        left(&mut l, false, (700, 300));
        assert!(l.captured());
    }
}
