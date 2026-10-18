use super::*;

#[test]
fn semantic_pointer_anchor_keeps_the_real_hit_rectangle_at_every_ui_scale() {
    let item = Interactable {
        rect: [20, 30, 400, 70],
        anchor: Some([85, 85]),
        id: 0x10_0000_0000_0001,
        kind: InteractableKind::ListItem,
    };
    for (n, d) in [(1, 4), (1, 2), (1, 1), (2, 1), (3, 1), (6, 1)] {
        let items = targets(&[item], ((100, 50), (n, d)), (3840, 2160));
        let scale = n as f64 / d as f64;
        let target = &items[0];
        assert_eq!(target.rect, [100.0 + 20.0 * scale, 50.0 + 30.0 * scale, 400.0 * scale, 70.0 * scale]);
        assert_eq!(target.center(), ((100.0 + 85.0 * scale).floor(), (50.0 + 85.0 * scale).floor()));
        assert!(inside(target.rect, target.center()));
    }
}

#[test]
fn clipped_anchor_stays_in_the_visible_fragment() {
    let item = Interactable {
        rect: [20, 30, 400, 70],
        anchor: Some([419, 65]),
        id: 7,
        kind: InteractableKind::Button,
    };
    let mut target = targets(&[item], ((0, 0), (1, 1)), (800, 600))[0];
    target.rect = [20.0, 30.0, 80.0, 70.0];
    assert_eq!(target.center(), (99.0, 65.0));
    assert!(inside(target.rect, target.center()));
}

#[test]
fn text_links_are_directional_targets_and_click_at_the_semantic_tip() {
    let items = [
        Interactable {
            rect: [20, 20, 400, 24],
            anchor: Some([100, 31]),
            id: 7,
            kind: InteractableKind::Button,
        },
        Interactable {
            rect: [20, 60, 400, 24],
            anchor: Some([160, 71]),
            id: 8,
            kind: InteractableKind::Button,
        },
    ];
    let targets = targets(&items, ((0, 0), (1, 1)), (800, 600));
    assert_eq!(pick(&targets, targets[0].center(), Some(7), (20.0, 20.0)), Some(1));
    let mut snap = Snapper::default();
    snap.enter(0.0, (90.0, 31.0), &targets, Rates::default());
    snap.tick(1.0, &targets, Rates::default());
    let actions = snap.click(Button::Left, true).unwrap();
    assert!(
        actions.iter().any(|a| matches!(
            a,
            Action::Button {
                button: Button::Left,
                down: true,
                at: (100.0, 31.0)
            }
        )),
        "{actions:?}"
    );
}

#[test]
fn clipping_rejects_hidden_controls_and_keeps_full_anchor_and_slider_affines() {
    for scale in [0.5, 1.0, 3.0, 6.0] {
        let controls = [
            Interactable {
                rect: [0, 0, 200, 100],
                anchor: Some([190, 50]),
                id: 1,
                kind: InteractableKind::Button,
            },
            Interactable {
                rect: [300, 0, 200, 100],
                anchor: Some([490, 50]),
                id: 2,
                kind: InteractableKind::Button,
            },
        ];
        let mut placed = targets_mapped(
            &controls,
            |[x, y, w, h]| [x as f64 * scale, y as f64 * scale, w as f64 * scale, h as f64 * scale],
            (3840, 2160),
        );
        clip_targets(&mut placed, |_| Some([0.0, 0.0, 100.0 * scale, 100.0 * scale]));
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].id, 1);
        assert_eq!(placed[0].anchor, Some((190.0 * scale, 50.0 * scale)), "clipping preserves the original affine");
        assert_eq!(placed[0].center(), ((100.0 * scale).ceil() - 1.0, (50.0 * scale).floor()));
        assert!(inside(placed[0].rect, placed[0].center()));
    }
}

#[test]
fn clipping_a_prior_target_does_not_reassign_a_list_to_an_unrelated_control() {
    let controls = [
        Interactable {
            rect: [600, 10, 100, 20],
            anchor: None,
            id: 1,
            kind: InteractableKind::Button,
        },
        Interactable {
            rect: [10, 10, 100, 100],
            anchor: None,
            id: 2,
            kind: InteractableKind::List {
                scroll: [10, 10, 100, 100],
                step: 20,
            },
        },
        Interactable {
            rect: [10, 10, 100, 20],
            anchor: Some([40, 20]),
            id: 3,
            kind: InteractableKind::ListItem,
        },
    ];
    let mut placed = targets(&controls, ((0, 0), (1, 1)), (800, 600));
    clip_targets(&mut placed, |_| Some([0.0, 0.0, 400.0, 600.0]));
    assert_eq!(placed.len(), 2);
    assert_eq!(placed[1].kind, TargetKind::ListItem { list: Some(0) });
    assert!(matches!(placed[0].kind, TargetKind::List { has_items: true, .. }));
}

