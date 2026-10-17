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
//!
//! # Driven by the user's settings
//!
//! [`Layout::apply_settings`] sets the arrangement from the viewer's
//! settings (`viewer_settings::Layout`); a spec (`TILE_VIEWER_LAYOUT`) and
//! the Ctrl+Shift keys change it on top, for testing. Views A and B are the
//! main pane and the second pane.
//!
//! - **Split**: one view, A beside B (1:1, or A twice as wide), or A above B.
//! - **Zoom box**: a world magnifier in a box over the window (or, in a
//!   vertical split, over A or B, `zoom_place`) sized 1:3:1 or 1:7:1
//!   (margin : box : margin), a box of 1:1:1 or 1:3:1 about the pointer,
//!   or a strip across the top of that region (`zoom_strip` of its height)
//!   that the views give room to.
//! - **Screens**: a screen the driver names (`Scene::interface`) goes over
//!   the window, or its whole picture is fitted into A or B (as `ui=pane`).
//! - **Roles**: `Free` (A shows the driver's view, B is moved by hand), or
//!   `UnitMouse` (A follows the focus of interest, B follows the world
//!   cursor and captures the mouse; an open popup moves fully into A where
//!   it fits).
//! - **Smooth world movement**: a view following the driver's view or the
//!   focus of interest glides to where it goes (a critically damped spring,
//!   [`Glide`]; retargeting keeps its velocity) instead of jumping.
//!
//! # The divider
//!
//! While the mouse is not captured the divider between A and B is a wall
//! except in its middle third (drawn without the line there) and where the
//! driver's always reachable controls are (`Interface::reachable`, e.g. the
//! unit's orders): the pointer pushed against it stays in its view (the
//! window is asked to move the system pointer back, [`Layout::take_warp`])
//! and scrolls that view instead (`Input::Edge` for the driver's view shown
//! unmoved, else the viewer pans it). Through the gaps it moves freely. The
//! driver's roaming controls (`Interface::roaming`, a game's corner menu
//! buttons) are drawn in the view the pointer is in. While the mouse is
//! captured the whole divider is a wall. Ctrl pressed and released on its
//! own releases a captured mouse; over a view following the world cursor
//! the pointer then shows that a click captures it again
//! ([`Layout::take_pointer_icon`]).

use std::collections::BTreeMap;

use scene::{Button, Input, Scene};
use tile_render::{Camera, ScreenCopy};
use viewer_settings::{Place, Roles, ScreenPlace};

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
/// How fast the viewer scrolls a view the pointer pushes against the
/// divider, in screen pixels per second.
pub const EDGE_PAN_SPEED: f64 = 600.0;
/// The longest frame step smooth movement takes (a stalled frame does not
/// make a view jump).
const MAX_STEP: f64 = 0.1;

/// Smooth movement of a view's centre: a critically damped spring pulling
/// it towards its target, with momentum (retargeting keeps the velocity),
/// in world pixels (not snapped to tiles).
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Glide {
    pos: (f64, f64),
    vel: (f64, f64),
    /// Whether `pos` was placed yet.
    placed: bool,
}

impl Glide {
    /// The spring's angular frequency (1/s): settles in about half a second.
    pub const OMEGA: f64 = 9.0;

    /// At `at`, at rest.
    pub fn snap(&mut self, at: (i32, i32)) {
        *self = Glide { pos: (at.0 as f64, at.1 as f64), vel: (0.0, 0.0), placed: true };
    }

    /// Whether it rests at `target`.
    pub fn settled(&self, target: (i32, i32)) -> bool {
        self.placed && self.vel == (0.0, 0.0) && self.pos == (target.0 as f64, target.1 as f64)
    }

    pub fn velocity(&self) -> (f64, f64) {
        self.vel
    }

    /// Moves `dt` seconds towards `target` (the first time: placed there);
    /// the centre now. In a world wrapping along an axis (`period` > 0) it
    /// goes the short way round.
    pub fn step(&mut self, target: (i32, i32), dt: f64, period: (i32, i32)) -> (i32, i32) {
        if !self.placed {
            self.snap(target);
            return target;
        }
        let dt = dt.clamp(0.0, MAX_STEP);
        let axis = |pos: &mut f64, vel: &mut f64, target: i32, period: i32| {
            let mut t = target as f64;
            if period > 0 {
                let p = period as f64;
                // The target's copy nearest the centre.
                t = *pos + (t - *pos + p / 2.0).rem_euclid(p) - p / 2.0;
            }
            // x(t) = t + (e0 + (v0 + ω e0) t) e^{-ω t} (exact).
            let (e0, v0) = (*pos - t, *vel);
            let w = Self::OMEGA;
            let decay = (-w * dt).exp();
            let c = v0 + w * e0;
            let e = (e0 + c * dt) * decay;
            let v = (v0 - w * c * dt) * decay;
            if e.abs() < 0.5 && v.abs() < 4.0 {
                *pos = target as f64;
                *vel = 0.0;
            } else {
                *pos = t + e;
                *vel = v;
                if period > 0 {
                    // The copy nearest the target itself (a view following
                    // the driver's keeps its labels near their place).
                    let (p, target) = (period as f64, target as f64);
                    *pos = target + (*pos - target + p / 2.0).rem_euclid(p) - p / 2.0;
                }
            }
        };
        axis(&mut self.pos.0, &mut self.vel.0, target.0, period.0);
        axis(&mut self.pos.1, &mut self.vel.1, target.1, period.1);
        (self.pos.0.round() as i32, self.pos.1.round() as i32)
    }
}

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
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FreeView {
    pub center: (i32, i32),
    pub zoom: usize,
    pub follow: Follow,
    /// Placed by the viewer, not by hand, while the driver's view may still
    /// be settling (see [`SETTLE_FRAMES`]): it goes on following it.
    settling: bool,
    /// Its smooth movement (with smooth world movement on).
    glide: Glide,
    /// Following the focus of interest: moved by hand while the focus was
    /// here; it follows again once the focus moves.
    hand_focus: Option<(i32, i32)>,
}

impl FreeView {
    pub fn new(center: (i32, i32), zoom: usize, follow: Follow) -> Self {
        Self { center, zoom, follow, settling: false, glide: Glide::default(), hand_focus: None }
    }

    /// Moved by hand to `center`.
    fn moved_by_hand(&mut self, center: (i32, i32), focus: Option<(i32, i32)>) {
        match self.follow {
            // Following the focus of interest: until it moves.
            Follow::Focus => self.hand_focus = focus.or(Some(UNPLACED)),
            _ => self.follow = Follow::Free,
        }
        self.settling = false;
        self.center = center;
        self.glide.snap(center);
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

#[derive(Copy, Clone, Debug, PartialEq)]
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
    /// Its size where it goes beside the pointer ([`ZoomPlace::Beside`]).
    pub size: (u32, u32),
    pub place: ZoomPlace,
    /// The region its box or strip goes in (a vertical split's left or right
    /// view, else the window).
    pub region: Place,
}

/// Where the magnifier goes.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ZoomPlace {
    /// Beside the pointer, `Magnifier::size` (Ctrl+Shift+M).
    #[default]
    Beside,
    /// A box in the middle of its region, `1 : n : 1` of it (margin : box :
    /// margin) each way.
    Box(u32),
    /// A box about the pointer (the world cursor while the mouse is
    /// captured), `1 : n : 1` of its region, kept inside it.
    AboutPointer(u32),
    /// A strip across the top of its region, this many thousandths of its
    /// height; the views below give it room.
    Strip(u32),
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
    /// A part of the driver's interface the viewer drew elsewhere (index
    /// into `Layout::moved`).
    Moved(usize),
}

/// One of the two views of a split.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Side {
    /// The main pane (left, or top).
    A,
    /// The second pane (right, or bottom).
    B,
}

impl Side {
    fn hit(self) -> Hit {
        match self {
            Side::A => Hit::Main,
            Side::B => Hit::Pane,
        }
    }
}

/// The pointer the window shows.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PointerIcon {
    #[default]
    Default,
    /// Over a view following the world cursor, the mouse free: a click
    /// captures it.
    Capture,
}

/// The driver's whole screen fitted into a pane (`ui=pane`, or a screen
/// placed there): which pane, where and at which zoom.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Fit {
    hit: Hit,
    origin: (i32, i32),
    zoom: (u32, u32),
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
    /// From the last frame: where the driver's screen sits and at which
    /// zoom, when it is fitted into a pane.
    fit: Option<Fit>,
    /// Smooth world movement (see [`Glide`]).
    pub smooth: bool,
    /// What the views follow (from the settings).
    pub roles: Roles,
    /// Where the driver's named screens go (from the settings).
    pub screen_places: BTreeMap<String, ScreenPlace>,
    /// Seconds passed since the last frame ([`Layout::advance`]).
    step: f64,
    /// The view the pointer is in (none: not known since it entered).
    side: Option<Side>,
    /// The view whose side of the divider the pointer pushes against.
    against: Option<Side>,
    /// The pointer as the window last reported it (before the divider held
    /// it back), and where the window is to move the system pointer.
    raw_pointer: (i32, i32),
    warp: Option<(i32, i32)>,
    /// Whether the divider holds the pointer back (not where the window
    /// cannot move the system pointer).
    walls: bool,
    /// Ctrl went down with no other key or button since (and none held).
    ctrl_alone: bool,
    /// The keys held.
    keys_down: Vec<u32>,
    /// The pointer the window shows as last told to it.
    icon: PointerIcon,
    /// From the last frame: the driver's interface parts (see
    /// `scene::Interface`): the rectangle reachable from every view, the
    /// screen named, and the parts drawn elsewhere (window rectangle, how
    /// far they were moved).
    reachable: Option<[i32; 4]>,
    screen: Option<String>,
    /// Whether the last frame had a popup (for the log).
    popup_shown: bool,
    /// Seconds of frames so far (when presses were made).
    clock: f64,
    /// The last press that went to the driver: where it was made in the
    /// window, where the driver got it, when.
    last_press: Option<((i32, i32), (i32, i32), f64)>,
    /// How far the open popup moves, when it opened at a click.
    popup_anchor: Option<(i32, i32)>,
    /// The world cursor to capture the mouse at again once the popup that
    /// let it go closes; the screen likewise.
    recapture: Option<(i32, i32)>,
    recapture_screen: Option<(i32, i32)>,
    /// The pointer goes to the screen just opened (this frame).
    pointer_to_screen: bool,
    moved: Vec<([i32; 4], (i32, i32))>,
    /// Whether the last frame showed the magnifier (it may be left out, or
    /// be under a screen fitted into a pane).
    magnifier_shown: bool,
    /// Whether the scene of the last frame had a world (the zoom box and
    /// strip show only then).
    world_shown: bool,
    /// Where a world magnifier looked last (it keeps looking there while
    /// nothing is under the pointer).
    magnifier_center: Option<(i32, i32)>,
    /// From the last frame: the focus of interest.
    focus: Option<(i32, i32)>,
    /// Whether zoomed views show text at their resolution (the driver's
    /// zoomed art) rather than its pixels scaled.
    pub sharp_text: bool,
    window: (u32, u32),
    pointer: (i32, i32),
    /// The world cursor, and the motion not yet a whole world pixel.
    cursor: (i32, i32),
    /// Whether the world cursor has been put where the driver first shows a
    /// world (while it settles, it follows the driver's view).
    cursor_placed: bool,
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
    /// The main pane's world rectangle as last told to the driver.
    reported_main: (i32, i32, i32, i32),
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
            cursor_placed: false,
            motion_rest: (0.0, 0.0),
            locked: false,
            grab: None,
            reported_zooms: [(0, 0); 8],
            art_zooms: Vec::new(),
            reported_views: [(0, 0, 0, 0); 4],
            reported_main: (0, 0, 0, 0),
            sharp_text: true,
            ui_in_pane: false,
            fit: None,
            smooth: false,
            roles: Roles::Free,
            screen_places: BTreeMap::new(),
            step: 0.0,
            side: None,
            against: None,
            raw_pointer: (i32::MIN, i32::MIN),
            warp: None,
            walls: true,
            ctrl_alone: false,
            keys_down: Vec::new(),
            icon: PointerIcon::Default,
            reachable: None,
            screen: None,
            popup_shown: false,
            clock: 0.0,
            last_press: None,
            popup_anchor: None,
            recapture: None,
            recapture_screen: None,
            pointer_to_screen: false,
            moved: Vec::new(),
            magnifier_shown: true,
            world_shown: true,
            magnifier_center: None,
            focus: None,
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

