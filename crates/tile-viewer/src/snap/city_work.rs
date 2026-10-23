use super::*;
use scene::{FreeSprite, Scene, View, ViewState};

#[test]
fn city_worked_tile_alt_target_clicks_the_same_native_tile_across_splits_and_scales() {
    let item = Interactable { region: None, alt_right_click: false, rect: [1056, 460, 64, 32], anchor: Some([1088, 476]),
        kind: InteractableKind::Button, id: 0x740C34_0F000002 };
    let scene = Scene {
        screen_size: (1920, 1080),
        shape: scene::WorldShape { cols: 40, rows: 30, wrap_x: true, wrap_y: false },
        interface: scene::Interface { screen: Some("City".into()), ..Default::default() },
        views: vec![
            ViewState { id: 0, view: View { interface_only: true, camera_x: 9999, camera_y: 9999, ..Default::default() }, overlay: vec![] },
            ViewState { id: 1, view: View { camera_x: 192, camera_y: 182, origin: (448, 306), viewport: (1024, 320), ..Default::default() }, overlay: vec![] },
            ViewState { id: 255, view: View { interface_only: true, menu: true, ..Default::default() },
                overlay: vec![FreeSprite { x: 448, y: 156, screen: true, after_pass: u32::MAX, ..Default::default() }] },
        ],
        interactables: vec![item], ..Default::default()
    };
    for split in [viewer_settings::Split::Standard, viewer_settings::Split::Vertical1To1, viewer_settings::Split::Vertical2To1, viewer_settings::Split::Horizontal1To1] {
        for pixel_scale in [1, 2, 3, 6] {
            for frame_scale in [0.5, 1.0, 2.0] {
                let mut layout = Layout::default();
                layout.apply_settings(&viewer_settings::Layout { split, ..Default::default() });
                let plan = crate::scaling::plan((3840, 2160), scene.screen_size,
                    &viewer_settings::Resolution { pixel_scale, frame_scale, ..Default::default() });
                layout.driver_screen = Some(plan.screen);
                for _ in 0..2 {
                    let mut frame = layout.frame(&scene, plan.present.canvas, (0, 0, 0, 0),
                        vec![[448, 156, 1024, 150], [448, 626, 1024, 298]], None);
                    layout.set_interface_shown(crate::scaling::arrange(&plan, &scene, &mut frame, |_: &FreeSprite| Some([448, 156, 1024, 768])).shown);
                }
                let targets = SnapInput::targets(&scene, &layout);
                let target = targets.iter().find(|t| t.id == item.id).expect("worked tile remains visible");
                let point = target.center();
                let mut snapper = Snapper::default();
                snapper.enter(0.0, point, &targets, Rates::default());
                snapper.tick(1.0, &targets, Rates::default());
                assert_eq!(snapper.target_id(), Some(item.id));
                let actions = snapper.click(Button::Left, true).unwrap();
                let action = actions.into_iter().find(|a| matches!(a, Action::Button { .. })).unwrap();
                let input = to_input(action, &layout).unwrap();
                let Input::MouseButton { button: Button::Left, down: true, x, y } = input else { panic!("worked tile became a world-camera action: {input:?}") };
                assert!((x - 1088).abs() <= 1 && (y - 476).abs() <= 1,
                    "{split:?}, pixels {pixel_scale}, frame {frame_scale}: native city point {:?}", (x, y));
                // Bridge's city camera inverse restores native game camera
                // coordinates: tile(12,10) centre is (832,352) at camera0.
                assert!((x - 256 - 832).abs() <= 1 && (y - 124 - 352).abs() <= 1);
            }
        }
    }
}
