//! What a frame moves on in time, carried over to the live layout once that
//! frame is presented.
//!
//! The window prepares every frame on its live layout and then puts the
//! layout back as it was (`ui_submission::Mapping`). Once the frame is
//! presented, its layout is taken whole if no input came meanwhile; else
//! the live layout keeps that input and takes over only what the frame
//! decided ([`Layout::adopt_presented_mapping`]). Motion a frame moved on
//! (a view gliding, flying or sliding, edge scrolling, the zoom boxes'
//! glides, the frame clock) is the frame's to decide too: left behind, the
//! next frame would move it on again from where it was before this one,
//! and frames of uneven length would show it going back and forth.
//!
//! All of that is in [`Motion`]. It is taken as each frame begins (the
//! state the frame was prepared from) and, once presented, carried over
//! three-way, field by field ([`carry`]): what input changed since the
//! frame was prepared stays as input left it; what input did not change
//! takes the frame's value.

use super::{FreeView, Layout, ZoomBoxMotion, ZoomMouseMotion, pan::Pan};

/// `live` (the layout input moved on) after a frame prepared from `base`
/// left it at `frame`: as input left it if input changed it, else the
/// frame's.
pub(super) fn carry<T: PartialEq>(live: &mut T, base: &T, frame: T) {
    if *live == *base {
        *live = frame;
    }
}

/// As [`carry`], field by field (`fields`) while there is one in all three
/// (a flight input turned elsewhere while the frame moved it on: where it
/// goes from input, how far it got from the frame).
pub(super) fn carry_some<T: PartialEq>(live: &mut Option<T>, base: &Option<T>, frame: Option<T>, fields: impl FnOnce(&mut T, &T, T)) {
    match (live.as_mut(), base, frame) {
        (Some(l), Some(b), Some(f)) => fields(l, b, f),
        (_, _, f) => carry(live, base, f),
    }
}

