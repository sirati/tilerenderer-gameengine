use super::*;

fn queue() -> ProductionQueue {
    let rows = (0..4).map(|k| QueueRow { slot: k, rect: [100, 200 + 20 * k as i32, 160, 20], grip: [100, 200 + 20 * k as i32, 14, 20], grip_id: 0x10 + u64::from(k) }).collect();
    ProductionQueue { stamp: 77, current: [400, 300, 114, 95], rows }
}

fn scene() -> Scene {
    let mut s = Scene::default();
    s.interface.extensions = vec![queue().extension()];
    s
}

fn button(down: bool, x: i32, y: i32) -> Input {
    Input::MouseButton { button: Button::Left, down, x, y }
}

/// The drag as the viewer runs it: on the scene, through the extension.
fn route(d: &mut QueueDrag, input: &Input, s: &Scene) -> Option<Vec<Input>> {
    InterfaceExtension::route(d, input, s)
}

fn edit(from: u8, to: Target) -> Vec<Input> {
    vec![QueueEdit { stamp: 77, from, to }.input()]
}

const STYLE: QueueStyle = QueueStyle {
    groove: [1, 2, 3, 255],
    edge: [4, 5, 6, 255],
    line: [7, 8, 9, 255],
    line_edge: [10, 11, 12, 255],
    hint_fill: [13, 14, 15, 80],
    hint_frame: [16, 17, 18, 255],
    arrow: [19, 20, 21, 255],
    dim: [22, 23, 24, 170],
    ghost_back: [25, 26, 27, 110],
    ghost_frame: [28, 29, 30, 150],
    ghost_opacity: 130,
};

#[test]
fn the_queue_and_its_edits_round_trip_as_scene_extensions() {
    let q = queue();
    let e = q.extension();
    assert_eq!(e.kind, QUEUE);
    assert_eq!(ProductionQueue::decode(&e.data), Some(q.clone()));
    assert_eq!(ProductionQueue::of(&scene()), Some(q));
    assert_eq!(ProductionQueue::of(&Scene::default()), None);
    assert_eq!(ProductionQueue::decode(&e.data[..e.data.len() - 1]), None, "truncated");
    for to in [Target::Before(0), Target::Before(4), Target::Current] {
        let edit = QueueEdit { stamp: 0x1234_5678, from: 3, to };
        assert_eq!(QueueEdit::of(&edit.input()), Some(edit));
    }
    assert_eq!(QueueEdit::of(&Input::Extension { kind: QUEUE, data: vec![0; 6] }), None, "another kind");
    assert_eq!(QueueEdit::of(&Input::Quit), None);
    // Through the scene protocol as a whole.
    let updates = [scene::Update::Interface(scene().interface)];
    assert_eq!(scene::wire::decode_updates(&scene::wire::encode_updates(&updates)).unwrap(), updates);
}

#[test]
fn drags_drop_between_the_rows_above_or_below_the_middle() {
    let q = queue();
    // Row 0 dragged: into row 2's upper half: before 2; its lower half: before 3.
    assert_eq!(target(&q, 0, (150, 245), false), Some(Target::Before(2)));
    assert_eq!(target(&q, 0, (150, 255), false), Some(Target::Before(3)));
    assert_eq!(target(&q, 0, (150, 278), false), Some(Target::Before(4)), "below the last row's middle: the end");
    assert_eq!(target(&q, 3, (150, 192), false), Some(Target::Before(0)), "half a row above the first");
    // Where it is already: nothing.
    assert_eq!(target(&q, 1, (150, 215), false), None);
    assert_eq!(target(&q, 1, (150, 235), false), None);
    assert_eq!(target(&q, 0, (150, 400), false), None, "outside the rows");
    assert_eq!(target(&q, 0, (99, 245), false), None);
    assert_eq!(target(&q, 2, (450, 340), false), Some(Target::Current));
}

