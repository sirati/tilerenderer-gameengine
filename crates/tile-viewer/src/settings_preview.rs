//! Viewer-clock confirmation for temporary render multipliers.
use scene::{Input, SettingsPreview};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(10);
#[derive(Default)]
pub struct Preview {
    active: Option<(SettingsPreview, Instant)>,
    settled: Option<((u64, u64), u32)>,
    keys: BTreeSet<u32>,
    press: Option<bool>,
}
impl Preview {
    pub fn active(&self) -> bool {
        self.active.is_some()
    }
    pub fn seed_held_keys(&mut self, keys: impl IntoIterator<Item = u32>) {
        self.keys
            .extend(keys.into_iter().filter(|vk| !matches!(vk, 16..=18)));
    }

    /// Repeated metadata never renews a lease or reapplies a settled candidate.
    pub fn observe(&mut self, lease: Option<SettingsPreview>, now: Instant) {
        let Some(lease) = lease else {
            self.active = None;
            return;
        };
        let token = (lease.session, lease.id);
        if self.settled.is_some_and(|(seen, _)| token <= seen) {
            return;
        }
        if self.active.is_some_and(|(a, _)| (a.session, a.id) >= token) {
            return;
        }
        self.active = Some((lease, now + TIMEOUT));
        self.press = None;
    }
    pub fn project(
        &mut self,
        lease: Option<SettingsPreview>,
        settings: &mut viewer_settings::Settings,
    ) {
        if lease.is_none() {
            if let Some((_, scale)) = &mut self.settled {
                *scale = settings.resolution.pixel_scale;
            }
        }
        if let Some(lease) = lease {
            if let Some((active, _)) = self.active {
                if (lease.session, lease.id) < (active.session, active.id) {
                    settings.resolution.pixel_scale = active.pixel_scale;
                    return;
                }
            }
            if let Some((seen, scale)) = self.settled {
                if (lease.session, lease.id) <= seen {
                    settings.resolution.pixel_scale = scale;
                }
            }
        }
    }
    pub fn resolve(&mut self, keep: bool) -> Option<(Input, u32)> {
        let (lease, _) = self.active.take()?;
        let scale = if keep {
            lease.pixel_scale
        } else {
            lease.previous_pixel_scale
        };
        self.settled = Some(((lease.session, lease.id), scale));
        self.press = None;
        Some((
            Input::SettingsPreviewDecision {
                session: lease.session,
                id: lease.id,
                keep,
            },
            scale,
        ))
    }
    pub fn tick(&mut self, now: Instant) -> Option<(Input, u32)> {
        if self.active.is_some_and(|(_, deadline)| now >= deadline) {
            self.resolve(false)
        } else {
            None
        }
    }
    pub fn seconds(&self, now: Instant) -> Option<u64> {
        self.active.map(|(_, deadline)| {
            deadline.saturating_duration_since(now).as_secs_f64().ceil() as u64
        })
    }
    pub fn wake(&self, now: Instant) -> Option<Instant> {
        self.active
            .map(|(_, deadline)| deadline.min(now + Duration::from_millis(100)))
    }
    /// Physical window coordinates keep buttons fixed across multiplier changes.
    pub fn input(
        &mut self,
        input: &Input,
        pointer: (i32, i32),
        window: (u32, u32),
    ) -> (bool, Option<(Input, u32)>) {
        if let Input::Key { vk, down, .. } | Input::WorldKey { vk, down, .. } = input {
            // Modifiers retain their paired releases in the native dialog.
            if matches!(*vk, 16..=18) {
                return (false, None);
            }
            if self.keys.contains(vk) {
                if !down {
                    self.keys.remove(vk);
                }
                return (true, None);
            }
            if self.active() {
                if *down {
                    self.keys.insert(*vk);
                }
                return (
                    true,
                    if *down && (*vk == 13 || *vk == 27) {
                        self.resolve(*vk == 13)
                    } else {
                        None
                    },
                );
            }
        }
        if !self.active() {
            return (false, None);
        }
        let result = match input {
            Input::MouseButton {
                button: scene::Button::Left,
                down,
                ..
            } => {
                let hit = buttons(window)
                    .into_iter()
                    .enumerate()
                    .find(|(_, r)| contains(*r, pointer))
                    .map(|(i, _)| i == 0);
                if *down {
                    self.press = hit;
                    None
                } else {
                    let pressed = self.press.take();
                    if pressed.is_some() && pressed == hit {
                        self.resolve(hit.unwrap())
                    } else {
                        None
                    }
                }
            }
            _ => None,
        };
        let consumed = matches!(
            input,
            Input::MouseMove { .. }
                | Input::MouseButton { .. }
                | Input::Wheel { .. }
                | Input::Slider { .. }
                | Input::WorldMove { .. }
                | Input::WorldButton { .. }
                | Input::UnitOrder { .. }
        );
        (consumed, result)
    }
}
fn contains([x, y, w, h]: [i32; 4], (px, py): (i32, i32)) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

