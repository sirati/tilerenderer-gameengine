//! Preserve world positions when Civ III changes its artwork scale with Z.
//! Window pixels and UI geometry stay unchanged; cached world coordinates
//! and motion state move to the new game-pixel coordinate system.

use super::{Layout, UNPLACED};

impl Layout {
    pub(super) fn rescale_world(&mut self, downscale: u32) {
        let downscale = downscale.max(1);
        let Some(previous) = self.world_downscale.replace(downscale) else { return };
        if previous == downscale { return; }
        let ratio = previous as f64 / downscale as f64;
        let point = |p: (i32, i32)| {
            if p == UNPLACED { p } else { ((p.0 as f64 * ratio).round() as i32, (p.1 as f64 * ratio).round() as i32) }
        };
        let fractional = |p: (f64, f64)| (p.0 * ratio, p.1 * ratio);
        let split = |p: (i32, i32), rest: (f64, f64)| {
            let exact = fractional((p.0 as f64 + rest.0, p.1 as f64 + rest.1));
            let whole = (exact.0.trunc() as i32, exact.1.trunc() as i32);
            (whole, (exact.0 - whole.0 as f64, exact.1 - whole.1 as f64))
        };
        for view in self.views_mut() {
            if view.center != UNPLACED {
                (view.center, view.scale_rest) = split(view.center, view.scale_rest);
            }
            view.hand_focus = view.hand_focus.map(point);
            view.glide.pos = fractional(view.glide.pos);
            view.glide.vel = fractional(view.glide.vel);
        }
        (self.cursor, self.motion_rest) = split(self.cursor, self.motion_rest);
        self.focus = self.focus.map(point);
        if let Some(focus) = self.focus_at_camera_request {
            let (focus, rest) = split(focus, self.focus_at_camera_request_rest);
            self.focus_at_camera_request = Some(focus);
            self.focus_at_camera_request_rest = rest;
        }
        self.last_world_pointer = self.last_world_pointer.map(point);
        self.magnifier_center = self.magnifier_center.map(point);
        self.recapture = self.recapture.map(point);
        self.recapture_screen = self.recapture_screen.map(point);
        for pressed in &mut self.world_pressed { *pressed = pressed.map(point); }
        if let Some(explore) = &mut self.explore { explore.center = point(explore.center); }
        // Input still consults the last shown cameras before this frame's
        // replacements exist. Rebase their centres, keeping physical span
        // and zoom, so edge pans and hover use the new world coordinates.
        for (_, camera) in &mut self.shown {
            let (w, h) = camera.world_size();
            let center = point((camera.x + w as i32 / 2, camera.y + h as i32 / 2));
            camera.x = center.0 - w as i32 / 2;
            camera.y = center.1 - h as i32 / 2;
            camera.downscale = downscale;
        }
        self.pan_rescale_world(ratio);
        // Unit-order gestures use game pixels too. Their helper also
        // rebases the captured pointer's projected threshold coordinates.
        self.unit_orders_rescale_world(ratio);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Follow, Hit, ONE, WorldBounds};
    use scene::{Button, CameraFocus, Input, Scene, View, ViewState};
    use viewer_settings::{Place, Roles, Split, Zoom};

    fn scene(downscale: u32) -> Scene {
        Scene {
            views: vec![ViewState { id: 0, view: View { downscale, ..Default::default() }, overlay: vec![] }],
            camera_focus: Some(CameraFocus { serial: 1, center: (3000 / downscale as i32, 2000 / downscale as i32) }),
            focus: Some((3000 / downscale as i32, 2000 / downscale as i32)),
            shape: scene::WorldShape { cols: 62, rows: 125, wrap_x: true, wrap_y: false },
            ..Default::default()
        }
    }

    fn frame(l: &mut Layout, scale: u32) -> crate::layout::Frame {
        l.frame(&scene(scale), (800, 600), (0, 0, 0, 0), vec![], Some(WorldBounds { size: (8000 / scale as i32, 4000 / scale as i32), wrap: (true, false) }))
    }

