//! Viewer-clock confirmation for settings that can leave the game unusable
//! (`scene::SettingsPreview`): the pixel scale, a previewed menu size, the
//! window mode. Keep or Revert; nobody answering reverts by itself.
use scene::{Input, PreviewValue, SettingsPreview};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

/// How long the confirmation waits before it reverts by itself.
pub const TIMEOUT: Duration = Duration::from_secs(10);
#[derive(Default)]
pub struct Preview {
    active: Option<(SettingsPreview, Instant)>,
    settled: Option<((u64, u64), PreviewValue)>,
    keys: BTreeSet<u32>,
    press: Option<bool>,
    /// The countdown last drawn (`Preview::drew`).
    drawn: Option<u64>,
    /// The desktop's scale (window pixels per logical pixel; 0: 1): the
    /// confirmation's size follows it ([`Dialog::of`]).
    desktop_scale: f64,
}
impl Preview {
    pub fn active(&self) -> bool {
        self.active.is_some()
    }
    /// The desktop's scale the confirmation is sized for (the window's
    /// scale factor).
    pub fn set_desktop_scale(&mut self, scale: f64) {
        self.desktop_scale = scale;
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
    /// The settings the driver sent, with what this confirmation decided
    /// before the driver heard of it (or a newer change still asked about).
    pub fn project(
        &mut self,
        lease: Option<SettingsPreview>,
        settings: &mut viewer_settings::Settings,
    ) {
        if lease.is_none() {
            if let Some((_, value)) = &mut self.settled {
                *value = value.of(settings);
            }
        }
        if let Some(lease) = lease {
            if let Some((active, _)) = self.active {
                if (lease.session, lease.id) < (active.session, active.id) {
                    active.value.apply(settings);
                    return;
                }
            }
            if let Some((seen, value)) = self.settled {
                if (lease.session, lease.id) <= seen {
                    value.apply(settings);
                }
            }
        }
    }
    pub fn resolve(&mut self, keep: bool) -> Option<(Input, PreviewValue)> {
        let (lease, _) = self.active.take()?;
        let value = if keep { lease.value } else { lease.previous };
        self.settled = Some(((lease.session, lease.id), value));
        self.press = None;
        Some((
            Input::SettingsPreviewDecision {
                session: lease.session,
                id: lease.id,
                keep,
            },
            value,
        ))
    }
    pub fn tick(&mut self, now: Instant) -> Option<(Input, PreviewValue)> {
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
    /// When the countdown next changes (or the lease runs out): what it
    /// shows changes once a second, so it wakes the viewer no more often.
    pub fn wake(&self, now: Instant) -> Option<Instant> {
        let (_, deadline) = self.active?;
        let seconds = self.seconds(now).unwrap_or(0);
        Some(deadline.checked_sub(Duration::from_secs(seconds.saturating_sub(1))).unwrap_or(deadline).max(now))
    }
    /// Whether a frame must be drawn for the confirmation: it appeared,
    /// its countdown changed or it went (not while it waits unchanged).
    pub fn needs_frame(&self, now: Instant) -> bool {
        self.seconds(now) != self.drawn
    }
    /// The countdown the frame being drawn shows (`None`: no confirmation).
    pub fn drew(&mut self, seconds: Option<u64>) {
        self.drawn = seconds;
    }
    /// Physical window coordinates keep buttons fixed across multiplier changes.
    pub fn input(
        &mut self,
        input: &Input,
        pointer: (i32, i32),
        window: (u32, u32),
    ) -> (bool, Option<(Input, PreviewValue)>) {
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
                let hit = Dialog::of(window, self.desktop_scale)
                    .buttons()
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
                | Input::Scroll { .. }
                | Input::Slider { .. }
                | Input::WorldMove { .. }
                | Input::WorldButton { .. }
                | Input::WorldOrder { .. }
        );
        (consumed, result)
    }
}
fn contains([x, y, w, h]: [i32; 4], (px, py): (i32, i32)) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

/// The confirmation's design size, in logical pixels at 1×.
pub const DESIGN: (i32, i32) = (420, 160);
/// How much larger than [`DESIGN`] it is drawn, in logical pixels (times
/// the desktop's scale in window pixels), where the window has room.
pub const TARGET_SCALE: f64 = 2.0;

/// Where the confirmation is in a window: its size never follows the
/// settings it asks about (the pixel, menu or base UI scale), only the
/// window and the desktop's scale.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Dialog {
    /// Its panel, in window pixels.
    pub rect: [i32; 4],
    /// Window pixels per design pixel.
    pub scale: f64,
}

