use super::*;
use scene::{Scene, View, ViewState};

const ID: u64 = 0xc123_4567_89ab_cdef;

fn setup() -> (SnapInput, Layout, Scene) {
    let scene = Scene {
        seq: 1,
        views: vec![ViewState { id: 0, view: View { interface_only: true, ..Default::default() }, overlay: vec![] }],
        interactables: vec![Interactable { alt_right_click: false, anchor: None, id: ID, rect: [100, 100, 220, 24], kind: InteractableKind::Slider { track: [110, 112, 200, 1], value: 250_000 } }],
        ..Default::default()
    };
    let mut layout = Layout::default();
    layout.frame(&scene, (800, 600), (0, 0, 0, 0), vec![], None);
    let mut snap = SnapInput::default();
    snap.route(key(VK_MENU, true), &mut layout, &scene, 0.0, Rates::default());
    snap.route(key(VK_MENU, false), &mut layout, &scene, 0.01, Rates::default());
    snap.tick(&mut layout, &scene, 1.0, Rates::default());
    assert_eq!(snap.snapper.target_id(), Some(ID));
    (snap, layout, scene)
}

fn key(vk: u32, down: bool) -> Input { Input::Key { vk, down, text: None } }
fn button(button: Button, down: bool) -> Input { Input::MouseButton { button, down, x: 700, y: 500 } }
fn event(action: SliderAction, value: i32) -> Input { Input::Slider { id: ID, action, value } }
fn route(snap: &mut SnapInput, layout: &mut Layout, scene: &Scene, input: Input) -> Vec<Input> {
    snap.route(input, layout, scene, 1.0, Rates::default())
}
fn begin(snap: &mut SnapInput, layout: &mut Layout, scene: &Scene) {
    assert_eq!(route(snap, layout, scene, button(Button::Left, true)), vec![event(SliderAction::Begin, 250_000)]);
    assert!(route(snap, layout, scene, button(Button::Left, false)).is_empty());
}

#[test]
fn selecting_slider_preserves_value_and_pointer_until_motion_then_left_accepts_preview() {
    let (mut snap, mut layout, scene) = setup();
    let pointer = snap.snapper.pointer();
    begin(&mut snap, &mut layout, &scene);
    assert!(snap.tick(&mut layout, &scene, 1.1, Rates::default()).is_empty());
    assert_eq!(snap.motion((20.0, 80.0)), Some(vec![]));
    assert_eq!(snap.tick(&mut layout, &scene, 1.2, Rates::default()), vec![event(SliderAction::Preview, 350_000)]);
    assert_eq!(snap.snapper.pointer(), pointer);
    assert_eq!(snap.snapper.target_id(), Some(ID));
    // Preview any motion since the last tick before accepting; neither
    // selection, confirmation nor their releases press the native mouse.
    snap.motion((10.0, 0.0));
    assert_eq!(route(&mut snap, &mut layout, &scene, button(Button::Left, true)), vec![event(SliderAction::Preview, 400_000), event(SliderAction::Commit, 400_000)]);
    assert!(route(&mut snap, &mut layout, &scene, button(Button::Left, false)).is_empty());
    assert!(snap.slider.is_none());
    assert!(snap.snapper.snapping());
}

#[test]
fn right_cancel_discards_pending_motion_and_does_not_leak_mouse_buttons() {
    let (mut snap, mut layout, scene) = setup();
    begin(&mut snap, &mut layout, &scene);
    snap.motion((150.0, 0.0));
    assert_eq!(snap.tick(&mut layout, &scene, 1.1, Rates::default()), vec![event(SliderAction::Preview, 1_000_000)]);
    snap.motion((-100.0, 0.0));
    assert_eq!(route(&mut snap, &mut layout, &scene, button(Button::Right, true)), vec![event(SliderAction::Cancel, 500_000)]);
    assert!(route(&mut snap, &mut layout, &scene, button(Button::Right, false)).is_empty());
    assert!(snap.slider.is_none());
}

#[test]
fn committing_without_motion_never_previews_or_changes_value() {
    let (mut snap, mut layout, scene) = setup();
    begin(&mut snap, &mut layout, &scene);
    assert_eq!(route(&mut snap, &mut layout, &scene, button(Button::Left, true)), vec![event(SliderAction::Commit, 250_000)]);
}