    fn pan_a(l: &mut Layout) {
        l.route(Input::MouseButton { button: Button::Middle, down: true, x: 200, y: 300 });
        l.route(Input::MouseMove { x: 193, y: 297 });
        l.route(Input::MouseButton { button: Button::Middle, down: false, x: 193, y: 297 });
        assert_eq!(l.main_view.center, (3007, 2003));
    }

    #[test]
    fn z_rebases_the_full_center_of_a_view_larger_than_a_source_picture() {
        let mut l = Layout::default();
        l.world_downscale = Some(1);
        l.shown = vec![(Hit::Main, tile_render::Camera {
            x: 100, y: 200, zoom: (1, 12), ..tile_render::Camera::new((100, 400))
        })];
        l.rescale_world(2);
        let c = l.shown[0].1;
        assert_eq!((c.x, c.y), (-250, -1100));
        assert_eq!(c.downscale, 2);
    }

    #[test]
    fn z_keeps_manual_a_free_b_and_insets_at_the_same_world_position() {
        for smooth in [false, true] {
            let mut l = Layout::default();
            l.apply_settings(&viewer_settings::Layout { split: Split::Vertical1To1, roles: Roles::Free, smooth_world_movement: smooth, zoom: Zoom::Floating131, zoom_place: Place::RightOrTop, ..Default::default() });
            frame(&mut l, 1);
            pan_a(&mut l);
            l.split.as_mut().unwrap().view.moved_by_hand((3507, 2003), None);
            l.insets.push(crate::layout::Inset { rect: [20, 20, 150, 100], view: crate::layout::FreeView::new((4507, 2203), ONE, Follow::Free) });
            for scale in [2, 1, 4, 1] {
                let f = frame(&mut l, scale);
                assert_eq!(l.main_view.center, (3007 / scale as i32, 2003 / scale as i32));
                assert_eq!(l.split.unwrap().view.center, (3507 / scale as i32, 2003 / scale as i32));
                assert_eq!(l.insets[0].view.center, (4507 / scale as i32, 2203 / scale as i32));
                assert_eq!(l.camera_focus_serial, Some(1));
                assert!(l.main_panned_by_hand());
                // The magnifier keeps its original full-detail art even
                // while the normal world views use the Z artwork.
                let logical = l.shown.iter().find(|(hit, _)| *hit == Hit::Magnifier).unwrap().1;
                let rendered = f.cameras.iter().find(|c| c.origin == logical.origin && c.cells).unwrap();
                assert_eq!(rendered.downscale, 1);
            }
        }
    }

    #[test]
    fn z_preserves_camera_centres_and_only_clamps_the_wider_field_at_map_edges() {
        for focus_y in [1080, 672] {
            let mut l = Layout::default();
            l.apply_settings(&viewer_settings::Layout { split: Split::Vertical1To1, roles: Roles::Free, ..Default::default() });
            l.world_scale = (5, 4);
            let mut scene_at = |scale: u32| {
                let mut s = scene(scale);
                s.camera_focus.as_mut().unwrap().center = (2496 / scale as i32, focus_y / scale as i32);
                s.focus = s.camera_focus.map(|f| f.center);
                let bounds = WorldBounds { size: (8000 / scale as i32, 4000 / scale as i32), wrap: (true, false) };
                l.frame(&s, (1920, 1080), (0, 0, 0, 0), vec![], Some(bounds));
            };
            scene_at(1);
            drop(scene_at);
            l.split.as_mut().unwrap().view.moved_by_hand((3100, 1080), None);
            for scale in [1, 2, 1, 2, 1] {
                let mut s = scene(scale);
                let requested = (2496 / scale as i32, focus_y / scale as i32);
                s.camera_focus.as_mut().unwrap().center = requested;
                s.focus = Some(requested);
                let bounds = WorldBounds { size: (8000 / scale as i32, 4000 / scale as i32), wrap: (true, false) };
                l.frame(&s, (1920, 1080), (0, 0, 0, 0), vec![], Some(bounds));
                assert_eq!(l.main_view.center, requested, "Z must not reset the requested centre");
                assert_eq!(l.split.unwrap().view.center, (3100 / scale as i32, 1080 / scale as i32));
                for (hit, camera) in l.shown.iter().filter(|(hit, _)| matches!(hit, Hit::Main | Hit::Pane)) {
                    let (w, h) = camera.world_size();
                    let wanted = if *hit == Hit::Main { requested } else { (3100 / scale as i32, 1080 / scale as i32) };
                    let actual = (camera.x + w as i32 / 2, camera.y + h as i32 / 2);
                    assert_eq!(actual, (bounds.clamp_center(0, wanted.0, w as i32), bounds.clamp_center(1, wanted.1, h as i32)));
                    assert_eq!(h, 864);
                    if *hit == Hit::Main && focus_y == 672 && scale == 2 {
                        assert_eq!((wanted.1, actual.1), (336, 432), "only the reduced view's wider field reaches the top edge");
                    } else {
                        assert_eq!(actual, wanted, "an interior view must preserve its exact centre");
                    }
                }
            }
        }
    }