impl Dialog {
    /// [`DESIGN`] times [`TARGET_SCALE`] times the desktop's scale
    /// (`desktop_scale`, window pixels per logical pixel; 0 or less: 1),
    /// centred; in a window too small for that, as large as the window
    /// allows at the same aspect ratio: its whole width or its whole height,
    /// whichever runs out first.
    pub fn of(window: (u32, u32), desktop_scale: f64) -> Dialog {
        let (ww, wh) = (
            window.0.clamp(1, i32::MAX as u32) as i32,
            window.1.clamp(1, i32::MAX as u32) as i32,
        );
        let desktop = if desktop_scale.is_finite() && desktop_scale > 0.0 { desktop_scale } else { 1.0 };
        let target = TARGET_SCALE * desktop;
        let fit = (ww as f64 / DESIGN.0 as f64).min(wh as f64 / DESIGN.1 as f64);
        let scale = target.min(fit);
        let w = ((DESIGN.0 as f64 * scale).round() as i32).clamp(1, ww);
        let h = ((DESIGN.1 as f64 * scale).round() as i32).clamp(1, wh);
        Dialog { rect: [(ww - w) / 2, (wh - h) / 2, w, h], scale }
    }

    /// Keep and Revert, in window pixels.
    pub fn buttons(&self) -> [[i32; 4]; 2] {
        let [x, y, w, h] = self.rect;
        [17, 55].map(|left| [
            x + w * left / 100,
            y + h * 65 / 100,
            (w * 29 / 100).max(1),
            (h * 23 / 100).max(1),
        ])
    }

    /// The light frame around the panel, in window pixels.
    fn frame(&self) -> i32 {
        (2.0 * self.scale).round().max(1.0) as i32
    }

    /// The window pixels it draws (its panel and frame, within `window`).
    pub fn drawn_rect(&self, window: (u32, u32)) -> [i32; 4] {
        let [x, y, w, h] = self.rect;
        let frame = self.frame();
        let (x0, y0) = ((x - frame).max(0), (y - frame).max(0));
        let (x1, y1) = ((x + w + frame).min(window.0 as i32), (y + h + frame).min(window.1 as i32));
        [x0, y0, x1 - x0, y1 - y0]
    }
}

/// The confirmation over a frame shown as `present` says: drawn onto the
/// window after the picture (`Camera::on_target`), in its physical pixels
/// (readable at any pixel scale) whatever size the picture was composed
/// at, so it costs the GPU its panel's pixels only.
pub fn compose_presented(
    compiled: &render_desc::Compiled,
    present: tile_render::Present,
    desktop_scale: f64,
    seconds: u64,
    view: u32,
) -> (tile_render::Camera, Vec<tile_render::PlacedSprite>) {
    let (camera, sprites) = compose(compiled, present.target, desktop_scale, seconds, view);
    (tile_render::Camera { on_target: true, ..camera }, sprites)
}

/// The window pixels the confirmation draws (its panel and frame): all a
/// frame showing it costs more.
pub fn drawn_rect(window: (u32, u32), desktop_scale: f64) -> [i32; 4] {
    Dialog::of(window, desktop_scale).drawn_rect(window)
}

