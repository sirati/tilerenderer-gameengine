//! Moving the map with the mouse while it is free (not captured), in every
//! view: by edge scrolling, or by grabbing the map and dragging it
//! (`viewer_settings::MapMovement`). A child of `layout`: these are
//! [`Layout`]'s methods, hooked into its pointer routing and its frame.
//!
//! # Edge scrolling
//!
//! The pointer within [`EDGE_ZONE`] pixels of a window edge scrolls
//! the view it is in, if that view reaches that edge of the window; in a
//! corner both ways at once. Pushed against the divider's wall
//! (`Layout::against`) the view scrolls towards the divider as at its
//! outermost pixel. Over the driver's interface the view beneath counts
//! (the game's own screen edge works the same over its interface). Not
//! while button snapping holds the pointer.
//!
//! Every world view uses the same viewer clock and speed curve: the full
//! speed at the outermost pixel, easing (a smoothstep) to nothing at the
//! zone's inner end ([`edge_share`]). The full speed is the game's own at
//! its outermost pixel: its wheel step (`Scene::map_wheel_step`, which
//! follows the game's scroll speed setting and zoom) every
//! [`GAME_EDGE_TICK`], in world pixels whatever the view's zoom or the
//! world's scale, so the map passes as many tiles a second as the game's
//! edge scrolling moves it. A driver without that step scrolls at
//! [`EDGE_PAN_SPEED`] window pixels a second. Smooth world movement or
//! not, the same: the viewer moves by continuous pixels each frame, with
//! fractional steps carried over between frames. The driver's tile-based
//! edge scrolling stays disabled.
//!
//! The pointer pushed past a window edge (a drag, a warp landing a pixel
//! out, a clamp) is on that edge's outermost pixel, up to
//! `edge_clock::PUSH_MARGIN` out: pushing further into an edge never slows
//! or stops the scrolling. In the window the scrolling runs on the pointer
//! thread's clock (`edge_clock`, `pointer_thread`): it samples the pointer
//! at twice the display's rate and adds up how far each view scrolls; a
//! frame moves the view by what that grew since the last frame (as of one
//! tick before the frame, interpolated, so frames at any phase to the ticks
//! move by exactly their own time). Without that thread (tests, headless
//! rendering) each frame scrolls by its own time here.
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
//! edge scrolling in this mode except while a held right drag previews a
//! go-to order. That preview scrolls at the edge until release; a
//! separate left click confirms it. `world_orders` owns that gesture.
//! Insets, A and B alike; a view following
//! the world cursor moves the world cursor (it keeps it in its middle).
//!
//! # Continuous positions
//!
//! Panning keeps each view's centre as a continuous world position
//! ([`Exact`]) and draws the view at its nearest whole pixel. A drag, a
//! slide, edge scrolling and the wheel all go on from that continuous
//! position while the view is still where panning drew it, never from the
//! pixel the last frame showed, so rounding cannot build up or feed back
//! over chained pans. A view moved by something else (a game camera
//! request, a glide, a popup) is taken over once from where that put it
//! (a glide's own continuous position). Nothing goes back to the game.
//! While a gesture moves a view, what the view follows (the selected piece,
//! a standing game request, the driver's camera) leaves it alone
//! ([`Layout::pan_holds`]); otherwise the two would pull it in turn.
//!
//! Panning A while it follows the driver preserves its exact pixel centre.
//! It follows again when the driver requests a new semantic camera focus,
//! or moves its native camera if it supplies no semantic metadata. Panning
//! does not recenter the driver's camera or quantize A to a tile on release.

use scene::{Button, Input};
use viewer_settings::MapMovement;

use super::{Axis, EDGE_PAN_SPEED, Follow, Hit, Layout, Side, inside};

use crate::edge_clock::edge_share;
#[cfg(test)]
use crate::edge_clock::{EDGE_ZONE, edge_axis};
/// The game's edge scrolling timer (`Main_Screen_Form::scroll_at_edge`,
/// 0x4DE4F0 in PcGames): at its view's outermost pixel it moves the map by
/// its wheel step (`Scene::map_wheel_step`) every this many seconds.
pub const GAME_EDGE_TICK: f64 = 0.066;
/// How far (in window pixels) the pointer moves with the left button down
/// before it drags the map rather than clicks.
pub const DRAG_THRESHOLD: i32 = 4;
/// How long the pointer may rest before it is let go and the map still
/// slides on (seconds).
pub const STILL_BEFORE_RELEASE: f64 = 0.06;

/// How fast a view scrolls along an axis for the pointer at edge `e` of
/// it, in window pixels a second, signed, for a driver that does not tell
/// its own speed ([`EDGE_PAN_SPEED`] at the outermost pixel).
pub fn edge_speed(e: i32) -> f64 {
    EDGE_PAN_SPEED * edge_share(e)
}

/// The game's own edge scrolling speed at its outermost pixel, in world
/// pixels a second per axis, for its wheel step `step`; `None` without one.
pub fn game_edge_speed(step: (i32, i32)) -> Option<(f64, f64)> {
    (step != (0, 0)).then(|| (step.0 as f64 / GAME_EDGE_TICK, step.1 as f64 / GAME_EDGE_TICK))
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
}

/// A view sliding on after it was let go.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Coast {
    hit: Hit,
    pos: (f64, f64),
    momentum: Momentum,
    scale: f64,
}

/// Where panning last put a view: its continuous centre (world pixels,
/// not rounded), and the whole pixel the view was drawn at for it. The
/// next pan of that view goes on from `exact` while the view is still at
/// `drawn`; once something else moved it, from where that put it.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Exact {
    hit: Hit,
    exact: (f64, f64),
    drawn: (i32, i32),
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
    /// The native camera where a manual A pan began. A keeps its exact
    /// centre until the game itself next moves that camera.
    manual_driver: Option<(i32, i32)>,
    /// The view scrolled at the edge (and which way).
    edge_view: Option<(Hit, (i32, i32))>,
    /// The continuous centres of the views panning placed last.
    exact: Vec<Exact>,
    /// Replaying a held-back press as a click.
    replaying: bool,
    /// A view flying to a place picked on the minimap (smooth movement):
    /// its spring, from its continuous centre, and where it goes.
    flight: Option<Flight>,
    /// The right button went down on the minimap (B there): it drags B
    /// while held, and its release is ours too.
    minimap_right: bool,
    /// How far the pointer thread had scrolled each view (`edge_clock`
    /// `Totals`) when a frame last moved it by that.
    edge_taken: Vec<(u32, (f64, f64))>,
}

/// A view flying to `to` ([`Layout::minimap_button`]).
#[derive(Copy, Clone, Debug, PartialEq)]
struct Flight {
    hit: Hit,
    glide: super::Glide,
    to: (i32, i32),
}

impl Pan {
    /// The live map movement after a frame prepared from `base` moved it
    /// on to `frame` (`prepared_motion`): field by field, as input left it
    /// where input changed it, else the frame's. A gesture's own fields
    /// too: a flight input turned elsewhere flies on from where the frame
    /// got it, a drag the pointer moves keeps the frame's velocity samples.
    pub(super) fn carry(&mut self, base: &Pan, frame: Pan) {
        use super::prepared_motion::{carry, carry_fields, carry_keyed, carry_some};
        let Pan { mode, inside, press, grab, coast, manual_driver, edge_view, exact, replaying, flight, minimap_right, edge_taken } = frame;
        carry(&mut self.mode, &base.mode, mode);
        carry(&mut self.inside, &base.inside, inside);
        carry(&mut self.press, &base.press, press);
        carry_some(&mut self.grab, &base.grab, grab, |l, b, f| carry_fields!(Grab { hit, button, from, start, at, scale, tracker }, l, b, f));
        carry_some(&mut self.coast, &base.coast, coast, |l, b, f| carry_fields!(Coast { hit, pos, momentum, scale }, l, b, f));
        carry(&mut self.manual_driver, &base.manual_driver, manual_driver);
        carry(&mut self.edge_view, &base.edge_view, edge_view);
        carry_keyed(&mut self.exact, &base.exact, exact, |e| e.hit);
        carry(&mut self.replaying, &base.replaying, replaying);
        carry_some(&mut self.flight, &base.flight, flight, |l, b, f| carry_fields!(Flight { hit, glide, to }, l, b, f));
        carry(&mut self.minimap_right, &base.minimap_right, minimap_right);
        carry(&mut self.edge_taken, &base.edge_taken, edge_taken);
    }
}

/// The key of view `hit` for the pointer thread (`edge_clock::EdgeTarget::view`).
fn view_key(hit: Hit) -> Option<u32> {
    match hit {
        Hit::Main => Some(0),
        Hit::Pane => Some(1),
        Hit::Inset(k) => u32::try_from(k).ok()?.checked_add(2),
        _ => None,
    }
}

