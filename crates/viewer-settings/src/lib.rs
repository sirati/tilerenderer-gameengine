//! The viewer's own settings: how its picture is scaled, how its views are
//! arranged and what they follow, and how the pointer moves between them.
//! The preferences screen edits them (its "Resolution", "Layout" and "Zoom" tabs);
//! the viewer and the driver read them. Saved as TOML.
//!
//! Game-agnostic: screens are named by the driver (`ScreenKind` holds
//! strings), not by any one game.
//!
//! [`controls`]: the settings as named controls (choices, switches,
//! numbers with their ranges), as a preferences screen shows and sets them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub mod controls;
pub mod clean_map;
pub mod accessibility;

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
    #[serde(deserialize_with = "Layout::migrating")]
    pub layout: Layout,
    pub input: Input,
    pub animation: Animation,
    pub display: Display,
    pub clean_map: clean_map::CleanMap,
    pub accessibility: accessibility::Accessibility,
    /// What is shown once ever. A file without it is an existing install's:
    /// that has seen it all (only no file at all is a first run).
    #[serde(default = "FirstRun::done")]
    pub first_run: FirstRun,
}

/// Things shown once, the first time the game runs with the renderer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FirstRun {
    /// Preferences have been opened on the Resolution tab by themselves
    /// (once the title menu is up), so the user picks the pixel scale
    /// and sizes. Set when they open, whatever the user does then.
    pub preferences_shown: bool,
}

impl FirstRun {
    /// An install from before this marker, or one whose first run is over.
    pub fn done() -> Self {
        FirstRun { preferences_shown: true }
    }
}

/// Visual combat timing, independent of simulation and audio clocks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Animation {
    #[serde(deserialize_with = "combat_speed_setting")]
    pub combat_speed: f32,
}
fn combat_speed_setting<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    let combat_speed = f32::deserialize(deserializer)?;
    Ok(Animation { combat_speed }.effective_combat_speed())
}
impl Default for Animation { fn default() -> Self { Self { combat_speed: 1.0 } } }
impl Animation {
    pub fn effective_combat_speed(&self) -> f32 {
        if !self.combat_speed.is_finite() { return 1.0; }
        (self.combat_speed.clamp(1.0, 4.0) * 4.0).round() / 4.0
    }
}

/// The "Resolution" tab.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Resolution {
    /// Every rendered pixel shown as an n×n box of window pixels (1..=6,
    /// whole numbers only): the viewer renders at the window size divided
    /// by n (rounded down; what is left is a border of fewer than n
    /// pixels), never scaled by a fraction or filtered. 1, the default,
    /// renders every window pixel, on a high-density (Retina) display too;
    /// the menu, frame and world scales size things there. (0, "Auto" in
    /// older files, loads as 1.)
    pub pixel_scale: u32,
    /// Size in window pixels, independent of render resolution (half steps, 1..=6).
    /// Native menus, frame and world use base / pixel scale on the canvas.
    pub base_ui_scale: f32,
    /// Size menus relative to the window. When false menus use their
    /// native size times the base; title and Preferences still fit.
    pub scale_menus: bool,
    /// Standard menu height as a percentage of the window height, 25..=100.
    /// Only used when `scale_menus` is true.
    pub menu_scale_percent: f32,
    /// Fractional scale relative to the base for the game's frame
    /// (buttons, unit box, minimap).
    pub frame_scale: f32,
    /// Fractional scale of the world map relative to the base.
    pub world_scale: f32,
    /// How menu background art is sampled when a menu is drawn larger or
    /// smaller than its art, including background frames and tiles.
    /// Text, controls and world art keep their existing sharp rendering.
    pub background_sampling: BackgroundSampling,
    /// Transient, never saved: the base UI scale menus keep while
    /// Preferences previews another base behind its open dialog. `None`:
    /// menus use `base_ui_scale` like the frame and world.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub menu_base_ui_scale: Option<f32>,
}

/// [`Resolution::background_sampling`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackgroundSampling {
    /// One source pixel picked by a fixed spatial pattern, from the pixel
    /// footprint when shrinking or weighted neighbours when magnifying.
    /// Keeps the art's grain without blur.
    #[default]
    Dither,
    /// Area averaging when shrinking and bilinear colour when magnifying.
    /// Only background colour is smoothed; alpha masks stay sharp.
    Interpolate,
    /// The nearest source pixel, as everything else (may alias).
    Nearest,
}

impl Default for Resolution {
    fn default() -> Self {
        Self { pixel_scale: 1, base_ui_scale: 1.0, scale_menus: false, menu_scale_percent: 100.0, frame_scale: 1.0, world_scale: 1.0, background_sampling: BackgroundSampling::Dither, menu_base_ui_scale: None }
    }
}

impl<'de> Deserialize<'de> for Resolution {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // The old menu_scale was an arbitrary native-pixel multiplier. It
        // has no screen-relative meaning: consume it without turning 1.5
        // into 150% or 0.5 into 50%. Old files start with scaling off.
        #[derive(Deserialize)]
        #[serde(default)]
        struct Saved {
            pixel_scale: u32,
            base_ui_scale: Option<f32>,
            scale_menus: bool,
            menu_scale_percent: f32,
            menu_scale: Option<f32>,
            frame_scale: f32,
            world_scale: f32,
            background_sampling: BackgroundSampling,
            menu_base_ui_scale: Option<f32>,
        }
        impl Default for Saved {
            fn default() -> Self {
                let r = Resolution::default();
                Self { pixel_scale: r.pixel_scale, base_ui_scale: None, scale_menus: r.scale_menus, menu_scale_percent: r.menu_scale_percent,
                    menu_scale: None, frame_scale: r.frame_scale, world_scale: r.world_scale, background_sampling: r.background_sampling, menu_base_ui_scale: None }
            }
        }
        let saved = Saved::deserialize(deserializer)?;
        let _legacy_multiplier = saved.menu_scale;
        // (0 was "Auto", the desktop's scale: now ×1 everywhere.)
        let pixel_scale = saved.pixel_scale.clamp(1, Self::MAX_PIXEL_SCALE);
        // Old files sized native art with the pixel scale. Keep that size.
        let mut resolution = Self { pixel_scale, base_ui_scale: saved.base_ui_scale.unwrap_or(pixel_scale as f32), scale_menus: saved.scale_menus,
            menu_scale_percent: saved.menu_scale_percent, frame_scale: saved.frame_scale, world_scale: saved.world_scale,
            background_sampling: saved.background_sampling, menu_base_ui_scale: saved.menu_base_ui_scale };
        resolution.base_ui_scale = resolution.effective_base_ui_scale();
        resolution.menu_scale_percent = resolution.effective_menu_scale_percent();
        Ok(resolution)
    }
}

