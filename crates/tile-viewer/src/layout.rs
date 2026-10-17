//! How the viewer arranges the world in its window beyond what the driver
//! asks for: the driver's main view zoomed or moved, a second pane beside
//! it, floating insets and a magnifier beside the pointer. The driver's
//! interface (interface-only views) stays on top, unscaled, where the driver
//! put it.
//!
//! Every arranged view follows something ([`Follow`]): what the driver's
//! main view shows, the driver's point of interest (`Scene::focus`, e.g. the
//! active unit), the world cursor, or nothing (moved by hand). The world
//! cursor is the pointer's world pixel; a view following it keeps it in its
//! middle (but not past an edge of the world that does not wrap). Clicking
//! into such a view captures the mouse: its movement then moves the world
//! cursor, which is drawn in every view that shows it.
//!
//! Pointer input is routed through the arrangement: what lands on the
//! interface goes to the driver unchanged, what lands on the world goes to
//! the point of the driver's main view showing the same world pixel, and
//! world pixels the driver's view does not show (or covers with its
//! interface) become [`Input::WorldButton`] (with [`Input::Look`] for the
//! left button), and keys typed with the pointer over them
//! [`Input::WorldKey`].
//!
//! Controls (Ctrl+Shift with a key, so they do not collide with a game's):
//! S cycles the split (off, side by side, stacked), I adds an inset at the
//! pointer, X removes the inset under it, F cycles what the view under the
//! pointer follows, Space releases the mouse (over a view following the world
//! cursor while free: takes it, as a click there does; each change is
//! logged), M cycles the
//! magnifier (off, everything on screen, the world only), A toggles the
//! arrow at the point of interest, U puts the driver's whole screen
//! (interface too) into the main pane of a split instead of over the window,
//! arrow at the point of interest, T toggles sharp text in zoomed views (text
//! drawn at their zoom by the driver's zoomed art, or its pixels scaled), +/− and
//! Ctrl+wheel zoom what is under
//! the pointer, 0 resets its zoom, arrows pan it; a middle-button drag pans
//! a view, Ctrl+Shift+left drag moves an inset (resizes it at its
//! bottom-right corner), a left drag on the divider moves it.
//!
//! [`Layout::configure`] sets it up from a spec such as
//! `split=vertical,main=focus,pane=pointer,pane_zoom=2`.

use scene::{Button, Input, Scene};
use tile_render::{Camera, ScreenCopy};

/// Zoom steps, screen pixels per world pixel.
pub const ZOOMS: [(u32, u32); 11] = [(1, 4), (1, 3), (1, 2), (2, 3), (1, 1), (3, 2), (2, 1), (3, 1), (4, 1), (6, 1), (8, 1)];
const ONE: usize = 4;
/// Most insets at a time.
pub const MAX_INSETS: usize = 3;
/// Gap between the panes, and the frame around insets and the magnifier.
const GAP: i32 = 4;
/// A view's centre before it was placed (it starts where the driver looks).
const UNPLACED: (i32, i32) = (i32::MIN, i32::MIN);
/// For how many frames after a world first shows a free view not yet
/// moved by hand follows the driver's view (a game loading a save moves
/// its view about for a moment, about 3 s at 60 frames a second).
pub const SETTLE_FRAMES: u32 = 180;

/// What a view keeps in sight.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Follow {
    /// What the driver's main view shows.
    #[default]
    Driver,
    /// The driver's point of interest (`Scene::focus`, e.g. the active unit).
    Focus,
    /// The world cursor, in the middle.
    Pointer,
    /// Where the user moved it.
    Free,
}

impl Follow {
    fn next(self) -> Follow {
        match self {
            Follow::Driver => Follow::Focus,
            Follow::Focus => Follow::Pointer,
            Follow::Pointer => Follow::Free,
            Follow::Free => Follow::Driver,
        }
    }

    fn parse(s: &str) -> Option<Follow> {
        Some(match s {
            "driver" | "game" => Follow::Driver,
            "focus" | "unit" => Follow::Focus,
            "pointer" | "mouse" => Follow::Pointer,
            "free" => Follow::Free,
            _ => return None,
        })
    }
}

/// A view the viewer arranges: the world pixel at its centre, zoom (index
/// into [`ZOOMS`]) and what it follows.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FreeView {
    pub center: (i32, i32),
    pub zoom: usize,
    pub follow: Follow,
    /// Placed by the viewer, not by hand, while the driver's view may still
    /// be settling (see [`SETTLE_FRAMES`]): it goes on following it.
    settling: bool,
}

impl FreeView {
    pub fn new(center: (i32, i32), zoom: usize, follow: Follow) -> Self {
        Self { center, zoom, follow, settling: false }
    }

    /// World pixels it shows in a rectangle `rect` wide and high.
    fn source(&self, rect: [i32; 4]) -> (i32, i32) {
        let zoom = ZOOMS[self.zoom];
        ((rect[2].max(1) as u32 * zoom.1).div_ceil(zoom.0) as i32, (rect[3].max(1) as u32 * zoom.1).div_ceil(zoom.0) as i32)
    }

    /// Its camera in `rect`: centred, but not past an edge of the world
    /// that does not wrap.
    fn camera(&self, rect: [i32; 4], overhang: (u32, u32, u32, u32), bounds: Option<WorldBounds>) -> Camera {
        let (sw, sh) = self.source(rect);
        // (Not placed yet: where the world starts.)
        let center = if self.center == UNPLACED { (0, 0) } else { self.center };
        let (cx, cy) = match bounds {
            Some(b) => (b.clamp_center(0, center.0, sw), b.clamp_center(1, center.1, sh)),
            None => center,
        };
        Camera {
            x: cx - sw / 2,
            y: cy - sh / 2,
            zoom: ZOOMS[self.zoom],
            origin: (rect[0], rect[1]),
            overhang,
            ..Camera::new((rect[2].max(1) as u32, rect[3].max(1) as u32))
        }
    }
}

/// The world's extent in pixels, and which way it wraps.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WorldBounds {
    pub size: (i32, i32),
    pub wrap: (bool, bool),
}

impl WorldBounds {
    fn axis(&self, axis: usize) -> (i32, bool) {
        if axis == 0 { (self.size.0, self.wrap.0) } else { (self.size.1, self.wrap.1) }
    }

    /// A view centre on `axis` for a view `span` world pixels long.
    fn clamp_center(&self, axis: usize, c: i32, span: i32) -> i32 {
        match self.axis(axis) {
            (_, true) => c,
            (size, false) if size <= span => size / 2,
            (size, false) => c.clamp(span / 2, size - span + span / 2),
        }
    }

