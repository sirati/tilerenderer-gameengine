use super::*;
use scene::{Scene, View, ViewState};

const ID: u64 = 0xc123_4567_89ab_cdef;

fn setup() -> (SnapInput, Layout, Scene) {
    let scene = Scene {
        seq: 1,
        views: vec![ViewState { id: 0, view: View { interface_only: true, ..Default::default() }, overlay: vec![] }],
        interactables: vec![Interactable { anchor: None, id: ID, rect: [100, 100, 220, 24], kind: InteractableKind::Slider { track: [110, 112, 200, 1], value: 250_000 } }],
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