#[test]
fn a_click_drop_takes_the_clicked_rows_place() {
    let q = queue();
    assert_eq!(target(&q, 0, (150, 250), true), Some(Target::Before(3)), "down: after that row");
    assert_eq!(target(&q, 3, (150, 210), true), Some(Target::Before(0)), "up: before it");
    assert_eq!(target(&q, 1, (150, 230), true), None, "its own row");
}

#[test]
fn a_drag_from_a_handle_never_reaches_the_driver_and_drops_on_release() {
    let mut d = QueueDrag::default();
    let s = scene();
    assert_eq!(route(&mut d, &button(true, 105, 210), &s), Some(vec![]), "the press on the handle is the drag's");
    assert!(d.active());
    assert_eq!(route(&mut d, &Input::MouseMove { x: 120, y: 272 }, &s), None, "motion still goes on");
    assert_eq!(route(&mut d, &button(false, 120, 272), &s), Some(edit(0, Target::Before(4))));
    assert!(!d.active());
    // A press on the row's text is the driver's.
    assert_eq!(route(&mut d, &button(true, 200, 210), &s), None);
    assert_eq!(route(&mut d, &button(false, 200, 210), &s), None);
}

#[test]
fn dropping_on_the_production_box_swaps_and_elsewhere_puts_back() {
    let s = scene();
    let mut d = QueueDrag::default();
    route(&mut d, &button(true, 105, 250), &s);
    route(&mut d, &Input::MouseMove { x: 420, y: 320 }, &s);
    assert_eq!(route(&mut d, &button(false, 420, 320), &s), Some(edit(2, Target::Current)));
    route(&mut d, &button(true, 105, 250), &s);
    route(&mut d, &Input::MouseMove { x: 700, y: 20 }, &s);
    assert_eq!(route(&mut d, &button(false, 700, 20), &s), Some(vec![]), "let go elsewhere: nothing");
}

#[test]
fn a_click_picks_up_until_the_next_click_as_the_alt_mode_does_it() {
    let s = scene();
    let mut d = QueueDrag::default();
    assert_eq!(route(&mut d, &button(true, 106, 230), &s), Some(vec![]));
    assert_eq!(route(&mut d, &button(false, 106, 230), &s), Some(vec![]));
    assert!(d.active(), "picked up by the click");
    assert_eq!(route(&mut d, &Input::MouseMove { x: 457, y: 347 }, &s), None);
    assert_eq!(route(&mut d, &button(true, 457, 347), &s), Some(edit(1, Target::Current)));
    assert_eq!(route(&mut d, &button(false, 457, 347), &s), Some(vec![]), "its release too");
    assert!(!d.active());
    // Esc puts it back.
    route(&mut d, &button(true, 106, 230), &s);
    route(&mut d, &button(false, 106, 230), &s);
    assert_eq!(route(&mut d, &Input::Key { vk: VK_ESCAPE, down: true, text: None }, &s), Some(vec![]));
    assert!(!d.active());
}

#[test]
fn a_new_queue_lets_go_of_the_row() {
    let mut d = QueueDrag::default();
    route(&mut d, &button(true, 105, 210), &scene());
    let mut other = Scene::default();
    other.interface.extensions = vec![ProductionQueue { stamp: 78, ..queue() }.extension()];
    route(&mut d, &Input::MouseMove { x: 0, y: 0 }, &other);
    assert!(!d.active());
    assert_eq!(route(&mut d, &button(false, 120, 262), &other), None);
}

fn text(x: i32, y: i32) -> PlacedSprite {
    PlacedSprite { screen: true, view: Some(3), solid: Some(SolidRect { width: 40, height: 10, rgba: [0, 0, 0, 255] }), ..PlacedSprite::new(SpriteSetId::from_raw(9), x, y) }
}

