//! The settings as named controls, for a preferences screen: every setting
//! the screen edits has a key (`"layout.split"`, `"resolution.scale_menus"`,
//! ...) and a value of one of three kinds — a choice among named options, a
//! switch, a number. A screen shows [`Settings::control`] and hands what the
//! user did to [`Settings::set_control`], which checks and clamps it; the
//! settings stay the one source of truth.
//!
//! Choices are named like the enum variants they stand for (`"Vertical2To1"`,
//! `"LeftOrBottom"`); the pixel scale by its number (`"1"` .. `"6"`).

use crate::{BackgroundSampling, Layout, MapMovement, Place, Resolution, Roles, Settings, Split, UnitButtons, Zoom, ZoomContent};

/// A control's value.
#[derive(Clone, Debug, PartialEq)]
pub enum ControlValue {
    /// One of named options.
    Choice(String),
    /// A switch.
    On(bool),
    Number(f32),
}

/// The range and step of a number control.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Range {
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

impl Range {
    /// `v` on the nearest step from `min`, within the range (a value that is
    /// no number becomes `min`).
    pub fn snap(&self, v: f32) -> f32 {
        if !v.is_finite() {
            return self.min;
        }
        let v = v.clamp(self.min, self.max);
        let steps = ((v - self.min) / self.step).round();
        // Rounded to the step's precision: no 0.30000001.
        let snapped = self.min + steps * self.step;
        ((snapped * 1000.0).round() / 1000.0).clamp(self.min, self.max)
    }
}

/// The fractional scales of the game's frame and world map.
pub const SCALE_RANGE: Range = Range { min: 0.5, max: 3.0, step: 0.05 };
/// Standard menu height relative to the window.
pub const MENU_SCALE_PERCENT_RANGE: Range = Range { min: 25.0, max: 100.0, step: 1.0 };
/// The height of every zoom: the strip, the floating boxes and the boxes
/// at the mouse ([`Layout::zoom_strip`]).
pub const ZOOM_STRIP_RANGE: Range = Range { min: 0.05, max: 0.5, step: 0.05 };
/// A floating box's space below its region's top ([`Layout::zoom_box_top`]).
pub const ZOOM_BOX_TOP_RANGE: Range = Range { min: 0.0, max: 0.05, step: 0.005 };
/// The gap between where a floating box rests and where it moves out of the
/// pointer's way ([`Layout::zoom_box_drop`]).
pub const ZOOM_BOX_DROP_RANGE: Range = Range { min: 0.05, max: 0.15, step: 0.005 };
/// The space between the pointer and a box at the mouse above it
/// ([`Layout::zoom_mouse_above`]).
pub const ZOOM_MOUSE_ABOVE_RANGE: Range = Range { min: 0.0, max: 0.3, step: 0.01 };
/// The space between the pointer and a box at the mouse below it, where
/// there is no room above ([`Layout::zoom_mouse_below`]).
pub const ZOOM_MOUSE_BELOW_RANGE: Range = Range { min: 0.0, max: 0.4, step: 0.01 };
/// The zoom's magnification as an index into [`Layout::MAGNIFICATIONS`]
/// (`"layout.zoom_magnification_step"`).
pub const ZOOM_MAGNIFICATION_STEP_RANGE: Range = Range { min: 0.0, max: (Layout::MAGNIFICATIONS.len() - 1) as f32, step: 1.0 };
/// How often the snapping cursor's target is recomputed, per second.
pub const SNAP_RATE_RANGE: Range = Range { min: 1.0, max: 30.0, step: 1.0 };
/// How often the snapping cursor's ghost follows, per second.
pub const SNAP_GHOST_RANGE: Range = Range { min: 1.0, max: 120.0, step: 1.0 };
/// Additional sensitivity for captured world-cursor motion.
pub const CAPTURE_SENSITIVITY_RANGE: Range = Range { min: 0.1, max: 10.0, step: 0.1 };

/// Fighting animation speed in quarter steps.
pub const COMBAT_SPEED_RANGE: Range = Range { min: 1.0, max: 4.0, step: 0.25 };

/// Explicit frame limiter, including high-refresh displays.
pub const FPS_LIMIT_RANGE: Range = Range { min: 30.0, max: 480.0, step: 1.0 };

/// Every key [`Settings::control`] answers (besides the derived
/// `"layout.orientation"`). `"layout.screen_place"` is the one place of
/// every screen, for the split's orientation.
pub const KEYS: [&str; 42] = [
    "resolution.pixel_scale",
    "resolution.base_ui_scale",
    "resolution.scale_menus",
    "resolution.menu_scale_percent",
    "resolution.frame_scale",
    "resolution.world_scale",
    "resolution.background_sampling",
    "layout.split",
    "layout.separator_width",
    "layout.zoom_rounded",
    "layout.zoom_glass",
    "layout.zoom_place",
    "layout.zoom",
    "layout.zoom_content",
    "layout.zoom_strip",
    "layout.zoom_box_top",
    "layout.zoom_box_drop",
    "layout.zoom_mouse_above",
    "layout.zoom_mouse_below",
    "layout.zoom_magnification_step",
    "layout.zoom_box_block_pointer",
    "layout.menu_follows_pointer",
    "layout.screen_place",
    "layout.smooth_world_movement",
    "layout.roles",
    "layout.unit_buttons",
    "input.snap_updates_per_second",
    "input.snap_ghost_fps",
    "input.capture_sensitivity",
    "animation.combat_speed",
    "input.map_movement",
    "input.smooth_menu_scrolling",
    "input.snap_lists_to_items",
    "input.alt_in_menus",
    "input.capture_function_keys",
    "input.media_keys_as_function_keys",
    "display.show_fps",
    "display.fps_detailed",
    "display.vsync",
    "display.limit_fps",
    "display.fps_limit",
    "display.cursor_size",
];

