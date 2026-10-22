//! The capture view's attractor and commands that wait for a place of the
//! world (`scene::Interface::targeting`).

use super::*;
use scene::{Scene, View, ViewState};

const ALT: u32 = 0x12;
const ESC: u32 = 0x1B;
const TICK: f64 = 1.0 / 120.0;

/// The driver's main view and its interface, a strip at the bottom with
/// two buttons, A at (130, 535) and B at (330, 535); a world that does not
/// wrap north and south.
fn scene() -> Scene {
    let main = View { camera_x: 1000, camera_y: 500, zoom: (1, 1), ..Default::default() };
    let ui = View { interface_only: true, ..Default::default() };
    Scene {
        seq: 1,
        views: vec![ViewState { id: 0, view: main, overlay: vec![] }, ViewState { id: 255, view: ui, overlay: vec![] }],
        shape: scene::WorldShape { cols: 62, rows: 125, wrap_x: true, wrap_y: false },
        interactables: vec![
            Interactable { alt_right_click: false, anchor: None, rect: [100, 520, 60, 30], kind: InteractableKind::Button, id: 1 },
            Interactable { alt_right_click: false, anchor: None, rect: [300, 520, 60, 30], kind: InteractableKind::Button, id: 2 },
        ],
        ..Default::default()
    }
}

fn frame(l: &mut Layout, s: &Scene) {
    let bounds = crate::layout::WorldBounds { size: (8000, 4000), wrap: (true, false) };
    l.frame(s, (800, 600), (0, 0, 0, 0), vec![[0, 500, 800, 100]], Some(bounds));
}

fn layout(spec: &str, s: &Scene) -> Layout {
    let mut l = Layout::default();
    l.configure(spec).unwrap();
    frame(&mut l, s);
    frame(&mut l, s);
    l
}

fn key(vk: u32, down: bool) -> Input {
    Input::Key { vk, down, text: None }
}

fn click(snap: &mut SnapInput, l: &mut Layout, s: &Scene, now: f64) -> Vec<Input> {
    let mut out = snap.route(Input::MouseButton { button: Button::Left, down: true, x: 0, y: 0 }, l, s, now, Rates::default());
    out.extend(snap.route(Input::MouseButton { button: Button::Left, down: false, x: 0, y: 0 }, l, s, now + 0.05, Rates::default()));
    out
}

/// Ticks from `from` to `to`, with `motion` raw counts a tick while
/// `t < until`.
fn steer(snap: &mut SnapInput, l: &mut Layout, s: &Scene, from: f64, to: f64, motion: Point, until: f64) -> Vec<Input> {
    let mut out = Vec::new();
    let mut t = from;
    while t < to {
        t += TICK;
        if t < until {
            assert_eq!(snap.motion(motion), Some(vec![]));
        }
        out.extend(snap.tick(l, s, t, Rates::default()));
    }
    out
}

fn no_world(inputs: &[Input]) -> bool {
    inputs.iter().all(|i| !matches!(i, Input::WorldButton { .. } | Input::Look { .. } | Input::WorldKey { .. } | Input::WorldMove { .. }))
}

fn middle([x, y, w, h]: [i32; 4]) -> (i32, i32) {
    (x + w / 2, y + h / 2)
}

const LAYOUTS: [&str; 5] = [
    "main=pointer",
    "split=vertical,main=focus,pane=pointer",
    "split=horizontal,main=focus,pane=pointer",
    "split=vertical,main=pointer,pane=focus",
    "split=horizontal,main=pointer,pane=focus",
];