#[test]
fn scene_updates_keep_preview_and_current_track_scale_while_control_loss_cancels() {
    let (mut snap, mut layout, mut scene) = setup();
    begin(&mut snap, &mut layout, &scene);
    snap.motion((20.0, 0.0));
    snap.tick(&mut layout, &scene, 1.1, Rates::default());
    // A driver echo or asynchronous old value must not reset the edit.
    scene.interactables[0].kind = InteractableKind::Slider { track: [110, 112, 400, 1], value: 250_000 };
    snap.tick(&mut layout, &scene, 1.2, Rates::default());
    snap.motion((40.0, 0.0));
    assert_eq!(snap.tick(&mut layout, &scene, 1.3, Rates::default()), vec![event(SliderAction::Preview, 450_000)]);
    scene.interactables.clear();
    let out = snap.tick(&mut layout, &scene, 1.4, Rates::default());
    assert!(out.contains(&event(SliderAction::Cancel, 450_000)));
    assert!(snap.slider.is_none());
}

#[test]
fn escape_alt_ctrl_focus_and_quit_cancel_without_previewing_pending_motion() {
    for exit in [Some(key(VK_ESCAPE, true)), Some(key(VK_MENU, true)), Some(key(VK_CONTROL, true)), Some(Input::Quit), None] {
        let (mut snap, mut layout, scene) = setup();
        begin(&mut snap, &mut layout, &scene);
        snap.motion((20.0, 0.0));
        let out = match exit {
            Some(input) => route(&mut snap, &mut layout, &scene, input),
            None => snap.focus_lost(&mut layout),
        };
        assert_eq!(out.iter().filter(|i| matches!(i, Input::Slider { .. })).cloned().collect::<Vec<_>>(), vec![event(SliderAction::Cancel, 350_000)]);
        assert!(snap.slider.is_none());
    }
}

#[test]
fn vertical_sliders_use_only_vertical_motion_and_clamp_both_ends() {
    let (mut snap, mut layout, mut scene) = setup();
    scene.interactables[0].kind = InteractableKind::Slider { track: [110, 100, 1, 200], value: 250_000 };
    begin(&mut snap, &mut layout, &scene);
    snap.motion((1000.0, 0.0));
    assert!(snap.tick(&mut layout, &scene, 1.1, Rates::default()).is_empty());
    snap.motion((0.0, -1000.0));
    assert_eq!(snap.tick(&mut layout, &scene, 1.2, Rates::default()), vec![event(SliderAction::Preview, 0)]);
    snap.motion((0.0, 1000.0));
    assert_eq!(snap.tick(&mut layout, &scene, 1.3, Rates::default()), vec![event(SliderAction::Preview, 1_000_000)]);
}

fn plain() -> (SnapInput, Layout, Scene) {
    let (_, layout, mut scene) = setup();
    scene.interactables.push(Interactable { alt_right_click: false, anchor: None, id: 7, rect: [100, 300, 80, 20], kind: InteractableKind::Button });
    (SnapInput::default(), layout, scene)
}
fn step(value: i32) -> Input { Input::Slider { id: ID, action: SliderAction::Step, value } }
fn press_at(snap: &mut SnapInput, layout: &mut Layout, scene: &Scene, (x, y): (i32, i32)) -> Vec<Input> {
    let mut out = route(snap, layout, scene, Input::MouseButton { button: Button::Left, down: true, x, y });
    out.extend(route(snap, layout, scene, Input::MouseButton { button: Button::Left, down: false, x, y }));
    out
}
fn sliders(out: &[Input]) -> Vec<Input> { out.iter().filter(|i| matches!(i, Input::Slider { .. })).cloned().collect() }
fn keys(out: &[Input], vk: u32) -> bool { out.iter().any(|i| matches!(i, Input::Key { vk: k, .. } | Input::WorldKey { vk: k, .. } if *k == vk)) }

#[test]
fn arrows_without_a_selected_slider_keep_their_meaning() {
    let (mut snap, mut layout, scene) = plain();
    for vk in [VK_LEFT, VK_UP, 0x27, VK_DOWN] {
        let out = route(&mut snap, &mut layout, &scene, key(vk, true));
        assert!(keys(&out, vk) && sliders(&out).is_empty(), "{out:?}");
        assert!(keys(&route(&mut snap, &mut layout, &scene, key(vk, false)), vk));
    }
    assert_eq!(snap.selected_slider(), None);
}