pub fn compose(
    compiled: &render_desc::Compiled,
    window: (u32, u32),
    desktop_scale: f64,
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
    let dialog = Dialog::of(window, desktop_scale);
    // No dimming of the whole window: shading every window pixel cost a
    // full frame's GPU time (some 2.6 ms at 3258×1904). A light frame
    // sets the panel off instead.
    out.push(solid(dialog.drawn_rect(window), [150, 150, 160, 255]));
    let [x, y, w, h] = dialog.rect;
    out.push(solid(dialog.rect, [40, 40, 45, 255]));
    let buttons = dialog.buttons();
    for button in buttons {
        out.push(solid(button, [85, 85, 95, 255]));
    }
    let required = "Do you want to keep this? Reverting in 0123456789 seconds Keep Revert";
    if let Some((font, face)) = crate::host_status::face_for(compiled, required) {
        let pad = (w / 14).min((30.0 * dialog.scale).round() as i32);
        // 18 design pixels (12% of its height), smaller where the text
        // would not fit.
        let size = ((18.0 * dialog.scale).round() as i32).min(h * 12 / 100).max(1);
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
            let mut px = size;
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
            value: PreviewValue::PixelScale(3),
            previous: PreviewValue::PixelScale(1),
        }
    }
    #[test]
    fn resolution_confirmation_keeps_the_base_and_fine_sizes() {
        let mut settings = viewer_settings::Settings::default();
        settings.resolution.base_ui_scale = 2.5;
        settings.resolution.frame_scale = 1.5;
        settings.resolution.world_scale = 0.75;
        let original = settings.resolution.clone();
        let mut preview = Preview::default();
        for keep in [false, true] {
            let lease = lease(if keep { 2 } else { 1 });
            preview.observe(Some(lease), Instant::now());
            lease.value.apply(&mut settings);
            let (_, scale) = preview.resolve(keep).unwrap();
            preview.project(Some(lease), &mut settings);
            assert_eq!(settings.resolution, viewer_settings::Resolution { pixel_scale: match scale { PreviewValue::PixelScale(n) => n, _ => unreachable!() }, ..original.clone() });
        }
    }

    #[test]
    fn stalled_publisher_expires_and_repeated_metadata_cannot_restart_or_reapply() {
        let now = Instant::now();
        let mut p = Preview::default();
        p.observe(Some(lease(1)), now);
        p.observe(Some(lease(1)), now + Duration::from_secs(9));
        assert_eq!(p.seconds(now + Duration::from_secs(9)), Some(1));
        assert_eq!(p.tick(now + TIMEOUT).unwrap().1, PreviewValue::PixelScale(1));
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
        assert_eq!(result.unwrap().1, PreviewValue::PixelScale(3));
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
            value: PreviewValue::PixelScale(4),
            previous: PreviewValue::PixelScale(2),
        };
        p.observe(Some(lease), Instant::now());
        assert_eq!(p.resolve(true).unwrap().1, PreviewValue::PixelScale(4));
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
            value: PreviewValue::PixelScale(4),
            previous: PreviewValue::PixelScale(2),
        };
        for scale in [1, 2, 4, 6] {
            let mut p = Preview::default();
            p.observe(
                Some(SettingsPreview {
                    value: PreviewValue::PixelScale(scale),
                    ..lease
                }),
                Instant::now(),
            );
            let [x, y, _, _] = Dialog::of((1200, 900), 1.0).buttons()[1];
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
                PreviewValue::PixelScale(2)
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
            PreviewValue::PixelScale(2)
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
            value: PreviewValue::PixelScale(3),
            previous: PreviewValue::PixelScale(1),
        };
        let next = SettingsPreview {
            id: 2,
            value: PreviewValue::PixelScale(4),
            ..old
        };
        let mut p = Preview::default();
        p.observe(Some(old), t);
        p.resolve(false);
        p.observe(Some(next), t);
        p.observe(Some(old), t);
        let mut source = viewer_settings::Settings::default();
        source.resolution.pixel_scale = 3;
        p.project(Some(old), &mut source);
        assert_eq!(source.resolution.pixel_scale, 4);
        p.resolve(true);
        p.observe(None, t);
        source.resolution.pixel_scale = 2;
        p.project(None, &mut source);
        source.resolution.pixel_scale = 3;
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
            value: PreviewValue::PixelScale(4),
            previous: PreviewValue::PixelScale(2),
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
                    // The frame composed at the canvas as ever; the dialog on
                    // the window after it, in its pixels.
                    let drawn = crate::scaling::drawn(&[camera], &[], source, |_| zoomed_art);
                    assert_eq!(drawn.present, source);
                    let (modal, commands) = compose_presented(&compiled, drawn.present, 1.0, 10, 1);
                    assert_eq!(
                        (modal.on_target, modal.cells, modal.zoom, modal.viewport, modal.origin, modal.screen_shift),
                        (true, false, (1, 1), window, (0, 0), (0, 0))
                    );
                    for (i, button) in commands
                        .iter()
                        .filter(|s| s.solid.is_some_and(|r| r.rgba == [85, 85, 95, 255]))
                        .enumerate()
                    {
                        let rect = button.solid.unwrap();
                        // Derive bounds from the commands sent to the renderer,
                        // not buttons(): window pixels, drawn as they are.
                        let (x, y) = (button.x, button.y);
                        assert!(x >= 0 && y >= 0);
                        assert!(x + rect.width as i32 <= window.0 as i32);
                        assert!(y + rect.height as i32 <= window.1 as i32);
                        let pointer = (x + rect.width as i32 / 2, y + rect.height as i32 / 2);
                        let mut preview = Preview::default();
                        preview.observe(
                            Some(SettingsPreview {
                                session: 1,
                                id: 1,
                                value: PreviewValue::PixelScale(n),
                                previous: PreviewValue::PixelScale(1),
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
        let (_, commands) = compose(&c, (240, 100), 1.0, 10, 0);
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
                value: PreviewValue::PixelScale(3),
                previous: PreviewValue::PixelScale(1),
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
    /// The countdown asks for a frame when it appears, once a second as
    /// its number changes, and when it goes; not while it waits unchanged.
    #[test]
    fn countdown_draws_once_a_second_not_every_wake() {
        let start = Instant::now();
        let mut p = Preview::default();
        assert!(!p.needs_frame(start));
        p.observe(Some(SettingsPreview { session: 1, id: 1, value: PreviewValue::PixelScale(6), previous: PreviewValue::PixelScale(1) }), start);
        assert!(p.needs_frame(start), "it appears");
        p.drew(p.seconds(start));
        assert_eq!(p.seconds(start), Some(10));
        let mut frames = 0;
        let mut at = start;
        for step in 1..=99u64 {
            let now = start + Duration::from_millis(100 * step);
            if p.needs_frame(now) {
                frames += 1;
                p.drew(p.seconds(now));
            }
            assert!(p.wake(at).is_some_and(|w| w > at && w <= start + TIMEOUT), "{step}");
            at = now;
        }
        assert_eq!(frames, 9, "10 → 1 seconds: one frame per change");
        // It wakes when the number next changes, not sooner.
        let half = start + Duration::from_millis(2500);
        assert_eq!(p.wake(half), Some(start + Duration::from_secs(3)));
        assert_eq!(p.wake(start), Some(start + Duration::from_secs(1)));
        p.resolve(true);
        assert!(p.needs_frame(start + TIMEOUT), "it goes");
        p.drew(None);
        assert!(!p.needs_frame(start + TIMEOUT));
    }
}

#[cfg(test)]
mod size_tests {
    use super::*;

    /// Twice its design size in logical pixels where the window has room;
    /// in a smaller window as large as it fits, its whole width or its
    /// whole height, at the same aspect ratio.
    #[test]
    fn the_dialog_is_twice_its_design_or_fills_the_window_by_width_or_height() {
        let ratio = |d: Dialog| d.rect[2] as f64 / d.rect[3] as f64;
        let design = DESIGN.0 as f64 / DESIGN.1 as f64;
        // Room: 840×320 logical pixels, centred, at any desktop scale.
        for (window, desktop) in [((1920, 1080), 1.0), ((3258, 1904), 2.0), ((2560, 1440), 1.5)] {
            let d = Dialog::of(window, desktop);
            let (w, h) = ((840.0 * desktop).round() as i32, (320.0 * desktop).round() as i32);
            assert_eq!(d.rect, [(window.0 as i32 - w) / 2, (window.1 as i32 - h) / 2, w, h], "{window:?} at {desktop}");
            assert_eq!(d.scale, 2.0 * desktop);
        }
        // Too narrow: the whole width.
        let d = Dialog::of((640, 480), 1.0);
        assert_eq!((d.rect[0], d.rect[2]), (0, 640));
        assert!(d.rect[3] < 480 && (ratio(d) - design).abs() < 0.02);
        // Too low: the whole height.
        let d = Dialog::of((1600, 240), 1.0);
        assert_eq!((d.rect[1], d.rect[3]), (0, 240));
        assert!(d.rect[2] < 1600 && (ratio(d) - design).abs() < 0.02);
        // A HiDPI window too small for its target: limited the same way.
        let d = Dialog::of((1280, 800), 2.0);
        assert_eq!((d.rect[0], d.rect[2]), (0, 1280));
        // Its buttons stay inside it, and clickable.
        for window in [(1920, 1080), (640, 480), (1600, 240), (100, 60)] {
            let d = Dialog::of(window, 1.0);
            for [x, y, w, h] in d.buttons() {
                assert!(w > 0 && h > 0 && x >= d.rect[0] && y >= d.rect[1]);
                assert!(x + w <= d.rect[0] + d.rect[2] && y + h <= d.rect[1] + d.rect[3]);
            }
            let [x, y, w, h] = d.drawn_rect(window);
            assert!(x >= 0 && y >= 0 && x + w <= window.0 as i32 && y + h <= window.1 as i32);
        }
    }

    /// The window mode and the menu size revert on the viewer's clock like
    /// the pixel scale: the settings it shows go back before the driver
    /// hears, and stay so until the driver's settings agree.
    #[test]
    fn timeout_reverts_the_window_mode_and_the_menu_size() {
        let t = Instant::now();
        for (value, previous) in [
            (PreviewValue::Windowed(true), PreviewValue::Windowed(false)),
            (PreviewValue::menus(true, 40.0), PreviewValue::menus(false, 100.0)),
        ] {
            let mut p = Preview::default();
            let lease = SettingsPreview { session: 2, id: 7, value, previous };
            p.observe(Some(lease), t);
            assert_eq!(p.tick(t + TIMEOUT - Duration::from_millis(1)), None);
            let (input, reverted) = p.tick(t + TIMEOUT).unwrap();
            assert_eq!(input, Input::SettingsPreviewDecision { session: 2, id: 7, keep: false });
            assert_eq!(reverted, previous);
            // The driver still sends the previewed value: shown reverted.
            let mut settings = viewer_settings::Settings::default();
            value.apply(&mut settings);
            p.project(Some(lease), &mut settings);
            assert_eq!(previous.of(&settings), previous);
        }
    }
}

#[cfg(test)]
mod window_mode_tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};
    struct NoImages;
    impl render_desc::ImageLoader for NoImages {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
            Err("no images".into())
        }
    }

    /// The viewer follows the window mode its driver's settings ask for,
    /// and goes back to the previous one when the confirmation times out.
    #[test]
    fn expiry_switches_the_window_mode_back() {
        let compiled = render_desc::Description::parse("grid={layout='orthogonal',step=[8,8]}").unwrap().compile(&NoImages, 16).unwrap();
        let (driver, display) = scene::connect();
        let config = crate::ViewerConfig { fullscreen: true, ..Default::default() };
        let mut app = crate::new_app(config, crate::Assets::new(Arc::new(compiled)), display, Arc::new(AtomicBool::new(false)));
        let t = Instant::now();
        let lease = SettingsPreview { session: 1, id: 1, value: PreviewValue::Windowed(true), previous: PreviewValue::Windowed(false) };
        app.settings_preview.observe(Some(lease), t);
        app.follow_window_mode(true);
        assert!(!app.config.fullscreen, "windowed while asking");
        let expired = app.settings_preview.tick(t + TIMEOUT);
        app.resolve_settings_preview(expired);
        assert!(app.config.fullscreen, "fullscreen again");
        assert!(!app.settings.display.windowed);
        assert_eq!(driver.take_input(), vec![Input::SettingsPreviewDecision { session: 1, id: 1, keep: false }]);
    }
}
