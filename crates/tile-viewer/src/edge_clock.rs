//! Edge scrolling on the pointer thread's clock (`pointer_thread`).
//!
//! The layout says which view the pointer is in and how that view meets the
//! window's edges ([`EdgeTarget`], published whenever the layout changes it:
//! each frame and each pointer motion it handles). On every tick the pointer
//! thread takes the newest pointer, finds how far into the edge zone it is
//! and adds that tick's scrolling to the view's running total ([`Totals`]).
//! The frame moves each view by what its total grew since the last frame.
//! So the scrolling distance follows the pointer at the thread's steady rate
//! (240 Hz on a 120 Hz display), however unevenly frames and the main
//! thread's events come, and a frame stalled 50 ms still moves the map by
//! 50 ms of scrolling, sampled all along.
//!
//! The pointer pushed past an edge of the window counts as on that edge's
//! outermost pixel ([`pushed_onto`]): pushing further into the edge never
//! slows the scrolling, whatever positions the system reports for it.

use std::sync::Arc;
use std::time::Instant;

use handoff::Latest;
use tile_render::Present;

use crate::pointer_thread::{OnTick, Tick};

/// How near (in window pixels) a window edge the pointer scrolls the view
/// it is in: three times the game's own zone
/// (`scene::EDGE_ZONE`).
pub const EDGE_ZONE: i32 = 3 * scene::EDGE_ZONE;

/// How far past the window (in window pixels) a pointer still counts as
/// pushed against its edge; further out (a drag that left the window) it
/// scrolls nothing.
pub const PUSH_MARGIN: i32 = EDGE_ZONE;

/// The longest step one tick or frame may scroll (seconds): after a stall
/// (the machine slept) the map does not jump.
pub const MAX_STEP: f64 = 0.1;

/// Coordinate `v` along a window `size` long, pushed onto it: past an edge
/// by up to [`PUSH_MARGIN`] it is that edge's outermost pixel; further out
/// `None`.
pub fn pushed_onto(v: i32, size: i32) -> Option<i32> {
    if v < -PUSH_MARGIN || v > size - 1 + PUSH_MARGIN {
        None
    } else {
        Some(v.clamp(0, (size - 1).max(0)))
    }
}

/// Per axis, where pointer coordinate `v` of a window `size` long is at its
/// edge: 0 if not within [`EDGE_ZONE`] of an edge the view reaches (`low`:
/// the left or top one; `high`: the right or bottom one), else how far in
/// it is plus one, negative at the low edge. Pushed past the edge (up to
/// [`PUSH_MARGIN`]) it is at the outermost pixel.
pub fn edge_axis(v: i32, size: i32, low: bool, high: bool) -> i32 {
    let Some(v) = pushed_onto(v, size) else { return 0 };
    let far = size - 1 - v;
    if low && v < EDGE_ZONE {
        -(v + 1)
    } else if high && far < EDGE_ZONE {
        far + 1
    } else {
        0
    }
}

/// The share of the full edge scrolling speed for the pointer at edge `e`
/// of a view (as [`edge_axis`] counts), signed: 1 at the outermost pixel,
/// easing smoothly (a smoothstep, flat at both ends) to nearly nothing at
/// the zone's inner end, so the map neither jumps on entering the zone nor
/// slows abruptly near the edge.
pub fn edge_share(e: i32) -> f64 {
    if e == 0 {
        return 0.0;
    }
    let t = (e.abs() - 1).min(EDGE_ZONE - 1) as f64 / EDGE_ZONE as f64;
    (1.0 - t * t * (3.0 - 2.0 * t)) * e.signum() as f64
}