#[test]
fn a_clicked_slider_takes_arrow_steps_until_a_press_elsewhere() {
    let (mut snap, mut layout, scene) = plain();
    let out = press_at(&mut snap, &mut layout, &scene, (150, 110));
    assert!(out.iter().any(|i| matches!(i, Input::MouseButton { down: true, .. })), "the driver gets the press: {out:?}");
    assert_eq!(snap.selected_slider(), Some(ID));
    for (vk, value) in [(VK_LEFT, -1), (VK_UP, -1), (0x27, 1), (VK_DOWN, 1)] {
        let out = route(&mut snap, &mut layout, &scene, key(vk, true));
        assert_eq!(out, vec![step(value)], "never the driver's key or the world's");
        assert!(route(&mut snap, &mut layout, &scene, key(vk, false)).is_empty());
    }
    // Shift: ten steps; a held key repeats.
    route(&mut snap, &mut layout, &scene, key(VK_SHIFT, true));
    assert_eq!(route(&mut snap, &mut layout, &scene, key(0x27, true)), vec![step(10)]);
    assert_eq!(route(&mut snap, &mut layout, &scene, key(0x27, true)), vec![step(10)]);
    route(&mut snap, &mut layout, &scene, key(0x27, false));
    route(&mut snap, &mut layout, &scene, key(VK_SHIFT, false));
    // A press elsewhere deselects: arrows are keys again.
    press_at(&mut snap, &mut layout, &scene, (120, 310));
    assert_eq!(snap.selected_slider(), None);
    assert!(keys(&route(&mut snap, &mut layout, &scene, key(VK_LEFT, true)), VK_LEFT));
}

#[test]
fn esc_and_the_slider_leaving_the_screen_deselect() {
    let (mut snap, mut layout, mut scene) = plain();
    press_at(&mut snap, &mut layout, &scene, (150, 110));
    assert!(route(&mut snap, &mut layout, &scene, key(VK_ESCAPE, true)).is_empty(), "Esc only deselects");
    assert!(route(&mut snap, &mut layout, &scene, key(VK_ESCAPE, false)).is_empty());
    assert_eq!(snap.selected_slider(), None);
    assert!(keys(&route(&mut snap, &mut layout, &scene, key(VK_ESCAPE, true)), VK_ESCAPE), "the next Esc is the driver's");
    route(&mut snap, &mut layout, &scene, key(VK_ESCAPE, false));
    press_at(&mut snap, &mut layout, &scene, (150, 110));
    assert_eq!(snap.selected_slider(), Some(ID));
    // Another tab: the slider is gone.
    scene.interactables.retain(|i| i.id != ID);
    snap.tick(&mut layout, &scene, 2.0, Rates::default());
    assert_eq!(snap.selected_slider(), None);
    assert!(keys(&route(&mut snap, &mut layout, &scene, key(VK_LEFT, true)), VK_LEFT));
}

#[test]
fn the_alt_mode_slider_takes_arrows_snapped_to_and_inside_its_edit() {
    let (mut snap, mut layout, mut scene) = setup();
    snap.tick(&mut layout, &scene, 1.05, Rates::default());
    assert_eq!(snap.selected_slider(), Some(ID));
    assert_eq!(route(&mut snap, &mut layout, &scene, key(0x27, true)), vec![step(1)]);
    route(&mut snap, &mut layout, &scene, key(0x27, false));
    // In its edit: pending motion first, then the step; Cancel undoes both.
    begin(&mut snap, &mut layout, &scene);
    snap.motion((20.0, 0.0));
    assert_eq!(route(&mut snap, &mut layout, &scene, key(VK_LEFT, true)), vec![event(SliderAction::Preview, 350_000), step(-1)]);
    // The driver's stepped value becomes the edit's; motion goes on from it.
    scene.interactables[0].kind = InteractableKind::Slider { track: [110, 112, 200, 1], value: 340_000 };
    snap.tick(&mut layout, &scene, 1.2, Rates::default());
    snap.motion((20.0, 0.0));
    assert_eq!(snap.tick(&mut layout, &scene, 1.3, Rates::default()), vec![event(SliderAction::Preview, 440_000)]);
    assert_eq!(route(&mut snap, &mut layout, &scene, button(Button::Right, true)), vec![event(SliderAction::Cancel, 440_000)]);
    // Leaving the mode keeps it selected as the slider last edited.
    route(&mut snap, &mut layout, &scene, button(Button::Right, false));
    route(&mut snap, &mut layout, &scene, key(VK_MENU, true));
    route(&mut snap, &mut layout, &scene, key(VK_MENU, false));
    snap.tick(&mut layout, &scene, 3.0, Rates::default());
    assert!(!snap.snapper.snapping());
    assert_eq!(route(&mut snap, &mut layout, &scene, key(VK_UP, true)), vec![step(-1)]);
}

