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
    pub display: Display,
}

/// The "Resolution" tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Resolution {
    /// Every rendered pixel shown as an n×n box of window pixels (1..=6,
    /// whole numbers only): the viewer renders at the window size divided
    /// by n (rounded down; what is left is a border of fewer than n
    /// pixels), never scaled by a fraction or filtered. [`Resolution::AUTO`]
    /// (0, the default): n is the desktop's scale rounded down (2 at 200 %,
    /// 1 at 150 %).
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
        Self { pixel_scale: Self::AUTO, menu_scale: 1.0, frame_scale: 1.0, world_scale: 1.0 }
    }
}

impl Resolution {
    /// Most pixels per rendered pixel.
    pub const MAX_PIXEL_SCALE: u32 = 6;
    /// `pixel_scale`: the desktop's scale.
    pub const AUTO: u32 = 0;

    /// Window pixels per rendered pixel on a desktop scaled `desktop` times
    /// (e.g. 2.0 at 200 %; the operating system's scale for the window):
    /// the pixel scale, or for [`Resolution::AUTO`] the desktop's rounded
    /// down (between 1 and [`Resolution::MAX_PIXEL_SCALE`]).
    pub fn effective_pixel_scale(&self, desktop: f64) -> u32 {
        match self.pixel_scale {
            Self::AUTO => if desktop.is_finite() { (desktop.floor() as u32).clamp(1, Self::MAX_PIXEL_SCALE) } else { 1 },
            n => n.min(Self::MAX_PIXEL_SCALE),
        }
    }

    /// The smallest resolution a pixel scale renders at, where the window
    /// allows: in a smaller window the pixel scale is lowered (to 1 at
    /// least) rather than leave too few pixels to show anything.
    pub const MIN_RENDERED: (u32, u32) = (320, 240);

    /// The pixel scale in a window of `window` pixels: the effective one
    /// ([`Resolution::effective_pixel_scale`]), lowered as far as needed
    /// to render at least [`Resolution::MIN_RENDERED`] (or the window).
    pub fn pixel_scale_in(&self, (w, h): (u32, u32), desktop: f64) -> u32 {
        let most = (w / Self::MIN_RENDERED.0).min(h / Self::MIN_RENDERED.1).max(1);
        self.effective_pixel_scale(desktop).min(most)
    }

    /// The resolution actually rendered for a window of `window` pixels on
    /// a desktop scaled `desktop` times ([`Resolution::pixel_scale_in`]):
    /// rounded down, at least 1.
    pub fn realised(&self, (w, h): (u32, u32), desktop: f64) -> (u32, u32) {
        let n = self.pixel_scale_in((w, h), desktop);
        ((w / n).max(1), (h / n).max(1))
    }

    /// The pixel scale in words, e.g. "×3", or "Auto (×2)" (`desktop` as
    /// for [`Resolution::realised`]).
    pub fn describe_pixel_scale(&self, desktop: f64) -> String {
        let n = self.effective_pixel_scale(desktop);
        if self.pixel_scale == Self::AUTO { format!("Auto (×{n})") } else { format!("×{n}") }
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
    /// A box on top, `1:3:1` of the width (margin : box : margin), its height
    /// from that at the window's aspect ratio. Not sized by `zoom_strip`.
    Floating131,
    /// As `Floating131`, `1:7:1`.
    Floating171,
    /// A box at the mouse: above it (below where there is no room), centred
    /// on it across; `1:1:1` of the width, [`Layout::zoom_strip`] of the
    /// height.
    Mouse111,
    /// As `Mouse111`, `1:3:1` of the width.
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
    /// Height of [`Zoom::TopStrip`] and of the boxes at the mouse
    /// ([`Zoom::Mouse111`], [`Zoom::Mouse131`]) as a fraction of the window
    /// (or of the view they are in), 0.05..=0.5.
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
    /// Button snapping (the Alt mode): how often the mouse's motion picks
    /// the next element, per second (a move lasts 1.33 of these intervals).
    pub snap_updates_per_second: f32,
    /// How often the translucent target cursor follows, per second.
    pub snap_ghost_fps: f32,
}

impl Default for Input {
    fn default() -> Self {
        Self { snap_updates_per_second: 3.0, snap_ghost_fps: 30.0 }
    }
}

/// What the viewer shows besides the picture (on the "Resolution" tab).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    /// A frame counter in the window's top-right corner: frames per second,
    /// the frame time, and with a software renderer the share of frames it
    /// skipped (unchanged).
    pub show_fps: bool,
    /// How frames reach the screen (`TILE_VIEWER_PRESENT` overrides it).
    pub present: Present,
    /// The most frames per second the viewer draws; 0: as many as the
    /// display shows (its refresh rate) with `Vsync`, with the others as
    /// many as it can (`TILE_VIEWER_MAX_FPS` overrides it).
    pub max_fps: u32,
}