impl Resolution {
    /// Finite half steps, also for settings made outside Preferences.
    pub fn effective_base_ui_scale(&self) -> f32 {
        if self.base_ui_scale.is_finite() {
            (self.base_ui_scale.clamp(1.0, Self::MAX_PIXEL_SCALE as f32) * 2.0).round() / 2.0
        } else { 1.0 }
    }

    /// The base UI scale menus use: [`Self::menu_base_ui_scale`] while a
    /// preview holds them, else the base itself, in finite half steps.
    pub fn effective_menu_base_ui_scale(&self) -> f32 {
        Self { base_ui_scale: self.menu_base_ui_scale.unwrap_or(self.base_ui_scale), ..Self::default() }.effective_base_ui_scale()
    }

    /// A finite screen-relative menu percentage, including settings made
    /// programmatically rather than through a preferences control. Whole
    /// percents, as the control's steps: a hand-edited 60.758 is 61.
    pub fn effective_menu_scale_percent(&self) -> f32 {
        if self.menu_scale_percent.is_finite() { self.menu_scale_percent.clamp(25.0, 100.0).round() } else { 100.0 }
    }

    /// Most pixels per rendered pixel.
    pub const MAX_PIXEL_SCALE: u32 = 6;

    /// Window pixels per rendered pixel: the pixel scale, between 1 and
    /// [`Resolution::MAX_PIXEL_SCALE`], whatever the desktop's scale.
    pub fn effective_pixel_scale(&self) -> u32 {
        self.pixel_scale.clamp(1, Self::MAX_PIXEL_SCALE)
    }

    /// The smallest resolution a pixel scale renders at, where the window
    /// allows: in a smaller window the pixel scale is lowered (to 1 at
    /// least) rather than leave too few pixels to show anything.
    pub const MIN_RENDERED: (u32, u32) = (320, 240);

    /// The pixel scale in a window of `window` pixels: the effective one
    /// ([`Resolution::effective_pixel_scale`]), lowered as far as needed
    /// to render at least [`Resolution::MIN_RENDERED`] (or the window).
    pub fn pixel_scale_in(&self, (w, h): (u32, u32)) -> u32 {
        let most = (w / Self::MIN_RENDERED.0).min(h / Self::MIN_RENDERED.1).max(1);
        self.effective_pixel_scale().min(most)
    }

    /// The resolution actually rendered for a window of `window` physical
    /// pixels ([`Resolution::pixel_scale_in`]): rounded down, at least 1.
    pub fn realised(&self, (w, h): (u32, u32)) -> (u32, u32) {
        let n = self.pixel_scale_in((w, h));
        ((w / n).max(1), (h / n).max(1))
    }

    /// The pixel scale in words, e.g. "×3".
    pub fn describe_pixel_scale(&self) -> String {
        format!("×{}", self.effective_pixel_scale())
    }

    /// The first run's sizes on a desktop scaled `desktop` times (2.0 at
    /// 200 %) in a window `height` physical pixels tall: at ×1 (every
    /// window pixel rendered), things as large as they were when the pixel
    /// scale followed the desktop (n, its scale rounded down): the frame
    /// and the world n times, menus scaled to the share of the window a
    /// standard 768-row menu took at n. Only the sliders' starting values;
    /// nothing changes on a desktop at 100 %.
    pub fn seed_for_desktop(&mut self, desktop: f64, height: u32) {
        let n = if desktop.is_finite() { desktop.floor().clamp(1.0, 3.0) } else { 1.0 };
        if n <= 1.0 {
            return;
        }
        (self.frame_scale, self.world_scale) = (n as f32, n as f32);
        self.scale_menus = true;
        self.menu_scale_percent = (768.0 * n * 100.0 / f64::from(height.max(1))) as f32;
        self.menu_scale_percent = self.effective_menu_scale_percent();
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
    /// A box at the top of its region, centred across, `1:3:1` of the
    /// width, [`Layout::zoom_strip`] of the height, its top
    /// [`Layout::zoom_box_top`] below the region's top. Once the pointer
    /// comes within 5% of the height of where it rests, it moves (gliding)
    /// below that place, its top [`Layout::zoom_box_drop`] below that
    /// place's bottom (or holds the pointer out,
    /// [`Layout::zoom_box_block_pointer`]).
    Floating131,
    /// As `Floating131`, `1:7:1`.
    Floating171,
    /// A box at the mouse: [`Layout::zoom_mouse_above`] above it (else
    /// [`Layout::zoom_mouse_below`] below it, where there is no room above),
    /// centred on it across; `1:1:1` of the width, [`Layout::zoom_strip`]
    /// of the height.
    Mouse111,
    /// As `Mouse111`, `1:3:1` of the width.
    Mouse131,
    /// A strip across the top (a horizontal split), its height
    /// [`Layout::zoom_strip`] of the window.
    TopStrip,
    /// A box beside the mouse: right of the part of the window it
    /// enlarges (else left of it), centred on the mouse's height,
    /// [`Layout::zoom_strip`] of the window high and half as wide again.
    /// Shows over every screen, not only over the map. Older settings'
    /// `menu_zoom` (the "Menu zoom beside pointer" switch) load as this,
    /// showing [`ZoomContent::WorldAndMenus`].
    Beside,
}

/// What the zoom shows ([`Layout::zoom_content`]), wherever it goes.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoomContent {
    /// The map alone, drawn again at the zoom's magnification (art at its
    /// full resolution).
    #[default]
    World,
    /// The finished picture under the mouse, menus and the game's
    /// interface included, its pixels enlarged (no finer detail).
    WorldAndMenus,
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

/// Where the active unit's order buttons go while there are two views
/// (one view: where the game puts them).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitButtons {
    /// Where the game puts them, across both views at the bottom, split at
    /// the divider with a gap there for the pointer to cross.
    #[default]
    Spanning,
    /// Inside the view the pointer is in (its area below a zoom strip),
    /// moving with the pointer to the other view.
    ActiveView,
}