fn rect(s: &PlacedSprite) -> Option<Rect> {
    let r = s.solid?;
    let (mut x0, mut y0, mut x1, mut y1) = (s.x, s.y, s.x + r.width as i32, s.y + r.height as i32);
    if let Some(c) = s.clip { (x0, y0, x1, y1) = (x0.max(c[0]), y0.max(c[1]), x1.min(c[2]), y1.min(c[3])) }
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// What the viewer draws: through the extension, on the scene.
fn drawn(d: &QueueDrag, s: &Scene, free: &[PlacedSprite]) -> Option<Vec<PlacedSprite>> {
    assert_eq!(d.draws(s), s.interface.extension(QUEUE).is_some());
    InterfaceExtension::draw(d, s, free, &rect)
}

#[test]
fn handles_come_before_the_text_which_moves_right() {
    let s = scene();
    let free: Vec<PlacedSprite> = (0..4).map(|k| text(102, 205 + 20 * k)).collect();
    let out = drawn(&QueueDrag::new(STYLE), &s, &free).unwrap();
    assert_eq!(out[0].x, 100 + 14 + 2, "text right of the handle");
    assert_eq!(out[0].view, Some(3));
    let bars: Vec<_> = out[4..].iter().filter(|p| p.solid.unwrap().rgba == STYLE.groove).collect();
    assert_eq!(bars.len(), 3 * 4, "three bars a row");
    assert!(bars.iter().all(|b| b.view == Some(3) && b.x >= 100 && b.x + b.solid.unwrap().width as i32 <= 114));
    assert!(drawn(&QueueDrag::new(STYLE), &Scene::default(), &free).is_none());
}

#[test]
fn a_drag_shows_its_copy_the_line_and_over_the_box_the_hint() {
    let s = scene();
    let free: Vec<PlacedSprite> = (0..4).map(|k| text(102, 205 + 20 * k)).collect();
    let mut d = QueueDrag::new(STYLE);
    route(&mut d, &button(true, 105, 210), &s);
    route(&mut d, &Input::MouseMove { x: 110, y: 272 }, &s);
    let out = drawn(&d, &s, &free).unwrap();
    assert!(out.iter().any(|p| p.solid.is_some_and(|r| r.rgba == STYLE.line && r.height == 2) && p.y == 279), "line below the last row");
    assert!(out.iter().any(|p| p.solid.is_some_and(|r| r.rgba == STYLE.ghost_back) && (p.x, p.y) == (105, 262)), "the copy where the pointer took it");
    route(&mut d, &Input::MouseMove { x: 420, y: 320 }, &s);
    let out = drawn(&d, &s, &free).unwrap();
    assert!(out.iter().any(|p| p.solid.is_some_and(|r| r.rgba == STYLE.hint_frame)));
    assert!(!out.iter().any(|p| p.solid.is_some_and(|r| r.rgba == STYLE.line && r.height == 2)));
}

#[test]
fn the_copy_stays_in_the_queue_panel_and_lets_the_hints_show_through() {
    let s = scene();
    let panel = queue().rows.iter().map(|r| r.rect[0] + r.rect[2]).max().unwrap();
    let free: Vec<PlacedSprite> = (0..4).map(|k| text(102, 205 + 20 * k)).collect();
    let mut d = QueueDrag::new(STYLE);
    route(&mut d, &button(true, 105, 210), &s);
    for (x, y) in [(170, 272), (420, 320)] {
        route(&mut d, &Input::MouseMove { x, y }, &s);
        let out = drawn(&d, &s, &free).unwrap();
        for p in out.iter().filter(|p| p.solid.is_some_and(|r| r.rgba == STYLE.ghost_back || r.rgba == STYLE.ghost_frame)) {
            assert!(p.x + p.solid.unwrap().width as i32 <= panel + 1, "copy past the panel at {x}: {p:?}");
        }
        assert!(out.iter().filter(|p| p.clip.is_some() && p.solid.is_none() && p.y > 230).all(|p| p.clip.unwrap()[2] <= panel));
    }
    let neutral = QueueStyle::default();
    assert!(neutral.ghost_back[3] <= 128 && neutral.ghost_opacity <= 140, "translucent enough for the line and arrows");
}
