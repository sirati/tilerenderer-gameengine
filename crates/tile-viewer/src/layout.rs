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
//! world pixels the driver's view does not show become [`Input::Look`].
//!
//! Controls (Ctrl+Shift with a key, so they do not collide with a game's):
//! S cycles the split (off, side by side, stacked), I adds an inset at the
//! pointer, X removes the inset under it, F cycles what the view under the
//! pointer follows, Space releases (or takes) the mouse, M cycles the
//! magnifier (off, everything on screen, the world only), A toggles the
//! arrow at the point of interest, +/− and Ctrl+wheel zoom what is under
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
}

impl FreeView {
    pub fn new(center: (i32, i32), zoom: usize, follow: Follow) -> Self {
        Self { center, zoom, follow }
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
        let (cx, cy) = match bounds {
            Some(b) => (b.clamp_center(0, self.center.0, sw), b.clamp_center(1, self.center.1, sh)),
            None => self.center,
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
    window: (u32, u32),
    pointer: (i32, i32),
    /// The world cursor, and the motion not yet a whole world pixel.
    cursor: (i32, i32),
    motion_rest: (f64, f64),
    /// Whether the mouse is captured (moving the world cursor), and a
    /// change of that for the window to make.
    locked: bool,
    grab: Option<bool>,
    ctrl: bool,
    shift: bool,
    drag: Option<Drag>,
    /// Per button: whether its press went to the driver.
    pressed: [bool; 3],
    /// From the last frame: the driver's main view (scene camera, unzoomed),
    /// where the world is shown, the interface rectangles, the world.
    main: Option<Camera>,
    shown: Vec<(Hit, Camera)>,
    interface: Vec<[i32; 4]>,
    bounds: Option<WorldBounds>,
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
            ctrl: false,
            shift: false,
            drag: None,
            pressed: [false; 3],
            main: None,
            shown: Vec::new(),
            interface: Vec::new(),
            bounds: None,
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
    /// (screen, world, off), `magnifier_zoom`, `arrow` (on, off).
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
                _ => return Err(format!("unknown key {key:?}")),
            }
        }
        Ok(())
    }

    const MAGNIFIER: Magnifier = Magnifier { mode: MagnifierMode::Screen, zoom: ONE + 3, size: (240, 160) };

    /// A change of mouse capture for the window to make (true: capture and
    /// hide the pointer, report motion to [`Layout::motion`]).
    pub fn take_grab(&mut self) -> Option<bool> {
        self.grab.take()
    }

    /// The mouse was let go (e.g. the window lost focus).
    pub fn released(&mut self) {
        self.locked = false;
    }

    /// The main pane's rectangle, and the second pane's if split.
    fn panes(&self) -> ([i32; 4], Option<[i32; 4]>) {
        let (w, h) = (self.window.0 as i32, self.window.1 as i32);
        match self.split {
            None => ([0, 0, w, h], None),
            Some(s) => match s.axis {
                Axis::Vertical => {
                    let a = ((w as f32 * s.fraction) as i32).clamp(GAP, w - GAP);
                    ([0, 0, a, h], Some([a + GAP, 0, w - a - GAP, h]))
                }
                Axis::Horizontal => {
                    let a = ((h as f32 * s.fraction) as i32).clamp(GAP, h - GAP);
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

    fn magnifier_rect(&self, m: &Magnifier) -> [i32; 4] {
        // Beside the pointer, kept inside the window.
        let (w, h) = (m.size.0 as i32, m.size.1 as i32);
        let (ww, wh) = (self.window.0 as i32, self.window.1 as i32);
        let x = if self.pointer.0 + 24 + w <= ww { self.pointer.0 + 24 } else { self.pointer.0 - 24 - w };
        let y = (self.pointer.1 - h / 2).clamp(0, (wh - h).max(0));
        [x.clamp(0, (ww - w).max(0)), y, w, h]
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
        for v in self.views_mut() {
            match v.follow {
                Follow::Driver => v.center = driver.unwrap_or(v.center),
                Follow::Focus => v.center = focus.or(driver).unwrap_or(v.center),
                Follow::Pointer => v.center = cursor,
                Follow::Free if v.center == UNPLACED => v.center = driver.unwrap_or((0, 0)),
                Follow::Free => {}
            }
        }
        self.interface = interface;
        self.shown.clear();
        let mut frame = Frame { view_cameras: vec![None; scene.views.len()], ..Default::default() };
        let (main_rect, pane_rect) = self.panes();
        let push = |frame: &mut Frame, camera: Camera| {
            frame.cameras.push(camera);
            frame.cameras.len() as u32 - 1
        };
        // The driver's world views, the main one arranged.
        for (i, (v, camera)) in scene.views.iter().zip(&base).enumerate() {
            if v.view.interface_only {
                continue;
            }
            let camera = if Some(i) == main { self.arranged_main(camera, main_rect, overhang) } else { *camera };
            frame.view_cameras[i] = Some(push(&mut frame, camera));
            if Some(i) == main {
                self.shown.push((Hit::Main, camera));
            }
        }
        // The viewer's own views (world only: no overlay belongs to them).
        let frame_of = |rect: [i32; 4]| {
            let r = [rect[0] - GAP / 2, rect[1] - GAP / 2, rect[2] + GAP, rect[3] + GAP];
            // A zoomed view of nothing: a block of the background colour.
            Camera { zoom: (2, 1), origin: (r[0], r[1]), cells: false, ..Camera::new((r[2].max(1) as u32, r[3].max(1) as u32)) }
        };
        if let (Some(split), Some(rect)) = (self.split, pane_rect) {
            let camera = split.view.camera(rect, overhang, bounds);
            push(&mut frame, camera);
            self.shown.push((Hit::Pane, camera));
        }
        for (k, inset) in self.insets.iter().enumerate() {
            push(&mut frame, frame_of(inset.rect));
            let camera = inset.view.camera(inset.rect, overhang, bounds);
            push(&mut frame, camera);
            self.shown.push((Hit::Inset(k), camera));
        }
        // The interface on top, as the driver placed it.
        for (i, (v, camera)) in scene.views.iter().zip(&base).enumerate() {
            if v.view.interface_only {
                frame.view_cameras[i] = Some(push(&mut frame, *camera));
            }
        }
        if let Some(m) = self.magnifier {
            let rect = self.magnifier_rect(&m);
            let zoom = ZOOMS[m.zoom];
            match m.mode {
                MagnifierMode::Screen => {
                    // The window around the pointer, at the magnifier's zoom.
                    let (sw, sh) = ((m.size.0 * zoom.1).div_ceil(zoom.0) as i32, (m.size.1 * zoom.1).div_ceil(zoom.0) as i32);
                    push(&mut frame, frame_of(rect));
                    frame.copies.push(ScreenCopy { source: [self.pointer.0 - sw / 2, self.pointer.1 - sh / 2, sw, sh], at: (rect[0], rect[1]), zoom });
                }
                MagnifierMode::World => {
                    let center = if self.locked { Some(self.cursor) } else { self.world_under(self.pointer) };
                    push(&mut frame, frame_of(rect));
                    let camera = FreeView::new(center.unwrap_or((0, 0)), m.zoom, Follow::Free).camera(rect, overhang, None);
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
            ..*camera
        }
    }

    fn hit(&self, p: (i32, i32)) -> Hit {
        if self.magnifier.is_some_and(|m| inside(self.magnifier_rect(&m), p)) {
            return Hit::Magnifier;
        }
        if self.interface.iter().any(|&r| inside(r, p)) {
            return Hit::Interface;
        }
        if let Some(k) = (0..self.insets.len()).rev().find(|&k| inside(self.insets[k].rect, p)) {
            return Hit::Inset(k);
        }
        if self.divider().is_some_and(|r| inside(r, p)) {
            return Hit::Divider;
        }
        let (main, pane) = self.panes();
        if pane.is_some_and(|r| inside(r, p)) {
            return Hit::Pane;
        }
        if inside(main, p) && self.main.is_some() {
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
        let hit = match self.hit(p) {
            Hit::Magnifier => {
                // Under a screen magnifier: what it magnifies.
                let m = self.magnifier?;
                if m.mode == MagnifierMode::World {
                    return None;
                }
                Hit::Main
            }
            h => h,
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

    /// Where a pointer event goes in the driver's window: `Ok(q)`, or
    /// `Err(world pixel)` if the driver does not show it, `Err(None)` if
    /// nothing.
    fn route_point(&self, p: (i32, i32)) -> Result<(i32, i32), Option<(i32, i32)>> {
        if self.locked {
            return self.route_world(self.cursor);
        }
        match self.hit(p) {
            Hit::Interface => Ok(p),
            Hit::Divider => Err(None),
            Hit::Magnifier => {
                let m = self.magnifier.ok_or(None)?;
                let rect = self.magnifier_rect(&m);
                let zoom = ZOOMS[m.zoom];
                let (n, d) = (zoom.0 as i32, zoom.1 as i32);
                let (sw, sh) = ((m.size.0 as i32 * d + n - 1) / n, (m.size.1 as i32 * d + n - 1) / n);
                let off = ((p.0 - rect[0]) * d / n, (p.1 - rect[1]) * d / n);
                match m.mode {
                    // The screen pixel it shows, routed as if pointed at.
                    MagnifierMode::Screen => {
                        let q = (self.pointer.0 - sw / 2 + off.0, self.pointer.1 - sh / 2 + off.1);
                        let mut without = self.clone();
                        without.magnifier = None;
                        without.route_point(q)
                    }
                    MagnifierMode::World => {
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
            Input::MouseMove { x, y } => {
                if self.locked {
                    return vec![];
                }
                self.pointer = (x, y);
                if let Some(drag) = self.drag {
                    self.dragged(drag, (x, y));
                    return vec![];
                }
                match self.route_point((x, y)) {
                    Ok((x, y)) => vec![Input::MouseMove { x, y }],
                    Err(_) => vec![],
                }
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
                if let Some((_, camera)) = self.shown.iter().find(|(h, _)| *h == hit) {
                    self.cursor = world_at(camera, p);
                }
                self.locked = true;
                self.grab = Some(true);
                return vec![];
            }
            _ => {}
        }
        match self.route_point(p) {
            Ok(q) => {
                self.pressed[b] = true;
                vec![Input::MouseButton { button, down, x: q.0, y: q.1 }]
            }
            Err(Some(w)) if button == Button::Left => vec![Input::Look { x: w.0, y: w.1 }],
            Err(_) => vec![],
        }
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
        const KEYS: [u32; 17] = [0x53, 0x49, 0x58, 0x4D, 0x30, 0xBB, 0xBD, 0x6B, 0x6D, 0x25, 0x26, 0x27, 0x28, 0x46, 0x20, 0x41, 0x60];
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
                    None => Some(Split { axis: Axis::Vertical, fraction: 0.5, view: FreeView::new(self.world_under(p).unwrap_or(UNPLACED), ONE, Follow::Free) }),
                    Some(s) if s.axis == Axis::Vertical => Some(Split { axis: Axis::Horizontal, ..s }),
                    Some(_) => None,
                }
            }
            // I: an inset at the pointer.
            0x49 if self.insets.len() < MAX_INSETS => {
                let (w, h) = (self.window.0 as i32 / 4, self.window.1 as i32 / 4);
                let view = FreeView::new(self.world_under(p).unwrap_or(UNPLACED), ONE + 2, Follow::Free);
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
            // Space: release (or take) the mouse.
            0x20 => {
                let lock = !self.locked && self.follows_pointer();
                if lock != self.locked {
                    self.locked = lock;
                    self.grab = Some(lock);
                }
            }
            // A: the arrow at the point of interest.
            0x41 => self.focus_arrow = !self.focus_arrow,
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
        l.split.as_mut().unwrap().view.center = (5000, 2000);
        let f = frame(&mut l, vec![]);
        assert_eq!(f.cameras.len(), 3);
        assert_eq!(f.cameras[0].viewport, (400, 600));
        assert_eq!(f.cameras[1].origin, (404, 0));
        // A click there: the driver is asked to look.
        let out = click(&mut l, (600, 300));
        assert!(matches!(out[..], [Input::Look { .. }]), "{out:?}");
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
        assert_eq!(f.copies, [ScreenCopy { source: [250, 275, 100, 50], at: (324, 250), zoom: (2, 1) }]);
        // A click on the magnifier goes to what it shows.
        assert_eq!(click(&mut l, (334, 260))[0], Input::MouseButton { button: Button::Left, down: true, x: 255, y: 280 });
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
    fn config_rejects_nonsense() {
        let mut l = Layout::default();
        assert!(l.configure("split=diagonal").is_err());
        assert!(l.configure("pane_zoom=5/7").is_err());
        assert!(l.configure("arrow=on,magnifier=world").is_ok());
        assert!(l.focus_arrow && l.magnifier.unwrap().mode == MagnifierMode::World);
    }
}