/// As [`carry`], per entry of a list with one entry per `key` (each
/// view's own), in any order.
pub(super) fn carry_keyed<T: PartialEq + Copy, K: PartialEq>(live: &mut Vec<T>, base: &[T], frame: Vec<T>, key: impl Fn(&T) -> K) {
    let find = |list: &[T], k: &K| list.iter().find(|e| key(e) == *k).copied();
    let mut keys: Vec<K> = Vec::new();
    for e in live.iter().chain(base).chain(&frame) {
        let k = key(e);
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    *live = keys
        .iter()
        .filter_map(|k| {
            let mut l = find(live, k);
            carry(&mut l, &find(base, k), find(&frame, k));
            l
        })
        .collect();
}

/// [`carry`] for every field of a struct: `carry_fields!(Type { a, b }, live,
/// base, frame)`. It names every field (no `..`), so a field added to the
/// struct does not compile until it is named here, carried as all others.
macro_rules! carry_fields {
    ($ty:ident { $($field:ident),* $(,)? }, $live:expr, $base:expr, $frame:expr) => {{
        let (live, base) = ($live, $base);
        let $ty { $($field),* } = $frame;
        $( $crate::layout::prepared_motion::carry(&mut live.$field, &base.$field, $field); )*
    }};
}
pub(super) use carry_fields;

/// Everything a frame moves on in time, as one frame found it or left it.
/// A new kind of motion a frame moves on belongs here.
#[derive(Clone, Debug)]
pub(super) struct Motion {
    /// The time not yet taken by a frame, and the frames' clock.
    step: f64,
    clock: f64,
    /// The views' centres and glides (A, B with a split, the insets),
    /// each whole ([`FreeView`]) and carried field by field.
    main_view: FreeView,
    pane_view: Option<FreeView>,
    inset_views: Vec<FreeView>,
    zoom_box_motion: Option<ZoomBoxMotion>,
    zoom_mouse_motion: Option<ZoomMouseMotion>,
    /// Map movement: flights, slides, edge scrolling, drags' trackers
    /// ([`Pan::carry`]).
    pan: Pan,
}

impl FreeView {
    fn carry(&mut self, base: &Self, frame: Self) {
        carry_fields!(FreeView { center, zoom, follow, settling, glide, hand_focus, scale_rest }, self, base, frame);
    }
}

impl Layout {
    /// The motion as it is now.
    pub(super) fn prepared_motion(&self) -> Motion {
        Motion {
            step: self.step,
            clock: self.clock,
            main_view: self.main_view,
            pane_view: self.split.map(|s| s.view),
            inset_views: self.insets.iter().map(|i| i.view).collect(),
            zoom_box_motion: self.zoom_box_motion,
            zoom_mouse_motion: self.zoom_mouse_motion,
            pan: self.pan.clone(),
        }
    }

    /// Takes the motion of a frame prepared from `base` and presented as
    /// `frame` into this live layout (see the module's documentation).
    pub(super) fn carry_motion(&mut self, base: &Motion, frame: Motion) {
        let Motion { step, clock, main_view, pane_view, inset_views, zoom_box_motion, zoom_mouse_motion, pan } = frame;
        carry(&mut self.step, &base.step, step);
        carry(&mut self.clock, &base.clock, clock);
        self.main_view.carry(&base.main_view, main_view);
        // (Views come and go with the settings, not with a frame: a view
        // only the live layout or only the frame has stays as it is live.)
        if let (Some(live), Some(b), Some(f)) = (self.split.as_mut(), &base.pane_view, pane_view) {
            live.view.carry(b, f);
        }
        if self.insets.len() == base.inset_views.len() && inset_views.len() == base.inset_views.len() {
            for ((live, b), f) in self.insets.iter_mut().zip(&base.inset_views).zip(inset_views) {
                live.view.carry(b, f);
            }
        }
        carry(&mut self.zoom_box_motion, &base.zoom_box_motion, zoom_box_motion);
        carry(&mut self.zoom_mouse_motion, &base.zoom_mouse_motion, zoom_mouse_motion);
        self.pan.carry(&base.pan, pan);
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use std::time::Duration;
    use viewer_settings::{MapMovement, Split as S};

    const BOUNDS: Option<WorldBounds> = Some(WorldBounds { size: (8000, 4000), wrap: (true, false) });
    /// 200 × 100 of the 8000 × 4000 world: 40 world pixels a minimap pixel.
    const MINIMAP: scene::Minimap = scene::Minimap { at: (0, 500), size: (200, 100), shown: [0, 500, 200, 100], world: (8000, 4000) };
    /// Frames of uneven length, as a real window has them.
    const FRAME_MS: [u64; 4] = [16, 7, 25, 11];

    /// The driver's view at (`x`, 500), 800 × 600, the interface over it,
    /// a minimap at its bottom left.
    fn scene_at(x: i32) -> Scene {
        let main = scene::View { camera_x: x, camera_y: 500, zoom: (1, 1), ..Default::default() };
        let ui = scene::View { interface_only: true, ..Default::default() };
        Scene {
            views: vec![scene::ViewState { id: 0, view: main, overlay: vec![] }, scene::ViewState { id: 255, view: ui, overlay: vec![] }],
            focus: Some((3000, 2000)),
            shape: scene::WorldShape { cols: 62, rows: 125, wrap_x: true, wrap_y: false },
            minimap: Some(MINIMAP),
            map_wheel_step: (128, 64),
            ..Default::default()
        }
    }

    /// A and B side by side (A 0..400, B 404..800), smooth movement on.
    fn split(roles: Roles, movement: MapMovement) -> Layout {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: S::Vertical1To1, roles, smooth_world_movement: true, ..Default::default() });
        l.set_map_movement(movement);
        for _ in 0..2 {
            l.frame(&scene_at(0), (800, 600), (0, 0, 0, 0), vec![[0, 500, 200, 100]], BOUNDS);
        }
        l
    }

    fn mv(l: &mut Layout, p: (i32, i32)) {
        l.route(Input::MouseMove { x: p.0, y: p.1 });
    }

    fn button(l: &mut Layout, button: Button, down: bool, p: (i32, i32)) {
        l.route(Input::MouseButton { button, down, x: p.0, y: p.1 });
    }

    /// As the window does it: a frame (the `k`th, of uneven length)
    /// prepared on the live layout, which is then put back as it was;
    /// `input` comes while the frame is in flight, so once presented only
    /// what the frame decided is taken over
    /// ([`Layout::adopt_presented_mapping`]). What the frame showed, by
    /// `shown`.
    fn presented<T>(l: &mut Layout, s: &Scene, k: usize, shown: impl Fn(&Layout) -> T, input: impl FnOnce(&mut Layout)) -> T {
        let before = l.clone();
        l.advance(Duration::from_millis(FRAME_MS[k % FRAME_MS.len()]));
        l.frame(s, (800, 600), (0, 0, 0, 0), vec![[0, 500, 200, 100]], BOUNDS);
        let what = shown(l);
        let prepared = std::mem::replace(l, before);
        input(l);
        l.adopt_presented_mapping(prepared);
        what
    }

    fn b(l: &Layout) -> (i32, i32) {
        l.split.unwrap().view.center
    }

    /// Never back: each axis moves one way only (or not at all).
    fn one_way(shown: &[(i32, i32)]) -> bool {
        let way = |d: i32, w: &mut i32| {
            let s = d.signum();
            let ok = s == 0 || *w == 0 || s == *w;
            if s != 0 {
                *w = s;
            }
            ok
        };
        let (mut wx, mut wy) = (0, 0);
        shown.windows(2).all(|p| way(p[1].0 - p[0].0, &mut wx) & way(p[1].1 - p[0].1, &mut wy))
    }

    /// A pointer wandering over A, somewhere new every frame.
    fn wander(k: usize) -> (i32, i32) {
        (100 + (k as i32 * 7) % 200, 100 + (k as i32 * 5) % 200)
    }

    #[test]
    fn a_minimap_flight_of_b_goes_steadily_on_while_the_pointer_moves() {
        let s = scene_at(0);
        let mut l = split(Roles::Free, MapMovement::Drag);
        let from = b(&l);
        let to = MINIMAP.world_at((100, 550)).unwrap();
        assert_eq!(to, (4020, 2020));
        assert!(from.0 < to.0 && from.1 < to.1, "{from:?}");
        mv(&mut l, (100, 550));
        button(&mut l, Button::Right, true, (100, 550));
        button(&mut l, Button::Right, false, (100, 550));
        assert!(l.pan_holds(Hit::Pane), "flying");
        let mut shown = vec![from];
        for k in 0..200 {
            shown.push(presented(&mut l, &s, k, b, |l| mv(l, wander(k))));
        }
        assert!(one_way(&shown), "back and forth: {shown:?}");
        assert_eq!(b(&l), to, "{shown:?}");
        assert!(!l.pan_holds(Hit::Pane), "landed");
    }

    #[test]
    fn a_right_drag_along_the_minimap_turns_b_steadily_while_frames_are_adopted() {
        let s = scene_at(0);
        let mut l = split(Roles::Free, MapMovement::Drag);
        mv(&mut l, (100, 550));
        button(&mut l, Button::Right, true, (100, 550));
        // Every frame the pointer goes on along the minimap (B's flight turns
        // there); then it rests on it.
        let at = |k: usize| (100 + k.min(40) as i32, 550);
        let mut shown = vec![b(&l)];
        for k in 0..240 {
            shown.push(presented(&mut l, &s, k, b, |l| mv(l, at(k))));
        }
        button(&mut l, Button::Right, false, at(240));
        assert!(one_way(&shown), "back and forth: {shown:?}");
        assert_eq!(b(&l), MINIMAP.world_at(at(240)).unwrap(), "{shown:?}");
    }

    #[test]
    fn a_view_let_go_slides_steadily_on_while_the_pointer_moves() {
        let s = scene_at(0);
        let mut l = split(Roles::Free, MapMovement::Drag);
        // B dragged left, frame by frame, and let go moving.
        let start = b(&l);
        button(&mut l, Button::Left, true, (700, 300));
        for k in 0..10 {
            presented(&mut l, &s, k, b, |l| mv(l, (690 - 20 * k as i32, 300)));
        }
        let dragged = presented(&mut l, &s, 10, b, |l| button(l, Button::Left, false, (500, 300)));
        assert!(dragged.0 > start.0 + 150, "{start:?} {dragged:?}");
        let mut shown = vec![dragged];
        for k in 0..240 {
            shown.push(presented(&mut l, &s, k, b, |l| mv(l, wander(k))));
        }
        assert!(one_way(&shown), "back and forth: {shown:?}");
        let slid = shown.last().unwrap().0 - dragged.0;
        assert!(slid > 100, "slid {slid}: {shown:?}");
        assert!(!l.pan_holds(Hit::Pane), "stopped");
    }

    #[test]
    fn a_view_gliding_after_the_game_goes_steadily_on_while_the_pointer_moves() {
        let mut l = split(Roles::Free, MapMovement::Drag);
        // The game scrolls its view 1600 pixels at once: A glides there.
        let s = scene_at(1600);
        let from = l.main_view.center;
        let mut shown = vec![from];
        for k in 0..200 {
            shown.push(presented(&mut l, &s, k, |l| l.main_view.center, |l| mv(l, (600 + (k as i32 * 7) % 150, 100 + (k as i32 * 5) % 200))));
        }
        assert!(one_way(&shown), "back and forth: {shown:?}");
        assert_eq!(l.main_view.center, (from.0 + 1600, from.1), "{shown:?}");
    }

    #[test]
    fn edge_scrolling_goes_steadily_on_while_the_pointer_moves_along_the_edge() {
        let s = scene_at(0);
        let mut l = split(Roles::Free, MapMovement::EdgeScroll);
        let from = b(&l);
        let mut shown = vec![from];
        let mut ms = 0;
        mv(&mut l, (799, 200));
        for k in 0..60 {
            ms += FRAME_MS[k % FRAME_MS.len()];
            shown.push(presented(&mut l, &s, k, b, |l| mv(l, (799, 200 + k as i32 % 50))));
        }
        assert!(one_way(&shown), "back and forth: {shown:?}");
        // As far as the game's own speed goes in that time.
        let gone = (b(&l).0 - from.0) as f64;
        let want = 128.0 / pan::GAME_EDGE_TICK * ms as f64 / 1000.0;
        assert!((gone - want).abs() <= 1.0, "{gone} vs {want}: {shown:?}");
    }

    #[test]
    fn the_frame_clock_runs_on_through_frames_presented_after_input() {
        let s = scene_at(0);
        let mut l = split(Roles::Free, MapMovement::Drag);
        let start = l.clock;
        let mut ms = 0;
        for k in 0..20 {
            ms += FRAME_MS[k % FRAME_MS.len()];
            presented(&mut l, &s, k, |_| (), |l| mv(l, wander(k)));
        }
        assert!((l.clock - start - ms as f64 / 1000.0).abs() < 1e-9, "{} vs {ms} ms", l.clock - start);
    }

    #[test]
    fn input_while_a_frame_is_in_flight_wins_over_what_the_frame_moved() {
        let s = scene_at(0);
        let mut l = split(Roles::Free, MapMovement::Drag);
        mv(&mut l, (100, 550));
        button(&mut l, Button::Right, true, (100, 550));
        button(&mut l, Button::Right, false, (100, 550));
        presented(&mut l, &s, 0, b, |_| {});
        // A drag of B starting while the frame is in flight ends the
        // flight: the frame flying it on does not bring it back.
        let held = presented(&mut l, &s, 1, b, |l| {
            button(l, Button::Left, true, (600, 300));
            mv(l, (620, 300));
        });
        assert!(l.pan_holds(Hit::Pane));
        let grabbed = b(&l);
        assert_ne!(grabbed, held, "the frame flew B on, the grab held it before");
        for k in 2..60 {
            presented(&mut l, &s, k, b, |_| {});
        }
        assert_eq!(b(&l), grabbed, "still held where it was grabbed");
        button(&mut l, Button::Left, false, (620, 300));
    }
}
