//! Keep native hover/targeting at the point the renderer actually shows
//! beneath a stationary pointer while its independent cameras move.

use super::Layout;
use scene::{Input, WorldOrderAction};

impl Layout {
    /// Remember input already emitted by a real pointer event, including a
    /// wheel's predicted post-pan point. The draw path must not send it twice.
    pub(super) fn remember_world_inputs(&mut self, inputs: &[Input]) {
        for input in inputs {
            match *input {
                Input::WorldMove { x, y }
                | Input::WorldButton { x, y, .. }
                | Input::WorldKey { x, y, .. }
                | Input::WorldOrder {
                    action: WorldOrderAction::Preview,
                    x,
                    y,
                } => self.last_world_pointer = Some((x, y)),
                Input::MouseMove { .. }
                | Input::MouseButton { .. }
                | Input::Wheel { .. }
                | Input::WorldOrder {
                    action: WorldOrderAction::Cancel,
                    ..
                } => self.last_world_pointer = None,
                _ => {}
            }
        }
    }

    /// Called after layout and scaled UI hit maps are final. A free
    /// pointer's world point can change through a pan, fling or focus
    /// request even when the window reports no mouse movement. Captured
    /// input instead follows the logical cursor, independent of its camera.
    pub fn refresh_world_pointer(&mut self, active: bool) -> Vec<Input> {
        let mut out = self.take_world_order_events();
        if !active
            || self.snapping
            || self.explore.is_some()
            || self.pan_has_gesture()
            || self.drag.is_some()
            || self.menu_pan.is_dragging()
        {
            self.last_world_pointer = None;
            return out;
        }
        let point = self.route_point(self.pointer).err().flatten();
        if point.is_some() && point == self.last_world_pointer {
            return out;
        }
        if let Some(mut orders) = self.world_order_motion(self.pointer) {
            // Held right drags update their preview. After release the
            // same helper suppresses hover, keeping the armed destination.
            out.extend(self.take_world_order_events());
            out.append(&mut orders);
        } else if let Some((x, y)) = point {
            out.push(Input::WorldMove { x, y });
        }
        self.remember_world_inputs(&out);
        // Suppressed pending previews still remember the observed pointer,
        // so a static frame does not revisit the same suppressed movement.
        self.last_world_pointer = point;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Follow, Hit, WorldBounds};
    use scene::{Button, CameraFocus, Scene, View, ViewState};
    use std::time::Duration;
    use viewer_settings::{MapMovement, Roles, Split};