#[test]
fn each_view_of_a_split_has_an_attractor_at_its_middle_in_every_layout() {
    let s = scene();
    for spec in LAYOUTS {
        let l = layout(spec, &s);
        let (rect, at) = l.capture_view().unwrap_or_else(|| panic!("{spec}: no capture view"));
        let captures: Vec<Target> = SnapInput::targets(&s, &l).into_iter().filter(Target::is_capture).collect();
        let split = spec.starts_with("split");
        assert_eq!(captures.len(), if split { 2 } else { 1 }, "{spec}: one attractor a view");
        // The capture view's first, with the id of old.
        let c = captures[0].center();
        assert_eq!((c.0 as i32, c.1 as i32), middle(rect), "{spec}");
        assert_eq!(at, middle(rect), "{spec}");
        assert_eq!(captures[0].id, CAPTURE_ID, "{spec}");
        assert_eq!(captures[0].rect, [rect[0] as f64, rect[1] as f64, rect[2] as f64, rect[3] as f64], "{spec}: the capture view's");
        let attractors = l.attractors();
        assert!(attractors[0].captures, "{spec}");
        if split {
            // The other view's, at its middle; a click there does not capture.
            let other = attractors[1];
            assert!(!other.captures, "{spec}");
            assert_eq!(other.at, middle(other.rect), "{spec}");
            assert_ne!(other.rect, rect, "{spec}");
            assert_eq!(captures[1].id, CAPTURE_ID - 1, "{spec}");
            assert_eq!(captures[1].center(), (other.at.0 as f64, other.at.1 as f64), "{spec}");
        }
    }
    // One view not following the world cursor: none; split, both views.
    let l = layout("main=focus", &s);
    assert!(l.capture_view().is_none());
    assert!(!SnapInput::targets(&s, &l).iter().any(Target::is_capture));
    let l = layout("split=vertical,main=focus,pane=focus", &s);
    assert_eq!(l.attractors().iter().filter(|a| !a.captures).count(), 2);
}

/// Alt pressed with the pointer at the strip's button B: the mode on (it
/// starts on the driver's element nearest, never on an attractor), then
/// one motion from there to window pixel `to`, an attractor's point: the
/// pointer at rest there by 2 s.
fn alt_at(l: &mut Layout, s: &Scene, to: (i32, i32)) -> SnapInput {
    l.route(Input::MouseMove { x: 330, y: 535 });
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), l, s, 0.0, Rates::default());
    snap.route(key(ALT, false), l, s, 0.0, Rates::default());
    assert!(snap.active());
    steer(&mut snap, l, s, 0.0, 1.0, (0.0, 0.0), 0.0);
    snap_to(&mut snap, l, s, 1.0, to);
    assert_eq!(snap.snapper.target(), Some((to.0 as f64, to.1 as f64)));
    snap
}

/// One motion from where the pointer is to `to`, as long and as far as
/// that (picks the element there, `pick`), then a second at rest.
fn snap_to(snap: &mut SnapInput, l: &mut Layout, s: &Scene, now: f64, to: (i32, i32)) -> Vec<Input> {
    let from = snap.snapper.target().unwrap();
    assert_eq!(snap.motion(((to.0 as f64 - from.0) / MOTION_GAIN, (to.1 as f64 - from.1) / MOTION_GAIN)), Some(vec![]));
    steer(snap, l, s, now, now + 1.0, (0.0, 0.0), 0.0)
}

#[test]
fn the_other_views_attractor_puts_the_free_pointer_there_and_nothing_goes_to_the_game() {
    let s = scene();
    for spec in ["split=vertical,main=focus,pane=pointer", "split=horizontal,main=pointer,pane=focus"] {
        let mut l = layout(spec, &s);
        let other = l.attractors()[1];
        let mut snap = alt_at(&mut l, &s, other.at);
        assert_eq!(snap.snapper.target_id(), Some(CAPTURE_ID - 1), "{spec}");
        let _ = l.take_warp();
        let out = click(&mut snap, &mut l, &s, 2.5);
        assert!(out.is_empty(), "{spec}: {out:?}");
        assert!(!l.captured() && !snap.active() && !l.snapping(), "{spec}");
        assert_eq!(l.take_warp(), Some(other.at), "{spec}: the system pointer there");
        // Ticks after: nothing for the game, no attractors drawn.
        assert!(no_world(&steer(&mut snap, &mut l, &s, 2.6, 3.0, (0.0, 0.0), 0.0)));
        assert!(snap.attractor_points().is_empty(), "{spec}");
    }
}

