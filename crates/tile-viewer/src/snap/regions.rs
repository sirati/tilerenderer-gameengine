//! Controls larger than where they take the pointer: a check box with its
//! label, a scroll bar's whole length, draw button snapping from all of it.
use super::*;

const IDENTITY: ((i32, i32), (u32, u32)) = ((0, 0), (1, 1));
const WINDOW: (u32, u32) = (1000, 800);

fn button(id: u64, x: i32, y: i32) -> Interactable {
    Interactable { region: None, alt_right_click: false, anchor: None, rect: [x - 10, y - 10, 20, 20], kind: InteractableKind::Button, id }
}

/// A check box at (300, 100), 16 square, its label to x 500.
fn check(label: bool) -> Interactable {
    Interactable { region: label.then_some([300, 100, 200, 18]), alt_right_click: false, anchor: None, rect: [300, 100, 16, 16], kind: InteractableKind::Button, id: 7 }
}

#[test]
fn a_check_boxs_label_draws_the_pointer_to_its_box() {
    // From A at (480, 300) straight up onto the label's right end: the box
    // is 42° off that way, C at (560, 110) 23°.
    let from = (480.0, 300.0);
    let with = targets(&[button(1, 480, 300), check(true), button(3, 560, 110)], IDENTITY, WINDOW);
    let k = pick(&with, from, Some(1), (0.0, -190.0)).unwrap();
    assert_eq!(with[k].id, 7);
    // It points at the box, not the label.
    assert_eq!(with[k].center(), (308.0, 108.0));
    // The box alone loses to C.
    let without = targets(&[button(1, 480, 300), check(false), button(3, 560, 110)], IDENTITY, WINDOW);
    assert_eq!(without[pick(&without, from, Some(1), (0.0, -190.0)).unwrap()].id, 3);
}

#[test]
fn snapping_on_over_the_label_takes_the_check_box_and_clicks_its_box() {
    // Beside the label, C nearer; over it, the check box though its box is far.
    let t = targets(&[check(true), button(3, 560, 110)], IDENTITY, WINDOW);
    assert_eq!(t[nearest(&t, (540.0, 105.0)).unwrap()].id, 3);
    assert_eq!(t[nearest(&t, (480.0, 105.0)).unwrap()].id, 7);
    let mut s = Snapper::default();
    s.enter(0.0, (480.0, 105.0), &t, Rates::default());
    assert_eq!(s.target_id(), Some(7));
    // A click there is the box's: it toggles as clicking the box does.
    let actions = s.click(Button::Left, true).unwrap();
    assert!(actions.contains(&Action::Button { button: Button::Left, down: true, at: (308.0, 108.0) }), "{actions:?}");
}

#[test]
fn the_region_is_placed_with_its_control() {
    // Twice the size, from (10, 20).
    let t = targets(&[check(true)], ((10, 20), (2, 1)), WINDOW);
    assert_eq!(t[0].rect, [610.0, 220.0, 32.0, 32.0]);
    assert_eq!(t[0].area, [610.0, 220.0, 400.0, 36.0]);
    assert_eq!(targets(&[check(false)], IDENTITY, WINDOW)[0].area, [300.0, 100.0, 16.0, 16.0]);
}

/// A vertical scroll bar at (700, 100), 16×400, arrows included; its knob
/// travels from y 116 to 484, now at the top.
fn bar(value: i32) -> Interactable {
    Interactable { region: None, alt_right_click: false, anchor: None, rect: [700, 100, 16, 400], kind: InteractableKind::Slider { track: [708, 116, 1, 368], value }, id: 9 }
}

#[test]
fn a_scroll_bars_whole_length_draws_the_pointer_onto_its_knob() {
    // From A at (600, 470) right, onto the bar's bottom: its knob (and its
    // middle) far up, more than 45° off.
    let t = targets(&[button(1, 600, 470), bar(0)], IDENTITY, WINDOW);
    let k = pick(&t, (600.0, 470.0), Some(1), (100.0, 0.0)).unwrap();
    assert_eq!(t[k].id, 9);
    assert_eq!(t[k].center(), (708.0, 116.0), "on the knob");
    // The knob where the value puts it; a horizontal slider's too.
    assert_eq!(targets(&[bar(500_000)], IDENTITY, WINDOW)[0].center(), (708.0, 300.0));
    let slider = Interactable { region: None, alt_right_click: false, anchor: None, rect: [100, 100, 220, 24], kind: InteractableKind::Slider { track: [110, 112, 200, 1], value: 250_000 }, id: 5 };
    assert_eq!(targets(&[slider], IDENTITY, WINDOW)[0].center(), (160.0, 112.0));
}

#[test]
fn the_pointer_follows_the_knob_as_the_bar_scrolls() {
    let mut s = Snapper::default();
    let top = targets(&[bar(0)], IDENTITY, WINDOW);
    s.enter(0.0, (708.0, 300.0), &top, Rates::default());
    assert_eq!(s.target(), Some((708.0, 116.0)));
    let middle = targets(&[bar(500_000)], IDENTITY, WINDOW);
    s.tick(0.1, &middle, Rates::default());
    assert_eq!(s.target(), Some((708.0, 300.0)));
}