/// How the viewer's frames reach the screen.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Present {
    /// Each frame waits for the display's refresh (no tearing; at most its
    /// rate, e.g. 60 or 120 frames per second).
    #[default]
    Vsync,
    /// Frames as fast as they are drawn, the newest shown at each refresh
    /// (no tearing; where the system has it, else `Vsync`).
    Mailbox,
    /// Frames as fast as they are drawn, shown at once (may tear; where the
    /// system has it, else `Mailbox`, else `Vsync`).
    Immediate,
}

impl Present {
    /// From `TILE_VIEWER_PRESENT`-style names (`vsync`/`fifo`, `mailbox`,
    /// `immediate`), any case.
    pub fn parse(name: &str) -> Option<Present> {
        match name.to_ascii_lowercase().as_str() {
            "vsync" | "fifo" => Some(Present::Vsync),
            "mailbox" => Some(Present::Mailbox),
            "immediate" => Some(Present::Immediate),
            _ => None,
        }
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
        assert!(!partial.display.show_fps);
    }

    #[test]
    fn the_frame_counter_is_off_by_default_and_older_files_still_load() {
        assert!(!Settings::default().display.show_fps);
        // A file from before the setting existed.
        let old: Settings = toml::from_str("[resolution]\npixel_scale = 2\n\n[layout]\nsplit = \"Vertical1To1\"\n").unwrap();
        assert_eq!(old.display, Display::default());
        assert_eq!((old.resolution.pixel_scale, old.layout.split), (2, Split::Vertical1To1));
        let on: Settings = toml::from_str("[display]\nshow_fps = true\n").unwrap();
        assert!(on.display.show_fps);
        assert_eq!((on.display.present, on.display.max_fps), (Present::Vsync, 0));
        let fast: Settings = toml::from_str("[display]\npresent = \"Mailbox\"\nmax_fps = 240\n").unwrap();
        assert_eq!((fast.display.present, fast.display.max_fps), (Present::Mailbox, 240));
        assert_eq!(Present::parse("IMMEDIATE"), Some(Present::Immediate));
        assert_eq!(on.layout, Layout::default());
        let text = toml::to_string_pretty(&on).unwrap();
        assert!(text.contains("[display]") && text.contains("show_fps = true"), "{text}");
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), on);
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
        assert_eq!(r.realised((3840, 2160), 2.0), (1280, 720));
        assert_eq!(r.describe_pixel_scale(2.0), "×3");
        assert_eq!(Resolution { pixel_scale: 9, ..Default::default() }.realised((3840, 2160), 1.0), (640, 360));
        // A small window: the pixel scale lowered to keep 320×240 at least.
        assert_eq!(r.realised((800, 600), 1.0), (400, 300));
        assert_eq!(r.realised((100, 100), 1.0), (100, 100));
        assert_eq!(r.realised((0, 0), 1.0), (1, 1));
        // Auto, the default: the desktop's scale, fractional ones too.
        let auto = Resolution::default();
        assert_eq!(auto.realised((3840, 2160), 2.0), (1920, 1080));
        assert_eq!(auto.describe_pixel_scale(2.0), "Auto (×2)");
        // At 150 %: rounded down, whole boxes only.
        assert_eq!(auto.realised((3840, 2160), 1.5), (3840, 2160));
        assert_eq!(auto.describe_pixel_scale(1.5), "Auto (×1)");
        assert_eq!(auto.realised((3840, 2160), 2.5), (1920, 1080));
        assert_eq!(auto.realised((1920, 1080), 1.0), (1920, 1080));
        assert_eq!(auto.realised((1920, 1080), f64::NAN), (1920, 1080));
    }

    #[test]
    fn a_saved_pixel_scale_stays_what_it_was() {
        // Files from before Auto: their number; without one: Auto.
        let one: Settings = toml::from_str("[resolution]\npixel_scale = 1\n").unwrap();
        assert_eq!(one.resolution.pixel_scale, 1);
        let none: Settings = toml::from_str("[resolution]\nmenu_scale = 1.5\n").unwrap();
        assert_eq!(none.resolution.pixel_scale, Resolution::AUTO);
    }
}