#[test]
fn with_drag_or_under_a_popup_the_capture_views_attractor_does_not_capture() {
    let s = scene();
    // Drag: the free pointer goes there.
    let mut l = layout("split=vertical,main=focus,pane=pointer", &s);
    l.set_map_movement(viewer_settings::MapMovement::Drag);
    frame(&mut l, &s);
    let (rect, _) = l.capture_view().unwrap();
    assert!(!l.attractors()[0].captures);
    let mut snap = alt_at(&mut l, &s, middle(rect));
    let _ = l.take_warp();
    assert!(click(&mut snap, &mut l, &s, 2.5).is_empty());
    assert!(!l.captured() && !snap.active());
    assert_eq!(l.take_warp(), Some(middle(rect)));
}

#[test]
fn attractors_are_drawn_while_the_mode_is_on_and_the_mouse_free() {
    let s = scene();
    let mut l = layout("split=vertical,main=focus,pane=pointer", &s);
    let points: Vec<(i32, i32)> = l.attractors().iter().map(|a| a.at).collect();
    l.route(Input::MouseMove { x: 330, y: 560 });
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    snap.route(key(ALT, false), &mut l, &s, 0.0, Rates::default());
    steer(&mut snap, &mut l, &s, 0.0, 1.0, (0.0, 0.0), 0.0);
    let drawn: Vec<(i32, i32)> = snap.attractor_points().iter().map(|&((x, y), _)| (x as i32, y as i32)).collect();
    assert_eq!(drawn, points);
    assert_eq!(snap.attractor_points().iter().map(|a| a.1).collect::<Vec<_>>(), [true, false], "B's captures, A's not");
    // Off: none.
    snap.route(key(ALT, true), &mut l, &s, 1.5, Rates::default());
    snap.route(key(ALT, false), &mut l, &s, 1.5, Rates::default());
    steer(&mut snap, &mut l, &s, 1.5, 2.5, (0.0, 0.0), 0.0);
    assert!(snap.attractor_points().is_empty());
    // On while captured: none drawn (the capture view is still a target).
    let (rect, _) = l.capture_view().unwrap();
    l.route(Input::MouseButton { button: Button::Left, down: true, x: middle(rect).0, y: middle(rect).1 });
    l.route(Input::MouseButton { button: Button::Left, down: false, x: middle(rect).0, y: middle(rect).1 });
    assert!(l.captured());
    snap.route(key(ALT, true), &mut l, &s, 3.0, Rates::default());
    snap.route(key(ALT, false), &mut l, &s, 3.0, Rates::default());
    steer(&mut snap, &mut l, &s, 3.0, 3.5, (0.0, 0.0), 0.0);
    assert!(snap.active() && snap.attractor_points().is_empty());
}

#[test]
fn the_attractor_stays_in_the_middle_while_the_world_cursor_is_off_it() {
    // Captured, the world cursor pushed to the world's north edge: the
    // view is held there, the world cursor above its middle.
    let s = scene();
    let mut l = layout("main=pointer", &s);
    let (rect, _) = l.capture_view().unwrap();
    l.route(Input::MouseButton { button: Button::Left, down: true, x: middle(rect).0, y: middle(rect).1 });
    l.route(Input::MouseButton { button: Button::Left, down: false, x: middle(rect).0, y: middle(rect).1 });
    assert!(l.captured());
    for _ in 0..40 {
        l.motion((0.0, -200.0));
        frame(&mut l, &s);
    }
    let cursor = l.world_cursor_view().unwrap().1;
    assert!(cursor.1 < middle(rect).1 - 20, "{cursor:?}: off the middle");
    assert_eq!(l.capture_view(), Some((rect, middle(rect))));
}

#[test]
fn the_attractor_draws_the_pointer_past_elements_off_the_way() {
    // From B up towards the middle of the view, C lies near but 42° off
    // the motion, the middle 17° off and far: the middle is picked.
    let mut t = targets(&[Interactable { alt_right_click: false, anchor: None, rect: [270, 470, 20, 20], kind: InteractableKind::Button, id: 3 }], ((0, 0), (1, 1)), (800, 600));
    t.push(Target::capture([0, 0, 800, 600], (400, 300)));
    assert_eq!(pick(&t, (330.0, 535.0), Some(2), (0.0, -60.0)).map(|k| t[k].id), Some(CAPTURE_ID));
    // As a button there it would lose to C.
    t[1].kind = TargetKind::Button;
    assert_eq!(pick(&t, (330.0, 535.0), Some(2), (0.0, -60.0)).map(|k| t[k].id), Some(3));
}