/// The range of number control `key`, if it is one.
pub fn range(key: &str) -> Option<Range> {
    if key.starts_with("accessibility.") { return crate::accessibility::range(key); }
    match key {
        "resolution.base_ui_scale" => Some(Range { min: 1.0, max: Resolution::MAX_PIXEL_SCALE as f32, step: 0.5 }),
        "resolution.menu_scale_percent" => Some(MENU_SCALE_PERCENT_RANGE),
        "resolution.frame_scale" | "resolution.world_scale" => Some(SCALE_RANGE),
        "layout.separator_width" => Some(Range { min: 1.0, max: 16.0, step: 1.0 }),
        "layout.zoom_strip" => Some(ZOOM_STRIP_RANGE),
        "layout.zoom_box_top" => Some(ZOOM_BOX_TOP_RANGE),
        "layout.zoom_box_drop" => Some(ZOOM_BOX_DROP_RANGE),
        "layout.zoom_mouse_above" => Some(ZOOM_MOUSE_ABOVE_RANGE),
        "layout.zoom_mouse_below" => Some(ZOOM_MOUSE_BELOW_RANGE),
        "layout.zoom_magnification_step" => Some(ZOOM_MAGNIFICATION_STEP_RANGE),
        "input.snap_updates_per_second" => Some(SNAP_RATE_RANGE),
        "input.snap_ghost_fps" => Some(SNAP_GHOST_RANGE),
        "input.capture_sensitivity" => Some(CAPTURE_SENSITIVITY_RANGE),
        "display.fps_limit" => Some(FPS_LIMIT_RANGE),
        "animation.combat_speed" => Some(COMBAT_SPEED_RANGE),
        _ => None,
    }
}

/// Enums named by their variants.
trait Named: Sized + Copy + PartialEq + 'static {
    const ALL: &'static [(Self, &'static str)];

    fn name(self) -> &'static str {
        Self::ALL.iter().find(|(v, _)| *v == self).map_or("", |(_, n)| n)
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().find(|(_, n)| *n == name).map(|(v, _)| *v)
    }
}

impl Named for Split {
    const ALL: &'static [(Self, &'static str)] = &[
        (Split::Standard, "Standard"),
        (Split::Vertical1To1, "Vertical1To1"),
        (Split::Vertical2To1, "Vertical2To1"),
        (Split::Horizontal1To1, "Horizontal1To1"),
    ];
}

impl Named for Place {
    const ALL: &'static [(Self, &'static str)] =
        &[(Place::Floating, "Floating"), (Place::LeftOrBottom, "LeftOrBottom"), (Place::RightOrTop, "RightOrTop")];
}

impl Named for Zoom {
    const ALL: &'static [(Self, &'static str)] = &[
        (Zoom::Off, "Off"),
        (Zoom::Floating131, "Floating131"),
        (Zoom::Floating171, "Floating171"),
        (Zoom::Mouse111, "Mouse111"),
        (Zoom::Mouse131, "Mouse131"),
        (Zoom::TopStrip, "TopStrip"),
        (Zoom::Beside, "Beside"),
    ];
}

impl Named for ZoomContent {
    const ALL: &'static [(Self, &'static str)] = &[(ZoomContent::World, "World"), (ZoomContent::WorldAndMenus, "WorldAndMenus")];
}

impl Named for Roles {
    const ALL: &'static [(Self, &'static str)] = &[(Roles::Free, "Free"), (Roles::UnitMouse, "UnitMouse")];
}

impl Named for UnitButtons {
    const ALL: &'static [(Self, &'static str)] = &[(UnitButtons::Spanning, "Spanning"), (UnitButtons::ActiveView, "ActiveView")];
}

impl Named for BackgroundSampling {
    const ALL: &'static [(Self, &'static str)] = &[
        (BackgroundSampling::Dither, "Dither"),
        (BackgroundSampling::Interpolate, "Interpolate"),
        (BackgroundSampling::Nearest, "Nearest"),
    ];
}

impl Named for MapMovement {
    const ALL: &'static [(Self, &'static str)] = &[(MapMovement::EdgeScroll, "EdgeScroll"), (MapMovement::Drag, "Drag")];
}

fn choice<T: Named>(v: T) -> Option<ControlValue> {
    Some(ControlValue::Choice(v.name().to_owned()))
}

/// Sets `slot` to the option named by `value`; whether that is an option.
fn set_choice<T: Named>(slot: &mut T, value: &ControlValue) -> bool {
    match value {
        ControlValue::Choice(name) => T::from_name(name).map(|v| *slot = v).is_some(),
        _ => false,
    }
}

fn set_number(slot: &mut f32, key: &str, value: &ControlValue) -> bool {
    match (value, range(key)) {
        (ControlValue::Number(v), Some(r)) => {
            *slot = r.snap(*v);
            true
        }
        _ => false,
    }
}

/// Whether screens are placed for a horizontal split (else a vertical one:
/// [`crate::ScreenPlace::for_split`]).
fn horizontal(l: &Layout) -> bool {
    l.split == Split::Horizontal1To1
}

