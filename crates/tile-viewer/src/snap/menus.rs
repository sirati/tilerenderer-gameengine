//! The Alt mode on by itself while a menu is open
//! (`viewer_settings::Input::alt_in_menus`, [`SnapInput::set_alt_in_menus`]).

use super::*;
use scene::{Scene, View, ViewState};

const ALT: u32 = 0x12;
const ESC: u32 = 0x1B;
const TICK: f64 = 1.0 / 60.0;

/// One view following the world cursor, two buttons in the strip at the
/// bottom; `screen` names an open screen (a menu), none: the world.
fn scene(screen: Option<&str>) -> Scene {
    let main = View { camera_x: 1000, camera_y: 500, zoom: (1, 1), ..Default::default() };
    let ui = View { interface_only: true, ..Default::default() };
    let mut s = Scene {
        seq: 1,
        views: vec![ViewState { id: 0, view: main, overlay: vec![] }, ViewState { id: 255, view: ui, overlay: vec![] }],
        shape: scene::WorldShape { cols: 62, rows: 125, wrap_x: true, wrap_y: false },
        interactables: vec![
            Interactable { alt_right_click: false, anchor: None, rect: [100, 520, 60, 30], kind: InteractableKind::Button, id: 1 },
            Interactable { alt_right_click: false, anchor: None, rect: [300, 520, 60, 30], kind: InteractableKind::Button, id: 2 },
        ],
        ..Default::default()
    };
    s.interface.screen = screen.map(String::from);
    s
}

fn show(l: &mut Layout, s: &Scene) {
    let bounds = crate::layout::WorldBounds { size: (8000, 4000), wrap: (true, false) };
    l.frame(s, (800, 600), (0, 0, 0, 0), vec![[0, 500, 800, 100]], Some(bounds));
}

/// Frames and ticks of `s` for `seconds` from `from`; what went to the
/// driver.
fn run(snap: &mut SnapInput, l: &mut Layout, s: &Scene, from: f64, seconds: f64) -> Vec<Input> {
    let mut out = Vec::new();
    let mut t = from;
    while t < from + seconds {
        t += TICK;
        l.advance(std::time::Duration::from_secs_f64(TICK));
        show(l, s);
        out.extend(snap.tick(l, s, t, Rates::default()));
    }
    out
}

fn key(snap: &mut SnapInput, l: &mut Layout, s: &Scene, vk: u32, down: bool, now: f64) -> Vec<Input> {
    snap.route(Input::Key { vk, down, text: None }, l, s, now, Rates::default())
}

fn setup() -> (SnapInput, Layout) {
    let mut l = Layout::default();
    l.configure("main=pointer").unwrap();
    show(&mut l, &scene(None));
    let mut snap = SnapInput::default();
    snap.set_alt_in_menus(true);
    (snap, l)
}

#[test]
fn the_mode_comes_on_with_a_menu_and_goes_with_the_last_but_never_in_the_world() {
    let (mut snap, mut l) = setup();
    run(&mut snap, &mut l, &scene(None), 0.0, 1.0);
    assert!(!snap.active(), "in the world the player presses Alt");
    run(&mut snap, &mut l, &scene(Some("F1")), 1.0, 0.2);
    assert!(snap.active() && snap.snapper.snapping(), "a menu: on by itself");
    assert!(!l.captured());
    // Another menu after it: still on.
    run(&mut snap, &mut l, &scene(Some("City")), 1.2, 0.2);
    assert!(snap.snapper.snapping());
    // All closed: off (the pointer lands), the world's again.
    run(&mut snap, &mut l, &scene(None), 1.4, 2.0);
    assert!(!snap.active() && !l.snapping());
}

#[test]
fn esc_closes_the_menu_and_alt_turns_the_mode_off_until_the_menus_are_gone() {
    let (mut snap, mut l) = setup();
    let menu = scene(Some("F1"));
    run(&mut snap, &mut l, &menu, 0.0, 0.2);
    assert!(snap.snapper.snapping());
    // Esc is the menu's (the game closes it), the mode stays.
    let out = key(&mut snap, &mut l, &menu, ESC, true, 0.3);
    assert!(out.contains(&Input::Key { vk: ESC, down: true, text: None }), "{out:?}");
    let out = key(&mut snap, &mut l, &menu, ESC, false, 0.35);
    assert!(out.contains(&Input::Key { vk: ESC, down: false, text: None }), "{out:?}");
    assert!(snap.snapper.snapping());
    // Alt: off, and it stays off in this menu and the next.
    key(&mut snap, &mut l, &menu, ALT, true, 0.4);
    key(&mut snap, &mut l, &menu, ALT, false, 0.45);
    run(&mut snap, &mut l, &menu, 0.5, 1.0);
    assert!(!snap.active());
    run(&mut snap, &mut l, &scene(Some("F2")), 1.5, 0.5);
    assert!(!snap.active(), "declined until the menus are all closed");
    // Closed, then a menu again: on again.
    run(&mut snap, &mut l, &scene(None), 2.0, 0.5);
    run(&mut snap, &mut l, &scene(Some("F3")), 2.5, 0.2);
    assert!(snap.snapper.snapping());
}

#[test]
fn the_players_own_mode_stays_after_the_menus_and_the_setting_off_or_the_window_behind_does_nothing() {
    // Turned on in the world with Alt: a menu coming and going leaves it.
    let (mut snap, mut l) = setup();
    run(&mut snap, &mut l, &scene(None), 0.0, 0.2);
    key(&mut snap, &mut l, &scene(None), ALT, true, 0.3);
    key(&mut snap, &mut l, &scene(None), ALT, false, 0.35);
    assert!(snap.snapper.snapping());
    run(&mut snap, &mut l, &scene(Some("F1")), 0.4, 0.3);
    run(&mut snap, &mut l, &scene(None), 0.7, 1.0);
    assert!(snap.snapper.snapping(), "the player's, not the menu's");
    // The setting off (or the window not in front): menus as before.
    let (mut snap, mut l) = setup();
    snap.set_alt_in_menus(false);
    run(&mut snap, &mut l, &scene(Some("F1")), 0.0, 1.0);
    assert!(!snap.active());
}

#[test]
fn a_menu_opening_over_a_captured_mouse_snaps_with_the_mouse_let_go_and_captures_after() {
    let (mut snap, mut l) = setup();
    let world = scene(None);
    // Captured in the view (a click into it).
    l.route(Input::MouseMove { x: 400, y: 250 });
    l.route(Input::MouseButton { button: Button::Left, down: true, x: 400, y: 250 });
    l.route(Input::MouseButton { button: Button::Left, down: false, x: 400, y: 250 });
    assert!(l.captured());
    run(&mut snap, &mut l, &scene(Some("Menu")), 0.0, 0.3);
    assert!(!l.captured() && snap.snapper.snapping(), "the menu: let go, snapping");
    // Its attractor drawn, not capturing under the menu (a free pointer).
    assert!(!snap.attractor_points().is_empty());
    run(&mut snap, &mut l, &world, 0.3, 1.5);
    assert!(!snap.active());
    assert!(l.captured(), "the world's capture taken up again");
}