#[test]
fn clicking_the_attractor_recaptures_and_nothing_goes_to_the_world() {
    let s = scene();
    for spec in LAYOUTS {
        let mut l = layout(spec, &s);
        let (rect, _) = l.capture_view().unwrap();
        l.route(Input::MouseMove { x: 330, y: 560 });
        let mut snap = SnapInput::default();
        snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
        snap.route(key(ALT, false), &mut l, &s, 0.0, Rates::default());
        assert!(snap.active(), "{spec}");
        steer(&mut snap, &mut l, &s, 0.0, 1.0, (0.0, 0.0), 0.0);
        // Towards the capture view's middle (as far: in a split the other
        // view's attractor may lie the same way).
        let to = middle(rect);
        let out = snap_to(&mut snap, &mut l, &s, 1.0, to);
        assert_eq!(snap.snapper.target_id(), Some(CAPTURE_ID), "{spec}");
        assert_eq!(snap.snapper.target(), Some((to.0 as f64, to.1 as f64)), "{spec}");
        assert!(no_world(&out), "{spec}: {out:?}");
        let out = click(&mut snap, &mut l, &s, 2.5);
        assert!(out.is_empty(), "{spec}: {out:?}");
        assert!(l.captured() && !snap.active() && !l.snapping(), "{spec}");
        assert_eq!(l.take_grab(), Some(true), "{spec}");
    }
}

#[test]
fn a_command_waiting_for_a_place_recaptures_where_the_player_looks_and_targets_that_tile() {
    let mut s = scene();
    for spec in LAYOUTS {
        let mut l = layout(spec, &s);
        let (rect, _) = l.capture_view().unwrap();
        l.route(Input::MouseMove { x: 330, y: 560 });
        let mut snap = SnapInput::default();
        snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
        snap.route(key(ALT, false), &mut l, &s, 0.0, Rates::default());
        steer(&mut snap, &mut l, &s, 0.0, 0.5, (0.0, 0.0), 0.0);
        assert!(snap.active() && !l.captured(), "{spec}");
        // Its button clicked (or its key): the game waits for a tile.
        let out = click(&mut snap, &mut l, &s, 0.5);
        assert!(no_world(&out) && snap.active(), "{spec}: {out:?}");
        let before = l.world_cursor_view();
        let _ = l.take_warp();
        s.interface.targeting = true;
        let out = steer(&mut snap, &mut l, &s, 0.6, 0.7, (0.0, 0.0), 0.0);
        assert!(!snap.active() && !l.snapping() && l.captured(), "{spec}");
        assert_eq!(l.take_grab(), Some(true), "{spec}");
        // The view and the world cursor stay; the system pointer goes back
        // to the view's middle.
        assert_eq!(l.world_cursor_view(), before, "{spec}");
        assert_eq!(l.take_warp(), Some(middle(rect)), "{spec}");
        // No click; the tile under the world cursor is the target at once.
        assert!(out.iter().all(|i| !matches!(i, Input::MouseButton { .. } | Input::WorldButton { .. })), "{spec}: {out:?}");
        let moves: Vec<&Input> = out.iter().filter(|i| matches!(i, Input::MouseMove { .. } | Input::WorldMove { .. })).collect();
        assert_eq!(moves.len(), 1, "{spec}: {out:?}");
        let cursor = before.unwrap().1;
        // (What the pointer over the world cursor's place gives.)
        let mut probe = layout(spec, &s);
        probe.route(Input::MouseButton { button: Button::Left, down: true, x: middle(rect).0, y: middle(rect).1 });
        probe.route(Input::MouseButton { button: Button::Left, down: false, x: middle(rect).0, y: middle(rect).1 });
        assert_eq!(probe.world_cursor_view().map(|v| v.1), Some(cursor), "{spec}");
        assert_eq!(probe.pointer_to(cursor), [moves[0].clone()], "{spec}: the target is the tile under the world cursor");
        // The warp coming back as the system pointer's place: no motion of
        // the world cursor.
        snap.route(Input::MouseMove { x: middle(rect).0, y: middle(rect).1 }, &mut l, &s, 0.75, Rates::default());
        assert_eq!(l.world_cursor_view(), before, "{spec}");
        frame(&mut l, &s);
        assert_eq!(l.world_cursor_view(), before, "{spec}: the camera stays");
        // Its motion moves the world cursor.
        assert_eq!(snap.motion((1.0, 0.0)), None);
        // Alt again while the command still waits: the mode, not ended
        // again by the same command.
        snap.route(key(ALT, true), &mut l, &s, 1.0, Rates::default());
        snap.route(key(ALT, false), &mut l, &s, 1.0, Rates::default());
        steer(&mut snap, &mut l, &s, 1.0, 1.2, (0.0, 0.0), 0.0);
        assert!(snap.active(), "{spec}");
        // The game cancels it (Esc): nothing happens; Esc ends the mode.
        s.interface.targeting = false;
        steer(&mut snap, &mut l, &s, 1.2, 1.3, (0.0, 0.0), 0.0);
        assert!(snap.active(), "{spec}");
        snap.route(key(ESC, true), &mut l, &s, 1.3, Rates::default());
        snap.route(key(ESC, false), &mut l, &s, 1.35, Rates::default());
        assert!(!snap.active(), "{spec}");
    }
}