    fn settle(l: &mut Layout, s: &Scene) {
        let scale = crate::world_downscale(s) as i32;
        for _ in 0..120 {
            l.advance(std::time::Duration::from_millis(16));
            l.frame(s, (800, 600), (0, 0, 0, 0), vec![], Some(WorldBounds { size: (8000 / scale, 4000 / scale), wrap: (true, false) }));
        }
    }

    fn click(l: &mut Layout, at: (i32, i32)) -> Vec<Input> {
        let mut out = l.route(Input::MouseButton { button: Button::Left, down: true, x: at.0, y: at.1 });
        out.extend(l.route(Input::MouseButton { button: Button::Left, down: false, x: at.0, y: at.1 }));
        out
    }

    #[test]
    fn unit_focus_after_selection_in_either_view_overrides_only_older_semantic_requests() {
        for smooth in [false, true] {
            let mut l = Layout::default();
            l.apply_settings(&viewer_settings::Layout { split: Split::Vertical1To1, roles: Roles::UnitMouse, smooth_world_movement: smooth, ..Default::default() });
            let mut s = scene(1);
            settle(&mut l, &s);
            assert!(l.capture_at((600, 300)));
            l.cursor = (6500, 2600);
            settle(&mut l, &s);
            let out = click(&mut l, (600, 300));
            assert!(out.iter().any(|input| matches!(input, Input::WorldButton { button: Button::Left, down: true, x: 6500, y: 2600 })));
            // Selection through B changes only the selected-unit focus.
            // The native camera stays parked and emits no new request.
            s.focus = Some((6500, 2600));
            settle(&mut l, &s);
            assert_eq!((l.main_view.center, l.split.unwrap().view.center), ((6500, 2600), (6500, 2600)));
            l.release_here("test selection in A");
            l.route(Input::MouseMove { x: 100, y: 300 });
            let selected = l.world_under((100, 300)).unwrap();
            let out = click(&mut l, (100, 300));
            assert!(out.iter().any(|input| matches!(input, Input::WorldButton { button: Button::Left, down: true, x, y } if (*x, *y) == selected)));
            s.focus = Some(selected);
            settle(&mut l, &s);
            assert_eq!((l.main_view.center, l.split.unwrap().view.center), (selected, (6500, 2600)));
            // A new visible AI movement wins while the player's active
            // unit stays selected, then a later unit selection wins again.
            s.camera_focus = Some(CameraFocus { serial: 2, center: (4500, 1800) });
            settle(&mut l, &s);
            assert_eq!((l.main_view.center, l.split.unwrap().view.center), ((4500, 1800), (6500, 2600)));
            s.focus = Some((3600, 2400));
            settle(&mut l, &s);
            assert_eq!(l.main_view.center, (3600, 2400));
            s.focus = Some(selected);
            settle(&mut l, &s);
            assert_eq!(l.main_view.center, selected, "returning to an old unit must not revive the earlier AI request");
            // Z changes coordinates without changing either focus signal.
            s.views[0].view.downscale = 2;
            s.camera_focus.as_mut().unwrap().center = (2250, 900);
            s.focus = Some((selected.0 / 2, selected.1 / 2));
            settle(&mut l, &s);
            assert_eq!((l.main_view.center, l.split.unwrap().view.center), ((selected.0 / 2, selected.1 / 2), (3250, 1300)));
            s.camera_focus.as_mut().unwrap().serial = 3;
            settle(&mut l, &s);
            assert_eq!((l.main_view.center, l.split.unwrap().view.center), ((2250, 900), (3250, 1300)));
            assert_eq!(l.main_view.follow, Follow::Focus);
        }
    }