impl Settings {
    /// The value of control `key`; `None` for a key there is no such control
    /// for. Besides the settings: `"layout.orientation"`, `"vertical"` or
    /// `"horizontal"`, which of the two screen places the screen shows.
    pub fn control(&self, key: &str) -> Option<ControlValue> {
        let (r, l, i) = (&self.resolution, &self.layout, &self.input);
        if key.starts_with("clean_map.") { return self.clean_map.control(key); }
        if key.starts_with("accessibility.") { return self.accessibility.control(key); }
        match key {
            "resolution.pixel_scale" => Some(ControlValue::Choice(r.effective_pixel_scale().to_string())),
            "resolution.base_ui_scale" => Some(ControlValue::Number(r.effective_base_ui_scale())),
            "resolution.scale_menus" => Some(ControlValue::On(r.scale_menus)),
            "resolution.menu_scale_percent" => Some(ControlValue::Number(r.effective_menu_scale_percent())),
            "resolution.frame_scale" => Some(ControlValue::Number(r.frame_scale)),
            "resolution.world_scale" => Some(ControlValue::Number(r.world_scale)),
            "resolution.background_sampling" => choice(r.background_sampling),
            "layout.split" => choice(l.split),
            "layout.zoom_place" => choice(l.zoom_place),
            "layout.zoom" => choice(l.zoom),
            "layout.zoom_content" => choice(l.zoom_content),
            "layout.separator_width" => Some(ControlValue::Number(l.separator_width.clamp(1, 16) as f32)),
            "layout.zoom_rounded" => Some(ControlValue::On(l.zoom_rounded)),
            "layout.zoom_glass" => Some(ControlValue::On(l.zoom_glass)),
            "layout.zoom_strip" => Some(ControlValue::Number(l.zoom_strip)),
            "layout.zoom_box_top" => Some(ControlValue::Number(l.zoom_box_top)),
            "layout.zoom_box_drop" => Some(ControlValue::Number(l.zoom_box_drop)),
            "layout.zoom_mouse_above" => Some(ControlValue::Number(l.zoom_mouse_above)),
            "layout.zoom_mouse_below" => Some(ControlValue::Number(l.zoom_mouse_below)),
            "layout.zoom_magnification_step" => Some(ControlValue::Number(l.zoom_magnification_step() as f32)),
            "layout.zoom_box_block_pointer" => Some(ControlValue::On(l.zoom_box_block_pointer)),
            "layout.menu_follows_pointer" => Some(ControlValue::On(l.menu_follows_pointer)),
            "layout.smooth_world_movement" => Some(ControlValue::On(l.smooth_world_movement)),
            "layout.roles" => choice(l.roles),
            "layout.unit_buttons" => choice(l.unit_buttons),
            "layout.orientation" => Some(ControlValue::Choice(if horizontal(l) { "horizontal" } else { "vertical" }.into())),
            "layout.screen_place" => choice(l.screen_place.for_split(l.split)),
            "input.snap_updates_per_second" => Some(ControlValue::Number(i.snap_updates_per_second)),
            "input.snap_ghost_fps" => Some(ControlValue::Number(i.snap_ghost_fps)),
            "input.capture_sensitivity" => Some(ControlValue::Number(i.capture_sensitivity)),
            "animation.combat_speed" => Some(ControlValue::Number(self.animation.effective_combat_speed())),
            "input.map_movement" => choice(i.map_movement),
            "input.smooth_menu_scrolling" => Some(ControlValue::On(i.smooth_menu_scrolling)),
            "input.snap_lists_to_items" => Some(ControlValue::On(i.snap_lists_to_items)),
            "input.alt_in_menus" => Some(ControlValue::On(i.alt_in_menus)),
            "input.capture_function_keys" => Some(ControlValue::On(i.capture_function_keys)),
            "input.media_keys_as_function_keys" => Some(ControlValue::On(i.media_keys_as_function_keys)),
            "display.vsync" => Some(ControlValue::On(self.display.present == crate::Present::Vsync)),
            "display.limit_fps" => Some(ControlValue::On(self.display.max_fps > 0)),
            "display.fps_limit" => Some(ControlValue::Number(FPS_LIMIT_RANGE.snap(if self.display.max_fps > 0 { self.display.max_fps } else { self.display.fps_limit } as f32))),
            "display.show_fps" => Some(ControlValue::On(self.display.show_fps)),
            "display.fps_detailed" => Some(ControlValue::On(self.display.fps_detailed)),
            "display.cursor_size" => Some(ControlValue::Choice(match self.display.cursor_size {
                crate::Display::SYSTEM_CURSOR_SIZE => "system".into(),
                n => n.to_string(),
            })),
            _ => None,
        }
    }

    /// Every control and its value (see [`Settings::control`]).
    pub fn controls(&self) -> Vec<(String, ControlValue)> {
        KEYS.iter()
            .map(|k| k.to_string())
            .chain(std::iter::once("layout.orientation".to_owned()))
            .chain(crate::clean_map::CleanMap::keys())
            .chain(crate::accessibility::KEYS.into_iter().map(str::to_owned))
            .filter_map(|k| self.control(&k).map(|v| (k, v)))
            .collect()
    }

    /// Copy one numeric control exactly, without snapping a previously
    /// accepted value to today's slider steps. Used by live preview and
    /// per-slider undo. Other controls, including the FPS enable switch,
    /// keep their current values.
    pub fn restore_number_control(&mut self, key: &str, source: &Self) -> bool {
        let before = self.clone();
        macro_rules! copy { ($group:ident, $field:ident) => { self.$group.$field = source.$group.$field }; }
        match key {
            "resolution.base_ui_scale" => copy!(resolution, base_ui_scale),
            "resolution.menu_scale_percent" => copy!(resolution, menu_scale_percent),
            "resolution.frame_scale" => copy!(resolution, frame_scale),
            "resolution.world_scale" => copy!(resolution, world_scale),
            "layout.separator_width" => copy!(layout, separator_width),
            "layout.zoom_strip" => copy!(layout, zoom_strip),
            "layout.zoom_box_top" => copy!(layout, zoom_box_top),
            "layout.zoom_box_drop" => copy!(layout, zoom_box_drop),
            "layout.zoom_mouse_above" => copy!(layout, zoom_mouse_above),
            "layout.zoom_mouse_below" => copy!(layout, zoom_mouse_below),
            "layout.zoom_magnification_step" => copy!(layout, zoom_magnification),
            "input.snap_updates_per_second" => copy!(input, snap_updates_per_second),
            "input.snap_ghost_fps" => copy!(input, snap_ghost_fps),
            "input.capture_sensitivity" => copy!(input, capture_sensitivity),
            "animation.combat_speed" => copy!(animation, combat_speed),
            "accessibility.ring_width" => copy!(accessibility, ring_width),
            "accessibility.hud_size" => copy!(accessibility, hud_size),
            "accessibility.arrow_size" => copy!(accessibility, arrow_size),
            "display.fps_limit" => {
                if (self.display.max_fps > 0) == (source.display.max_fps > 0) {
                    copy!(display, fps_limit);
                    copy!(display, max_fps);
                } else if let Some(ControlValue::Number(limit)) = source.control(key) {
                    self.display.fps_limit = limit as u32;
                    if self.display.max_fps > 0 { self.display.max_fps = limit as u32; }
                }
            }
            _ => return false,
        }
        *self != before
    }