#[test]
fn c_centres_the_capture_view_on_the_active_unit() {
    const C: u32 = 0x43;
    const CTRL: u32 = 0x11;
    let mut s = scene();
    for spec in LAYOUTS {
        let mut l = layout(spec, &s);
        // (Settled: the world cursor placed where the driver first showed
        // its world.)
        for _ in 0..=crate::layout::SETTLE_FRAMES {
            frame(&mut l, &s);
        }
        let (rect, _) = l.capture_view().unwrap();
        // The unit: a world pixel off the capture view's middle.
        let unit_at = (rect[0] + rect[2] / 4, rect[1] + rect[3] / 3);
        l.route(Input::MouseMove { x: unit_at.0, y: unit_at.1 });
        let unit = l.pointer_world().unwrap();
        s.focus = Some(unit);
        let mut snap = SnapInput::default();
        // Ctrl+C is not the game's C.
        snap.route(key(CTRL, true), &mut l, &s, 0.0, Rates::default());
        snap.route(key(C, true), &mut l, &s, 0.0, Rates::default());
        snap.route(key(C, false), &mut l, &s, 0.0, Rates::default());
        snap.route(key(CTRL, false), &mut l, &s, 0.0, Rates::default());
        frame(&mut l, &s);
        l.route(Input::MouseMove { x: middle(rect).0, y: middle(rect).1 });
        assert_ne!(l.pointer_world(), Some(unit), "{spec}");
        // C: the game centres its view; the capture view centres too.
        let out = snap.route(key(C, true), &mut l, &s, 0.1, Rates::default());
        assert!(out.iter().any(|i| matches!(i, Input::Key { vk: C, down: true, .. } | Input::WorldKey { vk: C, down: true, .. })), "{spec}: the game gets it: {out:?}");
        snap.route(key(C, false), &mut l, &s, 0.1, Rates::default());
        for _ in 0..120 {
            frame(&mut l, &s);
        }
        l.route(Input::MouseMove { x: middle(rect).0, y: middle(rect).1 });
        let at = l.pointer_world().unwrap();
        assert!((at.0 - unit.0).abs() <= 1 && (at.1 - unit.1).abs() <= 1, "{spec}: {at:?} vs {unit:?}");
        s.focus = None;
    }
}

#[test]
fn a_command_waiting_already_or_without_the_mode_changes_nothing() {
    // Given with the mouse free and no mode: the mouse stays free.
    let mut s = scene();
    let mut l = layout("main=pointer", &s);
    l.route(Input::MouseMove { x: 330, y: 560 });
    let mut snap = SnapInput::default();
    s.interface.targeting = true;
    steer(&mut snap, &mut l, &s, 0.0, 0.1, (0.0, 0.0), 0.0);
    assert!(!l.captured());
    // The mode turned on while it waits: it stays on.
    snap.route(key(ALT, true), &mut l, &s, 0.2, Rates::default());
    steer(&mut snap, &mut l, &s, 0.2, 0.4, (0.0, 0.0), 0.0);
    assert!(snap.active() && !l.captured());
}