/// A view the pointer is in, and how it meets the window's edges: what the
/// pointer thread needs to scroll it. Canvas pixels (the layout's).
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeTarget {
    /// The layout's key for the view.
    pub view: u32,
    pub window: (i32, i32),
    /// Where the pointer counts as in this view: a newer pointer outside it
    /// (the layout has not looked again yet) counts at its nearest pixel.
    pub area: [i32; 4],
    /// The view's rectangle: the window edges it reaches scroll it.
    pub rect: [i32; 4],
    /// Below a zoom strip: the view's top row is its upward edge where
    /// the pointer is between these x (from, to).
    pub top_row: Option<(i32, i32)>,
    /// Pushed against the divider's wall: the axis (0: x, 1: y) and the
    /// direction it scrolls at full speed.
    pub against: Option<(usize, i32)>,
    /// Over these (the driver's interface) only the game's own thin zone
    /// (`scene::EDGE_ZONE`) scrolls.
    pub interface: Vec<[i32; 4]>,
    /// The speed at the outermost pixel, world pixels a second per axis.
    pub full: (f64, f64),
    /// How the canvas shows in the window (`None`: one window pixel each).
    pub present: Option<Present>,
}

fn inside([x, y, w, h]: [i32; 4], p: (i32, i32)) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}

impl EdgeTarget {
    /// The edge (as [`edge_axis`] counts, per axis) of the pointer at canvas
    /// pixel `p`, already found in this view by the layout.
    pub fn edge_of(&self, p: (i32, i32)) -> (i32, i32) {
        let (ww, wh) = self.window;
        let [x, y, w, h] = self.rect;
        let mut e = (edge_axis(p.0, ww, x <= 0, x + w >= ww), edge_axis(p.1, wh, y <= 0, y + h >= wh));
        if let Some((from, to)) = self.top_row {
            if p.0 >= from && p.0 < to {
                // (Over the strip itself: not the map's edge.)
                e.1 = if p.1 < y { 0 } else { edge_axis(p.1 - y, h, true, false) };
            }
        }
        match self.against {
            Some((0, toward)) => e.0 = toward,
            Some((_, toward)) => e.1 = toward,
            None => {}
        }
        // Over the interface (the minimap, the HUD) only the game's own thin
        // edge scrolls: the wider zone would move the map under its controls.
        let held = (pushed_onto(p.0, ww).unwrap_or(p.0), pushed_onto(p.1, wh).unwrap_or(p.1));
        if self.interface.iter().any(|&r| inside(r, held)) {
            let thin = |d: i32| if d.abs() > scene::EDGE_ZONE { 0 } else { d };
            e = (thin(e.0), thin(e.1));
        }
        e
    }

    /// [`Self::edge_of`] for a newer pointer at window pixel `at`: kept to
    /// this view's area.
    pub fn edge_at(&self, at: (f64, f64)) -> (i32, i32) {
        let p = match self.present {
            Some(present) => present.to_canvas(at),
            None => (at.0.floor() as i32, at.1.floor() as i32),
        };
        let (Some(px), Some(py)) = (pushed_onto(p.0, self.window.0), pushed_onto(p.1, self.window.1)) else { return (0, 0) };
        let [x, y, w, h] = self.area;
        let p = (px.clamp(x, (x + w - 1).max(x)), py.clamp(y, (y + h - 1).max(y)));
        self.edge_of(p)
    }

    /// World pixels to scroll over `dt` seconds at edge `e`.
    pub fn step(&self, e: (i32, i32), dt: f64) -> (f64, f64) {
        let dt = dt.clamp(0.0, MAX_STEP);
        (edge_share(e.0) * self.full.0 * dt, edge_share(e.1) * self.full.1 * dt)
    }
}

/// What the pointer thread scrolled: per view, how far in all (world
/// pixels) and as of the tick before, when those two ticks were, and where
/// the pointer was at the last tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Totals {
    /// (view, total, total at the tick before).
    pub moved: Vec<(u32, (f64, f64), (f64, f64))>,
    /// (the tick before, the last tick).
    pub at: Option<(Instant, Instant)>,
    pub edge: Option<(u32, (i32, i32))>,
    pub ticks: u64,
}

impl Totals {
    /// How far view `view` scrolled in all, as of the last tick.
    pub fn of(&self, view: u32) -> (f64, f64) {
        self.moved.iter().find(|(v, _, _)| *v == view).map_or((0.0, 0.0), |&(_, m, _)| m)
    }