/// A screen of the driver's that can be placed (named by the driver; the
/// viewer does not read the name).
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

/// Arrangement and zoom settings, edited on the Layout and Zoom tabs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub split: Split,
    /// Divider width in physical window pixels, clamped to 1..=16.
    pub separator_width: u32,
    /// Round every zoom box (floating, at the mouse, beside it; world or
    /// menus). The top strip stays square.
    pub zoom_rounded: bool,
    /// Shade every zoom box's rim as polished glass (the strip has no rim).
    pub zoom_glass: bool,
    /// Draw every divider (the split's, the zoom strip's) as a bar of the
    /// zoom's glass, with round ends where it stops. It is then at least
    /// 16 window pixels thick (the pointer's wall with it).
    pub glass_dividers: bool,
    /// Where the zoom box goes in a vertical split (else unused).
    pub zoom_place: Place,
    pub zoom: Zoom,
    /// What the zoom shows: the map, or the map with menus.
    pub zoom_content: ZoomContent,
    /// Height of every zoom ([`Zoom::TopStrip`], the floating boxes and the
    /// boxes at the mouse) as a fraction of the window (or of the view it
    /// is in), 0.05..=0.5, default 0.2.
    pub zoom_strip: f32,
    /// The floating boxes ([`Zoom::Floating131`], [`Zoom::Floating171`]):
    /// the space between their region's top and their top, as a fraction
    /// of the region's height, 0..=0.05, default 0.025.
    pub zoom_box_top: f32,
    /// The floating boxes, moved out of the pointer's way (while it is
    /// where they rest): the gap between where they rest and their top, as
    /// a fraction of the region's height, 0.05..=0.15, default 0.05. Saved as
    /// `zoom_box_gap`: the older `zoom_box_drop` (how far below the
    /// pointer they dropped) meant something else and is not read.
    #[serde(rename = "zoom_box_gap")]
    pub zoom_box_drop: f32,
    /// The boxes at the mouse ([`Zoom::Mouse111`], [`Zoom::Mouse131`]): the
    /// space between the pointer and the bottom of the box above it, as a
    /// fraction of its region's height, 0..=0.3, default 0.1.
    pub zoom_mouse_above: f32,
    /// The boxes at the mouse, below the pointer where there is no room
    /// above it: the space between the pointer and the box's top, as a
    /// fraction of its region's height, 0..=0.4, default 0.2.
    pub zoom_mouse_below: f32,
    /// How much the zoom box or strip enlarges the world beyond how large
    /// the view under it shows it (world scale and view zoom): one of
    /// [`Layout::MAGNIFICATIONS`] (another value counts as the nearest),
    /// default 2.
    pub zoom_magnification: f32,
    /// Unused: the floating boxes' own height (or Auto) in older settings
    /// files. Read so they still load, never written;
    /// [`Layout::zoom_strip`] sizes every zoom now.
    #[serde(skip_serializing)]
    pub zoom_box_height: f32,
    /// Hold the pointer outside floating zoom boxes instead of moving the
    /// box down out of its way. Top strips always hold the pointer below
    /// them.
    pub zoom_box_block_pointer: bool,
    /// With a floating [`Layout::screen_place`]: open the generic menu and
    /// popups in the view that held the mouse when they opened.
    pub menu_follows_pointer: bool,
    /// Where every screen, advisor, dialog and popup goes (one place per
    /// split orientation). Context menus and the tile information box open
    /// at their click instead.
    pub screen_place: ScreenPlace,
    /// The world moves smoothly (by pixels, easing) instead of snapping to
    /// tiles.
    pub smooth_world_movement: bool,
    pub roles: Roles,
    /// Where the unit's order buttons go with two views.
    pub unit_buttons: UnitButtons,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            split: Split::Standard,
            separator_width: 4,
            zoom_rounded: true,
            zoom_glass: true,
            glass_dividers: false,
            zoom_place: Place::Floating,
            zoom: Zoom::Off,
            zoom_content: ZoomContent::World,
            zoom_strip: 0.2,
            zoom_box_top: 0.025,
            zoom_box_drop: 0.05,
            zoom_mouse_above: 0.1,
            zoom_mouse_below: 0.2,
            zoom_magnification: 2.0,
            zoom_box_height: 0.0,
            zoom_box_block_pointer: false,
            menu_follows_pointer: true,
            screen_place: ScreenPlace::default(),
            smooth_world_movement: false,
            roles: Roles::Free,
            unit_buttons: UnitButtons::Spanning,
        }
    }
}

impl Layout {
    /// The zoom's magnifications to choose from ([`Layout::zoom_magnification`]).
    pub const MAGNIFICATIONS: [f32; 4] = [1.5, 2.0, 3.0, 4.0];

    /// The index into [`Layout::MAGNIFICATIONS`] of the zoom's magnification
    /// (the nearest one; the default for a value that is not a number).
    pub fn zoom_magnification_step(&self) -> usize {
        let m = if self.zoom_magnification.is_finite() { self.zoom_magnification } else { 2.0 };
        (0..Self::MAGNIFICATIONS.len()).min_by(|&a, &b| (Self::MAGNIFICATIONS[a] - m).abs().total_cmp(&(Self::MAGNIFICATIONS[b] - m).abs())).unwrap_or(1)
    }