/// Whether rectangles (x, y, w, h) share a pixel.
fn overlap(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

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

/// A camera's rectangle in the window (x, y, w, h).
fn rect_of(c: &Camera) -> [i32; 4] {
    [c.origin.0, c.origin.1, c.viewport.0 as i32, c.viewport.1 as i32]
}

/// [`world_at`] rounding towards minus infinity (for cameras cut out of
/// others).
fn world_at_floor(camera: &Camera, (px, py): (i32, i32)) -> (i32, i32) {
    let (n, d) = (camera.zoom.0.max(1) as i32, camera.zoom.1.max(1) as i32);
    (camera.x + ((px - camera.origin.0) * d).div_euclid(n), camera.y + ((py - camera.origin.1) * d).div_euclid(n))
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

    const MAGNIFIER: Magnifier = Magnifier { mode: MagnifierMode::Screen, zoom: ONE + 3, size: (240, 160), place: ZoomPlace::Beside, region: Place::Floating };
    /// The zoom box's zoom to start with (2/1).
    const ZOOM_BOX_ZOOM: usize = ONE + 2;

    /// Arranges the views as the user's settings say: the split, the zoom
    /// box, where named screens go, what the views follow and whether they
    /// move smoothly. Views already placed stay where they look.
    pub fn apply_settings(&mut self, s: &viewer_settings::Layout) {
        use viewer_settings::{Split as S, Zoom};
        let (main_follow, pane_follow) = match s.roles {
            Roles::Free => (Follow::Driver, Follow::Free),
            Roles::UnitMouse => (Follow::Focus, Follow::Pointer),
        };
        self.split = match s.split {
            S::Standard => None,
            kind => {
                let (axis, fraction) = match kind {
                    S::Vertical2To1 => (Axis::Vertical, 2.0 / 3.0),
                    S::Horizontal1To1 => (Axis::Horizontal, 0.5),
                    _ => (Axis::Vertical, 0.5),
                };
                let mut view = self.split.map_or(FreeView::new(UNPLACED, ONE, pane_follow), |s| s.view);
                if view.follow != pane_follow {
                    view.follow = pane_follow;
                    view.hand_focus = None;
                }
                Some(Split { axis, fraction, view })
            }
        };
        if self.main_view.follow != main_follow {
            self.main_view.follow = main_follow;
            self.main_view.hand_focus = None;
        }
        let zoom = self.magnifier.filter(|m| m.place != ZoomPlace::Beside).map_or(Self::ZOOM_BOX_ZOOM, |m| m.zoom);
        let place = match s.zoom {
            Zoom::Off => None,
            Zoom::Floating131 => Some(ZoomPlace::Box(3)),
            Zoom::Floating171 => Some(ZoomPlace::Box(7)),
            Zoom::Mouse111 => Some(ZoomPlace::AboutPointer(1)),
            Zoom::Mouse131 => Some(ZoomPlace::AboutPointer(3)),
            Zoom::TopStrip => Some(ZoomPlace::Strip((s.zoom_strip.clamp(0.1, 0.5) * 1000.0).round() as u32)),
        };
        self.magnifier = place.map(|place| Magnifier { mode: MagnifierMode::World, zoom, place, region: s.zoom_place, ..Self::MAGNIFIER });
        self.screen_places = s.screens.clone();
        self.smooth = s.smooth_world_movement;
        self.roles = s.roles;
        // Nothing follows the world cursor any more: the mouse goes free.
        if self.locked && !self.follows_pointer() {
            self.release("the layout changed");
        }
    }

    /// Time passed (for smooth movement and the viewer's own scrolling):
    /// taken by the next frame.
    pub fn advance(&mut self, dt: std::time::Duration) {
        self.step += dt.as_secs_f64();
    }

    /// Where the window is to move the system pointer (the divider held it
    /// back), if it is to.
    pub fn take_warp(&mut self) -> Option<(i32, i32)> {
        self.warp.take()
    }

    /// The window cannot move the system pointer (e.g. on Wayland): the
    /// divider lets the pointer through everywhere while the mouse is free,
    /// the pointer the user sees being where the layout has it.
    pub fn pointer_cannot_be_moved(&mut self) {
        if self.walls {
            self.walls = false;
            // Where the system pointer really is.
            self.pointer = self.raw_pointer;
            self.side = self.side_of(self.pointer);
            self.against = None;
            log::info!("viewer: the system pointer cannot be moved back here; the divider lets it through while the mouse is free");
        }
    }

    /// The pointer the window is to show, when that changed.
    pub fn take_pointer_icon(&mut self) -> Option<PointerIcon> {
        let icon = if !self.locked && self.capture_hit(self.pointer).is_some() { PointerIcon::Capture } else { PointerIcon::Default };
        (icon != self.icon).then(|| {
            self.icon = icon;
            icon
        })
    }

    /// The zooms of the views showing the world scaled, when they changed
    /// since last asked (for the driver, `Input::Zooms`; none without sharp
    /// text).
    pub fn take_zooms_change(&mut self) -> Option<[(u8, u8); 8]> {
        let mut zooms = [(0, 0); 8];
        if self.sharp_text {
            let views = std::iter::once(self.main_view).chain(self.split.map(|s| s.view)).chain(self.insets.iter().map(|i| i.view));
            let used = views.map(|v| v.zoom).chain(self.magnifier.filter(|m| m.mode == MagnifierMode::World).map(|m| m.zoom)).filter(|&z| z != ONE);
            // (The fitted screen's zoom, too.)
            let mut used: Vec<(u32, u32)> = used.map(|z| ZOOMS[z]).chain(self.fit.map(|f| f.zoom)).filter(|z| z.0 <= 255 && z.1 <= 255).collect();
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

    /// The world rectangle the main pane shows of the driver's main view
    /// where that is not all of it (as of the last frame; `Input::MainView`,
    /// w 0: all of it), when it changed since last asked.
    pub fn take_main_view_change(&mut self) -> Option<(i32, i32, i32, i32)> {
        let whole = self.fit.is_some() || (self.split.is_none() && self.strip().is_none() && self.main_unmoved());
        let rect = match self.shown.iter().find(|(h, _)| *h == Hit::Main) {
            Some((_, c)) if !whole => {
                let (w, h) = c.source_size();
                (c.x, c.y, w as i32, h as i32)
            }
            _ => (0, 0, 0, 0),
        };
        (rect != self.reported_main).then(|| {
            self.reported_main = rect;
            rect
        })
    }

    /// The world rectangles the split pane and the insets show (as of the
    /// last frame), when they changed since last asked (for the driver,
    /// `Input::Views`).
    pub fn take_views_change(&mut self) -> Option<[(i32, i32, i32, i32); 4]> {
        let mut views = [(0, 0, 0, 0); 4];
        // (The main pane is a view of its own while the driver's screen is
        // fitted into the second one; the fitted screen is the driver's.)
        let fitted = self.fit.map(|f| f.hit);
        let own = self.shown.iter().filter(|(h, _)| {
            Some(*h) != fitted && (matches!(h, Hit::Pane | Hit::Inset(_)) || (*h == Hit::Main && fitted == Some(Hit::Pane)))
        });
        for (slot, (_, c)) in views.iter_mut().zip(own) {
            let (w, h) = c.source_size();
            *slot = (c.x, c.y, w as i32, h as i32);
        }
        (views != self.reported_views).then(|| {
            self.reported_views = views;
            views
        })
    }

    /// The pane the driver's whole screen is fitted into, if it is: the main
    /// pane with `ui_in_pane`, else where the settings put the screen the
    /// driver names (`screen`; a vertical split's left or right view, a
    /// horizontal one's bottom or top view).
    fn fit_target(&self, screen: Option<&str>) -> Option<Hit> {
        let split = self.split?;
        if self.ui_in_pane {
            return Some(Hit::Main);
        }
        let place = self.screen_places.get(screen?)?;
        let vertical = split.axis == Axis::Vertical;
        match (if vertical { place.vertical } else { place.horizontal }, vertical) {
            (Place::Floating, _) => None,
            (Place::LeftOrBottom, true) | (Place::RightOrTop, false) => Some(Hit::Main),
            (Place::LeftOrBottom, false) | (Place::RightOrTop, true) => Some(Hit::Pane),
        }
    }

    /// The zoom fitting the window into `rect` (reduced).
    fn screen_fit(&self, rect: [i32; 4]) -> (u32, u32) {
        let (ww, wh) = (self.window.0.max(1), self.window.1.max(1));
        let (pw, ph) = (rect[2].max(1) as u32, rect[3].max(1) as u32);
        // The smaller of the two ratios, as a fraction.
        let (n, d) = if pw as u64 * wh as u64 <= ph as u64 * ww as u64 { (pw, ww) } else { (ph, wh) };
        let g = gcd(n, d);
        (n / g, d / g)
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
    fn capture(&mut self, hit: Hit, _p: (i32, i32)) {
        // Only a capture: the world cursor at the middle of the view as it
        // is (where it was when the mouse was let go), the view staying
        // put; nothing goes to the driver.
        if let Some((_, camera)) = self.shown.iter().find(|(h, _)| *h == hit) {
            let (w, h) = camera.source_size();
            self.cursor = (camera.x + w as i32 / 2, camera.y + h as i32 / 2);
        }
        self.locked = true;
        self.grab = Some(true);
        self.against = None;
        log::info!("viewer: mouse captured, it moves the world cursor at {:?} (Ctrl on its own or Ctrl+Shift+Space releases it)", self.cursor);
    }

    /// Releases the mouse (never captures it).
    fn release(&mut self, why: &str) {
        let was = std::mem::replace(&mut self.locked, false);
        self.grab = Some(false);
        if was {
            log::info!("viewer: mouse released ({why}); a click into the view following the world cursor captures it again");
        }
    }

    /// Releases the mouse, the system pointer showing up where the world
    /// cursor is.
    fn release_here(&mut self, why: &str) {
        let was = self.locked;
        self.release(why);
        if let (true, Some(p)) = (was, self.cursor_on_window()) {
            self.pointer = p;
            self.raw_pointer = p;
            self.warp = Some(p);
            self.side = self.side_of(p);
        }
    }

    /// Where the world cursor is in the window: in the view following it.
    fn cursor_on_window(&self) -> Option<(i32, i32)> {
        let (_, c) = self.shown.iter().find(|(h, _)| self.follow_of(*h) == Some(Follow::Pointer))?;
        let (n, d) = (c.zoom.0.max(1) as i32, c.zoom.1.max(1) as i32);
        Some((c.origin.0 + (self.cursor.0 - c.x) * n / d, c.origin.1 + (self.cursor.1 - c.y) * n / d))
    }

    /// The pointer left the window, or the window lost the focus: it is at
    /// no edge any more (what goes to the driver), and comes back into
    /// whichever view it enters.
    pub fn pointer_gone(&mut self) -> Vec<Input> {
        self.side = None;
        self.against = None;
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

    /// Whether the main pane shows the driver's main view unmoved (or
    /// gliding back to it): following it at zoom 1, each window pixel the
    /// driver's own (perhaps cropped), its screen not fitted into a pane.
    fn main_unmoved(&self) -> bool {
        self.main.is_some() && self.main_view.follow == Follow::Driver && self.main_view.zoom == ONE && self.fit.is_none() && self.shown.iter().any(|(h, _)| *h == Hit::Main)
    }

    /// Where pointer `p` is at the window's edge (see `Input::Edge`): only
    /// over the driver's main view shown unmoved, at zoom 1 where the driver
    /// shows it (a window pixel is the driver's), so that the window's edge
    /// is the driver's screen edge; not while the mouse is captured or
    /// dragging, nor with the driver's screen fitted into a pane. Pushed
    /// against the divider from the main pane: at that edge.
    fn edge_at(&self, p: (i32, i32)) -> (i32, i32) {
        let (main_rect, _) = self.panes();
        let over_main = match self.hit(p) {
            Hit::Main => true,
            // The driver's interface where the main view is.
            Hit::Interface | Hit::Moved(_) => inside(main_rect, p),
            _ => false,
        };
        let pushed = self.against == Some(Side::A);
        if !self.main_unmoved() || !(over_main || pushed) || self.locked || self.drag.is_some() {
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
        let edge = (at(p.0, self.window.0), at(p.1, self.window.1));
        // The main pane is left of (above) the divider: its right (bottom)
        // edge, at the outermost pixel.
        match self.split.map(|s| s.axis) {
            Some(Axis::Vertical) if pushed => (1, edge.1),
            Some(Axis::Horizontal) if pushed => (edge.0, 1),
            _ => edge,
        }
    }

    /// The zoom strip's rectangle, if the magnifier is one, and the region
    /// it is carved from (`None`: the whole window; else a split's pane).
    fn strip(&self) -> Option<([i32; 4], Option<Side>)> {
        let m = self.magnifier.filter(|_| self.world_shown)?;
        let ZoomPlace::Strip(thousandths) = m.place else { return None };
        let (w, h) = (self.window.0 as i32, self.window.1 as i32);
        let side = match (self.split.map(|s| s.axis), m.region) {
            (Some(Axis::Vertical), Place::LeftOrBottom) => Some(Side::A),
            (Some(Axis::Vertical), Place::RightOrTop) => Some(Side::B),
            _ => None,
        };
        let column = match side {
            None => [0, 0, w, h],
            Some(s) => {
                let (a, b) = self.panes_in([0, 0, w, h]);
                if s == Side::A { a } else { b.unwrap_or(a) }
            }
        };
        let height = ((column[3] as i64 * thousandths as i64) / 1000) as i32;
        Some(([column[0], column[1], column[2], height.clamp(0, (column[3] - GAP).max(0))], side))
    }

    /// The main pane's rectangle, and the second pane's if split (below the
    /// zoom strip, if there is one).
    fn panes(&self) -> ([i32; 4], Option<[i32; 4]>) {
        let (w, h) = (self.window.0 as i32, self.window.1 as i32);
        let below = |r: [i32; 4], s: [i32; 4]| [r[0], s[1] + s[3] + GAP, r[2], (r[3] - s[3] - GAP).max(1)];
        match self.strip() {
            None => self.panes_in([0, 0, w, h]),
            Some((s, None)) => self.panes_in(below([0, 0, w, h], s)),
            Some((s, Some(side))) => {
                let (a, b) = self.panes_in([0, 0, w, h]);
                match (side, b) {
                    (Side::A, b) => (below(a, s), b),
                    (Side::B, Some(b)) => (a, Some(below(b, s))),
                    (Side::B, None) => (a, None),
                }
            }
        }
    }

    /// The panes in `area`.
    fn panes_in(&self, [x, y, w, h]: [i32; 4]) -> ([i32; 4], Option<[i32; 4]>) {
        match self.split {
            None => ([x, y, w, h], None),
            Some(s) => match s.axis {
                Axis::Vertical => {
                    let a = ((w as f32 * s.fraction) as i32).clamp(GAP, (w - GAP).max(GAP));
                    ([x, y, a, h], Some([x + a + GAP, y, w - a - GAP, h]))
                }
                Axis::Horizontal => {
                    let a = ((h as f32 * s.fraction) as i32).clamp(GAP, (h - GAP).max(GAP));
                    ([x, y, w, a], Some([x, y + a + GAP, w, h - a - GAP]))
                }
            },
        }
    }

    /// The gap between the panes across the split's axis (from, to:
    /// window x for side by side, y for stacked), and its extent along the
    /// divider (from, to).
    fn gap(&self) -> Option<((i32, i32), (i32, i32))> {
        let split = self.split?;
        let (a, b) = self.panes();
        let b = b?;
        Some(match split.axis {
            Axis::Vertical => ((a[0] + a[2], b[0]), (a[1].min(b[1]), (a[1] + a[3]).max(b[1] + b[3]))),
            Axis::Horizontal => ((a[1] + a[3], b[1]), (a[0].min(b[0]), (a[0] + a[2]).max(b[0] + b[2]))),
        })
    }

    /// The stretches along the divider (from, to) where the pointer passes
    /// from one view to the other: its middle third while the mouse is not
    /// captured, and where the driver's always reachable controls are.
    fn open_spans(&self) -> Vec<(i32, i32)> {
        let Some((across, (s0, s1))) = self.gap() else { return vec![] };
        let mut spans = Vec::new();
        if !self.locked {
            let third = (s1 - s0) / 3;
            spans.push((s0 + third, s1 - third));
        }
        if let (Some([x, y, w, h]), Some(split)) = (self.reachable, self.split) {
            // Only where it reaches over the gap.
            let (lo, hi, a0, a1) = match split.axis {
                Axis::Vertical => (x, x + w, y, y + h),
                Axis::Horizontal => (y, y + h, x, x + w),
            };
            if lo < across.1 && hi > across.0 && a1 > a0 {
                spans.push((a0.max(s0), a1.min(s1)));
            }
        }
        spans.retain(|&(a, b)| b > a);
        spans
    }

    /// The divider's rectangle (the gap and two pixels into each pane).
    fn divider(&self) -> Option<[i32; 4]> {
        let ((g0, g1), (s0, s1)) = self.gap()?;
        Some(match self.split?.axis {
            Axis::Vertical => [g0 - 2, s0, g1 - g0 + 4, s1 - s0],
            Axis::Horizontal => [s0, g0 - 2, s1 - s0, g1 - g0 + 4],
        })
    }

    /// Which side of the divider window pixel `p` is on (none: in the gap,
    /// or no split).
    fn side_of(&self, p: (i32, i32)) -> Option<Side> {
        let ((g0, g1), _) = self.gap()?;
        let v = if self.split?.axis == Axis::Vertical { p.0 } else { p.1 };
        if v < g0 {
            Some(Side::A)
        } else if v >= g1 {
            Some(Side::B)
        } else {
            None
        }
    }

    /// The pointer moving from `from` to `to`: where it goes, held back at
    /// the divider unless it crosses through an open stretch (or within the
    /// always reachable controls). Notes the view it is in and whether it
    /// pushes against the divider.
    fn constrain(&mut self, from: (i32, i32), to: (i32, i32)) -> (i32, i32) {
        self.against = None;
        // (Where the system pointer cannot be moved back, a wall would hold
        // the pointer where it is not seen: it passes.)
        if !self.walls {
            self.side = self.side_of(to);
            return to;
        }
        let Some(((g0, g1), _)) = self.gap().filter(|_| !self.locked && self.drag.is_none()) else {
            self.side = None;
            return to;
        };
        let vertical = self.split.is_some_and(|s| s.axis == Axis::Vertical);
        let target = self.side_of(to);
        let Some(side) = self.side else {
            // Not known where it was (it just came in): where it is now.
            self.side = target;
            return to;
        };
        let (fa, fb, ta, tb) = if vertical { (from.0, from.1, to.0, to.1) } else { (from.1, from.0, to.1, to.0) };
        let spans = self.open_spans();
        let in_span = |v: i32| spans.iter().any(|&(a, b)| v >= a && v < b);
        let reachable = self.reachable.is_some_and(|r| {
            let grown = [r[0] - GAP, r[1] - GAP, r[2] + 2 * GAP, r[3] + 2 * GAP];
            inside(grown, from) || inside(grown, to)
        });
        // Its side's last pixel before the divider.
        let held = if side == Side::A { g0 - 1 } else { g1 };
        let crossing = match side {
            Side::A => ta >= g0,
            Side::B => ta < g1,
        };
        if !crossing {
            self.side = Some(side);
            // Resting against a wall pushes against it (as at the window's
            // edge; also where the window moved the pointer back to).
            if ta == held && !in_span(tb) && !reachable {
                self.against = Some(side);
            }
            return to;
        }
        // Where the move meets the divider (along it).
        let edge = if side == Side::A { g0 } else { g1 - 1 };
        let along = if ta == fa { tb } else { fb + ((tb - fb) as i64 * (edge - fa) as i64 / (ta - fa) as i64) as i32 };
        let open = reachable
            || self.interface_over_divider(if vertical { (edge, along) } else { (along, edge) })
            || match target {
                // Into the gap: only where it is open.
                None => in_span(tb),
                // Already in the gap through an open stretch: on through it.
                Some(_) => in_span(along) || (self.side_of(from).is_none() && in_span(fb)),
            };
        if open {
            if let Some(t) = target {
                self.side = Some(t);
            }
            return to;
        }
        // A wall: it stays at its side's last pixel and pushes against it.
        self.against = Some(side);
        if vertical { (held, to.1) } else { (to.0, held) }
    }

    /// Whether window pixel `p` of the divider lies under something of the
    /// driver's reaching across the gap (its interface where it put it, a
    /// popup or another part drawn elsewhere, its other world views): the
    /// pointer is over that there, not over a view, and passes freely
    /// (e.g. a screen floating over both views, or a popup).
    fn interface_over_divider(&self, p: (i32, i32)) -> bool {
        let (Some(((g0, g1), _)), Some(split)) = (self.gap(), self.split) else { return false };
        if self.fit.is_some() {
            return false;
        }
        let across = |r: &[i32; 4]| match split.axis {
            Axis::Vertical => r[0] < g0 && r[0] + r[2] > g1,
            Axis::Horizontal => r[1] < g0 && r[1] + r[3] > g1,
        };
        let moved = self.moved.iter().map(|(r, _)| r);
        self.interface.iter().chain(&self.driver_rects).chain(moved).any(|r| across(r) && inside(*r, p))
    }

    /// The region a zoom box goes in: a vertical split's left or right
    /// pane, else the window.
    fn zoom_region(&self, m: &Magnifier) -> [i32; 4] {
        let (a, b) = self.panes();
        match (self.split.map(|s| s.axis), m.region, b) {
            (Some(Axis::Vertical), Place::LeftOrBottom, _) => a,
            (Some(Axis::Vertical), Place::RightOrTop, Some(b)) => b,
            _ => [0, 0, self.window.0 as i32, self.window.1 as i32],
        }
    }

    /// The magnifier's rectangle, and (screen magnifier beside the
    /// pointer) the rectangle of the window it magnifies: around the
    /// pointer, inside the window, the magnifier beside it (so it never
    /// shows itself or its frame). A zoom box or strip shows the world.
    fn magnifier_geometry(&self, m: &Magnifier) -> ([i32; 4], Option<[i32; 4]>) {
        let boxed = |region: [i32; 4], n: u32| {
            let n = n.max(1) as i64;
            ((region[2] as i64 * n / (n + 2)) as i32, (region[3] as i64 * n / (n + 2)) as i32)
        };
        match m.place {
            ZoomPlace::Beside => {}
            ZoomPlace::Box(n) => {
                let r = self.zoom_region(m);
                let (w, h) = boxed(r, n);
                return ([r[0] + (r[2] - w) / 2, r[1] + (r[3] - h) / 2, w, h], None);
            }
            ZoomPlace::AboutPointer(n) => {
                let r = self.zoom_region(m);
                let (w, h) = boxed(r, n);
                let (px, py) = if self.locked { self.cursor_on_window().unwrap_or(self.pointer) } else { self.pointer };
                let x = (px - w / 2).clamp(r[0], (r[0] + r[2] - w).max(r[0]));
                let y = (py - h / 2).clamp(r[1], (r[1] + r[3] - h).max(r[1]));
                return ([x, y, w, h], None);
            }
            ZoomPlace::Strip(_) => return (self.strip().map_or([0, 0, 0, 0], |(s, _)| s), None),
        }
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
    /// rectangles the driver's interface covers (window pixels; not those of
    /// its roaming and popup views, `scene::Interface`); `bounds`: the
    /// world's extent.
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
        // (No zoom box or strip without a world: e.g. on a title screen.)
        self.world_shown = scene.shape.cols > 0 && scene.shape.rows > 0;
        let dt = std::mem::take(&mut self.step).min(MAX_STEP);
        let base = crate::scene_cameras(scene, window, overhang);
        // Its own views show the world as the driver's do (their world
        // pixels are those views' pixels).
        let downscale = crate::world_downscale(scene);
        // The driver's main view: its first world view filling the window.
        let main = scene.views.iter().position(|v| !v.view.interface_only && (v.view.viewport.0 == 0 || v.view.viewport.1 == 0));
        self.main = main.map(|i| base[i]);
        // The parts of the driver's interface the viewer arranges.
        let parts = &scene.interface;
        let part = |id: Option<u32>| id.and_then(|id| scene.views.iter().position(|v| v.id == id && v.view.interface_only));
        let (roaming, popup, screen_part) = (part(parts.roaming), part(parts.popup), part(parts.screen_view));
        self.reachable = parts.reachable;
        if parts.screen != self.screen {
            match &parts.screen {
                Some(name) => log::info!("viewer: screen {name} open, placed {}", self.fit_target(Some(name)).map_or("over the window", |h| if h == Hit::Main { "in A" } else { "in B" })),
                None => log::info!("viewer: screen {} closed", self.screen.as_deref().unwrap_or_default()),
            }
            // A screen opening lets go of a captured mouse, the pointer on
            // its main control (below, once it is placed); captured again
            // when it closes.
            match (&parts.screen, &self.screen) {
                (Some(_), None) if self.locked => {
                    self.recapture_screen = Some(self.cursor);
                    self.release_here("a screen opened");
                    self.pointer_to_screen = true;
                }
                (None, Some(_)) => {
                    if let Some(cursor) = self.recapture_screen.take().filter(|_| self.follows_pointer() && !self.locked) {
                        self.cursor = cursor;
                        self.locked = true;
                        self.grab = Some(true);
                        self.against = None;
                        log::info!("viewer: mouse captured again (the screen closed)");
                    }
                }
                _ => {}
            }
        }
        self.clock += dt;
        if popup.is_some() != self.popup_shown {
            self.popup_shown = popup.is_some();
            log::info!("viewer: popup {}", if self.popup_shown { "open, a view of its own" } else { "closed" });
            match popup {
                Some(i) => self.popup_opened(rect_of(&base[i])),
                None => self.popup_closed(),
            }
        }
        self.screen = parts.screen.clone();
        self.focus = scene.focus;
        // Where the views look (from what the last frame showed). (While a
        // view follows the world cursor it stays where the mouse was let
        // go, whatever the pointer is over.)
        if !self.locked && !self.follows_pointer() {
            if let Some(w) = self.world_under(self.pointer).filter(|_| self.hit_follow(self.pointer) != Some(Follow::Pointer)) {
                self.cursor = w;
            }
        }
        let (main_rect, pane_rect) = self.panes();
        let fit_target = self.fit_target(self.screen.as_deref());
        // Pushed against the divider: the view on that side scrolls (the
        // driver's own view shown unmoved scrolls itself, `Input::Edge`).
        self.edge_pan(dt, fit_target);
        if let Some(b) = bounds {
            self.cursor = b.clamp_point(self.cursor);
        }
        let driver = self.main.map(|m| (m.x + m.viewport.0 as i32 / 2, m.y + m.viewport.1 as i32 / 2));
        // The world cursor starts where the driver first shows a world (a
        // view following it shows that until the mouse is captured there).
        if !self.cursor_placed && !self.locked {
            if let Some(d) = driver.filter(|_| scene.shape.cols > 0 && scene.shape.rows > 0) {
                if !self.follows_pointer() || self.world_frames > SETTLE_FRAMES {
                    self.cursor_placed = true;
                }
                self.cursor = d;
            }
        }
        // The main pane following the driver shows the middle of the
        // driver's view in its own middle (a split or a zoom strip crops it
        // round its middle): what the driver centres on (the active unit,
        // a popup's subject, a place looked at) is in the middle of A.
        let main_driver = driver;
        let (cursor, focus) = (self.cursor, scene.focus);
        let has_world = scene.shape.cols > 0 && scene.shape.rows > 0;
        self.world_frames = if has_world { self.world_frames.saturating_add(1) } else { 0 };
        let settled = self.world_frames > SETTLE_FRAMES;
        let period = bounds.map_or((0, 0), |b| (if b.wrap.0 { b.size.0 } else { 0 }, if b.wrap.1 { b.size.1 } else { 0 }));
        let smooth = self.smooth;
        for (k, v) in self.views_mut().enumerate() {
            let target = match v.follow {
                Follow::Driver if k == 0 => main_driver,
                Follow::Driver => driver,
                Follow::Focus => match v.hand_focus {
                    // Moved by hand: there until the focus moves.
                    Some(h) if focus.unwrap_or(UNPLACED) == h => None,
                    _ => {
                        v.hand_focus = None;
                        focus.or(driver)
                    }
                },
                Follow::Pointer => Some(cursor),
                // Placed where the driver first shows a world (not before: a
                // title screen has none, its empty view looks at (0, 0)).
                Follow::Free if v.center == UNPLACED || v.settling => {
                    match driver.filter(|_| has_world) {
                        Some(d) if settled => {
                            v.center = d;
                            if std::mem::take(&mut v.settling) {
                                log::info!("viewer: a free view placed at world pixel {d:?} (the middle of the driver's view)");
                            }
                        }
                        Some(d) => (v.center, v.settling) = (d, true),
                        None => {}
                    }
                    None
                }
                Follow::Free => None,
            };
            match target {
                // Gliding there (the world cursor's view keeps it in its
                // middle, a free view is moved by hand).
                Some(t) if smooth && matches!(v.follow, Follow::Driver | Follow::Focus) => v.center = v.glide.step(t, dt, period),
                Some(t) => {
                    v.glide.snap(t);
                    v.center = t;
                }
                None => v.glide.snap(v.center),
            }
        }
        self.interface = interface;
        self.art_zooms = scene.zooms.clone();
        self.shown.clear();
        self.driver_rects.clear();
        self.moved.clear();
        let mut frame = Frame { view_cameras: vec![None; scene.views.len()], ..Default::default() };
        let push = |frame: &mut Frame, camera: Camera| {
            frame.cameras.push(camera);
            frame.cameras.len() as u32 - 1
        };
        // The driver's views: either the main one arranged, the others (and
        // the interface) over the viewer's own views where the driver put
        // them, or (`ui_in_pane` with a split, or a named screen the
        // settings put into a pane) the driver's whole screen, interface
        // included, fitted into a pane as one picture (its one-pixel lines
        // kept).
        // (Its head: the main view, else the first filling the window, e.g.
        // the game's view hidden by a city screen.)
        let head = main.or_else(|| scene.views.iter().position(|v| v.view.viewport.0 == 0 || v.view.viewport.1 == 0)).or((!scene.views.is_empty()).then_some(0));
        let fit = fit_target.filter(|_| head.is_some()).and_then(|hit| {
            let rect = if hit == Hit::Main { main_rect } else { pane_rect? };
            Some(Fit { hit, origin: (rect[0], rect[1]), zoom: self.screen_fit(rect) })
        });
        self.fit = fit;
        let mut over: Vec<(usize, Camera)> = Vec::new();
        if let (Some(fit), Some(m)) = (fit, head) {
            let zoom = fit.zoom;
            let head = Camera {
                zoom,
                origin: fit.origin,
                viewport: ((window.0 * zoom.0).div_ceil(zoom.1), (window.1 * zoom.0).div_ceil(zoom.1)),
                zoomed_art: self.zoomed_art(zoom),
                ..base[m]
            };
            frame.view_cameras[m] = Some(push(&mut frame, head));
            if Some(m) == main {
                self.shown.push((fit.hit, head));
            }
            // (The roaming controls too, where the driver put them.)
            for (i, camera) in base.iter().enumerate().filter(|&(i, _)| i != m) {
                let stacked = Camera { stacked: true, hairline: true, zoomed_art: self.zoomed_art(zoom), ..*camera };
                frame.view_cameras[i] = Some(push(&mut frame, stacked));
            }
            // The screen in the second pane: the main pane shows the world.
            if fit.hit == Hit::Pane {
                let v = self.main_view;
                let mut camera = v.camera(main_rect, overhang, if v.follow == Follow::Driver { None } else { bounds });
                camera.zoomed_art = self.zoomed_art(camera.zoom);
                camera.downscale = downscale;
                push(&mut frame, camera);
                self.shown.push((Hit::Main, camera));
            }
        } else {
            let roaming_shift = self.roaming_shift(pane_rect);
            let popup_shift = popup.map(|i| self.popup_shift(rect_of(&base[i]), main_rect));
            let screen_shift = screen_part.map(|i| Some(self.screen_shift(rect_of(&base[i]))));
            for (i, (v, camera)) in scene.views.iter().zip(&base).enumerate() {
                let shift = if Some(i) == roaming {
                    Some(roaming_shift)
                } else if Some(i) == popup {
                    Some(popup_shift)
                } else if Some(i) == screen_part {
                    screen_shift
                } else {
                    None
                };
                if Some(i) == main {
                    let camera = self.arranged_main(camera, main_rect, overhang);
                    frame.view_cameras[i] = Some(push(&mut frame, camera));
                    self.shown.push((Hit::Main, camera));
                } else if let Some(shift) = shift {
                    // A part of the interface drawn on its own, where it goes.
                    let Some(d) = shift else { continue };
                    let r = rect_of(camera);
                    self.moved.push(([r[0] + d.0, r[1] + d.1, r[2], r[3]], d));
                    over.push((i, Camera { origin: (camera.origin.0 + d.0, camera.origin.1 + d.1), screen_shift: d, ..*camera }));
                } else {
                    over.push((i, *camera));
                    if !v.view.interface_only {
                        self.driver_rects.push(rect_of(camera));
                    }
                }
            }
        }
        // The viewer's own views, as many as the renderer draws (the second
        // pane first, then the magnifier, then the insets in order, then
        // what fills the open stretches of the divider).
        let mut room = tile_render::MAX_VIEWS.saturating_sub(frame.cameras.len() + over.len());
        let mut take = |n: usize| {
            let fits = n <= room;
            if fits {
                room -= n;
            }
            fits
        };
        let pane_shown = fit.is_none_or(|f| f.hit != Hit::Pane);
        let pane = pane_rect.filter(|_| self.split.is_some() && pane_shown && take(1));
        let magnifier = self.magnifier.filter(|m| take(if m.mode == MagnifierMode::Screen && m.place == ZoomPlace::Beside { 1 } else { 2 }));
        let insets = (0..self.insets.len()).take_while(|_| take(2)).count();
        let left_out = usize::from(pane.is_none() && pane_rect.is_some() && self.split.is_some() && pane_shown)
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
        // The divider's open stretches show no line: each pane's picture
        // goes on into its half of the gap there.
        let fillers: Vec<Camera> = self.fillers();
        for filler in fillers.into_iter().filter(|_| take(1)) {
            push(&mut frame, filler);
        }
        for (k, inset) in self.insets.iter().enumerate().take(insets) {
            push(&mut frame, frame_of(inset.rect));
            let mut camera = inset.view.camera(inset.rect, overhang, bounds);
            camera.zoomed_art = self.zoomed_art(camera.zoom);
            camera.downscale = downscale;
            push(&mut frame, camera);
            self.shown.push((Hit::Inset(k), camera));
        }
        // The zoom box or strip shows the world, so it lies over the views
        // and under the driver's interface (whose menus stay usable); not
        // over a screen fitted into a pane. The magnifier beside the
        // pointer is on top.
        let fit_rect = fit.map(|f| if f.hit == Hit::Main { main_rect } else { pane_rect.unwrap_or(main_rect) });
        let magnifier = magnifier.filter(|m| m.place == ZoomPlace::Beside || (self.world_shown && !fit_rect.is_some_and(|f| overlap(f, self.magnifier_geometry(m).0))));
        self.magnifier_shown = magnifier.is_some();
        let zoom_box = magnifier.filter(|m| m.place != ZoomPlace::Beside);
        if let Some(m) = zoom_box {
            let (rect, _) = self.magnifier_geometry(&m);
            push(&mut frame, frame_of(rect));
            // The world under the pointer (the world cursor while the mouse
            // is captured; where it looked last while nothing is under it).
            let center = if self.locked { Some(self.cursor) } else { self.world_under(self.pointer) }.or(self.magnifier_center);
            self.magnifier_center = center;
            let mut camera = FreeView::new(center.unwrap_or((0, 0)), m.zoom, Follow::Free).camera(rect, overhang, None);
            camera.zoomed_art = self.zoomed_art(camera.zoom);
            camera.downscale = downscale;
            push(&mut frame, camera);
            self.shown.push((Hit::Magnifier, camera));
        }
        // The driver's other views and its interface on top, as it placed
        // them (unless they are in a pane).
        for (i, camera) in over {
            frame.view_cameras[i] = Some(push(&mut frame, camera));
        }
        if let Some(m) = magnifier.filter(|_| zoom_box.is_none()) {
            let (rect, source) = self.magnifier_geometry(&m);
            let zoom = ZOOMS[m.zoom];
            push(&mut frame, frame_of(rect));
            match source {
                // The window around the pointer, at the magnifier's zoom.
                Some(source) => frame.copies.push(ScreenCopy { source, at: (rect[0], rect[1]), zoom }),
                None => {
                    // The world under the pointer (the world cursor while
                    // the mouse is captured; where it looked last while
                    // nothing is under it).
                    let center = if self.locked { Some(self.cursor) } else { self.world_under(self.pointer) }.or(self.magnifier_center);
                    self.magnifier_center = center;
                    let mut camera = FreeView::new(center.unwrap_or((0, 0)), m.zoom, Follow::Free).camera(rect, overhang, None);
                    camera.zoomed_art = self.zoomed_art(camera.zoom);
                    camera.downscale = downscale;
                    push(&mut frame, camera);
                    self.shown.push((Hit::Magnifier, camera));
                }
            }
        }
        // The pointer on the screen that just opened: its main control (as
        // the driver names it), else its middle, where the screen shows.
        if std::mem::take(&mut self.pointer_to_screen) {
            let screen = screen_part.map(|i| rect_of(&base[i])).unwrap_or([0, 0, window.0 as i32, window.1 as i32]);
            let [x, y, w, h] = parts.focus_control.unwrap_or(screen);
            let p = self.driver_to_window((x + w / 2, y + h / 2));
            (self.pointer, self.raw_pointer, self.warp) = (p, p, Some(p));
            self.side = self.side_of(p);
            log::info!("viewer: the pointer on the screen's {} at {p:?}", if parts.focus_control.is_some() { "main control" } else { "middle" });
        }
        if self.follows_pointer() {
            frame.markers.push(Marker::Cursor(self.cursor));
        }
        if let (true, Some(f)) = (self.focus_arrow, scene.focus) {
            frame.markers.push(Marker::Focus(f));
        }
        frame
    }

    /// The side whose controls are shown: the one the pointer is in (the
    /// view following the world cursor while the mouse is captured).
    pub fn active_side(&self) -> Side {
        if self.locked && self.split.is_some_and(|s| s.view.follow == Follow::Pointer) && self.main_view.follow != Follow::Pointer {
            return Side::B;
        }
        if self.locked {
            return Side::A;
        }
        self.side.unwrap_or(Side::A)
    }

    /// How far the driver's roaming controls move: to the same place in B
    /// as they have in the window while B is the active view; in A they are
    /// where the driver put them (over a zoom strip too, which lies under
    /// the interface). They show in the active view only.
    fn roaming_shift(&self, pane_rect: Option<[i32; 4]>) -> Option<(i32, i32)> {
        Some(match (self.active_side(), pane_rect) {
            (Side::B, Some(b)) if self.split.is_some() => (b[0], b[1]),
            _ => (0, 0),
        })
    }

    /// A popup at `rect` opened. One the driver opened where a click went
    /// (a context menu: its corner at the click) goes where that click was
    /// made in the window (e.g. at the world cursor in B), inside the
    /// window. While it is open the mouse is let go, so that the pointer can
    /// reach it (captured again once it closes).
    fn popup_opened(&mut self, [x, y, w, h]: [i32; 4]) {
        self.popup_anchor = self.last_press.filter(|&(_, (qx, qy), t)| self.clock - t < 2.0 && (x - qx).abs() <= 40 && (y - qy).abs() <= 40).map(|((px, py), (qx, qy), _)| {
            let (ww, wh) = (self.window.0 as i32, self.window.1 as i32);
            ((x + px - qx).clamp(0, (ww - w).max(0)) - x, (y + py - qy).clamp(0, (wh - h).max(0)) - y)
        });
        if let Some(d) = self.popup_anchor {
            log::info!("viewer: the popup opened at the click: moved by {d:?}");
        }
        if self.locked {
            self.recapture = Some(self.cursor);
            self.release_here("a popup is open");
        }
    }

    /// Where driver window pixel `p` shows in the window (as of this frame):
    /// in the screen fitted into a pane, in a part drawn elsewhere, or where
    /// the driver has it.
    fn driver_to_window(&self, (x, y): (i32, i32)) -> (i32, i32) {
        if let Some(Fit { origin: (ox, oy), zoom: (n, d), .. }) = self.fit {
            let (n, d) = (n as i32, d.max(1) as i32);
            return (ox + x * n / d, oy + y * n / d);
        }
        match self.moved.iter().find(|(r, d)| inside([r[0] - d.0, r[1] - d.1, r[2], r[3]], (x, y))) {
            Some((_, (dx, dy))) => (x + dx, y + dy),
            None => (x, y),
        }
    }

    /// The popup closed: a mouse let go for it is captured again.
    fn popup_closed(&mut self) {
        self.popup_anchor = None;
        if let Some(cursor) = self.recapture.take().filter(|_| self.follows_pointer() && !self.locked) {
            self.cursor = cursor;
            self.locked = true;
            self.grab = Some(true);
            self.against = None;
            log::info!("viewer: mouse captured again (the popup closed)");
        }
    }

    /// How far the popup at `rect` moves: where the click it came from was
    /// made (see [`Layout::popup_opened`]); else, with the views' roles
    /// `UnitMouse` and a split, fully into the main pane if it fits there
    /// (the least way); else nowhere.
    fn popup_shift(&self, [x, y, w, h]: [i32; 4], a: [i32; 4]) -> (i32, i32) {
        if let Some(d) = self.popup_anchor {
            return d;
        }
        if self.roles != Roles::UnitMouse || self.split.is_none() || w > a[2] || h > a[3] {
            return (0, 0);
        }
        (x.clamp(a[0], a[0] + a[2] - w) - x, y.clamp(a[1], a[1] + a[3] - h) - y)
    }

    /// How far the driver's named screen at `rect` moves: below the zoom
    /// box or strip where it would be under it and there is room below (it
    /// lies over the views, under the driver's interface); else nowhere
    /// (the box or strip goes under it).
    fn screen_shift(&self, [x, y, w, h]: [i32; 4]) -> (i32, i32) {
        let Some(m) = self.magnifier.filter(|m| self.world_shown && matches!(m.place, ZoomPlace::Box(_) | ZoomPlace::Strip(_))) else { return (0, 0) };
        let boxed = self.magnifier_geometry(&m).0;
        let below = boxed[1] + boxed[3] + GAP;
        if overlap(boxed, [x, y, w, h]) && below > y && below + h <= self.window.1 as i32 { (0, below - y) } else { (0, 0) }
    }

    /// The parts of the driver's interface drawn elsewhere (as of the last
    /// frame): their window rectangle and how far they were moved.
    pub fn moved_parts(&self) -> &[([i32; 4], (i32, i32))] {
        if self.fit.is_some() { &[] } else { &self.moved }
    }

    /// What shows the panes' pictures in the divider's open stretches: for
    /// each pane showing the world (not a fitted screen), its camera cut to
    /// its half of the gap there.
    fn fillers(&self) -> Vec<Camera> {
        let (Some(((g0, g1), _)), Some(split)) = (self.gap(), self.split) else { return vec![] };
        let fitted = self.fit.map(|f| f.hit);
        let half = (g1 - g0) / 2;
        let mut out = Vec::new();
        for (a, b) in self.open_spans() {
            for (hit, from, to) in [(Hit::Main, g0, g0 + half), (Hit::Pane, g0 + half, g1)] {
                let Some((_, c)) = self.shown.iter().find(|(h, _)| *h == hit && Some(hit) != fitted) else { continue };
                if to <= from || c.stacked {
                    continue;
                }
                let (at, size) = match split.axis {
                    Axis::Vertical => ((from, a), ((to - from) as u32, (b - a) as u32)),
                    Axis::Horizontal => ((a, from), ((b - a) as u32, (to - from) as u32)),
                };
                let (x, y) = world_at_floor(c, at);
                out.push(Camera { x, y, origin: at, viewport: size, ..*c });
            }
        }
        out
    }

    /// Scrolls the view the pointer pushes against the divider with (unless
    /// the driver scrolls it, `Input::Edge`, or it shows a fitted screen):
    /// by [`EDGE_PAN_SPEED`] towards the divider.
    fn edge_pan(&mut self, dt: f64, fitted: Option<Hit>) {
        let (Some(side), Some(split)) = (self.against, self.split) else { return };
        if self.locked || dt <= 0.0 || fitted == Some(side.hit()) || (side == Side::A && self.main_unmoved()) {
            return;
        }
        let zoom = self.shown.iter().find(|(h, _)| *h == side.hit()).map_or((1, 1), |(_, c)| c.zoom);
        let step = (EDGE_PAN_SPEED * dt * zoom.1 as f64 / zoom.0.max(1) as f64).round().max(1.0) as i32;
        // Towards the divider: right (down) from the main pane, left (up)
        // from the second.
        let along = if side == Side::A { step } else { -step };
        let d = if split.axis == Axis::Vertical { (along, 0) } else { (0, along) };
        let focus = self.focus;
        let view = match side {
            Side::A => &mut self.main_view,
            Side::B => &mut self.split.as_mut().expect("split").view,
        };
        match view.follow {
            // The world cursor's view keeps it in its middle: it moves.
            Follow::Pointer => self.cursor = (self.cursor.0 + d.0, self.cursor.1 + d.1),
            // (Zoomed, the driver's view does not scroll at the viewer's
            // edges either.)
            Follow::Driver if view.zoom != ONE => {}
            _ => {
                let to = (view.center.0 + d.0, view.center.1 + d.1);
                view.moved_by_hand(to, focus);
            }
        }
    }

    /// The driver's main view as the main pane shows it: following the
    /// driver at zoom 1 it shows the driver's window pixels where they are
    /// (cropped to the pane; gliding back there with smooth movement), at
    /// other zooms it is zoomed about the driver's centre; otherwise it is
    /// placed like any arranged view. The driver's screen sprites (labels)
    /// stay on their world pixels.
    fn arranged_main(&self, camera: &Camera, rect: [i32; 4], overhang: (u32, u32, u32, u32)) -> Camera {
        let v = self.main_view;
        let driver = v.follow == Follow::Driver;
        let crop = (camera.x + rect[0] - camera.origin.0, camera.y + rect[1] - camera.origin.1);
        let unmoved = driver && v.zoom == ONE && v.center == (crop.0 + rect[2] / 2, crop.1 + rect[3] / 2);
        let (x, y, cull) = if unmoved {
            (crop.0, crop.1, camera.cull)
        } else {
            let placed = if driver { v } else { FreeView { follow: Follow::Free, ..v } };
            let c = placed.camera(rect, overhang, if driver { None } else { self.bounds });
            // Zoomed in on what the driver shows, its cell range still holds.
            let centred = v.center == (camera.x + camera.viewport.0 as i32 / 2, camera.y + camera.viewport.1 as i32 / 2);
            let cull = if driver && v.zoom > ONE && centred { camera.cull } else { None };
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
        if let Some(m) = self.magnifier.filter(|_| self.magnifier_shown) {
            if inside(self.magnifier_geometry(&m).0, p) {
                // The zoom box or strip lies under the driver's interface.
                if m.place != ZoomPlace::Beside {
                    if let Some(hit) = self.interface_at(p) {
                        return hit;
                    }
                }
                return Hit::Magnifier;
            }
        }
        self.hit_below_magnifier(p)
    }

    /// The driver's interface (and its other views) at `p`, where it lies
    /// over the window (not in a pane), the parts drawn elsewhere on top.
    fn interface_at(&self, p: (i32, i32)) -> Option<Hit> {
        if self.fit.is_some() {
            return None;
        }
        if let Some(k) = self.moved.iter().rposition(|&(r, _)| inside(r, p)) {
            return Some(Hit::Moved(k));
        }
        self.interface.iter().chain(&self.driver_rects).any(|&r| inside(r, p)).then_some(Hit::Interface)
    }

    /// [`Layout::hit`] as if there were no magnifier.
    fn hit_below_magnifier(&self, p: (i32, i32)) -> Hit {
        if let Some(hit) = self.interface_at(p) {
            return hit;
        }
        if let Some(k) = (0..self.insets_shown.min(self.insets.len())).rev().find(|&k| inside(self.insets[k].rect, p)) {
            return Hit::Inset(k);
        }
        // (The divider is a wall but where it is open.)
        let along = if self.split.is_some_and(|s| s.axis == Axis::Vertical) { p.1 } else { p.0 };
        if self.divider().is_some_and(|r| inside(r, p)) && self.open_spans().iter().any(|&(a, b)| along >= a && along < b) {
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

    /// The view a click at `p` captures the mouse in: one following the
    /// world cursor under it (also below a world magnifier, which shows what
    /// is under the pointer: a zoom box over the capturing view).
    fn capture_hit(&self, p: (i32, i32)) -> Option<Hit> {
        let hit = match self.hit(p) {
            Hit::Magnifier if self.magnifier.is_some_and(|m| self.magnifier_geometry(&m).1.is_none()) => self.hit_below_magnifier(p),
            h => h,
        };
        (matches!(hit, Hit::Main | Hit::Pane | Hit::Inset(_)) && self.follow_of(hit) == Some(Follow::Pointer)).then_some(hit)
    }

    /// The world pixel under window pixel `p`, if a world view shows one.
    fn world_under(&self, p: (i32, i32)) -> Option<(i32, i32)> {
        let (p, hit) = match self.hit(p) {
            Hit::Magnifier => {
                // Under a screen magnifier: what it magnifies; under a world
                // magnifier: what is below it (what it looks at).
                let m = self.magnifier?;
                let (rect, source) = self.magnifier_geometry(&m);
                match source {
                    Some(source) => {
                        let q = self.magnified(&m, rect, source, p);
                        (q, self.hit_below_magnifier(q))
                    }
                    None => (p, self.hit_below_magnifier(p)),
                }
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
        // (The same world pixel round a world that wraps: the copy the
        // driver's view shows.)
        let near = |v: i32, wraps: bool, size: i32| if wraps && size > 0 { v.rem_euclid(size) } else { v };
        let (wrap, size) = self.bounds.map_or(((false, false), (0, 0)), |b| (b.wrap, b.size));
        let (x, y) = (near(wx - m.x, wrap.0, size.0) + m.origin.0, near(wy - m.y, wrap.1, size.1) + m.origin.1);
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

    /// The world pixel under the system pointer while the mouse is free, if
    /// a view of the last frame shows one there (for drawing the cursor in
    /// the other views showing it).
    pub fn pointer_world(&self) -> Option<(i32, i32)> {
        if self.locked { None } else { self.world_under(self.pointer) }
    }

    /// The view following the world cursor in the last frame: its window
    /// rectangle (x, y, w, h), where the world cursor is in it, and whether
    /// the view is held at the edge of a world that does not wrap, so the
    /// cursor is off its middle (for the cursor's drawing, `crate::cursor`).
    pub fn world_cursor_view(&self) -> Option<([i32; 4], (i32, i32), bool)> {
        let (_, c) = self.shown.iter().find(|(h, _)| self.follow_of(*h) == Some(Follow::Pointer))?;
        let (n, d) = (c.zoom.0.max(1), c.zoom.1.max(1));
        let span = ((c.viewport.0 * d).div_ceil(n) as i32, (c.viewport.1 * d).div_ceil(n) as i32);
        let held = self.bounds.is_some_and(|b| b.clamp_center(0, self.cursor.0, span.0) != self.cursor.0 || b.clamp_center(1, self.cursor.1, span.1) != self.cursor.1);
        let rect = [c.origin.0, c.origin.1, c.viewport.0 as i32, c.viewport.1 as i32];
        Some((rect, self.cursor_on_window()?, held))
    }

    /// While the mouse is captured: the world cursor goes under window pixel
    /// `p` (kept inside the view following it), as the system pointer shows
    /// it while that view is held at the world's edge; what the driver gets.
    pub fn pointer_to(&mut self, p: (i32, i32)) -> Vec<Input> {
        let Some((_, c)) = self.shown.iter().find(|(h, _)| self.follow_of(*h) == Some(Follow::Pointer)).copied() else { return vec![] };
        if !self.locked {
            return vec![];
        }
        let x = p.0.clamp(c.origin.0, c.origin.0 + c.viewport.0 as i32 - 1);
        let y = p.1.clamp(c.origin.1, c.origin.1 + c.viewport.1 as i32 - 1);
        self.cursor = world_at(&c, (x, y));
        self.motion_rest = (0.0, 0.0);
        if let Some(b) = self.bounds {
            self.cursor = b.clamp_point(self.cursor);
        }
        match self.route_world(self.cursor) {
            Ok((x, y)) => vec![Input::MouseMove { x, y }],
            Err(_) => vec![],
        }
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
        self.fit.map_or(((0, 0), (1, 1)), |f| (f.origin, f.zoom))
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
            // Drawn elsewhere: where the driver put it.
            Hit::Moved(k) => {
                let (_, (dx, dy)) = *self.moved.get(k).ok_or(None)?;
                Ok((p.0 - dx, p.1 - dy))
            }
            // The driver's whole screen, scaled: its window pixel.
            hit if self.fit.is_some_and(|f| f.hit == hit) => {
                let Fit { origin: (ox, oy), zoom: (n, d), .. } = self.fit.ok_or(None)?;
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
                    // The world pixel it shows there.
                    None => {
                        let (_, camera) = self.shown.iter().find(|(h, _)| *h == Hit::Magnifier).ok_or(None)?;
                        self.route_world(world_at(camera, p))
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
        // (Ctrl held while the mouse moves is no tap: it does not let go.)
        if dx != 0.0 || dy != 0.0 {
            self.ctrl_alone = false;
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
        // Ctrl on its own (no other key, button or wheel while it is down)
        // releases a captured mouse when it comes up.
        match input {
            // (A key held repeats its press.)
            Input::Key { vk: VK_CONTROL, down: true, .. } if !self.keys_down.contains(&VK_CONTROL) => self.ctrl_alone = self.keys_down.is_empty(),
            Input::Key { vk: VK_CONTROL, down: true, .. } => {}
            Input::Key { vk: VK_CONTROL, down: false, .. } => {
                if std::mem::take(&mut self.ctrl_alone) && self.locked {
                    self.release_here("Ctrl on its own");
                }
            }
            Input::Key { down: true, .. } | Input::MouseButton { down: true, .. } | Input::Wheel { .. } => self.ctrl_alone = false,
            _ => {}
        }
        if let Input::Key { vk, down, .. } = input {
            self.keys_down.retain(|&k| k != vk);
            if down {
                self.keys_down.push(vk);
            }
        }
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
                let from = self.pointer;
                self.raw_pointer = (x, y);
                // The divider may hold it back (the system pointer is moved
                // back too).
                let (x, y) = self.constrain(from, (x, y));
                if (x, y) != self.raw_pointer {
                    log::debug!("viewer: the divider holds the pointer at {:?} (it went to {:?})", (x, y), self.raw_pointer);
                    self.warp = Some((x, y));
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
            Input::MouseButton { button, down, x, y } => {
                let p = self.held_back((x, y));
                self.button(button, down, p)
            }
            Input::Wheel { delta, x, y } => {
                let (x, y) = self.held_back((x, y));
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

    /// A window position of the pointer as the divider left it (where the
    /// window last reported the pointer, the pointer held back there).
    fn held_back(&self, p: (i32, i32)) -> (i32, i32) {
        if p == self.raw_pointer { self.pointer } else { p }
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
            (Button::Left, _) if !self.locked && self.capture_hit(p).is_some() => {
                let into = self.capture_hit(p).expect("checked");
                self.capture(into, p);
                return vec![];
            }
            _ => {}
        }
        match self.route_point(p) {
            Ok(q) => {
                self.pressed[b] = true;
                // (Where it was made in the window: the world cursor while
                // the mouse is captured.)
                let at = if self.locked { self.cursor_on_window().unwrap_or(p) } else { p };
                self.last_press = Some((at, q, self.clock));
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
                let focus = self.focus;
                if let Some(view) = self.free_view(hit) {
                    // Moved by hand: it follows nothing now (the focus of
                    // interest again once it moves).
                    view.moved_by_hand((center.0 - (p.0 - from.0) * d / n, center.1 - (p.1 - from.1) * d / n), focus);
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
                    self.release_here("Ctrl+Shift+Space");
                } else if let Some(into) = self.capture_hit(p) {
                    self.capture(into, p);
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
                let focus = self.focus;
                if let (Some(v), Some((w, h)), Some(center)) = (self.free_view(hit), size, center) {
                    let (dx, dy) = match vk {
                        0x25 => (-(w as i32) / 8, 0),
                        0x26 => (0, -(h as i32) / 8),
                        0x27 => (w as i32 / 8, 0),
                        _ => (0, h as i32 / 8),
                    };
                    v.moved_by_hand((center.0 + dx, center.1 + dy), focus);
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
        // The panes, the pictures going on into the divider's open middle
        // third, the interface.
        assert_eq!(f.cameras.len(), 5);
        assert_eq!(f.cameras[0].viewport, (400, 600));
        assert_eq!(f.cameras[1].origin, (404, 0));
        assert_eq!((f.cameras[2].origin, f.cameras[2].viewport), ((400, 200), (2, 200)));
        assert_eq!((f.cameras[3].origin, f.cameras[3].viewport), ((402, 200), (2, 200)));
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
        // Dragging the divider (where it is open) moves it, nothing reaches
        // the driver.
        assert!(l.route(Input::MouseButton { button: Button::Left, down: true, x: 401, y: 300 }).is_empty());
        assert!(l.route(Input::MouseMove { x: 200, y: 300 }).is_empty());
        assert!(l.route(Input::MouseButton { button: Button::Left, down: false, x: 200, y: 300 }).is_empty());
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
        assert_eq!(click(&mut l, (100, 100))[0], Input::MouseButton { button: Button::Left, down: true, x: 300, y: 100 });
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
            // (The pictures going on into the divider's middle third: the
            // pane's, and the main pane's unless it is the fitted screen.)
            let fillers = if ui == "pane" { 1 } else { 2 };
            assert_eq!(f.cameras.len(), 2 + 1 + 2 * MAX_INSETS + 2 + fillers, "ui={ui}");
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
        assert_eq!(click(&mut l, (100, 300))[0], Input::MouseButton { button: Button::Left, down: true, x: 300, y: 300 });
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
        // (Bar the divider's fillers, cut out of the panes' cameras.)
        let world: Vec<_> = f.cameras.iter().filter(|c| c.cells).collect();
        assert_eq!(world.iter().filter(|c| c.viewport.0 > GAP as u32).count(), 4, "{:?}", f.cameras);
        assert!(world.iter().all(|c| c.downscale == 2), "{world:?}");
        assert_eq!(f.cameras[0].zoom, (2, 1));
        // The interface stays as it is.
        assert_eq!(f.cameras[f.view_cameras[1].unwrap() as usize].downscale, 1);
    }

    // ---- Driven by the settings -------------------------------------------

    use viewer_settings::{Layout as Settings, Split as S, Zoom};

    fn settings(f: impl FnOnce(&mut Settings)) -> Settings {
        let mut s = Settings::default();
        f(&mut s);
        s
    }

    /// A layout set up by `s`, one frame drawn.
    fn arranged(s: &Settings) -> Layout {
        let mut l = Layout::default();
        l.apply_settings(s);
        frame(&mut l, vec![]);
        l
    }

    fn mv(l: &mut Layout, p: (i32, i32)) -> Vec<Input> {
        l.route(Input::MouseMove { x: p.0, y: p.1 })
    }

    fn key(l: &mut Layout, vk: u32, down: bool) -> Vec<Input> {
        l.route(Input::Key { vk, down, text: None })
    }

    fn camera_at(f: &Frame, origin: (i32, i32)) -> Camera {
        *f.cameras.iter().find(|c| c.origin == origin && c.cells).unwrap_or_else(|| panic!("no camera at {origin:?}: {:?}", f.cameras))
    }

    fn rect(c: &Camera) -> [i32; 4] {
        [c.origin.0, c.origin.1, c.viewport.0 as i32, c.viewport.1 as i32]
    }

    #[test]
    fn what_a_shows_of_the_drivers_view_is_reported_where_it_is_not_all_of_it() {
        // One view, the driver's unmoved: all of it.
        let mut l = arranged(&Settings::default());
        assert_eq!(l.take_main_view_change(), None);
        // A split: A's half (the driver's view is at 1000, 500, 800x600).
        let mut l = arranged(&settings(|s| s.split = S::Vertical1To1));
        assert_eq!(l.take_main_view_change(), Some((1200, 500, 400, 600)));
        assert_eq!(l.take_main_view_change(), None);
        // Zoomed in 2/1 in one view: half as much each way, about its middle.
        let mut l = arranged(&Settings::default());
        l.main_view.zoom = ONE + 2;
        frame(&mut l, vec![]);
        assert_eq!(l.take_main_view_change(), Some((1200, 650, 400, 300)));
    }

    #[test]
    fn a_click_on_a_world_pixel_the_drivers_view_shows_round_the_wrap_goes_to_it() {
        // The driver's view at x 1000..1800 of a world 8000 wide that wraps;
        // B looks at the same place one turn further round.
        let mut l = arranged(&settings(|s| s.split = S::Vertical1To1));
        l.split.as_mut().unwrap().view.center = (9400, 800);
        frame(&mut l, vec![]);
        assert_eq!(click(&mut l, (602, 300))[0], Input::MouseButton { button: Button::Left, down: true, x: 400, y: 300 });
    }

    #[test]
    fn the_settings_arrange_the_splits() {
        for (split, a, b) in [
            (S::Standard, [0, 0, 800, 600], None),
            (S::Vertical1To1, [0, 0, 400, 600], Some([404, 0, 396, 600])),
            // A twice as wide as B.
            (S::Vertical2To1, [0, 0, 533, 600], Some([537, 0, 263, 600])),
            (S::Horizontal1To1, [0, 0, 800, 300], Some([0, 304, 800, 296])),
        ] {
            let mut l = arranged(&settings(|s| s.split = split));
            let f = frame(&mut l, vec![]);
            assert_eq!(rect(&f.cameras[0]), a, "{split:?}");
            assert_eq!(l.panes(), (a, b), "{split:?}");
            if let Some(b) = b {
                assert_eq!(rect(&camera_at(&f, (b[0], b[1]))), b, "{split:?}");
            }
            // A shows the middle of the driver's view in its middle (the
            // driver's view, 800x600 at 1000, 500, cropped round it).
            assert_eq!((f.cameras[0].x, f.cameras[0].y), (1400 - a[2] / 2, 800 - a[3] / 2), "{split:?}");
        }
    }

    #[test]
    fn the_settings_say_what_the_views_follow() {
        let free = arranged(&settings(|s| s.split = S::Vertical1To1));
        assert_eq!((free.main_view.follow, free.split.unwrap().view.follow), (Follow::Driver, Follow::Free));
        let mut l = arranged(&settings(|s| {
            s.split = S::Vertical1To1;
            s.roles = Roles::UnitMouse;
            s.smooth_world_movement = true;
        }));
        assert_eq!((l.main_view.follow, l.split.unwrap().view.follow), (Follow::Focus, Follow::Pointer));
        assert!(l.smooth);
        // A follows the active unit (scene focus (3000, 2000)).
        let f = frame(&mut l, vec![]);
        let (sw, sh) = f.cameras[0].source_size();
        assert_eq!((f.cameras[0].x + sw as i32 / 2, f.cameras[0].y + sh as i32 / 2), (3000, 2000));
        // Back to free: the captured mouse goes free, the panes keep where
        // they look.
        click(&mut l, (600, 300));
        assert!(l.locked);
        l.take_grab();
        let center = l.split.unwrap().view.center;
        l.apply_settings(&settings(|s| s.split = S::Vertical1To1));
        assert!(!l.locked);
        assert_eq!(l.take_grab(), Some(false));
        assert_eq!(l.split.unwrap().view.center, center);
        // One view following the unit.
        let one = arranged(&settings(|s| s.roles = Roles::UnitMouse));
        assert_eq!((one.main_view.follow, one.split), (Follow::Focus, None));
    }

    #[test]
    fn the_spec_overrides_the_settings_for_testing() {
        let mut l = Layout::default();
        l.apply_settings(&settings(|s| s.split = S::Vertical2To1));
        l.configure("split=horizontal,fraction=0.25").unwrap();
        let s = l.split.unwrap();
        assert_eq!((s.axis, s.fraction), (Axis::Horizontal, 0.25));
    }

    #[test]
    fn zoom_boxes_float_or_hover_about_the_pointer() {
        let geometry = |s: &Settings, pointer: (i32, i32)| {
            let mut l = arranged(s);
            mv(&mut l, pointer);
            let f = frame(&mut l, vec![]);
            let m = l.magnifier.unwrap();
            assert_eq!(m.mode, MagnifierMode::World);
            let (r, source) = l.magnifier_geometry(&m);
            assert_eq!(source, None);
            // A world view in the box, centred on what is under the pointer.
            let c = camera_at(&f, (r[0], r[1]));
            assert_eq!((c.zoom, rect(&c)), ((2, 1), r));
            r
        };
        // 1:3:1 and 1:7:1 of the window, in its middle.
        assert_eq!(geometry(&settings(|s| s.zoom = Zoom::Floating131), (10, 10)), [160, 120, 480, 360]);
        assert_eq!(geometry(&settings(|s| s.zoom = Zoom::Floating171), (10, 10)), [89, 67, 622, 466]);
        // In a vertical split, over A or B as `zoom_place` says.
        let left = settings(|s| (s.split, s.zoom, s.zoom_place) = (S::Vertical1To1, Zoom::Floating131, Place::LeftOrBottom));
        assert_eq!(geometry(&left, (10, 10)), [80, 120, 240, 360]);
        let right = Settings { zoom_place: Place::RightOrTop, ..left.clone() };
        assert_eq!(geometry(&right, (10, 10)), [483, 120, 237, 360]);
        let over = Settings { zoom_place: Place::Floating, ..left.clone() };
        assert_eq!(geometry(&over, (10, 10)), [160, 120, 480, 360]);
        // (Not in a horizontal split.)
        let stacked = Settings { split: S::Horizontal1To1, ..right.clone() };
        assert_eq!(geometry(&stacked, (10, 10)), [160, 120, 480, 360]);
        // About the pointer: 1:1:1, 1:3:1, kept inside the region.
        assert_eq!(geometry(&settings(|s| s.zoom = Zoom::Mouse111), (400, 300)), [267, 200, 266, 200]);
        assert_eq!(geometry(&settings(|s| s.zoom = Zoom::Mouse131), (400, 300)), [160, 120, 480, 360]);
        assert_eq!(geometry(&settings(|s| s.zoom = Zoom::Mouse111), (790, 5)), [534, 0, 266, 200]);
        let mouse_right = Settings { zoom: Zoom::Mouse111, ..right };
        assert_eq!(geometry(&mouse_right, (420, 590)), [404, 400, 132, 200]);
    }

    #[test]
    fn a_click_into_a_zoom_box_acts_on_the_world_it_shows() {
        let mut l = arranged(&settings(|s| s.zoom = Zoom::Mouse111));
        mv(&mut l, (400, 300));
        let f = frame(&mut l, vec![]);
        // The box is centred on the world under the pointer (driver pixel
        // (400, 300), world (1400, 800)); at 2/1 a click 20 pixels right of
        // its middle is 10 world pixels right.
        let c = camera_at(&f, (267, 200));
        assert_eq!(world_at(&c, (400, 300)), (1400, 800));
        assert_eq!(click(&mut l, (420, 300))[0], Input::MouseButton { button: Button::Left, down: true, x: 410, y: 300 });
    }

    #[test]
    fn the_drivers_interface_stays_over_a_zoom_box_or_strip_and_takes_its_clicks() {
        // A screen of the driver's over the middle of the window, partly
        // over the box (1:3:1: 160..640 x 120..480).
        let screen = vec![[100, 100, 300, 200]];
        for zoom in [Zoom::Floating131, Zoom::TopStrip] {
            let mut l = arranged(&settings(|s| s.zoom = zoom));
            let f = frame(&mut l, screen.clone());
            let (r, _) = l.magnifier_geometry(&l.magnifier.unwrap());
            let boxed = f.cameras.iter().position(|c| c.cells && c.origin == (r[0], r[1]) && c.zoom == (2, 1)).unwrap();
            assert!(f.view_cameras[1].unwrap() as usize > boxed, "{zoom:?}: the interface is drawn after the box");
            assert_eq!(click(&mut l, (200, 130))[0], Input::MouseButton { button: Button::Left, down: true, x: 200, y: 130 }, "{zoom:?}");
            // Beside it, the box's world.
            assert_ne!(click(&mut l, (500, 130))[0], Input::MouseButton { button: Button::Left, down: true, x: 500, y: 130 }, "{zoom:?}");
        }
        // Not over the driver's screen fitted into the view it is in.
        let mut l = arranged(&settings(|s| (s.split, s.zoom, s.zoom_place) = (S::Vertical1To1, Zoom::Floating131, Place::RightOrTop)));
        l.screen_places.insert("F1".into(), ScreenPlace { vertical: Place::RightOrTop, horizontal: Place::Floating });
        let mut s = scene();
        s.interface.screen = Some("F1".into());
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(!f.cameras.iter().any(|c| c.cells && c.zoom == (2, 1) && c.origin == (483, 120)), "{:?}", f.cameras);
        assert_eq!(l.hit((600, 300)), Hit::Pane);
    }

    #[test]
    fn no_zoom_box_or_strip_without_a_world() {
        let title = Scene { shape: scene::WorldShape::default(), ..scene() };
        for zoom in [Zoom::Floating131, Zoom::TopStrip] {
            let mut l = arranged(&settings(|s| s.zoom = zoom));
            let f = l.frame(&title, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            assert!(!f.cameras.iter().any(|c| c.zoom == (2, 1)), "{zoom:?}: {:?}", f.cameras);
            // The window is the driver's as without a zoom box.
            assert_eq!(l.panes().0, [0, 0, 800, 600]);
            assert_eq!(l.hit((400, 300)), Hit::Main);
            // Once a world shows: there.
            let f = frame(&mut l, vec![]);
            assert!(f.cameras.iter().any(|c| c.zoom == (2, 1)), "{zoom:?}");
        }
    }

    #[test]
    fn a_screen_over_the_window_goes_below_the_zoom_strip_where_there_is_room() {
        let mut s = with_screen("F1");
        let part = scene::ViewState { id: 258, view: scene::View { interface_only: true, origin: (100, 100), viewport: (600, 400), ..Default::default() }, overlay: vec![] };
        s.views.push(part);
        s.interface.screen_view = Some(258);
        let screen_camera = |f: &Frame| f.cameras[f.view_cameras[2].unwrap() as usize];
        // Below the strip (150 high): moved down, clicks go where the driver has it.
        let mut l = arranged(&settings(|s| (s.zoom, s.zoom_strip) = (Zoom::TopStrip, 0.25)));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!((screen_camera(&f).origin, screen_camera(&f).screen_shift), ((100, 154), (0, 54)));
        assert_eq!(click(&mut l, (300, 300))[0], Input::MouseButton { button: Button::Left, down: true, x: 300, y: 246 });
        // Its top, under the strip where it was: the strip's world.
        assert!(!matches!(click(&mut l, (300, 120))[0], Input::MouseButton { x: 300, y: 120, .. }));
        // A box in the middle leaves no room below: it stays, over the box.
        let mut l = arranged(&settings(|s| s.zoom = Zoom::Floating131));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(screen_camera(&f).screen_shift, (0, 0));
        assert!(f.view_cameras[2].unwrap() as usize > f.cameras.iter().position(|c| c.zoom == (2, 1) && c.origin == (160, 120)).unwrap());
        assert_eq!(click(&mut l, (300, 300))[0], Input::MouseButton { button: Button::Left, down: true, x: 300, y: 300 });
        // No zoom box: where the driver put it.
        let mut l = arranged(&Settings::default());
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!((screen_camera(&f).origin, screen_camera(&f).screen_shift), ((100, 100), (0, 0)));
    }

    #[test]
    fn the_zoom_strip_takes_the_top_and_the_views_give_it_room() {
        let s = settings(|s| (s.zoom, s.zoom_strip) = (Zoom::TopStrip, 0.25));
        let mut l = arranged(&s);
        let f = frame(&mut l, vec![]);
        assert_eq!(l.magnifier_geometry(&l.magnifier.unwrap()).0, [0, 0, 800, 150]);
        assert_eq!(rect(&f.cameras[0]), [0, 154, 800, 446]);
        // The driver's view is cropped, not moved: its window pixels stay.
        assert_eq!((f.cameras[0].x, f.cameras[0].y), (1000, 577));
        assert!(l.main_unmoved());
        // In a vertical split over one column, the other keeps its height.
        let left = settings(|s| (s.split, s.zoom, s.zoom_place, s.zoom_strip) = (S::Vertical1To1, Zoom::TopStrip, Place::LeftOrBottom, 0.5));
        let l = arranged(&left);
        assert_eq!(l.magnifier_geometry(&l.magnifier.unwrap()).0, [0, 0, 400, 300]);
        assert_eq!(l.panes(), ([0, 304, 400, 296], Some([404, 0, 396, 600])));
        let l = arranged(&Settings { zoom_place: Place::RightOrTop, ..left.clone() });
        assert_eq!(l.panes(), ([0, 0, 400, 600], Some([404, 304, 396, 296])));
        let l = arranged(&Settings { zoom_place: Place::Floating, ..left });
        assert_eq!(l.panes(), ([0, 304, 400, 296], Some([404, 304, 396, 296])));
        // Clamped to 10..50 %.
        let l = arranged(&settings(|s| (s.zoom, s.zoom_strip) = (Zoom::TopStrip, 0.9)));
        assert_eq!(l.magnifier_geometry(&l.magnifier.unwrap()).0, [0, 0, 800, 300]);
    }

    /// The scene with screen `kind` named open.
    fn with_screen(kind: &str) -> Scene {
        let mut s = scene();
        s.interface.screen = Some(kind.into());
        s
    }

    #[test]
    fn named_screens_go_where_the_settings_put_them() {
        let place = |vertical, horizontal| ScreenPlace { vertical, horizontal };
        let mut s = settings(|s| s.split = S::Vertical1To1);
        s.screens.insert("F1".into(), place(Place::LeftOrBottom, Place::LeftOrBottom));
        s.screens.insert("City".into(), place(Place::RightOrTop, Place::RightOrTop));
        s.screens.insert("Help".into(), place(Place::Floating, Place::Floating));
        let fitted = |s: &Settings, kind: &str| {
            let mut l = Layout::default();
            l.apply_settings(s);
            l.frame(&with_screen(kind), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            let f = l.frame(&with_screen(kind), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            (l.fit, f)
        };
        // Left: the driver's whole screen in A.
        let (fit, f) = fitted(&s, "F1");
        assert_eq!(fit, Some(Fit { hit: Hit::Main, origin: (0, 0), zoom: (1, 2) }));
        assert!(f.cameras[1].stacked);
        // Right: in B (fitted to its size); A shows the world the driver's
        // view shows there.
        let (fit, f) = fitted(&s, "City");
        assert_eq!(fit, Some(Fit { hit: Hit::Pane, origin: (404, 0), zoom: (99, 200) }));
        assert_eq!((f.cameras[0].origin, f.cameras[0].zoom), ((404, 0), (99, 200)));
        let a = camera_at(&f, (0, 0));
        assert_eq!((rect(&a), a.x, a.y), ([0, 0, 400, 600], 1200, 500));
        // Floating, unnamed and unknown screens: over the window.
        assert_eq!(fitted(&s, "Help").0, None);
        assert_eq!(fitted(&s, "F7").0, None);
        // Horizontal: left-or-bottom is B (bottom), right-or-top is A (top).
        let h = Settings { split: S::Horizontal1To1, ..s.clone() };
        assert_eq!(fitted(&h, "F1").0.map(|f| f.hit), Some(Hit::Pane));
        assert_eq!(fitted(&h, "City").0.map(|f| f.hit), Some(Hit::Main));
        // One view: nowhere to put it.
        let one = Settings { split: S::Standard, ..s };
        assert_eq!(fitted(&one, "F1").0, None);
    }

    #[test]
    fn a_screen_fitted_into_b_takes_its_clicks_and_a_shows_the_world() {
        let mut s = settings(|s| s.split = S::Vertical1To1);
        s.screens.insert("F4".into(), ScreenPlace { vertical: Place::RightOrTop, horizontal: Place::Floating });
        let mut l = Layout::default();
        l.apply_settings(&s);
        l.frame(&with_screen("F4"), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        l.frame(&with_screen("F4"), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        // B's pixel (404 + 99, 100) is the driver's (200, 202).
        assert_eq!(click(&mut l, (503, 100))[0], Input::MouseButton { button: Button::Left, down: true, x: 200, y: 202 });
        // A is a view of the viewer's own now: the driver marks it.
        assert_eq!(l.take_views_change().unwrap()[0], (1200, 500, 400, 600));
        // No divider scrolling into the fitted screen; A is the viewer's.
        assert!(!l.main_unmoved());
    }

    // ---- The divider ------------------------------------------------------

    fn split_1to1() -> Layout {
        arranged(&settings(|s| s.split = S::Vertical1To1))
    }

    #[test]
    fn the_divider_is_open_in_its_middle_third_and_drawn_without_a_line_there() {
        let mut l = split_1to1();
        assert_eq!(l.gap(), Some(((400, 404), (0, 600))));
        assert_eq!(l.open_spans(), [(200, 400)]);
        let f = frame(&mut l, vec![]);
        let fillers: Vec<_> = f.cameras.iter().filter(|c| c.viewport.0 == 2).map(|c| (c.origin, c.viewport, (c.x, c.y))).collect();
        // A's picture goes on two pixels into the gap, B's from the other
        // side, the same world pixels as next to them.
        let b = camera_at(&f, (404, 0));
        assert_eq!(fillers, [((400, 200), (2, 200), (1600, 700)), ((402, 200), (2, 200), (b.x - 2, b.y + 200))]);
        // Horizontal: the same across.
        let mut h = arranged(&settings(|s| s.split = S::Horizontal1To1));
        assert_eq!(h.open_spans(), [(266, 534)]);
        let f = frame(&mut h, vec![]);
        assert!(f.cameras.iter().any(|c| c.origin == (266, 300) && c.viewport == (268, 2)), "{:?}", f.cameras);
    }

    #[test]
    fn the_divider_holds_the_pointer_back_and_scrolls_the_view_instead() {
        let mut l = split_1to1();
        mv(&mut l, (100, 100));
        assert_eq!(l.side, Some(Side::A));
        // Across the closed upper third: held at A's last pixel, the
        // system pointer moved back, and the driver's view (shown unmoved)
        // scrolls as at its right edge.
        let out = mv(&mut l, (600, 100));
        assert_eq!(out, [Input::MouseMove { x: 599, y: 100 }, Input::Edge { x: 1, y: 0 }]);
        assert_eq!(l.take_warp(), Some((399, 100)));
        assert_eq!((l.side, l.against), (Some(Side::A), Some(Side::A)));
        // The window moving the pointer back reports it there: it still
        // rests against the wall (no new edge).
        assert_eq!(mv(&mut l, (399, 100)), [Input::MouseMove { x: 599, y: 100 }]);
        assert_eq!(l.against, Some(Side::A));
        // Clicks go where it was held.
        l.route(Input::MouseMove { x: 650, y: 120 });
        assert_eq!(l.take_warp(), Some((399, 120)));
        assert_eq!(l.route(Input::MouseButton { button: Button::Left, down: true, x: 650, y: 120 }), [Input::MouseButton {
            button: Button::Left,
            down: true,
            x: 599,
            y: 120
        }]);
        l.route(Input::MouseButton { button: Button::Left, down: false, x: 650, y: 120 });
        // Away from it: no more scrolling.
        assert_eq!(mv(&mut l, (300, 100)), [Input::MouseMove { x: 500, y: 100 }, Input::Edge { x: 0, y: 0 }]);
        assert_eq!(l.against, None);
        // Through the open middle third: freely into B, neither view moves.
        mv(&mut l, (300, 300));
        let before = frame(&mut l, vec![]);
        let out = mv(&mut l, (600, 300));
        assert!(out.iter().all(|i| !matches!(i, Input::Edge { x, .. } if *x != 0)), "{out:?}");
        assert_eq!((l.take_warp(), l.side), (None, Some(Side::B)));
        l.advance(std::time::Duration::from_millis(100));
        assert_eq!(frame(&mut l, vec![]).cameras[..2], before.cameras[..2]);
        // From B across the lower third: held at B's first pixel; the
        // viewer scrolls B (moved by hand, 600 pixels a second) towards A.
        let center = l.split.unwrap().view.center;
        mv(&mut l, (600, 500));
        mv(&mut l, (100, 500));
        assert_eq!(l.take_warp(), Some((404, 500)));
        assert_eq!(l.against, Some(Side::B));
        l.advance(std::time::Duration::from_millis(100));
        frame(&mut l, vec![]);
        assert_eq!(l.split.unwrap().view.center, (center.0 - 60, center.1));
        assert_eq!(l.split.unwrap().view.follow, Follow::Free);
    }

    #[test]
    fn the_pointer_moves_through_the_gap_only_where_it_is_open() {
        let mut l = split_1to1();
        mv(&mut l, (100, 300));
        // Into the gap through the open third (on the divider: nothing for
        // the driver), then along it out of the open stretch: held at A's
        // side.
        assert_eq!(mv(&mut l, (401, 300)), []);
        assert_eq!((l.pointer, l.side), ((401, 300), Some(Side::A)));
        mv(&mut l, (401, 100));
        assert_eq!(l.pointer, (399, 100));
        // On from the gap into B through the open stretch.
        mv(&mut l, (401, 300));
        mv(&mut l, (450, 350));
        assert_eq!((l.pointer, l.side), ((450, 350), Some(Side::B)));
        // A diagonal move crossing the wall above the open stretch: held,
        // though it ends level with it.
        let mut l = split_1to1();
        mv(&mut l, (390, 0));
        mv(&mut l, (500, 300));
        assert_eq!(l.pointer, (399, 300));
        // Coming into the window (side not known): wherever it enters.
        let mut l = split_1to1();
        mv(&mut l, (100, 100));
        l.pointer_gone();
        mv(&mut l, (700, 50));
        assert_eq!((l.pointer, l.side), ((700, 50), Some(Side::B)));
    }

    #[test]
    fn the_pointer_crosses_the_divider_where_the_drivers_interface_lies_over_it() {
        let mut l = split_1to1();
        // A popup across the divider, low in the window (the divider's
        // closed lower third), and a screen the driver put across it high up.
        let mut s = with_parts(None);
        let part = |id, origin, viewport| scene::ViewState { id, view: scene::View { interface_only: true, origin, viewport, ..Default::default() }, overlay: vec![] };
        s.views.push(part(257, (300, 450), (450, 100)));
        s.interface.popup = Some(257);
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![[200, 20, 400, 100]], BOUNDS);
        // Over the popup: straight across, and clicks reach it there.
        mv(&mut l, (350, 500));
        mv(&mut l, (600, 500));
        assert_eq!((l.pointer, l.side, l.take_warp()), ((600, 500), Some(Side::B), None));
        assert_eq!(click(&mut l, (600, 500))[0], Input::MouseButton { button: Button::Left, down: true, x: 600, y: 500 });
        // And back over the screen high up.
        mv(&mut l, (500, 60));
        mv(&mut l, (250, 60));
        assert_eq!((l.pointer, l.side), ((250, 60), Some(Side::A)));
        // Beside them the wall holds.
        mv(&mut l, (250, 580));
        mv(&mut l, (600, 580));
        assert_eq!(l.pointer, (399, 580));
        // Over a part only on one side: the wall too.
        let mut l = split_1to1();
        let mut s = with_parts(None);
        s.views.push(part(257, (100, 450), (250, 100)));
        s.interface.popup = Some(257);
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        mv(&mut l, (300, 500));
        mv(&mut l, (600, 500));
        assert_eq!(l.pointer, (399, 500));
    }

    #[test]
    fn where_the_pointer_cannot_be_moved_back_the_divider_lets_it_through() {
        let mut l = split_1to1();
        mv(&mut l, (100, 100));
        mv(&mut l, (600, 100));
        assert_eq!((l.pointer, l.take_warp()), ((399, 100), Some((399, 100))));
        // The window says it cannot: the pointer is where the user sees it.
        l.pointer_cannot_be_moved();
        assert_eq!((l.pointer, l.side, l.against), ((600, 100), Some(Side::B), None));
        mv(&mut l, (100, 100));
        assert_eq!((l.pointer, l.side, l.take_warp()), ((100, 100), Some(Side::A), None));
        assert_eq!(click(&mut l, (100, 100))[0], Input::MouseButton { button: Button::Left, down: true, x: 300, y: 100 });
    }

    #[test]
    fn the_unit_orders_are_reachable_from_both_views() {
        let mut l = split_1to1();
        let mut s = scene();
        // The orders' buttons across the divider, low in the window.
        s.interface.reachable = Some([340, 520, 128, 64]);
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.open_spans(), [(200, 400), (520, 584)]);
        assert_eq!(f.cameras.iter().filter(|c| c.viewport == (2, 64) && c.origin.1 == 520).count(), 2);
        // From A into them and on into B.
        mv(&mut l, (300, 540));
        mv(&mut l, (420, 540));
        assert_eq!((l.pointer, l.side), ((420, 540), Some(Side::B)));
        mv(&mut l, (600, 590));
        assert_eq!(l.pointer, (600, 590));
        // And back.
        mv(&mut l, (420, 560));
        mv(&mut l, (380, 560));
        assert_eq!((l.pointer, l.side), ((380, 560), Some(Side::A)));
        // Below them the divider holds.
        mv(&mut l, (600, 560));
        mv(&mut l, (600, 595));
        mv(&mut l, (100, 595));
        assert_eq!(l.pointer, (404, 595));
        // Not reaching over the gap: no gap of their own.
        let mut s = scene();
        s.interface.reachable = Some([100, 520, 128, 64]);
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.open_spans(), [(200, 400)]);
    }

    #[test]
    fn pushing_against_the_divider_from_a_view_following_the_unit_holds_it_until_the_unit_moves() {
        let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
        frame(&mut l, vec![]);
        mv(&mut l, (100, 100));
        mv(&mut l, (600, 100));
        assert_eq!(l.against, Some(Side::A));
        l.advance(std::time::Duration::from_millis(50));
        frame(&mut l, vec![]);
        assert_eq!(l.main_view.center, (3030, 2000));
        assert_eq!(l.main_view.follow, Follow::Focus);
        mv(&mut l, (300, 100));
        frame(&mut l, vec![]);
        assert_eq!(l.main_view.center, (3030, 2000));
        // The unit moves on: A follows it again.
        let mut s = scene();
        s.focus = Some((3200, 2100));
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.main_view.center, (3200, 2100));
    }

    // ---- Roaming controls and popups --------------------------------------

    /// The scene with the corner's buttons in a view of their own (the
    /// rectangle (0, 0, 150, 50)) and, if asked, a popup at (300, 200),
    /// 300 × 150 (across the divider of a 1:1 split).
    fn with_parts(popup_width: Option<u32>) -> Scene {
        let mut s = scene();
        let part = |id, origin, viewport| ViewState { id, view: View { interface_only: true, origin, viewport, ..Default::default() }, overlay: vec![] };
        s.views.push(part(256, (0, 0), (150, 50)));
        s.interface.roaming = Some(256);
        if let Some(w) = popup_width {
            s.views.push(part(257, (300, 200), (w, 150)));
            s.interface.popup = Some(257);
        }
        s
    }

    #[test]
    fn the_corner_buttons_are_in_the_view_the_pointer_is_in() {
        let mut l = split_1to1();
        let s = with_parts(None);
        mv(&mut l, (100, 100));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        // In A they are where the driver put them (the driver's interface
        // has them no more: only the active view shows them).
        let c = f.cameras[f.view_cameras[2].unwrap() as usize];
        assert_eq!((c.origin, c.screen_shift), ((0, 0), (0, 0)));
        assert_eq!(click(&mut l, (30, 20))[0], Input::MouseButton { button: Button::Left, down: true, x: 30, y: 20 });
        // In B: at B's top-left corner, drawn there; clicks there reach
        // them where the driver has them.
        mv(&mut l, (600, 300));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let c = f.cameras[f.view_cameras[2].unwrap() as usize];
        assert_eq!((c.origin, c.viewport, c.screen_shift), ((404, 0), (150, 50), (404, 0)));
        assert_eq!(click(&mut l, (434, 20))[0], Input::MouseButton { button: Button::Left, down: true, x: 30, y: 20 });
        // Captured in B (it follows the world cursor): there too.
        let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
        click(&mut l, (600, 300));
        assert_eq!(l.active_side(), Side::B);
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.cameras[f.view_cameras[2].unwrap() as usize].origin, (404, 0));
        // With a zoom strip, over it where the driver put them (the strip
        // lies under the interface).
        let mut l = arranged(&settings(|s| s.zoom = Zoom::TopStrip));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.cameras[f.view_cameras[2].unwrap() as usize].origin, (0, 0));
        assert!(f.view_cameras[2].unwrap() as usize > f.cameras.iter().position(|c| c.zoom == (2, 1)).unwrap());
    }

    #[test]
    fn a_popup_moves_fully_into_a_where_it_fits() {
        let unit_mouse = settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse));
        let mut l = arranged(&unit_mouse);
        let s = with_parts(Some(300));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let c = f.cameras[f.view_cameras[3].unwrap() as usize];
        // The least way: its right edge at A's.
        assert_eq!((c.origin, c.screen_shift), ((100, 200), (-200, 0)));
        // A click on it goes where the driver has it; where it was, B's
        // world.
        assert_eq!(click(&mut l, (150, 250))[0], Input::MouseButton { button: Button::Left, down: true, x: 350, y: 250 });
        assert_eq!(l.hit((550, 250)), Hit::Pane);
        // Too wide for A: where the driver put it.
        let mut l = arranged(&unit_mouse);
        let f = l.frame(&with_parts(Some(450)), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.cameras[f.view_cameras[3].unwrap() as usize].origin, (300, 200));
        assert_eq!(click(&mut l, (550, 250))[0], Input::MouseButton { button: Button::Left, down: true, x: 550, y: 250 });
        // With free roles it floats.
        let mut l = split_1to1();
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.cameras[f.view_cameras[3].unwrap() as usize].screen_shift, (0, 0));
    }

    #[test]
    fn a_context_menu_opens_where_the_click_was_made_and_lets_the_mouse_go() {
        let unit_mouse = settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse));
        let mut l = arranged(&unit_mouse);
        mv(&mut l, (600, 300));
        frame(&mut l, vec![]);
        click(&mut l, (600, 300));
        assert!(l.captured());
        // The world cursor into what the driver's view shows.
        let c = l.cursor;
        l.motion(((1400 - c.0).rem_euclid(8000) as f64, (800 - c.1) as f64));
        let f = frame(&mut l, vec![]);
        // A right click at the world cursor (B's middle) the driver's view shows.
        let at = l.cursor_on_window().unwrap();
        let out = l.route(Input::MouseButton { button: Button::Right, down: true, x: 600, y: 300 });
        let Input::MouseButton { x: qx, y: qy, .. } = out[0] else { panic!("{out:?} {:?}", f.cameras) };
        l.route(Input::MouseButton { button: Button::Right, down: false, x: 600, y: 300 });
        // The driver opens its menu there.
        let mut s = with_parts(None);
        let part = |id, origin, viewport| ViewState { id, view: View { interface_only: true, origin, viewport, ..Default::default() }, overlay: vec![] };
        s.views.push(part(257, (qx, qy), (150, 200)));
        s.interface.popup = Some(257);
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let c = f.cameras[f.view_cameras[3].unwrap() as usize];
        assert_eq!(c.origin, (at.0.min(800 - 150), at.1.min(600 - 200)));
        // The mouse is let go, the pointer at the menu; its rows take clicks.
        assert!(!l.captured());
        assert_eq!(l.take_grab(), Some(false));
        let row = (c.origin.0 + 20, c.origin.1 + 30);
        assert_eq!(click(&mut l, row)[0], Input::MouseButton { button: Button::Left, down: true, x: qx + 20, y: qy + 30 });
        // Closed: captured again, at the world cursor it had.
        let cursor = l.cursor;
        l.frame(&with_parts(None), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(l.captured());
        assert_eq!((l.take_grab(), l.cursor), (Some(true), cursor));
    }

    #[test]
    fn a_screen_opening_lets_the_mouse_go_with_the_pointer_on_its_main_control() {
        let unit_mouse = settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse));
        let mut l = arranged(&unit_mouse);
        mv(&mut l, (600, 300));
        frame(&mut l, vec![]);
        click(&mut l, (600, 300));
        assert!(l.captured());
        let cursor = l.cursor;
        let _ = l.take_grab();
        // The city screen over the window, its production box at 500, 400.
        let mut s = with_screen("City");
        s.interface.focus_control = Some([480, 380, 40, 40]);
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(!l.captured());
        assert_eq!((l.take_grab(), l.take_warp(), l.pointer), (Some(false), Some((500, 400)), (500, 400)));
        // Fitted into A (the settings place it there): where it shows.
        let mut l2 = arranged(&unit_mouse);
        l2.screen_places.insert("City".into(), ScreenPlace { vertical: Place::LeftOrBottom, horizontal: Place::Floating });
        mv(&mut l2, (600, 300));
        frame(&mut l2, vec![]);
        click(&mut l2, (600, 300));
        l2.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        // (A is 400 wide: the 800x600 screen at 1/2.)
        assert_eq!(l2.take_warp(), Some((250, 200)));
        // Closed: captured again at the world cursor it had.
        l.frame(&scene(), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(l.captured());
        assert_eq!((l.take_grab(), l.cursor), (Some(true), BOUNDS.unwrap().clamp_point(cursor)));
        // A screen opening with the mouse free leaves the pointer alone.
        let mut l = arranged(&unit_mouse);
        mv(&mut l, (100, 100));
        frame(&mut l, vec![]);
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!((l.take_warp(), l.pointer), (None, (100, 100)));
    }

    #[test]
    fn a_click_capturing_the_mouse_is_only_that() {
        let unit_mouse = settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse));
        let mut l = arranged(&unit_mouse);
        // The pointer over A first: B stays where it is.
        mv(&mut l, (100, 100));
        let before = frame(&mut l, vec![]);
        let b = camera_at(&before, (404, 0));
        mv(&mut l, (700, 500));
        // The capturing click: nothing for the driver, B does not move, the
        // world cursor at its middle.
        assert!(click(&mut l, (700, 500)).is_empty());
        assert!(l.captured());
        let after = frame(&mut l, vec![]);
        assert_eq!(camera_at(&after, (404, 0)), b);
        let (w, h) = b.source_size();
        assert_eq!(l.cursor, (b.x + w as i32 / 2, b.y + h as i32 / 2));
    }

    #[test]
    fn the_divider_closes_and_opens_with_the_capture_only() {
        let unit_mouse = settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse));
        let captured = || {
            let mut l = arranged(&unit_mouse);
            click(&mut l, (600, 300));
            frame(&mut l, vec![]);
            assert_eq!(l.open_spans(), []);
            l
        };
        let middle = [(200, 400)];
        // Ctrl down: still captured, the divider solid; up, alone: let go,
        // the middle third open.
        let mut l = captured();
        key(&mut l, VK_CONTROL, true);
        frame(&mut l, vec![]);
        assert!(l.captured() && l.open_spans().is_empty());
        key(&mut l, VK_CONTROL, false);
        frame(&mut l, vec![]);
        assert!(!l.captured());
        assert_eq!(l.open_spans(), middle);
        // Ctrl with another key: no change.
        let mut l = captured();
        key(&mut l, VK_CONTROL, true);
        key(&mut l, 0x43, true);
        key(&mut l, 0x43, false);
        key(&mut l, VK_CONTROL, false);
        frame(&mut l, vec![]);
        assert!(l.captured() && l.open_spans().is_empty());
        // Ctrl held while the mouse moves: no change.
        let mut l = captured();
        key(&mut l, VK_CONTROL, true);
        l.motion((5.0, 0.0));
        frame(&mut l, vec![]);
        key(&mut l, VK_CONTROL, false);
        frame(&mut l, vec![]);
        assert!(l.captured() && l.open_spans().is_empty());
    }

    // ---- Mouse capture ----------------------------------------------------

    #[test]
    fn ctrl_on_its_own_releases_the_mouse_and_a_click_captures_it_again() {
        let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
        // Over B (it follows the world cursor) the pointer shows a click
        // captures; over A not.
        mv(&mut l, (100, 300));
        assert_eq!(l.take_pointer_icon(), None);
        mv(&mut l, (600, 300));
        assert_eq!(l.take_pointer_icon(), Some(PointerIcon::Capture));
        assert!(click(&mut l, (600, 300)).is_empty());
        assert_eq!(l.take_grab(), Some(true));
        assert_eq!(l.take_pointer_icon(), Some(PointerIcon::Default));
        frame(&mut l, vec![]);
        // Captured, the divider is closed all along.
        assert_eq!(l.open_spans(), []);
        l.motion((25.0, 10.0));
        frame(&mut l, vec![]);
        let cursor = l.cursor;
        // Ctrl with another key, a button or the wheel: no.
        key(&mut l, VK_CONTROL, true);
        key(&mut l, 0x43, true);
        key(&mut l, 0x43, false);
        key(&mut l, VK_CONTROL, false);
        key(&mut l, VK_CONTROL, true);
        l.route(Input::Wheel { delta: 120, x: 600, y: 300 });
        key(&mut l, VK_CONTROL, false);
        // Alt held (button snapping): no.
        key(&mut l, 0x12, true);
        key(&mut l, VK_CONTROL, true);
        key(&mut l, VK_CONTROL, false);
        key(&mut l, 0x12, false);
        assert!(l.locked);
        assert_eq!(l.take_grab(), None);
        // Ctrl on its own (held a while: repeated presses): released, the
        // system pointer where the world cursor is (B's middle).
        assert_eq!(key(&mut l, VK_CONTROL, true), [Input::Key { vk: VK_CONTROL, down: true, text: None }]);
        key(&mut l, VK_CONTROL, true);
        key(&mut l, VK_CONTROL, false);
        assert!(!l.locked);
        assert_eq!(l.take_grab(), Some(false));
        let at = l.cursor_on_window().unwrap();
        assert_eq!(l.take_warp(), Some(at));
        assert_eq!(l.cursor, cursor);
        assert_eq!(l.take_pointer_icon(), Some(PointerIcon::Capture));
        // Free, the pointer goes anywhere (through the open divider too).
        assert_eq!(l.open_spans(), [(200, 400)]);
        mv(&mut l, (600, 300));
        mv(&mut l, (100, 300));
        assert_eq!(l.pointer, (100, 300));
        assert_eq!(l.take_pointer_icon(), Some(PointerIcon::Default));
        // A click in B captures again; Ctrl alone while free does nothing.
        key(&mut l, VK_CONTROL, true);
        key(&mut l, VK_CONTROL, false);
        assert_eq!(l.take_grab(), None);
        click(&mut l, (650, 200));
        assert!(l.locked);
        assert_eq!(l.take_grab(), Some(true));
    }

    #[test]
    fn the_world_cursor_is_drawn_in_both_views_and_stays_in_bs_middle() {
        let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
        click(&mut l, (600, 300));
        // Move the cursor next to the unit A follows: A shows it too.
        l.cursor = (3000, 2000);
        let f = frame(&mut l, vec![]);
        assert!(f.markers.contains(&Marker::Cursor((3000, 2000))));
        let b = camera_at(&f, (404, 0));
        let (sw, sh) = b.source_size();
        assert_eq!((b.x + sw as i32 / 2, b.y + sh as i32 / 2), (3000, 2000));
        assert!(inside([f.cameras[0].x, f.cameras[0].y, 400, 600], (3000, 2000)));
        // At the edge of the world that does not wrap it moves off the
        // middle.
        l.motion((0.0, 5000.0));
        let f = frame(&mut l, vec![]);
        assert_eq!(l.cursor.1, 3999);
        assert_eq!(camera_at(&f, (404, 0)).y, 4000 - 600);
        assert_eq!(l.cursor_on_window().unwrap().1, 599);
    }

    #[test]
    fn a_zoom_box_over_the_capturing_view_captures_too() {
        let mut l = arranged(&settings(|s| (s.split, s.roles, s.zoom) = (S::Vertical1To1, Roles::UnitMouse, Zoom::Mouse111)));
        mv(&mut l, (600, 300));
        frame(&mut l, vec![]);
        assert_eq!(l.hit((600, 300)), Hit::Magnifier);
        assert_eq!(l.take_pointer_icon(), Some(PointerIcon::Capture));
        click(&mut l, (600, 300));
        assert!(l.locked);
        // Captured: the box hovers about the world cursor.
        let at = l.cursor_on_window().unwrap();
        let r = l.magnifier_geometry(&l.magnifier.unwrap()).0;
        assert_eq!((r[0] + r[2] / 2, r[1] + r[3] / 2), (at.0.clamp(404 + 66, 800 - 66), at.1));
    }

    // ---- Smooth world movement --------------------------------------------

    #[test]
    fn a_glide_eases_in_without_overshooting_and_settles_exactly() {
        let mut g = Glide::default();
        assert_eq!(g.step((100, 50), 0.016, (0, 0)), (100, 50));
        assert!(g.settled((100, 50)));
        let mut last = 100;
        let mut speeds = Vec::new();
        for frame in 0..120 {
            let (x, y) = g.step((1100, 50), 1.0 / 60.0, (0, 0));
            assert!(x >= last && x <= 1100, "frame {frame}: {x}");
            assert_eq!(y, 50);
            speeds.push(x - last);
            last = x;
        }
        assert!(g.settled((1100, 50)), "{g:?}");
        // It speeds up, then slows down (no jump to the target).
        let top = speeds.iter().copied().max().unwrap();
        let peak = speeds.iter().position(|&s| s == top).unwrap();
        assert!(speeds[0] < top && peak > 0 && peak < 40 && top < 200, "{speeds:?}");
        // Mostly there in half a second.
        let mut h = Glide::default();
        h.snap((0, 0));
        for _ in 0..30 {
            h.step((1000, 0), 1.0 / 60.0, (0, 0));
        }
        assert!(h.pos.0 > 900.0, "{h:?}");
    }

    #[test]
    fn retargeting_a_glide_keeps_its_velocity() {
        let mut g = Glide::default();
        g.snap((0, 0));
        for _ in 0..10 {
            g.step((1000, 0), 1.0 / 60.0, (0, 0));
        }
        let (v, x) = (g.velocity().0, g.pos.0);
        assert!(v > 0.0);
        // A new target behind it: it goes on the same way a moment, slowing.
        g.step((-1000, 0), 0.001, (0, 0));
        assert!((g.velocity().0 - v).abs() < v * 0.2, "{} vs {v}", g.velocity().0);
        assert!(g.pos.0 > x);
        for _ in 0..240 {
            g.step((-1000, 0), 1.0 / 60.0, (0, 0));
        }
        assert!(g.settled((-1000, 0)));
        // A stalled frame does not make it jump past the target.
        let mut s = Glide::default();
        s.snap((0, 0));
        let (x, _) = s.step((1000, 0), 5.0, (0, 0));
        assert!(x < 1000 && x > 0, "{x}");
    }

    #[test]
    fn a_glide_goes_the_short_way_round_a_wrapping_world() {
        let mut g = Glide::default();
        g.snap((7900, 0));
        let (x, _) = g.step((100, 0), 1.0 / 60.0, (8000, 0));
        // Rightwards over the seam, not 7800 pixels back.
        assert!(g.velocity().0 > 0.0 && (x > 7900 || x < 100), "{x}");
        for _ in 0..240 {
            g.step((100, 0), 1.0 / 60.0, (8000, 0));
        }
        assert!(g.settled((100, 0)), "{g:?}");
    }

    #[test]
    fn with_smooth_movement_a_view_glides_to_a_new_focus() {
        let mut l = arranged(&settings(|s| (s.split, s.roles, s.smooth_world_movement) = (S::Vertical1To1, Roles::UnitMouse, true)));
        assert_eq!(l.main_view.center, (3000, 2000));
        let mut s = scene();
        s.focus = Some((3600, 2000));
        l.advance(std::time::Duration::from_millis(16));
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let first = l.main_view.center.0;
        assert!(first > 3000 && first < 3100, "{first}");
        for _ in 0..120 {
            l.advance(std::time::Duration::from_millis(16));
            l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        }
        assert_eq!(l.main_view.center, (3600, 2000));
        // Without: at once.
        let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
        l.advance(std::time::Duration::from_millis(16));
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.main_view.center, (3600, 2000));
    }

    #[test]
    fn the_drivers_view_glides_by_pixels_and_lands_unmoved() {
        let mut l = arranged(&settings(|s| (s.split, s.smooth_world_movement) = (S::Vertical1To1, true)));
        let at = |x: i32| {
            let mut s = scene();
            s.views[0].view.camera_x = x;
            s
        };
        // The game scrolls a tile (64 pixels) at once: the pane follows by
        // pixels, the driver's labels with it; the edge still scrolls it.
        l.advance(std::time::Duration::from_millis(16));
        let f = l.frame(&at(1064), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(f.cameras[0].x > 1200 && f.cameras[0].x < 1264, "{}", f.cameras[0].x);
        assert_eq!(f.cameras[0].screen_shift.0, 1064 - f.cameras[0].x);
        assert!(l.main_unmoved());
        assert_eq!(mv(&mut l, (2, 300)).last(), Some(&Input::Edge { x: -3, y: 0 }));
        for _ in 0..120 {
            l.advance(std::time::Duration::from_millis(16));
            l.frame(&at(1064), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        }
        let f = l.frame(&at(1064), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        // (A shows the driver's middle in its own: 200 pixels further on.)
        assert_eq!((f.cameras[0].x, f.cameras[0].screen_shift, f.cameras[0].cull), (1264, (-200, 0), None));
    }
}
