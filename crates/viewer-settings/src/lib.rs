//! The viewer's own settings: how its picture is scaled, how its views are
//! arranged and what they follow, and how the pointer moves between them.
//! The preferences screen edits them (its "Resolution" and "Layout" tabs);
//! the viewer and the driver read them. Saved as TOML.
//!
//! Game-agnostic: screens are named by the driver (`ScreenKind` holds
//! strings), not by any one game.
//!
//! [`controls`]: the settings as named controls (choices, switches,
//! numbers with their ranges), as a preferences screen shows and sets them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub mod controls;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0}: {1}")]
    Parse(PathBuf, toml::de::Error),
    #[error("{0}")]
    Write(#[from] toml::ser::Error),
}

/// Everything the user can set.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub resolution: Resolution,
    pub layout: Layout,
    pub input: Input,
}

/// The "Resolution" tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Resolution {
    /// Every rendered pixel shown as an n×n box (1..=6): the picture is
    /// rendered at the window size divided by n (rounded down) and scaled up
    /// by nearest pixel.
    pub pixel_scale: u32,
    /// Fractional scale of menus (full-screen and floating screens).
    pub menu_scale: f32,
    /// Fractional scale of the game's frame (the interface around the world:
    /// buttons, unit box, minimap).
    pub frame_scale: f32,
    /// Fractional scale of the world map.
    pub world_scale: f32,
}

impl Default for Resolution {
    fn default() -> Self {
        Self { pixel_scale: 1, menu_scale: 1.0, frame_scale: 1.0, world_scale: 1.0 }
    }
}

impl Resolution {
    /// Most pixels per rendered pixel.
    pub const MAX_PIXEL_SCALE: u32 = 6;

    /// The resolution actually rendered for a window of `window` pixels.
    pub fn realised(&self, (w, h): (u32, u32)) -> (u32, u32) {
        let n = self.pixel_scale.clamp(1, Self::MAX_PIXEL_SCALE);
        ((w / n).max(1), (h / n).max(1))
    }
}

/// How the window is divided into views A and B.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Split {
    /// One view.
    #[default]
    Standard,
    /// A left, B right, equal widths.
    Vertical1To1,
    /// A left twice as wide as B.
    Vertical2To1,
    /// A top, B bottom, equal heights.
    Horizontal1To1,
}

impl Split {
    pub fn is_vertical(self) -> bool {
        matches!(self, Split::Vertical1To1 | Split::Vertical2To1)
    }
}

/// Where something goes relative to a split.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Place {
    /// Floating over the split, centred.
    #[default]
    Floating,
    /// Left in a vertical split, bottom in a horizontal one.
    LeftOrBottom,
    /// Right in a vertical split, top in a horizontal one.
    RightOrTop,
}

/// The zoom (magnifier) box.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Zoom {
    #[default]
    Off,
    /// A box on top, its size `1:3:1` of the window (margin : box : margin).
    Floating131,
    /// As `Floating131`, `1:7:1`.
    Floating171,
    /// A box hovering about the mouse, `1:1:1`.
    Mouse111,
    /// A box hovering about the mouse, `1:3:1`.
    Mouse131,
    /// A strip across the top (a horizontal split), its height
    /// [`Layout::zoom_strip`] of the window.
    TopStrip,
}

/// What views A and B follow.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Roles {
    /// Both views are moved by hand (edge scrolling, keys).
    #[default]
    Free,
    /// A follows the focus of interest (the active unit, the current popup's
    /// subject; popups are placed fully inside it if they fit), B captures
    /// the mouse and follows it.
    UnitMouse,
}

/// A screen of the game that can be placed (named by the driver, e.g.
/// "F1".."F8", "City", "Palace", "Spy", "Menu", "Help").
pub type ScreenKind = String;

/// Where a screen goes, kept for each orientation separately (switching
/// the split keeps both).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenPlace {
    pub vertical: Place,
    pub horizontal: Place,
}

impl ScreenPlace {
    /// The place for `split`.
    pub fn for_split(&self, split: Split) -> Place {
        if split == Split::Horizontal1To1 { self.horizontal } else { self.vertical }
    }
}