    /// The screens older settings files placed one by one (written by the
    /// renderer's older versions under the screen names of the game they
    /// drove: kept only to read
    /// those files as they were), in the order
    /// their places are migrated ([`Layout::migrated_place`]).
    pub const SCREENS: [&'static str; 13] = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "City", "Palace", "Spy", "Menu", "Help"];

    /// Where screen `kind` goes now: every kind shares [`Layout::screen_place`].
    pub fn place_of(&self, _kind: &str) -> Place {
        self.screen_place.for_split(self.split)
    }

    /// The single place for older per-screen places: per orientation the
    /// most common one, ties going to the first in [`Layout::SCREENS`]
    /// order (other names after them). Entries that do not parse are left
    /// out.
    pub fn migrated_place(old: &toml::Table) -> ScreenPlace {
        let mut names: Vec<&String> = old.keys().collect();
        let rank = |n: &str| Layout::SCREENS.iter().position(|s| *s == n).unwrap_or(Layout::SCREENS.len());
        names.sort_by_key(|n| rank(n));
        let places: Vec<ScreenPlace> = names.iter().filter_map(|n| old[n.as_str()].clone().try_into().ok()).collect();
        let common = |of: fn(&ScreenPlace) -> Place| {
            let mut best: Option<(Place, usize)> = None;
            for p in places.iter().map(of) {
                let n = places.iter().filter(|q| of(q) == p).count();
                if best.is_none_or(|(_, m)| n > m) {
                    best = Some((p, n));
                }
            }
            best.map_or(Place::Floating, |(p, _)| p)
        };
        ScreenPlace { vertical: common(|p| p.vertical), horizontal: common(|p| p.horizontal) }
    }

    /// Reads the layout table, migrating an older file's per-screen
    /// `screens` table to [`Layout::screen_place`] (an explicit
    /// `screen_place` wins) and its `menu_zoom` switch
    /// ([`Layout::migrate_menu_zoom`]; an explicit `zoom_content` wins).
    /// Unknown keys are ignored.
    fn migrating<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Layout, D::Error> {
        let mut table = toml::Table::deserialize(d)?;
        let old = table.remove("screens");
        let menu_zoom = table.remove("menu_zoom");
        let explicit = table.contains_key("screen_place");
        let explicit_content = table.contains_key("zoom_content");
        let mut layout: Layout = toml::Value::Table(table).try_into().map_err(serde::de::Error::custom)?;
        if let (false, Some(toml::Value::Table(old))) = (explicit, old) {
            layout.screen_place = Layout::migrated_place(&old);
        }
        if let (false, Some(toml::Value::Boolean(on))) = (explicit_content, menu_zoom) {
            layout.migrate_menu_zoom(on);
        }
        Ok(layout)
    }

    /// An older file's `menu_zoom` switch. On, it showed the whole picture
    /// (menus too) in a box beside the mouse in place of the world zoom
    /// chosen: now [`Zoom::Beside`] showing [`ZoomContent::WorldAndMenus`].
    /// Off, the world zoom chosen showed the map: [`ZoomContent::World`].
    pub fn migrate_menu_zoom(&mut self, menu_zoom: bool) {
        if menu_zoom {
            (self.zoom, self.zoom_content) = (Zoom::Beside, ZoomContent::WorldAndMenus);
        } else {
            self.zoom_content = ZoomContent::World;
        }
    }
}

/// How the mouse moves the map while it is free (not captured).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapMovement {
    /// The pointer resting at a window edge or corner (or pushed against
    /// the divider) scrolls the view it is in, faster nearer the edge.
    #[default]
    EdgeScroll,
    /// As in many strategy games: the left button held on the map and dragged
    /// past a few pixels grabs it, the map following the pointer 1:1 and
    /// sliding on with momentum when let go (a shorter click is a click);
    /// no edge scrolling. A quick right click is the driver's (e.g. a menu), while a
    /// right drag or a long right press preselects a world order's tile for a
    /// separate left-click confirmation.
    Drag,
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
    /// Additional motion multiplier for the captured world cursor only.
    /// The desktop pointer, menu snapping and free map movement keep their speed.
    pub capture_sensitivity: f32,
    /// Edge scrolling, or grabbing the map and dragging it.
    pub map_movement: MapMovement,
    /// Scroll lists in menus by pixels (the wheel and trackpad move them at
    /// once and they stop anywhere) independently of map movement.
    pub smooth_menu_scrolling: bool,
    /// With smooth list scrolling: once the scroll input rests, a list
    /// glides to an item's edge (else it stays where scrolling stopped).
    pub snap_lists_to_items: bool,
    /// Button snapping (the Alt mode) comes on by itself while a menu is
    /// open (a screen, popup, dialog or context menu) and goes off when the
    /// last one closes. In the world Alt still turns it on.
    pub alt_in_menus: bool,
    /// Record the Alt mode's interactions for tuning its picking: raw mouse
    /// motion, where its pointer rests and is clicked, and the boxes it can
    /// snap to (geometry and timing only, no game text or names), in files
    /// `alt-mode-*.altrec` next to the game's log, 64 MB at most in all
    /// (`docs/alt-mode-recording.md`).
    pub record_alt_mode: bool,
    /// macOS: while the game's window is in front, the system shortcuts
    /// on F1-F12 (F11 Show Desktop, Ctrl+F1..F8, ...) are paused so the keys
    /// reach the game, and restored when it is not (`function_keys`).
    /// Elsewhere nothing takes them.
    pub capture_function_keys: bool,
    /// macOS, with `capture_function_keys`: the top row's media keys
    /// (brightness, volume, Mission Control, ...) act as F1-F12 as if Fn
    /// were held. Needs the Accessibility permission.
    pub media_keys_as_function_keys: bool,
}

impl Default for Input {
    fn default() -> Self {
        Self { snap_updates_per_second: 3.0, snap_ghost_fps: 30.0, capture_sensitivity: 3.0, map_movement: MapMovement::EdgeScroll, smooth_menu_scrolling: false, snap_lists_to_items: true, alt_in_menus: true, record_alt_mode: true, capture_function_keys: true, media_keys_as_function_keys: false }
    }
}

