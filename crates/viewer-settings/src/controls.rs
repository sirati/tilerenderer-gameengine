//! The settings as named controls, for a preferences screen: every setting
//! the screen edits has a key (`"layout.split"`, `"resolution.menu_scale"`,
//! ...) and a value of one of three kinds — a choice among named options, a
//! switch, a number. A screen shows [`Settings::control`] and hands what the
//! user did to [`Settings::set_control`], which checks and clamps it; the
//! settings stay the one source of truth.
//!
//! Choices are named like the enum variants they stand for (`"Vertical2To1"`,
//! `"LeftOrBottom"`); the pixel scale by its number (`"1"` .. `"6"`).

use crate::{Layout, Place, Resolution, Roles, Settings, Split, Zoom};

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

/// The fractional scales (menus, the game's frame, the world map).
pub const SCALE_RANGE: Range = Range { min: 0.5, max: 3.0, step: 0.05 };
/// The height of the zoom strip ([`Layout::zoom_strip`]).
pub const ZOOM_STRIP_RANGE: Range = Range { min: 0.1, max: 0.5, step: 0.05 };
/// How often the snapping cursor's target is recomputed, per second.
pub const SNAP_RATE_RANGE: Range = Range { min: 1.0, max: 30.0, step: 1.0 };
/// How often the snapping cursor's ghost follows, per second.
pub const SNAP_GHOST_RANGE: Range = Range { min: 1.0, max: 120.0, step: 1.0 };

/// The prefix of the screen place controls: `"layout.screens.F1"`, ...
pub const SCREENS_PREFIX: &str = "layout.screens.";

/// Every key [`Settings::control`] answers (besides one screen place per
/// screen of [`Layout::SCREENS`], and the derived `"layout.orientation"`).
pub const KEYS: [&str; 13] = [
    "resolution.pixel_scale",
    "resolution.menu_scale",
    "resolution.frame_scale",
    "resolution.world_scale",
    "layout.split",
    "layout.zoom_place",
    "layout.zoom",
    "layout.zoom_strip",
    "layout.smooth_world_movement",
    "layout.roles",
    "input.snap_updates_per_second",
    "input.snap_ghost_fps",
    "display.show_fps",
];

/// The range of number control `key`, if it is one.
pub fn range(key: &str) -> Option<Range> {
    match key {
        "resolution.menu_scale" | "resolution.frame_scale" | "resolution.world_scale" => Some(SCALE_RANGE),
        "layout.zoom_strip" => Some(ZOOM_STRIP_RANGE),
        "input.snap_updates_per_second" => Some(SNAP_RATE_RANGE),
        "input.snap_ghost_fps" => Some(SNAP_GHOST_RANGE),
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
    ];
}