    fn scene() -> Scene {
        Scene {
            views: vec![ViewState {
                id: 0,
                view: View::default(),
                overlay: vec![],
            }],
            camera_focus: Some(CameraFocus {
                serial: 1,
                center: (3000, 2000),
            }),
            focus: Some((3000, 2000)),
            shape: scene::WorldShape {
                cols: 125,
                rows: 250,
                wrap_x: false,
                wrap_y: false,
            },
            ..Default::default()
        }
    }
    fn frame(l: &mut Layout, scene: &Scene, ui: Vec<[i32; 4]>) {
        l.frame(
            scene,
            (800, 600),
            (0, 0, 0, 0),
            ui,
            Some(WorldBounds {
                size: (16_000, 8000),
                wrap: (false, false),
            }),
        );
    }
    fn layout(mode: MapMovement) -> Layout {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout {
            split: Split::Vertical1To1,
            roles: Roles::Free,
            ..Default::default()
        });
        l.set_map_movement(mode);
        frame(&mut l, &scene(), vec![]);
        frame(&mut l, &scene(), vec![]);
        l.split.as_mut().unwrap().view.center = (7000, 3000);
        l.split.as_mut().unwrap().view.settling = false;
        frame(&mut l, &scene(), vec![]);
        l
    }
    fn moved(l: &mut Layout, p: (i32, i32)) -> Vec<Input> {
        l.route(Input::MouseMove { x: p.0, y: p.1 })
    }
    fn button(l: &mut Layout, button: Button, down: bool, p: (i32, i32)) -> Vec<Input> {
        l.route(Input::MouseButton {
            button,
            down,
            x: p.0,
            y: p.1,
        })
    }
    fn tick(l: &mut Layout) {
        l.advance(Duration::from_millis(100));
        frame(l, &scene(), vec![]);
    }

    #[test]
    fn a_stationary_g_pointer_tracks_a_and_b_edge_scroll_without_static_frame_input() {
        for p in [(0, 300), (799, 300)] {
            let mut l = layout(MapMovement::EdgeScroll);
            moved(&mut l, p);
            l.route(Input::Key {
                vk: 0x47,
                down: true,
                text: None,
            });
            assert_eq!(l.refresh_world_pointer(true), []);
            let before = l.world_under(p).unwrap();
            tick(&mut l);
            let after = l.world_under(p).unwrap();
            assert_ne!(after, before);
            assert_eq!(l.pointer, p);
            assert_eq!(
                l.refresh_world_pointer(true),
                [Input::WorldMove {
                    x: after.0,
                    y: after.1
                }]
            );
            for _ in 0..240 {
                frame(&mut l, &scene(), vec![]);
                assert_eq!(l.refresh_world_pointer(true), []);
            }
        }
    }

    #[test]
    fn a_stationary_pointer_tracks_fling_and_semantic_focus_changes() {
        let mut l = layout(MapMovement::Drag);
        let p = (170, 300);
        moved(&mut l, (200, 300));
        button(&mut l, Button::Left, true, (200, 300));
        moved(&mut l, p);
        tick(&mut l);
        button(&mut l, Button::Left, false, p);
        // Seed the point reported at release, then let the view coast.
        l.refresh_world_pointer(true);
        let before = l.world_under(p).unwrap();
        tick(&mut l);
        let after = l.world_under(p).unwrap();
        assert_ne!(after, before);
        assert_eq!(
            l.refresh_world_pointer(true),
            [Input::WorldMove {
                x: after.0,
                y: after.1
            }]
        );
        let mut focus = scene();
        focus.camera_focus = Some(CameraFocus {
            serial: 2,
            center: (5000, 2000),
        });
        frame(&mut l, &focus, vec![]);
        let target = l.world_under(p).unwrap();
        assert_ne!(target, after);
        assert_eq!(
            l.refresh_world_pointer(true),
            [Input::WorldMove {
                x: target.0,
                y: target.1
            }]
        );
    }

    #[test]
    fn held_right_drag_retargets_during_autoscroll_but_release_keeps_the_destination() {
        for (from, edge, hit) in [
            ((80, 300), (0, 300), Hit::Main),
            ((720, 300), (799, 300), Hit::Pane),
        ] {
            let mut l = layout(MapMovement::Drag);
            moved(&mut l, from);
            assert_eq!(button(&mut l, Button::Right, true, from), []);
            assert!(matches!(
                moved(&mut l, edge)[..],
                [Input::WorldOrder {
                    action: WorldOrderAction::Preview,
                    ..
                }]
            ));
            let old = l.world_under(edge).unwrap();
            tick(&mut l);
            let target = l.world_under(edge).unwrap();
            assert_ne!(target, old);
            assert_eq!(
                l.refresh_world_pointer(true),
                [Input::WorldOrder {
                    action: WorldOrderAction::Preview,
                    x: target.0,
                    y: target.1
                }]
            );
            assert_eq!(l.refresh_world_pointer(true), []);
            assert_eq!(button(&mut l, Button::Right, false, edge), []);
            tick(&mut l);
            assert_eq!(
                l.world_under(edge),
                Some(target),
                "right release stops autoscroll"
            );
            assert_eq!(l.refresh_world_pointer(true), []);
            // A later camera change must not alter the released preview.
            let view = if hit == Hit::Main {
                &mut l.main_view
            } else {
                &mut l.split.as_mut().unwrap().view
            };
            view.follow = Follow::Free;
            view.center.0 += 80;
            frame(&mut l, &scene(), vec![]);
            assert_ne!(l.world_under(edge).unwrap(), target);
            for _ in 0..240 {
                assert_eq!(l.refresh_world_pointer(true), []);
            }
        }
    }

    #[test]
    fn unpresented_camera_attempts_do_not_advance_the_reported_pointer() {
        let mut l = layout(MapMovement::EdgeScroll);
        let p = (799, 300);
        moved(&mut l, p);
        let reported = l.last_world_pointer;
        // Simulate two acquisitions failing after layout: neither attempt
        // calls the presented-frame refresh hook.
        tick(&mut l);
        tick(&mut l);
        assert_eq!(l.last_world_pointer, reported);
        let presented = l.world_under(p).unwrap();
        assert_ne!(Some(presented), reported);
        assert_eq!(
            l.refresh_world_pointer(true),
            [Input::WorldMove {
                x: presented.0,
                y: presented.1
            }]
        );
        assert_eq!(l.refresh_world_pointer(true), []);
    }

    #[test]
    fn captured_cursor_camera_follow_does_not_repeat_already_sent_motion() {
        let mut l = layout(MapMovement::EdgeScroll);
        l.split.as_mut().unwrap().view.follow = Follow::Pointer;
        l.cursor = (7000, 3000);
        frame(&mut l, &scene(), vec![]);
        l.capture(Hit::Pane, (600, 300));
        let out = l.motion((20.0, -10.0));
        assert!(matches!(out[..], [Input::WorldMove { .. }]));
        tick(&mut l);
        for _ in 0..240 {
            assert_eq!(l.refresh_world_pointer(true), []);
        }
    }

    #[test]
    fn ui_snapping_inactive_windows_and_active_map_drags_do_not_leak_world_hover() {
        let p = (600, 300);
        let mut l = layout(MapMovement::EdgeScroll);
        moved(&mut l, p);
        l.split.as_mut().unwrap().view.center.0 += 40;
        frame(&mut l, &scene(), vec![[500, 100, 250, 350]]);
        assert!(l.route_point(p).is_ok());
        assert_eq!(l.refresh_world_pointer(true), []);
        frame(&mut l, &scene(), vec![]);
        assert_eq!(l.refresh_world_pointer(false), []);
        l.set_snapping(true);
        assert_eq!(l.refresh_world_pointer(true), []);
        l.set_snapping(false);
        assert!(matches!(
            l.refresh_world_pointer(true)[..],
            [Input::WorldMove { .. }]
        ));
        l.set_map_movement(MapMovement::Drag);
        button(&mut l, Button::Middle, true, p);
        moved(&mut l, (560, 300));
        tick(&mut l);
        assert!(l.pan_has_gesture());
        assert_eq!(l.refresh_world_pointer(true), []);
    }
}