    /// A world pixel kept on the world (wrapped where it wraps).
    fn clamp_point(&self, (x, y): (i32, i32)) -> (i32, i32) {
        let one = |c: i32, (size, wrap): (i32, bool)| match wrap {
            true => c.rem_euclid(size.max(1)),
            false => c.clamp(0, (size - 1).max(0)),
        };
        (one(x, self.axis(0)), one(y, self.axis(1)))
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Side by side.
    Vertical,
    /// One over the other.
    Horizontal,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Split {
    pub axis: Axis,
    /// The main pane's share of the window.
    pub fraction: f32,
    pub view: FreeView,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Inset {
    /// Window pixels: x, y, width, height.
    pub rect: [i32; 4],
    pub view: FreeView,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MagnifierMode {
    /// Everything on screen under the pointer.
    Screen,
    /// The world under the pointer, without the interface.
    World,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Magnifier {
    pub mode: MagnifierMode,
    pub zoom: usize,
    pub size: (u32, u32),
}

/// A sprite the viewer adds to the world.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Marker {
    /// The world cursor at this world pixel.
    Cursor((i32, i32)),
    /// The arrow at the point of interest.
    Focus((i32, i32)),
}

/// What one frame draws: cameras in drawing order, for each scene view the
/// camera that shows its overlay, screen copies and markers on top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub cameras: Vec<Camera>,
    pub view_cameras: Vec<Option<u32>>,
    pub copies: Vec<ScreenCopy>,
    pub markers: Vec<Marker>,
}

/// Where a point of the window shows the world.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Hit {
    /// On the driver's interface (or nothing the viewer arranges).
    Interface,
    Main,
    Pane,
    Inset(usize),
    Magnifier,
    Divider,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Drag {
    Pan { hit: Hit, from: (i32, i32), center: (i32, i32) },
    MoveInset { index: usize, from: (i32, i32), rect: [i32; 4], resize: bool },
    Divider,
}

/// The arrangement, and the state of pointer gestures.
#[derive(Clone, Debug)]
pub struct Layout {
    /// The main pane (normally following the driver's main view).
    pub main_view: FreeView,
    pub split: Option<Split>,
    pub insets: Vec<Inset>,
    pub magnifier: Option<Magnifier>,
    /// Whether to draw the arrow at the driver's point of interest.
    pub focus_arrow: bool,
    /// With a split: the driver's whole screen (its interface too) fitted
    /// into the main pane, rather than its interface over the window and
    /// only its world in the pane.
    pub ui_in_pane: bool,
    /// From the last frame: where the driver's screen sits in the main pane
    /// and at which zoom, when it is fitted there.
    fit: Option<((i32, i32), (u32, u32))>,
    /// Whether zoomed views show text at their resolution (the driver's
    /// zoomed art) rather than its pixels scaled.
    pub sharp_text: bool,
    window: (u32, u32),
    pointer: (i32, i32),
    /// The world cursor, and the motion not yet a whole world pixel.
    cursor: (i32, i32),
    motion_rest: (f64, f64),
    /// Whether the mouse is captured (moving the world cursor), and a
    /// change of that for the window to make.
    locked: bool,
    grab: Option<bool>,
    /// The zooms in use as last told to the driver.
    reported_zooms: [(u8, u8); 8],
    /// The world rectangles of the viewer's own views as last told to the
    /// driver.
    reported_views: [(i32, i32, i32, i32); 4],
    /// Zooms the scene has zoomed art (text) for, from the last frame ((0, 0):
    /// every zoom).
    art_zooms: Vec<(u32, u32)>,
    ctrl: bool,
    shift: bool,
    drag: Option<Drag>,
    /// Per button: whether its press went to the driver.
    pressed: [bool; 3],
    /// Per button: the world pixel its press went to as `Input::WorldButton`.
    world_pressed: [Option<(i32, i32)>; 3],
    /// Keys whose press went to the driver as `Input::WorldKey`.
    world_keys: Vec<u32>,
    /// From the last frame: the driver's main view (scene camera, unzoomed),
    /// where the world is shown, the interface rectangles, the world.
    main: Option<Camera>,
    shown: Vec<(Hit, Camera)>,
    interface: Vec<[i32; 4]>,
    /// From the last frame: the rectangles of the driver's other world views
    /// (e.g. a city screen's map) over the window, how many insets were
    /// drawn, and how many of the viewer's views were left out.
    driver_rects: Vec<[i32; 4]>,
    insets_shown: usize,
    left_out: usize,
    /// Frames in a row the driver has shown a world.
    world_frames: u32,
    bounds: Option<WorldBounds>,
    /// The pointer's place at the window's edge as last told to the driver
    /// (`Input::Edge`).
    edge: (i32, i32),
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            main_view: FreeView::new(UNPLACED, ONE, Follow::Driver),
            split: None,
            insets: Vec::new(),
            magnifier: None,
            focus_arrow: false,
            window: (1, 1),
            pointer: (0, 0),
            cursor: (0, 0),
            motion_rest: (0.0, 0.0),
            locked: false,
            grab: None,
            reported_zooms: [(0, 0); 8],
            art_zooms: Vec::new(),
            reported_views: [(0, 0, 0, 0); 4],
            sharp_text: true,
            ui_in_pane: false,
            fit: None,
            ctrl: false,
            shift: false,
            drag: None,
            pressed: [false; 3],
            world_pressed: [None; 3],
            world_keys: Vec::new(),
            main: None,
            shown: Vec::new(),
            interface: Vec::new(),
            driver_rects: Vec::new(),
            insets_shown: 0,
            left_out: 0,
            world_frames: 0,
            bounds: None,
            edge: (0, 0),
        }
    }
}

const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;

fn inside([x, y, w, h]: [i32; 4], (px, py): (i32, i32)) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

fn button_index(b: Button) -> usize {
    match b {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
    }
}

/// The world pixel `camera` shows at window pixel `p`.
fn world_at(camera: &Camera, (px, py): (i32, i32)) -> (i32, i32) {
    let (n, d) = (camera.zoom.0.max(1) as i32, camera.zoom.1.max(1) as i32);
    (camera.x + (px - camera.origin.0) * d / n, camera.y + (py - camera.origin.1) * d / n)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

fn parse_zoom(s: &str) -> Option<usize> {
    let (n, d) = match s.split_once('/') {
        Some((n, d)) => (n.trim().parse().ok()?, d.trim().parse().ok()?),
        None => (s.trim().parse().ok()?, 1),
    };
    ZOOMS.iter().position(|&z| z == (n, d))
}

impl Layout {
    /// Sets the layout from a spec: comma-separated `key=value`, keys
    /// `split` (vertical, horizontal, off), `fraction` (the main pane's
    /// share), `main` and `pane` (what they follow: driver, focus, pointer,
    /// free), `main_zoom` and `pane_zoom` (e.g. 2 or 1/2), `magnifier`
    /// (screen, world, off), `magnifier_zoom`, `arrow` (on, off), `text`
    /// (sharp: zoomed views show text at their resolution; pixel), `ui`
    /// (window; pane: with a split, the driver's whole screen in the main pane).
    pub fn configure(&mut self, spec: &str) -> Result<(), String> {
        for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (key, value) = item.split_once('=').ok_or_else(|| format!("{item:?}: expected key=value"))?;
            let bad = || format!("{key}: bad value {value:?}");
            fn pane(l: &mut Layout) -> &mut Split {
                l.split.get_or_insert(Split { axis: Axis::Vertical, fraction: 0.5, view: FreeView::new(UNPLACED, ONE, Follow::Free) })
            }
            match key {
                "split" => match value {
                    "vertical" => pane(self).axis = Axis::Vertical,
                    "horizontal" => pane(self).axis = Axis::Horizontal,
                    "off" => self.split = None,
                    _ => return Err(bad()),
                },
                "fraction" => pane(self).fraction = value.parse::<f32>().map_err(|_| bad())?.clamp(0.1, 0.9),
                "main" => self.main_view.follow = Follow::parse(value).ok_or_else(bad)?,
                "pane" => pane(self).view.follow = Follow::parse(value).ok_or_else(bad)?,
                "main_zoom" => self.main_view.zoom = parse_zoom(value).ok_or_else(bad)?,
                "pane_zoom" => pane(self).view.zoom = parse_zoom(value).ok_or_else(bad)?,
                "magnifier" => {
                    let mode = match value {
                        "screen" => Some(MagnifierMode::Screen),
                        "world" => Some(MagnifierMode::World),
                        "off" => None,
                        _ => return Err(bad()),
                    };
                    self.magnifier = mode.map(|mode| Magnifier { mode, ..self.magnifier.unwrap_or(Self::MAGNIFIER) });
                }
                "magnifier_zoom" => {
                    let zoom = parse_zoom(value).ok_or_else(bad)?;
                    self.magnifier = Some(Magnifier { zoom, ..self.magnifier.unwrap_or(Self::MAGNIFIER) });
                }
                "arrow" => self.focus_arrow = matches!(value, "on" | "1" | "true"),
                "ui" => {
                    self.ui_in_pane = match value {
                        "pane" => true,
                        "window" => false,
                        _ => return Err(bad()),
                    }
                }
                "text" => {
                    self.sharp_text = match value {
                        "sharp" => true,
                        "pixel" => false,
                        _ => return Err(bad()),
                    }
                }
                _ => return Err(format!("unknown key {key:?}")),
            }
        }
        Ok(())
    }

    const MAGNIFIER: Magnifier = Magnifier { mode: MagnifierMode::Screen, zoom: ONE + 3, size: (240, 160) };

    /// The zooms of the views showing the world scaled, when they changed
    /// since last asked (for the driver, `Input::Zooms`; none without sharp
    /// text).
    pub fn take_zooms_change(&mut self) -> Option<[(u8, u8); 8]> {
        let mut zooms = [(0, 0); 8];
        if self.sharp_text {
            let views = std::iter::once(self.main_view).chain(self.split.map(|s| s.view)).chain(self.insets.iter().map(|i| i.view));
            let used = views.map(|v| v.zoom).chain(self.magnifier.filter(|m| m.mode == MagnifierMode::World).map(|m| m.zoom)).filter(|&z| z != ONE);
            // (The fitted screen's zoom, too.)
            let mut used: Vec<(u32, u32)> = used.map(|z| ZOOMS[z]).chain(self.fit.map(|f| f.1)).filter(|z| z.0 <= 255 && z.1 <= 255).collect();
            used.sort();
            used.dedup();
            for (slot, z) in zooms.iter_mut().zip(used) {
                *slot = (z.0 as u8, z.1 as u8);
            }
        }
        (zooms != self.reported_zooms).then(|| {
            self.reported_zooms = zooms;
            zooms
        })
    }

    /// The world rectangles the split pane and the insets show (as of the
    /// last frame), when they changed since last asked (for the driver,
    /// `Input::Views`).
    pub fn take_views_change(&mut self) -> Option<[(i32, i32, i32, i32); 4]> {
        let mut views = [(0, 0, 0, 0); 4];
        let own = self.shown.iter().filter(|(h, _)| matches!(h, Hit::Pane | Hit::Inset(_)));
        for (slot, (_, c)) in views.iter_mut().zip(own) {
            let (w, h) = c.source_size();
            *slot = (c.x, c.y, w as i32, h as i32);
        }
        (views != self.reported_views).then(|| {
            self.reported_views = views;
            views
        })
    }

    /// With the driver's screen in the main pane (`ui_in_pane`, split): the
    /// zoom fitting the window into `rect` (reduced).
    fn screen_fit(&self, rect: [i32; 4]) -> Option<(u32, u32)> {
        self.split?;
        if !self.ui_in_pane {
            return None;
        }
        let (ww, wh) = (self.window.0.max(1), self.window.1.max(1));
        let (pw, ph) = (rect[2].max(1) as u32, rect[3].max(1) as u32);
        // The smaller of the two ratios, as a fraction.
        let (n, d) = if pw as u64 * wh as u64 <= ph as u64 * ww as u64 { (pw, ww) } else { (ph, wh) };
        let g = gcd(n, d);
        Some((n / g, d / g))
    }

    /// Whether a view at `zoom` shows the driver's zoomed art.
    fn zoomed_art(&self, zoom: (u32, u32)) -> bool {
        self.sharp_text && zoom.0 != zoom.1 && self.art_zooms.iter().any(|&z| z == zoom || z == (0, 0))
    }

    /// A change of mouse capture for the window to make (true: capture and
    /// hide the pointer, report motion to [`Layout::motion`]).
    pub fn take_grab(&mut self) -> Option<bool> {
        self.grab.take()
    }

    /// The mouse was let go (e.g. the window lost focus).
    pub fn released(&mut self) {
        if self.locked {
            self.release("the window lost the mouse");
        }
    }

    /// Captures the mouse at window pixel `p` of `hit` (a view following
    /// the world cursor): the world cursor starts there.
    fn capture(&mut self, hit: Hit, p: (i32, i32)) {
        if let Some((_, camera)) = self.shown.iter().find(|(h, _)| *h == hit) {
            self.cursor = world_at(camera, p);
        }
        self.locked = true;
        self.grab = Some(true);
        log::info!("viewer: mouse captured, it moves the world cursor at {:?} (Ctrl+Shift+Space releases it)", self.cursor);
    }

    /// Releases the mouse (never captures it).
    fn release(&mut self, why: &str) {
        let was = std::mem::replace(&mut self.locked, false);
        self.grab = Some(false);
        if was {
            log::info!("viewer: mouse released ({why})");
        }
    }

    /// The pointer left the window, or the window lost the focus: it is at
    /// no edge any more (what goes to the driver).
    pub fn pointer_gone(&mut self) -> Vec<Input> {
        self.edge_change((0, 0))
    }

    /// `Input::Edge` for the pointer now at `edge`, if that changed.
    fn edge_change(&mut self, edge: (i32, i32)) -> Vec<Input> {
        if edge == self.edge {
            return vec![];
        }
        self.edge = edge;
        vec![Input::Edge { x: edge.0, y: edge.1 }]
    }

    /// Where pointer `p` is at the window's edge (see `Input::Edge`): only
    /// over the driver's main view shown unmoved, at zoom 1 where the driver
    /// shows it (a window pixel is the driver's), so that the window's edge
    /// is the driver's screen edge; not while the mouse is captured or
    /// dragging, nor with the driver's screen fitted into a pane.
    fn edge_at(&self, p: (i32, i32)) -> (i32, i32) {
        let (main_rect, _) = self.panes();
        let unmoved = self.main.is_some_and(|m| {
            self.shown.iter().any(|(h, c)| *h == Hit::Main && c.zoom == (1, 1) && (c.x, c.y, c.origin) == (m.x, m.y, m.origin) && c.screen_shift == (0, 0))
        });
        let over_main = match self.hit(p) {
            Hit::Main => true,
            // The driver's interface where the main view is.
            Hit::Interface => inside(main_rect, p),
            _ => false,
        };
        if !unmoved || !over_main || self.locked || self.drag.is_some() || self.fit.is_some() {
            return (0, 0);
        }
        let at = |v: i32, size: u32| {
            let far = size as i32 - 1 - v;
            if v < 0 || far < 0 {
                0
            } else if v < scene::EDGE_ZONE {
                -(v + 1)
            } else if far < scene::EDGE_ZONE {
                far + 1
            } else {
                0
            }
        };
        (at(p.0, self.window.0), at(p.1, self.window.1))
    }

    /// The main pane's rectangle, and the second pane's if split.
    fn panes(&self) -> ([i32; 4], Option<[i32; 4]>) {
        let (w, h) = (self.window.0 as i32, self.window.1 as i32);
        match self.split {
            None => ([0, 0, w, h], None),
            Some(s) => match s.axis {
                Axis::Vertical => {
                    let a = ((w as f32 * s.fraction) as i32).clamp(GAP, (w - GAP).max(GAP));
                    ([0, 0, a, h], Some([a + GAP, 0, w - a - GAP, h]))
                }
                Axis::Horizontal => {
                    let a = ((h as f32 * s.fraction) as i32).clamp(GAP, (h - GAP).max(GAP));
                    ([0, 0, w, a], Some([0, a + GAP, w, h - a - GAP]))
                }
            },
        }
    }

    fn divider(&self) -> Option<[i32; 4]> {
        let (main, pane) = self.panes();
        let pane = pane?;
        Some(match self.split?.axis {
            Axis::Vertical => [main[2] - 2, 0, pane[0] - main[2] + 4, main[3]],
            Axis::Horizontal => [0, main[3] - 2, main[2], pane[1] - main[3] + 4],
        })
    }

    /// The magnifier's rectangle, and (screen magnifier) the rectangle of the
    /// window it magnifies: around the pointer, inside the window, the
    /// magnifier beside it (so it never shows itself or its frame).
    fn magnifier_geometry(&self, m: &Magnifier) -> ([i32; 4], Option<[i32; 4]>) {
        let (w, h) = (m.size.0 as i32, m.size.1 as i32);
        let (ww, wh) = (self.window.0 as i32, self.window.1 as i32);
        let (px, py) = self.pointer;
        let y = (py - h / 2).clamp(0, (wh - h).max(0));
        if m.mode == MagnifierMode::World {
            // Beside the pointer, kept inside the window.
            let x = if px + 24 + w <= ww { px + 24 } else { px - 24 - w };
            return ([x.clamp(0, (ww - w).max(0)), y, w, h], None);
        }
        let (n, d) = ZOOMS[m.zoom];
        let (sw, sh) = ((m.size.0 * d).div_ceil(n) as i32, (m.size.1 * d).div_ceil(n) as i32);
        let (sx, sy) = ((px - sw / 2).clamp(0, (ww - sw).max(0)), (py - sh / 2).clamp(0, (wh - sh).max(0)));
        // Right of what it magnifies, else left of it, its frame clear of it.
        let x = if sx + sw + GAP + w <= ww {
            sx + sw + GAP
        } else if sx - GAP - w >= 0 {
            sx - GAP - w
        } else {
            (ww - w).max(0)
        };
        ([x, y, w, h], Some([sx, sy, sw, sh]))
    }

    /// The window pixel a screen magnifier (`rect`, magnifying `source`)
    /// shows at `p`.
    fn magnified(&self, m: &Magnifier, rect: [i32; 4], source: [i32; 4], p: (i32, i32)) -> (i32, i32) {
        let (n, d) = ZOOMS[m.zoom];
        let (n, d) = (n as i32, d as i32);
        (source[0] + (p.0 - rect[0]) * d / n, source[1] + (p.1 - rect[1]) * d / n)
    }

    fn views_mut(&mut self) -> impl Iterator<Item = &mut FreeView> {
        std::iter::once(&mut self.main_view).chain(self.split.as_mut().map(|s| &mut s.view)).chain(self.insets.iter_mut().map(|i| &mut i.view))
    }

    fn follows_pointer(&self) -> bool {
        self.main_view.follow == Follow::Pointer
            || self.split.is_some_and(|s| s.view.follow == Follow::Pointer)
            || self.insets.iter().any(|i| i.view.follow == Follow::Pointer)
    }

    /// The frame for `scene` in a window of `window`. `interface`: the
    /// rectangles the driver's interface covers (window pixels); `bounds`:
    /// the world's extent.
    pub fn frame(
        &mut self,
        scene: &Scene,
        window: (u32, u32),
        overhang: (u32, u32, u32, u32),
        interface: Vec<[i32; 4]>,
        bounds: Option<WorldBounds>,
    ) -> Frame {
        self.window = window;
        self.bounds = bounds;
        let base = crate::scene_cameras(scene, window, overhang);
        // Its own views show the world as the driver's do (their world
        // pixels are those views' pixels).
        let downscale = crate::world_downscale(scene);
        // The driver's main view: its first world view filling the window.
        let main = scene.views.iter().position(|v| !v.view.interface_only && (v.view.viewport.0 == 0 || v.view.viewport.1 == 0));
        self.main = main.map(|i| base[i]);
        // Where the views look (from what the last frame showed).
        if !self.locked {
            if let Some(w) = self.world_under(self.pointer).filter(|_| self.hit_follow(self.pointer) != Some(Follow::Pointer)) {
                self.cursor = w;
            }
        }
        if let Some(b) = bounds {
            self.cursor = b.clamp_point(self.cursor);
        }
        let driver = self.main.map(|m| (m.x + m.viewport.0 as i32 / 2, m.y + m.viewport.1 as i32 / 2));
        let (cursor, focus) = (self.cursor, scene.focus);
        let has_world = scene.shape.cols > 0 && scene.shape.rows > 0;
        self.world_frames = if has_world { self.world_frames.saturating_add(1) } else { 0 };
        let settled = self.world_frames > SETTLE_FRAMES;
        for v in self.views_mut() {
            match v.follow {
                Follow::Driver => v.center = driver.unwrap_or(v.center),
                Follow::Focus => v.center = focus.or(driver).unwrap_or(v.center),
                Follow::Pointer => v.center = cursor,
                // Placed where the driver first shows a world (not before: a
                // title screen has none, its empty view looks at (0, 0)).
                Follow::Free if v.center == UNPLACED || v.settling => match driver.filter(|_| has_world) {
                    Some(d) if settled => {
                        v.center = d;
                        if std::mem::take(&mut v.settling) {
                            log::info!("viewer: a free view placed at world pixel {d:?} (the middle of the driver's view)");
                        }
                    }
                    Some(d) => (v.center, v.settling) = (d, true),
                    None => {}
                },
                Follow::Free => {}
            }
        }
        self.interface = interface;
        self.art_zooms = scene.zooms.clone();
        self.shown.clear();
        self.driver_rects.clear();
        let mut frame = Frame { view_cameras: vec![None; scene.views.len()], ..Default::default() };
        let (main_rect, pane_rect) = self.panes();
        let push = |frame: &mut Frame, camera: Camera| {
            frame.cameras.push(camera);
            frame.cameras.len() as u32 - 1
        };
        // The driver's views: either the main one arranged, the others (and
        // the interface) over the viewer's own views where the driver put
        // them, or (`ui_in_pane` with a split) the driver's whole screen,
        // interface included, fitted into the main pane as one picture (its
        // one-pixel lines kept).
        let fit = self.screen_fit(main_rect);
        // (Its head: the main view, else the first filling the window, e.g.
        // the game's view hidden by a city screen.)
        let head = main.or_else(|| scene.views.iter().position(|v| v.view.viewport.0 == 0 || v.view.viewport.1 == 0)).or((!scene.views.is_empty()).then_some(0));
        let fit = fit.filter(|_| head.is_some());
        self.fit = fit.map(|z| ((main_rect[0], main_rect[1]), z));
        let mut over: Vec<(usize, Camera)> = Vec::new();
        if let (Some(zoom), Some(m)) = (fit, head) {
            let head = Camera {
                zoom,
                origin: (main_rect[0], main_rect[1]),
                viewport: ((window.0 * zoom.0).div_ceil(zoom.1), (window.1 * zoom.0).div_ceil(zoom.1)),
                zoomed_art: self.zoomed_art(zoom),
                ..base[m]
            };
            frame.view_cameras[m] = Some(push(&mut frame, head));
            if Some(m) == main {
                self.shown.push((Hit::Main, head));
            }
            for (i, camera) in base.iter().enumerate().filter(|&(i, _)| i != m) {
                let stacked = Camera { stacked: true, hairline: true, zoomed_art: self.zoomed_art(zoom), ..*camera };
                frame.view_cameras[i] = Some(push(&mut frame, stacked));
            }
        } else {
            for (i, (v, camera)) in scene.views.iter().zip(&base).enumerate() {
                if Some(i) == main {
                    let camera = self.arranged_main(camera, main_rect, overhang);
                    frame.view_cameras[i] = Some(push(&mut frame, camera));
                    self.shown.push((Hit::Main, camera));
                } else {
                    over.push((i, *camera));
                    if !v.view.interface_only {
                        self.driver_rects.push([camera.origin.0, camera.origin.1, camera.viewport.0 as i32, camera.viewport.1 as i32]);
                    }
                }
            }
        }
        // The viewer's own views, as many as the renderer draws (the second
        // pane first, then the magnifier, then the insets in order).
        let mut room = tile_render::MAX_VIEWS.saturating_sub(frame.cameras.len() + over.len());
        let mut take = |n: usize| {
            let fits = n <= room;
            if fits {
                room -= n;
            }
            fits
        };
        let pane = pane_rect.filter(|_| self.split.is_some() && take(1));
        let magnifier = self.magnifier.filter(|m| take(if m.mode == MagnifierMode::Screen { 1 } else { 2 }));
        let insets = (0..self.insets.len()).take_while(|_| take(2)).count();
        let left_out = usize::from(pane.is_none() && pane_rect.is_some() && self.split.is_some())
            + usize::from(magnifier.is_none() && self.magnifier.is_some())
            + (self.insets.len() - insets);
        if left_out != self.left_out {
            if left_out > 0 {
                log::warn!("viewer: {left_out} of its own views left out (at most {} views are drawn)", tile_render::MAX_VIEWS);
            }
            self.left_out = left_out;
        }
        self.insets_shown = insets;
        // (World only: no overlay belongs to them.)
        let frame_of = |rect: [i32; 4]| {
            let r = [rect[0] - GAP / 2, rect[1] - GAP / 2, rect[2] + GAP, rect[3] + GAP];
            // A zoomed view of nothing: a block of the background colour.
            Camera { zoom: (2, 1), origin: (r[0], r[1]), cells: false, ..Camera::new((r[2].max(1) as u32, r[3].max(1) as u32)) }
        };
        if let (Some(split), Some(rect)) = (self.split, pane) {
            let mut camera = split.view.camera(rect, overhang, bounds);
            camera.zoomed_art = self.zoomed_art(camera.zoom);
            camera.downscale = downscale;
            push(&mut frame, camera);
            self.shown.push((Hit::Pane, camera));
        }
        for (k, inset) in self.insets.iter().enumerate().take(insets) {
            push(&mut frame, frame_of(inset.rect));
            let mut camera = inset.view.camera(inset.rect, overhang, bounds);
            camera.zoomed_art = self.zoomed_art(camera.zoom);
            camera.downscale = downscale;
            push(&mut frame, camera);
            self.shown.push((Hit::Inset(k), camera));
        }
        // The driver's other views and its interface on top, as it placed
        // them (unless they are in the main pane).
        for (i, camera) in over {
            frame.view_cameras[i] = Some(push(&mut frame, camera));
        }
        if let Some(m) = magnifier {
            let (rect, source) = self.magnifier_geometry(&m);
            let zoom = ZOOMS[m.zoom];
            push(&mut frame, frame_of(rect));
            match source {
                // The window around the pointer, at the magnifier's zoom.
                Some(source) => frame.copies.push(ScreenCopy { source, at: (rect[0], rect[1]), zoom }),
                None => {
                    let center = if self.locked { Some(self.cursor) } else { self.world_under(self.pointer) };
                    let mut camera = FreeView::new(center.unwrap_or((0, 0)), m.zoom, Follow::Free).camera(rect, overhang, None);
                    camera.zoomed_art = self.zoomed_art(camera.zoom);
                    camera.downscale = downscale;
                    push(&mut frame, camera);
                }
            }
        }
        if self.follows_pointer() {
            frame.markers.push(Marker::Cursor(self.cursor));
        }
        if let (true, Some(f)) = (self.focus_arrow, scene.focus) {
            frame.markers.push(Marker::Focus(f));
        }
        frame
    }

    /// The driver's main view as the main pane shows it: following the
    /// driver it is zoomed about the driver's centre (at zoom 1, unmoved);
    /// otherwise it is placed like any arranged view. The driver's screen
    /// sprites (labels) stay on their world pixels.
    fn arranged_main(&self, camera: &Camera, rect: [i32; 4], overhang: (u32, u32, u32, u32)) -> Camera {
        let v = self.main_view;
        let unmoved = v.follow == Follow::Driver && v.zoom == ONE && (rect[0], rect[1]) == camera.origin;
        let (x, y, cull) = if unmoved {
            (camera.x, camera.y, camera.cull)
        } else {
            let placed = if v.follow == Follow::Driver { v } else { FreeView { follow: Follow::Free, ..v } };
            let c = placed.camera(rect, overhang, if v.follow == Follow::Driver { None } else { self.bounds });
            // Zoomed in on what the driver shows, its cell range still holds.
            let cull = if v.follow == Follow::Driver && v.zoom >= ONE { camera.cull } else { None };
            (c.x, c.y, cull)
        };
        let shift = (camera.x - x + camera.origin.0 - rect[0], camera.y - y + camera.origin.1 - rect[1]);
        Camera {
            x,
            y,
            zoom: ZOOMS[v.zoom],
            viewport: (rect[2].max(1) as u32, rect[3].max(1) as u32),
            origin: (rect[0], rect[1]),
            screen_shift: shift,
            cull,
            zoomed_art: self.zoomed_art(ZOOMS[v.zoom]),
            ..*camera
        }
    }

    fn hit(&self, p: (i32, i32)) -> Hit {
        if self.magnifier.is_some_and(|m| inside(self.magnifier_geometry(&m).0, p)) {
            return Hit::Magnifier;
        }
        self.hit_below_magnifier(p)
    }

    /// [`Layout::hit`] as if there were no magnifier.
    fn hit_below_magnifier(&self, p: (i32, i32)) -> Hit {
        // (The interface and the driver's other views lie over the window,
        // unless they are in the main pane.)
        if self.fit.is_none() && self.interface.iter().chain(&self.driver_rects).any(|&r| inside(r, p)) {
            return Hit::Interface;
        }
        if let Some(k) = (0..self.insets_shown.min(self.insets.len())).rev().find(|&k| inside(self.insets[k].rect, p)) {
            return Hit::Inset(k);
        }
        if self.divider().is_some_and(|r| inside(r, p)) {
            return Hit::Divider;
        }
        let (main, pane) = self.panes();
        if pane.is_some_and(|r| inside(r, p)) {
            return Hit::Pane;
        }
        if inside(main, p) && (self.main.is_some() || self.fit.is_some()) {
            return Hit::Main;
        }
        Hit::Interface
    }

    /// What the view under `p` follows, if it is an arranged one.
    fn hit_follow(&self, p: (i32, i32)) -> Option<Follow> {
        match self.hit(p) {
            Hit::Main => Some(self.main_view.follow),
            Hit::Pane => self.split.map(|s| s.view.follow),
            Hit::Inset(k) => self.insets.get(k).map(|i| i.view.follow),
            _ => None,
        }
    }

    /// The world pixel under window pixel `p`, if a world view shows one.
    fn world_under(&self, p: (i32, i32)) -> Option<(i32, i32)> {
        let (p, hit) = match self.hit(p) {
            Hit::Magnifier => {
                // Under a screen magnifier: what it magnifies.
                let m = self.magnifier?;
                let (rect, source) = self.magnifier_geometry(&m);
                let q = self.magnified(&m, rect, source?, p);
                (q, self.hit_below_magnifier(q))
            }
            h => (p, h),
        };
        let (_, camera) = self.shown.iter().find(|(h, _)| *h == hit)?;
        Some(world_at(camera, p))
    }

    /// Where world pixel `w` is in the driver's main view (its window
    /// pixels), if it shows it.
    fn in_main(&self, (wx, wy): (i32, i32)) -> Option<(i32, i32)> {
        let m = self.main?;
        let (x, y) = (wx - m.x + m.origin.0, wy - m.y + m.origin.1);
        inside([m.origin.0, m.origin.1, m.viewport.0 as i32, m.viewport.1 as i32], (x, y)).then_some((x, y))
    }

    /// Where a view made at window pixel `p` looks: the world pixel there,
    /// or (no world yet) where the driver first shows one.
    fn placed_at(&self, p: (i32, i32)) -> (i32, i32) {
        let world = self.bounds.is_some_and(|b| b.size.0 > 0 && b.size.1 > 0);
        self.world_under(p).filter(|_| world).unwrap_or(UNPLACED)
    }

    /// Where the driver gets a pointer event on world pixel `w`: `Ok(q)`,
    /// or `Err(Some(w))` if its view does not show it (or shows its
    /// interface there).
    fn route_world(&self, w: (i32, i32)) -> Result<(i32, i32), Option<(i32, i32)>> {
        match self.in_main(w) {
            Some(q) if self.interface.iter().any(|&r| inside(r, q)) => Err(Some(w)),
            Some(q) => Ok(q),
            None => Err(Some(w)),
        }
    }

    /// Whether the mouse is captured (its motion moves the world cursor).
    pub fn captured(&self) -> bool {
        self.locked
    }

    /// The window size of the last frame.
    pub fn window(&self) -> (u32, u32) {
        self.window
    }

    /// How the driver's screen (its window pixels, those of its interface)
    /// lies in the window, as of the last frame: driver pixel `s` is at
    /// window pixel `origin + s · zoom.0 / zoom.1` (1:1 at the window's
    /// corner unless it is fitted into a pane).
    pub fn screen_mapping(&self) -> ((i32, i32), (u32, u32)) {
        self.fit.unwrap_or(((0, 0), (1, 1)))
    }

    /// Where the pointer shows in the window: the world cursor in the view
    /// following it while the mouse is captured (else in the driver's main
    /// view), the pointer otherwise.
    pub fn pointer_in_window(&self) -> (i32, i32) {
        if !self.locked {
            return self.pointer;
        }
        let following = self.shown.iter().find(|(h, _)| self.follow_of(*h) == Some(Follow::Pointer));
        let camera = following.or_else(|| self.shown.iter().find(|(h, _)| *h == Hit::Main)).map(|(_, c)| c);
        camera.map_or(self.pointer, |c| {
            let (n, d) = (c.zoom.0.max(1) as i32, c.zoom.1.max(1) as i32);
            (c.origin.0 + (self.cursor.0 - c.x) * n / d, c.origin.1 + (self.cursor.1 - c.y) * n / d)
        })
    }

    /// The pointer was put at window pixel `p` by other means than the
    /// mouse (e.g. the snapping pointer, `crate::snap`): it is there now,
    /// and while the mouse is captured, over a view following the world
    /// cursor, so is the world cursor.
    pub fn place_pointer(&mut self, p: (i32, i32)) {
        self.pointer = p;
        if self.locked && self.hit_follow(p) == Some(Follow::Pointer) {
            if let Some(w) = self.world_under(p) {
                self.cursor = w;
            }
        }
    }

    /// Where a pointer event at window pixel `p` goes in the driver's
    /// window, as if the mouse were not captured (the snapping pointer
    /// points at the driver's interface while it is): `None` if it does
    /// not reach the driver's window.
    pub fn route_window(&self, p: (i32, i32)) -> Option<(i32, i32)> {
        self.route_free(p).ok()
    }

    /// Where a pointer event goes in the driver's window: `Ok(q)`, or
    /// `Err(world pixel)` if the driver does not show it, `Err(None)` if
    /// nothing.
    fn route_point(&self, p: (i32, i32)) -> Result<(i32, i32), Option<(i32, i32)>> {
        if self.locked {
            return self.route_world(self.cursor);
        }
        self.route_free(p)
    }

    /// [`Layout::route_point`] for the pointer at `p` (not the world cursor).
    fn route_free(&self, p: (i32, i32)) -> Result<(i32, i32), Option<(i32, i32)>> {
        match self.hit(p) {
            Hit::Interface => Ok(p),
            Hit::Divider => Err(None),
            // The driver's whole screen, scaled: its window pixel.
            Hit::Main if self.fit.is_some() => {
                let ((ox, oy), (n, d)) = self.fit.ok_or(None)?;
                let (n, d) = (n as i32, d as i32);
                Ok(((p.0 - ox) * d / n, (p.1 - oy) * d / n))
            }
            Hit::Magnifier => {
                let m = self.magnifier.ok_or(None)?;
                let (rect, source) = self.magnifier_geometry(&m);
                match source {
                    // The screen pixel it shows, routed as if pointed at.
                    Some(source) => {
                        let q = self.magnified(&m, rect, source, p);
                        let mut without = self.clone();
                        without.magnifier = None;
                        without.route_free(q)
                    }
                    None => {
                        let zoom = ZOOMS[m.zoom];
                        let (n, d) = (zoom.0 as i32, zoom.1 as i32);
                        let (sw, sh) = ((m.size.0 as i32 * d + n - 1) / n, (m.size.1 as i32 * d + n - 1) / n);
                        let off = ((p.0 - rect[0]) * d / n, (p.1 - rect[1]) * d / n);
                        let c = self.world_under(self.pointer).ok_or(None)?;
                        self.route_world((c.0 - sw / 2 + off.0, c.1 - sh / 2 + off.1))
                    }
                }
            }
            hit => {
                let (_, camera) = self.shown.iter().find(|(h, _)| *h == hit).ok_or(None)?;
                self.route_world(world_at(camera, p))
            }
        }
    }

    /// Relative mouse motion while the mouse is captured: moves the world
    /// cursor (in the pixels of the view following it).
    pub fn motion(&mut self, (dx, dy): (f64, f64)) -> Vec<Input> {
        if !self.locked {
            return vec![];
        }
        let zoom = self
            .shown
            .iter()
            .find(|(h, _)| self.follow_of(*h) == Some(Follow::Pointer))
            .map_or((1, 1), |(_, c)| c.zoom);
        let scale = zoom.1 as f64 / zoom.0 as f64;
        let (rx, ry) = (self.motion_rest.0 + dx * scale, self.motion_rest.1 + dy * scale);
        let (mx, my) = (rx.trunc(), ry.trunc());
        self.motion_rest = (rx - mx, ry - my);
        self.cursor = (self.cursor.0 + mx as i32, self.cursor.1 + my as i32);
        if let Some(b) = self.bounds {
            self.cursor = b.clamp_point(self.cursor);
        }
        match self.route_world(self.cursor) {
            Ok((x, y)) => vec![Input::MouseMove { x, y }],
            Err(_) => vec![],
        }
    }

    fn follow_of(&self, hit: Hit) -> Option<Follow> {
        match hit {
            Hit::Main => Some(self.main_view.follow),
            Hit::Pane => self.split.map(|s| s.view.follow),
            Hit::Inset(k) => self.insets.get(k).map(|i| i.view.follow),
            _ => None,
        }
    }

    /// Takes a window input; returns what goes to the driver.
    pub fn route(&mut self, input: Input) -> Vec<Input> {
        match input {
            Input::Key { vk, down, .. } if vk == VK_CONTROL || vk == VK_SHIFT => {
                if vk == VK_CONTROL {
                    self.ctrl = down;
                } else {
                    self.shift = down;
                }
                vec![input]
            }
            Input::Key { vk, down, .. } if self.ctrl && self.shift && self.command(vk, down) => vec![],
            Input::Key { vk, down, text } => vec![self.key(vk, down, text)],
            Input::MouseMove { x, y } => {
                if self.locked {
                    return vec![];
                }
                self.pointer = (x, y);
                if let Some(drag) = self.drag {
                    self.dragged(drag, (x, y));
                    return self.edge_change((0, 0));
                }
                let mut out = match self.route_point((x, y)) {
                    Ok((x, y)) => vec![Input::MouseMove { x, y }],
                    Err(_) => vec![],
                };
                // (After the move: a driver takes the edge for where the
                // pointer is.)
                out.extend(self.edge_change(self.edge_at((x, y))));
                out
            }
            Input::MouseButton { button, down, x, y } => self.button(button, down, (x, y)),
            Input::Wheel { delta, x, y } => {
                if self.ctrl {
                    self.zoom_at((x, y), delta.signum());
                    return vec![];
                }
                match self.route_point((x, y)) {
                    Ok((x, y)) => vec![Input::Wheel { delta, x, y }],
                    Err(_) => vec![],
                }
            }
            other => vec![other],
        }
    }

    fn button(&mut self, button: Button, down: bool, p: (i32, i32)) -> Vec<Input> {
        let b = button_index(button);
        if !down {
            self.drag = None;
            // The release goes where the press went.
            if let Some(at) = self.world_pressed[b].take() {
                let (x, y) = match self.route_point(p) {
                    Err(Some(w)) => w,
                    _ => at,
                };
                return vec![Input::WorldButton { button, down, x, y }];
            }
            if !std::mem::take(&mut self.pressed[b]) {
                return vec![];
            }
            let q = self.route_point(p).unwrap_or_else(|_| self.clamp_to_main(p));
            return vec![Input::MouseButton { button, down, x: q.0, y: q.1 }];
        }
        let hit = if self.locked { Hit::Interface } else { self.hit(p) };
        // Viewer gestures.
        match (button, hit) {
            (Button::Middle, Hit::Main | Hit::Pane | Hit::Inset(_)) => {
                if let Some(center) = self.shown.iter().find(|(h, _)| *h == hit).map(|(_, c)| {
                    let (sw, sh) = c.source_size();
                    (c.x + sw as i32 / 2, c.y + sh as i32 / 2)
                }) {
                    self.drag = Some(Drag::Pan { hit, from: p, center });
                }
                return vec![];
            }
            (Button::Left, Hit::Divider) => {
                self.drag = Some(Drag::Divider);
                return vec![];
            }
            (Button::Left, Hit::Inset(index)) if self.ctrl && self.shift => {
                let rect = self.insets[index].rect;
                let resize = p.0 >= rect[0] + rect[2] - 16 && p.1 >= rect[1] + rect[3] - 16;
                self.drag = Some(Drag::MoveInset { index, from: p, rect, resize });
                return vec![];
            }
            // Clicking into a view that follows the world cursor captures
            // the mouse there.
            (Button::Left, Hit::Main | Hit::Pane | Hit::Inset(_)) if self.follow_of(hit) == Some(Follow::Pointer) => {
                self.capture(hit, p);
                return vec![];
            }
            _ => {}
        }
        match self.route_point(p) {
            Ok(q) => {
                self.pressed[b] = true;
                vec![Input::MouseButton { button, down, x: q.0, y: q.1 }]
            }
            // A world pixel the driver's window does not take input for:
            // the driver may look there (left button) and act on it.
            Err(Some(w)) => {
                self.world_pressed[b] = Some(w);
                let press = Input::WorldButton { button, down, x: w.0, y: w.1 };
                if button == Button::Left { vec![Input::Look { x: w.0, y: w.1 }, press] } else { vec![press] }
            }
            Err(None) => vec![],
        }
    }

    /// A key that is not the viewer's: `Input::WorldKey` while the pointer
    /// is over a world pixel the driver's window does not take input for
    /// (a release follows its press), else `Input::Key`.
    fn key(&mut self, vk: u32, down: bool, text: Option<char>) -> Input {
        let world = match self.route_point(self.pointer) {
            Err(Some(w)) => Some(w),
            _ => None,
        };
        let pressed = self.world_keys.iter().position(|&k| k == vk);
        let (x, y) = match (down, pressed, world) {
            (true, _, Some(w)) => {
                if pressed.is_none() {
                    self.world_keys.push(vk);
                }
                w
            }
            // (Held into a view that takes input: stays a world key.)
            (true, Some(_), None) => self.cursor_or_pointer_world(),
            (false, Some(k), w) => {
                self.world_keys.remove(k);
                w.unwrap_or_else(|| self.cursor_or_pointer_world())
            }
            _ => return Input::Key { vk, down, text },
        };
        Input::WorldKey { vk, down, text, x, y }
    }

    /// The world pixel under the pointer (the world cursor while the mouse
    /// is captured), or (0, 0).
    fn cursor_or_pointer_world(&self) -> (i32, i32) {
        if self.locked { self.cursor } else { self.world_under(self.pointer).unwrap_or((0, 0)) }
    }

    /// Window pixel `p` moved into the driver's main view (for releases
    /// outside it).
    fn clamp_to_main(&self, p: (i32, i32)) -> (i32, i32) {
        match self.main {
            Some(m) => (
                p.0.clamp(m.origin.0, m.origin.0 + m.viewport.0 as i32 - 1),
                p.1.clamp(m.origin.1, m.origin.1 + m.viewport.1 as i32 - 1),
            ),
            None => p,
        }
    }

    fn free_view(&mut self, hit: Hit) -> Option<&mut FreeView> {
        match hit {
            Hit::Main => Some(&mut self.main_view),
            Hit::Pane => self.split.as_mut().map(|s| &mut s.view),
            Hit::Inset(k) => self.insets.get_mut(k).map(|i| &mut i.view),
            _ => None,
        }
    }

    fn dragged(&mut self, drag: Drag, p: (i32, i32)) {
        match drag {
            Drag::Pan { hit, from, center } => {
                let zoom = self.shown.iter().find(|(h, _)| *h == hit).map_or((1, 1), |(_, c)| c.zoom);
                let (n, d) = (zoom.0 as i32, zoom.1 as i32);
                if let Some(view) = self.free_view(hit) {
                    // Moved by hand: it follows nothing now.
                    view.follow = Follow::Free;
                    view.settling = false;
                    view.center = (center.0 - (p.0 - from.0) * d / n, center.1 - (p.1 - from.1) * d / n);
                }
            }
            Drag::MoveInset { index, from, rect, resize } => {
                let (dx, dy) = (p.0 - from.0, p.1 - from.1);
                if let Some(inset) = self.insets.get_mut(index) {
                    inset.rect = if resize { [rect[0], rect[1], (rect[2] + dx).max(48), (rect[3] + dy).max(32)] } else { [rect[0] + dx, rect[1] + dy, rect[2], rect[3]] };
                }
            }
            Drag::Divider => {
                if let Some(split) = &mut self.split {
                    split.fraction = match split.axis {
                        Axis::Vertical => p.0 as f32 / self.window.0.max(1) as f32,
                        Axis::Horizontal => p.1 as f32 / self.window.1.max(1) as f32,
                    }
                    .clamp(0.1, 0.9);
                }
            }
        }
    }

    /// Zooms what is under `p` one step in (`step` > 0) or out.
    fn zoom_at(&mut self, p: (i32, i32), step: i32) {
        let next = |z: usize| (z as i32 + step).clamp(0, ZOOMS.len() as i32 - 1) as usize;
        let hit = if self.locked { self.shown.iter().map(|(h, _)| *h).find(|&h| self.follow_of(h) == Some(Follow::Pointer)) } else { Some(self.hit(p)) };
        match hit {
            Some(Hit::Magnifier) => {
                if let Some(m) = &mut self.magnifier {
                    m.zoom = next(m.zoom).max(ONE);
                }
            }
            Some(hit @ (Hit::Main | Hit::Pane | Hit::Inset(_))) => {
                if let Some(v) = self.free_view(hit) {
                    v.zoom = next(v.zoom);
                }
            }
            _ => self.main_view.zoom = next(self.main_view.zoom),
        }
    }

    /// A Ctrl+Shift command key; whether it was one.
    fn command(&mut self, vk: u32, down: bool) -> bool {
        const KEYS: [u32; 19] = [0x53, 0x49, 0x58, 0x4D, 0x30, 0xBB, 0xBD, 0x6B, 0x6D, 0x25, 0x26, 0x27, 0x28, 0x46, 0x20, 0x41, 0x60, 0x54, 0x55];
        if !KEYS.contains(&vk) {
            return false;
        }
        if !down {
            return true;
        }
        let p = self.pointer;
        let hit = self.hit(p);
        match vk {
            // S: split off → side by side → stacked → off.
            0x53 => {
                self.split = match self.split {
                    None => Some(Split { axis: Axis::Vertical, fraction: 0.5, view: FreeView::new(self.placed_at(p), ONE, Follow::Free) }),
                    Some(s) if s.axis == Axis::Vertical => Some(Split { axis: Axis::Horizontal, ..s }),
                    Some(_) => None,
                }
            }
            // I: an inset at the pointer.
            0x49 if self.insets.len() < MAX_INSETS => {
                let (w, h) = (self.window.0 as i32 / 4, self.window.1 as i32 / 4);
                let view = FreeView::new(self.placed_at(p), ONE + 2, Follow::Free);
                self.insets.push(Inset { rect: [p.0 - w / 2, p.1 - h / 2, w.max(48), h.max(32)], view });
            }
            // X: remove the inset under the pointer (else the last one).
            0x58 => {
                let k = match hit {
                    Hit::Inset(k) => Some(k),
                    _ => self.insets.len().checked_sub(1),
                };
                if let Some(k) = k {
                    self.insets.remove(k);
                }
            }
            // F: what the view under the pointer follows.
            0x46 => {
                if let Some(v) = self.free_view(if hit == Hit::Interface { Hit::Main } else { hit }) {
                    v.follow = v.follow.next();
                }
            }
            // Space: release the mouse; or take it, over a view following
            // the world cursor (as a click into it does).
            0x20 => {
                if self.locked {
                    self.release("Ctrl+Shift+Space");
                } else if self.hit_follow(p) == Some(Follow::Pointer) {
                    self.capture(hit, p);
                } else {
                    log::info!("viewer: mouse stays free (no view under the pointer follows the world cursor)");
                }
            }
            // A: the arrow at the point of interest.
            0x41 => self.focus_arrow = !self.focus_arrow,
            // T: text at the zoomed views' resolution, or scaled.
            0x54 => self.sharp_text = !self.sharp_text,
            // U: the driver's screen fitted into the main pane, or over the window.
            0x55 => self.ui_in_pane = !self.ui_in_pane,
            // M: magnifier off → everything → world → off.
            0x4D => {
                self.magnifier = match self.magnifier {
                    None => Some(Self::MAGNIFIER),
                    Some(m) if m.mode == MagnifierMode::Screen => Some(Magnifier { mode: MagnifierMode::World, ..m }),
                    Some(_) => None,
                }
            }
            // 0: zoom 1 for what is under the pointer.
            0x30 | 0x60 => match hit {
                Hit::Magnifier => {}
                hit @ (Hit::Pane | Hit::Inset(_)) => {
                    if let Some(v) = self.free_view(hit) {
                        v.zoom = ONE;
                    }
                }
                _ => self.main_view.zoom = ONE,
            },
            0xBB | 0x6B => self.zoom_at(p, 1),
            0xBD | 0x6D => self.zoom_at(p, -1),
            // Arrows: pan what is under the pointer by an eighth of it.
            0x25..=0x28 => {
                let hit = if hit == Hit::Interface { Hit::Main } else { hit };
                let size = self.shown.iter().find(|(h, _)| *h == hit).map(|(_, c)| c.source_size());
                let center = self.shown.iter().find(|(h, _)| *h == hit).map(|(_, c)| (c.x + c.source_size().0 as i32 / 2, c.y + c.source_size().1 as i32 / 2));
                if let (Some(v), Some((w, h)), Some(center)) = (self.free_view(hit), size, center) {
                    let (dx, dy) = match vk {
                        0x25 => (-(w as i32) / 8, 0),
                        0x26 => (0, -(h as i32) / 8),
                        0x27 => (w as i32 / 8, 0),
                        _ => (0, h as i32 / 8),
                    };
                    v.follow = Follow::Free;
                    v.settling = false;
                    v.center = (center.0 + dx, center.1 + dy);
                }
            }
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scene::{View, ViewState};

    fn scene() -> Scene {
        let main = View { camera_x: 1000, camera_y: 500, zoom: (1, 1), ..Default::default() };
        let ui = View { interface_only: true, ..Default::default() };
        Scene {
            views: vec![ViewState { id: 0, view: main, overlay: vec![] }, ViewState { id: 255, view: ui, overlay: vec![] }],
            focus: Some((3000, 2000)),
            shape: scene::WorldShape { cols: 62, rows: 125, wrap_x: true, wrap_y: false },
            ..Default::default()
        }
    }

    const BOUNDS: Option<WorldBounds> = Some(WorldBounds { size: (8000, 4000), wrap: (true, false) });

    fn frame(l: &mut Layout, interface: Vec<[i32; 4]>) -> Frame {
        l.frame(&scene(), (800, 600), (0, 0, 0, 0), interface, BOUNDS)
    }

    fn click(l: &mut Layout, p: (i32, i32)) -> Vec<Input> {
        let mut out = l.route(Input::MouseButton { button: Button::Left, down: true, x: p.0, y: p.1 });
        out.extend(l.route(Input::MouseButton { button: Button::Left, down: false, x: p.0, y: p.1 }));
        out
    }

    fn ctrl_shift(l: &mut Layout, vk: u32) -> Vec<Input> {
        l.route(Input::Key { vk: VK_CONTROL, down: true, text: None });
        l.route(Input::Key { vk: VK_SHIFT, down: true, text: None });
        let out = l.route(Input::Key { vk, down: true, text: None });
        l.route(Input::Key { vk: VK_SHIFT, down: false, text: None });
        l.route(Input::Key { vk: VK_CONTROL, down: false, text: None });
        out
    }

    #[test]
    fn unzoomed_main_view_passes_input_through() {
        let mut l = Layout::default();
        let f = frame(&mut l, vec![]);
        assert_eq!(f.cameras.len(), 2);
        assert_eq!((f.cameras[0].x, f.cameras[0].y, f.cameras[0].screen_shift), (1000, 500, (0, 0)));
        assert_eq!(click(&mut l, (10, 20)), [
            Input::MouseButton { button: Button::Left, down: true, x: 10, y: 20 },
            Input::MouseButton { button: Button::Left, down: false, x: 10, y: 20 }
        ]);
    }

    #[test]
    fn zoomed_art_for_every_zoom_shows_in_every_zoomed_view() {
        // Text drawn from outlines: art for every zoom (0, 0); for one zoom
        // only, other zooms show the 1:1 twins; `text=pixel` none.
        for (zooms, spec, want) in [(vec![(0, 0)], "main_zoom=3/2", true), (vec![(2, 1)], "main_zoom=3/2", false), (vec![(2, 1)], "main_zoom=2", true), (vec![(0, 0)], "main_zoom=2,text=pixel", false)] {
            let mut l = Layout::default();
            l.configure(spec).unwrap();
            let s = Scene { zooms, ..scene() };
            let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![[0, 500, 800, 100]], BOUNDS);
            assert_eq!(f.cameras[0].zoomed_art, want, "{:?} {spec}", s.zooms);
        }
    }

    #[test]
    fn zoomed_main_view_maps_clicks_to_the_same_world_pixel() {
        let mut l = Layout::default();
        l.configure("main_zoom=2").unwrap();
        let f = frame(&mut l, vec![[0, 500, 800, 100]]);
        // Centred: world (1000 + 200, 500 + 150) at the window's top-left.
        assert_eq!((f.cameras[0].x, f.cameras[0].y, f.cameras[0].zoom), (1200, 650, (2, 1)));
        // Labels follow: a sprite the driver put at window (400, 300) (world
        // 1400, 800) stays there.
        assert_eq!(f.cameras[0].screen_shift, (-200, -150));
        // Window (100, 40) shows world (1250, 670): the driver's (250, 170).
        assert_eq!(click(&mut l, (100, 40))[0], Input::MouseButton { button: Button::Left, down: true, x: 250, y: 170 });
        // The interface stays where it is.
        assert_eq!(click(&mut l, (100, 550))[0], Input::MouseButton { button: Button::Left, down: true, x: 100, y: 550 });
    }

    #[test]
    fn split_pane_looks_at_what_the_driver_does_not_show() {
        let mut l = Layout::default();
        frame(&mut l, vec![]);
        assert!(ctrl_shift(&mut l, 0x53).is_empty());
        l.split.as_mut().unwrap().view = FreeView { center: (5000, 2000), ..FreeView::new((0, 0), ONE, Follow::Free) };
        let f = frame(&mut l, vec![]);
        assert_eq!(f.cameras.len(), 3);
        assert_eq!(f.cameras[0].viewport, (400, 600));
        assert_eq!(f.cameras[1].origin, (404, 0));
        // A click there: the driver is asked to look, and told of the
        // press and release on that world pixel.
        let out = click(&mut l, (600, 300));
        assert!(
            matches!(out[..], [
                Input::Look { x, y },
                Input::WorldButton { button: Button::Left, down: true, x: x2, y: y2 },
                Input::WorldButton { button: Button::Left, down: false, x: x3, y: y3 }
            ] if (x, y) == (x2, y2) && (x, y) == (x3, y3)),
            "{out:?}"
        );
        // Dragging the divider moves it, nothing reaches the driver.
        assert!(l.route(Input::MouseButton { button: Button::Left, down: true, x: 401, y: 10 }).is_empty());
        assert!(l.route(Input::MouseMove { x: 200, y: 10 }).is_empty());
        assert!(l.route(Input::MouseButton { button: Button::Left, down: false, x: 200, y: 10 }).is_empty());
        assert!((l.split.unwrap().fraction - 0.25).abs() < 0.01);
    }

    #[test]
    fn screen_magnifier_copies_around_the_pointer() {
        let mut l = Layout::default();
        l.configure("magnifier=screen,magnifier_zoom=2").unwrap();
        l.magnifier.as_mut().unwrap().size = (200, 100);
        l.route(Input::MouseMove { x: 300, y: 300 });
        let f = frame(&mut l, vec![]);
        // Beside what it magnifies, its frame clear of it.
        assert_eq!(f.copies, [ScreenCopy { source: [250, 275, 100, 50], at: (354, 250), zoom: (2, 1) }]);
        // A click on the magnifier goes to what it shows.
        assert_eq!(click(&mut l, (364, 260))[0], Input::MouseButton { button: Button::Left, down: true, x: 255, y: 280 });
    }

    #[test]
    fn the_screen_magnifier_never_magnifies_itself() {
        // At 3/1 (what it shows reaches past where it used to sit, 24 pixels
        // right of the pointer), anywhere in the window: what it copies is in
        // the window and clear of it and its frame.
        let mut l = Layout::default();
        l.configure("magnifier=screen").unwrap();
        for p in [(300, 250), (10, 10), (790, 590), (400, 300), (560, 20), (230, 500)] {
            l.route(Input::MouseMove { x: p.0, y: p.1 });
            let f = frame(&mut l, vec![]);
            let [c] = f.copies[..] else { panic!("{:?}", f.copies) };
            let [sx, sy, sw, sh] = c.source;
            assert!(sx >= 0 && sy >= 0 && sx + sw <= 800 && sy + sh <= 600, "{p:?}: {c:?}");
            let (w, h) = (240 / 3, 160_u32.div_ceil(3) as i32);
            assert_eq!((sw, sh), (w, h));
            let frame_rect = [c.at.0 - GAP / 2, c.at.1 - GAP / 2, 240 + GAP, 160 + GAP];
            let overlap = sx < frame_rect[0] + frame_rect[2] && frame_rect[0] < sx + sw && sy < frame_rect[1] + frame_rect[3] && frame_rect[1] < sy + sh;
            assert!(!overlap, "{p:?}: {c:?}");
            // Around the pointer where the window allows.
            assert!(inside([sx - 1, sy - 1, sw + 2, sh + 2], p), "{p:?}: {c:?}");
        }
    }

    #[test]
    fn focus_pane_and_mouse_locked_pane() {
        // The left half shows the active unit, the right one keeps the
        // world cursor in its middle.
        let mut l = Layout::default();
        l.configure("split=vertical,main=focus,pane=pointer").unwrap();
        frame(&mut l, vec![]);
        let f = frame(&mut l, vec![]);
        let (sw, sh) = f.cameras[0].source_size();
        assert_eq!((f.cameras[0].x + sw as i32 / 2, f.cameras[0].y + sh as i32 / 2), (3000, 2000));
        // Click into the right half: the mouse is captured there.
        assert!(click(&mut l, (600, 300)).is_empty());
        assert_eq!(l.take_grab(), Some(true));
        let before = frame(&mut l, vec![]);
        let cursor = l.cursor;
        assert!(before.markers.contains(&Marker::Cursor(cursor)));
        // Moving the mouse moves the world cursor, and the right half with
        // it (it stays in the middle); the left half stays.
        l.motion((30.0, -12.0));
        let after = frame(&mut l, vec![]);
        assert_eq!(l.cursor, (cursor.0 + 30, cursor.1 - 12));
        assert_eq!((after.cameras[1].x - before.cameras[1].x, after.cameras[1].y - before.cameras[1].y), (30, -12));
        assert_eq!(after.cameras[0], before.cameras[0]);
        // At the top of the world (no vertical wrap) the view stops, the
        // cursor goes on to the edge.
        l.motion((0.0, -100_000.0));
        let top = frame(&mut l, vec![]);
        assert_eq!(l.cursor.1, 0);
        assert_eq!(top.cameras[1].y, 0);
        // Released with Ctrl+Shift+Space.
        assert!(ctrl_shift(&mut l, 0x20).is_empty());
        assert_eq!(l.take_grab(), Some(false));
    }

    #[test]
    fn input_before_the_first_frame_is_fine() {
        // Configured split, the window size not known yet.
        let mut l = Layout::default();
        l.configure("split=horizontal,main=focus,pane=pointer").unwrap();
        click(&mut l, (5, 5));
        l.route(Input::MouseMove { x: 3, y: 3 });
        frame(&mut l, vec![]);
    }

    #[test]
    fn the_drivers_screen_fits_into_the_main_pane() {
        // ui=pane: the driver's map and interface as one picture at half size
        // in the left half; a click there goes to the driver's window pixel.
        let mut l = Layout::default();
        l.configure("split=vertical,ui=pane").unwrap();
        frame(&mut l, vec![[0, 500, 800, 100]]);
        let f = frame(&mut l, vec![[0, 500, 800, 100]]);
        assert_eq!((f.cameras[0].zoom, f.cameras[0].origin, f.cameras[0].viewport), ((1, 2), (0, 0), (400, 300)));
        assert!(f.cameras[1].stacked);
        assert_eq!(f.view_cameras, [Some(0), Some(1)]);
        assert_eq!(click(&mut l, (100, 270))[0], Input::MouseButton { button: Button::Left, down: true, x: 200, y: 540 });
    }

    /// A split whose pane (right half) shows world (5000, 2000) around its
    /// middle, which the driver's main view does not show.
    fn free_pane() -> Layout {
        let mut l = Layout::default();
        l.configure("split=vertical,pane=free").unwrap();
        frame(&mut l, vec![]);
        l.split.as_mut().unwrap().view = FreeView { center: (5000, 2000), ..FreeView::new((0, 0), ONE, Follow::Free) };
        frame(&mut l, vec![]);
        l
    }

    #[test]
    fn buttons_on_a_free_pane_go_to_the_world() {
        let mut l = free_pane();
        // The pane is 396 wide from x 404, the camera centred on 5000.
        let (wx, wy) = (5000 - 198 + (600 - 404), 2000 - 300 + 300);
        let right = |down| Input::MouseButton { button: Button::Right, down, x: 600, y: 300 };
        assert_eq!(l.route(right(true)), [Input::WorldButton { button: Button::Right, down: true, x: wx, y: wy }]);
        // The release goes where the press went, even off the pane.
        assert_eq!(l.route(Input::MouseButton { button: Button::Right, down: false, x: 100, y: 300 }), [
            Input::WorldButton { button: Button::Right, down: false, x: wx, y: wy }
        ]);
        // On the main pane, buttons still go to the driver's window.
        assert_eq!(click(&mut l, (100, 100))[0], Input::MouseButton { button: Button::Left, down: true, x: 100, y: 100 });
    }

    #[test]
    fn keys_over_a_free_pane_go_to_the_world() {
        let mut l = free_pane();
        let key = |down| Input::Key { vk: 0x66, down, text: None };
        // Over the main pane: a plain key.
        l.route(Input::MouseMove { x: 100, y: 100 });
        assert_eq!(l.route(key(true)), [key(true)]);
        assert_eq!(l.route(key(false)), [key(false)]);
        // Over the free pane: a world key at the pointer's world pixel.
        l.route(Input::MouseMove { x: 600, y: 300 });
        let (wx, wy) = (5000 - 198 + (600 - 404), 2000);
        assert_eq!(l.route(key(true)), [Input::WorldKey { vk: 0x66, down: true, text: None, x: wx, y: wy }]);
        // Released over the main pane: still a world key (its press was).
        l.route(Input::MouseMove { x: 100, y: 100 });
        assert!(matches!(l.route(key(false))[..], [Input::WorldKey { vk: 0x66, down: false, .. }]));
        // Ctrl and Shift stay plain keys; viewer commands stay the viewer's.
        l.route(Input::MouseMove { x: 600, y: 300 });
        assert_eq!(l.route(Input::Key { vk: VK_CONTROL, down: true, text: None }), [Input::Key { vk: VK_CONTROL, down: true, text: None }]);
        l.route(Input::Key { vk: VK_SHIFT, down: true, text: None });
        assert!(l.route(Input::Key { vk: 0x41, down: true, text: None }).is_empty());
    }

    #[test]
    fn world_the_driver_covers_with_its_interface_goes_to_the_world() {
        // ui=pane: the driver's screen in the left half, all of it covered
        // by its interface (e.g. a full-screen menu); the right half shows
        // the same world the driver's main view does.
        let mut l = Layout::default();
        l.configure("split=vertical,ui=pane,pane=driver").unwrap();
        let cover = vec![[0, 0, 800, 600]];
        frame(&mut l, cover.clone());
        let f = frame(&mut l, cover);
        let pane = f.cameras.iter().find(|c| c.origin == (404, 0)).copied().unwrap();
        let out = l.route(Input::MouseButton { button: Button::Right, down: true, x: 500, y: 200 });
        assert_eq!(out, [Input::WorldButton { button: Button::Right, down: true, x: pane.x + 96, y: pane.y + 200 }]);
        // The fitted screen still takes its clicks.
        assert!(matches!(click(&mut l, (100, 100))[0], Input::MouseButton { .. }));
    }

    #[test]
    fn the_pointer_at_the_window_edge_is_at_the_drivers_edge_only_unmoved() {
        let mut l = Layout::default();
        frame(&mut l, vec![[0, 560, 800, 40]]);
        // The middle: no edge; the right edge, its outermost pixel and 3 in.
        assert_eq!(l.route(Input::MouseMove { x: 400, y: 300 }), vec![Input::MouseMove { x: 400, y: 300 }]);
        assert_eq!(l.route(Input::MouseMove { x: 799, y: 300 }), vec![Input::MouseMove { x: 799, y: 300 }, Input::Edge { x: 1, y: 0 }]);
        assert_eq!(l.route(Input::MouseMove { x: 796, y: 300 }), vec![Input::MouseMove { x: 796, y: 300 }, Input::Edge { x: 4, y: 0 }]);
        // Unchanged: not sent again; a corner, over the driver's interface.
        assert_eq!(l.route(Input::MouseMove { x: 796, y: 300 }), vec![Input::MouseMove { x: 796, y: 300 }]);
        assert_eq!(l.route(Input::MouseMove { x: 0, y: 599 }), vec![Input::MouseMove { x: 0, y: 599 }, Input::Edge { x: -1, y: 1 }]);
        assert_eq!(l.pointer_gone(), vec![Input::Edge { x: 0, y: 0 }]);
        assert_eq!(l.pointer_gone(), vec![]);
        // Zoomed in, the window's edge is not the driver's.
        l.route(Input::MouseMove { x: 799, y: 300 });
        l.main_view.zoom = ONE + 2;
        frame(&mut l, vec![]);
        assert_eq!(l.route(Input::MouseMove { x: 798, y: 300 }).last(), Some(&Input::Edge { x: 0, y: 0 }));
        // Split: the main pane's outer edges are the driver's, the divider is
        // not, the second pane is a view of the viewer's own.
        l.main_view.zoom = ONE;
        l.split = Some(Split { axis: Axis::Vertical, fraction: 0.5, view: FreeView::new((0, 0), ONE, Follow::Free) });
        frame(&mut l, vec![]);
        assert_eq!(l.route(Input::MouseMove { x: 2, y: 300 }).last(), Some(&Input::Edge { x: -3, y: 0 }));
        assert_eq!(l.route(Input::MouseMove { x: 390, y: 300 }).last(), Some(&Input::Edge { x: 0, y: 0 }));
        assert!(!l.route(Input::MouseMove { x: 799, y: 300 }).iter().any(|i| matches!(i, Input::Edge { .. })));
        // The driver's screen fitted into the pane: none.
        l.ui_in_pane = true;
        frame(&mut l, vec![]);
        assert!(!l.route(Input::MouseMove { x: 0, y: 300 }).iter().any(|i| matches!(i, Input::Edge { x, .. } if *x != 0)));
    }

    #[test]
    fn config_rejects_nonsense() {
        let mut l = Layout::default();
        assert!(l.configure("split=diagonal").is_err());
        assert!(l.configure("pane_zoom=5/7").is_err());
        assert!(l.configure("arrow=on,magnifier=world").is_ok());
        assert!(l.focus_arrow && l.magnifier.unwrap().mode == MagnifierMode::World);
    }

    #[test]
    fn the_viewers_most_views_fit_the_renderer() {
        // A split, three insets and a world magnifier (two views each), the
        // driver's screen in the pane or over the window: every view drawn.
        for ui in ["pane", "window"] {
            let mut l = Layout::default();
            l.configure(&format!("split=vertical,ui={ui},magnifier=world")).unwrap();
            frame(&mut l, vec![]);
            for p in [(100, 100), (500, 200), (300, 400)] {
                l.route(Input::MouseMove { x: p.0, y: p.1 });
                ctrl_shift(&mut l, 0x49);
            }
            assert_eq!(l.insets.len(), MAX_INSETS);
            let f = frame(&mut l, vec![]);
            assert_eq!(f.cameras.len(), 2 + 1 + 2 * MAX_INSETS + 2, "ui={ui}");
            assert!(f.cameras.len() <= tile_render::MAX_VIEWS);
            // The interface on top of the viewer's own views (bar the
            // magnifier), or in the pane.
            let interface = f.view_cameras[1].unwrap() as usize;
            assert_eq!(interface, if ui == "pane" { 1 } else { f.cameras.len() - 3 }, "ui={ui}");
        }
        // A driver with many views: the viewer leaves out what does not
        // fit (insets last), never more than the renderer draws.
        let mut s = scene();
        for id in 1..10 {
            let v = View { camera_x: 100 * id as i32, origin: (10 * id as i32, 10), viewport: (40, 30), ..Default::default() };
            s.views.insert(id as usize, ViewState { id, view: v, overlay: vec![] });
        }
        let mut l = Layout::default();
        l.configure("split=vertical,magnifier=world").unwrap();
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        for p in [(100, 300), (500, 200), (300, 400)] {
            l.route(Input::MouseMove { x: p.0, y: p.1 });
            ctrl_shift(&mut l, 0x49);
        }
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.cameras.len(), tile_render::MAX_VIEWS);
        assert!(f.view_cameras.iter().all(Option::is_some));
        assert_eq!(l.insets_shown, 1);
        // An inset left out takes no clicks.
        let k = (0..3).find(|&k| !inside(l.insets[0].rect, (l.insets[k].rect[0] + 5, l.insets[k].rect[1] + 5))).unwrap();
        let r = l.insets[k].rect;
        assert_ne!(l.hit((r[0] + 5, r[1] + 5)), Hit::Inset(k));
    }

    /// The city screen open (as the bridge shows it): the game's view hidden
    /// (interface only), the city's surroundings in a view of their own
    /// within the right half, the interface over all.
    fn city_screen() -> Scene {
        let game = View { camera_x: 1000, camera_y: 500, interface_only: true, ..Default::default() };
        let city = View { camera_x: 1964, camera_y: 748, viewport: (300, 200), origin: (450, 150), ..Default::default() };
        let ui = View { interface_only: true, ..Default::default() };
        Scene {
            views: vec![ViewState { id: 0, view: game, overlay: vec![] }, ViewState { id: 1, view: city, overlay: vec![] }, ViewState { id: 255, view: ui, overlay: vec![] }],
            ..Default::default()
        }
    }

    #[test]
    fn a_city_screens_map_stays_over_the_pane() {
        // With a split and the interface over the window, the city screen's
        // map keeps its camera, over the pane, and takes its clicks itself.
        let mut l = Layout::default();
        l.configure("split=vertical,pane=free").unwrap();
        let interface = vec![[0, 0, 800, 150], [0, 350, 800, 250]];
        l.frame(&scene(), (800, 600), (0, 0, 0, 0), interface.clone(), BOUNDS);
        let f = l.frame(&city_screen(), (800, 600), (0, 0, 0, 0), interface, BOUNDS);
        let city = f.view_cameras[1].unwrap() as usize;
        let pane = f.cameras.iter().position(|c| c.origin == (404, 0)).unwrap();
        assert!(city > pane, "{:?}", f.cameras);
        assert_eq!((f.cameras[city].x, f.cameras[city].y, f.cameras[city].origin), (1964, 748, (450, 150)));
        assert_eq!(f.view_cameras[2], Some(f.cameras.len() as u32 - 1));
        assert_eq!(click(&mut l, (600, 250))[0], Input::MouseButton { button: Button::Left, down: true, x: 600, y: 250 });
        // Beside it the pane still shows the world.
        assert!(matches!(click(&mut l, (780, 250))[..], [Input::Look { .. }, ..]));
    }

    #[test]
    fn a_city_screen_fits_into_the_main_pane() {
        // ui=pane: the whole city screen (its map too) at half size in the
        // left half, its one-pixel lines kept; clicks go to its window pixel.
        let mut l = Layout::default();
        l.configure("split=vertical,ui=pane").unwrap();
        l.frame(&city_screen(), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let f = l.frame(&city_screen(), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!((f.cameras[0].zoom, f.cameras[0].origin, f.cameras[0].cells), ((1, 2), (0, 0), false));
        assert!(f.cameras[1..3].iter().all(|c| c.stacked && c.hairline));
        assert_eq!(f.view_cameras, [Some(0), Some(1), Some(2)]);
        assert_eq!(click(&mut l, (300, 120))[0], Input::MouseButton { button: Button::Left, down: true, x: 600, y: 240 });
    }

    #[test]
    fn a_free_pane_starts_where_the_driver_shows_the_world() {
        // Configured before the game shows a map (a title screen: no world,
        // an empty game view at (0, 0) and the interface), the pane is
        // placed once it does, at the middle of the driver's view (not at
        // the world's corner), and follows it while it settles (a game
        // loading a save moves it about).
        let mut l = Layout::default();
        l.configure("split=vertical,ui=pane,pane=free").unwrap();
        let empty = |id, interface_only| ViewState { id, view: View { interface_only, ..Default::default() }, overlay: vec![] };
        let title = Scene { views: vec![empty(0, false), empty(255, true)], ..Default::default() };
        for _ in 0..3 {
            l.frame(&title, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        }
        assert_eq!(l.split.unwrap().view.center, UNPLACED);
        let at = |x: i32, y: i32| {
            let mut s = scene();
            (s.views[0].view.camera_x, s.views[0].view.camera_y) = (x - 400, y - 300);
            s
        };
        l.frame(&at(5000, 3000), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.split.unwrap().view.center, (5000, 3000));
        let f = frame(&mut l, vec![]);
        assert_eq!(l.split.unwrap().view.center, (1400, 800));
        let pane = f.cameras.iter().find(|c| c.origin == (404, 0)).unwrap();
        assert_eq!((pane.x + 198, pane.y + 300), (1400, 800));
        // Then it stays put.
        for _ in 0..SETTLE_FRAMES {
            frame(&mut l, vec![]);
        }
        l.frame(&at(2000, 1000), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.split.unwrap().view.center, (1400, 800));
        // Moved by hand while settling, it stays where it was moved.
        let mut l = Layout::default();
        l.configure("split=vertical,pane=free").unwrap();
        frame(&mut l, vec![]);
        l.route(Input::MouseButton { button: Button::Middle, down: true, x: 600, y: 300 });
        l.route(Input::MouseMove { x: 500, y: 300 });
        l.route(Input::MouseButton { button: Button::Middle, down: false, x: 500, y: 300 });
        let moved = l.split.unwrap().view.center;
        assert_eq!(moved, (1500, 800));
        l.frame(&at(2000, 1000), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.split.unwrap().view.center, moved);
    }

    #[test]
    fn ctrl_shift_space_releases_and_never_captures_by_accident() {
        // Free, over a view that does not follow the world cursor: nothing.
        let mut l = Layout::default();
        l.configure("split=vertical,pane=pointer").unwrap();
        frame(&mut l, vec![]);
        l.route(Input::MouseMove { x: 100, y: 300 });
        ctrl_shift(&mut l, 0x20);
        assert!(!l.locked);
        assert_eq!(l.take_grab(), None);
        assert_eq!(click(&mut l, (100, 300))[0], Input::MouseButton { button: Button::Left, down: true, x: 100, y: 300 });
        // Over the pane following it: taken; again: released.
        l.route(Input::MouseMove { x: 600, y: 300 });
        ctrl_shift(&mut l, 0x20);
        assert!(l.locked);
        assert_eq!(l.take_grab(), Some(true));
        ctrl_shift(&mut l, 0x20);
        assert!(!l.locked);
        assert_eq!(l.take_grab(), Some(false));
        // Captured by a click, the window losing the mouse releases it.
        click(&mut l, (600, 300));
        assert!(l.locked);
        l.released();
        assert!(!l.locked);
    }

    #[test]
    fn the_viewers_views_show_the_world_as_small_as_the_drivers() {
        // A zoomed-out game view (the world at half size): the main view
        // zoomed 2/1, a pane, an inset and a world magnifier all draw the
        // half-size world, in its pixels.
        let mut s = scene();
        s.views[0].view.downscale = 2;
        assert_eq!(crate::world_downscale(&s), 2);
        let mut l = Layout::default();
        l.configure("split=vertical,main_zoom=2,magnifier=world").unwrap();
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        l.route(Input::MouseMove { x: 100, y: 100 });
        ctrl_shift(&mut l, 0x49);
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let world: Vec<_> = f.cameras.iter().filter(|c| c.cells).collect();
        assert_eq!(world.len(), 4, "{:?}", f.cameras);
        assert!(world.iter().all(|c| c.downscale == 2), "{world:?}");
        assert_eq!(f.cameras[0].zoom, (2, 1));
        // The interface stays as it is.
        assert_eq!(f.cameras[f.view_cameras[1].unwrap() as usize].downscale, 1);
    }
}