/// What the viewer shows besides the picture (on the "Resolution" tab).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    /// A frame counter in the window's top-right corner: frames per second,
    /// the frame time, and with a software renderer the share of frames it
    /// skipped (unchanged).
    pub show_fps: bool,
    /// Add CPU frame-build wall time and delayed GPU timestamp metrics.
    pub fps_detailed: bool,
    /// How frames reach the screen (`TILE_VIEWER_PRESENT` overrides it).
    pub present: Present,
    /// The most frames per second the viewer draws; 0: as many as the
    /// display shows (its refresh rate) with `Vsync`, with the others as
    /// many as it can (`TILE_VIEWER_MAX_FPS` overrides it).
    pub max_fps: u32,
    /// Remembered slider value while the explicit frame limiter is off.
    /// Existing nonzero `max_fps` remains authoritative when enabled.
    pub fps_limit: u32,
    /// The mouse cursor's size in window pixels, the system pointer made
    /// from the game's art and the cursor the viewer draws alike;
    /// [`Display::SYSTEM_CURSOR_SIZE`] (0, the default): the operating
    /// system's cursor size (its accessibility size and display scale).
    pub cursor_size: u32,
    /// The viewer's window is a window on the desktop rather than
    /// borderless fullscreen. Switched at run time (Preferences previews it
    /// behind a timed confirmation); at start the window opens so, or as a
    /// window anyway when the launcher asks for one (`--window`). Off:
    /// fullscreen, the launcher's default.
    pub windowed: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self { show_fps: false, fps_detailed: false, present: Present::Vsync, max_fps: 0, fps_limit: 120, cursor_size: Self::SYSTEM_CURSOR_SIZE, windowed: false }
    }
}

impl Display {
    /// `cursor_size`: the operating system's.
    pub const SYSTEM_CURSOR_SIZE: u32 = 0;
    /// The sizes the preferences offer besides the system's.
    pub const CURSOR_SIZES: [u32; 7] = [24, 32, 48, 64, 96, 128, 192];