#[test]
fn native_keyboard_focus_border_tracks_selection_and_defers_to_hover_art() {
    let (mut snap, mut layout, mut scene) = plain();
    assert_eq!(snap.keyboard_focus_rect(&scene, &layout), None);
    press_at(&mut snap, &mut layout, &scene, (150, 110));
    let [x, y, w, h] = layout.driver_rect_to_window(scene.interactables[0].rect);
    assert_eq!(snap.keyboard_focus_rect(&scene, &layout), Some([
        x.floor() as i32, y.floor() as i32,
        (x + w).ceil() as i32 - x.floor() as i32,
        (y + h).ceil() as i32 - y.floor() as i32,
    ]));
    scene.looks.push(scene::Looks {
        id: ID,
        select: scene::Select::None,
        shown: scene::LookState::default(),
        views: vec![scene::ViewLooks {
            view: 0, sprites: (0, 0),
            states: vec![(scene::LookState { pointer: scene::PointerState::Hover, selected: false }, vec![scene::FreeSprite::default()])],
        }],
    });
    assert_eq!(snap.keyboard_focus_rect(&scene, &layout), None, "hover art already identifies keyboard focus");
    scene.looks.clear();
    scene.interactables.clear();
    assert_eq!(snap.keyboard_focus_rect(&scene, &layout), None, "a removed control never keeps a border, even before tick");
}

#[test]
fn keyboard_focus_changes_on_the_same_tick_as_the_snapped_target() {
    let (mut snap, mut layout, mut scene) = setup();
    scene.interactables.push(Interactable {
        alt_right_click: false, anchor: None, id: 7,
        rect: [500, 100, 80, 24], kind: InteractableKind::Button,
    });
    assert_eq!(snap.selected_slider(), Some(ID));
    snap.motion((330.0, 0.0));
    snap.tick(&mut layout, &scene, 2.0, Rates::default());
    assert_eq!(snap.snapper.target_id(), Some(7));
    assert_eq!(snap.selected_slider(), None);
    snap.motion((-330.0, 0.0));
    snap.tick(&mut layout, &scene, 3.0, Rates::default());
    assert_eq!(snap.snapper.target_id(), Some(ID));
    assert_eq!(snap.selected_slider(), Some(ID));
}

#[test]
fn an_arrow_after_a_tab_or_modal_replaces_the_edited_slider_never_steps_the_retired_control() {
    for same_id_button in [false, true] {
        let (mut snap, mut layout, mut scene) = setup();
        begin(&mut snap, &mut layout, &scene);
        snap.motion((20.0, 0.0));
        if same_id_button {
            scene.interactables[0].kind = InteractableKind::Button;
        } else {
            scene.interactables.clear();
        }
        // Input can arrive after publishing another tab or modal and before
        // the next animation tick. The old slider's ID must not receive Step.
        let out = route(&mut snap, &mut layout, &scene, key(VK_LEFT, true));
        assert_eq!(sliders(&out), vec![event(SliderAction::Cancel, 350_000)], "{same_id_button}: {out:?}");
        assert!(keys(&out, VK_LEFT), "the new screen keeps its arrow key: {out:?}");
        assert!(snap.slider.is_none());
    }
}

#[test]
fn changing_tabs_while_a_slider_arrow_is_held_consumes_its_release_but_not_the_next_press() {
    let (mut snap, mut layout, mut scene) = plain();
    press_at(&mut snap, &mut layout, &scene, (150, 110));
    assert_eq!(route(&mut snap, &mut layout, &scene, key(VK_LEFT, true)), vec![step(-1)]);
    scene.interactables.clear();
    assert!(route(&mut snap, &mut layout, &scene, key(VK_LEFT, false)).is_empty());
    let out = route(&mut snap, &mut layout, &scene, key(VK_LEFT, true));
    assert!(sliders(&out).is_empty() && keys(&out, VK_LEFT), "{out:?}");
}