/// The "Layout" tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub split: Split,
    /// Where the zoom box goes in a vertical split (else unused).
    pub zoom_place: Place,
    pub zoom: Zoom,
    /// Height of [`Zoom::TopStrip`] as a fraction of the window, 0.1..=0.5.
    pub zoom_strip: f32,
    /// Where each screen goes.
    pub screens: BTreeMap<ScreenKind, ScreenPlace>,
    /// The world moves smoothly (by pixels, easing) instead of snapping to
    /// tiles.
    pub smooth_world_movement: bool,
    pub roles: Roles,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            split: Split::Standard,
            zoom_place: Place::Floating,
            zoom: Zoom::Off,
            zoom_strip: 0.25,
            screens: BTreeMap::new(),
            smooth_world_movement: false,
            roles: Roles::Free,
        }
    }
}

impl Layout {
    /// The screens the layout tab lists, in its order.
    pub const SCREENS: [&'static str; 13] = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "City", "Palace", "Spy", "Menu", "Help"];

    /// Where screen `kind` goes now.
    pub fn place_of(&self, kind: &str) -> Place {
        self.screens.get(kind).map_or(Place::Floating, |p| p.for_split(self.split))
    }
}

/// Pointer behaviour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Input {
    /// Button snapping (hold Alt): how often the snap target is recomputed
    /// from the pointer's momentum, per second.
    pub snap_updates_per_second: f32,
    /// How often the translucent target cursor follows, per second.
    pub snap_ghost_fps: f32,
}

impl Default for Input {
    fn default() -> Self {
        Self { snap_updates_per_second: 3.0, snap_ghost_fps: 30.0 }
    }
}

impl Settings {
    /// Where settings are kept: `$CIV3R_SETTINGS`, else
    /// `$XDG_CONFIG_HOME/civ3r/settings.toml`, else `~/.config/civ3r/settings.toml`
    /// (`%APPDATA%\civ3r\settings.toml` on Windows).
    pub fn default_path() -> PathBuf {
        if let Some(p) = std::env::var_os("CIV3R_SETTINGS") {
            return p.into();
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("civ3r").join("settings.toml")
    }

    /// The settings at `path`; defaults if there is no file.
    pub fn load(path: &Path) -> Result<Settings, SettingsError> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| SettingsError::Parse(path.into(), e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(SettingsError::Io(path.into(), e)),
        }
    }

    /// Writes the settings to `path` (its folder made if missing), whole or
    /// not at all: into a file beside it, then moved over it, so that a
    /// reader never sees half a file.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let text = toml::to_string_pretty(self)?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| SettingsError::Io(dir.into(), e))?;
        }
        let mut temp = path.as_os_str().to_owned();
        temp.push(".new");
        let temp = PathBuf::from(temp);
        std::fs::write(&temp, text).map_err(|e| SettingsError::Io(temp.clone(), e))?;
        std::fs::rename(&temp, path).map_err(|e| SettingsError::Io(path.into(), e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_fills_defaults() {
        let mut s = Settings::default();
        s.resolution.pixel_scale = 3;
        s.layout.split = Split::Vertical2To1;
        s.layout.screens.insert("F1".into(), ScreenPlace { vertical: Place::RightOrTop, horizontal: Place::LeftOrBottom });
        let back: Settings = toml::from_str(&toml::to_string_pretty(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.layout.place_of("F1"), Place::RightOrTop);
        let partial: Settings = toml::from_str("[layout]\nsmooth_world_movement = true\n").unwrap();
        assert!(partial.layout.smooth_world_movement);
        assert_eq!(partial.resolution, Resolution::default());
    }

    #[test]
    fn saved_settings_load_back_and_a_missing_file_gives_defaults() {
        let dir = std::env::temp_dir().join(format!("viewer-settings-test-{}", std::process::id()));
        let path = dir.join("sub").join("settings.toml");
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
        let mut s = Settings::default();
        s.layout.roles = Roles::UnitMouse;
        s.input.snap_updates_per_second = 5.0;
        s.save(&path).unwrap();
        s.layout.zoom = Zoom::Mouse131;
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
        assert!(!dir.join("sub").join("settings.toml.new").exists());
        std::fs::write(&path, "[layout]\nsplit = 7\n").unwrap();
        assert!(matches!(Settings::load(&path), Err(SettingsError::Parse(..))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn realised_resolution_divides_by_the_pixel_scale() {
        let r = Resolution { pixel_scale: 3, ..Default::default() };
        assert_eq!(r.realised((3840, 2160)), (1280, 720));
        assert_eq!(Resolution { pixel_scale: 9, ..Default::default() }.realised((100, 100)), (16, 16));
    }
}
