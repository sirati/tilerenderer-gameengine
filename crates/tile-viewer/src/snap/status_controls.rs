use super::*;
use scene::{FreeSprite, Scene, View, ViewState};

#[test]
fn status_orb_and_select_unit_targets_follow_the_drawn_panel_across_splits_and_scales() {
    let gui = 0x9F_DB10u64;
    let orb_id = gui << 32 | 0x3000_0000;
    let select_id = (gui + 0x274) << 32 | 0x0E00_0000;
    let items = vec![
        Interactable { region: None, alt_right_click: false, anchor: None, rect: [1621, 1011, 48, 28], kind: InteractableKind::Button, id: orb_id },
        Interactable { region: None, alt_right_click: false, anchor: None, rect: [1740, 940, 32, 22], kind: InteractableKind::Button, id: select_id },
    ];
    let sprite = |x, y| FreeSprite { x, y, screen: true, after_pass: u32::MAX, ..Default::default() };
    let scene = Scene {
        screen_size: (1920, 1080),
        shape: scene::WorldShape { cols: 62, rows: 125, wrap_x: true, wrap_y: false },
        views: vec![
            ViewState { id: 0, view: View { camera_x: 1000, camera_y: 500, zoom: (1, 1), ..Default::default() }, overlay: vec![] },
            ViewState { id: 254, view: View { interface_only: true, ..Default::default() }, overlay: vec![sprite(1621, 940)] },
        ],
        interactables: items.clone(),
        ..Default::default()
    };
    let rect_of = |_: &FreeSprite| Some([1621, 940, 295, 140]);
    for split in [viewer_settings::Split::Standard, viewer_settings::Split::Vertical1To1, viewer_settings::Split::Vertical2To1, viewer_settings::Split::Horizontal1To1] {
        for pixel_scale in 1..=6 {
            for frame_scale in [0.5, 1.0, 2.0] {
                let mut layout = Layout::default();
                layout.apply_settings(&viewer_settings::Layout { split, ..Default::default() });
                let plan = crate::scaling::plan((3840, 2160), scene.screen_size,
                    &viewer_settings::Resolution { pixel_scale, base_ui_scale: pixel_scale as f32, frame_scale, ..Default::default() });
                layout.driver_screen = Some(plan.screen);
                for _ in 0..2 {
                    let mut frame = layout.frame(&scene, plan.present.canvas, (0, 0, 0, 0), vec![[1621, 940, 295, 140]], None);
                    layout.set_interface_shown(crate::scaling::arrange(&plan, &scene, &mut frame, rect_of).shown);
                }
                let targets = SnapInput::targets(&scene, &layout);
                for item in &items {
                    let target = targets.iter().find(|t| t.id == item.id).unwrap();
                    let shown = layout.driver_rect_to_window(item.rect);
                    assert_eq!(target.rect, shown);
                    let point = target.center();
                    let mut snapper = Snapper::default();
                    snapper.enter(0.0, point, &targets, Rates::default());
                    assert_eq!(snapper.target_id(), Some(item.id));
                    let input = to_input(Action::Button { button: Button::Left, down: true, at: point }, &layout).unwrap();
                    let Input::MouseButton { x, y, .. } = input else { panic!("status target reached the world: {input:?}") };
                    let [ix, iy, iw, ih] = item.rect;
                    assert!(x >= ix && x < ix + iw && y >= iy && y < iy + ih,
                        "{split:?}, pixel x{pixel_scale}, frame {frame_scale}: {input:?} vs {item:?}");
                }
                // Steering from the orb toward the unit's actual icon can
                // reach it, even when that HUD is across the split.
                let orb = targets.iter().find(|t| t.id == orb_id).unwrap().center();
                let unit = targets.iter().find(|t| t.id == select_id).unwrap().center();
                assert_eq!(targets[pick(&targets, orb, Some(orb_id), (unit.0 - orb.0, unit.1 - orb.1)).unwrap()].id, select_id);
            }
        }
    }
}