/// The view of key `view` ([`view_key`]).
fn hit_of_view(view: u32) -> Option<Hit> {
    match view {
        0 => Some(Hit::Main),
        1 => Some(Hit::Pane),
        k => Some(Hit::Inset(usize::try_from(k - 2).ok()?)),
    }
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
    /// End map gestures and momentum when the driver leaves its world.
    pub(super) fn clear_world_motion(&mut self) {
        // (What the pointer thread scrolled was taken: not again.)
        self.pan = Pan { mode: self.pan.mode, edge_taken: std::mem::take(&mut self.pan.edge_taken), ..Default::default() };
        if let Some(clock) = &self.edge_clock {
            clock.aim(None);
        }
    }

    /// How the mouse moves the map (from the settings).
    pub fn set_map_movement(&mut self, m: MapMovement) {
        if self.pan.mode != m {
            self.cancel_world_order();
            log::info!("viewer: map movement: {}", if m == MapMovement::Drag { "dragging (left or middle button)" } else { "edge scrolling" });
            self.pan.mode = m;
            self.pan.press = None;
            self.pan.edge_view = None;
        }
    }

    pub fn map_movement(&self) -> MapMovement {
        self.pan.mode
    }

    pub(super) fn pan_has_gesture(&self) -> bool {
        self.pan.press.is_some() || self.pan.grab.is_some()
    }

    /// Whether `button` holds a view grabbed (a drag, not a click).
    pub(super) fn pan_grabbed(&self, button: Button) -> bool {
        self.pan.grab.is_some_and(|g| g.button == button)
    }

    /// Forgets a left press that may have become a drag: it was a click
    /// taken by someone else (`world_orders`), never replayed to the driver.
    pub(super) fn pan_forget_press(&mut self) {
        self.pan.press = None;
    }

    fn driver_center(&self) -> Option<(i32, i32)> {
        self.main.map(|m| (m.x + m.viewport.0 as i32 / 2, m.y + m.viewport.1 as i32 / 2))
    }

    /// The world pixels per window pixel of the view `hit` (as the last
    /// frame showed it). Where it is comes from [`Layout::exact_center`],
    /// never from the snapped camera.
    fn shown_scale(&self, hit: Hit) -> Option<f64> {
        let (_, c) = self.shown.iter().find(|(h, _)| *h == hit)?;
        Some(c.zoom.1 as f64 / c.zoom.0.max(1) as f64)
    }

    /// Centre `c` of view `hit` kept off the edges of a world that does not
    /// wrap (as its camera shows it), and per axis whether it was held.
    fn clamp_view_center(&self, hit: Hit, c: (f64, f64)) -> ((f64, f64), (bool, bool)) {
        let (Some(b), Some((_, cam))) = (self.bounds, self.shown.iter().find(|(h, _)| *h == hit)) else { return (c, (false, false)) };
        let (w, h) = cam.world_size();
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
    /// view following the world cursor moves the cursor). A following the
    /// driver keeps that mode, but ignores the unchanged native camera.
    pub(super) fn place_view(&mut self, hit: Hit, c: (i32, i32)) {
        let driver = hit == Hit::Main && self.main_view.follow == Follow::Driver;
        if driver && self.pan.manual_driver.is_none() {
            self.pan.manual_driver = self.driver_center();
        }
        match self.follow_of(hit) {
            Some(Follow::Pointer) => {
                self.cursor = self.bounds.map_or(c, |b| b.clamp_point(c));
                // (Placed by hand: no longer put where the driver's view
                // settles, as a free view moved by hand is not.)
                self.cursor_placed = true;
            }
            Some(_) => {
                let focus = self.focus;
                if let Some(v) = self.free_view(hit) {
                    v.moved_by_hand(c, focus);
                    if driver { v.follow = Follow::Driver; }
                }
            }
            None => {}
        }
    }

    /// The whole world pixel view `hit` is centred on now (a view following
    /// the world cursor: the cursor).
    fn view_point(&self, hit: Hit) -> Option<(i32, i32)> {
        match self.follow_of(hit)? {
            Follow::Pointer => Some(self.cursor),
            _ => match hit {
                Hit::Main => Some(self.main_view.center),
                Hit::Pane => self.split.map(|s| s.view.center),
                Hit::Inset(k) => self.insets.get(k).map(|i| i.view.center),
                _ => None,
            },
        }
    }

    /// The continuous centre a pan of view `hit` starts from. While the view
    /// is still where panning drew it, that pan's own exact centre: never
    /// re-derived from the rounded pixel, so rounding cannot build up over
    /// pans. Moved since by something else (a game focus request, a popup,
    /// following the driver), it is taken over once from there: from a
    /// glide's own continuous position if it is that glide's pixel, else
    /// from the whole pixel. Nothing flows back from here to the game.
    fn exact_center(&self, hit: Hit) -> Option<(f64, f64)> {
        let now = self.view_point(hit)?;
        if let Some(e) = self.pan.exact.iter().find(|e| e.hit == hit && e.drawn == now) {
            return Some(e.exact);
        }
        let glide = match (self.follow_of(hit)?, hit) {
            (Follow::Pointer, _) => None,
            (_, Hit::Main) => Some(self.main_view.glide),
            (_, Hit::Pane) => self.split.map(|s| s.view.glide),
            (_, Hit::Inset(k)) => self.insets.get(k).map(|i| i.view.glide),
            _ => None,
        };
        Some(match glide {
            Some(g) if g.placed && (g.pos.0.round() as i32, g.pos.1.round() as i32) == now => g.pos,
            _ => (now.0 as f64, now.1 as f64),
        })
    }

    /// Puts the middle of view `hit` at continuous world position `c` (by
    /// hand): drawn at its nearest whole pixel, `c` itself kept for the
    /// next pan ([`Layout::exact_center`]).
    fn place_exact(&mut self, hit: Hit, c: (f64, f64)) {
        self.place_view(hit, (c.0.round() as i32, c.1.round() as i32));
        self.pan.exact.retain(|e| e.hit != hit);
        if let Some(drawn) = self.view_point(hit) {
            self.pan.exact.push(Exact { hit, exact: c, drawn });
        }
    }

    /// Whether a pan gesture moves view `hit` now (grabbed, sliding or
    /// scrolled at the edge): what the view follows leaves it alone
    /// meanwhile (`Layout::frame`), so the two cannot pull it in turn.
    pub(super) fn pan_holds(&self, hit: Hit) -> bool {
        self.pan.grab.is_some_and(|g| g.hit == hit)
            || self.pan.coast.is_some_and(|c| c.hit == hit)
            || self.pan.edge_view.is_some_and(|(h, _)| h == hit)
            || self.pan.flight.is_some_and(|f| f.hit == hit)
    }

    /// The right button at `p` on the driver's minimap with B shown: B goes
    /// to the world pixel picked there (flying, with smooth movement, else
    /// at once), through its continuous centre, and neither the press nor
    /// its release reach the driver; held, moving over the minimap drags B
    /// along ([`Layout::minimap_drag`]). B keeps that place as a hand pan
    /// does; the game's camera does not pull it back. The left button stays
    /// the game's: its minimap moves the game's camera, which A shows. Without
    /// B (no split), over a view, captured or snapping: not ours (the
    /// driver's as ever).
    pub(super) fn minimap_button(&mut self, button: Button, down: bool, p: (i32, i32)) -> bool {
        if button != Button::Right {
            return false;
        }
        if !down {
            return std::mem::take(&mut self.pan.minimap_right);
        }
        let Some(to) = self.minimap_b_target(p) else { return false };
        self.pan.coast = self.pan.coast.filter(|c| c.hit != Hit::Pane);
        self.pan.grab = self.pan.grab.filter(|g| g.hit != Hit::Pane);
        self.pan.minimap_right = true;
        log::info!("viewer: B to world pixel {to:?} (right click on the minimap)");
        self.send_b(to);
        true
    }

    /// The pointer moved to `p` with the right button held since a press on
    /// the minimap: B follows it over the minimap (off it, B stays).
    pub(super) fn minimap_drag(&mut self, p: (i32, i32)) {
        if !self.pan.minimap_right {
            return;
        }
        if let Some(to) = self.minimap_b_target(p) {
            self.send_b(to);
        }
    }

    /// Where B goes for the minimap at `p`: its centre clamped to the world,
    /// if B is shown, the pointer is free and `p` is on the minimap.
    fn minimap_b_target(&self, p: (i32, i32)) -> Option<(i32, i32)> {
        if self.locked || self.snapping || self.split.is_none() || !self.shown.iter().any(|(h, _)| *h == Hit::Pane) {
            return None;
        }
        let minimap = self.minimap?;
        if !matches!(self.hit(p), Hit::Interface | Hit::Moved(_)) {
            return None;
        }
        let world = self.route_free(p).ok().and_then(|q| minimap.world_at(q))?;
        self.exact_center(Hit::Pane)?;
        // (Not past the edge of a world that does not wrap.)
        let (to, _) = self.clamp_view_center(Hit::Pane, (world.0 as f64, world.1 as f64));
        Some((to.0.round() as i32, to.1.round() as i32))
    }

    /// B to world pixel `to`: a flight with smooth movement (one under way
    /// turns there), else at once.
    fn send_b(&mut self, to: (i32, i32)) {
        let Some(from) = self.exact_center(Hit::Pane) else { return };
        if self.smooth {
            if let Some(f) = self.pan.flight.as_mut().filter(|f| f.hit == Hit::Pane) {
                f.to = to;
                return;
            }
            let glide = super::Glide { pos: from, vel: (0.0, 0.0), placed: true };
            self.pan.flight = Some(Flight { hit: Hit::Pane, glide, to });
            self.place_exact(Hit::Pane, from);
        } else {
            self.pan.flight = None;
            self.place_exact(Hit::Pane, (to.0 as f64, to.1 as f64));
        }
    }

    /// A minimap flight `dt` seconds on (the short way round a wrapping
    /// world), ending where it lands; any press on its view ends it.
    fn fly(&mut self, dt: f64) {
        let Some(mut f) = self.pan.flight else { return };
        if self.locked || !self.shown.iter().any(|(h, _)| *h == f.hit) {
            self.pan.flight = None;
            return;
        }
        f.glide.step(f.to, dt, self.world_period);
        self.place_exact(f.hit, f.glide.pos);
        self.pan.flight = (!f.glide.settled(f.to)).then_some(f);
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
            Hit::Magnifier | Hit::Divider | Hit::Beside | Hit::Screen => return None,
        };
        self.shown.iter().any(|(h, _)| *h == hit).then_some(hit)
    }

    /// The pointer at `p`, pushed back onto the window if it is past an
    /// edge by no more than `edge_clock::PUSH_MARGIN`: pushed into an edge
    /// it is on that edge.
    fn pushed_in(&self, p: (i32, i32)) -> Option<(i32, i32)> {
        let (ww, wh) = (self.window.0 as i32, self.window.1 as i32);
        Some((crate::edge_clock::pushed_onto(p.0, ww)?, crate::edge_clock::pushed_onto(p.1, wh)?))
    }

    /// The view the pointer at `p` is in and where it is at that view's
    /// edges (as `Input::Edge`): the window's edges the view reaches, and
    /// the divider's wall it pushes against (at its outermost pixel); only
    /// the game's thin zone over the interface. Pushed past the window's
    /// edge, it is on that edge.
    pub(super) fn view_edge(&self, p: (i32, i32)) -> Option<(Hit, (i32, i32))> {
        let p = self.pushed_in(p)?;
        let hit = self.pan_view_at(p)?;
        Some((hit, self.edge_target(hit)?.edge_of(p)))
    }

    /// How view `hit` meets the window's edges, for finding where a pointer
    /// in it is at them ([`crate::edge_clock::EdgeTarget`]).
    fn edge_target(&self, hit: Hit) -> Option<crate::edge_clock::EdgeTarget> {
        let (_, cam) = self.shown.iter().find(|(h, _)| *h == hit)?;
        let rect = super::rect_of(cam);
        let [x, y, _, _] = rect;
        // A top strip reserves the window's top. The first map row below
        // it (right below the strip's divider, `Layout::dividers`) becomes
        // the upward scroll boundary for the affected pane.
        let top_row = self.dividers().strip.filter(|band| y == band.across.1).map(|band| band.along);
        let against = match (self.against, self.split) {
            (Some(side), Some(split)) if side.hit() == hit => {
                let toward = if side == Side::A { 1 } else { -1 };
                Some((if split.axis == Axis::Vertical { 0 } else { 1 }, toward))
            }
            _ => None,
        };
        let (a, b) = self.panes();
        let area = match hit {
            Hit::Inset(k) => self.insets.get(k).map(|i| i.rect),
            Hit::Main => Some(a),
            Hit::Pane => b,
            _ => None,
        }
        .unwrap_or(rect);
        let scale = self.shown_scale(hit)?;
        // The game's speed in world pixels (as many tiles a second however
        // large the view shows them), else window pixels through the zoom.
        let full = game_edge_speed(self.map_wheel_step).unwrap_or((EDGE_PAN_SPEED * scale, EDGE_PAN_SPEED * scale));
        Some(crate::edge_clock::EdgeTarget {
            view: view_key(hit)?,
            window: (self.window.0 as i32, self.window.1 as i32),
            area,
            rect: [x, y, rect[2], rect[3]],
            top_row,
            against,
            interface: self.interface_cover(),
            full,
            present: None,
        })
    }

    /// A native map wheel notch moves tiles, rather than scrolling the
    /// parked native camera or depending on how large this pane is drawn.
    /// UI wheels and drivers without map-wheel metadata keep their routing.
    pub(super) fn map_wheel(&mut self, delta: i32, p: (i32, i32)) -> Option<Vec<Input>> {
        let hit = self.wheel_view(p)?;
        let notches = -(delta / 120) as f64;
        if notches == 0.0 { return Some(vec![]); }
        let step = if self.shift { (self.map_wheel_step.0 as f64, 0.0) } else { (0.0, self.map_wheel_step.1 as f64) };
        self.wheel_pan(hit, (notches * step.0, notches * step.1), p)
    }

    /// A scroll at `p` along both axes (`Input::Scroll`; `d` in lines, or
    /// window pixels if `pixels`, positive moving the content right and
    /// down), in any map movement mode: pans the view under the pointer
    /// (captured: the world cursor's). Lines move the game's own wheel
    /// step a line, a fraction of a line that fraction of it; pixels (a
    /// trackpad) move the map with the fingers, at the view's zoom. With
    /// Shift a scroll only up or down goes sideways. `None`: not the map's
    /// (its `Wheel` notches go on as ever).
    pub(super) fn map_scroll(&mut self, d: (f64, f64), pixels: bool, p: (i32, i32)) -> Option<Vec<Input>> {
        let hit = self.wheel_view(p)?;
        let d = if self.shift && d.0 == 0.0 { (d.1, 0.0) } else { d };
        let per = if pixels {
            let scale = self.shown_scale(hit)?;
            (scale, scale)
        } else {
            (self.map_wheel_step.0 as f64, self.map_wheel_step.1 as f64)
        };
        self.wheel_pan(hit, (-d.0 * per.0, -d.1 * per.1), p)
    }

    /// The view the wheel at `p` pans, if the map's: the one under the
    /// pointer (beneath the interface and the zoom box), or with the mouse
    /// captured the world cursor's. Not while snapping points at the
    /// interface, nor in a pane showing the driver's screen; not for a
    /// driver without map-wheel metadata (its routing stays its own).
    fn wheel_view(&self, p: (i32, i32)) -> Option<Hit> {
        if self.map_wheel_step == (0, 0) || self.snapping { return None; }
        let hit = if self.locked {
            self.shown.iter().find(|(hit, _)| self.follow_of(*hit) == Some(Follow::Pointer)).map(|(hit, _)| *hit)?
        } else {
            if !matches!(self.hit(p), Hit::Main | Hit::Pane | Hit::Inset(_) | Hit::Magnifier) { return None; }
            self.pan_view_at(p)?
        };
        (!self.fit.is_some_and(|fit| fit.hit == hit)).then_some(hit)
    }

    /// Pans view `hit` by `by` world pixels for the wheel at `p`, from its
    /// continuous centre (a drag of it going on from there); what goes to
    /// the driver: where its world pointer now is.
    fn wheel_pan(&mut self, hit: Hit, by: (f64, f64), p: (i32, i32)) -> Option<Vec<Input>> {
        let (center, _) = self.clamp_view_center(hit, self.exact_center(hit)?);
        let (to, _) = self.clamp_view_center(hit, (center.0 + by.0, center.1 + by.1));
        self.pan.coast = None;
        if let Some(g) = self.pan.grab.as_mut().filter(|g| g.hit == hit) {
            // (Grabbed meanwhile: the pointer holds the moved map.)
            g.start = (g.start.0 + to.0 - center.0, g.start.1 + to.1 - center.1);
            g.at = to;
        }
        self.place_exact(hit, to);
        let old_point = if self.locked { Some(self.cursor) } else { self.world_under(p) };
        let out = old_point.map(|old| {
            let point = if self.locked { self.cursor } else { (old.0 + (to.0 - center.0).round() as i32, old.1 + (to.1 - center.1).round() as i32) };
            Input::WorldMove { x: point.0, y: point.1 }
        }).into_iter().collect();
        Some(out)
    }

    /// All arranged world panes scroll locally. Keep the native edge
    /// pointer neutral so its tile-based scrolling cannot race the viewer.
    pub(super) fn driver_edge(&self, _p: (i32, i32)) -> (i32, i32) {
        (0, 0)
    }

    /// The pointer left the window (or it lost the focus).
    pub(super) fn pan_pointer_gone(&mut self) {
        self.pan.inside = false;
        self.aim_edge_clock();
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
        self.pan.flight = self.pan.flight.filter(|f| f.hit != hit);
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
        // A drag may pass through the divider, but a reserved zoom area
        // still holds the pointer outside, as OS confinement does.
        let raw = p;
        let p = self.constrain_zoom(self.pointer, p);
        if p != raw && self.walls == super::Walls::Warp {
            self.warp = Some(p);
        }
        self.pointer = p;
        self.raw_pointer = raw;
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
        self.place_exact(g.hit, at);
        Some(self.edge_change((0, 0)))
    }

    /// Grabs view `hit` at window pixel `from`: from its continuous centre
    /// (as the world's edge holds it), not the pixel the last frame showed.
    fn grab(&mut self, hit: Hit, button: Button, from: (i32, i32)) {
        let Some(scale) = self.shown_scale(hit) else { return };
        let Some(center) = self.exact_center(hit) else { return };
        let (center, _) = self.clamp_view_center(hit, center);
        self.pan.coast = None;
        self.pan.edge_view = None;
        self.pan.flight = None;
        self.pan.grab = Some(Grab { hit, button, from, start: center, at: center, scale, tracker: Tracker::new(from) });
        log::info!("viewer: {} grabbed ({button:?} button) at {from:?}", view_name(hit));
    }

    /// Lets go of the view grabbed at `p`: it slides on at its exact
    /// pixel position, without asking the driver to quantize its camera.
    fn let_go(&mut self, p: (i32, i32)) -> Vec<Input> {
        let Some(g) = self.pan.grab.take() else { return vec![] };
        let momentum = Momentum { vel: g.tracker.release(p) };
        let mut out = Vec::new();
        if momentum.moving() {
            self.pan.coast = Some(Coast { hit: g.hit, pos: g.at, momentum, scale: g.scale });
        }
        log::info!("viewer: {} let go, sliding at {:.0?} px/s", view_name(g.hit), momentum.vel);
        out.extend(self.edge_change(self.driver_edge(p)));
        out
    }

    /// A frame `dt` seconds after the last (the driver's screen fitted into
    /// `fitted`, if it is): the dragging pointer's velocity sampled, a
    /// sliding view moved on and the view at the pointer's edge scrolled.
    /// Drivers without semantic camera requests resume A following when
    /// their own camera moves.
    pub(super) fn pan_frame(&mut self, dt: f64, fitted: Option<Hit>, native_follow: bool) {
        let pointer = self.pointer;
        if let Some(g) = &mut self.pan.grab {
            g.tracker.sample(pointer, dt);
        }
        self.slide(dt);
        self.fly(dt);
        self.edge_scroll(dt, fitted);
        if native_follow {
            self.follow_driver();
        } else if self.main_view.follow != Follow::Driver {
            self.pan.manual_driver = None;
        }
    }

    fn slide(&mut self, dt: f64) {
        let Some(mut c) = self.pan.coast else { return };
        if self.locked || self.click_popup_open() || !self.shown.iter().any(|(h, _)| *h == c.hit) {
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
        self.place_exact(c.hit, pos);
        self.pan.coast = c.momentum.moving().then_some(c);
    }

    fn follow_driver(&mut self) {
        let Some(anchor) = self.pan.manual_driver else { return };
        if self.main_view.follow != Follow::Driver {
            self.pan.manual_driver = None;
        } else if self.driver_center().is_some_and(|center| center != anchor) {
            // Only a new native camera position resumes following. An idle
            // frame or a quantized copy of the old position must not undo
            // the user's sub-tile pan. Cancel any old fling as well.
            self.pan.manual_driver = None;
            self.pan.coast = self.pan.coast.filter(|c| c.hit != Hit::Main);
            log::info!("viewer: A follows the driver's new camera position");
        }
    }

    /// A semantic engine camera request takes priority over a previous
    /// manual pan or fling, including a repeat request for the same tile.
    pub(super) fn resume_game_focus(&mut self) {
        self.pan.manual_driver = None;
        self.main_view.hand_focus = None;
        self.main_view.scale_rest = (0.0, 0.0);
        self.pan.coast = self.pan.coast.filter(|c| c.hit != Hit::Main);
        self.pan.grab = self.pan.grab.filter(|g| g.hit != Hit::Main);
        self.pan.press = self.pan.press.filter(|p| p.hit != Hit::Main);
        // (The next pan starts from where the request puts A.)
        self.pan.exact.retain(|e| e.hit != Hit::Main);
    }

    pub(super) fn pan_rescale_world(&mut self, ratio: f64) {
        let point = |p: (i32, i32)| ((p.0 as f64 * ratio).round() as i32, (p.1 as f64 * ratio).round() as i32);
        let scaled = |p: (f64, f64)| (p.0 * ratio, p.1 * ratio);
        self.pan.manual_driver = self.pan.manual_driver.map(point);
        // The continuous centres scale exactly; the views were moved to
        // their rescaled pixels already (`rescale_world`).
        let mut exact = std::mem::take(&mut self.pan.exact);
        // (Only where the view still was where panning drew it: its old
        // pixel and the fraction it carried, `scale_rest`, map there.)
        let near = |old: i32, now: i32| (old as f64 * ratio - now as f64).abs() < ratio + 1.0;
        exact.retain_mut(|e| {
            e.exact = scaled(e.exact);
            match self.view_point(e.hit) {
                Some(drawn) if near(e.drawn.0, drawn.0) && near(e.drawn.1, drawn.1) => { e.drawn = drawn; true }
                _ => false,
            }
        });
        self.pan.exact = exact;
        if let Some(g) = &mut self.pan.grab {
            g.at = scaled(g.at);
            g.start = g.at;
            g.from = self.pointer;
        }
        if let Some(c) = &mut self.pan.coast {
            c.pos = scaled(c.pos);
        }
        if let Some(f) = &mut self.pan.flight {
            (f.glide.pos, f.glide.vel, f.to) = (scaled(f.glide.pos), scaled(f.glide.vel), point(f.to));
        }
    }

    pub(super) fn main_panned_by_hand(&self) -> bool {
        self.pan.manual_driver.is_some() && self.main_view.follow == Follow::Driver
    }

    /// Whether the pointer scrolls a view at its edges now (not where it is).
    fn edge_scroll_active(&self) -> bool {
        // A held right-button piece preview can reach beyond this pane in
        // Drag mode. Its pending destination stops scrolling on release.
        let enabled = self.pan.mode == MapMovement::EdgeScroll || self.world_order_dragging();
        // (Not under a popup opened at a world click: the map stays put.)
        enabled && self.pan.inside && !self.locked && !self.snapping && self.drag.is_none() && self.pan.grab.is_none() && !self.click_popup_open()
    }

    /// The view the pointer thread is to scroll from now on (`edge_clock`),
    /// for the pointer where the layout has it: its view (not one fitted to
    /// the driver's screen, `fitted`), while edge scrolling is active.
    fn edge_aim(&self, fitted: Option<Hit>) -> Option<crate::edge_clock::EdgeTarget> {
        if !self.edge_scroll_active() {
            return None;
        }
        let hit = self.pan_view_at(self.pushed_in(self.pointer)?)?;
        if fitted == Some(hit) {
            return None;
        }
        let mut target = self.edge_target(hit)?;
        target.present = self.pointer_present;
        Some(target)
    }

    /// Scrolls with the pointer thread's clock (`edge_clock`) from now on.
    pub fn attach_edge_clock(&mut self, clock: std::sync::Arc<crate::edge_clock::EdgeClock>) {
        self.pan.edge_taken = clock.totals().moved.iter().map(|&(v, total, _)| (v, total)).collect();
        self.edge_clock = Some(clock);
    }

    /// Tells the pointer thread which view the pointer is in now (after it
    /// moved): it scrolls that one until the next frame looks again.
    pub(super) fn aim_edge_clock(&self) {
        if let Some(clock) = &self.edge_clock {
            clock.aim(self.edge_aim(self.edge_fitted));
        }
    }

    fn edge_scroll(&mut self, dt: f64, fitted: Option<Hit>) {
        self.edge_fitted = fitted;
        let target = self
            .view_edge(self.pointer)
            .filter(|&(hit, e)| self.edge_scroll_active() && e != (0, 0) && fitted != Some(hit));
        let sign = |(h, e): (Hit, (i32, i32))| (h, (e.0.signum(), e.1.signum()));
        if target.map(sign) != self.pan.edge_view {
            match target {
                Some((hit, e)) => log::info!("viewer: edge scrolling {} towards {:?}", view_name(hit), (e.0.signum(), e.1.signum())),
                None if self.pan.edge_view.is_some() => log::info!("viewer: edge scrolling ends"),
                None => {}
            }
            self.pan.edge_view = target.map(sign);
        }
        if let Some(clock) = self.edge_clock.clone() {
            // What the pointer thread scrolled since the last frame, on its
            // own clock (none of it while scrolling is not active now).
            let totals = clock.totals();
            for &(view, _, _) in &totals.moved {
                let total = totals.shown(view, self.frame_instant);
                let taken = match self.pan.edge_taken.iter_mut().find(|(v, _)| *v == view) {
                    Some((_, taken)) => std::mem::replace(taken, total),
                    None => {
                        self.pan.edge_taken.push((view, total));
                        (0.0, 0.0)
                    }
                };
                let by = (total.0 - taken.0, total.1 - taken.1);
                let Some(hit) = hit_of_view(view).filter(|&h| fitted != Some(h) && self.edge_scroll_active()) else { continue };
                let Some(t) = self.edge_target(hit) else { continue };
                // (After the window stood still for long, as far as one long
                // frame scrolls, as without the thread: no jump.)
                let most = (t.full.0.abs() * crate::edge_clock::MAX_STEP, t.full.1.abs() * crate::edge_clock::MAX_STEP);
                self.edge_move(hit, (by.0.clamp(-most.0, most.0), by.1.clamp(-most.1, most.1)));
            }
            clock.aim(self.edge_aim(fitted));
            return;
        }
        let Some((hit, e)) = target else { return };
        if dt <= 0.0 {
            return;
        }
        let Some(t) = self.edge_target(hit) else { return };
        self.edge_move(hit, t.step(e, dt));
    }

    /// Scrolls view `hit` by `by` world pixels from its continuous centre,
    /// held at the world's edge.
    fn edge_move(&mut self, hit: Hit, by: (f64, f64)) {
        if by == (0.0, 0.0) {
            return;
        }
        let Some(center) = self.exact_center(hit) else { return };
        let (center, _) = self.clamp_view_center(hit, center);
        let (to, _) = self.clamp_view_center(hit, (center.0 + by.0, center.1 + by.1));
        self.place_exact(hit, to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pointer resting on each edge as macOS reports it (the flipped
    /// view's y is `height − cocoa_y`: 0 at the top, but the bottom
    /// boundary itself, `height`, on the last row), windowed and
    /// fullscreen with a scaled canvas: every edge is its outermost pixel.
    #[test]
    fn every_edge_scrolls_from_macos_style_pointer_positions() {
        use tile_render::Present;
        let presents = [
            ("windowed", Present::unscaled((800, 600))),
            ("fullscreen, canvas 2x", Present { canvas: (960, 540), zoom: (2, 1), at: (0, 0), target: (1920, 1080) }),
            ("fullscreen, canvas 3/2", Present { canvas: (1280, 720), zoom: (3, 2), at: (0, 0), target: (1920, 1080) }),
        ];
        for (name, p) in presents {
            let (tw, th) = (p.target.0 as f64, p.target.1 as f64);
            let (cw, ch) = (p.canvas.0 as i32, p.canvas.1 as i32);
            let flipped = |cocoa_x: f64, cocoa_y: f64| (cocoa_x, th - cocoa_y);
            let edge = |at: (f64, f64)| {
                let (x, y) = p.to_canvas(at);
                (edge_axis(x, cw, true, true), edge_axis(y, ch, true, true))
            };
            assert_eq!(edge(flipped(tw / 2.0, th)), (0, -1), "{name}: top");
            assert_eq!(edge(flipped(tw / 2.0, 0.0)), (0, 1), "{name}: bottom");
            assert_eq!(edge(flipped(0.0, th / 2.0)), (-1, 0), "{name}: left");
            assert_eq!(edge(flipped(tw - 1.0, th / 2.0)), (1, 0), "{name}: right (last column)");
            assert_eq!(edge(flipped(tw, th / 2.0)), (1, 0), "{name}: right (boundary)");
            assert_eq!(edge(flipped(0.0, 0.0)), (-1, 1), "{name}: bottom-left corner");
            assert_eq!(edge(flipped(tw, 0.0)), (1, 1), "{name}: bottom-right corner");
            // Past the window (a drag outside) is no edge.
            assert_eq!(edge(flipped(tw / 2.0, -2.0 * th)), (0, 0), "{name}: below the window");
        }
    }

    #[test]
    fn edge_scrolling_is_faster_nearer_the_edge() {
        assert_eq!(edge_speed(0), 0.0);
        assert_eq!(edge_speed(1), EDGE_PAN_SPEED);
        assert_eq!(edge_speed(-1), -EDGE_PAN_SPEED);
        assert_eq!(edge_share(1), 1.0);
        // Nearly nothing at the zone's inner end, nothing past it.
        assert!(edge_share(EDGE_ZONE) > 0.0 && edge_share(EDGE_ZONE) < 0.01, "{}", edge_share(EDGE_ZONE));
        assert_eq!(edge_share(-EDGE_ZONE), -edge_share(EDGE_ZONE));
        // Monotonic: slower with every pixel further in, and smooth (no
        // step between neighbouring pixels larger than a 1/16th of full).
        for e in 1..EDGE_ZONE {
            assert!(edge_speed(e) > edge_speed(e + 1), "{e}");
            assert!(edge_share(-e) < edge_share(-e - 1), "{e}");
            assert!(edge_share(e) - edge_share(e + 1) < 1.0 / 16.0, "{e}");
        }
        // Flat at both ends: the first pixels in still scroll at nearly
        // full speed.
        assert!(edge_share(4) > 0.97);
        // The game's speed: its wheel step every 66 ms.
        assert_eq!(game_edge_speed((0, 0)), None);
        let (x, y) = game_edge_speed((256, 128)).unwrap();
        assert!((x - 256.0 / 0.066).abs() < 1e-9 && (y - 128.0 / 0.066).abs() < 1e-9);
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
        // Slower further in: 15 pixels in, about three quarters of that
        // (the zone is 48 pixels wide, easing out to nothing).
        mv(&mut l, (784, 300));
        tick(&mut l, 100);
        assert_eq!(b(&l), (start.0 + 166, start.1 - 60));
        // Its bottom edge (its left one is the divider's, no window edge).
        mv(&mut l, (406, 599));
        tick(&mut l, 100);
        assert_eq!(b(&l), (start.0 + 166, start.1));
        mv(&mut l, (600, 300));
        tick(&mut l, 100);
        assert_eq!(b(&l), (start.0 + 166, start.1));
        assert_eq!(l.split.unwrap().view.follow, Follow::Free);
        // A uses the same viewer integrator, without starting the native
        // game's independent tile-based edge scrolling.
        let a = l.main_view;
        mv(&mut l, (300, 300));
        for at in [(0, 300), (0, 0), (3, 599), (200, 599)] {
            assert!(edges(&mv(&mut l, at)).is_empty());
        }
        tick(&mut l, 100);
        assert_eq!((l.main_view.center, l.main_view.follow), ((a.center.0, a.center.1 + 60), Follow::Driver));
        // Its right side is the divider's, open in the middle: no edge.
        assert!(edges(&mv(&mut l, (399, 300))).is_empty());
        // Gone from the window: nothing scrolls.
        mv(&mut l, (799, 300));
        l.pointer_gone();
        let at = b(&l);
        tick(&mut l, 100);
        assert_eq!(b(&l), at);
    }

    /// The pointer resting on an edge, then pushed further into it: the
    /// system reports it on the edge, warped back onto it, or past it (a
    /// drag, a warp landing a pixel out, a clamp), event after event. The
    /// map scrolls at that edge's full speed on every frame, never stopping
    /// between events: all four edges, the corners, and B at its edges and
    /// against the divider.
    #[test]
    fn pushing_into_an_edge_scrolls_at_full_speed_on_every_frame() {
        let cases: [((i32, i32), (i32, i32)); 8] = [
            ((0, 300), (-1, 0)),
            ((799, 300), (1, 0)),
            ((400, 0), (0, -1)),
            ((400, 599), (0, 1)),
            ((0, 0), (-1, -1)),
            ((799, 0), (1, -1)),
            ((0, 599), (-1, 1)),
            ((799, 599), (1, 1)),
        ];
        for (at, dir) in cases {
            let mut l = Layout::default();
            l.set_map_movement(MapMovement::EdgeScroll);
            frame(&mut l);
            frame(&mut l);
            mv(&mut l, at);
            tick(&mut l, 16);
            for (i, out) in [0, 1, 3, 0, 2, 0, 48, 0, 5, 1].into_iter().enumerate() {
                let p = (at.0 + dir.0 * out, at.1 + dir.1 * out);
                mv(&mut l, p);
                let before = l.main_view.center;
                tick(&mut l, 16);
                let moved = (l.main_view.center.0 - before.0, l.main_view.center.1 - before.1);
                // 600 px/s for 16 ms: 9 or 10 whole pixels a frame.
                for (m, d) in [(moved.0, dir.0), (moved.1, dir.1)] {
                    assert!(if d == 0 { m == 0 } else { m * d >= 9 }, "edge {dir:?}, push {i} to {p:?}: moved {moved:?}");
                }
            }
        }
        // B: its right edge, and pushed against the divider from B's side
        // low down, where the divider is closed (every position past it
        // pushes: the divider holds the pointer at 404).
        let mut l = split(MapMovement::EdgeScroll, Roles::Free);
        for (at, dir, pushes) in [((799, 300), 1, [0, 2, 1, 0, 30, 0]), ((404, 590), -1, [10, 50, 20, 100, 30, 60])] {
            mv(&mut l, (600, at.1));
            mv(&mut l, (at.0 + dir * 10, at.1));
            tick(&mut l, 16);
            for out in pushes {
                mv(&mut l, (at.0 + dir * out, at.1));
                let before = b(&l);
                tick(&mut l, 16);
                let moved = b(&l).0 - before.0;
                assert!(moved * dir >= 9, "B towards {dir}, pushed {out}: moved {moved}");
            }
        }
    }

    /// One frame at `at_ms` on a virtual clock: the pointer thread ticked
    /// every `tick_ms` up to it, with the pointer at `pointer`.
    fn clocked_frame(l: &mut Layout, clock: &crate::edge_clock::EdgeClock, ticked: &mut f64, at_ms: f64, tick_ms: f64, pointer: (f64, f64)) -> Frame {
        while *ticked + tick_ms <= at_ms {
            *ticked += tick_ms;
            clock.step(Some(pointer), tick_ms / 1000.0, virtual_ms(*ticked));
        }
        l.frame_instant = Some(virtual_ms(at_ms));
        frame(l)
    }

    /// `ms` milliseconds into a virtual clock.
    fn virtual_ms(ms: f64) -> std::time::Instant {
        static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        *BASE.get_or_init(std::time::Instant::now) + Duration::from_secs_f64(ms / 1000.0)
    }

    /// Edge scrolling on the pointer thread's clock: pushing into the edge
    /// with motion events coming irregularly (bunched after stalls) and
    /// frames 8 to 60 ms apart, every frame moves the view by the full
    /// speed times the time since the last frame, never less (so never a
    /// stop between events), give or take the pixel it is drawn at.
    #[test]
    fn on_the_pointer_threads_clock_pushing_into_an_edge_scrolls_steadily_through_stalled_frames() {
        let mut l = Layout::default();
        l.set_map_movement(MapMovement::EdgeScroll);
        frame(&mut l);
        frame(&mut l);
        let clock = crate::edge_clock::EdgeClock::new();
        l.attach_edge_clock(clock.clone());
        let tick = 1000.0 / 240.0;
        let (mut now, mut ticked) = (50.0, 0.0);
        mv(&mut l, (0, 300));
        // (Scrolling for 50 ms already: the first frame sees a tick before it.)
        clocked_frame(&mut l, &clock, &mut ticked, now, tick, (0.0, 300.0));
        let mut shown = l.main_view.center;
        let mut from_ms = now;
        let frames = [16.0, 50.0, 8.0, 60.0, 16.0, 16.0, 45.0, 8.0, 33.0, 50.0];
        for (k, ms) in frames.into_iter().cycle().take(60).enumerate() {
            now += ms;
            // Motion events only between frames, past the edge by up to 48.
            let x = -[0, 1, 3, 0, 48, 2, 0][k % 7];
            for _ in 0..(k % 3) {
                mv(&mut l, (x, 300));
            }
            clocked_frame(&mut l, &clock, &mut ticked, now, tick, (x as f64, 300.0));
            let moved = shown.0 - l.main_view.center.0;
            let want = 600.0 * (now - from_ms) / 1000.0;
            assert!((moved as f64 - want).abs() <= 1.0, "frame {k} ({ms} ms): moved {moved}, {want:.1} wanted");
            assert_eq!(l.main_view.center.1, shown.1);
            from_ms = now;
            shown = l.main_view.center;
        }
        // Left: what it had scrolled stays scrolled, nothing more.
        mv(&mut l, (400, 300));
        clocked_frame(&mut l, &clock, &mut ticked, now + 100.0, tick, (400.0, 300.0));
        assert_eq!(l.main_view.center, shown);
    }

    /// The same with the real pointer thread and wall time: frames spin
    /// the CPU for 8 to 50 ms, motion events come only between frames. Each
    /// frame moves the view by the full speed times its own time.
    #[test]
    fn with_the_real_pointer_thread_frames_stalled_50_ms_still_scroll_by_their_time() {
        use crate::pointer_thread::{OnTick, PointerThread, tick_for};
        let mut l = Layout::default();
        l.set_map_movement(MapMovement::EdgeScroll);
        frame(&mut l);
        frame(&mut l);
        let clock = crate::edge_clock::EdgeClock::new();
        l.attach_edge_clock(clock.clone());
        let thread = PointerThread::spawn(tick_for(Some(120_000)), None, vec![clock.clone() as std::sync::Arc<dyn OnTick>]).unwrap();
        thread.report(Some((0.0, 300.0)));
        mv(&mut l, (0, 300));
        let stall = |ms: u64| {
            let until = std::time::Instant::now() + Duration::from_millis(ms);
            while std::time::Instant::now() < until {
                std::hint::spin_loop();
            }
        };
        stall(30);
        l.frame_instant = Some(std::time::Instant::now());
        frame(&mut l);
        let (mut shown, mut at) = (l.exact_center(Hit::Main).unwrap().0, l.frame_instant.unwrap());
        let (mut worst, mut total, mut want_total) = (0.0f64, 0.0, 0.0);
        for (k, ms) in [16, 50, 8, 50, 16, 33, 50, 8, 45, 16].into_iter().cycle().take(30).enumerate() {
            stall(ms);
            let x = -[0, 1, 3, 0, 48, 2][k % 6];
            for _ in 0..(k % 3) {
                thread.report(Some((x as f64, 300.0)));
                mv(&mut l, (x, 300));
            }
            let now = std::time::Instant::now();
            l.frame_instant = Some(now);
            frame(&mut l);
            let moved = shown - l.exact_center(Hit::Main).unwrap().0;
            let want = 600.0 * (now - at).as_secs_f64();
            worst = worst.max((moved - want).abs() / want);
            assert!(moved > 0.0, "frame {k} ({ms} ms) did not scroll");
            (total, want_total) = (total + moved, want_total + want);
            (shown, at) = (l.exact_center(Hit::Main).unwrap().0, now);
        }
        drop(thread);
        // (A tick or two of scheduling noise on a loaded machine per frame;
        // in all, the full speed.)
        assert!(worst < 0.6, "a frame off by {:.0} %", worst * 100.0);
        assert!((total / want_total - 1.0).abs() < 0.05, "{total:.0} px scrolled, {want_total:.0} wanted");
    }

    /// With the pointer thread, a pointer that leaves the window (or a
    /// view grabbed, a popup at a click) stops what it scrolls at once,
    /// also what it ticked before the frame saw that.
    #[test]
    fn on_the_pointer_threads_clock_nothing_scrolls_once_the_pointer_is_gone() {
        let mut l = split(MapMovement::EdgeScroll, Roles::Free);
        let clock = crate::edge_clock::EdgeClock::new();
        l.attach_edge_clock(clock.clone());
        mv(&mut l, (799, 300));
        let start = b(&l);
        let mut ticked = 0.0;
        clocked_frame(&mut l, &clock, &mut ticked, 100.0, 4.0, (799.0, 300.0));
        // (Shown one tick late: 96 of the 100 ms.)
        assert_eq!(b(&l).0 - start.0, 58);
        // The thread ticked on before the layout heard the pointer left.
        clock.step(Some((799.0, 300.0)), 0.05, virtual_ms(150.0));
        l.pointer_gone();
        let at = b(&l);
        clocked_frame(&mut l, &clock, &mut ticked, 200.0, 4.0, (799.0, 300.0));
        assert_eq!(b(&l), at);
    }

    /// Printed with `--ignored --nocapture`: the view's speed frame by
    /// frame while the pointer is pushed into the left edge (positions on
    /// it and past it), frames 8 to 60 ms apart; scrolled by each frame's
    /// time (`inline`) and on the pointer thread's clock (`thread`).
    #[test]
    #[ignore]
    fn edge_push_velocity_trace() {
        let frames = [16.0, 50.0, 8.0, 60.0, 16.0, 16.0, 45.0, 8.0, 33.0, 50.0];
        let pushes = [0, -1, -3, 0, -48, -2, 0];
        for threaded in [false, true] {
            let mut l = Layout::default();
            l.set_map_movement(MapMovement::EdgeScroll);
            frame(&mut l);
            frame(&mut l);
            let clock = crate::edge_clock::EdgeClock::new();
            if threaded {
                l.attach_edge_clock(clock.clone());
            }
            mv(&mut l, (0, 300));
            let (mut now, mut ticked) = (0.0, 0.0);
            l.advance(Duration::from_millis(16));
            clocked_frame(&mut l, &clock, &mut ticked, now, 1000.0 / 240.0, (0.0, 300.0));
            let mut row = Vec::new();
            for (k, ms) in frames.into_iter().enumerate() {
                now += ms;
                let x = pushes[k % pushes.len()];
                mv(&mut l, (x, 300));
                let before = l.exact_center(Hit::Main).unwrap().0;
                l.advance(Duration::from_secs_f64(ms / 1000.0));
                clocked_frame(&mut l, &clock, &mut ticked, now, 1000.0 / 240.0, (x as f64, 300.0));
                row.push(format!("{ms:>2} ms frame, x={x:>3}: {:>5.1} px/s", (before - l.exact_center(Hit::Main).unwrap().0) / (ms / 1000.0)));
            }
            println!("{}:\n  {}", if threaded { "thread" } else { "inline" }, row.join("\n  "));
        }
    }

    #[test]
    fn a_view_of_the_viewers_own_in_a_scrolls_itself_and_pushing_the_divider_adds_to_an_edge() {
        // A follows the piece: the viewer scrolls it at the window's corner
        // (until the piece moves); the driver hears of no edge.
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
        assert_eq!(b(&l), (start.0 - 60, start.1 + 54));
    }

    #[test]
    fn a_zoomed_view_following_the_driver_scrolls_locally() {
        let mut l = Layout::default();
        l.main_view.zoom = super::super::ONE + 2;
        frame(&mut l);
        frame(&mut l);
        let start = l.main_view.center;
        assert!(edges(&mv(&mut l, (790, 595))).is_empty());
        tick(&mut l, 100);
        // (The continuous position, drawn at its nearest pixel.)
        let world = |e: i32| (edge_speed(e) * 0.1 * l.shown_scale(Hit::Main).unwrap()).round() as i32;
        assert_eq!((world(10), world(5)), (27, 29));
        assert_eq!(l.main_view.center, (start.0 + world(10), start.1 + world(5)));
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
    fn leaving_the_game_for_title_or_preview_stops_map_drag_and_momentum() {
        for preview in [false, true] {
            for release_drag in [false, true] {
                let mut l = Layout::default();
                frame(&mut l);
                button(&mut l, Button::Middle, true, (200, 200));
                mv(&mut l, (230, 200));
                tick(&mut l, 10);
                mv(&mut l, (260, 200));
                tick(&mut l, 10);
                if release_drag {
                    button(&mut l, Button::Middle, false, (260, 200));
                    assert!(l.pan.coast.is_some());
                } else { assert!(l.pan.grab.is_some()); }
                let mut menu = scene_at(1000);
                menu.shape = Default::default();
                if preview { menu.interface.beside = Some(scene::Beside { screen: [100, 100, 400, 400], edge: scene::Edge::Left, world: None }); }
                l.frame(&menu, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                assert!(l.pan.grab.is_none() && l.pan.press.is_none() && l.pan.coast.is_none());
                assert!(l.pan.manual_driver.is_none() && l.pan.edge_view.is_none());
                assert_eq!(l.pointer_walls(), None);
                assert!(!l.captured());
                // A new game starts without the old drag or fling resuming.
                frame(&mut l);
                let at = l.main_view.center;
                tick(&mut l, 100);
                assert_eq!(l.main_view.center, at);
            }
        }
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
            // A converts that same window-pixel speed through its zoom.
            // Cross the divider through its open middle before A's corner.
            let start = l.main_view.center;
            mv(&mut l, (100, 300));
            assert!(edges(&mv(&mut l, (3, 0))).is_empty(), "{scale:?}");
            tick(&mut l, 100);
            assert_eq!(l.main_view.center, (start.0 - (edge_speed(4) * 0.1 * world_per_pixel).round() as i32, start.1 - (60.0 * world_per_pixel).round() as i32), "{scale:?}");
        }
    }

    #[test]
    fn manual_a_pan_keeps_subtile_pixels_until_the_game_moves_its_camera() {
        for smooth in [false, true] {
            let mut l = Layout::default();
            l.smooth = smooth;
            l.set_map_movement(MapMovement::Drag);
            frame(&mut l);
            frame(&mut l);
            assert_eq!(l.main_view.center, (1400, 800));
            left(&mut l, true, (400, 300));
            mv(&mut l, (393, 297));
            assert_eq!((l.main_view.center, l.main_view.follow), ((1407, 803), Follow::Driver));
            assert!(left(&mut l, false, (393, 297)).is_empty());
            // The unchanged, tile-aligned engine camera cannot pull A
            // back after release or after the old half-second timeout.
            for _ in 0..120 {
                tick(&mut l, 16);
                assert_eq!(l.main_view.center, (1407, 803), "smooth={smooth}");
            }
            assert!(l.main_panned_by_hand());
            // A real native camera change (minimap click, selected-piece
            // focus, site navigation) resumes normal following.
            for _ in 0..120 {
                l.advance(Duration::from_millis(16));
                l.frame(&scene_at(1128), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            }
            assert!(!l.main_panned_by_hand());
            assert_eq!((l.main_view.center, l.main_view.follow), ((1528, 800), Follow::Driver));
        }
    }

    #[test]
    fn a_and_b_edge_speeds_match_at_every_depth_zoom_and_frame_rate() {
        // Non-native centres expose accidental tile quantization. Test
        // asymmetric splits too: pane dimensions do not scale speed.
        for size in [S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
            for smooth in [false, true] {
                for zoom in [super::super::ONE - 1, super::super::ONE, super::super::ONE + 2] {
                    for scale in [(1, 1), (3, 2), (3, 1)] {
                        for depth in [1, 8, EDGE_ZONE / 2, EDGE_ZONE - 8] {
                            for ms in [4, 16, 40] {
                                let make = || {
                                    let mut l = split(MapMovement::EdgeScroll, Roles::Free);
                                    l.apply_settings(&viewer_settings::Layout { split: size, smooth_world_movement: smooth, ..Default::default() });
                                    l.world_scale = scale;
                                    l.main_view.zoom = zoom;
                                    l.split.as_mut().unwrap().view.zoom = zoom;
                                    frame(&mut l);
                                    frame(&mut l);
                                    l
                                };
                                let (mut a, mut b) = (make(), make());
                                let vertical = size != S::Horizontal1To1;
                                let (pa, pb) = if vertical { ((depth - 1, 300), (800 - depth, 300)) } else { ((400, depth - 1), (400, 600 - depth)) };
                                mv(&mut a, pa);
                                mv(&mut b, pb);
                                let (a0, b0) = (a.main_view.center, self::b(&b));
                                for _ in 0..100 {
                                    for l in [&mut a, &mut b] {
                                        l.advance(Duration::from_millis(ms));
                                        l.frame(&scene_at(1000), (800, 600), (0, 0, 0, 0), vec![], Some(WorldBounds { size: (8000, 4000), wrap: (true, true) }));
                                    }
                                }
                                let (da, db) = ((a0.0 - a.main_view.center.0, a0.1 - a.main_view.center.1), (self::b(&b).0 - b0.0, self::b(&b).1 - b0.1));
                                assert_eq!(da, db, "{size:?} smooth={smooth} zoom={zoom} scale={scale:?} depth={depth} ms={ms}");
                                let moved = if vertical { da.0 } else { da.1 };
                                assert!(moved > 0);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn edge_scrolling_moves_as_far_as_the_games_own_smooth_or_not() {
        // The game's wheel step (its scroll speed setting): at the
        // outermost pixel the map moves that far every 66 ms, in world
        // pixels however large the world is shown, smooth movement on or
        // off alike (only continuous instead of in steps).
        let wrap = Some(WorldBounds { size: (8000, 4000), wrap: (true, true) });
        for step in [(64, 32), (128, 64), (256, 128)] {
            for scale in [(1, 1), (5, 2), (5, 1)] {
                for zoom in [super::super::ONE, super::super::ONE + 2] {
                    let mut moved = vec![];
                    for smooth in [false, true] {
                        let mut s = scene_at(1000);
                        s.map_wheel_step = step;
                        let mut l = Layout::default();
                        l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, smooth_world_movement: smooth, ..Default::default() });
                        l.world_scale = scale;
                        l.split.as_mut().unwrap().view.zoom = zoom;
                        for _ in 0..2 {
                            l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], wrap);
                        }
                        let start = l.exact_center(Hit::Pane).unwrap();
                        // B's right edge, then its bottom-right corner.
                        mv(&mut l, (799, 300));
                        for _ in 0..60 {
                            l.advance(Duration::from_millis(16));
                            l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], wrap);
                        }
                        let at = l.exact_center(Hit::Pane).unwrap();
                        let want = step.0 as f64 / GAME_EDGE_TICK * 0.96;
                        let what = format!("step {step:?} scale {scale:?} zoom {zoom} smooth {smooth}");
                        assert!((at.0 - start.0 - want).abs() < 1e-6, "{what}: {} vs {want}", at.0 - start.0);
                        assert_eq!(at.1, start.1, "{what}");
                        mv(&mut l, (799, 599));
                        for _ in 0..60 {
                            l.advance(Duration::from_millis(16));
                            l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], wrap);
                        }
                        let end = l.exact_center(Hit::Pane).unwrap();
                        assert!((end.1 - at.1 - step.1 as f64 / GAME_EDGE_TICK * 0.96).abs() < 1e-6, "{what}");
                        moved.push((end.0 - start.0, end.1 - start.1));
                    }
                    assert_eq!(moved[0], moved[1], "step {step:?} scale {scale:?} zoom {zoom}: smooth or not, as far");
                }
            }
        }
    }

    #[test]
    fn edge_scrolling_slows_with_every_pixel_further_in() {
        let mut s = scene_at(1000);
        s.map_wheel_step = (128, 64);
        let mut last = f64::INFINITY;
        for depth in 1..=EDGE_ZONE + 1 {
            let mut l = split(MapMovement::EdgeScroll, Roles::Free);
            l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            let start = l.exact_center(Hit::Pane).unwrap();
            mv(&mut l, (800 - depth, 300));
            l.advance(Duration::from_millis(100));
            l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            let d = l.exact_center(Hit::Pane).unwrap().0 - start.0;
            if depth > EDGE_ZONE {
                assert_eq!(d, 0.0, "past the zone");
            } else {
                assert!(d > 0.0 && d < last, "{depth}: {d} after {last}");
            }
            last = d;
        }
    }

    #[test]
    fn game_zoom_art_does_not_change_the_a_b_edge_speed_ratio() {
        for downscale in [1, 2, 4] {
            for zoom in [super::super::ONE - 1, super::super::ONE, super::super::ONE + 2] {
                let mut scene = scene_at(1000);
                scene.views[0].view.downscale = downscale;
                let mut a = split(MapMovement::EdgeScroll, Roles::Free);
                let mut b = split(MapMovement::EdgeScroll, Roles::Free);
                for l in [&mut a, &mut b] {
                    l.main_view.zoom = zoom;
                    l.split.as_mut().unwrap().view.zoom = zoom;
                    l.frame(&scene, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                    l.frame(&scene, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                }
                let (a0, b0) = (a.main_view.center, self::b(&b));
                mv(&mut a, (0, 300));
                mv(&mut b, (799, 300));
                for _ in 0..20 {
                    for l in [&mut a, &mut b] {
                        l.advance(Duration::from_millis(16));
                        l.frame(&scene, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                    }
                }
                assert_eq!(a0.0 - a.main_view.center.0, self::b(&b).0 - b0.0, "game downscale {downscale}, zoom {zoom}");
                assert!(a.main_panned_by_hand());
            }
        }
    }

    #[test]
    fn semantic_game_focus_moves_only_a_and_repeat_requests_override_manual_pans() {
        for roles in [Roles::Free, Roles::UnitMouse] {
            for smooth in [false, true] {
                let mut l = Layout::default();
                l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles, smooth_world_movement: smooth, ..Default::default() });
                l.set_map_movement(MapMovement::Drag);
                let mut scene = scene_at(0);
                scene.views[0].view.camera_y = 0;
                scene.camera_focus = Some(scene::CameraFocus { serial: 1, center: (3000, 2000) });
                for _ in 0..2 {
                    l.frame(&scene, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                }
                assert_eq!((l.main_view.center, b(&l)), ((3000, 2000), (3000, 2000)));
                let b0 = b(&l);
                left(&mut l, true, (200, 300));
                mv(&mut l, (193, 297));
                left(&mut l, false, (193, 297));
                assert_eq!(l.main_view.center, (3007, 2003));
                // The physical engine camera may be parked, resized or
                // quantized. None of those coordinates can move local A.
                scene.views[0].view.camera_x = 128;
                for _ in 0..120 {
                    l.advance(Duration::from_millis(16));
                    l.frame(&scene, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                    assert_eq!((l.main_view.center, b(&l)), ((3007, 2003), b0));
                }
                // AI combat/movement emits a semantic request even if the
                // engine believes the tile is already in its own view.
                scene.camera_focus = Some(scene::CameraFocus { serial: 2, center: (5000, 2500) });
                for _ in 0..120 {
                    l.advance(Duration::from_millis(16));
                    l.frame(&scene, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                    assert_eq!(b(&l), b0, "AI focus must leave B independent");
                }
                assert_eq!(l.main_view.center, (5000, 2500));
                left(&mut l, true, (200, 300));
                mv(&mut l, (193, 297));
                left(&mut l, false, (193, 297));
                assert_eq!(l.main_view.center, (5007, 2503));
                scene.camera_focus = Some(scene::CameraFocus { serial: 3, center: (5000, 2500) });
                for _ in 0..120 {
                    l.advance(Duration::from_millis(16));
                    l.frame(&scene, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                }
                assert_eq!((l.main_view.center, b(&l)), ((5000, 2500), b0));
            }
        }
    }

    #[test]
    fn native_wheel_pans_the_hovered_view_far_from_the_parked_engine_camera() {
        for zoom in [super::super::ONE, super::super::ONE + 2] {
            for step in [(64, 32), (128, 64), (256, 128)] {
                let mut l = Layout::default();
                l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles: Roles::Free, ..Default::default() });
                l.main_view.zoom = zoom;
                l.split.as_mut().unwrap().view.zoom = zoom;
                let mut s = scene_at(0);
                s.views[0].view.camera_y = 0;
                s.camera_focus = Some(scene::CameraFocus { serial: 1, center: (3000, 2000) });
                s.map_wheel_step = step;
                l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                let native = l.main.unwrap();
                let out = l.route(Input::Wheel { delta: 120, x: 100, y: 300 });
                assert!(out.iter().all(|i| matches!(i, Input::WorldMove { .. })));
                assert_eq!((l.main_view.center, b(&l)), ((3000, 2000 - step.1), (3000, 2000)));
                l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                assert_eq!(l.main, Some(native));
                let a = l.main_view.center;
                let out = l.route(Input::Wheel { delta: -240, x: 600, y: 300 });
                assert!(out.iter().all(|i| matches!(i, Input::WorldMove { .. })));
                assert_eq!((l.main_view.center, b(&l)), (a, (3000, 2000 + 2 * step.1)));
                l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                l.route(Input::Key { vk: 0x10, down: true, text: None });
                l.route(Input::Wheel { delta: 120, x: 100, y: 300 });
                assert_eq!(l.main_view.center, (3000 - step.0, a.1));
                l.route(Input::Key { vk: 0x10, down: false, text: None });
                s.views[0].view.downscale = 2;
                s.camera_focus.as_mut().unwrap().center = (1500, 1000);
                s.map_wheel_step = (step.0 / 2, step.1 / 2);
                let bounds = Some(WorldBounds { size: (4000, 2000), wrap: (true, false) });
                l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], bounds);
                let a = l.main_view.center;
                let b0 = b(&l);
                l.route(Input::Wheel { delta: 120, x: 100, y: 300 });
                assert_eq!((l.main_view.center, b(&l)), ((a.0, a.1 - step.1 / 2), b0));
                // Native wheel input truncates partial notches. It does
                // not change zoom or replay a parked-camera wheel event.
                for delta in [-119, 0, 119] {
                    let at = l.main_view.center;
                    assert!(l.route(Input::Wheel { delta, x: 100, y: 300 }).is_empty());
                    assert_eq!(l.main_view.center, at);
                }
            }
        }
    }

    #[test]
    fn native_map_wheel_with_capture_moves_b_cursor_and_leaves_a_in_place() {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles: Roles::UnitMouse, ..Default::default() });
        let mut s = scene_at(0);
        s.camera_focus = Some(scene::CameraFocus { serial: 1, center: (3000, 2000) });
        s.map_wheel_step = (128, 64);
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(l.capture_at((600, 300)));
        let a = l.main_view.center;
        let out = l.route(Input::Wheel { delta: 120, x: 200, y: 300 });
        assert_eq!(out, [Input::WorldMove { x: 3000, y: 1936 }]);
        assert_eq!(l.main_view.center, a);
        assert_eq!(l.cursor, (3000, 1936));
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!((l.main_view.center, b(&l)), (a, (3000, 1936)));
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
    fn a_view_following_the_world_cursor_drags_the_cursor_and_a_click_is_the_worlds() {
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
        // A click is a click on the world there: Drag captures nothing
        // (the window's edges do nothing, the pointer may leave it).
        mv(&mut l, (600, 300));
        assert!(left(&mut l, true, (600, 300)).is_empty(), "held back until it is a click or a drag");
        let out = left(&mut l, false, (600, 300));
        assert!(!l.captured());
        // (Press and release for the game, through its window or its map.)
        let press = |down: bool| out.iter().any(|i| matches!(*i, Input::WorldButton { button: Button::Left, down: d, .. } | Input::MouseButton { button: Button::Left, down: d, .. } if d == down));
        assert!(press(true) && press(false), "{out:?}");
        assert_ne!(l.take_pointer_icon(), Some(super::super::PointerIcon::Capture), "no crosshair: a click does not capture");
        // Ctrl+Shift+Space does not capture either; the divider holds
        // nothing back and the system confines nothing.
        l.route(Input::Key { vk: 0x11, down: true, text: None });
        l.route(Input::Key { vk: 0x10, down: true, text: None });
        l.route(Input::Key { vk: 0x20, down: true, text: None });
        l.route(Input::Key { vk: 0x20, down: false, text: None });
        l.route(Input::Key { vk: 0x10, down: false, text: None });
        l.route(Input::Key { vk: 0x11, down: false, text: None });
        assert!(!l.captured());
        assert_eq!(l.pointer_walls(), None);
        // Back to edge scrolling, the click captures again.
        l.set_map_movement(MapMovement::EdgeScroll);
        tick(&mut l, 16);
        left(&mut l, true, (600, 300));
        left(&mut l, false, (600, 300));
        assert!(l.captured());
    }

    // ---- The continuous pan position --------------------------------------

    /// A frame of `scene`, `ms` milliseconds on.
    fn step(l: &mut Layout, scene: &Scene, ms: u64, bounds: Option<WorldBounds>) {
        l.advance(Duration::from_millis(ms));
        l.frame(scene, (800, 600), (0, 0, 0, 0), vec![], bounds);
    }

    /// Where view `hit` is centred (one following the world cursor: on the
    /// cursor, which it shows from the next frame on).
    fn center_of(l: &Layout, hit: Hit) -> (i32, i32) {
        l.view_point(hit).unwrap()
    }

    /// `d` the short way round a world `period` pixels around.
    fn wrapped(d: i32, period: i32) -> i32 {
        (d + period / 2).rem_euclid(period) - period / 2
    }

    /// Where positions `xs` along a world 8000 pixels around first turn
    /// back (standing still is no turn).
    fn turn(xs: &[i32]) -> Option<usize> {
        let mut way = 0;
        for (i, w) in xs.windows(2).enumerate() {
            let d = wrapped(w[1] - w[0], 8000).signum();
            if d != 0 && way != 0 && d != way {
                return Some(i);
            }
            if d != 0 {
                way = d;
            }
        }
        None
    }

    #[test]
    fn middle_drags_and_slides_never_turn_back_while_what_the_view_follows_moves() {
        // The piece A follows in UnitMouse roles walks every frame, a game
        // camera request (and a combat arrival) stands, the views glide:
        // none of it may pull the dragged or sliding view back between the
        // pointer's motions or the slide's steps. At every zoom and pixel
        // scale, short and long, across the wrapping seam at x = 8000.
        let one = super::super::ONE;
        for roles in [Roles::Free, Roles::UnitMouse] {
            for smooth in [false, true] {
                for zoom in [one - 1, one, one + 2] {
                    for scale in [(1, 1), (3, 2), (2, 1), (1, 2)] {
                        for hit in [Hit::Main, Hit::Pane] {
                            for game in [None, Some(false), Some(true)] {
                                for (pixels, ms) in [(3, 16), (40, 8)] {
                                    for dir in [1, -1] {
                                        let case = format!("{roles:?} smooth={smooth} zoom={zoom} scale={scale:?} {hit:?} game={game:?} {pixels}px/{ms}ms dir={dir}");
                                        let mut l = Layout::default();
                                        l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles, smooth_world_movement: smooth, ..Default::default() });
                                        l.world_scale = scale;
                                        l.main_view.zoom = zoom;
                                        l.split.as_mut().unwrap().view.zoom = zoom;
                                        let mut scene = scene_at(1000);
                                        scene.focus = Some((7950, 2000));
                                        if let Some(combat) = game {
                                            scene.camera_focus = Some(scene::CameraFocus { serial: 1, center: (7950, 2000) });
                                            if combat {
                                                scene.camera_arrival = Some(scene::CameraArrival { serial: 1, bounds: [7900, 1950, 8000, 2050] });
                                            }
                                        }
                                        for _ in 0..3 {
                                            step(&mut l, &scene, 16, BOUNDS);
                                        }
                                        if hit == Hit::Pane {
                                            let v = &mut l.split.as_mut().unwrap().view;
                                            (v.center, v.settling) = ((7950, 2000), false);
                                            (l.cursor, l.cursor_placed) = ((7950, 2000), true);
                                            step(&mut l, &scene, 16, BOUNDS);
                                        }
                                        let x0 = if hit == Hit::Main { 200 } else { 600 };
                                        mv(&mut l, (x0, 300));
                                        button(&mut l, Button::Middle, true, (x0, 300));
                                        let mut xs = vec![center_of(&l, hit).0];
                                        let mut ys = vec![center_of(&l, hit).1];
                                        let mut walk = |scene: &mut Scene, k: i32| scene.focus = Some((7950 + 5 * k, 2000 + k));
                                        for k in 1..=10 {
                                            mv(&mut l, (x0 - dir * pixels * k, 300 - k));
                                            let moved = center_of(&l, hit);
                                            walk(&mut scene, k);
                                            step(&mut l, &scene, ms, BOUNDS);
                                            // Nothing moved it between the pointer's motions.
                                            assert_eq!(center_of(&l, hit), moved, "{case}: pulled during the drag");
                                            xs.push(moved.0);
                                            ys.push(moved.1);
                                        }
                                        button(&mut l, Button::Middle, false, (x0 - dir * pixels * 10, 290));
                                        assert!(l.pan.coast.is_some(), "{case}");
                                        let mut frames = 0;
                                        while l.pan.coast.is_some() {
                                            walk(&mut scene, 11 + frames);
                                            step(&mut l, &scene, 16, BOUNDS);
                                            xs.push(center_of(&l, hit).0);
                                            ys.push(center_of(&l, hit).1);
                                            frames += 1;
                                            assert!(frames < 600, "{case}: never settles");
                                        }
                                        assert_eq!(turn(&xs), None, "{case}: {xs:?}");
                                        assert_eq!(turn(&ys), None, "{case}: {ys:?}");
                                        // (A fast fling may go more than half round.)
                                        let gone: i32 = xs.windows(2).map(|w| wrapped(w[1] - w[0], 8000)).sum::<i32>() * dir;
                                        assert!(gone > 0, "{case}: {gone}");
                                        // Let go and settled, it stays (a combat arrival
                                        // is the game's to show again, in Focus mode).
                                        let refocus = game == Some(true) && hit == Hit::Main && roles == Roles::UnitMouse;
                                        if !refocus {
                                            let at = center_of(&l, hit);
                                            for _ in 0..60 {
                                                step(&mut l, &scene, 16, BOUNDS);
                                                assert_eq!(center_of(&l, hit), at, "{case}: moved after settling");
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn repeated_pans_chain_on_the_continuous_position_without_drift() {
        // At a 3:2 pixel scale a window pixel is 2/3 of a world pixel. A pan
        // starting from the pixel the last one was drawn at rounds each 2/3
        // up to 1 (30 pans: 30 pixels, not 20). Across the seam, too.
        for scale in [(3, 2), (1, 2), (2, 1), (3, 1)] {
            for hit in [Hit::Main, Hit::Pane] {
                let mut l = split(MapMovement::EdgeScroll, Roles::Free);
                l.world_scale = scale;
                frame(&mut l);
                let v = &mut l.split.as_mut().unwrap().view;
                (v.center, v.settling) = ((7995, 2000), false);
                frame(&mut l);
                let per_pixel = l.shown_scale(hit).unwrap();
                let start = center_of(&l, hit);
                let x0 = if hit == Hit::Main { 200 } else { 600 };
                for n in 1..=30 {
                    mv(&mut l, (x0, 300));
                    button(&mut l, Button::Middle, true, (x0, 300));
                    mv(&mut l, (x0 - 1, 300));
                    // (Held still: no slide.)
                    for _ in 0..6 {
                        tick(&mut l, 16);
                    }
                    button(&mut l, Button::Middle, false, (x0 - 1, 300));
                    assert!(l.pan.coast.is_none());
                    tick(&mut l, 16);
                    let want = (n as f64 * per_pixel).round() as i32;
                    assert_eq!(wrapped(center_of(&l, hit).0 - start.0, 8000), want, "{scale:?} {hit:?} pan {n}");
                }
                // Short bursts of edge scrolling chain on the same position.
                let at = center_of(&l, hit);
                let exact = l.exact_center(hit).unwrap();
                let edge = if hit == Hit::Main { (200, 599) } else { (600, 599) };
                for n in 1..=20 {
                    mv(&mut l, edge);
                    tick(&mut l, 16);
                    mv(&mut l, (x0, 300));
                    tick(&mut l, 16);
                    let want = exact.1 + n as f64 * EDGE_PAN_SPEED * 0.016 * per_pixel;
                    assert!((l.exact_center(hit).unwrap().1 - want).abs() < 1e-6, "{scale:?} {hit:?} burst {n}");
                    assert_eq!(center_of(&l, hit), (at.0, want.round() as i32), "{scale:?} {hit:?} burst {n}");
                }
            }
        }
    }

    #[test]
    fn a_game_camera_glide_hands_over_to_a_pan_once_and_nothing_echoes_back() {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles: Roles::Free, smooth_world_movement: true, ..Default::default() });
        let mut scene = scene_at(0);
        scene.views[0].view.camera_y = 0;
        scene.camera_focus = Some(scene::CameraFocus { serial: 1, center: (3000, 2000) });
        for _ in 0..3 {
            step(&mut l, &scene, 16, BOUNDS);
        }
        assert_eq!(l.main_view.center, (3000, 2000));
        let parked = l.main;
        // The game centres on a piece: A glides there.
        scene.camera_focus = Some(scene::CameraFocus { serial: 2, center: (5000, 2000) });
        for _ in 0..4 {
            step(&mut l, &scene, 16, BOUNDS);
        }
        let mid = l.main_view.center;
        let glide = l.main_view.glide.pos;
        assert!(mid.0 > 3000 && mid.0 < 5000, "{mid:?}");
        // Grabbed mid-glide: the pan takes the glide's own continuous
        // position over, once.
        mv(&mut l, (200, 300));
        let mut out = button(&mut l, Button::Middle, true, (200, 300));
        assert_eq!(l.pan.grab.map(|g| g.start), Some(glide));
        out.extend(mv(&mut l, (199, 300)));
        assert_eq!(l.main_view.center, ((glide.0 + 1.0).round() as i32, 2000));
        let held = l.main_view.center;
        for _ in 0..30 {
            step(&mut l, &scene, 16, BOUNDS);
            assert_eq!(l.main_view.center, held, "the old glide resumed under the hand");
        }
        out.extend(button(&mut l, Button::Middle, false, (199, 300)));
        // Neither the standing request nor the parked native camera pull A
        // back, and nothing went to the game to move its camera.
        for _ in 0..60 {
            step(&mut l, &scene, 16, BOUNDS);
            assert_eq!(l.main_view.center, held);
        }
        assert_eq!(l.main, parked);
        assert!(out.iter().all(|i| matches!(i, Input::Edge { .. } | Input::MouseMove { .. })), "{out:?}");
        // What the game sees under the pointer is what A shows there.
        let shown = l.shown.iter().find(|(h, _)| *h == Hit::Main).unwrap().1;
        assert_eq!(l.world_under((200, 300)), Some((shown.x + 200, shown.y + 300)));
        // A new request takes A over, one way; the next pan starts from it.
        scene.camera_focus = Some(scene::CameraFocus { serial: 3, center: (6000, 2000) });
        for _ in 0..120 {
            step(&mut l, &scene, 16, BOUNDS);
        }
        assert_eq!(l.main_view.center, (6000, 2000));
        assert!(l.pan.exact.iter().all(|e| e.hit != Hit::Main));
        button(&mut l, Button::Middle, true, (200, 300));
        mv(&mut l, (199, 300));
        assert_eq!(l.main_view.center, (6001, 2000));
        button(&mut l, Button::Middle, false, (199, 300));
    }

    #[test]
    fn switching_movement_mode_or_game_zoom_mid_slide_keeps_the_slide_continuous() {
        for change in ["mode", "zoom"] {
            let mut l = split(MapMovement::Drag, Roles::Free);
            let mut scene = scene_at(1000);
            button(&mut l, Button::Middle, true, (600, 300));
            for k in 1..=10 {
                mv(&mut l, (600 - 20 * k, 300));
                step(&mut l, &scene, 16, BOUNDS);
            }
            button(&mut l, Button::Middle, false, (400, 300));
            for _ in 0..5 {
                step(&mut l, &scene, 16, BOUNDS);
            }
            let before = l.pan.coast.unwrap().pos;
            let mut bounds = BOUNDS;
            let ratio = match change {
                "mode" => {
                    l.set_map_movement(MapMovement::EdgeScroll);
                    1.0
                }
                _ => {
                    // Z: the game's world drawn at half size.
                    scene.views[0].view.downscale = 2;
                    bounds = Some(WorldBounds { size: (4000, 2000), wrap: (true, false) });
                    0.5
                }
            };
            let mut xs = vec![(before.0 * ratio).round() as i32];
            step(&mut l, &scene, 0, bounds);
            assert_eq!(l.pan.coast.map(|c| c.pos), Some((before.0 * ratio, before.1 * ratio)), "{change}");
            assert_eq!(l.exact_center(Hit::Pane), Some((before.0 * ratio, before.1 * ratio)), "{change}");
            while l.pan.coast.is_some() {
                step(&mut l, &scene, 16, bounds);
                xs.push(b(&l).0);
                assert!(xs.len() < 600);
            }
            assert_eq!(turn(&xs), None, "{change}: {xs:?}");
            assert!(xs[xs.len() - 1] > xs[0], "{change}");
        }
    }

    // ---- The wheel and trackpad --------------------------------------------

    /// A scroll as the window reports it, with its notches (`lines` of a
    /// wheel), at `p`.
    fn scroll(l: &mut Layout, d: (f64, f64), pixels: bool, p: (i32, i32)) -> Vec<Input> {
        let milli = |v: f64| (v * 1000.0).round() as i32;
        let mut out = l.route(Input::Scroll { dx: milli(d.0), dy: milli(d.1), pixels, x: p.0, y: p.1 });
        let notches = if pixels { 0 } else { d.1.trunc() as i32 };
        for _ in 0..notches.abs() {
            out.extend(l.route(Input::Wheel { delta: 120 * notches.signum(), x: p.0, y: p.1 }));
        }
        out
    }

    fn shift(l: &mut Layout, down: bool) {
        l.route(Input::Key { vk: 0x10, down, text: None });
    }

    #[test]
    fn the_wheel_and_trackpad_pan_the_map_under_the_pointer_in_every_mode() {
        let one = super::super::ONE;
        for movement in [MapMovement::EdgeScroll, MapMovement::Drag] {
            for roles in [Roles::Free, Roles::UnitMouse] {
                for smooth in [false, true] {
                    for zoom in [one, one + 2] {
                        for hit in [Hit::Main, Hit::Pane] {
                            let case = format!("{movement:?} {roles:?} smooth={smooth} zoom={zoom} {hit:?}");
                            let mut l = Layout::default();
                            l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles, smooth_world_movement: smooth, ..Default::default() });
                            l.set_map_movement(movement);
                            l.main_view.zoom = zoom;
                            l.split.as_mut().unwrap().view.zoom = zoom;
                            let mut scene = scene_at(0);
                            scene.camera_focus = Some(scene::CameraFocus { serial: 1, center: (3000, 2000) });
                            scene.map_wheel_step = (128, 64);
                            for _ in 0..2 {
                                step(&mut l, &scene, 16, BOUNDS);
                            }
                            let p = if hit == Hit::Main { (200, 300) } else { (600, 300) };
                            mv(&mut l, p);
                            let per_pixel = l.shown_scale(hit).unwrap();
                            let start = l.exact_center(hit).unwrap();
                            let other = if hit == Hit::Main { Hit::Pane } else { Hit::Main };
                            let other_at = center_of(&l, other);
                            // A wheel line up: the game's step up, once (its
                            // notch is not the interface's too).
                            let out = scroll(&mut l, (0.0, 1.0), false, p);
                            assert!(out.iter().all(|i| matches!(i, Input::WorldMove { .. })), "{case}: {out:?}");
                            assert_eq!(l.exact_center(hit).unwrap(), (start.0, start.1 - 64.0), "{case}");
                            // Shift: sideways; a tilt wheel: sideways by itself.
                            shift(&mut l, true);
                            scroll(&mut l, (0.0, 1.0), false, p);
                            shift(&mut l, false);
                            scroll(&mut l, (1.0, 0.0), false, p);
                            assert_eq!(l.exact_center(hit).unwrap(), (start.0 - 256.0, start.1 - 64.0), "{case}");
                            // A trackpad, diagonally: with the fingers, sub-pixel.
                            let before = l.exact_center(hit).unwrap();
                            for _ in 0..4 {
                                scroll(&mut l, (-2.25, 1.5), true, p);
                            }
                            let want = (before.0 + 9.0 * per_pixel, before.1 - 6.0 * per_pixel);
                            let now = l.exact_center(hit).unwrap();
                            assert!((now.0 - want.0).abs() < 1e-9 && (now.1 - want.1).abs() < 1e-9, "{case}: {now:?} {want:?}");
                            assert_eq!(center_of(&l, hit), (want.0.round() as i32, want.1.round() as i32), "{case}");
                            // It stays there (nothing it follows pulls it back).
                            for _ in 0..30 {
                                step(&mut l, &scene, 16, BOUNDS);
                            }
                            assert_eq!(center_of(&l, hit), (want.0.round() as i32, want.1.round() as i32), "{case}");
                            assert_eq!(center_of(&l, other), other_at, "{case}: the other view moved");
                            // Ctrl: zoom, no pan.
                            let at = l.exact_center(hit).unwrap();
                            l.route(Input::Key { vk: 0x11, down: true, text: None });
                            scroll(&mut l, (0.0, 1.0), false, p);
                            l.route(Input::Key { vk: 0x11, down: false, text: None });
                            assert_eq!(l.exact_center(hit), Some(at), "{case}");
                            step(&mut l, &scene, 16, BOUNDS);
                            // Grabbed meanwhile: the wheel's pan is kept.
                            // (At the zoom Ctrl gave it.)
                            let per_pixel = l.shown_scale(hit).unwrap();
                            let at = l.exact_center(hit).unwrap();
                            mv(&mut l, p);
                            button(&mut l, Button::Middle, true, p);
                            scroll(&mut l, (0.0, -1.0), false, p);
                            mv(&mut l, (p.0, p.1 + 1));
                            let now = l.exact_center(hit).unwrap();
                            assert!((now.0 - at.0).abs() < 1e-9 && (now.1 - (at.1 + 64.0 - per_pixel)).abs() < 1e-9, "{case}: {now:?} from {at:?} at {per_pixel}");
                            button(&mut l, Button::Middle, false, (p.0, p.1 + 1));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_wheel_over_the_interface_and_a_captured_wheel_keep_their_routes() {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles: Roles::UnitMouse, ..Default::default() });
        let mut scene = scene_at(0);
        scene.camera_focus = Some(scene::CameraFocus { serial: 1, center: (3000, 2000) });
        scene.map_wheel_step = (128, 64);
        let ui = vec![[0, 500, 200, 100]];
        l.frame(&scene, (800, 600), (0, 0, 0, 0), ui.clone(), BOUNDS);
        let (a, b0) = (l.main_view.center, b(&l));
        // Over the interface: its wheel, no pan.
        let out = scroll(&mut l, (1.0, 1.0), false, (100, 550));
        // (The scroll itself in the driver's pixels for the viewer's list
        // scrolling, then the driver's notch.)
        assert!(matches!(out[..], [Input::Scroll { dx: 1000, dy: 1000, pixels: false, .. }, Input::Wheel { delta: 120, .. }]), "{out:?}");
        assert_eq!((l.main_view.center, b(&l)), (a, b0));
        // Captured: the world cursor's view (B) pans, diagonally too.
        assert!(l.capture_at((600, 300)));
        let cursor = l.cursor;
        let out = scroll(&mut l, (2.0, 0.0), true, (200, 300));
        assert_eq!(out, [Input::WorldMove { x: cursor.0 - 2, y: cursor.1 }]);
        scroll(&mut l, (0.0, 1.0), false, (200, 300));
        assert_eq!(l.cursor, (cursor.0 - 2, cursor.1 - 64));
        l.frame(&scene, (800, 600), (0, 0, 0, 0), ui, BOUNDS);
        assert_eq!((l.main_view.center, b(&l)), (a, l.cursor));
    }

    // ---- The minimap ---------------------------------------------------------

    /// A minimap at the driver's screen's bottom left, 200 × 100 of an 8000
    /// × 4000 world: 40 world pixels a minimap pixel.
    const MINIMAP: scene::Minimap = scene::Minimap { at: (0, 500), size: (200, 100), shown: [0, 500, 200, 100], world: (8000, 4000) };

    fn minimap_scene() -> Scene {
        let mut s = scene_at(0);
        s.camera_focus = Some(scene::CameraFocus { serial: 1, center: (3000, 2000) });
        s.minimap = Some(MINIMAP);
        s
    }

    fn minimap_frame(l: &mut Layout, s: &Scene, ms: u64) {
        l.advance(Duration::from_millis(ms));
        l.frame(s, (800, 600), (0, 0, 0, 0), vec![[0, 500, 200, 100]], BOUNDS);
    }

    #[test]
    fn on_the_drawn_minimap_right_moves_b_and_left_goes_to_the_game_for_a() {
        use crate::scaling;
        let mut s = minimap_scene();
        s.screen_size = (1920, 1080);
        s.minimap = Some(scene::Minimap { at: (30, 960), shown: [30, 960, 200, 100], ..MINIMAP });
        s.views[1].overlay = vec![scene::FreeSprite { x: 5, y: 949, screen: true, after_pass: u32::MAX, ..Default::default() }];
        let rect_of = |_: &scene::FreeSprite| Some([5, 949, 280, 130]);
        for roles in [Roles::Free, Roles::UnitMouse] {
            for pixel_scale in [1, 2, 3] {
                for frame_scale in [0.75, 1.0, 1.5] {
                    for mouse in [Button::Right, Button::Left] {
                        let case = format!("{roles:?}, pixel scale {pixel_scale}, frame scale {frame_scale}, {mouse:?}");
                        let mut l = Layout::default();
                        l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles, ..Default::default() });
                        let p = scaling::plan((3832, 2106), s.screen_size, &viewer_settings::Resolution { pixel_scale, frame_scale, ..Default::default() });
                        l.driver_screen = Some(p.screen);
                        let frame = |l: &mut Layout| {
                            let mut f = l.frame(&s, p.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                            l.set_interface_shown(scaling::arrange(&p, &s, &mut f, rect_of).shown);
                        };
                        frame(&mut l);
                        frame(&mut l);
                        // Input advanced while the first minimap frame was in
                        // flight: install its routing into the older layout.
                        let presented = l.clone();
                        l.minimap = None;
                        l.adopt_presented_mapping(presented);
                        let a = l.main_view.center;
                        let shown = l.driver_rect_to_window([100, 1000, 1, 1]);
                        let canvas = (shown[0].ceil() as i32, shown[1].ceil() as i32);
                        // Include the physical-window-to-canvas input transform.
                        let physical = p.to_window(canvas);
                        let at = p.to_screen((physical.0 as f64, physical.1 as f64));
                        let q = l.route_window(at).expect("drawn HUD reaches the driver");
                        let expected = s.minimap.unwrap().world_at(q).expect("over the minimap");
                        let b0 = b(&l);
                        if mouse == Button::Left {
                            // The game's own minimap click: it moves the
                            // game's camera, which A shows; B stays.
                            let press = button(&mut l, mouse, true, at);
                            assert!(press.contains(&Input::MouseButton { button: mouse, down: true, x: q.0, y: q.1 }), "{case}: {press:?}");
                            let release = button(&mut l, mouse, false, at);
                            assert!(release.iter().any(|i| matches!(i, Input::MouseButton { button: Button::Left, down: false, .. })), "{case}: {release:?}");
                            frame(&mut l);
                            assert_eq!(b(&l), b0, "{case}: B moved");
                            continue;
                        }
                        assert_eq!(button(&mut l, mouse, true, at), [], "{case}");
                        // The release stays ours after leaving the minimap.
                        assert_eq!(button(&mut l, mouse, false, (400, 300)), [], "{case}");
                        frame(&mut l);
                        assert_eq!(l.main_view.center, a, "{case}: A moved");
                        assert_eq!(b(&l), expected, "{case}: B did not move");
                    }
                }
            }
        }
    }

    #[test]
    fn captured_world_clicks_do_not_pick_the_minimap_under_the_system_pointer() {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles: Roles::UnitMouse, ..Default::default() });
        minimap_frame(&mut l, &minimap_scene(), 16);
        l.locked = true;
        l.cursor = (3500, 2500);
        let before = b(&l);
        for mouse in [Button::Left, Button::Right] {
            let out = button(&mut l, mouse, true, (100, 550));
            assert!(out.iter().any(|i| matches!(i, Input::MouseButton { .. } | Input::WorldButton { .. })), "{mouse:?}: {out:?}");
            button(&mut l, mouse, false, (100, 550));
            assert_eq!(b(&l), before);
            assert!(l.pan.flight.is_none());
            assert!(l.locked);
        }
    }

    #[test]
    fn the_right_release_is_ours_even_after_the_minimap_closes_and_left_stays_the_games() {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles: Roles::Free, ..Default::default() });
        let mut s = minimap_scene();
        minimap_frame(&mut l, &s, 16);
        assert_eq!(button(&mut l, Button::Right, true, (100, 550)), []);
        // The left button with the right one held on the minimap: the game's.
        assert_eq!(button(&mut l, Button::Left, true, (100, 550)), [Input::MouseButton { button: Button::Left, down: true, x: 100, y: 550 }]);
        assert_eq!(button(&mut l, Button::Left, false, (100, 550)), [Input::MouseButton { button: Button::Left, down: false, x: 100, y: 550 }]);
        s.minimap = None;
        minimap_frame(&mut l, &s, 16);
        assert_eq!(button(&mut l, Button::Right, false, (400, 300)), []);
        // Taken: the next right click off the minimap is the game's again.
        assert_eq!(button(&mut l, Button::Right, true, (100, 550)), [Input::MouseButton { button: Button::Right, down: true, x: 100, y: 550 }]);
        assert_eq!(button(&mut l, Button::Right, false, (100, 550)), [Input::MouseButton { button: Button::Right, down: false, x: 100, y: 550 }]);
        // A right press the game had keeps its release with the game even
        // when it ends on the minimap.
        s.minimap = Some(scene::Minimap { shown: [0, 500, 100, 100], ..MINIMAP });
        minimap_frame(&mut l, &s, 16);
        assert!(!l.pan.minimap_right);
        assert_eq!(button(&mut l, Button::Right, true, (150, 550)), [Input::MouseButton { button: Button::Right, down: true, x: 150, y: 550 }]);
        assert_eq!(button(&mut l, Button::Right, false, (50, 550)), [Input::MouseButton { button: Button::Right, down: false, x: 50, y: 550 }]);
    }

    #[test]
    fn a_right_drag_on_the_minimap_drags_b_and_leaves_a() {
        for smooth in [false, true] {
            let mut l = Layout::default();
            l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles: Roles::Free, smooth_world_movement: smooth, ..Default::default() });
            let s = minimap_scene();
            for _ in 0..2 {
                minimap_frame(&mut l, &s, 16);
            }
            let a = l.main_view.center;
            mv(&mut l, (100, 550));
            assert_eq!(button(&mut l, Button::Right, true, (100, 550)), []);
            // Dragged on along the minimap: B follows to the last point.
            let out = mv(&mut l, (150, 560));
            assert!(!out.iter().any(|i| matches!(i, Input::MouseButton { .. })), "{out:?}");
            let to = MINIMAP.world_at((150, 560)).unwrap();
            // Off the minimap B stays where the drag left it.
            mv(&mut l, (400, 300));
            for _ in 0..120 {
                minimap_frame(&mut l, &s, 16);
            }
            assert_eq!(center_of(&l, Hit::Pane), to, "smooth={smooth}");
            assert_eq!(l.main_view.center, a, "smooth={smooth}: A moved");
            assert_eq!(button(&mut l, Button::Right, false, (400, 300)), []);
            // Released, moving over the minimap no longer moves B.
            mv(&mut l, (20, 520));
            for _ in 0..60 {
                minimap_frame(&mut l, &s, 16);
            }
            assert_eq!(center_of(&l, Hit::Pane), to, "smooth={smooth}");
        }
    }

    #[test]
    fn a_left_click_on_the_minimap_is_the_games_and_moves_a_only() {
        for roles in [Roles::Free, Roles::UnitMouse] {
            let mut l = Layout::default();
            l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles, ..Default::default() });
            let mut s = minimap_scene();
            for _ in 0..2 {
                minimap_frame(&mut l, &s, 16);
            }
            let b0 = b(&l);
            mv(&mut l, (100, 550));
            assert_eq!(button(&mut l, Button::Left, true, (100, 550)), [Input::MouseButton { button: Button::Left, down: true, x: 100, y: 550 }], "{roles:?}");
            // A left drag over the minimap is the game's too.
            assert_eq!(mv(&mut l, (120, 560)).iter().filter(|i| matches!(i, Input::MouseMove { .. })).count(), 1, "{roles:?}");
            assert_eq!(button(&mut l, Button::Left, false, (120, 560)), [Input::MouseButton { button: Button::Left, down: false, x: 120, y: 560 }], "{roles:?}");
            assert!(l.pan.flight.is_none());
            // The game centres its camera there: A goes, B stays.
            let to = MINIMAP.world_at((120, 560)).unwrap();
            s.camera_focus = Some(scene::CameraFocus { serial: 2, center: to });
            for _ in 0..90 {
                minimap_frame(&mut l, &s, 16);
            }
            assert_eq!(l.main_view.center, to, "{roles:?}");
            assert_eq!(b(&l), b0, "{roles:?}");
        }
    }

    #[test]
    fn a_right_click_on_the_minimap_sends_b_there_and_the_game_never_pulls_it() {
        let mouse = Button::Right;
        for roles in [Roles::Free, Roles::UnitMouse] {
            for smooth in [false, true] {
                let case = format!("{mouse:?} {roles:?} smooth={smooth}");
                let mut l = Layout::default();
                l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles, smooth_world_movement: smooth, ..Default::default() });
                let mut s = minimap_scene();
                for _ in 0..2 {
                    minimap_frame(&mut l, &s, 16);
                }
                let a = l.main_view.center;
                // Picked at minimap pixel (100, 50): its middle, 4020, 2020.
                let to = MINIMAP.world_at((100, 550)).unwrap();
                assert_eq!(to, (4020, 2020));
                mv(&mut l, (100, 550));
                assert_eq!(button(&mut l, mouse, true, (100, 550)), [], "{case}");
                let mut xs = vec![center_of(&l, Hit::Pane).0];
                let mut frames = 0;
                while center_of(&l, Hit::Pane) != to || l.pan.flight.is_some() {
                    minimap_frame(&mut l, &s, 16);
                    xs.push(center_of(&l, Hit::Pane).0);
                    frames += 1;
                    assert!(frames < 200, "{case}: never lands {xs:?}");
                }
                assert_eq!(button(&mut l, mouse, false, (100, 550)), [], "{case}");
                assert_eq!(turn(&xs), None, "{case}: {xs:?}");
                assert!(smooth == (frames > 5), "{case}: {frames} frames");
                assert_eq!(l.main_view.center, a, "{case}: A moved");
                // The game's camera (a new request, its native camera) moves
                // A only; B stays where it was sent.
                s.camera_focus = Some(scene::CameraFocus { serial: 2, center: (6000, 1000) });
                s.views[0].view.camera_x = 512;
                for _ in 0..90 {
                    minimap_frame(&mut l, &s, 16);
                }
                assert_eq!(center_of(&l, Hit::Pane), to, "{case}");
                assert_eq!(l.main_view.center, (6000, 1000), "{case}");
                // The next pan of B goes on from there.
                assert_eq!(l.exact_center(Hit::Pane), Some((to.0 as f64, to.1 as f64)), "{case}");
            }
        }
    }

    #[test]
    fn a_minimap_flight_goes_the_short_way_round_the_seam() {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles: Roles::Free, smooth_world_movement: true, ..Default::default() });
        let s = minimap_scene();
        minimap_frame(&mut l, &s, 16);
        let v = &mut l.split.as_mut().unwrap().view;
        (v.center, v.settling) = ((7900, 2020), false);
        minimap_frame(&mut l, &s, 16);
        // Minimap pixel 2: world x 100, 200 pixels on round the seam.
        button(&mut l, Button::Right, true, (2, 550));
        let mut gone = 0;
        let mut last = center_of(&l, Hit::Pane).0;
        while l.pan.flight.is_some() {
            minimap_frame(&mut l, &s, 16);
            let now = center_of(&l, Hit::Pane).0;
            let d = wrapped(now - last, 8000);
            assert!(d >= 0, "{last} -> {now}");
            gone += d;
            last = now;
        }
        assert_eq!(gone, 200);
        assert_eq!(center_of(&l, Hit::Pane).0.rem_euclid(8000), 100);
        button(&mut l, Button::Right, false, (2, 550));
    }

    #[test]
    fn without_b_either_click_on_the_minimap_stays_the_games() {
        for mouse in [Button::Left, Button::Right] {
            let mut l = Layout::default();
            let s = minimap_scene();
            minimap_frame(&mut l, &s, 16);
            let a = l.main_view.center;
            let press = button(&mut l, mouse, true, (100, 550));
            let release = button(&mut l, mouse, false, (100, 550));
            assert_eq!(press, [Input::MouseButton { button: mouse, down: true, x: 100, y: 550 }]);
            assert_eq!(release, [Input::MouseButton { button: mouse, down: false, x: 100, y: 550 }]);
            minimap_frame(&mut l, &s, 16);
            assert_eq!(l.main_view.center, a);
        }
        // With B, a right click elsewhere on the interface is still the game's.
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, ..Default::default() });
        let mut s = minimap_scene();
        s.minimap = Some(scene::Minimap { shown: [0, 500, 100, 100], ..MINIMAP });
        minimap_frame(&mut l, &s, 16);
        let b0 = b(&l);
        assert!(matches!(button(&mut l, Button::Right, true, (150, 550))[..], [Input::MouseButton { button: Button::Right, down: true, .. }]));
        assert_eq!(b(&l), b0);
    }
}