    /// How far view `view` scrolled as of one tick before `now`: between
    /// the last two ticks' totals, as far as `now` is past the last tick
    /// (at most the last). A frame drawn at `now` shows that, so frames at
    /// any phase to the ticks move the view by exactly their own time
    /// (one tick late), and never back. Without `now`: as of the last tick.
    pub fn shown(&self, view: u32, now: Option<Instant>) -> (f64, f64) {
        let Some(&(_, total, before)) = self.moved.iter().find(|(v, _, _)| *v == view) else { return (0.0, 0.0) };
        let (Some(now), Some((t0, t1))) = (now, self.at) else { return total };
        let tick = (t1 - t0).as_secs_f64();
        let into = now.saturating_duration_since(t1).as_secs_f64();
        let f = if tick > 0.0 { (into / tick).min(1.0) } else { 1.0 };
        (before.0 + (total.0 - before.0) * f, before.1 + (total.1 - before.1) * f)
    }
}

/// Edge scrolling's state shared by the layout (main thread: aims it, takes
/// what it scrolled) and the pointer thread (ticks it).
#[derive(Default)]
pub struct EdgeClock {
    target: Latest<EdgeTarget>,
    totals: Latest<Totals>,
}

impl std::fmt::Debug for EdgeClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EdgeClock").field("target", &self.target.load()).field("totals", &self.totals.load()).finish()
    }
}

impl EdgeClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Scroll `target` from the next tick on; `None`: nothing.
    pub fn aim(&self, target: Option<EdgeTarget>) {
        match target {
            Some(t) if self.target.load().as_deref() != Some(&t) => self.target.publish(t),
            Some(_) => {}
            None => self.target.clear(),
        }
    }

    pub fn totals(&self) -> Arc<Totals> {
        self.totals.load().unwrap_or_default()
    }

    /// One tick at `now`, `dt` seconds after the last, the pointer at window
    /// pixel `pointer` (if known). Only the pointer thread calls it.
    pub fn step(&self, pointer: Option<(f64, f64)>, dt: f64, now: Instant) {
        let target = self.target.load();
        let mut totals = Totals::clone(&self.totals());
        totals.ticks += 1;
        totals.at = Some((totals.at.map_or(now, |(_, last)| last), now));
        for (_, total, before) in &mut totals.moved {
            *before = *total;
        }
        let edge = target.as_deref().zip(pointer).map(|(t, p)| (t, t.edge_at(p)));
        totals.edge = edge.filter(|(_, e)| *e != (0, 0)).map(|(t, e)| (t.view, e));
        if let Some((t, e)) = edge.filter(|(_, e)| *e != (0, 0)) {
            let by = t.step(e, dt);
            match totals.moved.iter_mut().find(|(v, _, _)| *v == t.view) {
                Some((_, m, _)) => *m = (m.0 + by.0, m.1 + by.1),
                None => totals.moved.push((t.view, by, (0.0, 0.0))),
            }
        }
        self.totals.publish(totals);
    }
}