    /// The cursor size in window pixels, given the system's (`system`).
    pub fn effective_cursor_size(&self, system: u32) -> u32 {
        match self.cursor_size {
            Self::SYSTEM_CURSOR_SIZE => system.clamp(8, 512),
            n => n.clamp(8, 512),
        }
    }
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
    /// Where application `app`'s settings are kept: `$<env>` (the variable
    /// named `env`), else `$XDG_CONFIG_HOME/<app>/settings.toml`, else
    /// `~/.config/<app>/settings.toml` (`%APPDATA%\<app>\settings.toml` on
    /// Windows).
    pub fn default_path_for(app: &str, env: &str) -> PathBuf {
        if let Some(p) = std::env::var_os(env) {
            return p.into();
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join(app).join("settings.toml")
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
    fn base_ui_scale_defaults_and_migrates_saved_pixel_size() {
        assert_eq!(Settings::default().resolution.base_ui_scale, 1.0);
        for (pixel, base) in [(0, 1.0), (1, 1.0), (3, 3.0), (6, 6.0), (9, 6.0)] {
            let saved: Settings = toml::from_str(&format!("[resolution]\npixel_scale = {pixel}\n")).unwrap();
            assert_eq!(saved.resolution.base_ui_scale, base);
            let back: Settings = toml::from_str(&toml::to_string(&saved).unwrap()).unwrap();
            assert_eq!(back, saved);
        }
        for (value, expected) in [("2.5", 2.5), ("2.3", 2.5), ("0.0", 1.0), ("99.0", 6.0), ("nan", 1.0)] {
            let saved: Settings = toml::from_str(&format!("[resolution]\npixel_scale = 4\nbase_ui_scale = {value}\n")).unwrap();
            assert_eq!(saved.resolution.base_ui_scale, expected);
        }
    }

    #[test]
    fn round_trips_and_fills_defaults() {
        let mut s = Settings::default();
        s.resolution.pixel_scale = 3;
        s.layout.split = Split::Vertical2To1;
        s.layout.screen_place = ScreenPlace { vertical: Place::RightOrTop, horizontal: Place::LeftOrBottom };
        let back: Settings = toml::from_str(&toml::to_string_pretty(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.layout.place_of("F1"), Place::RightOrTop);
        let partial: Settings = toml::from_str("[layout]\nsmooth_world_movement = true\n").unwrap();
        assert!(partial.layout.smooth_world_movement);
        assert_eq!(partial.resolution, Resolution::default());
        assert!(!partial.display.show_fps);
    }

    #[test]
    fn every_zoom_is_a_fifth_high_and_older_floating_box_settings_still_load() {
        let l = Settings::default().layout;
        assert_eq!((l.zoom_strip, l.zoom_box_top, l.zoom_box_drop), (0.2, 0.025, 0.05));
        // (An older drop below the pointer is not the gap: not read.)
        let old = "[layout]\nzoom = 'Floating131'\nzoom_strip = 0.35\nzoom_box_height = 0.6\nzoom_box_block_pointer = true\nzoom_box_drop = 0.3\n";
        let s: Settings = toml::from_str(old).unwrap();
        assert_eq!((s.layout.zoom, s.layout.zoom_strip, s.layout.zoom_box_block_pointer), (Zoom::Floating131, 0.35, true));
        assert_eq!((s.layout.zoom_box_top, s.layout.zoom_box_drop), (0.025, 0.05));
        let gap: Settings = toml::from_str("[layout]\nzoom_box_gap = 0.1\n").unwrap();
        assert_eq!(gap.layout.zoom_box_drop, 0.1);
        assert!(toml::to_string(&gap).unwrap().contains("zoom_box_gap = 0.1"));
        // The magnification: 2× unless set, the nearest of the choices.
        assert_eq!((s.layout.zoom_magnification, s.layout.zoom_magnification_step()), (2.0, 1));
        for (m, step) in [(1.0, 0), (1.5, 0), (2.4, 1), (2.6, 2), (3.0, 2), (9.0, 3), (f32::NAN, 1)] {
            assert_eq!(Layout { zoom_magnification: m, ..Layout::default() }.zoom_magnification_step(), step, "{m}");
        }
        let four: Settings = toml::from_str("[layout]\nzoom_magnification = 4.0\n").unwrap();
        assert_eq!(four.layout.zoom_magnification_step(), 3);
        // The old height is read and ignored, and not written back.
        let saved = toml::to_string(&s).unwrap();
        assert!(!saved.contains("zoom_box_height"), "{saved}");
        let mut back = toml::from_str::<Settings>(&saved).unwrap();
        assert_eq!(back.layout.zoom_box_height, 0.0);
        back.layout.zoom_box_height = 0.6;
        assert_eq!(back, s);
    }

    #[test]
    fn unit_buttons_span_in_older_configs_and_the_setting_round_trips() {
        let old: Settings = toml::from_str("[layout]\nsplit = 'Vertical1To1'\n").unwrap();
        assert_eq!(old.layout.unit_buttons, UnitButtons::Spanning);
        let explicit: Settings = toml::from_str("[layout]\nunit_buttons = 'ActiveView'\n").unwrap();
        assert_eq!(explicit.layout.unit_buttons, UnitButtons::ActiveView);
        assert_eq!(toml::from_str::<Settings>(&toml::to_string(&explicit).unwrap()).unwrap(), explicit);
    }

    #[test]
    fn menus_follow_the_mouse_in_older_configs_and_the_setting_round_trips() {
        let old: Settings = toml::from_str("[layout]\nsplit = 'Vertical1To1'\n[layout.screens.Menu]\nvertical = 'LeftOrBottom'\n").unwrap();
        assert!(old.layout.menu_follows_pointer);
        assert_eq!(old.layout.screen_place.vertical, Place::LeftOrBottom);
        let explicit: Settings = toml::from_str("[layout]\nmenu_follows_pointer = false\n").unwrap();
        assert!(!explicit.layout.menu_follows_pointer);
        assert_eq!(toml::from_str::<Settings>(&toml::to_string(&explicit).unwrap()).unwrap(), explicit);
    }

    #[test]
    fn older_per_screen_places_become_the_one_place_for_every_screen() {
        // Most common per orientation; the first in the old list on a tie.
        let old = "[layout]\nsplit = 'Horizontal1To1'\nlost_setting = 3\n\
            [layout.screens.City]\nvertical = 'RightOrTop'\nhorizontal = 'LeftOrBottom'\n\
            [layout.screens.F1]\nvertical = 'LeftOrBottom'\nhorizontal = 'RightOrTop'\n\
            [layout.screens.F2]\nvertical = 'RightOrTop'\n\
            [layout.screens.Bogus]\nvertical = 'Sideways'\n";
        let s: Settings = toml::from_str(old).unwrap();
        assert_eq!(s.layout.screen_place, ScreenPlace { vertical: Place::RightOrTop, horizontal: Place::RightOrTop });
        for kind in Layout::SCREENS.iter().chain(&["Diplomacy", "Popup"]) {
            assert_eq!(s.layout.place_of(kind), Place::RightOrTop, "{kind}");
        }
        // Saved without the per-screen keys, and read back the same.
        let saved = toml::to_string_pretty(&s).unwrap();
        assert!(!saved.contains("screens"), "{saved}");
        assert_eq!(toml::from_str::<Settings>(&saved).unwrap(), s);
        // An explicit new place wins over leftovers; no old places: floating.
        let both: Settings = toml::from_str("[layout.screen_place]\nvertical = 'LeftOrBottom'\n[layout.screens.F1]\nvertical = 'RightOrTop'\n").unwrap();
        assert_eq!(both.layout.screen_place.vertical, Place::LeftOrBottom);
        let first: Settings = toml::from_str("[layout.screens.Help]\nvertical = 'RightOrTop'\n[layout.screens.F4]\nvertical = 'LeftOrBottom'\n").unwrap();
        assert_eq!(first.layout.screen_place.vertical, Place::LeftOrBottom);
        assert_eq!(toml::from_str::<Settings>("[layout]\nzoom = 'Off'\n").unwrap().layout.screen_place, ScreenPlace::default());
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
    fn map_movement_is_edge_scrolling_by_default_and_older_files_still_load() {
        assert_eq!(Settings::default().input.map_movement, MapMovement::EdgeScroll);
        assert!(!Settings::default().input.smooth_menu_scrolling);
        // A file from before the setting existed (its input section too).
        let old: Settings = toml::from_str("[input]\nsnap_updates_per_second = 5.0\n").unwrap();
        assert_eq!((old.input.map_movement, old.input.snap_updates_per_second), (MapMovement::EdgeScroll, 5.0));
        assert!(!old.input.smooth_menu_scrolling);
        // Lists snap to items by default, in older files too.
        assert!(Settings::default().input.snap_lists_to_items && old.input.snap_lists_to_items);
        let free: Settings = toml::from_str("[input]\nsnap_lists_to_items = false\n").unwrap();
        assert!(!free.input.snap_lists_to_items);
        let smooth: Settings = toml::from_str("[input]\nsmooth_menu_scrolling = true\n").unwrap();
        assert!(smooth.input.smooth_menu_scrolling);
        assert_eq!(toml::from_str::<Settings>(&toml::to_string(&smooth).unwrap()).unwrap(), smooth);
        // Alt mode in menus: on by default, older files too; off stays off.
        assert!(Settings::default().input.alt_in_menus && old.input.alt_in_menus);
        let off: Settings = toml::from_str("[input]\nalt_in_menus = false\n").unwrap();
        assert!(!off.input.alt_in_menus);
        // Recording the Alt mode: on by default, older files too; off stays off.
        assert!(Settings::default().input.record_alt_mode && old.input.record_alt_mode);
        let off: Settings = toml::from_str("[input]\nrecord_alt_mode = false\n").unwrap();
        assert!(!off.input.record_alt_mode);
        assert_eq!(toml::from_str::<Settings>(&toml::to_string(&off).unwrap()).unwrap(), off);
        let drag: Settings = toml::from_str("[input]\nmap_movement = \"Drag\"\n").unwrap();
        assert_eq!(drag.input.map_movement, MapMovement::Drag);
        assert_eq!(drag.input.snap_ghost_fps, Input::default().snap_ghost_fps);
        let text = toml::to_string_pretty(&drag).unwrap();
        assert!(text.contains("map_movement = \"Drag\""), "{text}");
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), drag);
        assert!(toml::from_str::<Settings>("[input]\nmap_movement = \"Wobble\"\n").is_err());
    }

    #[test]
    fn capture_sensitivity_defaults_for_older_files_and_round_trips() {
        for text in ["", "[input]\nsnap_updates_per_second = 5.0\n"] {
            let settings: Settings = toml::from_str(text).unwrap();
            assert_eq!(settings.input.capture_sensitivity, 3.0);
        }
        let settings: Settings = toml::from_str("[input]\ncapture_sensitivity = 2.5\n").unwrap();
        assert_eq!(settings.input.capture_sensitivity, 2.5);
        assert_eq!(toml::from_str::<Settings>(&toml::to_string(&settings).unwrap()).unwrap(), settings);
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
        assert_eq!(r.describe_pixel_scale(), "×3");
        assert_eq!(Resolution { pixel_scale: 9, ..Default::default() }.realised((3840, 2160)), (640, 360));
        // A small window: the pixel scale lowered to keep 320×240 at least.
        assert_eq!(r.realised((800, 600)), (400, 300));
        assert_eq!(r.realised((100, 100)), (100, 100));
        assert_eq!(r.realised((0, 0)), (1, 1));
        // The default, on a Retina window too: every window pixel.
        let default = Resolution::default();
        assert_eq!(default.realised((3258, 1904)), (3258, 1904));
        assert_eq!(default.describe_pixel_scale(), "×1");
    }

    #[test]
    fn a_first_run_on_a_high_density_desktop_starts_at_the_old_sizes() {
        // TS Neo: 200 %, a 1904-row window. A 768-row menu took 1536 rows.
        let mut r = Resolution::default();
        r.seed_for_desktop(2.0, 1904);
        assert_eq!((r.pixel_scale, r.frame_scale, r.world_scale, r.scale_menus, r.menu_scale_percent), (1, 2.0, 2.0, true, 81.0));
        // 150 % (Auto was ×1 there) and 100 %: unchanged.
        for desktop in [1.0, 1.5, f64::NAN] {
            let mut r = Resolution::default();
            r.seed_for_desktop(desktop, 1080);
            assert_eq!(r, Resolution::default(), "{desktop}");
        }
        // Within the sliders and the menu percent's range.
        let mut r = Resolution::default();
        r.seed_for_desktop(4.0, 1000);
        assert_eq!((r.frame_scale, r.world_scale, r.menu_scale_percent), (3.0, 3.0, 100.0));
    }

    #[test]
    fn a_settings_file_marks_the_first_run_over_only_no_file_starts_one() {
        assert!(!Settings::default().first_run.preferences_shown, "no file: the first run");
        let existing: Settings = toml::from_str("[resolution]\npixel_scale = 2\n").unwrap();
        assert!(existing.first_run.preferences_shown, "an existing install's file");
        let mut fresh = Settings::default();
        let saved: Settings = toml::from_str(&toml::to_string(&fresh).unwrap()).unwrap();
        assert!(!saved.first_run.preferences_shown, "saved before Preferences opened");
        fresh.first_run.preferences_shown = true;
        let text = toml::to_string(&fresh).unwrap();
        assert!(text.contains("[first_run]") && text.contains("preferences_shown = true"), "{text}");
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), fresh);
    }