    /// Sets control `key` to `value` (numbers clamped to their range and
    /// snapped to its step); whether the settings changed. The screens'
    /// place is set for the split's orientation, the other kept.
    pub fn set_control(&mut self, key: &str, value: &ControlValue) -> bool {
        if key.starts_with("clean_map.") { return self.clean_map.set_control(key, value); }
        if key.starts_with("accessibility.") { return self.accessibility.set_control(key, value); }
        let before = self.clone();
        let ok = {
            let (r, l, i, d) = (&mut self.resolution, &mut self.layout, &mut self.input, &mut self.display);
            match key {
                "resolution.pixel_scale" => match value {
                    ControlValue::Choice(n) => match n.parse::<u32>() {
                        Ok(n) if (1..=Resolution::MAX_PIXEL_SCALE).contains(&n) => {
                            r.pixel_scale = n;
                            true
                        }
                        _ => false,
                    },
                    _ => false,
                },
                "resolution.scale_menus" => match value {
                    ControlValue::On(on) => { r.scale_menus = *on; true }
                    _ => false,
                },
                "resolution.base_ui_scale" => set_number(&mut r.base_ui_scale, key, value),
                "resolution.menu_scale_percent" => set_number(&mut r.menu_scale_percent, key, value),
                "resolution.frame_scale" => set_number(&mut r.frame_scale, key, value),
                "resolution.world_scale" => set_number(&mut r.world_scale, key, value),
                "resolution.background_sampling" => set_choice(&mut r.background_sampling, value),
                "layout.split" => set_choice(&mut l.split, value),
                "layout.zoom_place" => set_choice(&mut l.zoom_place, value),
                "layout.zoom" => set_choice(&mut l.zoom, value),
                "layout.zoom_content" => set_choice(&mut l.zoom_content, value),
                "layout.separator_width" => {
                    let mut width = l.separator_width as f32;
                    let ok = set_number(&mut width, key, value);
                    if ok { l.separator_width = width as u32; }
                    ok
                }
                "layout.zoom_rounded" | "layout.zoom_glass" => match value {
                    ControlValue::On(on) => {
                        match key {
                            "layout.zoom_rounded" => l.zoom_rounded = *on,
                            _ => l.zoom_glass = *on,
                        }
                        true
                    }
                    _ => false,
                },
                "layout.zoom_strip" => set_number(&mut l.zoom_strip, key, value),
                "layout.zoom_box_top" => set_number(&mut l.zoom_box_top, key, value),
                "layout.zoom_box_drop" => set_number(&mut l.zoom_box_drop, key, value),
                "layout.zoom_mouse_above" => set_number(&mut l.zoom_mouse_above, key, value),
                "layout.zoom_mouse_below" => set_number(&mut l.zoom_mouse_below, key, value),
                "layout.zoom_magnification_step" => {
                    let mut step = l.zoom_magnification_step() as f32;
                    let set = set_number(&mut step, key, value);
                    if set {
                        l.zoom_magnification = Layout::MAGNIFICATIONS[step as usize];
                    }
                    set
                }
                "layout.zoom_box_block_pointer" => match value {
                    ControlValue::On(on) => {
                        l.zoom_box_block_pointer = *on;
                        true
                    }
                    _ => false,
                },
                "layout.menu_follows_pointer" => match value {
                    ControlValue::On(on) => {
                        l.menu_follows_pointer = *on;
                        true
                    }
                    _ => false,
                },
                "layout.smooth_world_movement" => match value {
                    ControlValue::On(on) => {
                        l.smooth_world_movement = *on;
                        true
                    }
                    _ => false,
                },
                "layout.roles" => set_choice(&mut l.roles, value),
                "layout.unit_buttons" => set_choice(&mut l.unit_buttons, value),
                "input.snap_updates_per_second" => set_number(&mut i.snap_updates_per_second, key, value),
                "input.snap_ghost_fps" => set_number(&mut i.snap_ghost_fps, key, value),
                "input.capture_sensitivity" => set_number(&mut i.capture_sensitivity, key, value),
                "animation.combat_speed" => set_number(&mut self.animation.combat_speed, key, value),
                "input.map_movement" => set_choice(&mut i.map_movement, value),
                "input.smooth_menu_scrolling" => match value {
                    ControlValue::On(on) => {
                        i.smooth_menu_scrolling = *on;
                        true
                    }
                    _ => false,
                },
                "input.snap_lists_to_items" => match value {
                    ControlValue::On(on) => {
                        i.snap_lists_to_items = *on;
                        true
                    }
                    _ => false,
                },
                "input.alt_in_menus" => match value {
                    ControlValue::On(on) => {
                        i.alt_in_menus = *on;
                        true
                    }
                    _ => false,
                },
                "input.capture_function_keys" => match value {
                    ControlValue::On(on) => {
                        i.capture_function_keys = *on;
                        true
                    }
                    _ => false,
                },
                "input.media_keys_as_function_keys" => match value {
                    ControlValue::On(on) => {
                        i.media_keys_as_function_keys = *on;
                        true
                    }
                    _ => false,
                },
                "display.vsync" => match value {
                    ControlValue::On(on) => {
                        if *on { d.present = crate::Present::Vsync; }
                        else if d.present == crate::Present::Vsync { d.present = crate::Present::Immediate; }
                        true
                    }
                    _ => false,
                },
                "display.limit_fps" => match value {
                    ControlValue::On(on) => {
                        if *on {
                            if d.max_fps == 0 { d.max_fps = FPS_LIMIT_RANGE.snap(d.fps_limit as f32) as u32; }
                        } else if d.max_fps > 0 {
                            d.fps_limit = FPS_LIMIT_RANGE.snap(d.max_fps as f32) as u32;
                            d.max_fps = 0;
                        }
                        true
                    }
                    _ => false,
                },
                "display.fps_limit" => match value {
                    ControlValue::Number(n) => {
                        d.fps_limit = FPS_LIMIT_RANGE.snap(*n) as u32;
                        if d.max_fps > 0 { d.max_fps = d.fps_limit; }
                        true
                    }
                    _ => false,
                },
                "display.show_fps" => match value {
                    ControlValue::On(on) => {
                        d.show_fps = *on;
                        true
                    }
                    _ => false,
                },
                "display.fps_detailed" => match value {
                    ControlValue::On(on) => {
                        d.fps_detailed = *on;
                        if *on { d.show_fps = true; }
                        true
                    }
                    _ => false,
                },
                "display.cursor_size" => match value {
                    ControlValue::Choice(n) if n == "system" => {
                        d.cursor_size = crate::Display::SYSTEM_CURSOR_SIZE;
                        true
                    }
                    ControlValue::Choice(n) => match n.parse::<u32>() {
                        Ok(n) if crate::Display::CURSOR_SIZES.contains(&n) => {
                            d.cursor_size = n;
                            true
                        }
                        _ => false,
                    },
                    _ => false,
                },
                "layout.screen_place" => {
                    let horizontal = horizontal(l);
                    let slot = if horizontal { &mut l.screen_place.horizontal } else { &mut l.screen_place.vertical };
                    set_choice(slot, value)
                }
                _ => false,
            }
        };
        ok && *self != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScreenPlace;

    #[test]
    fn numeric_restore_preserves_exact_values_and_independent_fps_switch() {
        let mut accepted = Settings::default();
        accepted.resolution.frame_scale = 1.234;
        accepted.layout.zoom_strip = 0.123;
        let mut draft = accepted.clone();
        draft.resolution.frame_scale = 2.0;
        draft.layout.zoom_strip = 0.4;
        draft.layout.zoom_box_drop = 0.5;
        assert!(draft.restore_number_control("resolution.frame_scale", &accepted));
        assert!(draft.restore_number_control("layout.zoom_strip", &accepted));
        assert_eq!(draft.resolution.frame_scale, 1.234);
        assert_eq!(draft.layout.zoom_strip, 0.123);
        assert_eq!(draft.layout.zoom_box_drop, 0.5);
        // The boxes at the mouse: each its own.
        (draft.layout.zoom_mouse_above, draft.layout.zoom_mouse_below) = (0.3, 0.4);
        accepted.layout.zoom_mouse_above = 0.123;
        assert!(draft.restore_number_control("layout.zoom_mouse_above", &accepted));
        assert_eq!((draft.layout.zoom_mouse_above, draft.layout.zoom_mouse_below), (0.123, 0.4));
        assert!(draft.restore_number_control("layout.zoom_mouse_below", &accepted));
        assert_eq!(draft.layout.zoom_mouse_below, 0.2);
        assert!(!draft.restore_number_control("resolution.frame_scale", &accepted));
        assert!(!draft.restore_number_control("resolution.pixel_scale", &accepted));
        // The active unit's arrow size: its own, the HUD's left alone.
        (draft.accessibility.arrow_size, draft.accessibility.hud_size, accepted.accessibility.arrow_size) = (4, 2, 1);
        assert!(draft.restore_number_control("accessibility.arrow_size", &accepted));
        assert_eq!((draft.accessibility.arrow_size, draft.accessibility.hud_size), (1, 2));
        accepted.display.max_fps = 144;
        draft.display.max_fps = 0;
        draft.display.fps_limit = 480;
        assert!(draft.restore_number_control("display.fps_limit", &accepted));
        assert_eq!((draft.display.max_fps, draft.display.fps_limit), (0, 144));
        accepted.display.max_fps = 0;
        accepted.display.fps_limit = 240;
        draft.display.max_fps = 480;
        assert!(draft.restore_number_control("display.fps_limit", &accepted));
        assert_eq!((draft.display.max_fps, draft.display.fps_limit), (240, 240));
        accepted.display.max_fps = 144;
        accepted.display.fps_limit = 120;
        draft.display.max_fps = 480;
        draft.restore_number_control("display.fps_limit", &accepted);
        assert_eq!((draft.display.max_fps, draft.display.fps_limit), (144, 120), "undo restores the exact accepted backing fields when enable is unchanged");
    }

    #[test]
    fn pacing_controls_preserve_legacy_config_and_remember_disabled_limit() {
        let mut settings: Settings = toml::from_str("[display]\npresent = 'Mailbox'\nmax_fps = 240\n").unwrap();
        assert_eq!(settings.control("display.vsync"), Some(ControlValue::On(false)));
        assert_eq!(settings.control("display.limit_fps"), Some(ControlValue::On(true)));
        assert_eq!(settings.control("display.fps_limit"), Some(ControlValue::Number(240.0)));
        assert!(!settings.set_control("display.vsync", &ControlValue::On(false)));
        assert_eq!(settings.display.present, crate::Present::Mailbox);
        assert!(settings.set_control("display.limit_fps", &ControlValue::On(false)));
        assert_eq!((settings.display.max_fps, settings.display.fps_limit), (0, 240));
        assert!(settings.set_control("display.fps_limit", &ControlValue::Number(360.0)));
        assert_eq!(settings.display.max_fps, 0, "moving a disabled slider must not turn the limiter on");
        let saved = toml::to_string(&settings).unwrap();
        settings = toml::from_str(&saved).unwrap();
        assert!(settings.set_control("display.limit_fps", &ControlValue::On(true)));
        assert_eq!(settings.display.max_fps, 360);
        assert!(settings.set_control("display.fps_limit", &ControlValue::Number(999.0)));
        assert_eq!((settings.display.max_fps, settings.display.fps_limit), (480, 480));
        assert!(settings.set_control("display.fps_limit", &ControlValue::Number(f32::NAN)));
        assert_eq!(settings.display.max_fps, 30);
        assert!(settings.set_control("display.vsync", &ControlValue::On(true)));
        assert_eq!(settings.display.present, crate::Present::Vsync);
        assert!(settings.set_control("display.vsync", &ControlValue::On(false)));
        assert_eq!(settings.display.present, crate::Present::Immediate);
        assert_eq!(settings.display.max_fps, 30, "VSync and explicit limiter are independent");
        assert!(!settings.set_control("display.vsync", &ControlValue::Number(0.0)));
        assert!(!settings.set_control("display.limit_fps", &ControlValue::Number(0.0)));
        assert!(!settings.set_control("display.fps_limit", &ControlValue::On(true)));
    }

    #[test]
    fn missing_pacing_fields_keep_vsync_and_no_explicit_cap() {
        for text in ["", "[display]\nshow_fps = true\n"] {
            let mut settings: Settings = toml::from_str(text).unwrap();
            assert_eq!((settings.display.present, settings.display.max_fps, settings.display.fps_limit), (crate::Present::Vsync, 0, 120));
            assert!(settings.set_control("display.limit_fps", &ControlValue::On(true)));
            assert_eq!(settings.display.max_fps, 120);
            let saved = toml::to_string(&settings).unwrap();
            assert_eq!(toml::from_str::<Settings>(&saved).unwrap(), settings);
        }
    }

    #[test]
    fn detailed_counter_enables_the_counter_and_round_trips() {
        let mut settings = Settings::default();
        assert!(!settings.display.fps_detailed);
        assert!(settings.set_control("display.fps_detailed", &ControlValue::On(true)));
        assert!(settings.display.show_fps && settings.display.fps_detailed);
        let encoded = toml::to_string(&settings).unwrap();
        let decoded: Settings = toml::from_str(&encoded).unwrap();
        assert!(decoded.display.show_fps && decoded.display.fps_detailed);
        assert!(settings.set_control("display.fps_detailed", &ControlValue::On(false)));
        assert!(settings.display.show_fps && !settings.display.fps_detailed);
        let old: Settings = toml::from_str("[display]\nshow_fps = true\n").unwrap();
        assert!(old.display.show_fps && !old.display.fps_detailed);
    }

    #[test]
    fn every_control_reads_back_what_was_set() {
        let mut s = Settings::default();
        for (key, value) in [
            ("resolution.pixel_scale", ControlValue::Choice("4".into())),
            ("resolution.scale_menus", ControlValue::On(true)),
            ("resolution.menu_scale_percent", ControlValue::Number(75.0)),
            ("resolution.frame_scale", ControlValue::Number(0.75)),
            ("resolution.world_scale", ControlValue::Number(2.0)),
            ("resolution.background_sampling", ControlValue::Choice("Interpolate".into())),
            ("layout.split", ControlValue::Choice("Vertical2To1".into())),
            ("layout.zoom_place", ControlValue::Choice("RightOrTop".into())),
            ("layout.zoom", ControlValue::Choice("TopStrip".into())),
            ("layout.zoom_strip", ControlValue::Number(0.35)),
            ("layout.zoom_box_top", ControlValue::Number(0.04)),
            ("layout.zoom_box_drop", ControlValue::Number(0.075)),
            ("layout.zoom_mouse_above", ControlValue::Number(0.25)),
            ("layout.zoom_mouse_below", ControlValue::Number(0.05)),
            ("layout.zoom_magnification_step", ControlValue::Number(3.0)),
            ("layout.zoom_box_block_pointer", ControlValue::On(true)),
            ("layout.menu_follows_pointer", ControlValue::On(false)),
            ("layout.smooth_world_movement", ControlValue::On(true)),
            ("layout.roles", ControlValue::Choice("UnitMouse".into())),
            ("layout.unit_buttons", ControlValue::Choice("ActiveView".into())),
            ("input.snap_updates_per_second", ControlValue::Number(7.0)),
            ("input.capture_sensitivity", ControlValue::Number(2.5)),
            ("display.show_fps", ControlValue::On(true)),
            ("input.map_movement", ControlValue::Choice("Drag".into())),
            ("input.smooth_menu_scrolling", ControlValue::On(true)),
            ("input.snap_lists_to_items", ControlValue::On(false)),
            ("input.alt_in_menus", ControlValue::On(false)),
            ("input.capture_function_keys", ControlValue::On(false)),
            ("input.media_keys_as_function_keys", ControlValue::On(true)),
            ("layout.screen_place", ControlValue::Choice("LeftOrBottom".into())),
        ] {
            assert!(s.set_control(key, &value), "{key}");
            assert_eq!(s.control(key), Some(value), "{key}");
        }
        assert_eq!(s.resolution.pixel_scale, 4);
        assert!(!s.set_control("resolution.pixel_scale", &ControlValue::Choice("auto".into())), "no Auto: ×1 is the default");
        assert_eq!(s.resolution.pixel_scale, 4);
        assert_eq!(s.layout.split, Split::Vertical2To1);
        assert_eq!(s.layout.zoom, Zoom::TopStrip);
        assert_eq!(s.layout.roles, Roles::UnitMouse);
        assert!(s.display.show_fps);
        assert_eq!(s.input.map_movement, MapMovement::Drag);
        assert_eq!(s.resolution.background_sampling, BackgroundSampling::Interpolate);
        assert_eq!(s.layout.screen_place, ScreenPlace { vertical: Place::LeftOrBottom, horizontal: Place::Floating });
        // Setting the same again changes nothing.
        assert!(!s.set_control("layout.split", &ControlValue::Choice("Vertical2To1".into())));
    }

    #[test]
    fn bad_values_are_refused_and_numbers_clamped_to_their_steps() {
        let mut s = Settings::default();
        assert!(!s.set_control("layout.split", &ControlValue::Choice("Diagonal".into())));
        assert!(!s.set_control("layout.split", &ControlValue::On(true)));
        assert!(!s.set_control("resolution.pixel_scale", &ControlValue::Choice("7".into())));
        assert!(!s.set_control("resolution.pixel_scale", &ControlValue::Choice("0".into())));
        assert!(!s.set_control("nonsense", &ControlValue::On(true)));
        assert!(!s.set_control("display.show_fps", &ControlValue::Number(1.0)));
        assert!(!s.set_control("display.show_fps", &ControlValue::On(false)));
        assert!(!s.set_control("input.map_movement", &ControlValue::Choice("Wobble".into())));
        assert!(!s.set_control("input.map_movement", &ControlValue::On(true)));
        assert!(!s.set_control("input.map_movement", &ControlValue::Choice("EdgeScroll".into())));
        assert!(!s.set_control("resolution.background_sampling", &ControlValue::Choice("Bilinear".into())));
        assert!(!s.set_control("resolution.background_sampling", &ControlValue::Choice("Dither".into())));
        assert!(!s.set_control("layout.screens.F1", &ControlValue::Choice("RightOrTop".into())));
        assert!(!s.set_control("layout.screen_place", &ControlValue::Choice("Sideways".into())));
        assert_eq!(s, Settings::default());
        assert!(s.set_control("layout.zoom_strip", &ControlValue::Number(0.9)));
        assert_eq!(s.layout.zoom_strip, 0.5);
        assert!(s.set_control("layout.zoom_strip", &ControlValue::Number(0.0)));
        assert_eq!(s.layout.zoom_strip, 0.05);
        assert!(s.set_control("resolution.menu_scale_percent", &ControlValue::Number(63.7)));
        assert_eq!(s.resolution.menu_scale_percent, 64.0);
        assert!(s.set_control("resolution.menu_scale_percent", &ControlValue::Number(0.0)));
        assert_eq!(s.resolution.menu_scale_percent, 25.0);
        assert!(!s.resolution.scale_menus);
        assert!(!s.set_control("resolution.menu_scale", &ControlValue::Number(1.5)));
        assert_eq!(s.control("resolution.menu_scale"), None);
        assert!(s.set_control("input.snap_updates_per_second", &ControlValue::Number(f32::NAN)));
        assert_eq!(s.input.snap_updates_per_second, 1.0);
        assert_eq!(SCALE_RANGE.snap(0.3), 0.5);
        assert_eq!(ZOOM_STRIP_RANGE.snap(0.3), 0.3);
    }

    #[test]
    fn cursor_size_defaults_to_the_system_and_takes_only_offered_sizes() {
        let mut s = Settings::default();
        assert_eq!(s.control("display.cursor_size"), Some(ControlValue::Choice("system".into())));
        assert_eq!(s.display.effective_cursor_size(48), 48);
        assert_eq!(s.display.effective_cursor_size(0), 8);
        for n in crate::Display::CURSOR_SIZES {
            assert!(s.set_control("display.cursor_size", &ControlValue::Choice(n.to_string())));
            assert_eq!(s.control("display.cursor_size"), Some(ControlValue::Choice(n.to_string())));
            assert_eq!(s.display.effective_cursor_size(24), n);
        }
        for bad in ["0", "33", "-1", "big"] {
            assert!(!s.set_control("display.cursor_size", &ControlValue::Choice(bad.into())), "{bad}");
        }
        assert!(!s.set_control("display.cursor_size", &ControlValue::Number(48.0)));
        assert!(s.set_control("display.cursor_size", &ControlValue::Choice("system".into())));
        assert_eq!(s, Settings::default());
        let saved: Settings = toml::from_str("[display]\ncursor_size = 64\n").unwrap();
        assert_eq!(saved.display.cursor_size, 64);
        assert_eq!(toml::from_str::<Settings>(&toml::to_string(&saved).unwrap()).unwrap(), saved);
    }

    #[test]
    fn base_ui_scale_snaps_without_changing_resolution_or_fine_scales() {
        let mut settings = Settings::default();
        settings.resolution.pixel_scale = 3;
        let before = settings.resolution.clone();
        assert!(settings.set_control("resolution.base_ui_scale", &ControlValue::Number(2.3)));
        assert_eq!(settings.control("resolution.base_ui_scale"), Some(ControlValue::Number(2.5)));
        assert_eq!(settings.resolution, Resolution { base_ui_scale: 2.5, ..before });
        assert!(settings.set_control("resolution.base_ui_scale", &ControlValue::Number(100.0)));
        assert_eq!(settings.resolution.base_ui_scale, Resolution::MAX_PIXEL_SCALE as f32);
    }

    #[test]
    fn capture_sensitivity_has_its_own_range_and_leaves_other_input_settings_alone() {
        let mut s = Settings::default();
        let before = s.input.clone();
        assert!(s.set_control("input.capture_sensitivity", &ControlValue::Number(2.53)));
        assert_eq!(s.input.capture_sensitivity, 2.5);
        assert!(s.set_control("input.capture_sensitivity", &ControlValue::Number(12.0)));
        assert_eq!(s.input.capture_sensitivity, 10.0);
        assert!(s.set_control("input.capture_sensitivity", &ControlValue::Number(-1.0)));
        assert_eq!(s.input.capture_sensitivity, 0.1);
        assert!(!s.set_control("input.capture_sensitivity", &ControlValue::Number(f32::NAN)));
        assert!(!s.set_control("input.capture_sensitivity", &ControlValue::On(true)));
        s.input.capture_sensitivity = before.capture_sensitivity;
        assert_eq!(s.input, before);
    }

    #[test]
    fn floating_boxes_have_no_own_height_but_a_top_margin_and_a_gap() {
        let mut s = Settings::default();
        assert!(!KEYS.contains(&"layout.zoom_box_height"));
        assert_eq!(s.control("layout.zoom_box_height"), None);
        assert!(!s.set_control("layout.zoom_box_height", &ControlValue::Number(0.6)));
        assert_eq!(s, Settings::default());
        assert!(s.set_control("layout.zoom_box_top", &ControlValue::Number(0.0124)));
        assert_eq!(s.layout.zoom_box_top, 0.01);
        assert!(s.set_control("layout.zoom_box_top", &ControlValue::Number(0.9)));
        assert_eq!(s.layout.zoom_box_top, 0.05);
        assert!(s.set_control("layout.zoom_box_drop", &ControlValue::Number(0.0624)));
        assert_eq!(s.layout.zoom_box_drop, 0.06);
        assert!(s.set_control("layout.zoom_box_drop", &ControlValue::Number(0.41)));
        assert_eq!(s.layout.zoom_box_drop, 0.15);
        assert!(s.set_control("layout.zoom_box_drop", &ControlValue::Number(-1.0)));
        assert_eq!(s.layout.zoom_box_drop, 0.05);
        assert!(!s.set_control("layout.zoom_box_drop", &ControlValue::Number(f32::NAN)));
        assert!(s.set_control("layout.zoom_mouse_above", &ControlValue::Number(0.9)));
        assert_eq!(s.layout.zoom_mouse_above, 0.3);
        assert!(s.set_control("layout.zoom_mouse_above", &ControlValue::Number(-1.0)));
        assert_eq!(s.layout.zoom_mouse_above, 0.0);
        assert!(s.set_control("layout.zoom_mouse_below", &ControlValue::Number(0.9)));
        assert_eq!(s.layout.zoom_mouse_below, 0.4);
        assert!(s.set_control("layout.zoom_mouse_below", &ControlValue::Number(0.1249)));
        assert_eq!(s.layout.zoom_mouse_below, 0.12);
        assert_eq!(s.layout.zoom_strip, 0.2);
    }

    #[test]
    fn the_zoom_magnification_is_chosen_by_step() {
        let mut s = Settings::default();
        assert_eq!(s.control("layout.zoom_magnification_step"), Some(ControlValue::Number(1.0)));
        for (step, magnification) in [(0.0, 1.5), (2.4, 3.0), (3.0, 4.0), (9.0, 4.0), (-1.0, 1.5)] {
            assert!(s.set_control("layout.zoom_magnification_step", &ControlValue::Number(step)) || s.layout.zoom_magnification == magnification, "{step}");
            assert_eq!(s.layout.zoom_magnification, magnification, "{step}");
        }
        assert!(!s.set_control("layout.zoom_magnification_step", &ControlValue::Number(f32::NAN)));
        assert_eq!(s.layout.zoom_magnification, 1.5);
    }

    #[test]
    fn screen_places_follow_the_orientation_and_keep_the_other() {
        let mut s = Settings::default();
        let set = |s: &mut Settings, v: &str| s.set_control("layout.screen_place", &ControlValue::Choice(v.into()));
        // Standard and vertical splits place for vertical.
        assert_eq!(s.control("layout.orientation"), Some(ControlValue::Choice("vertical".into())));
        assert!(set(&mut s, "RightOrTop"));
        s.layout.split = Split::Horizontal1To1;
        assert_eq!(s.control("layout.orientation"), Some(ControlValue::Choice("horizontal".into())));
        assert_eq!(s.control("layout.screen_place"), Some(ControlValue::Choice("Floating".into())));
        assert!(set(&mut s, "LeftOrBottom"));
        s.layout.split = Split::Vertical1To1;
        assert_eq!(s.control("layout.screen_place"), Some(ControlValue::Choice("RightOrTop".into())));
        assert_eq!(s.layout.screen_place, ScreenPlace { vertical: Place::RightOrTop, horizontal: Place::LeftOrBottom });
        // Every screen shares it.
        for kind in Layout::SCREENS {
            assert_eq!(s.layout.place_of(kind), Place::RightOrTop, "{kind}");
        }
    }

    #[test]
    fn controls_list_every_setting_and_screen() {
        let s = Settings::default();
        let controls = s.controls();
        assert_eq!(controls.len(), KEYS.len() + 1 + crate::clean_map::CleanMap::keys().count() + crate::accessibility::KEYS.len());
        for key in crate::clean_map::CleanMap::keys().chain(crate::accessibility::KEYS.into_iter().map(str::to_owned)) {
            assert_eq!(controls.iter().filter(|(k, _)| *k == key).count(), 1, "{key}");
        }
        assert!(controls.contains(&("layout.screen_place".into(), ControlValue::Choice("Floating".into()))));
        assert!(!controls.iter().any(|(k, _)| k.starts_with("layout.screens.")));
        assert!(controls.contains(&("resolution.pixel_scale".into(), ControlValue::Choice("1".into()))));
    }
}