impl OnTick for EdgeClock {
    fn tick(&self, tick: Tick) {
        self.step(tick.pointer, tick.dt, tick.at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_edge_zone_counts_from_the_edges_the_view_reaches() {
        // Both edges reached: 1 at the outermost pixel, 48 at the zone's end
        // (three times the game's 16).
        assert_eq!(EDGE_ZONE, 48);
        assert_eq!(edge_axis(0, 800, true, true), -1);
        assert_eq!(edge_axis(47, 800, true, true), -48);
        assert_eq!(edge_axis(48, 800, true, true), 0);
        assert_eq!(edge_axis(799, 800, true, true), 1);
        assert_eq!(edge_axis(752, 800, true, true), 48);
        assert_eq!(edge_axis(751, 800, true, true), 0);
        // An edge the view does not reach (the divider's side of a pane).
        assert_eq!(edge_axis(799, 800, true, false), 0);
        assert_eq!(edge_axis(0, 800, false, true), 0);
    }

    /// Pushed past the window it is the outermost pixel (full speed), not
    /// out of the zone: until a drag is well past the window.
    #[test]
    fn a_pointer_pushed_past_an_edge_is_on_it() {
        for v in [-1, -2, -PUSH_MARGIN] {
            assert_eq!(edge_axis(v, 800, true, true), -1, "{v}");
            assert_eq!(edge_axis(799 - v, 800, true, true), 1, "{v}");
        }
        assert_eq!(edge_axis(-PUSH_MARGIN - 1, 800, true, true), 0);
        assert_eq!(edge_axis(800 + PUSH_MARGIN, 800, true, true), 0);
        // Not an edge the view reaches: still nothing.
        assert_eq!(edge_axis(-3, 800, false, true), 0);
    }

    fn target() -> EdgeTarget {
        EdgeTarget {
            view: 0,
            window: (800, 600),
            area: [0, 0, 800, 600],
            rect: [0, 0, 800, 600],
            top_row: None,
            against: None,
            interface: vec![[0, 500, 200, 100]],
            full: (100.0, 100.0),
            present: None,
        }
    }

    #[test]
    fn a_target_scrolls_at_every_edge_and_corner_and_thin_over_the_interface() {
        let t = target();
        assert_eq!(t.edge_at((0.0, 300.0)), (-1, 0));
        assert_eq!(t.edge_at((-30.0, 300.0)), (-1, 0));
        assert_eq!(t.edge_at((799.0, 300.0)), (1, 0));
        assert_eq!(t.edge_at((840.0, 300.0)), (1, 0));
        assert_eq!(t.edge_at((400.0, -5.0)), (0, -1));
        assert_eq!(t.edge_at((400.0, 640.0)), (0, 1));
        assert_eq!(t.edge_at((-9.0, -9.0)), (-1, -1));
        assert_eq!(t.edge_at((900.0, 700.0)), (0, 0), "a drag far outside");
        // Over the interface, 20 px in: beyond the thin zone, nothing.
        assert_eq!(t.edge_at((100.0, 579.0)), (0, 0));
        assert_eq!(t.edge_at((100.0, 599.0)), (0, 1));
        assert_eq!(t.edge_at((100.0, 640.0)), (0, 1), "pushed out under the interface");
        // Against the divider's wall.
        let t = EdgeTarget { against: Some((0, 1)), rect: [0, 0, 396, 600], area: [0, 0, 396, 600], ..target() };
        assert_eq!(t.edge_at((395.0, 300.0)), (1, 0));
        assert_eq!(t.edge_at((500.0, 300.0)), (1, 0), "past the wall: at the wall");
    }

    /// Pushing into an edge with a stream of positions at and past it
    /// (as warps, clamps and drags report them) scrolls at full speed on
    /// every tick.
    #[test]
    fn pushing_into_the_edge_keeps_full_speed_on_every_tick() {
        let clock = EdgeClock::new();
        clock.aim(Some(target()));
        let dt = 1.0 / 240.0;
        let mut last = (0.0, 0.0);
        for (i, x) in [0.0, -1.0, -4.0, 0.0, -2.5, 0.0, -48.0, 0.0].into_iter().cycle().take(240).enumerate() {
            clock.step(Some((x, 300.0)), dt, Instant::now());
            let now = clock.totals().of(0);
            let speed = (now.0 - last.0) / dt;
            assert!((speed + 100.0).abs() < 1e-6, "tick {i} at x {x}: {speed}");
            last = now;
        }
        assert!((last.0 + 100.0).abs() < 1e-6, "{last:?}");
    }

    #[test]
    fn no_target_or_no_pointer_scrolls_nothing() {
        let clock = EdgeClock::new();
        clock.step(Some((0.0, 0.0)), 0.01, Instant::now());
        assert_eq!(clock.totals().of(0), (0.0, 0.0));
        clock.aim(Some(target()));
        clock.step(None, 0.01, Instant::now());
        assert_eq!(clock.totals().of(0), (0.0, 0.0));
        clock.step(Some((0.0, 300.0)), 0.01, Instant::now());
        assert_eq!(clock.totals().of(0), (-1.0, 0.0));
        clock.aim(None);
        clock.step(Some((0.0, 300.0)), 0.01, Instant::now());
        assert_eq!(clock.totals().of(0), (-1.0, 0.0));
    }
}