impl Named for Roles {
    const ALL: &'static [(Self, &'static str)] = &[(Roles::Free, "Free"), (Roles::UnitMouse, "UnitMouse")];
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
        match key {
            "resolution.pixel_scale" => {
                Some(ControlValue::Choice(r.pixel_scale.clamp(1, Resolution::MAX_PIXEL_SCALE).to_string()))
            }
            "resolution.menu_scale" => Some(ControlValue::Number(r.menu_scale)),
            "resolution.frame_scale" => Some(ControlValue::Number(r.frame_scale)),
            "resolution.world_scale" => Some(ControlValue::Number(r.world_scale)),
            "layout.split" => choice(l.split),
            "layout.zoom_place" => choice(l.zoom_place),
            "layout.zoom" => choice(l.zoom),
            "layout.zoom_strip" => Some(ControlValue::Number(l.zoom_strip)),
            "layout.smooth_world_movement" => Some(ControlValue::On(l.smooth_world_movement)),
            "layout.roles" => choice(l.roles),
            "layout.orientation" => Some(ControlValue::Choice(if horizontal(l) { "horizontal" } else { "vertical" }.into())),
            "input.snap_updates_per_second" => Some(ControlValue::Number(i.snap_updates_per_second)),
            "input.snap_ghost_fps" => Some(ControlValue::Number(i.snap_ghost_fps)),
            "display.show_fps" => Some(ControlValue::On(self.display.show_fps)),
            _ => {
                let kind = key.strip_prefix(SCREENS_PREFIX)?;
                Layout::SCREENS.contains(&kind).then(|| choice(l.place_of(kind))).flatten()
            }
        }
    }

    /// Every control and its value (see [`Settings::control`]).
    pub fn controls(&self) -> Vec<(String, ControlValue)> {
        let screens = Layout::SCREENS.iter().map(|s| format!("{SCREENS_PREFIX}{s}"));
        KEYS.iter()
            .map(|k| k.to_string())
            .chain(std::iter::once("layout.orientation".to_owned()))
            .chain(screens)
            .filter_map(|k| self.control(&k).map(|v| (k, v)))
            .collect()
    }

    /// Sets control `key` to `value` (numbers clamped to their range and
    /// snapped to its step); whether the settings changed. A screen's place
    /// is set for the split's orientation, the other kept.
    pub fn set_control(&mut self, key: &str, value: &ControlValue) -> bool {
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
                "resolution.menu_scale" => set_number(&mut r.menu_scale, key, value),
                "resolution.frame_scale" => set_number(&mut r.frame_scale, key, value),
                "resolution.world_scale" => set_number(&mut r.world_scale, key, value),
                "layout.split" => set_choice(&mut l.split, value),
                "layout.zoom_place" => set_choice(&mut l.zoom_place, value),
                "layout.zoom" => set_choice(&mut l.zoom, value),
                "layout.zoom_strip" => set_number(&mut l.zoom_strip, key, value),
                "layout.smooth_world_movement" => match value {
                    ControlValue::On(on) => {
                        l.smooth_world_movement = *on;
                        true
                    }
                    _ => false,
                },
                "layout.roles" => set_choice(&mut l.roles, value),
                "input.snap_updates_per_second" => set_number(&mut i.snap_updates_per_second, key, value),
                "input.snap_ghost_fps" => set_number(&mut i.snap_ghost_fps, key, value),
                "display.show_fps" => match value {
                    ControlValue::On(on) => {
                        d.show_fps = *on;
                        true
                    }
                    _ => false,
                },
                _ => match key.strip_prefix(SCREENS_PREFIX).filter(|k| Layout::SCREENS.contains(k)) {
                    Some(kind) => {
                        let horizontal = horizontal(l);
                        let mut place = l.place_of(kind);
                        if set_choice(&mut place, value) {
                            let slot = l.screens.entry(kind.to_owned()).or_default();
                            if horizontal {
                                slot.horizontal = place;
                            } else {
                                slot.vertical = place;
                            }
                            true
                        } else {
                            false
                        }
                    }
                    None => false,
                },
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
    fn every_control_reads_back_what_was_set() {
        let mut s = Settings::default();
        for (key, value) in [
            ("resolution.pixel_scale", ControlValue::Choice("4".into())),
            ("resolution.menu_scale", ControlValue::Number(1.25)),
            ("resolution.frame_scale", ControlValue::Number(0.75)),
            ("resolution.world_scale", ControlValue::Number(2.0)),
            ("layout.split", ControlValue::Choice("Vertical2To1".into())),
            ("layout.zoom_place", ControlValue::Choice("RightOrTop".into())),
            ("layout.zoom", ControlValue::Choice("TopStrip".into())),
            ("layout.zoom_strip", ControlValue::Number(0.35)),
            ("layout.smooth_world_movement", ControlValue::On(true)),
            ("layout.roles", ControlValue::Choice("UnitMouse".into())),
            ("input.snap_updates_per_second", ControlValue::Number(7.0)),
            ("display.show_fps", ControlValue::On(true)),
            ("layout.screens.City", ControlValue::Choice("LeftOrBottom".into())),
        ] {
            assert!(s.set_control(key, &value), "{key}");
            assert_eq!(s.control(key), Some(value), "{key}");
        }
        assert_eq!(s.resolution.pixel_scale, 4);
        assert_eq!(s.layout.split, Split::Vertical2To1);
        assert_eq!(s.layout.zoom, Zoom::TopStrip);
        assert_eq!(s.layout.roles, Roles::UnitMouse);
        assert!(s.display.show_fps);
        assert_eq!(s.layout.screens["City"], ScreenPlace { vertical: Place::LeftOrBottom, horizontal: Place::Floating });
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
        assert!(!s.set_control("layout.screens.F9", &ControlValue::Choice("Floating".into())));
        assert_eq!(s, Settings::default());
        assert!(s.set_control("layout.zoom_strip", &ControlValue::Number(0.9)));
        assert_eq!(s.layout.zoom_strip, 0.5);
        assert!(s.set_control("layout.zoom_strip", &ControlValue::Number(0.0)));
        assert_eq!(s.layout.zoom_strip, 0.1);
        assert!(s.set_control("resolution.menu_scale", &ControlValue::Number(1.333)));
        assert_eq!(s.resolution.menu_scale, 1.35);
        assert!(s.set_control("input.snap_updates_per_second", &ControlValue::Number(f32::NAN)));
        assert_eq!(s.input.snap_updates_per_second, 1.0);
        assert_eq!(SCALE_RANGE.snap(0.3), 0.5);
        assert_eq!(ZOOM_STRIP_RANGE.snap(0.3), 0.3);
    }

    #[test]
    fn screen_places_follow_the_orientation_and_keep_the_other() {
        let mut s = Settings::default();
        let set = |s: &mut Settings, v: &str| s.set_control("layout.screens.F3", &ControlValue::Choice(v.into()));
        // Standard and vertical splits place for vertical.
        assert_eq!(s.control("layout.orientation"), Some(ControlValue::Choice("vertical".into())));
        assert!(set(&mut s, "RightOrTop"));
        s.layout.split = Split::Horizontal1To1;
        assert_eq!(s.control("layout.orientation"), Some(ControlValue::Choice("horizontal".into())));
        assert_eq!(s.control("layout.screens.F3"), Some(ControlValue::Choice("Floating".into())));
        assert!(set(&mut s, "LeftOrBottom"));
        s.layout.split = Split::Vertical1To1;
        assert_eq!(s.control("layout.screens.F3"), Some(ControlValue::Choice("RightOrTop".into())));
        assert_eq!(s.layout.screens["F3"], ScreenPlace { vertical: Place::RightOrTop, horizontal: Place::LeftOrBottom });
    }

    #[test]
    fn controls_list_every_setting_and_screen() {
        let s = Settings::default();
        let controls = s.controls();
        assert_eq!(controls.len(), KEYS.len() + 1 + Layout::SCREENS.len());
        assert!(controls.contains(&("layout.screens.Help".into(), ControlValue::Choice("Floating".into()))));
        assert!(controls.contains(&("resolution.pixel_scale".into(), ControlValue::Choice("1".into()))));
    }
}