fn panel(window: (u32, u32)) -> [i32; 4] {
    let (ww, wh) = (
        window.0.max(1).min(i32::MAX as u32) as i32,
        window.1.max(1).min(i32::MAX as u32) as i32,
    );
    let margin = if ww >= 32 && wh >= 32 { 8 } else { 0 };
    let (w, h) = ((ww - 2 * margin).min(420), (wh - 2 * margin).min(160));
    [(ww - w) / 2, (wh - h) / 2, w, h]
}
fn buttons(window: (u32, u32)) -> [[i32; 4]; 2] {
    let [x, y, w, h] = panel(window);
    [
        [
            x + w * 17 / 100,
            y + h * 65 / 100,
            (w * 29 / 100).max(1),
            (h * 23 / 100).max(1),
        ],
        [
            x + w * 55 / 100,
            y + h * 65 / 100,
            (w * 29 / 100).max(1),
            (h * 23 / 100).max(1),
        ],
    ]
}

/// Commands are in physical window pixels. The camera removes the final
/// presentation border origin so the output transform restores it exactly.
pub fn compose_presented(
    compiled: &render_desc::Compiled,
    present: tile_render::Present,
    seconds: u64,
    view: u32,
) -> (tile_render::Camera, Vec<tile_render::PlacedSprite>) {
    let (mut camera, sprites) = compose(compiled, present.target, seconds, view);
    camera.viewport = present.canvas;
    camera.screen_shift = (-present.at.0, -present.at.1);
    (camera, sprites)
}