#[test]
fn clipping_a_slider_changes_its_hit_area_without_shrinking_the_value_track() {
    let control = Interactable {
        rect: [20, 30, 400, 24],
        anchor: None,
        id: 1,
        kind: InteractableKind::Slider {
            track: [30, 40, 380, 0],
            value: 750_000,
        },
    };
    let mut placed = targets(&[control], ((100, 50), (3, 1)), (3840, 2160));
    let before = placed[0].kind;
    clip_targets(&mut placed, |_| Some([0.0, 0.0, 600.0, 600.0]));
    assert_eq!(placed[0].kind, before);
    assert_eq!(placed[0].rect, [160.0, 140.0, 440.0, 72.0]);
    assert!(inside(placed[0].rect, placed[0].center()));
}

#[test]
fn oversized_menu_alt_targets_stay_in_the_opening_pane_through_all_panned_corners() {
    use scene::{Scene, View, ViewState, WorldShape};
    use viewer_settings::{Resolution, Split};
    let base = || {
        let mut scene = Scene::empty(
            WorldShape {
                cols: 10,
                rows: 10,
                wrap_x: false,
                wrap_y: false,
            },
            0,
        );
        scene.views = vec![
            ViewState {
                id: 0,
                view: View {
                    camera_x: 1000,
                    camera_y: 500,
                    ..Default::default()
                },
                overlay: vec![],
            },
            ViewState {
                id: 255,
                view: View {
                    interface_only: true,
                    ..Default::default()
                },
                overlay: vec![],
            },
        ];
        scene
    };
    for pixel_scale in [3, 6] {
        let plan = crate::scaling::plan(
            (3840, 2160),
            2.0,
            (1920, 1080),
            &Resolution {
                pixel_scale,
                menu_scale: 1.5,
                ..Default::default()
            },
        );
        let mut layout = Layout::default();
        layout.apply_settings(&viewer_settings::Layout {
            split: Split::Vertical1To1,
            menu_follows_pointer: true,
            ..Default::default()
        });
        layout.frame(&base(), plan.canvas(), (0, 0, 0, 0), vec![], None);
        let middle = (plan.canvas().0 as i32 / 4, plan.canvas().1 as i32 / 2);
        layout.route(Input::MouseMove { x: middle.0, y: middle.1 });
        let mut scene = base();
        scene.interface.screen = Some("Menu".into());
        scene.interface.screen_view = Some(258);
        scene.views.push(ViewState {
            id: 258,
            view: View {
                interface_only: true,
                origin: (448, 156),
                viewport: (1024, 768),
                ..Default::default()
            },
            overlay: vec![scene::FreeSprite::default()],
        });
        for (id, (x, y)) in [(1, (448, 156)), (2, (1471, 156)), (3, (448, 923)), (4, (1471, 923))] {
            scene.interactables.push(Interactable {
                rect: [x, y, 1, 1],
                anchor: Some([x, y]),
                id,
                kind: InteractableKind::Button,
            });
        }
        for delta in [(0, 0), (10000, 10000), (-10000, 10000), (10000, -10000), (-10000, -10000)] {
            if delta != (0, 0) {
                let down = Input::MouseButton {
                    button: Button::Middle,
                    down: true,
                    x: middle.0,
                    y: middle.1,
                };
                assert!(
                    layout.menu_pan.route(&down),
                    "x{pixel_scale} delta {delta:?} middle {middle:?}, pan {:?}",
                    layout.menu_pan
                );
                assert!(layout.menu_pan.route(&Input::MouseMove {
                    x: middle.0 + delta.0,
                    y: middle.1 + delta.1
                }));
                assert!(layout.menu_pan.route(&Input::MouseButton {
                    button: Button::Middle,
                    down: false,
                    x: middle.0,
                    y: middle.1
                }));
            }
            let mut frame = layout.frame(&scene, plan.canvas(), (0, 0, 0, 0), vec![], None);
            let pane = frame.menu_region.unwrap();
            let arranged = crate::scaling::arrange_panned(&plan, &scene, &mut frame, &mut layout.menu_pan, |_| Some([448, 156, 1024, 768]));
            layout.set_interface_shown(arranged.shown);
            let targets = SnapInput::targets(&scene, &layout);
            for target in targets.iter().filter(|t| !t.is_capture()) {
                let at = target.center();
                assert!(
                    inside([pane[0] as f64, pane[1] as f64, pane[2] as f64, pane[3] as f64], at),
                    "x{pixel_scale} {delta:?}: {target:?}"
                );
                let point = (at.0 as i32, at.1 as i32);
                let driver = layout.route_window(point).unwrap();
                let original = scene.interactables.iter().find(|i| i.id == target.id).unwrap();
                assert!((driver.0 - original.rect[0]).abs() <= 1 && (driver.1 - original.rect[1]).abs() <= 1);
            }
            if delta != (0, 0) {
                let expected = match (delta.0 > 0, delta.1 > 0) {
                    (true, true) => 1,
                    (false, true) => 2,
                    (true, false) => 3,
                    (false, false) => 4,
                };
                assert!(
                    targets.iter().any(|t| t.id == expected),
                    "missing corner {expected} at x{pixel_scale} {delta:?}: {targets:?}"
                );
            }
        }
    }
}