    #[test]
    fn menu_scaling_migrates_the_old_multiplier_without_reinterpreting_it() {
        for multiplier in [0.5, 1.0, 1.5, 3.0] {
            let s: Settings = toml::from_str(&format!("[resolution]\nmenu_scale = {multiplier}\nframe_scale = 1.25\n")).unwrap();
            assert!(!s.resolution.scale_menus);
            assert_eq!(s.resolution.menu_scale_percent, 100.0);
            assert_eq!(s.resolution.frame_scale, 1.25);
            let saved = toml::to_string(&s).unwrap();
            assert!(saved.contains("scale_menus = false"));
            assert!(saved.contains("menu_scale_percent = 100.0"));
            assert!(!saved.lines().any(|line| line.starts_with("menu_scale =")));
        }
        assert_eq!(toml::from_str::<Settings>("").unwrap().resolution, Resolution::default());
    }

    #[test]
    fn menu_percent_roundtrips_and_loaded_values_are_bounded() {
        let mut s = Settings::default();
        s.resolution.scale_menus = true;
        s.resolution.menu_scale_percent = 37.0;
        assert_eq!(toml::from_str::<Settings>(&toml::to_string(&s).unwrap()).unwrap(), s);
        let mixed: Settings = toml::from_str("[resolution]\nmenu_scale = 3.0\nscale_menus = true\nmenu_scale_percent = 37.0\n").unwrap();
        assert_eq!(mixed.resolution, s.resolution);
        for (value, expected) in [("-4", 25.0), ("150", 100.0), ("nan", 100.0), ("inf", 100.0), ("60.758", 61.0), ("37.49", 37.0), ("25.2", 25.0), ("99.5", 100.0)] {
            let s: Settings = toml::from_str(&format!("[resolution]\nscale_menus = true\nmenu_scale_percent = {value}\n")).unwrap();
            assert!(s.resolution.scale_menus);
            assert_eq!(s.resolution.menu_scale_percent, expected);
        }
    }

    #[test]
    fn a_saved_pixel_scale_stays_what_it_was() {
        // A number stays; "Auto" (0) and none are ×1.
        let three: Settings = toml::from_str("[resolution]\npixel_scale = 3\n").unwrap();
        assert_eq!(three.resolution.pixel_scale, 3);
        let auto: Settings = toml::from_str("[resolution]\npixel_scale = 0\n").unwrap();
        assert_eq!(auto.resolution.pixel_scale, 1);
        let none: Settings = toml::from_str("[resolution]\nmenu_scale = 1.5\n").unwrap();
        assert_eq!(none.resolution.pixel_scale, 1);
    }

    #[test]
    fn background_sampling_defaults_to_dither_and_roundtrips() {
        let old: Settings = toml::from_str("[resolution]\nworld_scale = 2.0\n").unwrap();
        assert_eq!(old.resolution.background_sampling, BackgroundSampling::Dither);
        for mode in [BackgroundSampling::Dither, BackgroundSampling::Interpolate, BackgroundSampling::Nearest] {
            let mut s = Settings::default();
            s.resolution.background_sampling = mode;
            let text = toml::to_string(&s).unwrap();
            assert_eq!(toml::from_str::<Settings>(&text).unwrap(), s, "{text}");
        }
        let smooth: Settings = toml::from_str("[resolution]\nbackground_sampling = \"Interpolate\"\n").unwrap();
        assert_eq!(smooth.resolution.background_sampling, BackgroundSampling::Interpolate);
        assert!(toml::from_str::<Settings>("[resolution]\nbackground_sampling = \"Blur\"\n").is_err());
    }
}