pub fn compose(
    compiled: &render_desc::Compiled,
    window: (u32, u32),
    seconds: u64,
    view: u32,
) -> (tile_render::Camera, Vec<tile_render::PlacedSprite>) {
    use tile_render::{Camera, GlyphShape, OutlineGlyph, PlacedSprite, SolidRect};
    let mut out = Vec::new();
    let solid = |[x, y, w, h]: [i32; 4], rgba| PlacedSprite {
        screen: true,
        view: Some(view),
        solid: Some(SolidRect {
            width: w as u32,
            height: h as u32,
            rgba,
        }),
        ..PlacedSprite::new(tile_render::SpriteSetId::EMPTY, x, y)
    };
    out.push(solid(
        [0, 0, window.0 as i32, window.1 as i32],
        [0, 0, 0, 150],
    ));
    let rect = panel(window);
    let [x, y, w, h] = rect;
    out.push(solid(rect, [40, 40, 45, 255]));
    let buttons = buttons(window);
    for button in buttons {
        out.push(solid(button, [85, 85, 95, 255]));
    }
    let required = "Do you want to keep this? Reverting in 0123456789 seconds Keep Revert";
    if let Some((font, face)) = crate::host_status::face_for(compiled, required) {
        let pad = (w / 14).min(30);
        for (text, area, baseline) in [
            (
                "Do you want to keep this?".to_owned(),
                [x + pad, y, w - 2 * pad, h],
                y + h * 22 / 100,
            ),
            (
                format!("Reverting in {seconds} seconds"),
                [x + pad, y, w - 2 * pad, h],
                y + h * 45 / 100,
            ),
            ("Keep".into(), buttons[0], y + h * 80 / 100),
            ("Revert".into(), buttons[1], y + h * 80 / 100),
        ] {
            let em = face.units_per_em().max(1) as i32;
            let width = |px: i32| {
                text.chars()
                    .map(|ch| face.advance(ch).unwrap_or(0) * px / em)
                    .sum::<i32>()
            };
            let mut px = (h * 12 / 100).clamp(1, 18);
            while px > 1 && width(px) > area[2] {
                px -= 1;
            }
            let mut pen = area[0] + (area[2] - width(px)).max(0) / 2;
            for ch in text.chars() {
                let advance = face.advance(ch).unwrap_or(0) * px / em;
                let glyph = OutlineGlyph {
                    font,
                    shape: GlyphShape::Glyph(face.glyph(ch).expect("complete coverage face")),
                    em64: px as u32 * 64,
                    advance,
                    bold: false,
                    italic: false,
                    color: [255, 255, 255],
                    key: None,
                };
                out.push(PlacedSprite {
                    screen: true,
                    view: Some(view),
                    clip: Some([area[0], area[1], area[0] + area[2], area[1] + area[3]]),
                    text: Some(glyph),
                    ..PlacedSprite::new(tile_render::SpriteSetId::EMPTY, pen, baseline)
                });
                pen += advance;
            }
        }
    }
    (
        Camera {
            cells: false,
            ..Camera::new(window)
        },
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lease(id: u64) -> SettingsPreview {
        SettingsPreview {
            session: 1,
            id,
            pixel_scale: 3,
            previous_pixel_scale: 1,
        }
    }
    #[test]
    fn stalled_publisher_expires_and_repeated_metadata_cannot_restart_or_reapply() {
        let now = Instant::now();
        let mut p = Preview::default();
        p.observe(Some(lease(1)), now);
        p.observe(Some(lease(1)), now + Duration::from_secs(9));
        assert_eq!(p.seconds(now + Duration::from_secs(9)), Some(1));
        assert_eq!(p.tick(now + TIMEOUT).unwrap().1, 1);
        p.observe(Some(lease(1)), now + TIMEOUT);
        assert!(!p.active());
        let mut settings = viewer_settings::Settings::default();
        settings.resolution.pixel_scale = 3;
        p.project(Some(lease(1)), &mut settings);
        assert_eq!(settings.resolution.pixel_scale, 1);
        p.observe(Some(lease(2)), now + TIMEOUT);
        assert!(p.active());
    }
    #[test]
    fn enter_repeat_and_release_never_reach_preferences() {
        let mut p = Preview::default();
        p.observe(Some(lease(1)), Instant::now());
        assert!(
            !p.input(
                &Input::Key {
                    vk: 16,
                    down: false,
                    text: None
                },
                (0, 0),
                (1000, 800)
            )
            .0
        );
        let key = |down| Input::Key {
            vk: 13,
            down,
            text: None,
        };
        let (used, result) = p.input(&key(true), (0, 0), (1000, 800));
        assert!(used);
        assert_eq!(result.unwrap().1, 3);
        assert!(p.input(&key(true), (0, 0), (1000, 800)).0);
        assert!(p.input(&key(false), (0, 0), (1000, 800)).0);
        assert!(!p.input(&key(true), (0, 0), (1000, 800)).0);
    }
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    #[test]
    fn keep_does_not_override_later_authoritative_cancel() {
        let mut p = Preview::default();
        let lease = SettingsPreview {
            session: 5,
            id: 1,
            pixel_scale: 4,
            previous_pixel_scale: 2,
        };
        p.observe(Some(lease), Instant::now());
        assert_eq!(p.resolve(true).unwrap().1, 4);
        p.observe(None, Instant::now());
        let mut authoritative = viewer_settings::Settings::default();
        authoritative.resolution.pixel_scale = 2;
        p.project(None, &mut authoritative);
        assert_eq!(authoritative.resolution.pixel_scale, 2);
    }
    #[test]
    fn physical_buttons_ignore_canvas_scale_and_escape_release_is_consumed() {
        let lease = SettingsPreview {
            session: 5,
            id: 1,
            pixel_scale: 4,
            previous_pixel_scale: 2,
        };
        for scale in [1, 2, 4, 6] {
            let mut p = Preview::default();
            p.observe(
                Some(SettingsPreview {
                    pixel_scale: scale,
                    ..lease
                }),
                Instant::now(),
            );
            let [x, y, _, _] = buttons((1200, 900))[1];
            let mouse = |down| Input::MouseButton {
                button: scene::Button::Left,
                down,
                x: 1,
                y: 1,
            };
            assert!(p.input(&mouse(true), (x + 10, y + 10), (1200, 900)).0);
            assert_eq!(
                p.input(&mouse(false), (x + 10, y + 10), (1200, 900))
                    .1
                    .unwrap()
                    .1,
                2
            );
        }
        let mut p = Preview::default();
        p.observe(Some(lease), Instant::now());
        assert_eq!(
            p.input(
                &Input::Key {
                    vk: 27,
                    down: true,
                    text: None
                },
                (0, 0),
                (1, 1)
            )
            .1
            .unwrap()
            .1,
            2
        );
        assert!(
            p.input(
                &Input::Key {
                    vk: 27,
                    down: false,
                    text: None
                },
                (0, 0),
                (1, 1)
            )
            .0
        );
    }
}

#[cfg(test)]
mod stale_tests {
    use super::*;
    #[test]
    fn stale_metadata_cannot_replace_a_newer_active_or_authoritative_multiplier() {
        let t = Instant::now();
        let old = SettingsPreview {
            session: 10,
            id: 1,
            pixel_scale: 3,
            previous_pixel_scale: 1,
        };
        let next = SettingsPreview {
            id: 2,
            pixel_scale: 4,
            ..old
        };
        let mut p = Preview::default();
        p.observe(Some(old), t);
        p.resolve(false);
        p.observe(Some(next), t);
        p.observe(Some(old), t);
        let mut source = viewer_settings::Settings::default();
        source.resolution.pixel_scale = old.pixel_scale;
        p.project(Some(old), &mut source);
        assert_eq!(source.resolution.pixel_scale, 4);
        p.resolve(true);
        p.observe(None, t);
        source.resolution.pixel_scale = 2;
        p.project(None, &mut source);
        source.resolution.pixel_scale = old.pixel_scale;
        p.project(Some(old), &mut source);
        assert_eq!(source.resolution.pixel_scale, 2);
    }
}

#[cfg(test)]
mod app_tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};
    struct NoImages;
    impl render_desc::ImageLoader for NoImages {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
            Err("no images".into())
        }
    }
    #[test]
    fn expiry_rolls_back_without_gpu_and_sends_even_while_a_frame_is_preparing() {
        let compiled = render_desc::Description::parse("grid={layout='orthogonal',step=[8,8]}")
            .unwrap()
            .compile(&NoImages, 16)
            .unwrap();
        let (driver, display) = scene::connect();
        let mut app = crate::new_app(
            crate::ViewerConfig::default(),
            crate::Assets::new(Arc::new(compiled)),
            display,
            Arc::new(AtomicBool::new(false)),
        );
        let t = Instant::now();
        let lease = SettingsPreview {
            session: 1,
            id: 1,
            pixel_scale: 4,
            previous_pixel_scale: 2,
        };
        app.settings_preview.observe(Some(lease), t);
        let mut candidate = app.settings.clone();
        candidate.resolution.pixel_scale = 4;
        app.set_settings(candidate);
        let pending = crate::ui_submission::Mapping::capture(&app);
        app.preparing_inputs = Some(Vec::new());
        let expired = app.settings_preview.tick(t + TIMEOUT);
        app.resolve_settings_preview(expired);
        assert!(app.gpu.is_none());
        assert_eq!(app.settings.resolution.pixel_scale, 2);
        assert!(app.preparing_inputs.as_ref().unwrap().is_empty());
        assert_eq!(
            driver.take_input(),
            vec![Input::SettingsPreviewDecision {
                session: 1,
                id: 1,
                keep: false
            }]
        );
        let _ = pending.activate(&mut app, false);
        assert_eq!(
            app.settings.resolution.pixel_scale, 2,
            "a pending frame cannot undo local rollback"
        );
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use tile_render::{Camera, Present};
    struct NoImages;
    impl render_desc::ImageLoader for NoImages {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
            Err("no images".into())
        }
    }
    fn compiled() -> render_desc::Compiled {
        render_desc::Description::parse("grid={layout='orthogonal',step=[8,8]}")
            .unwrap()
            .compile(&NoImages, 16)
            .unwrap()
    }
    #[test]
    fn rendered_buttons_are_clickable_through_actual_boxing_and_border_transform() {
        let compiled = compiled();
        for window in [(64, 48), (180, 90), (320, 120), (1207, 907)] {
            for n in 1..=6 {
                for zoomed_art in [false, true] {
                    let canvas = (window.0 / n, window.1 / n);
                    let at = (
                        ((window.0 - canvas.0 * n) / 2) as i32,
                        ((window.1 - canvas.1 * n) / 2) as i32,
                    );
                    let source = Present {
                        canvas,
                        zoom: (n, 1),
                        at,
                        target: window,
                    };
                    let camera = Camera {
                        zoom: (3, 2),
                        zoomed_art,
                        ..Camera::new(canvas)
                    };
                    let drawn =
                        crate::scaling::drawn_physical(&[camera], &[], source, |_| zoomed_art);
                    assert_eq!(drawn.present.zoom, (1, 1));
                    let (modal, commands) = compose_presented(&compiled, drawn.present, 10, 1);
                    assert_eq!(modal.zoom, (1, 1));
                    for (i, button) in commands
                        .iter()
                        .filter(|s| s.solid.is_some_and(|r| r.rgba == [85, 85, 95, 255]))
                        .enumerate()
                    {
                        let rect = button.solid.unwrap();
                        // Screen sprites receive camera.screen_shift before final Present.
                        // Derive bounds from the commands sent to the renderer, not buttons().
                        let canvas_x = button.x + modal.screen_shift.0;
                        let canvas_y = button.y + modal.screen_shift.1;
                        assert!(canvas_x >= 0 && canvas_y >= 0);
                        assert!(canvas_x + rect.width as i32 <= modal.viewport.0 as i32);
                        assert!(canvas_y + rect.height as i32 <= modal.viewport.1 as i32);
                        let x = canvas_x + drawn.present.at.0;
                        let y = canvas_y + drawn.present.at.1;
                        let pointer = (x + rect.width as i32 / 2, y + rect.height as i32 / 2);
                        let mut preview = Preview::default();
                        preview.observe(
                            Some(SettingsPreview {
                                session: 1,
                                id: 1,
                                pixel_scale: n,
                                previous_pixel_scale: 1,
                            }),
                            Instant::now(),
                        );
                        let click = |down| Input::MouseButton {
                            button: scene::Button::Left,
                            down,
                            x: 0,
                            y: 0,
                        };
                        assert!(preview.input(&click(true), pointer, window).0);
                        let result = preview.input(&click(false), pointer, window).1.unwrap().0;
                        assert!(
                            matches!(result,Input::SettingsPreviewDecision{keep,..} if keep==(i==0)),
                            "n={n} window={window:?} zoomed_art={zoomed_art}"
                        );
                    }
                    for command in &commands {
                        if let Some(clip) = command.clip {
                            assert!(
                                clip[0] >= 0
                                    && clip[1] >= 0
                                    && clip[2] <= window.0 as i32
                                    && clip[3] <= window.1 as i32
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn partial_first_font_uses_complete_fallback_for_every_prompt_glyph() {
        let mut c = compiled();
        c.fonts.insert(
            0,
            (
                "partial".into(),
                std::sync::Arc::new(tile_render::gpu_text::Face::from_curves(
                    1000,
                    &[('D', Vec::new(), 600)],
                )),
            ),
        );
        let (_, commands) = compose(&c, (240, 100), 10, 0);
        let text: Vec<_> = commands.iter().filter_map(|s| s.text).collect();
        assert!(!text.is_empty());
        assert!(text.iter().all(|g| g.font != 0));
        for g in text {
            let tile_render::GlyphShape::Glyph(id) = g.shape else {
                panic!("glyph expected")
            };
            assert!(id < c.fonts[g.font as usize].1.glyph_count());
        }
    }
    #[test]
    fn held_enter_repeat_cannot_confirm_until_release_and_fresh_press() {
        let mut p = Preview::default();
        p.observe(
            Some(SettingsPreview {
                session: 1,
                id: 1,
                pixel_scale: 3,
                previous_pixel_scale: 1,
            }),
            Instant::now(),
        );
        p.seed_held_keys([13, 18]);
        let key = |down| Input::Key {
            vk: 13,
            down,
            text: None,
        };
        for _ in 0..3 {
            assert!(p.input(&key(true), (0, 0), (500, 400)).1.is_none());
            assert!(p.active());
        }
        assert!(p.input(&key(false), (0, 0), (500, 400)).0);
        assert!(p.active());
        assert!(p.input(&key(true), (0, 0), (500, 400)).1.is_some());
    }
}