    #[test]
    fn z_does_not_end_an_ai_focus_when_the_unit_baseline_has_odd_pixels() {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: Split::Vertical1To1, roles: Roles::UnitMouse, ..Default::default() });
        let mut s = scene(1);
        s.focus = Some((3001, 2003));
        s.camera_focus = Some(CameraFocus { serial: 7, center: (4500, 1800) });
        settle(&mut l, &s);
        for scale in [2, 1, 4, 1] {
            s.views[0].view.downscale = scale;
            s.focus = Some((3001 / scale as i32, 2003 / scale as i32));
            s.camera_focus.as_mut().unwrap().center = (4500 / scale as i32, 1800 / scale as i32);
            settle(&mut l, &s);
            assert!(l.semantic_focus_active);
            assert_eq!(l.main_view.center, (4500 / scale as i32, 1800 / scale as i32));
        }
    }

    #[test]
    fn explicit_driver_follow_keeps_semantic_camera_target_after_a_unit_focus_change() {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: Split::Vertical1To1, roles: Roles::Free, ..Default::default() });
        let mut s = scene(1);
        s.camera_focus = Some(CameraFocus { serial: 1, center: (4500, 1800) });
        settle(&mut l, &s);
        let b = l.split.unwrap().view.center;
        s.focus = Some((6500, 2600));
        settle(&mut l, &s);
        assert_eq!((l.main_view.center, l.split.unwrap().view.center, l.main_view.follow), ((4500, 1800), b, Follow::Driver));
    }

    #[test]
    fn semantic_a_uses_the_same_world_edge_clamping_as_b() {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: Split::Vertical1To1, roles: Roles::Free, ..Default::default() });
        let mut s = scene(1);
        s.camera_focus = Some(CameraFocus { serial: 1, center: (0, 0) });
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], Some(WorldBounds { size: (8000, 4000), wrap: (false, false) }));
        let a = f.cameras.iter().find(|c| c.origin == (0, 0)).unwrap();
        let b = f.cameras.iter().find(|c| c.origin == (404, 0)).unwrap();
        assert_eq!((a.x, a.y, b.x, b.y), (0, 0, 0, 0));
        assert!(a.cull.is_none() && b.cull.is_none());
    }

    #[test]
    fn z_preserves_captured_cursor_and_fractional_motion_without_refocusing_b() {
        let mut l = Layout::default();
        l.apply_settings(&viewer_settings::Layout { split: Split::Vertical1To1, roles: Roles::UnitMouse, ..Default::default() });
        frame(&mut l, 1);
        pan_a(&mut l);
        assert!(l.capture_at((600, 400)));
        l.cursor = (3507, 2003);
        l.motion_rest = (0.25, -0.25);
        frame(&mut l, 1);
        for scale in [2, 1, 4, 1] {
            frame(&mut l, scale);
            let exact = (l.cursor.0 as f64 + l.motion_rest.0, l.cursor.1 as f64 + l.motion_rest.1);
            assert_eq!(exact, (3507.25 / scale as f64, 2002.75 / scale as f64));
            assert_eq!(l.split.unwrap().view.center, l.cursor);
            assert_eq!(l.main_view.center, (3007 / scale as i32, 2003 / scale as i32));
            assert!(l.captured());
        }
    }
}
