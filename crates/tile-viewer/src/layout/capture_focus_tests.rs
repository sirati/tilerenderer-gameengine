    #[test]
    fn captured_ai_focus_keeps_visible_tiles_still_and_only_exposes_clipped_edges() {
        for config in ["main=focus,pane=pointer,split=vertical", "main=pointer,pane=focus,split=vertical", "main=driver,pane=pointer,split=vertical", "main=focus,pane=pointer,split=horizontal"] {
            for smooth in [false, true] {
                let mut l = Layout::default();
                l.configure(config).unwrap();
                l.smooth = smooth;
                let mut s = scene();
                frame(&mut l, vec![]);
                l.locked = true;
                l.cursor_placed = true;
                l.cursor = (3000, 2000);
                let side = usize::from(l.main_view.follow == Follow::Pointer);
                if side == 0 { l.main_view.center = (3000, 2000); l.main_view.glide.snap((3000, 2000)); }
                else { let v = &mut l.split.as_mut().unwrap().view; v.center = (3000, 2000); v.glide.snap((3000, 2000)); v.hand_focus = s.focus; }
                for (serial, center) in [(1, (3000, 2000)), (2, (3064, 2032)), (3, (2936, 1968))] {
                    s.camera_focus = Some(scene::CameraFocus { serial, center });
                    l.step = 0.1;
                    l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                    let v = if side == 0 { l.main_view } else { l.split.unwrap().view };
                    assert_eq!(v.center, (3000, 2000), "{config}, smooth {smooth}, request {center:?}");
                    assert_eq!(v.glide.velocity(), (0.0, 0.0));
                }
                s.camera_focus = Some(scene::CameraFocus { serial: 4, center: (4000, 2500) });
                for _ in 0..120 {
                    l.step = 0.1;
                    l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                }
                let v = if side == 0 { l.main_view } else { l.split.unwrap().view };
                let rect = if side == 0 { l.panes().0 } else { l.panes().1.unwrap() };
                assert_ne!(v.center, (3000, 2000), "offscreen AI must remain reachable");
                assert_ne!(v.center, (4000, 2500), "expose rather than recenter");
                assert_eq!(v.expose_tile((4000, 2500), (64, 32), rect, BOUNDS, (1, 1)), v.center);
            }
        }
    }

    #[test]
    fn complete_diamond_extent_fits_at_every_edge_and_fractional_raster_zoom() {
        for zoom in 0..ZOOMS.len() {
            for scale in [(1, 1), (3, 2), (2, 3)] {
                for downscale in [1u32, 2, 3] {
                    let half = (64u32.div_ceil(downscale) as i32, 32u32.div_ceil(downscale) as i32);
                    let v = FreeView::new((3000, 2000), zoom, Follow::Focus);
                    let rect = [31, 17, 503, 407];
                    let c = v.camera(rect, (0, 0, 0, 0), None, scale);
                    let span = ((503u64 * c.zoom.1 as u64 / c.zoom.0 as u64) as i32, (407u64 * c.zoom.1 as u64 / c.zoom.0 as u64) as i32);
                    if span.0 < 2 * half.0 || span.1 < 2 * half.1 { continue; }
                    for dx in [-1, 0, 1] {
                        for dy in [-1, 0, 1] {
                            let tile = (if dx < 0 { c.x + half.0 - 1 } else if dx > 0 { c.x + span.0 - half.0 + 1 } else { 3000 },
                                        if dy < 0 { c.y + half.1 - 1 } else if dy > 0 { c.y + span.1 - half.1 + 1 } else { 2000 });
                            let moved = v.expose_tile(tile, half, rect, None, scale);
                            assert_eq!(moved, (3000 + dx, 2000 + dy));
                            let after = FreeView { center: moved, ..v }.camera(rect, (0, 0, 0, 0), None, scale);
                            // The four diamond extrema reach the full tile's raster bounds.
                            for p in [(tile.0 - half.0, tile.1), (tile.0 + half.0, tile.1), (tile.0, tile.1 - half.1), (tile.0, tile.1 + half.1)] {
                                assert!((p.0 - after.x) as i64 * c.zoom.0 as i64 >= 0);
                                assert!((p.1 - after.y) as i64 * c.zoom.0 as i64 >= 0);
                                assert!((p.0 - after.x) as i64 * c.zoom.0 as i64 <= rect[2] as i64 * c.zoom.1 as i64);
                                assert!((p.1 - after.y) as i64 * c.zoom.0 as i64 <= rect[3] as i64 * c.zoom.1 as i64);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn captured_visible_ai_stops_old_glide_but_uncaptured_ai_still_recenters() {
        for captured in [false, true] {
            let mut l = Layout::default();
            l.configure("main=focus,pane=pointer,split=vertical").unwrap();
            frame(&mut l, vec![]);
            l.smooth = true;
            l.locked = captured;
            l.cursor_placed = true;
            l.cursor = (3000, 2000);
            l.main_view.center = (3000, 2000);
            l.main_view.glide.snap(l.main_view.center);
            l.main_view.glide.vel = (90.0, -70.0);
            let mut s = scene();
            s.camera_focus = Some(scene::CameraFocus { serial: 1, center: (3064, 2032) });
            l.step = 0.1;
            l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            if captured {
                assert_eq!(l.main_view.center, (3000, 2000));
                assert_eq!(l.main_view.glide.velocity(), (0.0, 0.0));
            } else {
                assert_ne!(l.main_view.center, (3000, 2000));
            }
        }
    }

    #[test]
    fn captured_ai_extent_uses_clamped_camera_and_nearest_wrapped_tile() {
        let rect = [0, 0, 800, 600];
        let bounds = Some(WorldBounds { size: (8000, 4000), wrap: (false, false) });
        let v = FreeView::new((64, 32), ONE, Follow::Focus);
        assert_eq!(v.expose_tile((700, 500), (64, 32), rect, bounds, (1, 1)), v.center);
        let moved = v.expose_tile((900, 700), (64, 32), rect, bounds, (1, 1));
        assert_eq!(moved, (564, 432), "pan from the actual clamped viewport");
        assert_eq!(FreeView { center: moved, ..v }.expose_tile((900, 700), (64, 32), rect, bounds, (1, 1)), moved);
        let seam = FreeView::new((7950, 2000), ONE, Follow::Focus);
        assert_eq!(seam.expose_tile((14, 2000), (64, 32), rect, BOUNDS, (1, 1)), seam.center);
    }

    #[test]
    fn combat_arrival_requires_complete_area_in_the_primary_presented_view() {
        let mut l = Layout::default();
        l.configure("main=focus,pane=focus,split=vertical").unwrap();
        l.smooth = true;
        let mut s = scene();
        frame(&mut l, vec![]);
        l.locked = true;
        l.cursor_placed = true;
        l.main_view.center = (3000, 2000); l.main_view.glide.snap((3000, 2000));
        { let b = &mut l.split.as_mut().unwrap().view; b.center = (4000, 2500); b.glide.snap((4000, 2500)); }
        s.camera_focus = Some(scene::CameraFocus { serial: 17, center: (4000, 2500) });
        s.camera_arrival = Some(scene::CameraArrival { serial: 17, bounds: [3872, 2436, 4128, 2564] });
        l.step = 0.01;
        let first = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.camera_arrival_receipt(&s, &first), None, "B cannot acknowledge while required A is still offscreen");
        for _ in 0..120 { l.step = 0.1; l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
        let mut shown = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.camera_arrival_receipt(&s, &shown), Some(Input::CameraArrived { serial: 17 }));
        let main = l.shown.iter().find(|(h, _)| *h == Hit::Main).unwrap().1;
        let camera = shown.cameras.iter_mut().find(|c| c.cells && c.origin == main.origin && c.viewport == main.viewport).unwrap();
        camera.target_clip = Some([0, 0, 30, 30]);
        assert_eq!(l.camera_arrival_receipt(&s, &shown), Some(Input::CameraUnavailable { serial: 17 }), "a scissored-away fight cannot be acknowledged");
        s.camera_focus.as_mut().unwrap().serial = 18;
        assert_eq!(l.camera_arrival_receipt(&s, &shown), None, "an unrelated request serial cannot arrive");
    }

    #[test]
    fn combat_arrival_fails_open_for_no_view_and_impossible_region() {
        let mut l = Layout::default();
        let mut s = scene();
        s.camera_focus = Some(scene::CameraFocus { serial: 23, center: (3000, 2000) });
        s.camera_arrival = Some(scene::CameraArrival { serial: 23, bounds: [0, 0, 8000, 4000] });
        assert_eq!(l.camera_arrival_receipt(&s, &Frame::default()), Some(Input::CameraUnavailable { serial: 23 }));
        l.configure("main=focus,pane=pointer,split=vertical").unwrap();
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.camera_arrival_receipt(&s, &f), Some(Input::CameraUnavailable { serial: 23 }));
    }

    #[test]
    fn arrival_uses_b_focus_only_when_a_does_not_follow_the_request() {
        let mut l = Layout::default(); l.configure("main=focus,pane=focus,split=vertical").unwrap();
        let mut s = scene(); s.camera_focus = Some(scene::CameraFocus { serial: 31, center: (3000, 2000) });
        s.camera_arrival = Some(scene::CameraArrival { serial: 31, bounds: [2936, 1968, 3064, 2032] });
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        l.smooth = true;
        { let b = &mut l.split.as_mut().unwrap().view; b.center = (6000, 3000); b.glide.snap(b.center); }
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.camera_arrival_receipt(&s, &f), Some(Input::CameraArrived { serial: 31 }), "an unrelated B cannot block the complete primary A");
        l.main_view.follow = Follow::Pointer;
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.camera_arrival_receipt(&s, &f), None, "B now owns the focus and has not reached it");
        { let b = &mut l.split.as_mut().unwrap().view; b.center = (3000, 2000); b.glide.snap(b.center); }
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.camera_arrival_receipt(&s, &f), Some(Input::CameraArrived { serial: 31 }));
    }
