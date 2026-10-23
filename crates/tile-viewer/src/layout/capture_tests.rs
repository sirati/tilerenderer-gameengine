// The mouse capture across menus, nested menus, map movement modes and
// focus changes (`capture`). A child of `layout`'s tests, using their
// scenes and helpers.


fn show(l: &mut Layout, s: &Scene) {
    l.frame(s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
}

/// `seconds` of frames of `s`; every capture change asked for meanwhile.
fn wait(l: &mut Layout, s: &Scene, seconds: f64) -> Vec<bool> {
    let mut grabs = Vec::new();
    for _ in 0..(seconds / 0.1).ceil() as usize {
        l.advance(std::time::Duration::from_millis(100));
        show(l, s);
        grabs.extend(l.take_grab());
    }
    grabs
}

fn popup() -> Scene {
    with_parts(Some(300))
}

fn screen_and_popup(kind: &str) -> Scene {
    let mut s = popup();
    s.interface.screen = Some(kind.into());
    s
}

/// UnitMouse roles, the mouse captured in B; the world cursor.
fn captured() -> (Layout, (i32, i32)) {
    let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
    mv(&mut l, (600, 300));
    frame(&mut l, vec![]);
    click(&mut l, (600, 300));
    assert!(l.captured());
    let _ = l.take_grab();
    let cursor = l.cursor;
    (l, cursor)
}

/// The regression: a popup giving way to a screen in the same frame
/// captured the mouse under the screen (the popup's own "capture again").
#[test]
fn a_popup_giving_way_to_a_screen_never_captures_under_it() {
    let (mut l, cursor) = captured();
    show(&mut l, &popup());
    assert!(!l.captured());
    assert_eq!(l.take_grab(), Some(false));
    // The popup closes as the advisor opens.
    show(&mut l, &with_screen("F1"));
    assert!(!l.captured(), "captured under the screen");
    assert_eq!(wait(&mut l, &with_screen("F1"), 2.0), [], "nothing captures while it is open");
    assert!(!l.captured());
    // All closed: captured again, once, at the world cursor it had.
    show(&mut l, &scene());
    assert!(!l.captured(), "not at once");
    assert_eq!(wait(&mut l, &scene(), 0.3), [true]);
    assert!(l.captured());
    assert_eq!(l.cursor, BOUNDS.unwrap().clamp_point(cursor));
}

/// The other way round: the screen's own "capture again" captured the
/// mouse under a popup still open over where the screen was.
#[test]
fn a_screen_closing_under_its_popup_never_captures_under_the_popup() {
    let (mut l, _) = captured();
    show(&mut l, &with_screen("City"));
    assert!(!l.captured());
    show(&mut l, &screen_and_popup("City"));
    show(&mut l, &popup());
    assert!(!l.captured(), "captured under the popup");
    let _ = l.take_grab();
    assert_eq!(wait(&mut l, &popup(), 1.0), []);
    assert_eq!(wait(&mut l, &scene(), 0.5), [true]);
    assert!(l.captured());
}

/// A menu following another a moment later (the game's turn-start
/// popups, one advisor after another): no capture in between.
#[test]
fn menus_following_each_other_see_no_capture_in_between() {
    let (mut l, cursor) = captured();
    let mut grabs = Vec::new();
    for menu in [popup(), with_screen("F1"), with_screen("F2"), popup(), with_screen("City")] {
        show(&mut l, &menu);
        grabs.extend(l.take_grab());
        grabs.extend(wait(&mut l, &menu, 0.3));
        // A gap shorter than `RESUME_AFTER` between it and the next.
        grabs.extend(wait(&mut l, &scene(), 0.1));
        assert!(!l.captured());
    }
    assert_eq!(grabs, [false], "let go once, never captured again in between");
    assert_eq!(wait(&mut l, &scene(), 0.5), [true]);
    assert_eq!(l.cursor, BOUNDS.unwrap().clamp_point(cursor), "at the world cursor of before the first menu");
}

#[test]
fn nested_menus_capture_again_only_when_the_last_closes() {
    let (mut l, _) = captured();
    for s in [with_screen("Menu"), screen_and_popup("Menu"), with_screen("Menu"), screen_and_popup("Menu"), screen_and_popup("F3"), with_screen("F3")] {
        show(&mut l, &s);
        assert_eq!(wait(&mut l, &s, 0.5).into_iter().filter(|&g| g).count(), 0);
        assert!(!l.captured());
    }
    assert_eq!(wait(&mut l, &scene(), 0.5), [true]);
}

#[test]
fn losing_the_focus_forgets_the_capture_and_coming_back_never_captures() {
    let (mut l, _) = captured();
    show(&mut l, &popup());
    assert_eq!(l.take_grab(), Some(false));
    // Another window takes the focus while the popup is open.
    l.released();
    show(&mut l, &popup());
    assert_eq!(wait(&mut l, &scene(), 1.0), []);
    assert!(!l.captured(), "back from the other window, the player clicks to capture");
    click(&mut l, (600, 300));
    assert!(l.captured());
    // Losing it while captured, no menu: let go, not captured on return.
    l.released();
    assert!(!l.captured());
    assert_eq!(wait(&mut l, &scene(), 1.0), [false]);
}

#[test]
fn drag_map_movement_lets_go_and_never_captures_menus_or_not() {
    let (mut l, _) = captured();
    l.set_map_movement(viewer_settings::MapMovement::Drag);
    show(&mut l, &scene());
    assert!(!l.captured());
    assert_eq!(l.take_grab(), Some(false));
    // Menus come and go: nothing to take up again.
    show(&mut l, &popup());
    assert_eq!(wait(&mut l, &scene(), 1.0), []);
    // A click into B is the world's; no walls, no confinement.
    click(&mut l, (600, 300));
    assert!(!l.captured());
    assert_eq!(l.pointer_walls(), None);
    mv(&mut l, (100, 100));
    mv(&mut l, (700, 100));
    assert_eq!(l.pointer, (700, 100), "through the divider's closed third");
    // Edge scrolling again: still free until clicked into.
    l.set_map_movement(viewer_settings::MapMovement::EdgeScroll);
    assert_eq!(wait(&mut l, &scene(), 1.0), []);
    click(&mut l, (600, 300));
    assert!(l.captured());
}

#[test]
fn clicks_capture_beside_a_screen_in_the_other_pane_but_not_under_floating_menus() {
    // Floating over the window: a click on B's world does not capture.
    let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
    show(&mut l, &with_screen("F1"));
    mv(&mut l, (600, 300));
    click(&mut l, (600, 300));
    assert!(!l.captured());
    assert_ne!(l.take_pointer_icon(), Some(PointerIcon::Capture));
    // The screen in A: playing in B beside it (docs/modal-input.md).
    let mut l = screen_layout(Place::LeftOrBottom, Roles::UnitMouse, false);
    let s = with_screen("F1");
    show(&mut l, &s);
    mv(&mut l, (600, 300));
    click(&mut l, (600, 300));
    assert!(l.captured());
    // A popup over it lets go; it closing, the screen still open: not
    // again by itself (the player clicks into B again).
    show(&mut l, &screen_and_popup("F1"));
    assert!(!l.captured());
    assert_eq!(wait(&mut l, &s, 1.0).into_iter().filter(|&g| g).count(), 0);
    assert!(!l.captured());
    // All closed: taken up again.
    assert_eq!(wait(&mut l, &scene(), 0.5).last(), Some(&true));
}

/// UnitMouse roles, screens and popups placed in A: B, following the world
/// cursor, beside them. A popup open, the mouse free.
fn beside_popup() -> Layout {
    let mut l = screen_layout(Place::LeftOrBottom, Roles::UnitMouse, false);
    show(&mut l, &popup());
    assert!(!l.captured());
    l
}

/// A point on the popup (in A, below the corner's buttons).
fn on_popup(l: &Layout) -> (i32, i32) {
    (0..400).step_by(4).flat_map(|x| (60..600).step_by(4).map(move |y| (x, y))).find(|&p| matches!(l.hit(p), Hit::Interface | Hit::Moved(_))).expect("the popup in A")
}

#[test]
fn beside_a_popup_the_pointer_over_the_world_cursors_view_shows_a_click_captures() {
    let mut l = beside_popup();
    mv(&mut l, (600, 300));
    assert_eq!(l.take_pointer_icon(), Some(PointerIcon::Capture));
    let p = on_popup(&l);
    mv(&mut l, p);
    assert_eq!(l.take_pointer_icon(), Some(PointerIcon::Default), "over the popup at {p:?}");
}

#[test]
fn a_click_beside_a_popup_captures_sends_nothing_and_stays_while_it_is_open() {
    let mut l = beside_popup();
    mv(&mut l, (600, 300));
    assert_eq!(click(&mut l, (600, 300)), [], "no action in the game");
    assert!(l.captured());
    assert_eq!(l.take_grab(), Some(true));
    // `settle_capture` keeps it, the popup staying open.
    assert_eq!(wait(&mut l, &popup(), 1.0), []);
    assert!(l.captured());
    // The popup closing changes nothing.
    assert_eq!(wait(&mut l, &scene(), 1.0), []);
    assert!(l.captured());
}

#[test]
fn a_click_on_the_menu_outside_the_captured_view_lets_go_and_reaches_it() {
    let mut l = beside_popup();
    mv(&mut l, (600, 300));
    click(&mut l, (600, 300));
    let _ = l.take_grab();
    // (The Alt mode's click, `crate::snap`.) In the captured view: kept.
    l.menu_clicked((600, 300));
    assert!(l.captured());
    let p = on_popup(&l);
    l.menu_clicked(p);
    assert!(!l.captured());
    assert_eq!(l.take_grab(), Some(false));
    // The click itself goes to the popup.
    assert!(matches!(click(&mut l, p).first(), Some(Input::MouseButton { button: Button::Left, down: true, .. })));
    assert!(!l.captured());
    // Not captured again by itself while the popup is open.
    assert_eq!(wait(&mut l, &popup(), 1.0), []);
}

#[test]
fn a_menu_opening_beside_the_captured_view_still_lets_go() {
    let mut l = screen_layout(Place::LeftOrBottom, Roles::UnitMouse, false);
    mv(&mut l, (600, 300));
    click(&mut l, (600, 300));
    assert!(l.captured());
    let _ = l.take_grab();
    show(&mut l, &popup());
    assert!(!l.captured());
    assert_eq!(wait(&mut l, &popup(), 1.0), [false], "let go, never captured by itself");
    assert_eq!(wait(&mut l, &scene(), 0.5), [true]);
}

#[test]
fn a_popup_over_the_world_cursors_view_does_not_capture_there() {
    let mut l = screen_layout(Place::RightOrTop, Roles::UnitMouse, false);
    show(&mut l, &popup());
    let p = (600, 560);
    assert_eq!(l.capture_hit(p), Some(Hit::Pane), "B's world, beside the popup's body");
    mv(&mut l, p);
    assert_ne!(l.take_pointer_icon(), Some(PointerIcon::Capture));
    assert!(!click(&mut l, p).is_empty(), "a click on the world there");
    assert!(!l.captured());
}

#[test]
fn drag_map_movement_never_captures_beside_a_popup() {
    let mut l = beside_popup();
    l.set_map_movement(viewer_settings::MapMovement::Drag);
    show(&mut l, &popup());
    mv(&mut l, (600, 300));
    assert_ne!(l.take_pointer_icon(), Some(PointerIcon::Capture));
    assert!(!click(&mut l, (600, 300)).is_empty(), "a click on the world there");
    assert!(!l.captured());
}