#[cfg(test)]
mod combat_speed_settings_tests {
    use super::*;
    #[test]
    fn old_files_default_and_saved_speed_round_trips_with_sane_steps() {
        for text in ["", "[display]\nshow_fps=true\n", "[animation]\n"] {
            assert_eq!(toml::from_str::<Settings>(text).unwrap().animation.combat_speed, 1.0);
        }
        for (raw, expected) in [("0", 1.0), ("1.38", 1.5), ("2.13", 2.25), ("9", 4.0), ("nan", 1.0)] {
            let setting: Settings = toml::from_str(&format!("[animation]\ncombat_speed={raw}\n")).unwrap();
            assert_eq!(setting.animation.combat_speed, expected);
            assert_eq!(toml::from_str::<Settings>(&toml::to_string(&setting).unwrap()).unwrap(), setting);
        }
    }
    #[test]
    fn combat_slider_changes_only_its_committed_setting() {
        let mut settings = Settings::default(); let original = settings.clone();
        assert!(settings.set_control("animation.combat_speed", &controls::ControlValue::Number(2.13)));
        assert_eq!(settings.animation.combat_speed, 2.25);
        assert!(settings.set_control("animation.combat_speed", &controls::ControlValue::Number(f32::NAN)));
        assert_eq!(settings.animation.combat_speed, 1.0);
        settings.animation = original.animation.clone(); assert_eq!(settings, original);
    }
}

#[cfg(test)]
mod zoom_glass_tests {
    use super::*;
    use controls::ControlValue;

    #[test]
    fn old_layouts_default_to_glass_and_new_controls_round_trip() {
        let mut settings: Settings = toml::from_str("[layout]\nzoom = 'Floating131'\nzoom_strip = 0.35").unwrap();
        assert!(settings.layout.zoom_rounded && settings.layout.zoom_glass);
        assert_eq!(settings.layout.zoom_content, ZoomContent::World);
        assert_eq!(settings.control("layout.zoom_content"), Some(ControlValue::Choice("World".into())));
        assert!(!settings.set_control("layout.zoom_content", &ControlValue::On(true)));
        assert!(settings.set_control("layout.zoom_content", &ControlValue::Choice("WorldAndMenus".into())));
        assert_eq!(settings.control("layout.zoom_content"), Some(ControlValue::Choice("WorldAndMenus".into())));
        assert!(settings.set_control("layout.zoom", &ControlValue::Choice("Beside".into())));
        assert_eq!(settings.control("layout.menu_zoom"), None);
        assert_eq!(settings.layout.separator_width, 4);
        assert_eq!(settings.layout.zoom_strip, 0.35);
        for key in ["layout.zoom_rounded", "layout.zoom_glass"] {
            assert!(settings.set_control(key, &ControlValue::On(false)));
        }
        assert!(settings.set_control("layout.separator_width", &ControlValue::Number(99.0)));
        assert_eq!(settings.layout.separator_width, 16);
        assert_eq!(toml::from_str::<Settings>(&toml::to_string(&settings).unwrap()).unwrap(), settings);
    }
}

#[cfg(test)]
mod zoom_content_migration_tests {
    use super::*;

    fn load(text: &str) -> Layout {
        toml::from_str::<Settings>(text).unwrap().layout
    }

    /// The old "Menu zoom beside pointer" switch: on, whatever world zoom
    /// was chosen, the menus were shown beside the mouse; that is the box
    /// beside the mouse showing menus now. Off, the world zoom chosen shows
    /// the map as before. An explicit `zoom_content` wins, and the switch
    /// is never written again.
    #[test]
    fn menu_zoom_loads_as_a_box_beside_the_mouse_showing_menus() {
        for (zoom, name) in [(Zoom::Off, "Off"), (Zoom::Floating131, "Floating131"), (Zoom::Floating171, "Floating171"),
            (Zoom::Mouse111, "Mouse111"), (Zoom::Mouse131, "Mouse131"), (Zoom::TopStrip, "TopStrip")] {
            let on = load(&format!("[layout]\nzoom = '{name}'\nmenu_zoom = true\nzoom_strip = 0.3\nzoom_magnification = 3.0\n"));
            assert_eq!((on.zoom, on.zoom_content), (Zoom::Beside, ZoomContent::WorldAndMenus), "{name}");
            assert_eq!((on.zoom_strip, on.zoom_magnification), (0.3, 3.0), "{name}: size and magnification kept");
            let off = load(&format!("[layout]\nzoom = '{name}'\nmenu_zoom = false\n"));
            assert_eq!((off.zoom, off.zoom_content), (zoom, ZoomContent::World), "{name}");
        }
        // No switch at all: the defaults.
        assert_eq!(load("[layout]\nzoom = 'Mouse131'\n").zoom_content, ZoomContent::World);
        // A newer file's own choice wins over a stray old switch.
        let both = load("[layout]\nzoom = 'TopStrip'\nmenu_zoom = true\nzoom_content = 'World'\n");
        assert_eq!((both.zoom, both.zoom_content), (Zoom::TopStrip, ZoomContent::World));
        // Saved again: the new keys only, and they load back the same.
        let mut s = Settings::default();
        s.layout = load("[layout]\nzoom = 'Floating171'\nmenu_zoom = true\n");
        let saved = toml::to_string(&s).unwrap();
        assert!(!saved.contains("menu_zoom"), "{saved}");
        assert!(saved.contains("zoom = \"Beside\"") && saved.contains("zoom_content = \"WorldAndMenus\""), "{saved}");
        assert_eq!(toml::from_str::<Settings>(&saved).unwrap(), s);
    }
}

#[cfg(test)]
mod glass_dividers_tests {
    use super::*;
    use controls::ControlValue;

    #[test]
    fn glass_dividers_are_off_by_default_and_round_trip() {
        let mut settings: Settings = toml::from_str("[layout]\nsplit = 'Vertical1To1'").unwrap();
        assert!(!settings.layout.glass_dividers);
        assert_eq!(settings.control("layout.glass_dividers"), Some(ControlValue::On(false)));
        assert!(!settings.set_control("layout.glass_dividers", &ControlValue::Number(1.0)));
        assert!(settings.set_control("layout.glass_dividers", &ControlValue::On(true)));
        assert!(settings.layout.glass_dividers);
        assert_eq!(toml::from_str::<Settings>(&toml::to_string(&settings).unwrap()).unwrap(), settings);
    }
}
