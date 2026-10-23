//! How the viewer arranges the world in its window beyond what the driver
//! asks for: the driver's main view zoomed or moved, a second pane beside
//! it, floating insets and a magnifier beside the pointer. The driver's
//! interface (interface-only views) stays on top where the driver put it
//! (unless the viewer shows it elsewhere, [`Layout::interface_shown`]).
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
//! a view (sliding on when let go), Ctrl+Shift+left drag moves an inset
//! (resizes it at its bottom-right corner), a left drag on the divider moves
//! it.
//!
//! # Moving the map
//!
//! While the mouse is free, every view scrolls with the pointer at the
//! window's edges and corners it reaches. Every view uses the same speed
//! in window pixels, faster nearer the edge. With
//! `viewer_settings::MapMovement::Drag`, a left drag
//! past a few pixels grabs the map instead (a shorter press is a click), it
//! following the pointer 1:1 and sliding on with momentum. See `pan`.
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
//! - **Zoom box**: a world magnifier over the window (or, in a vertical
//!   split, over A or B, `zoom_place`; it stays inside that region): a
//!   floating box at the top of the region, centred across, 1:3:1 or 1:7:1
//!   of its width (margin : box : margin), its top `zoom_box_top` (2.5%)
//!   of the region's height below the region's top; a box at the pointer,
//!   1:1:1 or 1:3:1 across, above the pointer (below where there is no
//!   room); or a strip across the top of that region that the views give
//!   room to. All of them are `zoom_strip` of the region's height (one
//!   slider, default 20%). The pointer stops below a strip and scrolls
//!   upwards there. A floating box the pointer comes to drops, its top
//!   `zoom_box_drop` (30%) of the region's height below the pointer, or
//!   holds the pointer out with `zoom_box_block_pointer`. Boxes do not
//!   dodge the views' own interface (corner buttons, unit orders): they
//!   lie under it, and it keeps its clicks. Every zoom magnifies
//!   `zoom_magnification` (1.5×, 2×, 3×, 4×; default 2×); Ctrl+wheel over
//!   it changes that within those steps until the setting changes.
//! - **Screens**: a screen the driver names (`Scene::interface`) goes over
//!   the window, or its panel is fitted over A or B. Both world panes keep
//!   their own cameras. Generic menus instead keep their requested size
//!   in the opening mouse view by default, with clipping and middle-drag
//!   panning. Explicit `ui=pane` fits the driver's whole window.
//! - **Beside**: the driver may ask for its screen and its world side by side
//!   (`scene::Beside`, e.g. a menu previewing the world it picks): its
//!   screen goes to an edge of the window and the whole world, shown as
//!   small as it takes, into the rest; the viewer's own views wait meanwhile.
//! - **Roles**: `Free` (A shows the driver's view, B is moved by hand), or
//!   `UnitMouse` (A follows the focus of interest, B follows the world
//!   cursor and captures the mouse; popups follow the opening view).
//! - **Smooth world movement**: a view following the driver's view or the
//!   focus of interest glides to where it goes (a critically damped spring,
//!   [`Glide`]; retargeting keeps its velocity) instead of jumping.
//!
//! # The divider
//!
//! The divider is a band of frosted glass (`glass`). While the mouse is not
//! captured it is a wall except in its middle third (no glass there) and
//! along the rows of the driver's always reachable controls
//! (`Interface::reachable`, e.g. the unit's orders, reaching over the gap
//! or cut there); with Drag map movement nowhere (`capture`). The pointer
//! pushed against it stays in its view (the
//! window is asked to move the system pointer back, [`Layout::take_warp`],
//! or the system holds it out of [`Layout::pointer_walls`], [`Walls`])
//! and scrolls that view instead through the viewer's pixel-based camera
//! movement (with edge scrolling on). Through the gaps it moves
//! freely. The
//! driver's roaming controls (`Interface::roaming`, a game's corner menu
//! buttons) are drawn in the view the pointer is in. While the mouse is
//! captured the whole divider is a wall. Ctrl pressed and released on its
//! own releases a captured mouse; over a view following the world cursor
//! the pointer then shows that a click captures it again
//! ([`Layout::take_pointer_icon`]).


use scene::{Button, Input, Scene};

#[path = "pan.rs"]
mod pan;
#[path = "glass.rs"]
mod glass;
#[path = "capture.rs"]
mod capture;
#[path = "world_rescale.rs"]
mod world_rescale;
#[path = "unit_orders.rs"]
mod unit_orders;
#[path = "world_pointer.rs"]
mod world_pointer;
#[path = "detached_popups.rs"]
mod detached_popups;
#[cfg(test)]
#[path = "city_screen_tests.rs"]
mod city_screen_tests;
#[cfg(test)]
#[path = "cursor_screen_tests.rs"]
mod cursor_screen_tests;
pub use pan::{DRAG_THRESHOLD, Momentum, Tracker};
pub use capture::{Attractor, RESUME_AFTER};
pub use glass::Shade;
use tile_render::{Camera, ScreenCopy};
use viewer_settings::{Place, Roles, ScreenPlace, UnitButtons};

/// Zoom steps, screen pixels per world pixel.
mod arrival;

pub const ZOOMS: [(u32, u32); 11] = [(1, 4), (1, 3), (1, 2), (2, 3), (1, 1), (3, 2), (2, 1), (3, 1), (4, 1), (6, 1), (8, 1)];
const ONE: usize = 4;
/// Most insets at a time.
pub const MAX_INSETS: usize = 3;
/// Gap between the panes, and the frame around insets and the magnifier.
const GAP: i32 = 4;
/// How far below the pointer a zoom box at the mouse goes (where there is
/// no room above it): clear of the pointer's arrow.
const POINTER_CLEARANCE: i32 = 24;
/// A view's centre before it was placed (it starts where the driver looks).
const UNPLACED: (i32, i32) = (i32::MIN, i32::MIN);
/// For how many frames after a world first shows a free view not yet
/// moved by hand follows the driver's view (a game loading a save moves
/// its view about for a moment, about 3 s at 60 frames a second).
pub const SETTLE_FRAMES: u32 = 180;
/// How fast the viewer scrolls a view the pointer pushes against the
/// divider, or rests at the window's outermost pixel by, in screen pixels
/// per second (slower further in, `pan`).
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
    /// Fractional world pixels retained across changes of game art scale.
    scale_rest: (f64, f64),
}

impl FreeView {
    pub fn new(center: (i32, i32), zoom: usize, follow: Follow) -> Self {
        Self { center, zoom, follow, settling: false, glide: Glide::default(), hand_focus: None, scale_rest: (0.0, 0.0) }
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
        self.scale_rest = (0.0, 0.0);
        self.glide.snap(center);
    }

    /// Its zoom with the world shown `scale` times larger besides
    /// ([`Layout::world_scale`]).
    fn zoom_by(&self, scale: (u32, u32)) -> (u32, u32) {
        times(ZOOMS[self.zoom], scale)
    }

    /// World pixels it shows in a rectangle `rect` wide and high (at zoom
    /// [`FreeView::zoom_by`] `scale`).
    fn source(&self, rect: [i32; 4], scale: (u32, u32)) -> (i32, i32) {
        let zoom = self.zoom_by(scale);
        ((rect[2].max(1) as u32 * zoom.1).div_ceil(zoom.0) as i32, (rect[3].max(1) as u32 * zoom.1).div_ceil(zoom.0) as i32)
    }

    /// Smallest pan that exposes the complete tile, including its diamond
    /// extrema. Use the camera's raster span, rather than its rounded-up
    /// source size, so a fractional last source pixel cannot hide an edge.
    fn expose_tile(&self, tile: (i32, i32), half: (i32, i32), rect: [i32; 4], bounds: Option<WorldBounds>, scale: (u32, u32)) -> (i32, i32) {
        if self.center == UNPLACED { return tile; }
        let camera = self.camera(rect, (0, 0, 0, 0), bounds, scale);
        let source = self.source(rect, scale);
        let axis = |center: i32, start: i32, source: i32, tile: i32, half: i32, pixels: u32, axis: usize| {
            let span = (pixels as i64 * camera.zoom.1 as i64 / camera.zoom.0 as i64) as i32;
            if span < 2 * half { return tile; }
            let tile = match bounds.map(|b| b.axis(axis)) {
                Some((period, true)) if period > 0 => center + (tile - center + period / 2).rem_euclid(period) - period / 2,
                _ => tile,
            };
            let shift = if tile - half < start { tile - half - start }
                else if tile + half > start + span { tile + half - start - span }
                else { 0 };
            if shift == 0 { center } else { start + source / 2 + shift }
        };
        (axis(self.center.0, camera.x, source.0, tile.0, half.0, camera.viewport.0, 0),
         axis(self.center.1, camera.y, source.1, tile.1, half.1, camera.viewport.1, 1))
    }

    /// Its camera in `rect`: centred, but not past an edge of the world
    /// that does not wrap; the world `scale` times larger besides.
    fn camera(&self, rect: [i32; 4], overhang: (u32, u32, u32, u32), bounds: Option<WorldBounds>, scale: (u32, u32)) -> Camera {
        let (sw, sh) = self.source(rect, scale);
        // (Not placed yet: where the world starts.)
        let center = if self.center == UNPLACED { (0, 0) } else { self.center };
        let (cx, cy) = match bounds {
            Some(b) => (b.clamp_center(0, center.0, sw), b.clamp_center(1, center.1, sh)),
            None => center,
        };
        Camera {
            x: cx - sw / 2,
            y: cy - sh / 2,
            zoom: self.zoom_by(scale),
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
    /// A box at the top of its region, centred across, `1 : n : 1` of its
    /// width (margin : box : margin), [`ZoomBox::height`] of its height,
    /// [`ZoomBox::top`] below its top; it drops [`ZoomBox::drop`] below
    /// the pointer when the pointer comes to it.
    Box(u32, ZoomBox),
    /// A box at the pointer (the world cursor while the mouse is captured):
    /// above it, else below it, centred on it across, kept inside its
    /// region; `1 : n : 1` of the region's width, and this many
    /// thousandths of its height (`zoom_strip`).
    AboutPointer(u32, u32),
    /// A strip across the top of its region, this many thousandths of its
    /// height; the views below give it room.
    Strip(u32),
}

/// A floating zoom box's sizes, in thousandths of its region's height.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ZoomBox {
    /// Its height (`zoom_strip`).
    pub height: u32,
    /// The space between its region's top and its top (`zoom_box_top`).
    pub top: u32,
    /// How far below the pointer its top goes when the pointer comes to
    /// it (`zoom_box_drop`).
    pub drop: u32,
}

/// A sprite the viewer adds to the world.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Marker {
    /// The world cursor at this world pixel.
    Cursor((i32, i32)),
    /// The arrow at the point of interest.
    Focus((i32, i32)),
}

/// A driver's menu placed beside its optional world preview. The menu's
/// transform is final; interface scaling must not centre it a second time.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BesideScreen {
    pub source: [i32; 4],
    pub mapping: crate::scaling::Affine,
    pub region: [i32; 4],
}

/// A press on a world pixel the driver's window does not take (it gets the
/// tile, not the pixel): where in the window, the world pixel, the view.
#[derive(Copy, Clone, Debug, PartialEq)]
struct WorldPress {
    at: (i32, i32),
    world: (i32, i32),
    hit: Hit,
    clock: f64,
}

/// What one frame draws: cameras in drawing order, for each scene view the
/// camera that shows its overlay, screen copies and markers on top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub cameras: Vec<Camera>,
    pub beside_screen: Option<BesideScreen>,
    /// Named-screen cameras already placed in canvas pixels. Interface
    /// scaling leaves these cameras and their stacked group unchanged.
    pub placed: Vec<u32>,
    pub view_cameras: Vec<Option<u32>>,
    /// Canvas region belonging to the view that opened a generic popup.
    pub popup_region: Option<[i32; 4]>,
    /// A click-anchored popup's top-left corner on the canvas and its zoom
    /// (canvas pixels per driver pixel): placed there exactly, unclipped.
    pub popup_at: Option<((i32, i32), (u32, u32))>,
    /// World-anchored lower modal views retain their own place under a child.
    pub popup_places: Vec<(u32, ((i32, i32), (u32, u32)))>,
    /// Generic menus retain their requested scale in their opening view.
    pub menu_region: Option<[i32; 4]>,
    /// The area of the view the per-view interface (the corner's buttons,
    /// the hint in their place) shows in ([`Layout::view_area`], canvas
    /// pixels): scaled no larger than fits it and kept inside it.
    pub view_ui_region: Option<[i32; 4]>,
    /// Where the unit's order buttons (`scene::Interface::reachable`) go
    /// with two views (`scaling::arrange` moves their sprites).
    pub unit_orders: Option<UnitOrdersPlace>,
    pub copies: Vec<ScreenCopy>,
    pub markers: Vec<Marker>,
    /// The divider's glass (`glass`): per strip one pixel across the gap,
    /// the camera of the half of the gap it lies in (the pane's picture
    /// going on there) to draw it in, its rectangle (x, y, w, h), whether
    /// the split is side by side, and its shade (`crate::glass_sprite`).
    pub divider: Vec<(u32, [i32; 4], bool, Shade)>,
    /// Where the cursor drawn in a view the pointer is not in (the world
    /// cursor mirrored there) goes in the drawing order: before this camera,
    /// the first of the driver's interface over the world views, so that
    /// menus, screens, popups and the frame cover it (`crate::cursor`).
    /// `None`: no such place (no world arranged by the layout).
    pub cursor_layer: Option<u32>,
    /// Empty cameras at `cursor_layer` that `scaling::arrange` reserves for
    /// those cursors (one a world view drawn below it), for the window to
    /// fill (`crate::cursor::compose`).
    pub cursor_slots: Vec<u32>,
}

/// Where the unit's order buttons go while there are two views
/// ([`UnitButtons`]), in canvas pixels.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UnitOrdersPlace {
    /// Where the driver put them, those left of the middle of the side by
    /// side split's gap (from, to: canvas x) kept left of it and the others
    /// right of it, so the gap stays clear for the pointer.
    Gap((i32, i32)),
    /// Inside this view area (the active view's), at the same height where
    /// that is inside it, centred between the interface pieces on its row.
    In([i32; 4]),
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
    /// The active named screen's panel placed into a pane.
    Screen,
    /// A part of the driver's interface the viewer drew elsewhere (index
    /// into `Layout::moved`).
    Moved(usize),
    /// The world shown beside the driver's screen (`scene::Beside`).
    Beside,
}

/// One of the two views of a split.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Side {
    /// The main pane (left, or top).
    A,
    /// The second pane (right, or bottom).
    B,
}

/// How divider and zoom-area walls hold the system pointer back.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Walls {
    /// The window moves the system pointer back ([`Layout::take_warp`]).
    Warp,
    /// The system holds the pointer out of the walls
    /// ([`Layout::pointer_walls`], e.g. a Wayland pointer confinement): it
    /// is not moved back.
    Confined,
    /// Nothing holds it: the pointer passes (where the system pointer can
    /// neither be moved back nor confined).
    Open,
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

/// The driver's screen fitted into a pane: its active content for a named
/// screen, or its whole window with `ui=pane`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Fit {
    hit: Hit,
    /// Content rectangle in the driver's window pixels.
    source: [i32; 4],
    /// Where the content's top-left is shown in the canvas.
    origin: (i32, i32),
    zoom: (u32, u32),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Drag {
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
    /// Floating zoom boxes hold the pointer out instead of moving aside.
    pub zoom_box_block_pointer: bool,
    /// The zoom box's or strip's zoom the settings last asked for: it stays
    /// as the wheel changed it until they ask for another.
    zoom_box_zoom: Option<usize>,
    /// Map detail independently for A, B and the world magnifier.
    pub clean_map: viewer_settings::clean_map::CleanMap,
    /// Whether to draw the arrow at the driver's point of interest.
    pub focus_arrow: bool,
    /// With a split: the driver's whole screen (its interface too) fitted
    /// into the main pane, rather than its interface over the window and
    /// only its world in the pane.
    pub ui_in_pane: bool,
    /// From the last frame: where the driver's screen sits and at which
    /// zoom, when it is fitted into a pane.
    fit: Option<Fit>,
    /// Only the named screen's own UI placed over an otherwise normal pane.
    placed_screen: Option<Fit>,
    /// The active menu's actual sprite bounds, supplied by the viewer.
    pub screen_content: Option<[i32; 4]>,
    /// Smooth world movement (see [`Glide`]).
    pub smooth: bool,
    /// What the views follow (from the settings).
    pub roles: Roles,
    /// Where the unit's order buttons go with two views
    /// ([`Frame::unit_orders`]).
    pub unit_buttons: UnitButtons,
    /// Where every screen and popup goes (from the settings; click-anchored
    /// popups excepted).
    pub screen_place: ScreenPlace,
    /// With a floating `screen_place`: the generic menu and popups open in
    /// the view that held the mouse.
    pub menu_follows_pointer: bool,
    menu_side: Option<Side>,
    popup_side: Option<Side>,
    /// The world's grid steps (`Compiled::step`; a world shown smaller
    /// divides them evenly, see `fit_world`).
    pub grid_step: (u32, u32),
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
    /// How the divider holds the pointer back (see [`Walls`]).
    walls: Walls,
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
    /// The current popup body in driver pixels.
    popup_body: Option<[i32; 4]>,
    popup_view: Option<u32>,
    anchored_popup_layers: Vec<(u32, scene::PopupAnchor, WorldPress)>,
    /// Seconds of frames so far (when presses were made).
    clock: f64,
    /// The last press that went to the driver: where it was made in the
    /// window, where the driver got it, when.
    last_press: Option<((i32, i32), (i32, i32), f64)>,
    /// How far the open popup moves, when it opened at a click.
    popup_anchor: Option<(i32, i32)>,
    /// The last press on a world pixel (`Input::WorldButton`).
    world_press: Option<WorldPress>,
    /// The tile boxes the driver left to the viewer (`detached_popups`).
    detached: detached_popups::Detached,
    /// The open popup the driver anchors at a world click
    /// (`scene::Interface::popup_anchor`), and that click.
    anchored: Option<(scene::PopupAnchor, WorldPress)>,
    /// The driver's anchor for the open popup as last taken in.
    popup_anchor_seen: Option<scene::PopupAnchor>,
    /// The world click the last anchored popup belonged to, and when that
    /// popup closed: a popup an entry of it opens (a city menu's "Terrain
    /// details") belongs to the same click, however long ago it was made.
    chain_press: Option<(WorldPress, f64)>,
    /// Buttons whose press only dismissed a popup anchored at a world click
    /// (their release goes nowhere either).
    dismissed: [bool; 3],
    /// How far (world pixels) the anchored popup's view was moved for it
    /// to fit the window, while that view follows the pointer or driver.
    popup_pan: Option<(Hit, (i32, i32))>,
    /// The world's period where it wraps (world pixels; 0: no wrap).
    world_period: (i32, i32),
    /// A capture a menu interrupted, taken up again once no menu is open
    /// (`capture`).
    resume: capture::Resume,
    /// Where the open menus lie over the window, as of the last frame
    /// (`capture`: a view they overlap does not capture the mouse).
    menu_cover: capture::Cover,
    /// The pointer goes to the screen just opened (this frame), or to the
    /// popup.
    pointer_to_screen: bool,
    pointer_to_popup: bool,
    /// Driver pixel to focus again after interface scaling has its final map.
    focus_after_scale: Option<(i32, i32)>,
    /// Whether button snapping is on (see [`Layout::set_snapping`]).
    snapping: bool,
    moved: Vec<([i32; 4], (i32, i32))>,
    /// The window rectangles of the per-view interface the last frame drew
    /// (the corner's buttons or the hint in their place, as large as the
    /// HUD's scale shows them), laid out in [`Layout::view_area`]: the
    /// floating zoom boxes keep clear of them.
    view_ui: Vec<[i32; 4]>,
    /// Whether the last frame showed the magnifier (it may be left out, or
    /// be under a screen fitted into a pane).
    magnifier_shown: bool,
    /// Whether the scene of the last frame had a world (the zoom box and
    /// strip show only then).
    world_shown: bool,
    /// Where a world magnifier looked last (it keeps looking there while
    /// nothing is under the pointer).
    magnifier_center: Option<(i32, i32)>,
    /// From the last frame: the driver's screen and its world side by side
    /// (`scene::Interface::beside`): the world's rectangle.
    beside: Option<[i32; 4]>,
    beside_mapping: Option<crate::scaling::Affine>,
    /// From the last frame: the focus of interest.
    focus: Option<(i32, i32)>,
    /// Last semantic camera request consumed from a driver that parks its
    /// physical camera. Equal positions can still be distinct requests.
    camera_focus_serial: Option<u64>,
    /// A semantic camera request wins over the active unit only until that
    /// unit focus changes. Driver-following A always uses the request.
    semantic_focus_active: bool,
    focus_at_camera_request: Option<(i32, i32)>,
    focus_at_camera_request_rest: (f64, f64),
    /// Last virtual pointer already reported through input or a drawn frame.
    last_world_pointer: Option<(i32, i32)>,
    /// Last normal-world art scale; menus and fitted scenario previews do
    /// not change the independent world's coordinate system.
    world_downscale: Option<u32>,
    /// The game's current native map-wheel distance, independent of the
    /// physical native camera that may be parked at the origin.
    map_wheel_step: (i32, i32),
    /// The last `Input::Scroll` panned the map: the `Wheel` notches it
    /// made are not the interface's (until another input).
    scroll_panned: bool,
    /// The driver's minimap, where the renderer may point at it.
    minimap: Option<scene::Minimap>,
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
    /// Additional relative-motion multiplier while the world cursor is captured.
    capture_sensitivity: f64,
    /// Whether the mouse is captured (moving the world cursor), and a
    /// change of that for the window to make.
    locked: bool,
    /// Logical capture currently uses the system pointer at a world edge.
    pointer_at_edge: bool,
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
    /// Input cameras in the driver's world units. A world magnifier's
    /// render camera uses full-size artwork, with the same map coverage.
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
    /// Every world view shows the world this many times larger besides its
    /// own zoom (the viewer's resolution settings); the main view following
    /// the driver at zoom 1 times this still has the driver's edges.
    pub world_scale: (u32, u32),
    /// Where the viewer shows the driver's interface and its other views,
    /// if elsewhere than the layout put them (e.g. scaled): these take the
    /// pointer instead of the layout's own rectangles, and map it back to
    /// the driver's pixels.
    pub interface_shown: Option<Vec<crate::scaling::ShownPiece>>,
    /// Middle-button panning of a menu larger than the render canvas.
    pub menu_pan: crate::scaling::MenuPan,
    /// The driver's screen, if other than the window (the viewer draws its
    /// interface on its own, `crate::scaling`): its views filling its
    /// screen are as large as that.
    pub driver_screen: Option<(u32, u32)>,
    /// Canvas pixels per menu pixel for a driver-arranged preview screen.
    pub menu_scale: (u32, u32),
    /// Canvas pixels per driver pixel of the game's frame (its HUD:
    /// `scaling::Plan::frame`).
    pub hud_scale: (u32, u32),
    /// Edge scrolling and dragging the map (`pan`).
    pan: pan::Pan,
    unit_orders: unit_orders::UnitOrders,
    /// Exploring the world beside the driver's screen ([`Explore`]); the
    /// world beside it in the last frame; the last left press on it (when,
    /// where: for double-clicks).
    explore: Option<Explore>,
    beside_of: Option<(scene::WorldShape, Option<[i32; 4]>)>,
    beside_press: Option<(f64, (i32, i32))>,
    /// How many times smaller the driver's view showed the world beside
    /// the screen in the last frame.
    beside_view_downscale: u32,
    /// Keys whose press exploring kept from the driver (their release too).
    kept_keys: Vec<u32>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            main_view: FreeView::new(UNPLACED, ONE, Follow::Driver),
            split: None,
            insets: Vec::new(),
            magnifier: None,
            zoom_box_block_pointer: false,
            zoom_box_zoom: None,
            clean_map: Default::default(),
            focus_arrow: false,
            window: (1, 1),
            pointer: (0, 0),
            cursor: (0, 0),
            cursor_placed: false,
            motion_rest: (0.0, 0.0),
            capture_sensitivity: 1.0,
            locked: false,
            pointer_at_edge: false,
            grab: None,
            reported_zooms: [(0, 0); 8],
            art_zooms: Vec::new(),
            reported_views: [(0, 0, 0, 0); 4],
            reported_main: (0, 0, 0, 0),
            sharp_text: true,
            ui_in_pane: false,
            fit: None,
            placed_screen: None,
            screen_content: None,
            smooth: false,
            roles: Roles::Free,
            unit_buttons: UnitButtons::Spanning,
            screen_place: ScreenPlace::default(),
            menu_follows_pointer: true,
            menu_side: None,
            popup_side: None,
            grid_step: (64, 32),
            step: 0.0,
            side: None,
            against: None,
            raw_pointer: (i32::MIN, i32::MIN),
            warp: None,
            walls: Walls::Warp,
            ctrl_alone: false,
            keys_down: Vec::new(),
            icon: PointerIcon::Default,
            reachable: None,
            screen: None,
            popup_shown: false,
            popup_body: None,
            popup_view: None,
            anchored_popup_layers: Vec::new(),
            clock: 0.0,
            last_press: None,
            popup_anchor: None,
            world_press: None,
            detached: Default::default(),
            anchored: None,
            popup_anchor_seen: None,
            chain_press: None,
            dismissed: [false; 3],
            popup_pan: None,
            world_period: (0, 0),
            resume: capture::Resume::default(),
            menu_cover: capture::Cover::default(),
            pointer_to_screen: false,
            pointer_to_popup: false,
            focus_after_scale: None,
            snapping: false,
            moved: Vec::new(),
            view_ui: Vec::new(),
            magnifier_shown: true,
            world_shown: true,
            magnifier_center: None,
            beside: None,
            beside_mapping: None,
            focus: None,
            camera_focus_serial: None,
            semantic_focus_active: false,
            focus_at_camera_request: None,
            focus_at_camera_request_rest: (0.0, 0.0),
            last_world_pointer: None,
            world_downscale: None,
            map_wheel_step: (0, 0),
            scroll_panned: false,
            minimap: None,
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
            world_scale: (1, 1),
            interface_shown: None,
            menu_pan: crate::scaling::MenuPan::default(),
            driver_screen: None,
            menu_scale: (1, 1),
            hud_scale: (1, 1),
            pan: pan::Pan::default(),
            unit_orders: unit_orders::UnitOrders::default(),
            explore: None,
            beside_of: None,
            beside_press: None,
            beside_view_downscale: 1,
            kept_keys: Vec::new(),
        }
    }
}

const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;
const VK_ESCAPE: u32 = 0x1B;

/// Exploring the world beside the driver's screen (`scene::Beside`) up
/// close: a double-click on it shows it as the game's zoomed-out view
/// does (the Z key: the world at half size, [`EXPLORE_DOWNSCALE`], at the
/// zoom of the game's own view, `Layout::world_scale`) around the point
/// clicked, the mouse captured: the view follows the world cursor,
/// and nothing reaches the driver. Ctrl on its own lets the mouse go (a
/// click on the world captures it again); a double-click or Esc goes back
/// to the world fitted beside the screen, as does another world beside it
/// (e.g. another scenario picked) or the screen closing.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Explore {
    /// The world explored: the scene's world and the part the driver
    /// asked to fit (`scene::Beside::world`).
    of: (scene::WorldShape, Option<[i32; 4]>),
    /// The world pixel in the middle (in the pixels of the half-size
    /// world, [`EXPLORE_DOWNSCALE`]).
    center: (i32, i32),
}

/// How many times smaller a world beside a screen is explored: the game's
/// zoomed-out view (Z), whose game view shows the world at downscale 2.
const EXPLORE_DOWNSCALE: u32 = 2;
/// Its zoom ([`ZOOMS`] index): 1:1, as the game's own view (before the
/// world scale).
const EXPLORE_ZOOM: usize = ONE;
/// Two presses at most this far apart in time (seconds) and space (window
/// pixels) are a double-click.
const DOUBLE_CLICK_SECONDS: f64 = 0.5;
const DOUBLE_CLICK_PIXELS: i32 = 6;

/// Whether rectangles (x, y, w, h) share a pixel.
fn overlap(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// Span `(a, b)` less the spans `cut`: what is left of it, in order.
fn subtract_spans((a, b): (i32, i32), cut: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut left = vec![(a, b)];
    for &(c0, c1) in cut.iter().filter(|(c0, c1)| c1 > c0) {
        left = left
            .into_iter()
            .flat_map(|(l0, l1)| [(l0, l1.min(c0)), (l0.max(c1), l1)])
            .filter(|(l0, l1)| l1 > l0)
            .collect();
    }
    left
}

/// A rectangle less an overlapping rectangle, in at most four pieces.
fn subtract_rect([x, y, w, h]: [i32; 4], cut: [i32; 4]) -> Vec<[i32; 4]> {
    let (x0, y0) = (x.max(cut[0]), y.max(cut[1]));
    let (x1, y1) = ((x + w).min(cut[0] + cut[2]), (y + h).min(cut[1] + cut[3]));
    if x0 >= x1 || y0 >= y1 { return vec![[x, y, w, h]]; }
    [[x, y, w, y0 - y], [x, y1, w, y + h - y1], [x, y0, x0 - x, y1 - y0], [x1, y0, x + w - x1, y1 - y0]]
        .into_iter().filter(|r| r[2] > 0 && r[3] > 0).collect()
}

/// Stops a segment at the first side of a forbidden rectangle. The free
/// coordinate follows the mouse, so it slides along the box's boundary.
fn hold_outside(r: [i32; 4], from: (i32, i32), to: (i32, i32), window: (u32, u32)) -> (i32, i32) {
    if inside(r, from) {
        if !inside(r, to) { return to; }
        return [(r[0] - 1, to.1), (r[0] + r[2], to.1), (to.0, r[1] - 1), (to.0, r[1] + r[3])]
            .into_iter().filter(|&p| inside([0, 0, window.0 as i32, window.1 as i32], p))
            .min_by_key(|p| (p.0 as i64 - to.0 as i64).pow(2) + (p.1 as i64 - to.1 as i64).pow(2)).unwrap_or(to);
    }
    let (mut enter, mut leave, mut side) = (f64::NEG_INFINITY, f64::INFINITY, None);
    for (axis, a, b, lo, hi) in [(0, from.0, to.0, r[0], r[0] + r[2]), (1, from.1, to.1, r[1], r[1] + r[3])] {
        if a == b {
            if a < lo || a >= hi { return to; }
            continue;
        }
        let delta = b as f64 - a as f64;
        let (near, far, held) = if delta > 0.0 { ((lo as f64 - a as f64) / delta, (hi as f64 - a as f64) / delta, lo - 1) }
            else { ((hi as f64 - a as f64) / delta, (lo as f64 - a as f64) / delta, hi) };
        if near > enter { enter = near; side = Some((axis, held)); }
        leave = leave.min(far);
    }
    if enter >= 0.0 && enter <= 1.0 && enter <= leave && leave >= 0.0 {
        match side { Some((0, held)) => (held, to.1), Some((_, held)) => (to.0, held), None => to }
    } else { to }
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

/// Where the driver's screen and its world go when it asks for them side
/// by side in a window of `window`: how far its screen moves, and the
/// world's rectangle (the rest of the window, a gap from the screen).
fn beside_geometry(beside: scene::Beside, window: (u32, u32)) -> ((i32, i32), [i32; 4]) {
    let [sx, sy, sw, sh] = beside.screen;
    let (w, h) = (window.0 as i32, window.1 as i32);
    // Along the edge it stays where it is, within the window.
    let along = |at: i32, size: i32, room: i32| at.clamp(0, (room - size).max(0));
    let (to, world) = match beside.edge {
        scene::Edge::Left => ((0, along(sy, sh, h)), [sw + GAP, 0, w - sw - GAP, h]),
        scene::Edge::Right => (((w - sw).max(0), along(sy, sh, h)), [0, 0, w - sw - GAP, h]),
        scene::Edge::Top => ((along(sx, sw, w), 0), [0, sh + GAP, w, h - sh - GAP]),
        scene::Edge::Bottom => ((along(sx, sw, w), (h - sh).max(0)), [0, 0, w, h - sh - GAP]),
    };
    ((to.0 - sx, to.1 - sy), [world[0], world[1], world[2].max(0), world[3].max(0)])
}

/// A camera showing the part `area` (x, y, w, h) of a world (the pixels of
/// `camera`'s world, at downscale 1; [`whole_world`]: all of it) in `rect`,
/// centred: the world shown `n` times smaller (`Camera::downscale`) and
/// then zoomed by the fraction that makes it fit. `n` divides the grid's
/// steps (`step`; the cells of a world shown smaller lie on whole pixels),
/// the one nearest to fitting either way (zoomed least). A world the driver
/// shows smaller itself (its view's downscale, e.g. with sprites laid out
/// for it) is only zoomed.
fn fit_world(camera: &Camera, area: [i32; 4], rect: [i32; 4], step: (u32, u32)) -> Camera {
    // (`area` is in the pixels of the world as the driver shows it.)
    let shown = camera.downscale.max(1);
    let step = (step.0 / shown, step.1 / shown);
    let (ww, wh) = ((area[2] as i64).max(1), (area[3] as i64).max(1));
    let (rw, rh) = (rect[2].max(1) as i64, rect[3].max(1) as i64);
    // The zoom showing the world n times smaller in the rectangle: the
    // smaller of the two ratios, as a fraction.
    let zoom = |n: i64| if rw * wh <= rh * ww { (rw * n, ww) } else { (rh * n, wh) };
    let common = if shown > 1 { 1 } else { gcd(step.0, step.1) as i64 };
    let divisors: Vec<i64> = (1..=common).filter(|n| common % n == 0).collect();
    let least = ((ww + rw - 1) / rw).max((wh + rh - 1) / rh).max(1);
    // The smallest dividing n at which the world fits unzoomed, and the
    // largest below it (zoomed out a little instead).
    let above = divisors.iter().copied().find(|&n| n >= least);
    let below = divisors.iter().copied().rev().find(|&n| n < least);
    // (How far from 1 a zoom is, either way.)
    let off = |(a, b): (i64, i64)| if a >= b { a as f64 / b as f64 } else { b as f64 / a as f64 };
    let n = [above, below].into_iter().flatten().min_by(|&a, &b| off(zoom(a)).total_cmp(&off(zoom(b)))).unwrap_or(1);
    let (a, b) = zoom(n);
    let g = gcd(a as u32, b as u32) as i64;
    let zoom = ((a / g) as u32, (b / g) as u32);
    // The rectangle in the view's own pixels, the world in the middle.
    let (source_w, source_h) = (rw * zoom.1 as i64 / zoom.0 as i64, rh * zoom.1 as i64 / zoom.0 as i64);
    let (x, y) = (area[0] as i64 / n + (ww / n - source_w) / 2, area[1] as i64 / n + (wh / n - source_h) / 2);
    Camera {
        x: x as i32,
        y: y as i32,
        zoom,
        viewport: (rect[2].max(1) as u32, rect[3].max(1) as u32),
        origin: (rect[0], rect[1]),
        cull: None,
        screen_shift: (0, 0),
        downscale: shown * n as u32,
        ..*camera
    }
}

/// `bounds` of a world shown `shown` times smaller as the half-size world
/// explored beside a screen ([`EXPLORE_DOWNSCALE`]) has them.
fn explore_bounds(bounds: WorldBounds, shown: u32) -> WorldBounds {
    let at_half = |v: i32| (v as i64 * shown.max(1) as i64 / EXPLORE_DOWNSCALE as i64) as i32;
    WorldBounds { size: (at_half(bounds.size.0), at_half(bounds.size.1)), ..bounds }
}

/// All of a world of `bounds` for [`fit_world`] (in the pixels of a view
/// showing it `shown` times smaller): staggered rows reach a step (`step`,
/// at full size) further right than the columns.
fn whole_world(bounds: WorldBounds, step: (u32, u32), shown: u32) -> [i32; 4] {
    [0, 0, bounds.size.0 + (step.0 / shown.max(1)) as i32, bounds.size.1]
}

/// `v` driver pixels at `zoom`, rounded up.
fn scaled_up(v: i32, zoom: (u32, u32)) -> i32 {
    (v.max(0) as u64 * zoom.0 as u64).div_ceil(zoom.1.max(1) as u64) as i32
}

/// How far something `len` long at `at` must move back (positive: towards
/// the start) to lie within `0..room`; its start at 0 if it is longer.
fn fit_pan(at: i32, len: i32, room: i32) -> i32 {
    if at < 0 || len > room { at } else { (at + len - room).max(0) }
}

/// Canvas pixels `v` as world pixels at camera `zoom` (canvas pixels per
/// world pixel), rounded away from zero (moving at least that far).
fn world_steps(v: i32, zoom: (u32, u32)) -> i32 {
    let (n, d) = (zoom.0.max(1) as i64, zoom.1.max(1) as i64);
    let w = v.unsigned_abs() as u64 * d as u64;
    (w.div_ceil(n as u64) as i32) * v.signum()
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

/// The product of two zooms, reduced.
fn times(a: (u32, u32), b: (u32, u32)) -> (u32, u32) {
    let (n, d) = (a.0.max(1) * b.0.max(1), a.1.max(1) * b.1.max(1));
    let g = gcd(n, d);
    (n / g, d / g)
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
    /// The zoom box's or strip's zooms ([`ZOOMS`] indices): 3/2, 2/1, 3/1,
    /// 4/1, as `viewer_settings::Layout::MAGNIFICATIONS` (by its step); the
    /// wheel over it stays within them.
    const ZOOM_BOX_ZOOMS: std::ops::RangeInclusive<usize> = ONE + 1..=ONE + 4;

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
        // The magnification asked for; kept as the wheel changed it while
        // the setting stays the same.
        let wanted = Self::ZOOM_BOX_ZOOMS.start() + s.zoom_magnification_step();
        let zoom = match self.magnifier.filter(|m| m.place != ZoomPlace::Beside) {
            Some(m) if self.zoom_box_zoom == Some(wanted) => m.zoom,
            _ => wanted,
        };
        self.zoom_box_zoom = Some(wanted);
        // (`zoom_strip`: the height of every zoom, strip or box.)
        let thousandths = |v: f32, max: f32| (v.clamp(0.0, max) * 1000.0).round() as u32;
        let strip = thousandths(s.zoom_strip, 0.5).max(50);
        let floating = ZoomBox { height: strip, top: thousandths(s.zoom_box_top, 0.2), drop: thousandths(s.zoom_box_drop, 0.5) };
        let place = match s.zoom {
            Zoom::Off => None,
            Zoom::Floating131 => Some(ZoomPlace::Box(3, floating)),
            Zoom::Floating171 => Some(ZoomPlace::Box(7, floating)),
            Zoom::Mouse111 => Some(ZoomPlace::AboutPointer(1, strip)),
            Zoom::Mouse131 => Some(ZoomPlace::AboutPointer(3, strip)),
            Zoom::TopStrip => Some(ZoomPlace::Strip(strip)),
        };
        self.magnifier = place.map(|place| Magnifier { mode: MagnifierMode::World, zoom, place, region: s.zoom_place, ..Self::MAGNIFIER });
        self.zoom_box_block_pointer = s.zoom_box_block_pointer;
        self.screen_place = s.screen_place;
        self.menu_follows_pointer = s.menu_follows_pointer;
        self.smooth = s.smooth_world_movement;
        self.roles = s.roles;
        self.unit_buttons = s.unit_buttons;
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
        if self.snapping {
            return None;
        }
        self.warp.take()
    }

    /// The window cannot move the system pointer (e.g. on Wayland): the
    /// divider lets the pointer through everywhere while the mouse is free,
    /// the pointer the user sees being where the layout has it.
    pub fn pointer_cannot_be_moved(&mut self) {
        if self.walls == Walls::Warp {
            self.walls = Walls::Open;
            // Where the system pointer really is.
            self.pointer = self.raw_pointer;
            self.side = self.side_of(self.pointer);
            self.against = None;
            log::info!("viewer: the system pointer cannot be moved back here; the divider lets it through while the mouse is free");
        }
    }

    /// How the divider holds the pointer back from now on (the window says
    /// whether the system confines the pointer to [`Layout::pointer_walls`]).
    pub fn set_walls(&mut self, walls: Walls) {
        if walls == self.walls {
            return;
        }
        log::debug!("viewer: the divider's walls: {:?} (were {:?})", walls, self.walls);
        if walls == Walls::Open {
            // Where the system pointer really is.
            self.pointer = self.raw_pointer;
            self.side = self.side_of(self.pointer);
            self.against = None;
        }
        self.walls = walls;
    }

    /// The divider's walls as of the last frame, for the system to keep the
    /// pointer out of ([`Walls::Confined`]): canvas rectangles; none where
    /// the divider holds nothing back (no split, relative mouse capture,
    /// the divider dragged, a screen beside its world, button snapping). They
    /// are the gap's closed stretches less where the driver's interface
    /// reaches across the gap (and the always reachable controls, a little
    /// around them), as [`Layout::constrain`] lets the pointer through.
    fn divider_pointer_walls(&self) -> Option<Vec<[i32; 4]>> {
        if (self.locked && !self.pointer_at_edge) || self.drag.is_some() || (self.beside.is_some() && !self.pointer_at_edge) || self.snapping || self.pointer_free_everywhere() {
            return None;
        }
        let (((g0, g1), _), split) = (self.gap()?, self.split?);
        let vertical = split.axis == Axis::Vertical;
        // (across from, across to, along from, along to)
        let spans = |r: [i32; 4]| if vertical { (r[0], r[0] + r[2], r[1], r[1] + r[3]) } else { (r[1], r[1] + r[3], r[0], r[0] + r[2]) };
        // Along the divider: where the interface reaches across the gap.
        let mut open: Vec<(i32, i32)> = Vec::new();
        if self.fit.is_none() {
            let moved = self.moved.iter().map(|(r, _)| r);
            for &r in self.interface.iter().chain(&self.driver_rects).chain(moved) {
                let (lo, hi, a0, a1) = spans(r);
                if lo < g0 && hi > g1 {
                    open.push((a0, a1));
                }
            }
        }
        if let Some(r) = self.orders_shown() {
            // (All along their rows, a little around them: `open_spans`.)
            let (_, _, a0, a1) = spans([r[0] - GAP, r[1] - GAP, r[2] + 2 * GAP, r[3] + 2 * GAP]);
            open.push((a0, a1));
        }
        let mut walls = Vec::new();
        for (r, _) in self.divider_pieces() {
            let (_, _, a0, a1) = spans(r);
            for (b0, b1) in subtract_spans((a0, a1), &open) {
                walls.push(if vertical { [g0, b0, g1 - g0, b1 - b0] } else { [b0, g0, b1 - b0, g1 - g0] });
            }
        }
        (!walls.is_empty()).then_some(walls)
    }

    /// Rectangles the system must keep the free or captured-edge pointer
    /// outside. Zoom
    /// strips include the gap below them, so the pointer stops on the map's
    /// top row. Interface controls drawn over a zoom remain reachable.
    pub fn pointer_walls(&self) -> Option<Vec<[i32; 4]>> {
        if (self.locked && !self.pointer_at_edge) || self.drag.is_some() || (self.beside.is_some() && !self.pointer_at_edge) || self.snapping || self.pointer_free_everywhere() {
            return None;
        }
        let mut walls = self.divider_pointer_walls().unwrap_or_default();
        let bounds = self.pointer_bounds();
        if let Some(bounds) = bounds {
            // Capture owns the pointer across the divider's usual openings
            // too. A single unlocked axis is a one-pixel line.
            walls.extend(subtract_rect([0, 0, self.window.0 as i32, self.window.1 as i32], bounds));
        }
        walls.extend(self.zoom_pointer_walls());
        (!walls.is_empty() || bounds.is_some()).then_some(walls)
    }

    /// The visible pointer may move only along axes whose camera centre
    /// is held at the world edge. The other axis stays on the cursor's
    /// screen coordinate, as a one-pixel confinement line.
    pub fn pointer_bounds(&self) -> Option<[i32; 4]> {
        if !self.locked || !self.pointer_at_edge || self.snapping {
            return None;
        }
        let (mut r, at, _) = self.world_cursor_view()?;
        let held = self.world_cursor_axes()?;
        if !held.0 { r[0] = at.0; r[2] = 1; }
        if !held.1 { r[1] = at.1; r[3] = 1; }
        let (x, y) = (r[0].max(0), r[1].max(0));
        let (right, bottom) = ((r[0] + r[2]).min(self.window.0 as i32), (r[1] + r[3]).min(self.window.1 as i32));
        (right > x && bottom > y).then_some([x, y, right - x, bottom - y])
    }

    /// Rendering switched between relative capture and the visible edge
    /// pointer. The first motion starts at the drawn cursor's position.
    pub fn set_pointer_at_edge(&mut self, on: bool) {
        let on = on && self.locked && !self.snapping;
        if on && !self.pointer_at_edge {
            self.pointer = self.cursor_on_window().unwrap_or(self.pointer);
            self.raw_pointer = self.pointer;
        }
        self.pointer_at_edge = on;
    }

    fn zoom_pointer_walls(&self) -> Vec<[i32; 4]> {
        if (self.locked && !self.pointer_at_edge) || self.drag.is_some() || (self.beside.is_some() && !self.pointer_at_edge) || self.snapping {
            return vec![];
        }
        let Some(m) = self.magnifier.filter(|_| self.magnifier_shown && self.world_shown) else { return vec![] };
        let r = match m.place {
            ZoomPlace::Strip(_) => {
                let Some((mut r, _)) = self.strip() else { return vec![] };
                r[3] += GAP;
                r
            }
            ZoomPlace::Box(..) if self.zoom_box_block_pointer => {
                let [x, y, w, h] = self.magnifier_geometry(&m).0;
                [x - GAP / 2, y - GAP / 2, w + GAP, h + GAP]
            }
            _ => return vec![],
        };
        let mut walls = vec![r];
        let holes: Vec<[i32; 4]> = if let Some(shown) = &self.interface_shown {
            shown.iter().map(|s| s.rect).collect()
        } else if self.fit.is_none() {
            self.interface.iter().chain(&self.driver_rects).copied().chain(self.moved.iter().map(|(r, _)| *r)).collect()
        } else { vec![] };
        for mut cut in holes.into_iter().filter(|cut| overlap(r, *cut)) {
            // Keep each control connected to the map outside the wall.
            // An isolated hole would be unreachable with OS confinement.
            if matches!(m.place, ZoomPlace::Strip(_)) {
                cut[3] = cut[3].max(r[1] + r[3] - cut[1]);
            } else {
                let distances = [cut[0] - r[0], r[0] + r[2] - cut[0] - cut[2], cut[1] - r[1], r[1] + r[3] - cut[1] - cut[3]];
                match distances.iter().enumerate().min_by_key(|(_, distance)| *distance).map(|(side, _)| side) {
                    Some(0) => { cut[2] += cut[0] - r[0]; cut[0] = r[0]; }
                    Some(1) => cut[2] = r[0] + r[2] - cut[0],
                    Some(2) => { cut[3] += cut[1] - r[1]; cut[1] = r[1]; }
                    _ => cut[3] = r[1] + r[3] - cut[1],
                }
            }
            walls = walls.into_iter().flat_map(|r| subtract_rect(r, cut)).collect();
        }
        walls
    }

    fn constrain_zoom(&self, from: (i32, i32), to: (i32, i32)) -> (i32, i32) {
        if self.walls == Walls::Open || self.interface_at(to).is_some() {
            return to;
        }
        self.zoom_pointer_walls().into_iter().fold(to, |p, r| hold_outside(r, from, p, self.window))
    }

    /// The pointer the window is to show, when that changed.
    pub fn take_pointer_icon(&mut self) -> Option<PointerIcon> {
        let icon = if !self.locked && self.capture_hit(self.pointer).is_some_and(|h| self.capture_refused(h).is_none()) { PointerIcon::Capture } else { PointerIcon::Default };
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
        if self.sharp_text && self.world_shown && self.beside.is_none() {
            let views = std::iter::once(self.main_view).chain(self.split.map(|s| s.view)).chain(self.insets.iter().map(|i| i.view));
            let scale = self.world_scale;
            let magnifier_downscale = self.shown.iter().find(|(h, _)| *h == Hit::Magnifier)
                .map_or_else(|| self.main.map_or(1, |c| c.downscale.max(1)), |(_, c)| c.downscale.max(1));
            let world = self.magnifier.filter(|m| m.mode == MagnifierMode::World)
                .map(|m| times(ZOOMS[m.zoom], (1, magnifier_downscale)));
            let used = views.map(|v| v.zoom_by(scale)).chain(world).filter(|&z| z != (1, 1));
            // (The fitted screen's zoom, too.)
            let mut used: Vec<(u32, u32)> = used.chain(self.fit.or(self.placed_screen).map(|f| f.zoom)).filter(|z| z.0 <= 255 && z.1 <= 255).collect();
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
        let world_rect = |c: &Camera| {
            let (w, h) = c.world_size();
            (c.x, c.y, w as i32, h as i32)
        };
        let rect = match self.shown.iter().find(|(h, _)| *h == Hit::Main) {
            Some((_, c)) if self.fit.is_none() => {
                let shown = world_rect(c);
                // A single pane can still crop or move the driver's view
                // through resolution scaling, world zoom, or a glide.
                if self.main.as_ref().map(world_rect) == Some(shown) { (0, 0, 0, 0) } else { shown }
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
            let (w, h) = c.world_size();
            *slot = (c.x, c.y, w as i32, h as i32);
        }
        (views != self.reported_views).then(|| {
            self.reported_views = views;
            views
        })
    }

    /// The pane assigned to the driver's screen: the main pane with
    /// `ui_in_pane`, else where the settings put the named screen the
    /// driver names (`screen`; a vertical split's left or right view, a
    /// horizontal one's bottom or top view).
    fn fit_target(&self, screen: Option<&str>) -> Option<Hit> {
        if !self.world_shown { return None; }
        let split = self.split?;
        if self.ui_in_pane {
            return Some(Hit::Main);
        }
        screen?;
        self.shared_hit(split)
    }

    /// The pane the one screen place setting names for `split` (`None`:
    /// floating).
    fn shared_hit(&self, split: Split) -> Option<Hit> {
        let vertical = split.axis == Axis::Vertical;
        match (if vertical { self.screen_place.vertical } else { self.screen_place.horizontal }, vertical) {
            (Place::Floating, _) => None,
            (Place::LeftOrBottom, true) | (Place::RightOrTop, false) => Some(Hit::Main),
            (Place::LeftOrBottom, false) | (Place::RightOrTop, true) => Some(Hit::Pane),
        }
    }

    /// The part of the driver's window a named screen actually occupies.
    /// A screen part gives its own bounds. Screens containing a world view
    /// of their own, such as City, use their interface and that view together.
    /// `ui=pane` deliberately keeps the whole driver's window.
    fn fitted_source(&self, scene: &Scene, base: &[Camera], screen_part: Option<usize>, popup: Option<usize>) -> [i32; 4] {
        let (w, h) = self.driver_screen();
        let full = [0, 0, w.max(1) as i32, h.max(1) as i32];
        if self.ui_in_pane {
            return full;
        }
        let rects: Vec<_> = match (self.screen_content, screen_part) {
            (Some(r), _) => std::iter::once(r).chain(popup.map(|i| rect_of(&base[i]))).collect(),
            (_, Some(i)) => std::iter::once(rect_of(&base[i])).chain(popup.map(|i| rect_of(&base[i]))).collect(),
            _ => self.interface.iter().copied().chain(scene.views.iter().zip(base).filter_map(|(v, c)| {
                (!v.view.interface_only && v.view.viewport.0 > 0 && v.view.viewport.1 > 0).then(|| rect_of(c))
            })).collect(),
        };
        let rects: Vec<_> = rects.into_iter().filter_map(|[x, y, rw, rh]| {
            let (l, t, r, b) = (x.max(0), y.max(0), (x + rw).min(full[2]), (y + rh).min(full[3]));
            (r > l && b > t).then_some([l, t, r, b])
        }).collect();
        let Some(first) = rects.first() else { return full };
        let [l, t, r, b] = rects.iter().skip(1).fold(*first, |a, r| [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])]);
        [l, t, r - l, b - t]
    }

    /// The zoom fitting `source` into `rect` (reduced).
    fn screen_fit(source: [i32; 4], rect: [i32; 4]) -> (u32, u32) {
        let (ww, wh) = (source[2].max(1) as u32, source[3].max(1) as u32);
        let (pw, ph) = (rect[2].max(1) as u32, rect[3].max(1) as u32);
        // The smaller of the two ratios, as a fraction.
        let (n, d) = if pw as u64 * wh as u64 <= ph as u64 * ww as u64 { (pw, ww) } else { (ph, wh) };
        let g = gcd(n, d);
        (n / g, d / g)
    }

    /// Whether a view at `zoom` shows the driver's zoomed art.
    pub fn zoomed_art(&self, zoom: (u32, u32)) -> bool {
        self.sharp_text && zoom.0 != zoom.1 && self.art_zooms.iter().any(|&z| z == zoom || z == (0, 0))
    }

    /// A change of mouse capture for the window to make (true: capture and
    /// hide the pointer, report motion to [`Layout::motion`]).
    pub fn take_grab(&mut self) -> Option<bool> {
        self.grab.take()
    }

    /// The mouse was let go (e.g. the window lost focus).
    pub fn released(&mut self) {
        self.cancel_unit_order();
        self.last_world_pointer = None;
        self.menu_pan.cancel_drag();
        // (Back in the window a click captures it: nothing pending does.)
        self.forget_resume();
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
            let (w, h) = camera.world_size();
            self.cursor = (camera.x + w as i32 / 2, camera.y + h as i32 / 2);
        }
        self.locked = true;
        self.grab = Some(true);
        self.against = None;
        if self.menu_open() {
            log::info!("viewer: mouse captured in {hit:?} beside the open menu, it moves the world cursor at {:?} (a click on the menu, Ctrl on its own or Ctrl+Shift+Space releases it)", self.cursor);
        } else {
            log::info!("viewer: mouse captured, it moves the world cursor at {:?} (Ctrl on its own or Ctrl+Shift+Space releases it)", self.cursor);
        }
    }

    /// Releases the mouse (never captures it).
    fn release(&mut self, why: &str) {
        let was = std::mem::replace(&mut self.locked, false);
        self.pointer_at_edge = false;
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
            // A fixed zoom box may cover the captured view's centre.
            // Release the physical pointer outside its reserved area.
            let p = self.constrain_zoom(p, p);
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
        self.cancel_unit_order();
        self.last_world_pointer = None;
        self.menu_pan.cancel_drag();
        self.side = None;
        self.against = None;
        self.pan_pointer_gone();
        let mut out = self.take_unit_order_events();
        out.extend(self.edge_change((0, 0)));
        out
    }

    /// `Input::Edge` for the pointer now at `edge`, if that changed.
    fn edge_change(&mut self, edge: (i32, i32)) -> Vec<Input> {
        if edge == self.edge {
            return vec![];
        }
        self.edge = edge;
        vec![Input::Edge { x: edge.0, y: edge.1 }]
    }

    /// The native camera is also the visible A camera only for drivers
    /// without independent focus requests and without a local manual pan.
    fn native_main_follow(&self) -> bool {
        self.main_view.follow == Follow::Driver && self.camera_focus_serial.is_none() && !self.main_panned_by_hand()
    }

    /// Whether the main pane shows the driver's main view unmoved (or
    /// gliding back to it): following it at zoom 1, each window pixel the
    /// driver's own (perhaps cropped), its screen not fitted into a pane.
    #[cfg_attr(not(test), allow(dead_code))]
    fn main_unmoved(&self) -> bool {
        self.main.is_some() && self.native_main_follow() && self.main_view.zoom == ONE && self.fit.is_none() && self.shown.iter().any(|(h, _)| *h == Hit::Main)
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
    pub(crate) fn panes(&self) -> ([i32; 4], Option<[i32; 4]>) {
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

    /// The rectangle a view's own interface is laid out in (the corner's
    /// buttons, the hint shown in their place, and whatever else belongs
    /// with that view): the view's pane after every reservation the views
    /// give room to (a zoom strip; the split's divider). Without a split
    /// there is one view, A.
    pub(crate) fn view_area(&self, side: Side) -> [i32; 4] {
        match (side, self.panes()) {
            (Side::B, (_, Some(b))) if self.split.is_some() => b,
            (_, (a, _)) => a,
        }
    }

    /// Where the unit's order buttons go (none: where the driver put them,
    /// as with one view): split at a side by side split's gap, or inside
    /// the active view's area (`UnitButtons`).
    fn unit_orders_place(&self) -> Option<UnitOrdersPlace> {
        let split = self.split.filter(|_| self.world_shown && self.reachable.is_some())?;
        match self.unit_buttons {
            UnitButtons::Spanning if split.axis == Axis::Vertical => self.gap().map(|(across, _)| UnitOrdersPlace::Gap(across)),
            UnitButtons::Spanning => None,
            UnitButtons::ActiveView => Some(UnitOrdersPlace::In(self.view_area(self.active_side()))),
        }
    }

    /// The unit's order buttons (`scene::Interface::reachable`) where they
    /// are drawn (canvas pixels): their pieces as shown, the scale of the
    /// game's frame and the place `unit_orders_place` gave them applied;
    /// where the driver put them before the interface was arranged.
    fn orders_shown(&self) -> Option<[i32; 4]> {
        let r = self.reachable?;
        let within = |d: [i32; 4]| d[2] > 0 && d[3] > 0 && d[0] >= r[0] && d[1] >= r[1] && d[0] + d[2] <= r[0] + r[2] && d[1] + d[3] <= r[1] + r[3];
        let drawn = self.interface_shown.iter().flatten().filter(|s| within(s.driver)).map(|s| s.rect).reduce(|a, b| {
            let (x, y) = (a[0].min(b[0]), a[1].min(b[1]));
            [x, y, (a[0] + a[2]).max(b[0] + b[2]) - x, (a[1] + a[3]).max(b[1] + b[3]) - y]
        });
        Some(drawn.unwrap_or(r))
    }

    /// The zoom box or strip as the last frame showed it (canvas pixels).
    pub(crate) fn zoom_area(&self) -> Option<[i32; 4]> {
        self.shown.iter().find(|(h, _)| *h == Hit::Magnifier).map(|(_, c)| rect_of(c))
    }

    /// The panes in `area`.
    fn panes_in(&self, [x, y, w, h]: [i32; 4]) -> ([i32; 4], Option<[i32; 4]>) {
        if !self.world_shown { return ([x, y, w, h], None); }
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
    pub(crate) fn gap(&self) -> Option<((i32, i32), (i32, i32))> {
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
    pub(crate) fn open_spans(&self) -> Vec<(i32, i32)> {
        let Some((_, (s0, s1))) = self.gap() else { return vec![] };
        let mut spans = Vec::new();
        if !self.locked {
            let third = (s1 - s0) / 3;
            spans.push((s0 + third, s1 - third));
        }
        if let (Some([x, y, w, h]), Some(split)) = (self.orders_shown(), self.split) {
            // Where they are drawn, along all of their rows (columns),
            // whether they reach over the gap or not: the pointer crosses
            // from one view to the other anywhere there.
            let (a0, a1) = match split.axis {
                Axis::Vertical => (y, y + h),
                Axis::Horizontal => (x, x + w),
            };
            if a1 > a0 {
                spans.push((a0.max(s0), a1.min(s1)));
            }
        }
        spans.retain(|&(a, b)| b > a);
        spans
    }

    /// The gap's closed stretches (x, y, w, h), and whether the split is
    /// side by side.
    fn divider_pieces(&self) -> Vec<([i32; 4], bool)> {
        let (Some(((g0, g1), (s0, s1))), Some(split)) = (self.gap(), self.split) else { return vec![] };
        let vertical = split.axis == Axis::Vertical;
        let mut spans = self.open_spans();
        spans.sort();
        let mut pieces = Vec::new();
        let mut at = s0;
        for (a, b) in spans.into_iter().chain(std::iter::once((s1, s1))) {
            if a > at {
                pieces.push(if vertical { [g0, at, g1 - g0, a - at] } else { [at, g0, a - at, g1 - g0] });
            }
            at = at.max(b);
        }
        pieces.into_iter().map(|r| (r, vertical)).collect()
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
        if self.walls == Walls::Open || self.pointer_free_everywhere() {
            self.side = self.side_of(to);
            return to;
        }
        let Some(((g0, g1), _)) = self.gap().filter(|_| !self.locked && self.drag.is_none() && self.beside.is_none()) else {
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
        let reachable = self.orders_shown().is_some_and(|r| {
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
        // A box's width: `1 : n : 1` of its region's width (margin : box :
        // margin); its height `zoom_strip` of the region's.
        let width = |region: [i32; 4], n: u32| (region[2] as i64 * n.max(1) as i64 / (n.max(1) as i64 + 2)) as i32;
        let part = |region: [i32; 4], thousandths: u32| ((region[3] as i64 * thousandths as i64 / 1000) as i32).clamp(0, region[3]);
        // A box does not dodge the views' own interface (the corner's
        // buttons, the unit's orders): it lies under it, which stays drawn
        // on top and takes its clicks (`Layout::hit`).
        match m.place {
            ZoomPlace::Beside => {}
            ZoomPlace::Box(n, zb) => {
                // At the top of its region, `top` below it, centred across.
                let r = self.zoom_region(m);
                let (w, h) = (width(r, n), part(r, zb.height));
                let top = (r[1] + part(r, zb.top)).min(r[1] + r[3] - h);
                let [x, y] = [r[0] + (r[2] - w) / 2, top];
                let p = if self.locked { self.cursor_on_window().unwrap_or(self.pointer) } else { self.pointer };
                if (!self.zoom_box_block_pointer || self.locked) && self.interface_at(p).is_none() && inside([x - GAP, y - GAP, w + 2 * GAP, h + 2 * GAP], p) {
                    // The pointer came to it: it drops, its top `drop`
                    // below the pointer (at least clear of its arrow), as
                    // far as its region's bottom lets it.
                    let below = p.1 + POINTER_CLEARANCE;
                    let by = (p.1 + part(r, zb.drop)).max(below).min(r[1] + r[3] - h);
                    if by >= below {
                        return ([x, by, w, h], None);
                    }
                    // No room for all of it below the pointer (a tall box
                    // in a short region): cut it to what is below rather
                    // than covering the map pixel the user is pointing at,
                    // else to what is above.
                    if below < r[1] + r[3] {
                        return ([x, below, w, r[1] + r[3] - below], None);
                    }
                    let height = h.min((p.1 - GAP - r[1]).max(1));
                    return ([x, p.1 - GAP - height, w, height], None);
                }
                return ([x, y, w, h], None);
            }
            ZoomPlace::AboutPointer(n, thousandths) => {
                // Its height `zoom_strip` of its region's. Above the pointer
                // (the world cursor while the mouse is captured), else below
                // it, clear of its arrow; centred on it across; inside its
                // region; under the views' interface rather than dodging it.
                let r = self.zoom_region(m);
                let w = width(r, n);
                let h = ((r[3] as i64 * thousandths as i64 / 1000) as i32).clamp(0, r[3]);
                let (px, py) = if self.locked { self.cursor_on_window().unwrap_or(self.pointer) } else { self.pointer };
                let x = (px - w / 2).clamp(r[0], (r[0] + r[2] - w).max(r[0]));
                let above = py - GAP - h;
                let y = if above >= r[1] { above } else { py + POINTER_CLEARANCE };
                let y = y.clamp(r[1], (r[1] + r[3] - h).max(r[1]));
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

    /// Render a world magnifier from the full-size art, even when the
    /// game shows the world smaller. Keep its logical camera in `shown`
    /// for input: the driver still expects its own world pixels.
    fn magnifier_art_camera(&self, camera: Camera) -> Camera {
        let n = camera.downscale.max(1);
        let zoom = times(camera.zoom, (1, n));
        Camera {
            x: camera.x * n as i32,
            y: camera.y * n as i32,
            zoom,
            downscale: 1,
            zoomed_art: self.zoomed_art(zoom),
            ..camera
        }
    }

    fn views_mut(&mut self) -> impl Iterator<Item = &mut FreeView> {
        std::iter::once(&mut self.main_view).chain(self.split.as_mut().map(|s| &mut s.view)).chain(self.insets.iter_mut().map(|i| &mut i.view))
    }

    fn follows_pointer(&self) -> bool {
        self.main_view.follow == Follow::Pointer
            || self.explore.is_some()
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
        self.focus_after_scale = None;
        // The driver publishes an empty world outside a game. Keep its
        // title and setup screens independent of the configured map views.
        let had_world = std::mem::replace(&mut self.world_shown, scene.shape.cols > 0 && scene.shape.rows > 0);
        if let Some(beside) = scene.interface.beside {
            return self.frame_beside(scene, overhang, beside, bounds);
        }
        self.beside = None;
        self.beside_mapping = None;
        self.beside_of = None;
        self.leave_explore("the screen beside its world closed");
        if !self.world_shown {
            return self.frame_without_world(scene, overhang, interface, had_world);
        }
        let dt = std::mem::take(&mut self.step).min(MAX_STEP);
        // (The driver's views filling its screen: as large as that.)
        let base = crate::scene_cameras(scene, self.driver_screen(), overhang);
        // Its own views show the world as the driver's do (their world
        // pixels are those views' pixels).
        let downscale = crate::world_downscale(scene);
        if self.world_shown { self.rescale_world(downscale); }
        self.map_wheel_step = scene.map_wheel_step;
        self.minimap = scene.minimap;
        // Remember the opening view before a menu releases captured input.
        // Moving over its controls or resizing later must not send it elsewhere.
        if scene.interface.screen.as_deref() == Some("Menu") {
            if self.screen.as_deref() != Some("Menu") {
                self.menu_side = Some(self.active_side());
            }
        } else {
            self.menu_side = None;
        }
        // The driver's main view: its first world view filling the window.
        // (Not Preferences, `menu_fit`: it is fitted as at the title, the
        // window or the pane the screen place setting names.)
        let following_menu = !self.ui_in_pane && self.menu_follows_pointer && !scene.interface.menu_fit && self.split.is_some_and(|s| self.shared_hit(s).is_none()) && scene.interface.screen.as_deref() == Some("Menu");
        let named_target = (!self.ui_in_pane).then(|| self.fit_target(scene.interface.screen.as_deref())).flatten().or(following_menu.then_some(Hit::Main));
        let main = scene.views.iter().position(|v| !v.view.interface_only && (v.view.viewport.0 == 0 || v.view.viewport.1 == 0))
            .or_else(|| (named_target.is_some() && self.world_shown).then(|| scene.views.iter().position(|v| v.view.viewport.0 == 0 || v.view.viewport.1 == 0)).flatten());
        self.main = main.map(|i| Camera { cells: true, ..base[i] });
        // The parts of the driver's interface the viewer arranges.
        let parts = &scene.interface;
        let part = |id: Option<u32>| id.and_then(|id| scene.views.iter().position(|v| v.id == id && v.view.interface_only));
        // (The popup: the dialog on top, with what is attached to it, e.g.
        // its combo box's open list, `scene::Interface::placed_popup`.)
        let placed_popup = parts.placed_popup();
        let (roaming, popup, screen_part, hint) = (part(parts.roaming), part(placed_popup), part(parts.screen_view), part(parts.capture_hint));
        let popup_layers: Vec<usize> = parts.popup_views().filter_map(|id| part(Some(id))).collect();
        // While the mouse is captured the corner's buttons cannot be used:
        // the hint how to let it go shows in their place, in the view
        // following the world cursor (the active view). While snapping (it
        // points at the interface) the buttons show, to be snapped to.
        let hinting = hint.is_some() && self.locked && !self.snapping;
        self.reachable = parts.reachable;
        if parts.screen != self.screen {
            if parts.screen.is_some() { self.cancel_unit_order(); }
            match &parts.screen {
                Some(name) => log::info!("viewer: screen {name} open, placed {}", self.fit_target(Some(name)).or_else(|| following_menu.then(|| self.menu_side.unwrap_or(Side::A).hit())).map_or("over the window", |h| if h == Hit::Main { "in A" } else { "in B" })),
                None => log::info!("viewer: screen {} closed", self.screen.as_deref().unwrap_or_default()),
            }
            // A screen opening lets go of a captured mouse, the pointer on
            // its main control (below, once it is placed); captured again
            // once no menu is open (`settle_capture`, below).
            if let (Some(_), None) = (&parts.screen, &self.screen) {
                if self.locked {
                    self.suspend_capture();
                    self.release_here("a screen opened");
                    self.pointer_to_screen = true;
                }
            }
        }
        self.clock += dt;
        // A popup over a menu (a screen part or the menus): placed over that
        // menu (`scaling::MODAL_TOP`), never at a click nor in a pane.
        let over_menu = parts.screen.is_some() || screen_part.is_some() || scene.views.iter().any(|v| v.view.interface_only && v.view.menu && !v.overlay.is_empty());
        let previous_popup = self.popup_view;
        if let (Some(id), Some((anchor, press))) = (previous_popup, self.anchored) {
            if Some(id) != placed_popup && parts.modal_stack.contains(id) {
                self.anchored_popup_layers.retain(|(view, _, _)| *view != id);
                self.anchored_popup_layers.push((id, anchor, press));
            }
        }
        self.anchored_popup_layers.retain(|(id, _, _)| parts.is_popup_view(*id));
        self.detached_update(scene);
        self.popup_view = placed_popup;
        self.popup_body = popup.map(|i| rect_of(&base[i]));
        if popup.is_some() != self.popup_shown {
            self.popup_shown = popup.is_some();
            log::info!("viewer: popup {}", if self.popup_shown { "open, a view of its own" } else { "closed" });
            match popup {
                Some(i) => { self.cancel_unit_order(); self.popup_opened(rect_of(&base[i]), scene.interface.popup_anchor, over_menu); }
                None => self.popup_closed(),
            }
        }
        // Another popup in its place (e.g. a dialog a menu's entry opened):
        // anchored at a click only if the driver says so for it.
        if let Some(i) = popup.filter(|_| self.popup_anchor_seen != scene.interface.popup_anchor) {
            if self.anchored_popup_layers.is_empty() { self.popup_pan = None; }
            match scene.interface.popup_anchor {
                Some(a) => self.popup_opened(rect_of(&base[i]), Some(a), over_menu),
                None => (self.anchored, self.popup_anchor_seen) = (None, None),
            }
        }
        if previous_popup != placed_popup && self.anchored.is_none() {
            if let Some(index) = self.anchored_popup_layers.iter().position(|(id, _, _)| Some(*id) == placed_popup) {
                let (_, anchor, press) = self.anchored_popup_layers.remove(index);
                self.anchored = Some((anchor, press));
            }
        }
        self.screen = parts.screen.clone();
        // With the screen and popups known: no capture under a menu, and
        // one a menu interrupted taken up once all are gone (`capture`).
        self.cover_menus(following_menu, over_menu);
        self.settle_capture();
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
        // At the window's edge or pushed against the divider: the view
        // there scrolls through the same pixel movement in A and B;
        // a view let go slides on (`pan`).
        self.pan_frame(dt, fit_target.filter(|_| self.ui_in_pane), scene.camera_focus.is_none());
        if scene.camera_focus.map(|focus| focus.serial) != self.camera_focus_serial {
            self.camera_focus_serial = scene.camera_focus.map(|focus| focus.serial);
            self.semantic_focus_active = scene.camera_focus.is_some();
            self.focus_at_camera_request = scene.focus;
            self.focus_at_camera_request_rest = (0.0, 0.0);
            if self.semantic_focus_active {
                self.resume_game_focus();
            }
        } else if self.semantic_focus_active && scene.focus != self.focus_at_camera_request {
            // Selecting or moving a unit may not move the parked native
            // camera. Resume Focus mode without reviving an older AI focus
            // if the selected unit later returns to the request's baseline.
            self.semantic_focus_active = false;
        }
        if let Some(b) = bounds {
            self.cursor = b.clamp_point(self.cursor);
        }
        let driver = self.main.map(|m| (m.x + m.viewport.0 as i32 / 2, m.y + m.viewport.1 as i32 / 2));
        let semantic_main = scene.camera_focus.map(|focus| focus.center);
        let initial_driver = semantic_main.or(driver);
        // The world cursor starts where the driver first shows a world (a
        // view following it shows that until the mouse is captured there).
        if !self.cursor_placed && !self.locked {
            if let Some(d) = initial_driver.filter(|_| scene.shape.cols > 0 && scene.shape.rows > 0) {
                if !self.follows_pointer() || self.world_frames > SETTLE_FRAMES || semantic_main.is_some() {
                    self.cursor_placed = true;
                }
                self.cursor = d;
            }
        }
        // The main pane following the driver shows the middle of the
        // driver's view in its own middle (a split or a zoom strip crops it
        // round its middle): what the driver centres on (the active unit,
        // a popup's subject, a place looked at) is in the middle of A.
        let main_driver = semantic_main.or(driver);
        let (cursor, focus) = (self.cursor, scene.focus);
        let main_focus = if self.semantic_focus_active { semantic_main.or(focus).or(driver) } else { focus.or(semantic_main).or(driver) };
        let has_world = scene.shape.cols > 0 && scene.shape.rows > 0;
        self.world_frames = if has_world { self.world_frames.saturating_add(1) } else { 0 };
        let settled = self.world_frames > SETTLE_FRAMES || semantic_main.is_some();
        let period = bounds.map_or((0, 0), |b| (if b.wrap.0 { b.size.0 } else { 0 }, if b.wrap.1 { b.size.1 } else { 0 }));
        let smooth = self.smooth;
        let main_panned_by_hand = self.main_panned_by_hand();
        self.world_period = period;
        let popup_pan = self.popup_pan.and_then(|(hit, by)| match hit {
            Hit::Main => Some((0, by)),
            Hit::Pane => Some((1, by)),
            Hit::Inset(i) => Some((2 + i, by)),
            _ => None,
        });
        let capture_focus = self.locked && self.semantic_focus_active;
        let combat_focus = self.semantic_focus_active && scene.camera_arrival.is_some_and(|a| scene.camera_focus.is_some_and(|f| f.serial == a.serial));
        let arrival_area = scene.camera_arrival.filter(|a| scene.camera_focus.is_some_and(|f| f.serial == a.serial)).map(|a| a.bounds);
        let focus_scale = self.world_scale;
        let tile_half = (self.grid_step.0.div_ceil(downscale) as i32, self.grid_step.1.div_ceil(downscale) as i32);
        let focus_rects: Vec<_> = if capture_focus { std::iter::once(main_rect).chain(pane_rect.filter(|_| self.split.is_some())).chain(self.insets.iter().map(|i| i.rect)).collect() } else { Vec::new() };
        // A view a pan gesture moves (grabbed, sliding, edge scrolled) is
        // that gesture's alone until it ends: what it follows must not pull
        // it back between the gesture's steps (`pan`). What the focus did
        // meanwhile is the user's to override.
        let held: Vec<bool> = std::iter::once(Hit::Main)
            .chain(self.split.map(|_| Hit::Pane))
            .chain((0..self.insets.len()).map(Hit::Inset))
            .map(|hit| self.pan_holds(hit))
            .collect();
        for (k, v) in self.views_mut().enumerate() {
            // (A view following the world cursor is moved through the
            // cursor, below.)
            if held[k] && v.center != UNPLACED && v.follow != Follow::Pointer {
                if v.follow == Follow::Focus {
                    v.hand_focus = focus.or(Some(UNPLACED));
                }
                v.glide.snap(v.center);
                continue;
            }
            let target = match v.follow {
                Follow::Driver if k == 0 && main_panned_by_hand => None,
                Follow::Driver if k == 0 => main_driver,
                Follow::Driver => driver,
                Follow::Focus => match v.hand_focus {
                    // Moved by hand: there until the focus moves.
                    Some(h) if !capture_focus && !combat_focus && focus.unwrap_or(UNPLACED) == h => None,
                    _ => {
                        v.hand_focus = None;
                        if k == 0 || capture_focus || combat_focus { main_focus } else { focus.or(driver) }
                    }
                },
                Follow::Pointer => Some(cursor),
                // Placed where the driver first shows a world (not before: a
                // title screen has none, its empty view looks at (0, 0)).
                Follow::Free if v.center == UNPLACED || v.settling => {
                    match initial_driver.filter(|_| has_world) {
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
            let keep_tile = capture_focus && (v.follow == Follow::Focus || (k == 0 && v.follow == Follow::Driver));
            let target = if keep_tile {
                target.map(|t| {
                    let (point, half) = arrival_area.map(|[l, top, r, bottom]| ((l + (r-l) / 2, top + (bottom-top) / 2), ((r-l+1) / 2, (bottom-top+1) / 2))).unwrap_or((t, tile_half));
                    v.expose_tile(point, half, focus_rects[k], bounds, focus_scale)
                })
            } else { target };
            // (Moved for a popup opened at a click to fit the window.)
            let target = match popup_pan {
                Some((j, by)) if j == k && v.follow == Follow::Pointer => target.map(|t| (t.0 + by.0, t.1 + by.1)),
                _ => target,
            };
            match target {
                Some(t) if keep_tile && t == v.center => v.glide.snap(t),
                // Gliding there (the world cursor's view keeps it in its
                // middle, a free view is moved by hand).
                Some(t) if smooth && matches!(v.follow, Follow::Driver | Follow::Focus) => {
                    v.scale_rest = (0.0, 0.0);
                    v.center = v.glide.step(t, dt, period);
                },
                Some(t) => {
                    v.scale_rest = (0.0, 0.0);
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
        self.view_ui.clear();
        let menu_region = following_menu.then(|| if self.menu_side == Some(Side::B) { pane_rect.unwrap_or(main_rect) } else { main_rect });
        let mut frame = Frame { view_cameras: vec![None; scene.views.len()], popup_region: popup.filter(|_| !over_menu).and_then(|_| self.popup_region()), menu_region, ..Default::default() };
        // A modal popup over the menu (`over_menu`) keeps out of the screen
        // fitted into a pane: that screen's fit stays as without it, the
        // popup goes over it in that pane (`scaling::MODAL_TOP`).
        // (Not one the driver opened at a click on its screen, e.g. a city
        // screen garrison unit's menu: that one stays at the click, with the
        // screen.)
        let on_screen = parts.popup_on_screen && parts.screen.is_some() && parts.screen_view.is_none();
        let modal = popup.filter(|_| over_menu && self.anchored.is_none() && !on_screen);
        let push = |frame: &mut Frame, camera: Camera| {
            frame.cameras.push(camera);
            frame.cameras.len() as u32 - 1
        };
        // The driver's views: either the main one arranged, the others (and
        // the interface) over the viewer's own views where the driver put
        // them. Explicit `ui_in_pane` fits the driver's whole window as one
        // picture. A named panel instead gets its own group over the normal
        // world panes, with its one-pixel lines kept.
        // (Its head: the main view, else the first filling the window, e.g.
        // the game's view hidden by a city screen.)
        let head = main.or_else(|| scene.views.iter().position(|v| v.view.viewport.0 == 0 || v.view.viewport.1 == 0)).or((!scene.views.is_empty()).then_some(0));
        let placement = fit_target.filter(|_| head.is_some()).and_then(|hit| {
            let rect = if hit == Hit::Main { main_rect } else { pane_rect? };
            let source = self.fitted_source(scene, &base, screen_part, popup.filter(|_| modal.is_none()));
            let zoom = Self::screen_fit(source, rect);
            let size = ((source[2] as u32 * zoom.0).div_ceil(zoom.1), (source[3] as u32 * zoom.0).div_ceil(zoom.1));
            // A named panel fills its pane as far as its aspect ratio allows,
            // with the remaining letterbox space shared on both sides.
            let cropped = source != [0, 0, self.driver_screen().0 as i32, self.driver_screen().1 as i32];
            let origin = if cropped { (rect[0] + (rect[2] - size.0 as i32) / 2, rect[1] + (rect[3] - size.1 as i32) / 2) } else { (rect[0], rect[1]) };
            Some(Fit { hit, source, origin, zoom })
        });
        let fit = placement.filter(|_| self.ui_in_pane);
        self.fit = fit;
        self.placed_screen = placement.filter(|_| !self.ui_in_pane);
        let named: Vec<usize> = if self.placed_screen.is_some() {
            let dedicated = screen_part.is_some() || popup.is_some() || scene.views.iter().any(|v| v.view.interface_only && v.view.menu);
            scene.views.iter().enumerate().filter_map(|(i, v)| {
                if Some(i) == main || Some(i) == roaming || Some(i) == hint || (popup_layers.contains(&i) && modal.is_some()) { return None; }
                let city_map = self.screen.as_deref() == Some("City") && !v.view.interface_only && v.view.viewport.0 > 0 && v.view.viewport.1 > 0;
                let ui = v.view.interface_only && (self.screen.as_deref() == Some("City") || (!dedicated && screen_part.is_none()) || (screen_part.is_none() && v.view.menu) || Some(i) == screen_part || (popup.is_some() && popup_layers.contains(&i) && part(Some(parts.popup_root(v.id))) == popup && modal.is_none()));
                (ui || city_map).then_some(i)
            }).collect()
        } else { Vec::new() };
        if let Some(fit) = self.placed_screen {
            self.interface.retain(|r| !overlap(*r, fit.source));
            if modal.is_some() {
                frame.menu_region = Some(if fit.hit == Hit::Main { main_rect } else { pane_rect.unwrap_or(main_rect) });
            }
        }
        let mut over: Vec<(usize, Camera)> = Vec::new();
        // (Drawn over the screen fitted into a pane.)
        let mut on_top: Vec<(usize, Camera)> = Vec::new();
        if let (Some(fit), Some(m)) = (fit, head) {
            let zoom = fit.zoom;
            let [sx, sy, sw, sh] = fit.source;
            let head = Camera {
                x: base[m].x + sx,
                y: base[m].y + sy,
                zoom,
                origin: fit.origin,
                viewport: ((sw as u32 * zoom.0).div_ceil(zoom.1), (sh as u32 * zoom.0).div_ceil(zoom.1)),
                screen_shift: (fit.origin.0 - sx, fit.origin.1 - sy),
                zoomed_art: self.zoomed_art(zoom),
                ..base[m]
            };
            frame.view_cameras[m] = Some(push(&mut frame, head));
            if Some(m) == main {
                self.shown.push((fit.hit, head));
            }
            // (The roaming controls too, where the driver put them.)
            let layers: Vec<_> = parts.popup_views().collect();
            let mut order: Vec<_> = (0..base.len()).filter(|&index| index != m && Some(index) != hint).collect();
            order.sort_by_key(|&index| layers.iter().position(|&id| id == scene.views[index].id).map_or(0, |layer| layer + 1));
            for i in order {
                let camera = &base[i];
                let stacked = Camera {
                    origin: (camera.origin.0 - sx, camera.origin.1 - sy),
                    screen_shift: (camera.screen_shift.0 - sx, camera.screen_shift.1 - sy),
                    stacked: true,
                    hairline: true,
                    zoomed_art: self.zoomed_art(zoom),
                    ..*camera
                };
                frame.view_cameras[i] = Some(push(&mut frame, stacked));
            }
            // The world beside the screen may have the mouse captured (the
            // map is played there meanwhile): the hint how to let it go at
            // that pane's top-left, where its corner buttons would be.
            if let Some(i) = hint.filter(|_| hinting) {
                let world_rect = if fit.hit == Hit::Main { pane_rect } else { Some(main_rect) };
                if let Some(r) = world_rect.filter(|_| self.split.is_some()) {
                    let (d, camera) = ((r[0], r[1]), base[i]);
                    frame.view_ui_region = Some(r);
                    self.view_ui.push(self.view_ui_rect(rect_of(&camera), d, r));
                    over.push((i, Camera { origin: (camera.origin.0 + d.0, camera.origin.1 + d.1), screen_shift: d, ..camera }));
                }
            }
            // The screen in the second pane: the main pane shows the world.
            if fit.hit == Hit::Pane {
                let v = self.main_view;
                let mut camera = v.camera(main_rect, overhang, if self.native_main_follow() { None } else { bounds }, self.world_scale);
                camera.zoomed_art = self.zoomed_art(camera.zoom);
                camera.downscale = downscale;
                push(&mut frame, camera);
                self.shown.push((Hit::Main, camera));
            }
        } else {
            let roaming_shift = self.roaming_shift();
            let ui_area = self.view_area(self.active_side());
            if roaming.is_some() || hint.is_some() {
                frame.view_ui_region = Some(ui_area);
            }
            frame.unit_orders = self.unit_orders_place();
            // (A modal one is placed over its menu: `scaling::MODAL_TOP`.)
            let popup_shift = popup.map(|i| if modal.is_some() || on_screen { (0, 0) } else { self.popup_shift(rect_of(&base[i]), main_rect) });
            let screen_shift = screen_part.map(|i| Some(self.screen_shift(rect_of(&base[i]))));
            for (i, (v, camera)) in scene.views.iter().zip(&base).enumerate() {
                let inactive_menu = screen_part.is_some() && v.view.interface_only && v.view.menu;
                let inactive_map = Some(i) != main && !v.view.interface_only && v.view.viewport.0 > 0 && v.view.viewport.1 > 0;
                if named.contains(&i) || ((self.placed_screen.is_some() || following_menu) && (inactive_menu || inactive_map)) { continue; }
                let shift = if Some(i) == roaming {
                    Some(roaming_shift.filter(|_| !hinting))
                } else if Some(i) == hint {
                    Some(roaming_shift.filter(|_| hinting))
                } else if popup_layers.contains(&i) {
                    // (One attached to the dialog under it, a combo box's list: as
                    // that dialog.)
                    let root = part(Some(parts.popup_root(v.id))).unwrap_or(i);
                    Some(if modal.is_some() { Some((0, 0)) } else if Some(root) == popup { popup_shift } else { Some(self.popup_shift(rect_of(&base[root]), main_rect)) })
                } else if Some(i) == screen_part {
                    screen_shift
                } else {
                    None
                };
                if Some(i) == main {
                    let camera = self.arranged_main(&Camera { cells: true, ..*camera }, main_rect, overhang);
                    frame.view_cameras[i] = Some(push(&mut frame, camera));
                    self.shown.push((Hit::Main, camera));
                } else if let Some(shift) = shift {
                    // A part of the interface drawn on its own, where it goes.
                    let Some(d) = shift else { continue };
                    let r = rect_of(camera);
                    if Some(i) == roaming || Some(i) == hint {
                        self.view_ui.push(self.view_ui_rect(r, d, ui_area));
                    }
                    self.moved.push(([r[0] + d.0, r[1] + d.1, r[2], r[3]], d));
                    let moved = Camera { origin: (camera.origin.0 + d.0, camera.origin.1 + d.1), screen_shift: d, ..*camera };
                    if popup_layers.contains(&i) && modal.is_some() { on_top.push((i, moved)) } else { over.push((i, moved)) }
                } else {
                    over.push((i, *camera));
                    if !v.view.interface_only {
                        self.driver_rects.push(rect_of(camera));
                    }
                }
            }
            // The popups in the driver's order, bottom to top (a combo box's
            // open list over its dialog), not in the order of their views' ids.
            let layer = |&(i, _): &(usize, Camera)| popup_layers.iter().position(|&l| l == i).map_or(0, |k| k + 1);
            over.sort_by_key(layer);
            on_top.sort_by_key(layer);
        }
        // The viewer's own views, as many as the renderer draws (the second
        // pane first, then the magnifier, then the insets in order, then
        // what fills the open stretches of the divider).
        let named_cost = named.len() + usize::from(!named.is_empty());
        let mut room = tile_render::MAX_VIEWS.saturating_sub(frame.cameras.len() + over.len() + on_top.len() + named_cost);
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
            let mut camera = split.view.camera(rect, overhang, bounds, self.world_scale);
            camera.zoomed_art = self.zoomed_art(camera.zoom);
            camera.downscale = downscale;
            push(&mut frame, camera);
            self.shown.push((Hit::Pane, camera));
        }
        // Each pane's picture goes on under its half of the gap; the glass
        // over it where the divider is shown (`glass`).
        let mut halves = Vec::new();
        for (rect, camera) in self.gap_halves() {
            if !take(1) {
                break;
            }
            halves.push((rect, push(&mut frame, camera)));
        }
        for (rect, vertical, shade) in self.glass_strips() {
            if let Some(&(_, index)) = halves.iter().find(|(h, _)| inside(*h, (rect[0], rect[1]))) {
                frame.divider.push((index, rect, vertical, shade));
            }
        }
        for (k, inset) in self.insets.iter().enumerate().take(insets) {
            push(&mut frame, frame_of(inset.rect));
            let mut camera = inset.view.camera(inset.rect, overhang, bounds, self.world_scale);
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
            let mut camera = FreeView::new(center.unwrap_or((0, 0)), m.zoom, Follow::Free).camera(rect, overhang, None, (1, 1));
            camera.zoomed_art = self.zoomed_art(camera.zoom);
            camera.downscale = downscale;
            push(&mut frame, self.magnifier_art_camera(camera));
            self.shown.push((Hit::Magnifier, camera));
        }
        // The driver's other views and its interface on top, as it placed
        // them (unless they are in a pane). Mirrored cursors go under them.
        frame.cursor_layer = Some(frame.cameras.len() as u32);
        for (i, camera) in over {
            frame.view_cameras[i] = Some(push(&mut frame, camera));
        }
        if let Some(fit) = self.placed_screen.filter(|_| !named.is_empty()) {
            let [sx, sy, sw, sh] = fit.source;
            // This group holds UI only. Its empty head never copies or
            // transforms the driver's full game view into the pane.
            let head = Camera {
                zoom: fit.zoom,
                origin: fit.origin,
                viewport: ((sw as u32 * fit.zoom.0).div_ceil(fit.zoom.1), (sh as u32 * fit.zoom.0).div_ceil(fit.zoom.1)),
                cells: false,
                hairline: true,
                ..Camera::new((sw as u32, sh as u32))
            };
            let at = push(&mut frame, head);
            frame.placed.push(at);
            // Internal world views, such as the City's surroundings, go
            // under the panel's art, while both world panes stay independent.
            let mut named = named;
            // (A popup's attached combo box list over it.)
            named.sort_by_key(|&i| (scene.views[i].view.interface_only, popup_layers.iter().position(|&l| l == i).map_or(0, |k| k + 1), i));
            for i in named {
                let camera = base[i];
                let stacked = Camera {
                    origin: (camera.origin.0 - sx, camera.origin.1 - sy),
                    screen_shift: (camera.screen_shift.0 - sx, camera.screen_shift.1 - sy),
                    stacked: true,
                    hairline: true,
                    zoomed_art: self.zoomed_art(fit.zoom),
                    ..camera
                };
                let at = push(&mut frame, stacked);
                frame.view_cameras[i] = Some(at);
                frame.placed.push(at);
            }
        }
        for (i, camera) in on_top {
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
                    let mut camera = FreeView::new(center.unwrap_or((0, 0)), m.zoom, Follow::Free).camera(rect, overhang, None, (1, 1));
                    camera.zoomed_art = self.zoomed_art(camera.zoom);
                    camera.downscale = downscale;
                    push(&mut frame, self.magnifier_art_camera(camera));
                    self.shown.push((Hit::Magnifier, camera));
                }
            }
        }
        // The pointer on the popup the mouse was let go for (where it shows:
        // moved into the opening view it may cross the divider's wall).
        if std::mem::take(&mut self.pointer_to_popup) {
            self.focus_after_scale = popup.map(|i| {
                let [x, y, w, h] = rect_of(&base[i]);
                (x + w / 2, y + h / 2)
            });
            let placed = self.placed_screen.and_then(|_| popup.map(|i| {
                let [x, y, w, h] = rect_of(&base[i]);
                self.driver_to_window((x + w / 2, y + h / 2))
            }));
            let moved = popup.and_then(|i| self.moved.iter().find(|(r, d)| *r == { let b = rect_of(&base[i]); [b[0] + d.0, b[1] + d.1, b[2], b[3]] }))
                .map(|&([x, y, w, h], _)| (x + w / 2, y + h / 2));
            if let Some(p) = placed.or(moved) {
                (self.pointer, self.raw_pointer, self.warp) = (p, p, Some(p));
                self.side = self.side_of(p);
            }
        }
        // The pointer on the screen that just opened: its main control (as
        // the driver names it), else its middle, where the screen shows.
        if std::mem::take(&mut self.pointer_to_screen) {
            let screen = screen_part.map(|i| rect_of(&base[i])).or_else(|| self.placed_screen.map(|f| f.source))
                .unwrap_or([0, 0, self.driver_screen().0 as i32, self.driver_screen().1 as i32]);
            let [x, y, w, h] = parts.focus_control.unwrap_or(screen);
            self.focus_after_scale = Some((x + w / 2, y + h / 2));
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
        let inherited = main.map(|i| base[i].hidden_categories).or_else(|| base.first().map(|c| c.hidden_categories)).unwrap_or(0);
        let city_controls: Vec<u32> = if self.screen.as_deref() == Some("City") {
            scene.views.iter().enumerate().filter_map(|(i, v)| {
                (!v.view.interface_only && v.view.viewport.0 > 0 && v.view.viewport.1 > 0)
                    .then(|| frame.view_cameras[i]).flatten()
            }).collect()
        } else { Vec::new() };
        for (index, camera) in frame.cameras.iter_mut().enumerate().filter(|(_, c)| c.cells) {
            // The City's internal map is a UI control whether its panel
            // floats or is assigned to a pane. Preserve workers and yields
            // without exempting either world pane or the world magnifier.
            if camera.stacked || city_controls.contains(&(index as u32)) { camera.hidden_categories = 0; continue; }
            let hit = self.shown.iter().find(|(_, shown)| shown.origin == camera.origin && shown.viewport == camera.viewport).map(|(hit, _)| *hit);
            let setting = match hit {
                Some(Hit::Pane) => self.clean_map.b,
                Some(Hit::Magnifier) => self.clean_map.zoom,
                _ => {
                    // Narrow divider fillers belong to their adjacent pane.
                    let center = (camera.origin.0 + camera.viewport.0 as i32 / 2, camera.origin.1 + camera.viewport.1 as i32 / 2);
                    let in_b = self.split.zip(self.gap()).is_some_and(|(split, ((g0, g1), _))| {
                        let along = if split.axis == Axis::Vertical { center.0 } else { center.1 };
                        along >= g0 + (g1 - g0) / 2
                    });
                    if hit.is_none() && in_b { self.clean_map.b } else { self.clean_map.a }
                }
            };
            camera.hidden_categories = setting.mask(inherited);
        }
        // A popup the driver anchors at a world click: at that click in the
        // view it was made in, as that view shows the world now.
        if let (Some((a, press)), Some(i)) = (self.anchored, popup) {
            let [_, _, w, h] = rect_of(&base[i]);
            frame.popup_at = self.anchored_place(a, press, (w, h)).map(|(at, zoom, _)| (at, zoom));
        }
        for &(id, anchor, press) in &self.anchored_popup_layers {
            if let Some(view) = scene.view(id) {
                let size = (view.view.viewport.0 as i32, view.view.viewport.1 as i32);
                if let Some((at, zoom, _)) = self.anchored_place(anchor, press, size) {
                    frame.popup_places.push((id, (at, zoom)));
                }
            }
        }
        // The tile boxes the viewer keeps, at their clicks; one not placed
        // (closing, or its view gone) is not drawn.
        let detached = self.detached_places();
        for (i, v) in scene.views.iter().enumerate() {
            if scene.interface.detached.iter().any(|d| d.view == v.id) && !detached.iter().any(|(id, _)| *id == v.id) {
                if let Some(camera) = frame.view_cameras.get_mut(i) { *camera = None; }
            }
        }
        frame.popup_places.extend(detached);
        frame
    }

    /// Title and setup screens keep the driver's full screen. Their cameras
    /// also remain the input and Alt-target coordinate system after scaling.
    /// Configured map views stay saved until the driver publishes a world.
    fn frame_without_world(&mut self, scene: &Scene, overhang: (u32, u32, u32, u32), interface: Vec<[i32; 4]>, had_world: bool) -> Frame {
        self.clock += std::mem::take(&mut self.step).min(MAX_STEP);
        if had_world {
            self.cancel_unit_order();
            self.clear_world_motion();
            self.menu_pan.cancel_drag();
            self.drag = None;
            self.last_press = None;
            self.warp = None;
            if self.locked { self.release("the world closed"); }
            for view in self.views_mut() {
                *view = FreeView::new(UNPLACED, view.zoom, view.follow);
            }
        }
        self.bounds = None;
        self.main = None;
        self.fit = None;
        self.placed_screen = None;
        self.shown.clear();
        self.driver_rects.clear();
        self.moved.clear();
        self.view_ui.clear();
        self.interface = interface;
        self.interface_shown = None;
        self.insets_shown = 0;
        self.left_out = 0;
        self.magnifier_shown = false;
        self.magnifier_center = None;
        self.reachable = None;
        self.screen = scene.interface.screen.clone();
        self.popup_shown = scene.interface.popup.is_some();
        self.popup_body = None;
        self.popup_view = None;
        self.anchored_popup_layers.clear();
        self.menu_side = None;
        self.popup_side = None;
        self.popup_anchor = None;
        self.anchored = None;
        self.popup_anchor_seen = None;
        self.chain_press = None;
        self.popup_pan = None;
        self.forget_resume();
        self.pointer_to_screen = false;
        self.pointer_to_popup = false;
        self.against = None;
        self.side = None;
        self.focus = None;
        self.camera_focus_serial = None;
        self.semantic_focus_active = false;
        self.last_world_pointer = None;
        self.cursor_placed = false;
        self.world_downscale = None;
        self.map_wheel_step = (0, 0);
        self.minimap = None;
        self.world_frames = 0;
        self.art_zooms = scene.zooms.clone();
        let base = crate::scene_cameras(scene, self.driver_screen(), overhang);
        // Scene views are keyed by ID, not paint order. In particular the
        // traditional popup257 sorts before the screen258 it overlays.
        let layers: Vec<_> = scene.interface.popup_views().collect();
        let mut order: Vec<_> = (0..scene.views.len()).collect();
        order.sort_by_key(|&index| layers.iter().position(|&id| id == scene.views[index].id).map_or(0, |layer| layer + 1));
        let mut view_cameras = vec![None; base.len()];
        let cameras = order.iter().enumerate().map(|(camera, &view)| {
            view_cameras[view] = Some(camera as u32);
            base[view]
        }).collect();
        Frame { cameras, view_cameras, ..Default::default() }
    }

    /// The frame for a driver asking for its screen and its world side by
    /// side (`scene::Beside`): the screen at the edge it asks for (kept
    /// where it is along that edge), its interface moved with it, and the
    /// whole world fitted into the rest of the window (`fit_world`). None of
    /// the viewer's own views: the arrangement is the driver's.
    fn frame_beside(&mut self, scene: &Scene, overhang: (u32, u32, u32, u32), beside: scene::Beside, bounds: Option<WorldBounds>) -> Frame {
        if self.beside.is_none() {
            self.cancel_unit_order();
            self.clear_world_motion();
            self.drag = None;
            self.last_press = None;
            self.warp = None;
        }
        let base = crate::scene_cameras(scene, self.driver_screen(), overhang);
        self.clock += std::mem::take(&mut self.step).min(MAX_STEP);
        // Another world beside the screen: back to it fitted.
        let of = (scene.shape, beside.world);
        self.beside_of = Some(of);
        if self.explore.is_some_and(|e| e.of != of) {
            self.leave_explore("another world beside the screen");
        }
        self.main = None;
        self.fit = None;
        self.placed_screen = None;
        self.shown.clear();
        self.driver_rects.clear();
        self.interface.clear();
        self.insets_shown = 0;
        self.reachable = None;
        self.screen = None;
        self.art_zooms = scene.zooms.clone();
        self.focus = None;
        self.magnifier_shown = false;
        self.menu_side = None;
        self.forget_resume();
        if self.locked && self.explore.is_none() { self.release("a preview screen opened"); }
        let [sx, sy, sw, sh] = beside.screen;
        let scaled = |v: i32| (v as i64 * self.menu_scale.0.max(1) as i64).div_euclid(self.menu_scale.1.max(1) as i64) as i32;
        let scaled_screen = [scaled(sx), scaled(sy), scaled(sw).max(1), scaled(sh).max(1)];
        let (shift, world) = beside_geometry(scene::Beside { screen: scaled_screen, ..beside }, self.window);
        let mapping = crate::scaling::Affine { from: (sx, sy), to: (scaled_screen[0] + shift.0, scaled_screen[1] + shift.1), zoom: self.menu_scale };
        let (ww, wh) = (self.window.0 as i32, self.window.1 as i32);
        let region = match beside.edge {
            scene::Edge::Left => [0, 0, scaled_screen[2].min(ww), wh],
            scene::Edge::Right => [(ww - scaled_screen[2]).max(0), 0, scaled_screen[2].min(ww), wh],
            scene::Edge::Top => [0, 0, ww, scaled_screen[3].min(wh)],
            scene::Edge::Bottom => [0, (wh - scaled_screen[3]).max(0), ww, scaled_screen[3].min(wh)],
        };
        // Keep the driver's coordinate system for every menu camera and
        // apply its final placement once, including the requested UI scale.
        self.interface_shown = None;
        self.beside_mapping = Some(mapping);
        self.moved = vec![([0, 0, ww, wh], (mapping.to.0 - sx, mapping.to.1 - sy))];
        self.beside = Some(world);
        let mut frame = Frame { view_cameras: vec![None; scene.views.len()], beside_screen: Some(BesideScreen { source: beside.screen, mapping, region }), ..Default::default() };
        // The world: the driver's first world view, all of it (or the part
        // asked for) fitted, or explored up close.
        let main = scene.views.iter().position(|v| !v.view.interface_only);
        if let (Some(m), Some(b)) = (main, bounds.filter(|_| self.world_shown && world[2] > 0 && world[3] > 0)) {
            self.beside_view_downscale = base[m].downscale.max(1);
            let camera = match self.explore {
                Some(e) => self.explore_camera(&base[m], e, world, overhang, b),
                None => {
                    // (What the driver names of it, at the scale its view shows it.)
                    let shown = base[m].downscale.max(1) as i32;
                    let area = beside.world.map_or_else(|| whole_world(b, self.grid_step, shown as u32), |[x, y, w, h]| [x / shown, y / shown, w / shown, h / shown]);
                    fit_world(&base[m], area, world, self.grid_step)
                }
            };
            // (Text comes as drawn at 1:1 and is shown at the view's zoom: drawn
            // from outlines at the zoom, world text does not show in a view of
            // a world shown smaller.)
            let camera = Camera { zoomed_art: false, ..camera };
            frame.cameras.push(camera);
            frame.view_cameras[m] = Some(frame.cameras.len() as u32 - 1);
            self.shown.push((Hit::Beside, camera));
        }
        // The interface: only the part of the screen asked for (what is
        // around it, e.g. a background filling the window, would cover the
        // world), moved (screen sprites are in window pixels, moved by the
        // shift; the view's rectangle clips them).
        let [sx, sy, sw, sh] = beside.screen;
        let hint = scene.interface.capture_hint.and_then(|id| scene.views.iter().position(|v| v.id == id && v.view.interface_only));
        for (i, (_, c)) in scene.views.iter().zip(&base).enumerate().filter(|(i, (v, _))| v.view.interface_only && Some(*i) != hint && !scene.interface.is_popup_view(v.id)) {
            let (x0, y0) = (c.origin.0.max(sx), c.origin.1.max(sy));
            let (x1, y1) = ((c.origin.0 + c.viewport.0 as i32).min(sx + sw), (c.origin.1 + c.viewport.1 as i32).min(sy + sh));
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let camera = Camera { origin: (x0, y0), viewport: ((x1 - x0) as u32, (y1 - y0) as u32), ..*c };
            frame.cameras.push(mapping.camera(&camera));
            frame.view_cameras[i] = Some(frame.cameras.len() as u32 - 1);
        }
        // Modal canvases overlay the preview menu without changing/cropping
        // its fixed rectangle or the world alongside it.
        for id in scene.interface.popup_views() {
            if let Some(index) = scene.views.iter().position(|view| view.id == id) {
                frame.view_cameras[index] = Some(frame.cameras.len() as u32);
                frame.cameras.push(base[index]);
            }
        }
        // Exploring with the mouse captured: the world cursor, and the hint
        // how to let it go at the world's top-left (where the driver put it
        // for its window's top-left), unless snapping.
        if self.explore.is_some() && self.locked {
            frame.markers.push(Marker::Cursor(self.cursor));
            if let Some(i) = hint.filter(|_| !self.snapping) {
                let shift = (world[0], world[1]);
                let c = base[i];
                frame.cameras.push(Camera { origin: (c.origin.0 + shift.0, c.origin.1 + shift.1), screen_shift: shift, ..c });
                frame.view_cameras[i] = Some(frame.cameras.len() as u32 - 1);
            }
        }
        frame
    }

    /// The camera exploring the world beside the screen (`Explore`): the
    /// world as the game's zoomed-out view shows it (half size,
    /// [`EXPLORE_DOWNSCALE`], at the game view's zoom [`EXPLORE_ZOOM`] and
    /// the world scale), whatever scale the driver's view `base` shows it
    /// at, around the world cursor while the mouse is captured (else where
    /// it was), not past an edge of a world that does not wrap (`bounds`,
    /// at the scale of `base`). The world cursor keeps to the world at
    /// that size meanwhile.
    fn explore_camera(&mut self, base: &Camera, e: Explore, rect: [i32; 4], overhang: (u32, u32, u32, u32), bounds: WorldBounds) -> Camera {
        let bounds = explore_bounds(bounds, base.downscale);
        self.bounds = Some(bounds);
        let center = if self.locked { self.cursor } else { e.center };
        if let Some(e) = &mut self.explore {
            e.center = center;
        }
        let c = FreeView::new(center, EXPLORE_ZOOM, Follow::Pointer).camera(rect, overhang, Some(bounds), self.world_scale);
        Camera { x: c.x, y: c.y, zoom: c.zoom, viewport: c.viewport, origin: c.origin, cull: None, screen_shift: (0, 0), downscale: EXPLORE_DOWNSCALE, ..*base }
    }

    /// Explores the world beside the screen around window pixel `p` (on
    /// it), the mouse captured there.
    fn start_explore(&mut self, p: (i32, i32)) {
        let (Some(of), Some((_, c))) = (self.beside_of, self.shown.iter().find(|(h, _)| *h == Hit::Beside).copied()) else { return };
        // (The fitted view shows the world `c.downscale` times smaller: in
        // the pixels of the half-size world.)
        let q = world_at(&c, p);
        let to_half = |v: i32| (v as i64 * c.downscale.max(1) as i64 / EXPLORE_DOWNSCALE as i64) as i32;
        let center = (to_half(q.0), to_half(q.1));
        self.explore = Some(Explore { of, center });
        self.cursor = center;
        self.motion_rest = (0.0, 0.0);
        self.locked = true;
        self.grab = Some(true);
        self.against = None;
        log::info!("viewer: exploring the world beside the screen at {center:?}, as the game zoomed out (half size, zoom {:?}; Ctrl on its own lets the mouse go; a double-click or Esc goes back)", times(ZOOMS[EXPLORE_ZOOM], self.world_scale));
    }

    /// Back to the world fitted beside the screen (the mouse let go).
    fn leave_explore(&mut self, why: &str) {
        if self.explore.take().is_some() {
            if self.locked {
                self.release(why);
            }
            log::info!("viewer: the world beside the screen fitted again ({why})");
        }
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

    /// How far the driver's roaming controls move: to the same place in
    /// the active view's area ([`Layout::view_area`]: below a zoom strip,
    /// in A as in B) as the driver gives them in its window. They show in
    /// the active view only.
    fn roaming_shift(&self) -> Option<(i32, i32)> {
        let r = self.view_area(self.active_side());
        Some((r[0], r[1]))
    }

    /// The window rectangle of per-view interface `r` (driver pixels) moved
    /// by `d` into view area `area`, as `scaling::arrange` draws it: at the
    /// HUD's scale about its top-left, smaller where that does not fit the
    /// area, moved as little as needed to lie inside it.
    fn view_ui_rect(&self, r: [i32; 4], d: (i32, i32), area: [i32; 4]) -> [i32; 4] {
        let (n, m) = crate::scaling::fit_at_most(self.hud_scale, Some(r), (area[2].max(1) as u32, area[3].max(1) as u32));
        let s = |v: i32| (v as i64 * n.max(1) as i64 / m.max(1) as i64) as i32;
        let (w, h) = (s(r[2]), s(r[3]));
        let into = |at: i32, len: i32, lo: i32, side: i32| if len <= side { at.clamp(lo, lo + side - len) } else { at };
        [into(r[0] + d.0, w, area[0], area[2]), into(r[1] + d.1, h, area[1], area[3]), w, h]
    }

    /// A popup at `rect` opened. One the driver opened where a click went
    /// (a context menu: its corner at the click) goes where that click was
    /// made in the window (e.g. at the world cursor in B), inside the
    /// window. While it is open the mouse is let go, so that the pointer can
    /// reach it (captured again once it closes). One over a menu
    /// (`over_menu`) is a modal of that menu's: never moved to a click.
    fn popup_opened(&mut self, [x, y, w, h]: [i32; 4], anchor: Option<scene::PopupAnchor>, over_menu: bool) {
        self.popup_side = Some(self.active_side());
        // The driver anchors it at the world click just made: there, in
        // that view (its camera moved as little as it takes for the popup
        // to fit the window), whatever the screen place setting says.
        self.popup_anchor_seen = anchor;
        let fresh = self.world_press.filter(|p| self.clock - p.clock < 2.0);
        // (Opened from an entry of the popup at a click, the one in its
        // place or one just closed: that click's.)
        let chained = self.anchored.map(|(_, p)| p).or(self.chain_press.filter(|&(_, t)| self.clock - t < 2.0).map(|(p, _)| p));
        self.anchored = anchor.zip(fresh.or(chained));
        if let Some((a, press)) = self.anchored {
            self.popup_anchor = None;
            if let Some(((tx, ty), zoom, camera)) = self.anchored_place(a, press, (w, h)) {
                let size = (scaled_up(w, zoom), scaled_up(h, zoom));
                let (ww, wh) = (self.window.0 as i32, self.window.1 as i32);
                let pan = (fit_pan(tx, size.0, ww), fit_pan(ty, size.1, wh));
                if pan != (0, 0) {
                    let world = (world_steps(pan.0, camera.zoom), world_steps(pan.1, camera.zoom));
                    self.pan_for_popup(press.hit, world);
                    log::info!("viewer: the popup at the click needs {pan:?} more room: its view moved by {world:?}");
                }
            }
            log::info!("viewer: the popup opened at the world click {:?} ({:?})", press.world, press.hit);
            if self.locked {
                self.suspend_capture();
                self.release_here("a popup is open");
            }
            return;
        }
        self.popup_anchor = self.last_press.filter(|&(_, (qx, qy), t)| !over_menu && self.clock - t < 2.0 && (x - qx).abs() <= 40 && (y - qy).abs() <= 40).map(|((px, py), (qx, qy), _)| {
            let (ww, wh) = (self.window.0 as i32, self.window.1 as i32);
            ((x + px - qx).clamp(0, (ww - w).max(0)) - x, (y + py - qy).clamp(0, (wh - h).max(0)) - y)
        });
        if let Some(d) = self.popup_anchor {
            log::info!("viewer: the popup opened at the click: moved by {d:?}");
        }
        if self.locked {
            self.suspend_capture();
            self.release_here("a popup is open");
            // (On the popup, where it shows: below, once it is placed.)
            self.pointer_to_popup = self.popup_anchor.is_none();
        }
    }

    /// Where driver window pixel `p` shows in the window (as of this frame):
    /// in the screen fitted into a pane, in a part drawn elsewhere (by the
    /// viewer, or the layout), or where the driver has it.
    fn driver_to_window(&self, (x, y): (i32, i32)) -> (i32, i32) {
        if let Some(mapping) = self.beside_mapping.filter(|_| self.interface_shown.is_none()) { return mapping.apply((x, y)); }
        if let Some(m) = self.modal_shown().filter(|m| inside(m.driver, (x, y))) {
            let [x, y, _, _] = m.to_canvas([x, y, 0, 0]);
            return (x.round() as i32, y.round() as i32);
        }
        let fit = self.fit.or_else(|| self.placed_screen.filter(|f| inside(f.source, (x, y))));
        if let Some(Fit { source: [sx, sy, ..], origin: (ox, oy), zoom: (n, d), .. }) = fit {
            let (n, d) = (n as i32, d.max(1) as i32);
            return (ox + ((x - sx) * n).div_euclid(d), oy + ((y - sy) * n).div_euclid(d));
        }
        if self.interface_shown.is_some() {
            let [x, y, _, _] = self.driver_rect_to_window([x, y, 0, 0]);
            return (x.round() as i32, y.round() as i32);
        }
        match self.moved.iter().find(|(r, d)| inside([r[0] - d.0, r[1] - d.1, r[2], r[3]], (x, y))) {
            Some((_, (dx, dy))) => (x + dx, y + dy),
            None => (x, y),
        }
    }

    /// The popup closed (a mouse let go for it is captured again once no
    /// menu is open: `settle_capture`).
    fn popup_closed(&mut self) {
        self.popup_anchor = None;
        self.chain_press = self.anchored.map(|(_, p)| (p, self.clock));
        self.anchored = None;
        self.popup_anchor_seen = None;
        self.popup_pan = None;
        self.popup_side = None;
    }

    /// The opening view's current bounds, retained while the popup is open.
    /// Popups opened at a click have none: they may cross the divider.
    /// The screen place setting names one; floating, the opening view's
    /// with `menu_follows_pointer`.
    fn popup_region(&self) -> Option<[i32; 4]> {
        if self.anchored.is_some() || self.popup_anchor.is_some() { return None; }
        let split = self.split?;
        let (a, b) = self.panes();
        match self.shared_hit(split) {
            Some(Hit::Pane) => b,
            Some(_) => Some(a),
            None if self.menu_follows_pointer => Some(if self.popup_side == Some(Side::B) { b.unwrap_or(a) } else { a }),
            None => None,
        }
    }

    /// Where the anchored popup (`size`: its driver pixels) goes now: its
    /// top-left canvas pixel, its zoom, and the camera of the view the
    /// click was made in. The anchor's world pixel is taken in the wrap
    /// image nearest the click.
    fn anchored_place(&self, a: scene::PopupAnchor, press: WorldPress, (w, h): (i32, i32)) -> Option<((i32, i32), (u32, u32), Camera)> {
        let (_, camera) = *self.shown.iter().find(|(hit, _)| *hit == press.hit)?;
        let (n, d) = (camera.zoom.0.max(1) as i64, camera.zoom.1.max(1) as i64);
        let near = |world: i32, cam: i32, origin: i32, at: i32, period: i32| {
            let mut rel = (world - cam) as i64;
            if period > 0 {
                let want = (at - origin) as i64 * d / n;
                rel += (want - rel + period as i64 / 2).div_euclid(period as i64) * period as i64;
            }
            origin + (rel * n).div_euclid(d) as i32
        };
        let p = (near(a.world.0, camera.x, camera.origin.0, press.at.0, self.world_period.0), near(a.world.1, camera.y, camera.origin.1, press.at.1, self.world_period.1));
        let zoom = self.popup_zoom((w, h), a.world_scaled.then_some(camera.zoom));
        let offset = |v: i32| (v as i64 * zoom.0 as i64).div_euclid(zoom.1.max(1) as i64) as i32;
        Some(((p.0 + offset(a.offset.0), p.1 + offset(a.offset.1)), zoom, camera))
    }

    /// The anchored popup's zoom: the menus', smaller only where the popup
    /// would not fit the window at it. One showing a piece of the world
    /// (`world`: the zoom of the world in its view) is drawn at that zoom,
    /// as the game draws it beside its map at 1:1 (its picture of a tile
    /// as large as the tiles there, its frame and text with it), never
    /// smaller than the game's frame (the HUD) so that it stays readable.
    fn popup_zoom(&self, (w, h): (i32, i32), world: Option<(u32, u32)>) -> (u32, u32) {
        let z = match world {
            Some(c) => {
                let (c, f) = ((c.0.max(1), c.1.max(1)), (self.hud_scale.0.max(1), self.hud_scale.1.max(1)));
                let z = if c.0 as u64 * f.1 as u64 >= f.0 as u64 * c.1 as u64 { c } else { f };
                let g = gcd(z.0, z.1);
                (z.0 / g, z.1 / g)
            }
            None => (self.menu_scale.0.max(1), self.menu_scale.1.max(1)),
        };
        let (ww, wh) = (self.window.0.max(1) as u64, self.window.1.max(1) as u64);
        let (w, h) = (w.max(1) as u64, h.max(1) as u64);
        if w * z.0 as u64 <= ww * z.1 as u64 && h * z.0 as u64 <= wh * z.1 as u64 { return z; }
        let (n, d) = if ww * h <= wh * w { (ww, w) } else { (wh, h) };
        let g = gcd(n as u32, d as u32);
        (n as u32 / g, d as u32 / g)
    }

    /// Moves the view at `hit` by `by` world pixels for an anchored popup,
    /// as a pan by hand would (it stays there); one following the world
    /// cursor only while the popup is open (the cursor stays on the click).
    fn pan_for_popup(&mut self, hit: Hit, by: (i32, i32)) {
        let Some(center) = self.free_view(hit).map(|v| v.center) else { return };
        match self.follow_of(hit) {
            Some(Follow::Pointer) => self.popup_pan = Some((hit, by)),
            Some(_) => self.place_view(hit, (center.0 + by.0, center.1 + by.1)),
            None => {}
        }
    }

    /// Keep click-opened popups near their click and hotkey popups inside
    /// the view that held the mouse at opening. Interface scaling applies
    /// the same region after scaling so driver-window margins cannot move it.
    fn popup_shift(&self, [x, y, w, h]: [i32; 4], a: [i32; 4]) -> (i32, i32) {
        // (One anchored at a world click is placed once this frame's
        // cameras are: `Frame::popup_at`.)
        if self.anchored.is_some() { return (0, 0); }
        let anchored = self.popup_anchor.unwrap_or((0, 0));
        if let Some(r) = self.popup_region() {
            let keep = |at: i32, len: i32, origin: i32, span: i32| {
                if len <= span { at.clamp(origin, origin + span - len) } else { origin + (span - len) / 2 }
            };
            return (keep(x + anchored.0, w, r[0], r[2]) - x, keep(y + anchored.1, h, r[1], r[3]) - y);
        }
        if self.popup_anchor.is_some() { return anchored; }
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
        let Some(m) = self.magnifier.filter(|m| self.world_shown && matches!(m.place, ZoomPlace::Box(..) | ZoomPlace::Strip(_))) else { return (0, 0) };
        let boxed = self.magnifier_geometry(&m).0;
        let below = boxed[1] + boxed[3] + GAP;
        if overlap(boxed, [x, y, w, h]) && below > y && below + h <= self.window.1 as i32 { (0, below - y) } else { (0, 0) }
    }

    /// The parts of the driver's interface drawn elsewhere (as of the last
    /// frame): their window rectangle and how far they were moved.
    pub fn moved_parts(&self) -> &[([i32; 4], (i32, i32))] {
        if self.fit.is_some() { &[] } else { &self.moved }
    }

    /// The driver's main view as the main pane shows it: following the
    /// driver at zoom 1 it shows the driver's window pixels where they are
    /// (cropped to the pane; gliding back there with smooth movement), at
    /// other zooms it is zoomed about the driver's centre; otherwise it is
    /// placed like any arranged view. The driver's screen sprites (labels)
    /// stay on their world pixels.
    fn arranged_main(&self, camera: &Camera, rect: [i32; 4], overhang: (u32, u32, u32, u32)) -> Camera {
        let v = self.main_view;
        let driver = self.native_main_follow();
        let crop = (camera.x + rect[0] - camera.origin.0, camera.y + rect[1] - camera.origin.1);
        let zoom = v.zoom_by(self.world_scale);
        let unmoved = driver && zoom == (1, 1) && v.center == (crop.0 + rect[2] / 2, crop.1 + rect[3] / 2);
        let (x, y, cull) = if unmoved {
            (crop.0, crop.1, camera.cull)
        } else {
            let placed = if driver { v } else { FreeView { follow: Follow::Free, ..v } };
            let c = placed.camera(rect, overhang, if driver { None } else { self.bounds }, self.world_scale);
            // Zoomed in on what the driver shows, its cell range still holds.
            let centred = v.center == (camera.x + camera.viewport.0 as i32 / 2, camera.y + camera.viewport.1 as i32 / 2);
            let cull = if driver && zoom.0 > zoom.1 && centred { camera.cull } else { None };
            (c.x, c.y, cull)
        };
        let shift = (camera.x - x + camera.origin.0 - rect[0], camera.y - y + camera.origin.1 - rect[1]);
        Camera {
            x,
            y,
            zoom,
            viewport: (rect[2].max(1) as u32, rect[3].max(1) as u32),
            origin: (rect[0], rect[1]),
            screen_shift: shift,
            cull,
            zoomed_art: self.zoomed_art(zoom),
            ..*camera
        }
    }

    /// With the driver's screen and its world side by side: the world, or
    /// else the driver's screen (moved).
    fn beside_hit(&self, p: (i32, i32)) -> Option<Hit> {
        let world = self.beside?;
        Some(if inside(world, p) { Hit::Beside } else { Hit::Moved(0) })
    }

    fn hit(&self, p: (i32, i32)) -> Hit {
        if let Some(b) = self.beside_hit(p) {
            return b;
        }
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
        // (A modal popup over the screen fitted into a pane: on top.)
        if self.modal_shown().is_some_and(|_| self.interface_shown.iter().flatten().any(|s| s.is_modal() && inside(s.rect, p))) {
            return Some(Hit::Interface);
        }
        if self.placed_screen.is_some_and(|f| {
            let size = ((f.source[2] as u32 * f.zoom.0).div_ceil(f.zoom.1), (f.source[3] as u32 * f.zoom.0).div_ceil(f.zoom.1));
            inside([f.origin.0, f.origin.1, size.0 as i32, size.1 as i32], p)
        }) { return Some(Hit::Screen); }
        if self.fit.is_some() {
            return None;
        }
        // (Shown elsewhere by the viewer, the moved parts too.)
        if let Some(shown) = &self.interface_shown {
            return shown.iter().any(|s| inside(s.rect, p)).then_some(Hit::Interface);
        }
        if let Some(k) = self.moved.iter().rposition(|&(r, _)| inside(r, p)) {
            return Some(Hit::Moved(k));
        }
        self.interface.iter().chain(&self.driver_rects).any(|&r| inside(r, p)).then_some(Hit::Interface)
    }

    /// [`Layout::hit`] as if there were no magnifier.
    fn hit_below_magnifier(&self, p: (i32, i32)) -> Hit {
        if let Some(b) = self.beside_hit(p) {
            return b;
        }
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
        (matches!(hit, Hit::Main | Hit::Pane | Hit::Inset(_) | Hit::Beside) && self.follow_of(hit) == Some(Follow::Pointer)).then_some(hit)
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
        // A semantic focus stream means the native game camera is parked.
        // Every actual map point uses the virtual tile path, even where its
        // coordinates happen to fit the old native client rectangle. UI
        // points bypass this function and retain their client coordinates.
        if self.camera_focus_serial.is_some() && self.explore.is_none() {
            return Err(Some(w));
        }
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

    /// Per axis, whether the view following the world cursor is held at a
    /// nonwrapping world edge rather than centring that cursor.
    pub fn world_cursor_axes(&self) -> Option<(bool, bool)> {
        let (_, c) = self.shown.iter().find(|(h, _)| self.follow_of(*h) == Some(Follow::Pointer))?;
        let (n, d) = (c.zoom.0.max(1), c.zoom.1.max(1));
        let span = ((c.viewport.0 * d).div_ceil(n) as i32, (c.viewport.1 * d).div_ceil(n) as i32);
        Some(self.bounds.map_or((false, false), |b| (
            b.clamp_center(0, self.cursor.0, span.0) != self.cursor.0,
            b.clamp_center(1, self.cursor.1, span.1) != self.cursor.1,
        )))
    }

    /// While the mouse is captured: the world cursor goes under window pixel
    /// `p` (kept inside the view following it), as the system pointer shows
    /// it while that view is held at the world's edge; what the driver gets.
    pub fn pointer_to(&mut self, p: (i32, i32)) -> Vec<Input> {
        let Some((_, c)) = self.shown.iter().find(|(h, _)| self.follow_of(*h) == Some(Follow::Pointer)).copied() else { return vec![] };
        if !self.locked {
            return vec![];
        }
        let bounds = self.pointer_bounds().unwrap_or([c.origin.0, c.origin.1, c.viewport.0 as i32, c.viewport.1 as i32]);
        let x = p.0.clamp(bounds[0], bounds[0] + bounds[2] - 1);
        let y = p.1.clamp(bounds[1], bounds[1] + bounds[3] - 1);
        let from = self.pointer;
        self.raw_pointer = p;
        let (x, y) = self.constrain_zoom(from, (x, y));
        self.pointer = (x, y);
        // Wayland holds the physical pointer with the same walls. Where
        // the window can move it, this is the ordinary wall-warp fallback.
        if self.pointer_at_edge && self.pointer != p && self.walls == Walls::Warp {
            self.warp = Some(self.pointer);
        }
        let world = world_at(&c, (x, y));
        let absolute = if self.pointer_at_edge { self.world_cursor_axes().unwrap_or((true, true)) } else { (true, true) };
        if absolute.0 { self.cursor.0 = world.0; self.motion_rest.0 = 0.0; }
        if absolute.1 { self.cursor.1 = world.1; self.motion_rest.1 = 0.0; }
        if let Some(b) = self.bounds {
            self.cursor = b.clamp_point(self.cursor);
        }
        if let Some(mut out) = self.unit_order_motion(self.pointer) {
            let mut queued = self.take_unit_order_events();
            queued.append(&mut out);
            self.remember_world_inputs(&queued);
            return queued;
        }
        let out = match self.route_world(self.cursor) {
            Ok((x, y)) => vec![Input::MouseMove { x, y }],
            Err(Some((x, y))) if self.explore.is_none() => vec![Input::WorldMove { x, y }],
            Err(_) => vec![],
        };
        self.remember_world_inputs(&out);
        out
    }

    /// At an edge, centred axes continue to move the camera through raw
    /// relative motion. The unlocked axes follow the system pointer only.
    pub fn edge_motion(&mut self, delta: (f64, f64)) -> Vec<Input> {
        let Some(held) = self.world_cursor_axes().filter(|_| self.locked && self.pointer_at_edge && !self.snapping) else { return vec![] };
        let delta = (if held.0 { 0.0 } else { delta.0 }, if held.1 { 0.0 } else { delta.1 });
        if delta == (0.0, 0.0) { return vec![]; }
        self.motion(delta)
    }

    /// The window size of the last frame.
    pub fn window(&self) -> (u32, u32) {
        self.window
    }

    /// How the driver's screen (its window pixels, those of its interface)
    /// lies in the window, as of the last frame: driver pixel `s` is at
    /// window pixel `origin + s · zoom.0 / zoom.1` (1:1 at the window's
    /// corner unless it is fitted into a pane or moved beside its world).
    pub fn screen_mapping(&self) -> ((i32, i32), (u32, u32)) {
        if let Some(mapping) = self.beside_mapping {
            return (mapping.apply((0, 0)), mapping.zoom);
        }
        self.fit.map_or(((0, 0), (1, 1)), |f| {
            let (n, d) = (f.zoom.0 as i32, f.zoom.1.max(1) as i32);
            ((f.origin.0 - f.source[0] * n / d, f.origin.1 - f.source[1] * n / d), f.zoom)
        })
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

    /// Button snapping (`crate::snap`) holds the pointer (`on`) or gives it
    /// back: while it holds it, it points at the interface (the corner's
    /// buttons show to be snapped to), no divider or window edge acts on the
    /// pointer and the system pointer is not moved (a warp asked for waits);
    /// what the driver gets (the pointer at no edge any more).
    pub fn set_snapping(&mut self, on: bool) -> Vec<Input> {
        self.snapping = on;
        self.against = None;
        if on { self.edge_change((0, 0)) } else { vec![] }
    }

    pub fn snapping(&self) -> bool {
        self.snapping
    }

    /// The view a click captures the mouse in (as of the last frame): its
    /// window rectangle and the point to click, its middle (where a capture
    /// puts the world cursor; else where the world cursor is in it, should
    /// the driver's interface cover the middle), if a click there does
    /// capture it (not under the driver's interface).
    pub fn capture_view(&self) -> Option<([i32; 4], (i32, i32))> {
        self.shown.iter().filter(|(h, _)| matches!(h, Hit::Main | Hit::Pane | Hit::Inset(_))).find_map(|(h, c)| {
            if self.follow_of(*h) != Some(Follow::Pointer) {
                return None;
            }
            let rect = [c.origin.0, c.origin.1, c.viewport.0 as i32, c.viewport.1 as i32];
            let middle = (rect[0] + rect[2] / 2, rect[1] + rect[3] / 2);
            let cursor = self.cursor_on_window().filter(|&p| inside(rect, p));
            [Some(middle), cursor].into_iter().flatten().find(|&at| self.capture_hit(at) == Some(*h)).map(|at| (rect, at))
        })
    }

    /// A command of the driver's waits for a place of the world (button
    /// snapping ends for it, `crate::snap`): the mouse is captured again in
    /// the view following the world cursor, which stays where it is, the
    /// view too (the place looked at is the one the command is likely
    /// for). The system pointer goes back to the view's middle, a move that
    /// is not the mouse's: the world cursor does not follow it. What goes
    /// to the driver: the world cursor's place as the pointer's, so that
    /// the tile under it is the command's target at once (its path and
    /// turns shown; nothing is clicked). `None`: no such view.
    pub fn capture_for_target(&mut self) -> Option<Vec<Input>> {
        let (hit, camera) = self.shown.iter().find(|(h, _)| matches!(h, Hit::Main | Hit::Pane | Hit::Inset(_)) && self.follow_of(*h) == Some(Follow::Pointer)).copied()?;
        if !self.locked {
            let middle = (camera.origin.0 + camera.viewport.0 as i32 / 2, camera.origin.1 + camera.viewport.1 as i32 / 2);
            self.pointer = middle;
            self.raw_pointer = middle;
            self.warp = Some(middle);
            self.side = self.side_of(middle);
            // Where it may not be captured (Drag, a menu): the free pointer
            // there, the command's target the tile under it.
            if let Some(why) = self.capture_refused(hit) {
                log::info!("viewer: a command waits for a place of the world: the pointer goes to {hit:?} ({why})");
                let out = match self.route_world(self.world_under(middle).unwrap_or(self.cursor)) {
                    Ok((x, y)) => vec![Input::MouseMove { x, y }],
                    Err(Some((x, y))) if self.explore.is_none() => vec![Input::WorldMove { x, y }],
                    Err(_) => vec![],
                };
                self.remember_world_inputs(&out);
                return Some(out);
            }
            // (`capture` without moving the world cursor to the middle.)
            self.locked = true;
            self.grab = Some(true);
            self.against = None;
            log::info!("viewer: a command waits for a place of the world: mouse captured again in {hit:?}, the world cursor stays at {:?}", self.cursor);
        }
        let out = match self.route_world(self.cursor) {
            Ok((x, y)) => vec![Input::MouseMove { x, y }],
            Err(Some((x, y))) if self.explore.is_none() => vec![Input::WorldMove { x, y }],
            Err(_) => vec![],
        };
        self.remember_world_inputs(&out);
        Some(out)
    }

    /// The player asked the driver to centre on its point of interest (the
    /// game's C: centre on the active unit), `focus` (a world pixel): the
    /// view following the world cursor centres on it too (the world cursor
    /// goes there), and the views following the driver or the focus that
    /// were moved by hand follow it again. Not while a popup or a named
    /// screen takes the keys. Whether the world cursor moved.
    pub fn centre_on_focus(&mut self, focus: Option<(i32, i32)>) -> bool {
        let Some(f) = focus.filter(|_| !self.popup_shown && self.screen.is_none()) else { return false };
        self.resume_game_focus();
        for v in self.views_mut() {
            v.hand_focus = None;
        }
        if !self.follows_pointer() {
            return false;
        }
        self.cursor = self.bounds.map_or(f, |b| b.clamp_point(f));
        log::info!("viewer: centred on the driver's focus: the world cursor at {:?}", self.cursor);
        true
    }

    /// Lets go of a captured mouse, the system pointer showing at window
    /// pixel `p` (button snapping ended there, on the interface).
    pub fn let_go_at(&mut self, p: (i32, i32)) {
        self.release("button snapping ended on the interface");
        self.place_pointer(p);
    }

    /// Captures the mouse at window pixel `p` as a click there would (the
    /// snapping pointer's click on an attractor, [`Layout::attractors`]);
    /// whether it did: not where no view following the world cursor is,
    /// nor where Drag or a menu refuses it (a popup opened at a world click
    /// is dismissed, the mouse captured once it is gone; `capture`).
    pub fn capture_at(&mut self, p: (i32, i32)) -> bool {
        let Some(hit) = self.capture_hit(p).filter(|_| !self.locked) else { return false };
        self.pointer = p;
        self.raw_pointer = p;
        self.player_capture(hit, p)
    }

    /// The system has finished placing the free pointer at `p`. A hint may
    /// produce no motion event, or actual motion may have arrived while it
    /// was pending. Confirm its side before routing so this never asks for
    /// another warp across the divider.
    pub fn system_pointer_placed(&mut self, p: (i32, i32)) -> Vec<Input> {
        self.pointer = p;
        self.raw_pointer = p;
        self.side = self.side_of(p);
        self.route(Input::MouseMove { x: p.0, y: p.1 })
    }

    /// The pointer was put at window pixel `p` by other means than the
    /// mouse (e.g. the snapping pointer, `crate::snap`): it is there now
    /// (the system pointer moved there while the mouse is free), and while
    /// the mouse is captured, over a view following the world cursor, so is
    /// the world cursor.
    pub fn place_pointer(&mut self, p: (i32, i32)) {
        self.pointer = p;
        if !self.locked {
            // (The system pointer shows there.)
            self.raw_pointer = p;
            self.warp = Some(p);
            self.side = self.side_of(p);
        }
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
            Hit::Screen => {
                let Fit { source: [sx, sy, sw, sh], origin: (ox, oy), zoom: (n, d), .. } = self.placed_screen.ok_or(None)?;
                let (n, d) = (n as i32, d as i32);
                let q = (sx + ((p.0 - ox) * d).div_euclid(n), sy + ((p.1 - oy) * d).div_euclid(n));
                // (Never onto a modal popup over it shown elsewhere.)
                let q = self.modal_shown().map_or(q, |m| crate::scaling::ShownPiece::beside_modal(q, m.driver));
                inside([sx, sy, sw, sh], q).then_some(q).ok_or(None)
            }
            Hit::Interface => Ok(self.to_driver(p)),
            Hit::Divider => Err(None),
            // (Not the driver's view: nothing reaches it.)
            Hit::Beside => Err(None),
            // Drawn elsewhere: where the driver put it.
            Hit::Moved(k) => {
                if let Some(mapping) = self.beside_mapping {
                    return Ok(if self.interface_shown.is_some() { self.to_driver(p) } else { mapping.invert(p) });
                }
                let (_, (dx, dy)) = *self.moved.get(k).ok_or(None)?;
                Ok((p.0 - dx, p.1 - dy))
            }
            // The driver's whole screen, scaled: its window pixel.
            hit if self.fit.is_some_and(|f| f.hit == hit) => {
                let Fit { source: [sx, sy, sw, sh], origin: (ox, oy), zoom: (n, d), .. } = self.fit.ok_or(None)?;
                let (n, d) = (n as i32, d as i32);
                let q = (sx + ((p.0 - ox) * d).div_euclid(n), sy + ((p.1 - oy) * d).div_euclid(n));
                inside([sx, sy, sw, sh], q).then_some(q).ok_or(None)
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

    /// Installs the final interface hit map and focuses a newly opened
    /// screen or popup through it, after pixel and menu scaling.
    pub fn set_interface_shown(&mut self, shown: Option<Vec<crate::scaling::ShownPiece>>) {
        self.interface_shown = shown;
        if let Some(p) = self.focus_after_scale.take() {
            let p = self.driver_to_window(p);
            (self.pointer, self.raw_pointer, self.warp) = (p, p, Some(p));
            self.side = self.side_of(p);
        }
    }

    /// The driver's screen (the window but where told otherwise).
    fn driver_screen(&self) -> (u32, u32) {
        self.driver_screen.unwrap_or(self.window)
    }

    /// Where driver rectangle `r` (its window's pixels, e.g. a button) shows
    /// in the window: in the driver's screen fitted into a pane, else where
    /// the interface piece holding it is shown (`interface_shown`), else
    /// where the driver put it.
    pub fn driver_rect_to_window(&self, r: [i32; 4]) -> [f64; 4] {
        if let Some(mapping) = self.beside_mapping.filter(|_| self.interface_shown.is_none()) {
            let [x, y, w, h] = mapping.rect(r);
            return [x as f64, y as f64, w as f64, h as f64];
        }
        let middle = (r[0] + r[2] / 2, r[1] + r[3] / 2);
        if let Some(m) = self.modal_shown().filter(|m| inside(m.driver, middle)) {
            return m.to_canvas(r);
        }
        let fit = self.fit.or_else(|| self.placed_screen.filter(|f| inside(f.source, middle)));
        if let Some(Fit { source: [sx, sy, ..], origin: (ox, oy), zoom: (n, d), .. }) = fit {
            let s = n.max(1) as f64 / d.max(1) as f64;
            return [ox as f64 + (r[0] - sx) as f64 * s, oy as f64 + (r[1] - sy) as f64 * s, r[2] as f64 * s, r[3] as f64 * s];
        }
        let shown = self.interface_shown.as_deref().unwrap_or_default();
        match shown.iter().rev().find(|s| inside(s.driver, middle)) {
            Some(s) => s.to_canvas(r),
            None => [r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64],
        }
    }

    /// The visible clip for a driver control, chosen from the same source
    /// piece as `driver_rect_to_window`. Its full affine remains unchanged
    /// when the resulting hit area is cut to this rectangle.
    pub fn driver_target_clip(&self, r: [i32; 4]) -> Option<[f64; 4]> {
        let window = [0.0, 0.0, self.window.0 as f64, self.window.1 as f64];
        let clipped = |[x,y,w,h]: [f64;4]| {
            let (l,t) = (x.max(0.0), y.max(0.0));
            let (right,bottom) = ((x+w).min(window[2]), (y+h).min(window[3]));
            (right > l && bottom > t).then_some([l,t,right-l,bottom-t])
        };
        let middle = (r[0] + r[2]/2, r[1] + r[3]/2);
        if let Some(m) = self.modal_shown().filter(|m| inside(m.driver, middle)) {
            let [x, y, w, h] = m.clip;
            return clipped([x as f64, y as f64, w as f64, h as f64]);
        }
        if let Some(fit) = self.fit.or_else(|| self.placed_screen.filter(|f| inside(f.source,middle))) {
            let (a,b) = self.panes();
            let pane = if fit.hit == Hit::Main { a } else { b.unwrap_or(a) };
            let source = self.driver_rect_to_window(fit.source);
            let (l,t) = (source[0].max(pane[0] as f64),source[1].max(pane[1] as f64));
            let (right,bottom) = ((source[0]+source[2]).min((pane[0]+pane[2]) as f64),
                (source[1]+source[3]).min((pane[1]+pane[3]) as f64));
            return clipped([l,t,(right-l).max(0.0),(bottom-t).max(0.0)]);
        }
        if let Some(shown) = self.interface_shown.as_deref() {
            return shown.iter().rev().find(|piece| inside(piece.driver,middle))
                .and_then(|piece| { let [x,y,w,h] = piece.clip; clipped([x as f64,y as f64,w as f64,h as f64]) });
        }
        clipped(window)
    }

    /// The driver's pixel for window pixel `p` on its interface: where the
    /// piece shown there is in the driver's window, if the viewer shows it
    /// elsewhere ([`Layout::interface_shown`]); never onto a modal popup
    /// shown elsewhere from under it.
    fn to_driver(&self, p: (i32, i32)) -> (i32, i32) {
        let shown = self.interface_shown.as_deref().unwrap_or_default();
        let modal = self.modal_shown().and_then(|_| shown.iter().find(|s| s.is_modal() && inside(s.rect, p)));
        modal.or_else(|| shown.iter().rev().find(|s| inside(s.rect, p))).map_or(p, |s| s.pointer_to_driver(p))
    }

    /// The modal popup over the screen fitted into a pane, as shown (its
    /// body): it is over that screen, and placed elsewhere than the game
    /// has it.
    fn modal_shown(&self) -> Option<&crate::scaling::ShownPiece> {
        self.placed_screen?;
        self.interface_shown.as_deref()?.iter().find(|s| s.is_modal())
    }

    /// Sets the extra sensitivity of captured relative motion. Other pointer
    /// modes, including the visible pointer at a world edge, keep their speed.
    pub fn set_capture_sensitivity(&mut self, sensitivity: f32) {
        let range = viewer_settings::controls::CAPTURE_SENSITIVITY_RANGE;
        self.capture_sensitivity = if sensitivity.is_finite() { sensitivity.clamp(range.min, range.max) as f64 } else { 1.0 };
    }

    /// Relative mouse motion while the mouse is captured: moves the world
    /// cursor (in the pixels of the view following it).
    pub fn motion(&mut self, (dx, dy): (f64, f64)) -> Vec<Input> {
        if !self.locked || self.click_popup_open() {
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
        let scale = self.capture_sensitivity * zoom.1 as f64 / zoom.0 as f64;
        let (rx, ry) = (self.motion_rest.0 + dx * scale, self.motion_rest.1 + dy * scale);
        let (mx, my) = (rx.trunc(), ry.trunc());
        self.motion_rest = (rx - mx, ry - my);
        self.cursor = (self.cursor.0 + mx as i32, self.cursor.1 + my as i32);
        if let Some(b) = self.bounds {
            self.cursor = b.clamp_point(self.cursor);
        }
        if let Some(mut out) = self.unit_order_motion(self.pointer) {
            let mut queued = self.take_unit_order_events();
            queued.append(&mut out);
            self.remember_world_inputs(&queued);
            return queued;
        }
        let out = match self.route_world(self.cursor) {
            Ok((x, y)) => vec![Input::MouseMove { x, y }],
            Err(Some((x, y))) if self.explore.is_none() => vec![Input::WorldMove { x, y }],
            Err(_) => vec![],
        };
        self.remember_world_inputs(&out);
        out
    }

    fn follow_of(&self, hit: Hit) -> Option<Follow> {
        match hit {
            Hit::Beside if self.explore.is_some() => Some(Follow::Pointer),
            Hit::Main => Some(self.main_view.follow),
            Hit::Pane => self.split.map(|s| s.view.follow),
            Hit::Inset(k) => self.insets.get(k).map(|i| i.view.follow),
            _ => None,
        }
    }

    /// Takes a window input; returns what goes to the driver.
    pub fn route(&mut self, input: Input) -> Vec<Input> {
        if !matches!(input, Input::Wheel { .. } | Input::Scroll { .. }) {
            self.scroll_panned = false;
        }
        let mut out = self.take_unit_order_events();
        out.extend(self.detached.take_inputs());
        if self.unit_order_before(&input) {
            out.extend(self.take_unit_order_events());
            self.remember_world_inputs(&out);
            return out;
        }
        let routed = self.route_regular(input);
        out.extend(self.take_unit_order_events());
        out.extend(routed);
        self.remember_world_inputs(&out);
        out
    }

    fn route_regular(&mut self, input: Input) -> Vec<Input> {
        if !self.locked && self.menu_pan.route(&input) {
            if let Input::MouseMove { x, y } | Input::MouseButton { x, y, .. } = input {
                self.raw_pointer = (x, y);
                self.pointer = (x, y);
            }
            return vec![];
        }
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
            Input::Key { down: true, .. } | Input::MouseButton { down: true, .. } | Input::Wheel { .. } | Input::Scroll { .. } => self.ctrl_alone = false,
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
            // Exploring the world beside the screen: Esc goes back to it
            // fitted; with the mouse captured no key reaches the driver.
            // (A key whose press it kept: its release too.)
            Input::Key { vk, down, .. } if (down && self.explore.is_some() && (vk == VK_ESCAPE || self.locked)) || (!down && self.kept_keys.contains(&vk)) => {
                self.kept_keys.retain(|&k| k != vk);
                if down {
                    self.kept_keys.push(vk);
                    if vk == VK_ESCAPE {
                        self.leave_explore("Esc");
                    }
                }
                vec![]
            }
            // Esc closes the kept tile box of the view the pointer is in.
            Input::Key { vk: VK_ESCAPE, down: true, .. } if self.detached_escape() => {
                self.kept_keys.push(VK_ESCAPE);
                self.detached.take_inputs()
            }
            Input::Key { vk, down, text } => vec![self.key(vk, down, text)],
            Input::MouseMove { x, y } => {
                if self.locked {
                    return vec![];
                }
                // (A view grabbed follows it.)
                if let Some(out) = self.pan_motion((x, y)) {
                    return out;
                }
                let from = self.pointer;
                self.raw_pointer = (x, y);
                // The divider may hold it back (the system pointer is moved
                // back too).
                let divided = self.constrain(from, (x, y));
                let (x, y) = self.constrain_zoom(from, divided);
                if (x, y) != divided {
                    self.side = self.side_of((x, y));
                    self.against = None;
                }
                if (x, y) != self.raw_pointer {
                    log::debug!("viewer: holds the pointer at {:?} (it went to {:?})", (x, y), self.raw_pointer);
                    // (Confined, the system pointer is where the system
                    // holds it: at most a pixel into the wall.)
                    if self.walls == Walls::Warp {
                        self.warp = Some((x, y));
                    }
                }
                self.pointer = (x, y);
                if let Some(out) = self.unit_order_motion((x, y)) { return out; }
                if let Some(drag) = self.drag {
                    self.dragged(drag, (x, y));
                    return self.edge_change((0, 0));
                }
                let mut out = match self.route_point((x, y)) {
                    Ok((x, y)) => vec![Input::MouseMove { x, y }],
                    Err(Some((x, y))) => vec![Input::WorldMove { x, y }],
                    Err(None) => vec![],
                };
                // (After the move: a driver takes the edge for where the
                // pointer is.)
                out.extend(self.edge_change(self.driver_edge((x, y))));
                out
            }
            Input::MouseButton { button, down, x, y } => {
                let p = self.held_back((x, y));
                // (On the minimap: B goes there, `pan`.)
                if self.minimap_button(button, down, p) {
                    return vec![];
                }
                self.button(button, down, p)
            }
            Input::Wheel { delta, x, y } => {
                let (x, y) = self.held_back((x, y));
                // (The map stays put under a popup opened at a world click.)
                if self.click_popup_open() && (self.ctrl || self.locked || matches!(self.hit((x, y)), Hit::Main | Hit::Pane | Hit::Inset(_) | Hit::Magnifier | Hit::Beside)) {
                    return vec![];
                }
                if self.ctrl && self.world_shown {
                    self.zoom_at((x, y), delta.signum());
                    return vec![];
                }
                // (Its scroll panned the map already.)
                if self.scroll_panned { return vec![]; }
                if let Some(out) = self.map_wheel(delta, (x, y)) { return out; }
                match self.route_point((x, y)) {
                    Ok((x, y)) => vec![Input::Wheel { delta, x, y }],
                    Err(_) => vec![],
                }
            }
            // A scroll's motion: the map's (both axes, unrounded), else
            // nothing; its `Wheel` notches follow for the interface.
            Input::Scroll { dx, dy, pixels, x, y } => {
                let (x, y) = self.held_back((x, y));
                self.scroll_panned = false;
                if self.ctrl || (self.click_popup_open() && (self.locked || matches!(self.hit((x, y)), Hit::Main | Hit::Pane | Hit::Inset(_) | Hit::Magnifier | Hit::Beside))) {
                    return vec![];
                }
                let d = (dx as f64 / 1000.0, dy as f64 / 1000.0);
                match self.map_scroll(d, pixels, (x, y)) {
                    Some(out) => {
                        self.scroll_panned = true;
                        out
                    }
                    None => vec![],
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

    /// Whether a popup the driver opened at a world click (a context menu,
    /// a tile's information box: `scene::Interface::popup_anchor`) is open.
    /// While it is the map stays where it is (no edge scrolling, dragging,
    /// wheel or captured motion), and a press outside its body, or
    /// capturing the mouse, dismisses it (`Input::DismissPopup`) and does
    /// nothing else.
    pub fn click_popup_open(&self) -> bool {
        self.popup_shown && (self.popup_anchor_seen.is_some() || self.anchored.is_some() || !self.anchored_popup_layers.is_empty())
    }

    /// A press at `p` while a popup anchored at a world click is open: if it
    /// is outside its body (or the mouse is captured), it dismisses
    /// the popup; made where a left click into a view following the world
    /// cursor captures the mouse, it is captured there once the popup is
    /// gone (`capture`); `None`: a press like any other (on the popup).
    fn dismissing_press(&mut self, button: Button, p: (i32, i32)) -> Option<Vec<Input>> {
        if !self.locked {
            let on_popup = if let Some(shown) = self.interface_shown.as_deref() {
                shown.iter().any(|s| (s.is_modal() || Some(s.driver) == self.popup_body) && inside(s.rect, p))
            } else if let (Some((anchor, press)), Some([_, _, w, h])) = (self.anchored, self.popup_body) {
                self.anchored_place(anchor, press, (w, h)).is_some_and(|((x, y), zoom, _)| {
                    inside([x, y, scaled_up(w, zoom), scaled_up(h, zoom)], p)
                })
            } else {
                self.popup_body.is_some_and(|r| inside(r, self.to_driver(p)))
            };
            if on_popup { return None; }
        }
        self.dismissed[button_index(button)] = true;
        if button == Button::Left && !self.locked {
            // (Not under the popup: once it is gone, `capture`.)
            if let Some(into) = self.capture_hit(p) {
                self.capture_after_dismissal(into);
            }
        }
        log::info!("viewer: a press beside the popup opened at a world click dismisses it");
        Some(vec![Input::DismissPopup])
    }

    fn button(&mut self, button: Button, down: bool, p: (i32, i32)) -> Vec<Input> {
        if !down && std::mem::take(&mut self.dismissed[button_index(button)]) {
            return vec![];
        }
        if let Some(out) = self.detached_button(button, down, p) {
            return out;
        }
        if down && self.click_popup_open() {
            if let Some(out) = self.dismissing_press(button, p) { return out; }
        }
        if let Some(out) = self.unit_order_button(button, down, p) { return out; }
        // Grabbing a view, or a press that may become a drag (`pan`).
        if let Some(out) = self.pan_button(button, down, p) {
            return out;
        }
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
            let q = match self.route_point(p) {
                Ok(q) => q,
                // The driver may enable virtual map input while a button
                // is held. Release through the same native map handler at
                // the current world point rather than an old client clamp.
                Err(Some((x, y))) => return vec![Input::WorldButton { button, down, x, y }],
                Err(None) => self.clamp_to_main(p),
            };
            return vec![Input::MouseButton { button, down, x: q.0, y: q.1 }];
        }
        // The world beside the driver's screen: its own gestures.
        if let Some(out) = self.beside_button(button, p) {
            return out;
        }
        let hit = if self.locked { Hit::Interface } else { self.hit(p) };
        // Viewer gestures.
        match (button, hit) {
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
            // the mouse there (where it may: no menu over it, no Drag; else
            // it is a click on the world there, `capture`).
            (Button::Left, _) if !self.locked && self.capture_hit(p).is_some_and(|h| self.capture_refused(h).is_none()) => {
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
                // (Where it was made and in which view: the world cursor's
                // while the mouse is captured.)
                let hit = if self.locked { self.shown.iter().find(|(h, _)| self.follow_of(*h) == Some(Follow::Pointer)).map(|(h, _)| *h) } else { Some(self.hit(p)) };
                let at = if self.locked { self.cursor_on_window().unwrap_or(p) } else { p };
                self.world_press = hit.map(|hit| WorldPress { at, world: w, hit, clock: self.clock });
                if let Some(press) = self.world_press {
                    self.detached_press_sent(press);
                }
                let press = Input::WorldButton { button, down, x: w.0, y: w.1 };
                if button == Button::Left { vec![Input::Look { x: w.0, y: w.1 }, press] } else { vec![press] }
            }
            Err(None) => vec![],
        }
    }

    /// A press on the world beside the driver's screen (or anywhere while it
    /// is explored with the mouse captured): a double-click explores it, or
    /// goes back to it fitted; exploring with the mouse free, a click
    /// captures it again. None of it reaches the driver (`Some`: what does,
    /// nothing); `None`: not such a press.
    fn beside_button(&mut self, button: Button, p: (i32, i32)) -> Option<Vec<Input>> {
        self.beside?;
        let on_world = if self.locked { self.explore.is_some() } else { self.hit(p) == Hit::Beside };
        if !on_world {
            self.beside_press = None;
            return None;
        }
        if button != Button::Left {
            return Some(vec![]);
        }
        // (Where it is made: the world cursor while the mouse is captured.)
        let at = if self.locked { self.cursor_on_window().unwrap_or(p) } else { p };
        let near = |(t, q): (f64, (i32, i32))| {
            self.clock - t <= DOUBLE_CLICK_SECONDS && (q.0 - at.0).abs() <= DOUBLE_CLICK_PIXELS && (q.1 - at.1).abs() <= DOUBLE_CLICK_PIXELS
        };
        if self.beside_press.take().is_some_and(near) {
            if self.explore.is_some() {
                self.leave_explore("a double-click");
            } else {
                self.start_explore(p);
            }
        } else {
            self.beside_press = Some((self.clock, at));
            if self.explore.is_some() && !self.locked {
                self.capture(Hit::Beside, p);
            }
        }
        Some(vec![])
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
                    // (A zoom box or strip: within the settings' choices.)
                    let (lo, hi) = if m.place == ZoomPlace::Beside { (ONE, ZOOMS.len() - 1) } else { Self::ZOOM_BOX_ZOOMS.into_inner() };
                    m.zoom = next(m.zoom).clamp(lo, hi);
                }
            }
            Some(Hit::Beside) => {}
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
                    self.player_capture(into, p);
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
                let size = self.shown.iter().find(|(h, _)| *h == hit).map(|(_, c)| c.world_size());
                let center = self.shown.iter().find(|(h, _)| *h == hit).map(|(_, c)| (c.x + c.world_size().0 as i32 / 2, c.y + c.world_size().1 as i32 / 2));
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
    include!("layout/capture_focus_tests.rs");
    use super::*;

    /// The mouse capture across menus and modes (`capture`).
    mod capture_menus {
        use super::*;
        include!("layout/capture_tests.rs");
    }
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

    /// Frames of `s` (no menu in it) until a capture a menu interrupted is
    /// taken up again ([`capture::RESUME_AFTER`]): not in the first.
    fn menus_gone(l: &mut Layout, s: &Scene) {
        l.frame(s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(!l.captured(), "not at once: another menu may follow");
        for _ in 0..3 {
            l.advance(std::time::Duration::from_millis(100));
            l.frame(s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        }
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
    fn clean_map_masks_are_independent_for_a_b_and_full_detail_zoom() {
        use viewer_settings::clean_map::{Mode, View as Detail};
        for game_downscale in [1, 2] {
            for split in [viewer_settings::Split::Vertical1To1, viewer_settings::Split::Vertical2To1, viewer_settings::Split::Horizontal1To1] {
                let mut scene = scene();
                scene.views[0].view.downscale = game_downscale;
                scene.views[0].view.hidden_categories = viewer_settings::clean_map::ALL;
                let mut layout = Layout::default();
                layout.apply_settings(&viewer_settings::Layout { split, zoom: viewer_settings::Zoom::TopStrip, ..Default::default() });
                layout.clean_map.a = Detail { mode: Mode::Normal, hidden: 0 };
                layout.clean_map.b = Detail { mode: Mode::Custom, hidden: viewer_settings::clean_map::UNITS };
                layout.clean_map.zoom = Detail { mode: Mode::Custom, hidden: viewer_settings::clean_map::CITIES };
                let frame = layout.frame(&scene, (1920, 1080), (0, 0, 0, 0), vec![], BOUNDS);
                for (hit, expected) in [(Hit::Main, 0), (Hit::Pane, viewer_settings::clean_map::UNITS), (Hit::Magnifier, viewer_settings::clean_map::CITIES)] {
                    let logical = layout.shown.iter().find(|(h, _)| *h == hit).unwrap().1;
                    let drawn = frame.cameras.iter().find(|c| c.cells && c.origin == logical.origin && c.viewport == logical.viewport).unwrap();
                    assert_eq!(drawn.hidden_categories, expected, "{hit:?}, {split:?}, downscale{game_downscale}");
                }
                // Each half of the open divider continues its adjacent
                // pane's picture, including its own clean-map mask.
                let ((g0, g1), _) = layout.gap().unwrap();
                let mid = g0 + (g1 - g0) / 2;
                let vertical = layout.split.unwrap().axis == Axis::Vertical;
                let fillers: Vec<_> = frame.cameras.iter().filter(|c| c.cells && if vertical { c.viewport.0 <= (g1 - g0) as u32 } else { c.viewport.1 <= (g1 - g0) as u32 }).collect();
                assert!(fillers.len() >= 2);
                for camera in fillers {
                    let at = if vertical { camera.origin.0 } else { camera.origin.1 };
                    assert_eq!(camera.hidden_categories, if at >= mid { viewer_settings::clean_map::UNITS } else { 0 });
                }
                layout.clean_map = Default::default();
                let frame = layout.frame(&scene, (1920, 1080), (0, 0, 0, 0), vec![], BOUNDS);
                assert!(frame.cameras.iter().filter(|c| c.cells).all(|c| c.hidden_categories == viewer_settings::clean_map::ALL));
                scene.views[0].view.hidden_categories = 0;
                let frame = layout.frame(&scene, (1920, 1080), (0, 0, 0, 0), vec![], BOUNDS);
                assert!(frame.cameras.iter().filter(|c| c.cells).all(|c| c.hidden_categories == 0));
            }
        }
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
        // The panes, the pictures going on into the divider's halves (the
        // glass drawn in them), the interface.
        assert_eq!(f.cameras.len(), 5);
        assert_eq!(f.cameras[0].viewport, (400, 600));
        assert_eq!(f.cameras[1].origin, (404, 0));
        assert_eq!((f.cameras[2].origin, f.cameras[2].viewport), ((400, 0), (2, 600)));
        assert_eq!((f.cameras[3].origin, f.cameras[3].viewport), ((402, 0), (2, 600)));
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
        let (sw, sh) = f.cameras[0].world_size();
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

    #[test]
    fn a_screen_beside_its_world_goes_left_and_the_whole_world_fits_the_rest() {
        // The driver's dialog (1024×768 in the middle of 1920×1080) asks to be
        // left of its world; the user's split does not apply meanwhile.
        let mut l = Layout::default();
        l.configure("split=vertical").unwrap();
        let mut s = scene();
        s.interface.beside = Some(scene::Beside { screen: [448, 156, 1024, 768], edge: scene::Edge::Left, world: None });
        let f = l.frame(&s, (1920, 1080), (0, 0, 0, 0), vec![[448, 156, 1024, 768]], BOUNDS);
        assert_eq!(f.cameras.len(), 2, "the world and the interface only");
        let (world, ui) = (f.cameras[0], f.cameras[1]);
        assert_eq!(f.view_cameras, [Some(0), Some(1)]);
        // The interface moved left by 448, kept at its height, only the
        // dialog of it.
        assert_eq!((ui.origin, ui.viewport, ui.screen_shift), ((0, 156), (1024, 768), (-448, 0)));
        // The world in the rest (from 1028, past the gap), all of it: 8064
        // world pixels wide into 892, 8 times smaller (16 would be zoomed
        // more), a little zoomed out to fill.
        assert_eq!((world.origin, world.viewport), ((1028, 0), (892, 1080)));
        assert_eq!(world.downscale, 8);
        assert!(world.zoom.0 < world.zoom.1);
        let shown = |c: &Camera| {
            let (sw, sh) = c.world_size();
            [c.x, c.y, c.x + sw as i32, c.y + sh as i32]
        };
        let [x0, y0, x1, y1] = shown(&world);
        assert!(x0 <= 0 && y0 <= 0 && x1 >= 8064 / 9 - 1 && y1 >= 4000 / 9, "{:?} {:?}", world.zoom, shown(&world));
        // A click on the dialog reaches the driver where it drew it; one on
        // the world nothing; the snapping pointer knows where the dialog is.
        assert_eq!(click(&mut l, (100, 300))[0], Input::MouseButton { button: Button::Left, down: true, x: 548, y: 300 });
        assert!(click(&mut l, (1500, 500)).is_empty());
        assert_eq!(l.screen_mapping(), ((-448, 0), (1, 1)));
        // Without it, back to the user's arrangement.
        let f = l.frame(&scene(), (1920, 1080), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(f.cameras.len() > 2);
        assert_eq!(l.screen_mapping(), ((0, 0), (1, 1)));
    }

    #[test]
    fn a_preview_already_downscaled_by_the_driver_keeps_its_full_extent() {
        let driver = Camera { downscale: 2, ..Camera::new((3274, 2048)) };
        let fitted = fit_world(&driver, [-64, -64, 9152, 4384], [1375, 0, 254, 952], (64, 32));
        let (w, h) = fitted.world_size();
        assert_eq!(fitted.downscale, 2);
        assert!(h > 4096);
        // The area and camera both use the driver's downscaled pixels.
        assert!(fitted.x <= -64 && fitted.y <= -64);
        assert!(fitted.x + w as i32 >= 9152 - 64 - 1);
        assert!(fitted.y + h as i32 >= 4384 - 64 - 1);
    }

    #[test]
    fn large_world_extents_reach_the_driver_and_capture_center() {
        let camera = Camera { x: 100, y: 200, zoom: (1, 12), ..Camera::new((100, 400)) };
        let mut l = Layout::default();
        l.shown = vec![(Hit::Main, camera), (Hit::Pane, camera)];
        assert_eq!(l.take_main_view_change(), Some((100, 200, 1200, 4800)));
        assert_eq!(l.take_views_change().unwrap()[0], (100, 200, 1200, 4800));
        l.capture(Hit::Main, (0, 0));
        assert_eq!(l.cursor, (700, 2600));
    }

    #[test]
    fn a_world_of_any_size_fits() {
        for (size, rect) in [((100, 50), [0, 0, 892, 1080]), ((20_000, 30_000), [10, 20, 300, 200]), ((8960, 4544), [1028, 0, 892, 1080])] {
            let c = fit_world(&Camera::new((1, 1)), whole_world(WorldBounds { size, wrap: (false, false) }, (64, 32), 1), rect, (64, 32));
            assert!(64 % c.downscale == 0 && 32 % c.downscale == 0, "{size:?}: {c:?}");
            let (sw, sh) = c.world_size();
            let n = c.downscale as i32;
            assert!(c.x <= 0 && c.y <= 0, "{size:?}: {c:?}");
            assert!(c.x + sw as i32 >= (size.0 + 64) / n - 1 && c.y + sh as i32 >= size.1 / n - 1, "{size:?}: {c:?}");
            // Filling the rectangle one way.
            assert!(sw as i32 <= (size.0 + 64) / n + 2 || sh as i32 <= size.1 / n + 2, "{size:?}: {c:?}");
        }
    }

    /// A dialog left of its world, which the driver shows 4 times smaller,
    /// the part (`Beside::world`) around (8000, 4000) at full size fitted;
    /// a hint how to let the mouse go (view 259, at the window's top-left).
    fn beside_scene(cols: u32) -> Scene {
        let mut s = scene();
        s.shape.cols = cols;
        s.views[0].view.downscale = 4;
        s.views.push(ViewState { id: 259, view: View { interface_only: true, origin: (8, 8), viewport: (133, 38), ..Default::default() }, overlay: vec![] });
        s.interface.capture_hint = Some(259);
        s.interface.beside = Some(scene::Beside { screen: [448, 156, 1024, 768], edge: scene::Edge::Left, world: Some([6000, 3000, 4000, 2000]) });
        s
    }

    fn beside_frame(l: &mut Layout, s: &Scene) -> Frame {
        // (The bounds at the scale the driver's view shows the world.)
        l.frame(s, (1920, 1080), (0, 0, 0, 0), vec![[448, 156, 1024, 768]], Some(WorldBounds { size: (4000, 2000), wrap: (true, false) }))
    }

    /// Frames over a second (one frame takes at most `MAX_STEP`).
    fn wait(l: &mut Layout, s: &Scene) {
        for _ in 0..10 {
            l.advance(std::time::Duration::from_millis(100));
            beside_frame(l, s);
        }
    }

    fn centre(c: &Camera) -> (i32, i32) {
        let (sw, sh) = c.world_size();
        (c.x + sw as i32 / 2, c.y + sh as i32 / 2)
    }

    #[test]
    fn the_part_of_the_world_asked_for_fits_beside_the_screen() {
        let mut l = Layout::default();
        let f = beside_frame(&mut l, &beside_scene(62));
        let c = f.cameras[0];
        assert_eq!((c.origin, c.viewport), ((1028, 0), (892, 1080)));
        // 4000 × 2000 world pixels from (6000, 3000), 1000 × 500 in the
        // driver's view: as the driver shows it, a little zoomed out, around
        // (8000, 4000) (2000, 1000 there).
        assert_eq!(c.downscale, 4);
        assert!(c.zoom.0 < c.zoom.1);
        let (x, y) = centre(&c);
        assert!((x - 2000).abs() <= 2 && (y - 1000).abs() <= 2, "{c:?}");
    }

    #[test]
    fn a_double_click_explores_the_world_beside_the_screen_up_close() {
        let mut l = Layout::default();
        let s = beside_scene(62);
        let fitted = beside_frame(&mut l, &s).cameras[0];
        // The point double-clicked, in the pixels of the half-size world
        // (the game's zoomed-out view).
        let q = world_at(&fitted, (1500, 300));
        let n = fitted.downscale as i32 / 2;
        let at = (q.0 * n, q.1 * n);
        assert!(click(&mut l, (1500, 300)).is_empty());
        assert!(!l.captured());
        assert!(click(&mut l, (1500, 300)).is_empty());
        assert!(l.captured());
        // As the game's Z shows it (half size, 1:1, whatever the scale the
        // driver fits it at), around it;
        // the world cursor there, the hint at the world's top-left.
        let f = beside_frame(&mut l, &s);
        let c = f.cameras[0];
        assert_eq!((c.zoom, c.downscale, c.origin, c.viewport), ((1, 1), 2, (1028, 0), (892, 1080)));
        assert_eq!(centre(&c), at);
        assert_eq!(f.markers, [Marker::Cursor(at)]);
        assert_eq!(f.cameras[f.view_cameras[2].unwrap() as usize].origin, (1028 + 8, 8));
        // Nothing reaches the driver: clicks, keys, the wheel, motion.
        assert!(click(&mut l, (100, 400)).is_empty());
        assert!(key(&mut l, 0x41, true).is_empty() && key(&mut l, 0x41, false).is_empty());
        assert!(l.route(Input::Wheel { delta: 1, x: 300, y: 400 }).is_empty());
        assert!(l.motion((20.0, 0.0)).is_empty());
        let f = beside_frame(&mut l, &s);
        assert_eq!(centre(&f.cameras[0]), (at.0 + 20, at.1));
        // Ctrl on its own lets the mouse go: still explored, no hint; the
        // dialog takes clicks again.
        key(&mut l, VK_CONTROL, true);
        key(&mut l, VK_CONTROL, false);
        assert!(!l.captured());
        let f = beside_frame(&mut l, &s);
        assert_eq!((f.cameras[0].zoom, centre(&f.cameras[0])), ((1, 1), (at.0 + 20, at.1)));
        assert!(f.markers.is_empty() && f.view_cameras[2].is_none());
        assert_eq!(click(&mut l, (100, 400))[0], Input::MouseButton { button: Button::Left, down: true, x: 548, y: 400 });
        // A click on the world captures the mouse again, the view put.
        wait(&mut l, &s);
        assert!(click(&mut l, (1500, 300)).is_empty());
        assert!(l.captured());
        assert_eq!(centre(&beside_frame(&mut l, &s).cameras[0]), (at.0 + 20, at.1));
        // Esc goes back to the world fitted (and does not reach the driver).
        assert!(key(&mut l, VK_ESCAPE, true).is_empty() && key(&mut l, VK_ESCAPE, false).is_empty());
        assert!(!l.captured());
        assert_eq!(beside_frame(&mut l, &s).cameras[0], fitted);
        // So does a double-click while exploring.
        click(&mut l, (1500, 300));
        click(&mut l, (1500, 300));
        assert!(l.captured());
        wait(&mut l, &s);
        click(&mut l, (1500, 300));
        click(&mut l, (1500, 300));
        assert!(!l.captured());
        assert_eq!(beside_frame(&mut l, &s).cameras[0], fitted);
        // Slow clicks are no double-click.
        click(&mut l, (1500, 300));
        wait(&mut l, &s);
        click(&mut l, (1500, 300));
        assert!(!l.captured());
    }

    /// A double-click on a preview fitted very small (the driver's view at
    /// downscale 8) goes to the game's Z view: downscale 2 and the game
    /// view's zoom with the world scale (here x2), not the fitted zoom
    /// doubled; the world cursor keeps to the half-size world.
    #[test]
    fn a_double_click_goes_to_the_games_zoomed_out_view() {
        let mut l = Layout { world_scale: (2, 1), ..Layout::default() };
        let mut s = beside_scene(62);
        s.views[0].view.downscale = 8;
        let bounds = Some(WorldBounds { size: (2000, 1000), wrap: (false, false) });
        let frame = |l: &mut Layout| l.frame(&s, (1920, 1080), (0, 0, 0, 0), vec![[448, 156, 1024, 768]], bounds);
        let fitted = frame(&mut l).cameras[0];
        assert_eq!(fitted.downscale, 8);
        let q = world_at(&fitted, (1500, 300));
        let at = (q.0 * fitted.downscale as i32 / 2, q.1 * fitted.downscale as i32 / 2);
        click(&mut l, (1500, 300));
        click(&mut l, (1500, 300));
        assert!(l.captured());
        let c = frame(&mut l).cameras[0];
        assert_eq!((c.zoom, c.downscale), ((2, 1), 2));
        assert_eq!(centre(&c), at);
        // Moved far right: held at the half-size world's edge (8000 wide).
        l.motion((100_000.0, 0.0));
        let c = frame(&mut l).cameras[0];
        let (sw, _) = c.world_size();
        assert_eq!(c.x + sw as i32, 8000);
    }

    #[test]
    fn another_world_beside_the_screen_is_shown_fitted() {
        let mut l = Layout::default();
        let s = beside_scene(62);
        beside_frame(&mut l, &s);
        click(&mut l, (1500, 300));
        click(&mut l, (1500, 300));
        assert!(l.captured());
        // Another scenario picked: its world, fitted, the mouse let go.
        let other = beside_scene(80);
        let f = beside_frame(&mut l, &other);
        assert!(!l.captured());
        assert_ne!((f.cameras[0].zoom, f.cameras[0].downscale), ((1, 1), 2));
        assert!(f.markers.is_empty());
        // The screen closing ends it too.
        beside_frame(&mut l, &other);
        click(&mut l, (1500, 300));
        click(&mut l, (1500, 300));
        assert!(l.captured());
        frame(&mut l, vec![]);
        assert!(!l.captured());
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
    fn parked_native_camera_routes_all_map_input_virtually_and_keeps_ui_native() {
        let mut l = Layout::default();
        frame(&mut l, vec![[0, 500, 800, 100]]);
        l.camera_focus_serial = Some(0);
        assert_eq!(l.route(Input::MouseMove { x: 10, y: 20 }), [Input::WorldMove { x: 1010, y: 520 }]);
        assert_eq!(l.route(Input::Key { vk: 0x47, down: true, text: None }), [Input::WorldKey { vk: 0x47, down: true, text: None, x: 1010, y: 520 }]);
        assert!(matches!(click(&mut l, (10, 20))[..], [Input::Look { .. }, Input::WorldButton { down: true, .. }, Input::WorldButton { down: false, .. }]));
        assert_eq!(l.route(Input::MouseMove { x: 10, y: 550 }), [Input::MouseMove { x: 10, y: 550 }]);
        assert_eq!(click(&mut l, (10, 550))[0], Input::MouseButton { button: Button::Left, down: true, x: 10, y: 550 });
    }

    #[test]
    fn a_parked_native_camera_keeps_plain_map_wheel_and_ui_wheel_while_ctrl_still_zooms() {
        let mut l = Layout::default();
        frame(&mut l, vec![[0, 500, 800, 100]]);
        l.camera_focus_serial = Some(0);
        l.map_wheel_step = (128, 64);
        assert_eq!(l.route(Input::Wheel { delta: 120, x: 10, y: 20 }), [Input::WorldMove { x: 1010, y: 456 }]);
        assert_eq!(l.main_view.center, (1400, 736));
        assert_eq!(l.route(Input::Wheel { delta: -120, x: 10, y: 550 }), [Input::Wheel { delta: -120, x: 10, y: 550 }]);
        l.route(Input::Key { vk: VK_CONTROL, down: true, text: None });
        assert!(l.route(Input::Wheel { delta: 120, x: 10, y: 20 }).is_empty());
    }

    #[test]
    fn enabling_virtual_map_input_during_a_press_keeps_the_release() {
        let mut l = Layout::default();
        frame(&mut l, vec![]);
        assert_eq!(l.route(Input::MouseButton { button: Button::Left, down: true, x: 10, y: 20 }), [Input::MouseButton { button: Button::Left, down: true, x: 10, y: 20 }]);
        l.camera_focus_serial = Some(0);
        assert_eq!(l.route(Input::MouseButton { button: Button::Left, down: false, x: 20, y: 30 }), [Input::WorldButton { button: Button::Left, down: false, x: 1020, y: 530 }]);
        assert!(!l.pressed[0]);
        l.route(Input::MouseButton { button: Button::Left, down: true, x: 20, y: 30 });
        l.camera_focus_serial = None;
        assert!(matches!(l.route(Input::MouseButton { button: Button::Left, down: false, x: 30, y: 40 })[..], [Input::WorldButton { button: Button::Left, down: false, .. }]));
        assert!(l.world_pressed[0].is_none());
    }

    #[test]
    fn an_off_camera_world_pointer_keeps_hover_and_targeting_alive() {
        let mut l = free_pane();
        let pane = l.shown.iter().find(|(h, _)| *h == Hit::Pane).unwrap().1;
        let expected = world_at(&pane, (600, 300));
        assert!(l.in_main(expected).is_none());
        assert_eq!(l.route(Input::MouseMove { x: 600, y: 300 }), [Input::WorldMove { x: expected.0, y: expected.1 }]);
        // Returning to A restores ordinary client-coordinate input.
        assert_eq!(l.route(Input::MouseMove { x: 100, y: 100 }), [Input::MouseMove { x: 300, y: 100 }]);
        // Captured motion uses the same independent world point, even far
        // outside the original game camera, rather than dropping the move.
        l.split.as_mut().unwrap().view.follow = Follow::Pointer;
        l.cursor = (5000, 2000);
        frame(&mut l, vec![]);
        l.capture(Hit::Pane, (600, 300));
        let before = l.cursor;
        assert_eq!(l.motion((32.0, -16.0)), [Input::WorldMove { x: before.0 + 32, y: before.1 - 16 }]);
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
    fn world_edges_never_start_the_native_tile_based_scroller() {
        let mut l = Layout::default();
        frame(&mut l, vec![[0, 560, 800, 40]]);
        for zoom in [ONE, ONE + 2] {
            l.main_view.zoom = zoom;
            frame(&mut l, vec![[0, 560, 800, 40]]);
            for p in [(400, 300), (799, 300), (796, 300), (0, 599)] {
                let out = mv(&mut l, p);
                assert!(!out.iter().any(|i| matches!(i, Input::Edge { .. })), "{out:?}");
                assert_eq!(l.driver_edge(p), (0, 0));
            }
        }
        assert!(l.pointer_gone().is_empty());
        // Native edge state left by an older arrangement is still reset.
        l.edge = (1, 0);
        assert_eq!(l.pointer_gone(), [Input::Edge { x: 0, y: 0 }]);
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
            // (The divider's two halves: the panes' pictures going on under
            // the glass, a block of the background beside a fitted screen.)
            assert_eq!(f.cameras.len(), 2 + 1 + 2 * MAX_INSETS + 2 + 2, "ui={ui}");
            assert!(f.cameras.len() <= tile_render::MAX_VIEWS);
            // The interface on top of the viewer's own views (bar the
            // magnifier), or in the pane.
            let interface = f.view_cameras[1].unwrap() as usize;
            assert_eq!(interface, if ui == "pane" { 1 } else { f.cameras.len() - 3 }, "ui={ui}");
        }
        // A driver with many views: the viewer leaves out what does not
        // fit (insets last), never more than the renderer draws.
        let mut s = scene();
        for id in 1..(tile_render::MAX_VIEWS - 6) as u32 {
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
            shape: scene().shape,
            ..Default::default()
        }
    }

    #[test]
    fn city_screen_controls_keep_detail_when_clean_map_filters_world_panes() {
        use viewer_settings::clean_map::{ALL, CITIES, Mode, UNITS, View as Detail, YIELDS};
        for split in [S::Standard, S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
            for place in [Place::Floating, Place::LeftOrBottom, Place::RightOrTop] {
                for zoom in [Zoom::Off, Zoom::TopStrip] {
                    for downscale in [1, 2] {
                        let mut settings = settings(|s| (s.split, s.zoom) = (split, zoom));
                        settings.screen_place = ScreenPlace { vertical: place, horizontal: place };
                        let mut l = Layout::default();
                        l.apply_settings(&settings);
                        l.clean_map.a = Detail { mode: Mode::Custom, hidden: UNITS };
                        l.clean_map.b = Detail { mode: Mode::Custom, hidden: YIELDS };
                        l.clean_map.zoom = Detail { mode: Mode::Custom, hidden: CITIES };
                        let mut city = city_screen();
                        city.shape = scene().shape;
                        city.interface.screen = Some("City".into());
                        city.views[0].view.hidden_categories = ALL;
                        city.views[0].view.downscale = downscale;
                        city.views[1].view.downscale = downscale;
                        let f = l.frame(&city, (800, 600), (0, 0, 0, 0), vec![[0, 0, 800, 150], [0, 350, 800, 250]], BOUNDS);
                        let internal = f.cameras[f.view_cameras[1].unwrap() as usize];
                        assert_eq!(internal.hidden_categories, 0, "City control keeps workers and yields: {split:?}, {place:?}, {zoom:?}, downscale{downscale}");
                        for (hit, expected) in [(Hit::Main, UNITS), (Hit::Pane, YIELDS), (Hit::Magnifier, CITIES)] {
                            if let Some((_, shown)) = l.shown.iter().find(|(h, _)| *h == hit) {
                                let camera = f.cameras.iter().find(|c| c.cells && !c.stacked && c.origin == shown.origin && c.viewport == shown.viewport).unwrap();
                                assert_eq!(camera.hidden_categories, expected, "{hit:?} keeps its world filter: {split:?}, {place:?}, {zoom:?}, downscale{downscale}");
                            }
                        }
                    }
                }
            }
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
        // zoomed 2/1, a pane and an inset draw the half-size world, in its
        // pixels. The magnifier draws the same area from full-size art.
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
        let logical = l.shown.iter().find(|(hit, _)| *hit == Hit::Magnifier).unwrap().1;
        let magnifier = l.magnifier_art_camera(logical);
        assert!(world.contains(&&magnifier));
        assert!(world.iter().all(|&&c| c == magnifier || c.downscale == 2), "{world:?}");
        assert_eq!((magnifier.downscale, magnifier.zoom), (1, (3, 2)));
        assert_eq!(f.cameras[0].zoom, (2, 1));
        // The interface stays as it is.
        assert_eq!(f.cameras[f.view_cameras[1].unwrap() as usize].downscale, 1);
    }

    #[test]
    fn world_magnifiers_keep_full_detail_and_driver_input_at_every_game_scale() {
        for downscale in [1, 2, 4] {
            for place in [ZoomPlace::Beside, ZoomPlace::Box(3, ZoomBox { height: 250, top: 25, drop: 300 }), ZoomPlace::Box(7, ZoomBox { height: 250, top: 25, drop: 300 }), ZoomPlace::AboutPointer(3, 250), ZoomPlace::Strip(250)] {
                let mut s = scene();
                s.views[0].view.downscale = downscale;
                s.zooms = vec![(0, 0)];
                let mut l = Layout::default();
                l.configure("magnifier=world,magnifier_zoom=2").unwrap();
                l.magnifier.as_mut().unwrap().place = place;
                l.pointer = (200, 400);
                l.frame(&s, (800, 600), (16, 24, 8, 12), vec![], BOUNDS);
                let f = l.frame(&s, (800, 600), (16, 24, 8, 12), vec![], BOUNDS);
                let logical = l.shown.iter().find(|(h, _)| *h == Hit::Magnifier).unwrap().1;
                let r = rect_of(&logical);
                let rendered = f.cameras.iter().find(|c| c.cells && rect_of(c) == r).unwrap();
                assert_eq!((rendered.downscale, rendered.zoom), (1, times((2, 1), (1, downscale))), "{place:?}");
                assert_eq!((rendered.x, rendered.y), (logical.x * downscale as i32, logical.y * downscale as i32));
                assert_eq!(rendered.overhang, (16, 24, 8, 12));
                assert_eq!(rendered.zoomed_art, rendered.zoom != (1, 1));
                for p in [(r[0], r[1]), (r[0] + r[2] / 2, r[1] + r[3] / 2), (r[0] + r[2] - 1, r[1] + r[3] - 1)] {
                    let w = world_at(&logical, p);
                    let full = world_at(rendered, p);
                    assert_eq!((full.0 / downscale as i32, full.1 / downscale as i32), w, "{place:?} {p:?}");
                    assert_eq!(l.route_free(p), l.route_world(w), "{place:?} {p:?}");
                }
                let p = (r[0] + r[2] / 2, r[1] + r[3] / 2);
                let w = world_at(&logical, p);
                assert_eq!(l.magnifier_center, Some(w));
                if rendered.zoom != (1, 1) {
                    let zoom = (rendered.zoom.0 as u8, rendered.zoom.1 as u8);
                    assert!(l.take_zooms_change().unwrap().contains(&zoom));
                }
            }
        }
    }

    #[test]
    fn zoomed_out_magnifier_renders_original_pixels_instead_of_the_half_size_twin() {
        use tile_render::{AtlasBuilder, Grid, PlacedSprite, RgbaImage, SpriteSets, TileWorld};
        let mut s = scene();
        s.views[0].view.downscale = 2;
        let mut l = Layout::default();
        l.configure("magnifier=world,magnifier_zoom=2").unwrap();
        l.magnifier.as_mut().unwrap().size = (8, 8);
        l.pointer = (200, 400);
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let logical = l.shown.iter().find(|(h, _)| *h == Hit::Magnifier).unwrap().1;
        let rendered = f.cameras.iter().find(|c| c.cells && rect_of(c) == rect_of(&logical)).unwrap();
        let original = RgbaImage::from_fn(8, 8, |x, y| [x as u8 * 31, y as u8 * 31, (x ^ y) as u8 * 31, 255]);
        let mut builder = AtlasBuilder::new(16);
        let normal = builder.add(&original, (0, 0)).unwrap();
        let half = builder.add(&RgbaImage::from_fn(8, 8, |_, _| [255, 0, 255, 255]), (0, 0)).unwrap();
        builder.sprite_mut(normal).downscaled = half.0 + 1;
        let atlas = builder.finish();
        let mut sets = SpriteSets::default();
        let set = sets.single(normal);
        let grid = Grid { cols: 1, rows: 1, layout: tile_render::Layout::Orthogonal, wrap_x: false, wrap_y: false, step: (8, 8) };
        let mut world = TileWorld::new(grid, sets);
        world.free.push(PlacedSprite::new(set, rendered.x, rendered.y));
        let local = Camera { origin: (0, 0), ..*rendered };
        let actual = tile_render::reference::render(&world, &atlas, &local, 0, [0; 4]);
        assert_eq!(actual, original);
        let old = Camera { origin: (0, 0), ..logical };
        let pixelated = tile_render::reference::render(&world, &atlas, &old, 0, [0; 4]);
        assert_ne!(pixelated, actual);
        assert_eq!(pixelated.get(0, 0), [255, 0, 255, 255]);
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
    fn the_main_minimap_mark_uses_the_actual_camera_at_every_resolution_scale() {
        // A single unzoomed pane may still show less than the game's screen
        // because the render canvas or world scale changed.
        for (window, scale) in [((800, 600), (2, 1)), ((400, 300), (1, 1)), ((800, 600), (1, 2))] {
            let mut l = Layout::default();
            l.driver_screen = Some((800, 600));
            l.world_scale = scale;
            let f = l.frame(&scene(), window, (0, 0, 0, 0), vec![], BOUNDS);
            let c = f.cameras[0];
            let (w, h) = c.world_size();
            assert_eq!(l.take_main_view_change(), Some((c.x, c.y, w as i32, h as i32)), "window {window:?}, scale {scale:?}");
            assert_eq!(l.take_main_view_change(), None);
            // Returning to exactly the full driver's view restores its mark.
            l.world_scale = (1, 1);
            frame(&mut l, vec![]);
            assert_eq!(l.take_main_view_change(), Some((0, 0, 0, 0)));
        }
    }

    #[test]
    fn the_main_minimap_mark_follows_its_gliding_camera() {
        let mut l = arranged(&settings(|s| s.smooth_world_movement = true));
        assert_eq!(l.take_main_view_change(), None);
        let mut s = scene();
        s.views[0].view.camera_x += 64;
        l.advance(std::time::Duration::from_millis(16));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let c = f.cameras[0];
        assert_ne!(c.x, s.views[0].view.camera_x);
        assert_eq!(l.take_main_view_change(), Some((c.x, c.y, 800, 600)));
        for _ in 0..120 {
            l.advance(std::time::Duration::from_millis(16));
            l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        }
        assert_eq!(l.take_main_view_change(), Some((0, 0, 0, 0)));
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
        let (sw, sh) = f.cameras[0].world_size();
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
    fn zoom_boxes_sit_at_the_top_or_hover_about_the_pointer() {
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
        // 1:3:1 and 1:7:1 across the window, centred, at its top (2.5 %
        // of its height, 15, above it), `zoom_strip` (a fifth) high.
        assert_eq!(geometry(&settings(|s| s.zoom = Zoom::Floating131), (10, 10)), [160, 15, 480, 120]);
        assert_eq!(geometry(&settings(|s| s.zoom = Zoom::Floating171), (10, 10)), [89, 15, 622, 120]);
        // Never in the middle, whatever the split.
        for split in [S::Standard, S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
            for zoom in [Zoom::Floating131, Zoom::Floating171] {
                let r = geometry(&settings(|s| (s.split, s.zoom) = (split, zoom)), (10, 590));
                assert_eq!((r[1], r[3]), (15, 120), "{split:?} {zoom:?}");
            }
        }
        // In a vertical split, over A or B as `zoom_place` says: 1:3:1 of
        // that view's width, at its top.
        let left = settings(|s| (s.split, s.zoom, s.zoom_place) = (S::Vertical1To1, Zoom::Floating131, Place::LeftOrBottom));
        assert_eq!(geometry(&left, (10, 10)), [80, 15, 240, 120]);
        let right = Settings { zoom_place: Place::RightOrTop, ..left.clone() };
        assert_eq!(geometry(&right, (10, 10)), [483, 15, 237, 120]);
        let over = Settings { zoom_place: Place::Floating, ..left.clone() };
        assert_eq!(geometry(&over, (10, 10)), [160, 15, 480, 120]);
        // (Not in a horizontal split.)
        let stacked = Settings { split: S::Horizontal1To1, ..right.clone() };
        assert_eq!(geometry(&stacked, (10, 10)), [160, 15, 480, 120]);
        // At the pointer: 1:1:1, 1:3:1 across, `zoom_strip` (a fifth) of
        // the height, above the pointer, centred on it across.
        assert_eq!(geometry(&settings(|s| s.zoom = Zoom::Mouse111), (400, 300)), [267, 176, 266, 120]);
        assert_eq!(geometry(&settings(|s| s.zoom = Zoom::Mouse131), (400, 300)), [160, 176, 480, 120]);
        // No room above: below it, clear of its arrow; inside the window.
        assert_eq!(geometry(&settings(|s| s.zoom = Zoom::Mouse111), (790, 5)), [534, 29, 266, 120]);
        let mouse_right = Settings { zoom: Zoom::Mouse111, ..right };
        assert_eq!(geometry(&mouse_right, (420, 590)), [404, 466, 132, 120]);
    }

    #[test]
    fn floating_boxes_take_the_zoom_height_and_their_top_margin_in_their_region() {
        for split in [S::Standard, S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
            for place in [Place::Floating, Place::LeftOrBottom, Place::RightOrTop] {
                for zoom in [Zoom::Floating131, Zoom::Floating171] {
                    let mut s = settings(|s| (s.split, s.zoom_place, s.zoom) = (split, place, zoom));
                    let mut l = arranged(&s);
                    let m = l.magnifier.unwrap();
                    let initial = l.magnifier_geometry(&m).0;
                    let region = l.zoom_region(&m);
                    for (height, top) in [(0.05, 0.0), (0.2, 0.025), (0.5, 0.2), (0.25, 0.1)] {
                        (s.zoom_strip, s.zoom_box_top) = (height, top);
                        // (An old file's own height: ignored.)
                        s.zoom_box_height = 1.0;
                        l.apply_settings(&s);
                        // (The pointer off the box: it stays.)
                        l.place_pointer((region[0] + 2, region[1] + region[3] - 2));
                        let f = frame(&mut l, vec![]);
                        let r = l.magnifier_geometry(&l.magnifier.unwrap()).0;
                        let what = format!("{split:?} {place:?} {zoom:?} {height} {top}");
                        assert_eq!((r[0], r[2]), (initial[0], initial[2]), "{what}");
                        assert_eq!(r[0], region[0] + (region[2] - r[2]) / 2, "{what}");
                        let part = |v: f32| (region[3] as i64 * (v * 1000.0).round() as i64 / 1000) as i32;
                        assert_eq!((r[1], r[3]), (region[1] + part(top), part(height)), "{what}");
                        assert_eq!(rect(&camera_at(&f, (r[0], r[1]))), r, "{what}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_zoom_box_at_the_mouse_stays_in_the_view_zoom_place_names() {
        let geometry = |s: &Settings, pointer: (i32, i32)| {
            let mut l = arranged(s);
            mv(&mut l, pointer);
            frame(&mut l, vec![]);
            l.magnifier_geometry(&l.magnifier.unwrap()).0
        };
        let (a, b) = ([0, 0, 400, 600], [404, 0, 396, 600]);
        let within = |r: [i32; 4], v: [i32; 4]| r[0] >= v[0] && r[1] >= v[1] && r[0] + r[2] <= v[0] + v[2] && r[1] + r[3] <= v[1] + v[3];
        for zoom in [Zoom::Mouse111, Zoom::Mouse131] {
            for roles in [Roles::Free, Roles::UnitMouse] {
                let left = settings(|s| (s.split, s.roles, s.zoom, s.zoom_place) = (S::Vertical1To1, roles, zoom, Place::LeftOrBottom));
                let right = Settings { zoom_place: Place::RightOrTop, ..left.clone() };
                let over = Settings { zoom_place: Place::Floating, ..left.clone() };
                let mut last = None;
                // The mouse across both views: the box follows it across, in
                // its view, clamped at that view's sides.
                for x in [10, 200, 390, 410, 600, 790] {
                    let (l, r, o) = (geometry(&left, (x, 300)), geometry(&right, (x, 300)), geometry(&over, (x, 300)));
                    assert!(within(l, a), "{zoom:?} {roles:?} left at {x}: {l:?}");
                    assert!(within(r, b), "{zoom:?} {roles:?} right at {x}: {r:?}");
                    assert!(within(o, [0, 0, 800, 600]), "{zoom:?} {roles:?} floating at {x}: {o:?}");
                    for g in [l, r, o] {
                        assert_eq!((g[1], g[3]), (176, 120), "{zoom:?} {roles:?} at {x}: above the pointer");
                    }
                    if let Some((pl, pr, po)) = last {
                        assert!(l[0] >= pl && r[0] >= pr && o[0] >= po, "{zoom:?} at {x}: it follows the mouse");
                    }
                    last = Some((l[0], r[0], o[0]));
                }
            }
        }
        let mouse = settings(|s| (s.split, s.zoom, s.zoom_place) = (S::Vertical1To1, Zoom::Mouse111, Place::LeftOrBottom));
        // Centred on the pointer in its view; at its side with the pointer in
        // the other view.
        assert_eq!(geometry(&mouse, (200, 300)), [134, 176, 133, 120]);
        assert_eq!(geometry(&mouse, (600, 300)), [267, 176, 133, 120]);
        let mouse_right = Settings { zoom_place: Place::RightOrTop, ..mouse.clone() };
        assert_eq!(geometry(&mouse_right, (100, 300)), [404, 176, 132, 120]);
        assert_eq!(geometry(&mouse_right, (600, 300)), [534, 176, 132, 120]);
        // Floating: over the divider.
        let floating = Settings { zoom_place: Place::Floating, ..mouse };
        assert_eq!(geometry(&floating, (420, 300)), [287, 176, 266, 120]);
    }

    #[test]
    fn zoom_strip_sizes_every_zoom() {
        let geometry = |s: &Settings, pointer: (i32, i32)| {
            let mut l = arranged(s);
            mv(&mut l, pointer);
            frame(&mut l, vec![]);
            l.magnifier_geometry(&l.magnifier.unwrap()).0
        };
        for split in [S::Standard, S::Vertical1To1] {
            for place in [Place::LeftOrBottom, Place::RightOrTop, Place::Floating] {
                for zoom in [Zoom::Floating131, Zoom::Floating171] {
                    // (The pointer at the bottom left, off the box.)
                    let at = |h: f32| geometry(&settings(|s| (s.split, s.zoom, s.zoom_place, s.zoom_strip) = (split, zoom, place, h)), (2, 590));
                    for (h, px) in [(0.05, 30), (0.25, 150), (0.5, 300), (0.01, 30), (0.9, 300)] {
                        let r = at(h);
                        assert_eq!((r[1], r[3]), (15, px), "{split:?} {place:?} {zoom:?} {h}");
                        assert_eq!((r[0], r[2]), (at(0.2)[0], at(0.2)[2]), "{split:?} {place:?} {zoom:?} {h}");
                    }
                }
            }
        }
        let mouse = |h: f32| geometry(&settings(|s| (s.zoom, s.zoom_strip) = (Zoom::Mouse111, h)), (400, 400));
        assert_eq!(mouse(0.05), [267, 366, 266, 30]);
        assert_eq!(mouse(0.5), [267, 96, 266, 300]);
        // Below 5 %: 5 %; above 50 %: 50 %.
        assert_eq!(mouse(0.01), mouse(0.05));
        assert_eq!(mouse(0.9), mouse(0.5));
        // In a view: that fraction of its height (the whole window's here).
        let right = |h: f32| geometry(&settings(|s| (s.split, s.zoom, s.zoom_place, s.zoom_strip) = (S::Vertical1To1, Zoom::Mouse131, Place::RightOrTop, h)), (600, 400));
        assert_eq!(right(0.05), [482, 366, 237, 30]);
        // The strip too, from 5 %.
        let strip = |h: f32| geometry(&settings(|s| (s.zoom, s.zoom_strip) = (Zoom::TopStrip, h)), (400, 400));
        assert_eq!(strip(0.05), [0, 0, 800, 30]);
        assert_eq!(strip(0.5), [0, 0, 800, 300]);
    }

    #[test]
    fn a_floating_zoom_box_drops_below_the_pointer_so_clicks_reach_the_map_under_it() {
        let mut l = arranged(&settings(|s| s.zoom = Zoom::Floating131));
        // The pointer into the box at the top: it drops, its top 30 % of
        // the window's height (180) below the pointer.
        mv(&mut l, (400, 60));
        let f = frame(&mut l, vec![]);
        let r = l.magnifier_geometry(&l.magnifier.unwrap()).0;
        assert_eq!(r, [160, 240, 480, 120]);
        assert!(!inside(r, l.pointer));
        let c = camera_at(&f, (r[0], r[1]));
        let (w, h) = c.world_size();
        assert_eq!(Some((c.x + w as i32 / 2, c.y + h as i32 / 2)), l.world_under((400, 60)));
        assert_eq!(click(&mut l, (400, 60))[0], Input::MouseButton { button: Button::Left, down: true, x: 400, y: 60 });
        // Its drop setting; as far as the window's bottom lets it.
        let mut l = arranged(&settings(|s| (s.zoom, s.zoom_box_drop) = (Zoom::Floating131, 0.1)));
        mv(&mut l, (400, 60));
        assert_eq!(l.magnifier_geometry(&l.magnifier.unwrap()).0, [160, 120, 480, 120]);
        let mut l = arranged(&settings(|s| (s.zoom, s.zoom_strip, s.zoom_box_top, s.zoom_box_drop) = (Zoom::Floating131, 0.5, 0.2, 0.5)));
        mv(&mut l, (400, 200));
        assert_eq!(l.magnifier_geometry(&l.magnifier.unwrap()).0, [160, 300, 480, 300]);
        // No room for it all below the pointer: cut to what is below.
        mv(&mut l, (400, 400));
        assert_eq!(l.magnifier_geometry(&l.magnifier.unwrap()).0, [160, 424, 480, 176]);
        // Away from it: back at the top.
        mv(&mut l, (10, 590));
        assert_eq!(l.magnifier_geometry(&l.magnifier.unwrap()).0, [160, 120, 480, 300]);
    }

    #[test]
    fn the_zoom_magnifies_as_set_and_ctrl_wheel_stays_within_the_choices() {
        let shown_zoom = |l: &Layout| l.shown.iter().find(|(h, _)| *h == Hit::Magnifier).unwrap().1.zoom;
        for zoom in [Zoom::TopStrip, Zoom::Floating131, Zoom::Mouse111] {
            for (magnification, expected) in [(1.5, (3, 2)), (2.0, (2, 1)), (3.0, (3, 1)), (4.0, (4, 1)), (2.5, (2, 1))] {
                let l = arranged(&settings(|s| (s.zoom, s.zoom_magnification) = (zoom, magnification)));
                assert_eq!((ZOOMS[l.magnifier.unwrap().zoom], shown_zoom(&l)), (expected, expected), "{zoom:?} {magnification}");
            }
        }
        // Ctrl+wheel over the strip: up to 4×, down to 1.5×.
        let s = settings(|s| s.zoom = Zoom::TopStrip);
        let mut l = arranged(&s);
        l.set_walls(Walls::Open);
        mv(&mut l, (400, 50));
        key(&mut l, VK_CONTROL, true);
        let wheel = |l: &mut Layout, delta| { l.route(Input::Wheel { delta, x: 400, y: 50 }); ZOOMS[l.magnifier.unwrap().zoom] };
        assert_eq!([wheel(&mut l, 120), wheel(&mut l, 120), wheel(&mut l, 120)], [(3, 1), (4, 1), (4, 1)]);
        assert_eq!([wheel(&mut l, -120), wheel(&mut l, -120), wheel(&mut l, -120), wheel(&mut l, -120)], [(3, 1), (2, 1), (3, 2), (3, 2)]);
        key(&mut l, VK_CONTROL, false);
        // Kept while the setting stays (other settings changing); the
        // setting changed: as it says.
        l.apply_settings(&Settings { zoom_strip: 0.3, ..s.clone() });
        assert_eq!(ZOOMS[l.magnifier.unwrap().zoom], (3, 2));
        l.apply_settings(&Settings { zoom_magnification: 3.0, ..s });
        assert_eq!(ZOOMS[l.magnifier.unwrap().zoom], (3, 1));
    }

    #[test]
    fn zoom_boxes_lie_under_the_corner_buttons_and_leave_them_their_clicks() {
        // The corner buttons at A's top-left (0..150 x 0..50).
        let s = with_parts(None);
        let boxed = |f: &Frame, r: [i32; 4]| f.cameras.iter().position(|c| c.cells && c.origin == (r[0], r[1]) && c.zoom == (2, 1)).unwrap();
        // A floating box over A, at its top: over the buttons, not moved.
        let mut l = arranged(&settings(|s| (s.split, s.zoom, s.zoom_place) = (S::Vertical1To1, Zoom::Floating131, Place::LeftOrBottom)));
        mv(&mut l, (100, 400));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let r = l.magnifier_geometry(&l.magnifier.unwrap()).0;
        assert_eq!(r, [80, 15, 240, 120]);
        assert!(f.view_cameras[2].unwrap() as usize > boxed(&f, r), "the buttons are drawn over the box");
        // A click on a button over the box reaches it; the box stays.
        assert_eq!(click(&mut l, (100, 30))[0], Input::MouseButton { button: Button::Left, down: true, x: 100, y: 30 });
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.magnifier_geometry(&l.magnifier.unwrap()).0, r);
        // Beside the buttons, the box's world.
        assert_eq!(l.hit((200, 30)), Hit::Magnifier);
        // A box at the mouse: above the pointer, over the buttons, not
        // moved below them.
        for zoom in [Zoom::Mouse111, Zoom::Mouse131] {
            let mut l = arranged(&settings(|s| (s.split, s.zoom, s.zoom_place) = (S::Vertical1To1, zoom, Place::LeftOrBottom)));
            mv(&mut l, (100, 140));
            let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            let r = l.magnifier_geometry(&l.magnifier.unwrap()).0;
            assert_eq!((r[0], r[1], r[3]), (if zoom == Zoom::Mouse111 { 34 } else { 0 }, 16, 120), "{zoom:?}");
            assert!(f.view_cameras[2].unwrap() as usize > boxed(&f, r), "{zoom:?}: the buttons are drawn over the box");
            assert_ne!(l.hit((30, 20)), Hit::Magnifier, "{zoom:?}");
            assert_eq!(l.hit((60, 100)), Hit::Magnifier, "{zoom:?}");
        }
    }

    #[test]
    fn the_drivers_interface_stays_over_a_zoom_box_or_strip_and_takes_its_clicks() {
        // A screen of the driver's over the middle of the window, partly
        // over the box (1:3:1: 160..640 x 15..135).
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
        // Under the driver's screen placed in the view it is in (404..800 x
        // 0..297), which takes the clicks there.
        let mut l = arranged(&settings(|s| (s.split, s.zoom, s.zoom_place) = (S::Vertical1To1, Zoom::Floating131, Place::RightOrTop)));
        l.screen_place = ScreenPlace { vertical: Place::RightOrTop, horizontal: Place::Floating };
        let mut s = scene();
        s.interface.screen = Some("F1".into());
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let boxed = f.cameras.iter().position(|c| c.cells && c.zoom == (2, 1) && c.origin == (483, 15)).unwrap();
        assert!(f.placed.iter().all(|&at| at as usize > boxed), "{:?}", f.cameras);
        assert_eq!(l.hit((600, 60)), Hit::Screen);
    }

    #[test]
    fn title_and_setup_screens_ignore_saved_map_views_and_keep_full_window_input() {
        for split in [S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
            for zoom in [Zoom::Floating131, Zoom::TopStrip, Zoom::Mouse111] {
                for ui_in_pane in [false, true] {
                    let mut l = Layout::default();
                    l.apply_settings(&settings(|s| (s.split, s.zoom) = (split, zoom)));
                    l.ui_in_pane = ui_in_pane;
                    for name in [None, Some("Play New Game"), Some("Menu")] {
                        let mut menu = scene();
                        menu.shape = Default::default();
                        menu.interface.screen = name.map(str::to_owned);
                        let saved = (l.split, l.magnifier, l.ui_in_pane);
                        let f = l.frame(&menu, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                        assert_eq!(f.cameras, crate::scene_cameras(&menu, (800, 600), (0, 0, 0, 0)));
                        assert_eq!(f.view_cameras, [Some(0), Some(1)]);
                        assert!(f.markers.is_empty() && f.copies.is_empty() && f.placed.is_empty());
                        assert_eq!(f.menu_region, None);
                        assert_eq!(l.panes(), ([0, 0, 800, 600], None));
                        assert_eq!(l.pointer_walls(), None);
                        assert_eq!(l.capture_view(), None);
                        assert_eq!(l.screen_mapping(), ((0, 0), (1, 1)));
                        for p in [(400, 20), (401, 100), (700, 500)] {
                            assert_eq!(mv(&mut l, p), [Input::MouseMove { x: p.0, y: p.1 }]);
                            assert_eq!(l.route_window(p), Some(p));
                            assert_eq!(click(&mut l, p), [
                                Input::MouseButton { button: Button::Left, down: true, x: p.0, y: p.1 },
                                Input::MouseButton { button: Button::Left, down: false, x: p.0, y: p.1 },
                            ]);
                        }
                        assert_eq!((l.split, l.magnifier, l.ui_in_pane), saved);
                        assert_eq!(l.take_zooms_change(), None);
                        assert_eq!(l.take_views_change(), None);
                        assert_eq!(l.take_main_view_change(), None);
                    }
                    let game = frame(&mut l, vec![]);
                    assert!(game.cameras.len() > 2);
                    assert!(l.panes().1.is_some());
                    if !ui_in_pane { assert!(l.magnifier_shown); }
                }
            }
        }
    }

    #[test]
    fn returning_to_title_releases_game_capture_and_clears_view_reports() {
        let mut l = Layout::default();
        l.configure("split=vertical,pane=pointer,magnifier=world,ui=pane").unwrap();
        frame(&mut l, vec![]);
        l.route(Input::MouseMove { x: 600, y: 300 });
        click(&mut l, (600, 300));
        assert!(l.captured());
        assert_eq!(l.take_grab(), Some(true));
        l.take_main_view_change();
        assert!(l.take_views_change().is_some());
        assert!(l.take_zooms_change().is_some());
        let title = Scene { shape: Default::default(), ..scene() };
        let f = l.frame(&title, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.cameras.len(), title.views.len());
        assert!(!l.captured());
        assert_eq!(l.take_grab(), Some(false));
        assert_eq!(l.take_warp(), None);
        assert_eq!(l.pointer_walls(), None);
        assert_eq!(l.take_views_change(), Some([(0, 0, 0, 0); 4]));
        assert_eq!(l.take_zooms_change(), Some([(0, 0); 8]));
        assert_eq!(l.route_window((600, 300)), Some((600, 300)));
        assert_eq!(click(&mut l, (600, 300))[0], Input::MouseButton { button: Button::Left, down: true, x: 600, y: 300 });
        // Opening another game restores views without reviving capture.
        frame(&mut l, vec![]);
        assert!(l.panes().1.is_some());
        assert!(!l.captured());
        assert_eq!(l.take_grab(), None);
    }

    #[test]
    fn entering_an_optional_preview_releases_game_capture_and_gameplay_view_feedback() {
        for ready in [false, true] {
            let mut l = Layout::default();
            l.configure("split=vertical,pane=pointer,magnifier=world").unwrap();
            frame(&mut l, vec![]);
            click(&mut l, (600, 300));
            assert!(l.captured());
            l.take_grab();
            assert!(l.take_main_view_change().is_some());
            assert!(l.take_views_change().is_some());
            assert!(l.take_zooms_change().is_some());
            let mut preview = beside_scene(62);
            if !ready { preview.shape = Default::default(); }
            l.frame(&preview, (1920, 1080), (0, 0, 0, 0), vec![], BOUNDS);
            assert!(!l.captured());
            assert_eq!(l.take_grab(), Some(false));
            assert_eq!(l.take_warp(), None);
            assert_eq!(l.take_main_view_change(), Some((0, 0, 0, 0)));
            assert_eq!(l.take_views_change(), Some([(0, 0, 0, 0); 4]));
            assert_eq!(l.take_zooms_change(), Some([(0, 0); 8]));
            assert_eq!(l.pointer_walls(), None);
            let title = Scene { shape: Default::default(), ..scene() };
            l.frame(&title, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            assert_eq!(l.route_window((600, 300)), Some((600, 300)));
            assert_eq!(l.take_grab(), None);
            assert_eq!(l.take_views_change(), None);
            assert_eq!(l.take_zooms_change(), None);
            assert!(!l.captured());
        }
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
            assert_eq!(l.hit((400, 300)), Hit::Interface);
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
        // Below a box at the top (15..135) too.
        let mut l = arranged(&settings(|s| s.zoom = Zoom::Floating131));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!((screen_camera(&f).origin, screen_camera(&f).screen_shift), ((100, 139), (0, 39)));
        assert!(f.view_cameras[2].unwrap() as usize > f.cameras.iter().position(|c| c.zoom == (2, 1) && c.origin == (160, 15)).unwrap());
        assert_eq!(click(&mut l, (300, 300))[0], Input::MouseButton { button: Button::Left, down: true, x: 300, y: 261 });
        // No zoom box: where the driver put it.
        let mut l = arranged(&Settings::default());
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!((screen_camera(&f).origin, screen_camera(&f).screen_shift), ((100, 100), (0, 0)));
    }

    #[test]
    fn a_screen_goes_below_a_floating_zoom_box_its_height_and_clicks_with_it() {
        let mut s = with_screen("F1");
        let part = scene::ViewState { id: 258, view: scene::View { interface_only: true, origin: (420, 150), viewport: (300, 200), ..Default::default() }, overlay: vec![] };
        s.views.push(part);
        s.interface.screen_view = Some(258);
        let screen_camera = |f: &Frame| f.cameras[f.view_cameras[2].unwrap() as usize];
        // The box over B: 483..720 x 15.., `zoom_strip` of 600 high; the
        // screen (150..350) below it where they meet.
        for (strip, top, click_at, driver_y) in [(0.05, 150, 300, 300), (0.25, 169, 300, 281), (0.5, 319, 400, 231)] {
            let mut l = arranged(&settings(|s| (s.split, s.zoom, s.zoom_place, s.zoom_strip) = (S::Vertical1To1, Zoom::Floating131, Place::RightOrTop, strip)));
            let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            assert_eq!(l.magnifier_geometry(&l.magnifier.unwrap()).0, [483, 15, 237, (600.0 * strip) as i32]);
            assert_eq!((screen_camera(&f).origin, screen_camera(&f).screen_shift), ((420, top), (0, top - 150)), "zoom_strip {strip}");
            assert_eq!(click(&mut l, (500, click_at))[0], Input::MouseButton { button: Button::Left, down: true, x: 500, y: driver_y }, "zoom_strip {strip}");
        }
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
        s.screen_place = place(Place::LeftOrBottom, Place::RightOrTop);
        let fitted = |s: &Settings, kind: &str| {
            let mut l = Layout::default();
            l.apply_settings(s);
            l.frame(&with_screen(kind), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            let f = l.frame(&with_screen(kind), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            assert_eq!(l.fit, None, "a named panel does not fit the whole game window");
            (l.placed_screen, f)
        };
        // Only the named interface is placed in A or B. Their world cameras
        // stay independent beneath it.
        let (fit, f) = fitted(&s, "F1");
        assert_eq!(fit, Some(Fit { hit: Hit::Main, source: [0, 0, 800, 600], origin: (0, 0), zoom: (1, 2) }));
        let a = camera_at(&f, (0, 0));
        let b = camera_at(&f, (404, 0));
        assert_eq!((rect(&a), a.zoom, a.x, a.y), ([0, 0, 400, 600], (1, 1), 1200, 500));
        assert_eq!((rect(&b), b.zoom), ([404, 0, 396, 600], (1, 1)));
        assert!(f.placed.iter().all(|&i| !f.cameras[i as usize].cells));
        // One setting for every screen the driver names (Menu included while
        // the place is not floating).
        for kind in ["F1", "F7", "City", "Palace", "Spy", "Help", "Menu", "Diplomacy"] {
            assert_eq!(fitted(&s, kind).0.map(|f| f.hit), Some(Hit::Main), "{kind}");
        }
        let right = Settings { screen_place: place(Place::RightOrTop, Place::Floating), ..s.clone() };
        let (fit, f) = fitted(&right, "City");
        assert_eq!(fit, Some(Fit { hit: Hit::Pane, source: [0, 0, 800, 600], origin: (404, 0), zoom: (99, 200) }));
        assert_eq!(camera_at(&f, (0, 0)), a);
        assert_eq!(camera_at(&f, (404, 0)), b);
        let floating = Settings { screen_place: place(Place::Floating, Place::Floating), ..s.clone() };
        assert_eq!(fitted(&floating, "Help").0, None);
        assert_eq!(fitted(&floating, "City").0, None);
        // The horizontal split uses the other orientation's place.
        let h = Settings { split: S::Horizontal1To1, ..s.clone() };
        assert_eq!(fitted(&h, "F1").0.map(|f| f.hit), Some(Hit::Main));
        assert_eq!(fitted(&h, "City").0.map(|f| f.hit), Some(Hit::Main));
        let h = Settings { split: S::Horizontal1To1, screen_place: place(Place::Floating, Place::LeftOrBottom), ..s.clone() };
        assert_eq!(fitted(&h, "F5").0.map(|f| f.hit), Some(Hit::Pane));
        let one = Settings { split: S::Standard, ..s };
        assert_eq!(fitted(&one, "F1").0, None);
    }

    #[test]
    fn a_screen_placed_over_b_takes_its_clicks_without_replacing_either_world_camera() {
        let mut s = settings(|s| s.split = S::Vertical1To1);
        s.screen_place = ScreenPlace { vertical: Place::RightOrTop, horizontal: Place::Floating };
        let mut l = Layout::default();
        l.apply_settings(&s);
        let before = l.frame(&scene(), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let f = l.frame(&with_screen("F4"), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(camera_at(&before, (0, 0)), camera_at(&f, (0, 0)));
        assert_eq!(camera_at(&before, (404, 0)), camera_at(&f, (404, 0)));
        assert_eq!(click(&mut l, (503, 100))[0], Input::MouseButton { button: Button::Left, down: true, x: 200, y: 202 });
        // B keeps its own normal world extent and A keeps being the main view.
        assert_eq!(l.take_views_change().unwrap()[0], (1202, 500, 396, 600));
        assert!(l.main_unmoved());
        // Below the menu there is still B's world, not the original driver's
        // empty margins or an interactive copy of its entire game screen.
        assert_ne!(l.hit((600, 500)), Hit::Screen);
    }

    fn mouse_view_menu() -> Scene {
        let mut s = with_screen("Menu");
        s.interface.screen_view = Some(258);
        s.views.push(ViewState { id: 258, view: View { interface_only: true, origin: (760, 440), viewport: (400, 200), ..Default::default() }, overlay: vec![scene::FreeSprite::default()] });
        s
    }

    #[test]
    fn a_cursor_mirrored_in_b_is_drawn_under_its_menu_and_the_pointer_in_a_on_top() {
        use viewer_settings::Resolution;
        let mut s = settings(|s| s.split = S::Vertical1To1);
        s.screen_place = ScreenPlace { vertical: Place::RightOrTop, horizontal: Place::Floating };
        let mut l = Layout::default();
        l.apply_settings(&s);
        let plan = crate::scaling::plan((800, 600), (800, 600), &Resolution::default());
        let menu = with_screen("F4");
        l.frame(&scene(), plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
        let mut f = l.frame(&menu, plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
        crate::scaling::arrange_panned(&plan, &menu, &mut f, &mut l.menu_pan, |_| Some([0, 0, 800, 600]));
        let a = f.cameras.iter().position(|c| c.cells && !c.stacked && c.origin == (0, 0)).unwrap();
        let b = f.cameras.iter().position(|c| c.cells && !c.stacked && c.origin.0 >= 400).unwrap();
        // The cursor's places: after both worlds, before the interface.
        assert_eq!(f.cursor_slots.len(), 2, "one each for A and B: {:?}", f.cameras);
        let (first, last) = (f.cursor_slots[0] as usize, *f.cursor_slots.last().unwrap() as usize);
        assert!(a < first && b < first);
        let ui: Vec<usize> = f.view_cameras.iter().zip(&menu.views).filter(|(_, v)| v.view.interface_only)
            .filter_map(|(c, _)| c.map(|c| c as usize)).chain(f.placed.iter().map(|&c| c as usize)).collect();
        assert!(!ui.is_empty() && ui.iter().all(|&c| c > last), "interface {ui:?}, slots {:?}", f.cursor_slots);
        // The world cursor drawn where the captured pointer is (A, on top)
        // and mirrored in B (in a slot under the menu).
        // (Both panes show about the same stretch of the world here.)
        let (ca, cb) = (f.cameras[a], f.cameras[b]);
        assert!((cb.x..cb.x + cb.viewport.0 as i32).contains(&(ca.x + 100)));
        let marks = crate::cursor::Marks { world: vec![(ca.x + 100, ca.y + 100)], input: Some((100, 100)), ..Default::default() };
        let art = crate::cursor::SetArt { set: tile_render::SpriteSetId::from_raw(1), size: (32, 32), hotspot: (1, 1), anchor: (-1, -1) };
        let overlay = f.cameras.len() as u32;
        for scale in [crate::cursor::Scale::default(), crate::cursor::Scale::new(48, 1)] {
            let c = crate::cursor::compose(art, &marks, &f.cameras, &f.cursor_slots, scale, plan.canvas(), overlay, (0, 0));
            let views: Vec<usize> = c.sprites.iter().map(|s| s.view.unwrap() as usize).collect();
            assert_eq!(views.len(), 2, "{scale:?} {:?}", c.sprites);
            assert!(views.iter().any(|v| f.cursor_slots.contains(&(*v as u32))), "mirror under the menu: {views:?}");
            assert!(views.iter().any(|&v| v >= overlay as usize), "pointer on top: {views:?}");
            assert!(c.slots.iter().all(|(slot, _)| (*slot as usize) > b && ui.iter().all(|&u| u > *slot as usize)));
        }
    }

    #[test]
    fn hotkey_menus_keep_the_opening_mouse_view_and_requested_size_until_closed() {
        use viewer_settings::Resolution;
        for pixel_scale in 1..=6 {
            for split in [S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
                for side in [Side::A, Side::B] {
                    let mut plan = crate::scaling::plan((3840, 2160), (1920, 1080), &Resolution { pixel_scale, ..Default::default() });
                    plan.menu.zoom = crate::scaling::ratio(1.5);
                    assert_eq!(plan.present.zoom, (pixel_scale, 1));
                    let mut l = Layout::default();
                    l.apply_settings(&settings(|s| s.split = split));
                    l.driver_screen = Some(plan.screen);
                    l.set_walls(Walls::Open);
                    let before = l.frame(&scene(), plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                    let (a, b) = l.panes();
                    let pane = if side == Side::A { a } else { b.unwrap() };
                    let center = (pane[0] + pane[2] / 2, pane[1] + pane[3] / 2);
                    mv(&mut l, center);
                    key(&mut l, VK_ESCAPE, true);
                    key(&mut l, VK_ESCAPE, false);
                    let menu = mouse_view_menu();
                    let mut f = l.frame(&menu, plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                    assert_eq!(l.placed_screen, None, "generic menus keep their size instead of fitting into a pane");
                    assert_eq!(f.menu_region, Some(pane), "{split:?}, scale {pixel_scale}");
                    for origin in [(a[0], a[1]), (b.unwrap()[0], b.unwrap()[1])] {
                        assert_eq!(camera_at(&f, origin), camera_at(&before, origin));
                    }
                    let arranged = crate::scaling::arrange_panned(&plan, &menu, &mut f, &mut l.menu_pan, |_| Some([760, 440, 400, 200]));
                    let ui = f.cameras[f.view_cameras[2].unwrap() as usize];
                    assert_eq!(ui.zoom, (3, 2));
                    assert_eq!(ui.viewport, (600, 300));
                    assert_eq!(ui.target_clip, Some([pane[0], pane[1], pane[0] + pane[2], pane[1] + pane[3]]));
                    assert_eq!(ui.viewport.0 * pixel_scale, 600 * pixel_scale, "physical UI width grows through x6");
                    l.set_interface_shown(arranged.shown);
                    let mapped = l.route_window(center).unwrap();
                    assert!((mapped.0 - 960).abs() <= 2 && (mapped.1 - 540).abs() <= 2, "{mapped:?}");
                    let other = if side == Side::A { b.unwrap() } else { a };
                    let other_point = (other[0] + other[2] / 2, other[1] + other[3] / 2);
                    assert!(!matches!(l.interface_at(other_point), Some(Hit::Interface | Hit::Moved(_) | Hit::Screen)), "the other view has no invisible menu hitbox");
                    mv(&mut l, other_point);
                    assert_eq!(l.frame(&menu, plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS).menu_region, Some(pane), "menu stays put while open");
                    let resized = (plan.canvas().0 + 200, plan.canvas().1 + 100);
                    let f = l.frame(&menu, resized, (0, 0, 0, 0), vec![], BOUNDS);
                    let (ra, rb) = l.panes();
                    assert_eq!(f.menu_region, Some(if side == Side::A { ra } else { rb.unwrap() }), "resizing keeps the opening view");
                    l.frame(&scene(), plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                    assert_eq!(l.menu_side, None);
                    mv(&mut l, other_point);
                    assert_eq!(l.frame(&menu, plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS).menu_region, Some(other), "the next menu uses the new mouse view");
                }
            }
        }
    }

    #[test]
    fn menus_and_hotkey_popups_use_the_captured_view_before_releasing_the_mouse() {
        for menu in [false, true] {
            let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
            mv(&mut l, (600, 300));
            frame(&mut l, vec![]);
            click(&mut l, (600, 300));
            assert!(l.captured());
            // The captured world cursor determines B even if the old
            // physical pointer or its last free side still says A.
            l.side = Some(Side::A);
            l.pointer = (100, 100);
            let scene = if menu { mouse_view_menu() } else { with_parts(Some(300)) };
            let f = l.frame(&scene, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            assert!(!l.captured());
            if menu {
                assert_eq!(f.menu_region, l.panes().1);
                assert_eq!(l.placed_screen, None);
                assert_eq!(l.menu_side, Some(Side::B));
            } else {
                assert_eq!(l.popup_side, Some(Side::B));
                let c = f.cameras[f.view_cameras[3].unwrap() as usize];
                assert!(inside(l.panes().1.unwrap(), (c.origin.0, c.origin.1)));
                assert!(inside(l.panes().1.unwrap(), (c.origin.0 + 299, c.origin.1 + 149)));
            }
        }
    }

    #[test]
    fn popup_mouse_view_survives_pixel_scale_menu_scale_and_pointer_crossing() {
        use viewer_settings::Resolution;
        for pixel_scale in [1, 3, 6] {
            for menu_scale in [0.75, 1.0, 3.0] {
                for split in [S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
                    let mut plan = crate::scaling::plan((3840, 2160), (1920, 1080), &Resolution { pixel_scale, ..Default::default() });
                    plan.menu.zoom = crate::scaling::ratio(menu_scale);
                    let mut l = Layout::default();
                    l.apply_settings(&settings(|s| (s.split, s.roles) = (split, Roles::UnitMouse)));
                    l.driver_screen = Some(plan.screen);
                    l.set_walls(Walls::Open);
                    l.frame(&scene(), plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                    let (a, b) = l.panes();
                    let b = b.unwrap();
                    mv(&mut l, (b[0] + b[2] / 2, b[1] + b[3] / 2));
                    click(&mut l, (b[0] + b[2] / 2, b[1] + b[3] / 2));
                    assert!(l.captured());
                    let mut popup = scene();
                    popup.interface.popup = Some(257);
                    popup.views.push(ViewState { id: 257, view: View { interface_only: true, origin: (760, 440), viewport: (400, 200), ..Default::default() }, overlay: vec![scene::FreeSprite::default()] });
                    for crossing in [false, true] {
                        if crossing { mv(&mut l, (a[0] + a[2] / 2, a[1] + a[3] / 2)); }
                        let mut f = l.frame(&popup, plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                        assert_eq!(f.popup_region, Some(b));
                        let arranged = crate::scaling::arrange_panned(&plan, &popup, &mut f, &mut l.menu_pan, |_| Some([760, 440, 400, 200]));
                        let c = f.cameras[f.view_cameras[2].unwrap() as usize];
                        let center = (c.origin.0 + c.viewport.0 as i32 / 2, c.origin.1 + c.viewport.1 as i32 / 2);
                        assert!(inside(b, center), "{split:?}, pixel {pixel_scale}, menu {menu_scale}: {c:?} in {b:?}");
                        // Captured opening focuses the driver's centre;
                        // finalize it through the actual scaled hit map.
                        if !crossing { assert_eq!(l.focus_after_scale, Some((960, 540))); }
                        l.set_interface_shown(arranged.shown);
                        if !crossing {
                            let focused = l.take_warp().unwrap();
                            assert!((focused.0 - center.0).abs() <= 1 && (focused.1 - center.1).abs() <= 1, "{focused:?} vs {center:?}");
                        }
                        let q = l.route_window(center).unwrap();
                        assert!((q.0 - 960).abs() <= 2 && (q.1 - 540).abs() <= 2, "{q:?}");
                    }
                    l.frame(&scene(), plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                    assert_eq!(l.popup_side, None);
                }
            }
        }
    }

    #[test]
    fn mouse_view_menus_pan_all_corners_without_changing_pixels_or_covering_the_other_world() {
        use tile_render::{AtlasBuilder, Grid, LayerSpec, PlacedSprite, RgbaImage, SpriteSets, TileWorld};
        use viewer_settings::Resolution;
        const GREEN: [u8; 4] = [16, 128, 32, 255];
        let mut atlas = AtlasBuilder::new(256);
        let green = atlas.add(&RgbaImage::from_fn(8, 8, |_, _| GREEN), (0, 0)).unwrap();
        let panel = atlas.add(&RgbaImage::from_fn(128, 96, |x, y| [240, ((x * 3) % 251) as u8, ((y * 7) % 251) as u8, 255]), (0, 0)).unwrap();
        let atlas = atlas.finish();
        let mut sets = SpriteSets::default();
        let (green, panel) = (sets.single(green), sets.single(panel));
        let mut world = TileWorld::new(Grid { cols: 512, rows: 512, layout: tile_render::Layout::Orthogonal, wrap_x: false, wrap_y: false, step: (8, 8) }, sets);
        world.add_layer(LayerSpec::new("ground", vec![green]));
        for pixel_scale in 1..=6 {
            // The separate matrix uses real 4K canvases. This pixel oracle
            // uses a smaller equivalent viewport to compare every pixel.
            let mut plan = crate::scaling::plan((3840, 2160), (192, 108), &Resolution { pixel_scale, ..Default::default() });
            plan.menu.zoom = crate::scaling::ratio(1.5);
            plan.present.canvas = (384 / pixel_scale, 216 / pixel_scale);
            let mut l = Layout::default();
            l.apply_settings(&settings(|s| s.split = S::Vertical1To1));
            l.driver_screen = Some(plan.screen);
            l.set_walls(Walls::Open);
            l.frame(&scene(), plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
            let pane = l.panes().0;
            let center = (pane[0] + pane[2] / 2, pane[1] + pane[3] / 2);
            mv(&mut l, center);
            let mut menu = with_screen("Menu");
            menu.interface.screen_view = Some(258);
            menu.views.push(ViewState { id: 258, view: View { interface_only: true, origin: (32, 6), viewport: (128, 96), ..Default::default() }, overlay: vec![scene::FreeSprite::default()] });
            for delta in [(0, 0), (10000, 10000), (-10000, 10000), (10000, -10000), (-10000, -10000)] {
                if delta != (0, 0) {
                    let button = |down| Input::MouseButton { button: Button::Middle, down, x: center.0, y: center.1 };
                    let dragging = l.menu_pan.route(&button(true));
                    if dragging {
                        assert!(l.menu_pan.route(&Input::MouseMove { x: center.0 + delta.0, y: center.1 + delta.1 }));
                        assert!(l.menu_pan.route(&button(false)));
                    }
                }
                let mut f = l.frame(&menu, plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                let arranged = crate::scaling::arrange_panned(&plan, &menu, &mut f, &mut l.menu_pan, |_| Some([32, 6, 128, 96]));
                l.set_interface_shown(arranged.shown);
                let id = f.view_cameras[2].unwrap();
                assert_eq!(f.cameras[id as usize].zoom, (3, 2));
                world.free = vec![PlacedSprite { screen: true, view: Some(id), ..PlacedSprite::new(panel, 32, 6) }];
                let clipped = tile_render::reference::render_views(&world, &atlas, &f.cameras, plan.canvas(), 0, [0, 0, 0, 255]);
                let mut unconfined = f.cameras.clone();
                unconfined[id as usize].target_clip = None;
                let whole = tile_render::reference::render_views(&world, &atlas, &unconfined, plan.canvas(), 0, [0, 0, 0, 255]);
                for y in 0..plan.canvas().1 {
                    for x in 0..plan.canvas().0 {
                        if inside(pane, (x as i32, y as i32)) {
                            assert_eq!(clipped.get(x, y), whole.get(x, y), "clip must preserve fractional sampling at {x},{y}, x{pixel_scale}");
                        } else if x as i32 >= l.panes().1.unwrap()[0] {
                            assert_eq!(clipped.get(x, y), GREEN, "B remains its own world, x{pixel_scale}, delta {delta:?}");
                        }
                    }
                }
                if delta != (0, 0) {
                    let corner = [if delta.0 > 0 { 32 } else { 159 }, if delta.1 > 0 { 6 } else { 101 }, 1, 1];
                    let control = l.driver_rect_to_window(corner);
                    assert!(inside(pane, (control[0].floor() as i32, control[1].floor() as i32)), "every corner can be reached by panning, x{pixel_scale}, delta {delta:?}: {control:?} in {pane:?}");
                    let q = l.route_window((control[0].ceil() as i32, control[1].ceil() as i32)).unwrap();
                    assert!((q.0 - corner[0]).abs() <= 1 && (q.1 - corner[1]).abs() <= 1, "panned control hit map {q:?} vs {corner:?}");
                }
            }
        }
    }

    #[test]
    fn a_menu_over_city_moves_only_the_active_menu_panel() {
        let mut settings = settings(|s| s.split = S::Vertical1To1);
        settings.menu_follows_pointer = false;
        settings.screen_place = ScreenPlace { vertical: Place::RightOrTop, horizontal: Place::Floating };
        let mut l = Layout::default();
        l.apply_settings(&settings);
        l.driver_screen = Some((1920, 1080));
        let before = l.frame(&scene(), (1280, 720), (0, 0, 0, 0), vec![], BOUNDS);
        let mut s = city_screen();
        s.shape = scene().shape;
        s.views[2].view.menu = true;
        s.interface.screen = Some("Menu".into());
        s.interface.screen_view = Some(258);
        s.views.push(ViewState { id: 258, view: View { interface_only: true, origin: (448, 156), viewport: (1024, 768), ..Default::default() }, overlay: vec![] });
        let f = l.frame(&s, (1280, 720), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.view_cameras[1], None, "the underlying City's map is not part of Preferences");
        assert_eq!(f.view_cameras[2], None, "an inactive menu is not copied into the active panel");
        assert!(f.placed.iter().all(|&i| !f.cameras[i as usize].cells));
        assert_eq!(l.placed_screen.unwrap().source, [448, 156, 1024, 768]);
        assert_eq!(camera_at(&before, (0, 0)), camera_at(&f, (0, 0)));
        assert_eq!(camera_at(&before, (644, 0)), camera_at(&f, (644, 0)));
    }

    #[test]
    fn a_mouse_view_menu_over_city_moves_only_the_active_ui_at_requested_scale() {
        let mut l = Layout::default();
        l.apply_settings(&settings(|s| s.split = S::Vertical1To1));
        l.driver_screen = Some((1920, 1080));
        l.set_walls(Walls::Open);
        l.frame(&scene(), (1280, 720), (0, 0, 0, 0), vec![], BOUNDS);
        mv(&mut l, (960, 360));
        let mut s = city_screen();
        s.shape = scene().shape;
        s.views[2].view.menu = true;
        s.interface.screen = Some("Menu".into());
        s.interface.screen_view = Some(258);
        s.views.push(ViewState { id: 258, view: View { interface_only: true, origin: (448, 156), viewport: (1024, 768), ..Default::default() }, overlay: vec![scene::FreeSprite::default()] });
        let mut f = l.frame(&s, (1280, 720), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.view_cameras[1], None, "Preferences does not copy the underlying City's map");
        assert_eq!(f.view_cameras[2], None, "the underlying City UI does not join Preferences");
        assert_eq!(f.menu_region, l.panes().1);
        assert_eq!(l.shown.iter().filter(|(hit, _)| matches!(hit, Hit::Main | Hit::Pane)).count(), 2);
        assert!(f.placed.is_empty());
        let mut plan = crate::scaling::plan((3840, 2160), (1920, 1080), &viewer_settings::Resolution { pixel_scale: 3, ..Default::default() });
        plan.menu.zoom = crate::scaling::ratio(1.25);
        let arranged = crate::scaling::arrange_panned(&plan, &s, &mut f, &mut l.menu_pan, |_| Some([448, 156, 1024, 768]));
        let ui = f.cameras[f.view_cameras[3].unwrap() as usize];
        assert_eq!(ui.zoom, (5, 4));
        assert_eq!(ui.viewport, (1280, 960));
        l.set_interface_shown(arranged.shown);
        let b = l.panes().1.unwrap();
        let q = l.route_window((b[0] + b[2] / 2, b[1] + b[3] / 2)).unwrap();
        assert_eq!(q, (960, 540));
    }

    #[test]
    fn placed_menu_pixels_preserve_both_maps_and_citys_internal_map() {
        use tile_render::{AtlasBuilder, Grid, LayerSpec, PlacedSprite, RgbaImage, SpriteSets, TileWorld};
        const GREEN: [u8; 4] = [20, 180, 40, 255];
        const RED: [u8; 4] = [220, 30, 40, 255];
        const BLUE: [u8; 4] = [30, 50, 220, 255];
        for city in [false, true] {
            let mut atlas = AtlasBuilder::new(64);
            let green = atlas.add(&RgbaImage::from_fn(8, 8, |_, _| GREEN), (0, 0)).unwrap();
            let panel = atlas.add(&RgbaImage::from_fn(32, 24, |x, y| {
                let clear = if city { (4..20).contains(&y) } else { x < 4 && y < 4 };
                if clear { [0; 4] } else { RED }
            }), (0, 0)).unwrap();
            let blue = atlas.add(&RgbaImage::from_fn(32, 16, |_, _| BLUE), (0, 0)).unwrap();
            let atlas = atlas.finish();
            let mut sets = SpriteSets::default();
            let (green, panel, blue) = (sets.single(green), sets.single(panel), sets.single(blue));
            let mut world = TileWorld::new(Grid { cols: 512, rows: 512, layout: tile_render::Layout::Orthogonal, wrap_x: false, wrap_y: false, step: (8, 8) }, sets);
            world.add_layer(LayerSpec::new("ground", vec![green]));
            let name = if city { "City" } else { "F1" };
            let mut settings = settings(|s| s.split = S::Vertical1To1);
            settings.screen_place = ScreenPlace { vertical: Place::LeftOrBottom, horizontal: Place::Floating };
            let mut l = Layout::default();
            l.apply_settings(&settings);
            l.driver_screen = Some((80, 60));
            let mut s = scene();
            s.interface.screen = Some(name.into());
            if city {
                s.views[0].view.interface_only = true;
                s.views.push(ViewState { id: 1, view: View { camera_x: 1024, camera_y: 522, origin: (24, 22), viewport: (32, 16), ..Default::default() }, overlay: vec![] });
            } else {
                s.interface.screen_view = Some(258);
                s.views.push(ViewState { id: 258, view: View { interface_only: true, origin: (24, 18), viewport: (32, 24), ..Default::default() }, overlay: vec![] });
            }
            let f = l.frame(&s, (80, 60), (0, 0, 0, 0), vec![[24, 18, 32, 24]], BOUNDS);
            let panel_view = f.view_cameras[if city { 1 } else { 2 }].unwrap();
            world.free.push(PlacedSprite { screen: true, view: Some(panel_view), clip: Some([24, 18, 56, 42]), ..PlacedSprite::new(panel, 24, 18) });
            if city {
                // The City's own map view goes under its transparent band.
                world.free.insert(0, PlacedSprite { screen: true, view: f.view_cameras[2], ..PlacedSprite::new(blue, 24, 22) });
            }
            let image = tile_render::reference::render_views(&world, &atlas, &f.cameras, (80, 60), 0, [0, 0, 0, 255]);
            assert_eq!(image.get(10, 16), RED, "{name}: fitted panel top");
            assert_eq!(image.get(10, 43), RED, "{name}: fitted panel bottom");
            assert_eq!(image.get(0, 14), GREEN, "{name}: A remains visible above the panel");
            assert_eq!(image.get(50, 20), GREEN, "{name}: B contains its own world instead of the panel at its old coordinates");
            assert_eq!(image.get(79, 59), GREEN, "{name}: B remains visible outside the panel");
            if city {
                assert_eq!(image.get(10, 24), BLUE, "City map is transformed with the panel");
            } else {
                assert_eq!(image.get(0, 15), GREEN, "the UI group preserves transparent menu pixels");
            }
        }
    }

    #[test]
    fn named_city_and_advisor_panels_fill_their_pane_without_the_driver_margins() {
        use viewer_settings::Resolution;
        const SOURCE: [i32; 4] = [448, 156, 1024, 768];
        for pixel_scale in [1, 3, 6] {
            for split in [S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
                for place in [Place::LeftOrBottom, Place::RightOrTop] {
                    for city in [false, true] {
                        let name = if city { "City" } else { "F1" };
                        let mut settings = settings(|s| s.split = split);
                        settings.screen_place = ScreenPlace { vertical: place, horizontal: place };
                        let mut plan = crate::scaling::plan((3840, 2160), (1920, 1080), &Resolution { pixel_scale, ..Default::default() });
                        plan.menu.zoom = crate::scaling::ratio(3.0);
                        let mut l = Layout::default();
                        l.apply_settings(&settings);
                        l.driver_screen = Some(plan.screen);
                        let mut s = scene();
                        s.interface.screen = Some(name.into());
                        let interface = if city {
                            s.views[0].view.interface_only = true;
                            s.views.push(ViewState { id: 1, view: View { camera_x: 1448, camera_y: 748, origin: (448, 248), viewport: (1024, 416), ..Default::default() }, overlay: vec![] });
                            vec![[448, 156, 1024, 92], [448, 664, 1024, 260]]
                        } else {
                            s.interface.screen_view = Some(258);
                            s.views.push(ViewState { id: 258, view: View { interface_only: true, origin: (448, 156), viewport: (1024, 768), ..Default::default() }, overlay: vec![] });
                            // The original HUD exists outside the advisor's part.
                            vec![[0, 0, 160, 54], [0, 900, 400, 180]]
                        };
                        let before = l.frame(&scene(), plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                        let mut f = l.frame(&s, plan.canvas(), (0, 0, 0, 0), interface, BOUNDS);
                        let fit = l.placed_screen.unwrap();
                        assert_eq!(fit.source, SOURCE, "{name}, scale {pixel_scale}, {split:?}, {place:?}");
                        assert_eq!(l.fit, None);
                        let (a, b) = l.panes();
                        let pane = if fit.hit == Hit::Main { a } else { b.unwrap() };
                        let [x, y, w, h] = SOURCE;
                        let (n, d) = fit.zoom;
                        let size = ((w as u32 * n).div_ceil(d) as i32, (h as u32 * n).div_ceil(d) as i32);
                        assert!(size.0 == pane[2] || size.1 == pane[3], "the actual panel fills one pane dimension");
                        assert_eq!(fit.origin, (pane[0] + (pane[2] - size.0) / 2, pane[1] + (pane[3] - size.1) / 2));
                        for origin in [(a[0], a[1]), (b.unwrap()[0], b.unwrap()[1])] {
                            assert_eq!(camera_at(&before, origin), camera_at(&f, origin), "{name} preserves the world camera at {origin:?}");
                        }
                        assert_eq!(f.placed.iter().filter(|&&i| f.cameras[i as usize].cells).count(), usize::from(city));
                        if city {
                            let map = f.cameras[f.view_cameras[2].unwrap() as usize];
                            assert_eq!((map.origin, map.viewport, map.screen_shift), ((0, 92), (1024, 416), (-448, -156)));
                        }
                        // Each corner and the centre reach the corresponding
                        // content pixel, without including the old driver margins.
                        for (dx, dy) in [(0, 0), (size.0 - 1, 0), (0, size.1 - 1), (size.0 - 1, size.1 - 1), (size.0 / 2, size.1 / 2)] {
                            let p = (fit.origin.0 + dx, fit.origin.1 + dy);
                            let q = l.route_window(p).unwrap();
                            assert!(inside(SOURCE, q));
                            assert_eq!(q, (x + dx * d as i32 / n as i32, y + dy * d as i32 / n as i32));
                        }
                        if pixel_scale == 3 && split == S::Vertical1To1 && place == Place::LeftOrBottom {
                            assert_eq!((fit.origin, fit.zoom), ((0, 120), (5, 8)));
                            assert_eq!(click(&mut l, (320, 360))[0], Input::MouseButton { button: Button::Left, down: true, x: 960, y: 540 });
                            assert_eq!(l.driver_rect_to_window([896, 492, 128, 96]), [280.0, 330.0, 80.0, 60.0]);
                        }
                        // The general menu scale must not shrink or enlarge an
                        // already fitted panel or start oversized-menu panning.
                        let placed: Vec<_> = f.placed.iter().map(|&i| f.cameras[i as usize]).collect();
                        let arranged = crate::scaling::arrange_panned(&plan, &s, &mut f, &mut l.menu_pan, |_| None);
                        assert!(arranged.shown.is_some());
                        assert_eq!(f.placed.iter().map(|&i| f.cameras[i as usize]).collect::<Vec<_>>(), placed);
                        assert!(!l.menu_pan.route(&Input::MouseButton { button: Button::Middle, down: true, x: fit.origin.0, y: fit.origin.1 }));
                        l.frame(&scene(), plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                        assert_eq!(l.placed_screen, None);
                    }
                }
            }
        }
    }

    #[test]
    fn top_strips_hold_the_pointer_at_the_map_and_scroll_up_in_every_layout() {
        for split in [S::Standard, S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
            for place in [Place::Floating, Place::LeftOrBottom, Place::RightOrTop] {
                for walls in [Walls::Warp, Walls::Confined] {
                    let mut l = arranged(&settings(|s| (s.split, s.zoom, s.zoom_place) = (split, Zoom::TopStrip, place)));
                    l.set_walls(walls);
                    let (strip, _) = l.strip().unwrap();
                    let x = strip[0] + strip[2] / 3;
                    let boundary = strip[1] + strip[3] + GAP;
                    let (a, b) = l.panes();
                    let hit = if inside(a, (x, boundary)) { Hit::Main } else { Hit::Pane };
                    let pane = if hit == Hit::Main { a } else { b.unwrap() };
                    mv(&mut l, (x, boundary + pane[3] / 2));
                    l.take_warp();
                    let before = if hit == Hit::Main { l.main_view.center } else { l.split.unwrap().view.center };
                    let out = mv(&mut l, (x, strip[1] + strip[3] / 2));
                    assert_eq!(l.pointer, (x, boundary), "{split:?} {place:?} {walls:?}");
                    assert_eq!(l.take_warp(), (walls == Walls::Warp).then_some((x, boundary)));
                    assert!(l.pointer_walls().unwrap().iter().any(|&r| inside(r, (x, boundary - 1))));
                    assert!(!out.iter().any(|i| matches!(i, Input::Edge { .. })), "{out:?}");
                    l.advance(std::time::Duration::from_millis(100));
                    frame(&mut l, vec![]);
                    let after = if hit == Hit::Main { l.main_view.center } else { l.split.unwrap().view.center };
                    assert_eq!(after.1, before.1 - 60, "both panes scroll up locally: {split:?} {place:?} {hit:?}");
                    mv(&mut l, (x, boundary + 15));
                    l.advance(std::time::Duration::from_millis(100));
                    frame(&mut l, vec![]);
                    let slow = if hit == Hit::Main { l.main_view.center } else { l.split.unwrap().view.center };
                    assert_eq!(slow.1, after.1 - 15);
                    mv(&mut l, (x, boundary + 16));
                    l.advance(std::time::Duration::from_millis(100));
                    frame(&mut l, vec![]);
                    let stopped = if hit == Hit::Main { l.main_view.center } else { l.split.unwrap().view.center };
                    assert_eq!(stopped.1, slow.1);
                }
            }
        }
    }

    #[test]
    fn zoom_pointer_walls_leave_interface_controls_reachable() {
        let mut l = arranged(&settings(|s| s.zoom = Zoom::TopStrip));
        let control = [100, 50, 80, 30];
        frame(&mut l, vec![control]);
        let walls = l.pointer_walls().unwrap();
        assert!(walls.iter().all(|&r| !overlap(r, control)));
        // Confinement needs a connected path from map to control.
        assert!(walls.iter().all(|&r| !inside(r, (140, 120))));
        mv(&mut l, (140, 300));
        mv(&mut l, (140, 60));
        assert_eq!(l.pointer, (140, 60));
        assert_eq!(click(&mut l, (140, 60))[0], Input::MouseButton { button: Button::Left, down: true, x: 140, y: 60 });
        l.set_snapping(true);
        assert_eq!(l.pointer_walls(), None);
        mv(&mut l, (300, 60));
        assert_eq!(l.pointer, (300, 60));
    }

    #[test]
    fn floating_boxes_sit_at_the_top_then_drop_below_the_pointer_or_block_entry() {
        for split in [S::Standard, S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
            for place in [Place::Floating, Place::LeftOrBottom, Place::RightOrTop] {
                for zoom in [Zoom::Floating131, Zoom::Floating171] {
                    let mut s = settings(|s| (s.split, s.zoom, s.zoom_place) = (split, zoom, place));
                    let mut l = arranged(&s);
                    let m = l.magnifier.unwrap();
                    let initial = l.magnifier_geometry(&m).0;
                    let region = l.zoom_region(&m);
                    // 2.5 % of its region's height below its top, 20 % high.
                    let part = |thousandths: i64| (region[3] as i64 * thousandths / 1000) as i32;
                    assert_eq!(initial, [region[0] + (region[2] - initial[2]) / 2, region[1] + part(25), initial[2], part(200)]);
                    let p = (initial[0] + initial[2] / 3, initial[1] + initial[3] - 1);
                    // Start in this region without crossing a divider wall.
                    l.place_pointer((p.0, initial[1] + initial[3] + GAP));
                    l.take_warp();
                    mv(&mut l, p);
                    // Its top 30 % of the region's height below the pointer.
                    let moved = l.magnifier_geometry(&m).0;
                    assert_eq!(moved, [initial[0], p.1 + part(300), initial[2], initial[3]]);
                    assert_eq!(l.pointer, p);
                    assert!(!inside(moved, p));
                    assert!(l.zoom_pointer_walls().is_empty());
                    for walls in [Walls::Warp, Walls::Confined] {
                        s.zoom_box_block_pointer = true;
                        let mut l = arranged(&s);
                        l.set_walls(walls);
                        l.place_pointer((p.0, initial[1] + initial[3] + GAP));
                        l.take_warp();
                        mv(&mut l, p);
                        assert_eq!(l.pointer, (p.0, initial[1] + initial[3] + GAP / 2));
                        assert_eq!(l.magnifier_geometry(&m).0, initial);
                        assert!(!l.zoom_pointer_walls().is_empty());
                        assert_eq!(l.take_warp(), (walls == Walls::Warp).then_some(l.pointer));
                        let out = click(&mut l, p);
                        assert!(out.iter().all(|e| !matches!(e, Input::MouseButton { y, .. } if *y == p.1)), "raw click must use held pointer: {out:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_map_drag_cannot_enter_a_reserved_top_strip() {
        for walls in [Walls::Warp, Walls::Confined] {
            let mut l = arranged(&settings(|s| s.zoom = Zoom::TopStrip));
            l.set_walls(walls);
            mv(&mut l, (300, 300));
            l.route(Input::MouseButton { button: Button::Middle, down: true, x: 300, y: 300 });
            let out = mv(&mut l, (300, 50));
            assert_eq!(l.pointer, (300, 124));
            assert_eq!(l.raw_pointer, (300, 50));
            assert_eq!(l.take_warp(), (walls == Walls::Warp).then_some((300, 124)));
            assert!(out.iter().all(|e| !matches!(e, Input::MouseMove { y: 50, .. })));
            l.route(Input::MouseButton { button: Button::Middle, down: false, x: 300, y: 50 });
        }
    }

    #[test]
    fn a_blocked_box_moves_clear_of_a_captured_world_cursor() {
        let mut l = arranged(&settings(|s| {
            (s.split, s.roles, s.zoom, s.zoom_place, s.zoom_box_block_pointer) = (S::Vertical1To1, Roles::UnitMouse, Zoom::Floating131, Place::RightOrTop, true);
        }));
        mv(&mut l, (750, 500));
        click(&mut l, (750, 500));
        assert!(l.captured());
        frame(&mut l, vec![]);
        assert!(!inside(l.magnifier_geometry(&l.magnifier.unwrap()).0, l.cursor_on_window().unwrap()));
        assert_eq!(l.pointer_walls(), None);
        key(&mut l, VK_CONTROL, true);
        key(&mut l, VK_CONTROL, false);
        assert!(!l.captured());
        assert!(!inside(l.magnifier_geometry(&l.magnifier.unwrap()).0, l.pointer));
        assert!(!l.zoom_pointer_walls().is_empty());
    }

    #[test]
    fn zoom_walls_stop_fast_crossings_and_keep_clicks_on_the_held_map_pixel() {
        let mut l = arranged(&settings(|s| {
            (s.zoom, s.zoom_box_block_pointer) = (Zoom::Floating131, true);
        }));
        // (Across the box at the top, 15..135.)
        mv(&mut l, (100, 60));
        mv(&mut l, (700, 60));
        assert_eq!(l.pointer, (157, 60));
        assert_eq!(l.take_warp(), Some((157, 60)));
        assert_eq!(click(&mut l, (700, 60))[0], Input::MouseButton { button: Button::Left, down: true, x: 157, y: 60 });
        assert_eq!(l.route(Input::Wheel { delta: 120, x: 700, y: 60 }), [Input::Wheel { delta: 120, x: 157, y: 60 }]);
    }

    #[test]
    fn a_zoom_wall_keeps_the_pointer_on_its_original_side_of_an_open_divider() {
        let mut l = arranged(&settings(|s| {
            (s.split, s.zoom, s.zoom_box_block_pointer) = (S::Vertical1To1, Zoom::Floating131, true);
        }));
        mv(&mut l, (100, 60));
        mv(&mut l, (700, 60));
        assert_eq!((l.pointer, l.side), ((157, 60), Some(Side::A)));
    }

    #[test]
    fn even_the_tallest_floating_boxes_move_clear_of_the_free_and_captured_pointer() {
        for (height, top) in [(0.05, 0.0), (0.2, 0.025), (0.5, 0.2)] {
            let mut l = arranged(&settings(|s| (s.split, s.roles, s.zoom, s.zoom_place, s.zoom_strip, s.zoom_box_top) = (S::Vertical1To1, Roles::UnitMouse, Zoom::Floating171, Place::RightOrTop, height, top)));
            let at = (600, 20 + (600.0 * top) as i32);
            l.place_pointer(at);
            frame(&mut l, vec![]);
            assert!(!inside(l.magnifier_geometry(&l.magnifier.unwrap()).0, l.pointer));
            click(&mut l, at);
            assert!(l.captured());
            frame(&mut l, vec![]);
            assert!(!inside(l.magnifier_geometry(&l.magnifier.unwrap()).0, l.cursor_on_window().unwrap()));
            assert_eq!(l.pointer_walls(), None);
        }
    }

    // ---- The divider ------------------------------------------------------

    fn split_1to1() -> Layout {
        arranged(&settings(|s| s.split = S::Vertical1To1))
    }

    #[test]
    fn the_divider_is_open_in_its_middle_third_and_drawn_without_glass_there() {
        let mut l = split_1to1();
        assert_eq!(l.gap(), Some(((400, 404), (0, 600))));
        assert_eq!(l.open_spans(), [(200, 400)]);
        let f = frame(&mut l, vec![]);
        let fillers: Vec<_> = f.cameras.iter().filter(|c| c.viewport.0 == 2).map(|c| (c.origin, c.viewport, (c.x, c.y))).collect();
        // A's picture goes on two pixels into the gap, B's from the other
        // side, the same world pixels as next to them, the whole gap long
        // (under the glass too); no glass over the open third.
        let b = camera_at(&f, (404, 0));
        assert_eq!(fillers, [((400, 0), (2, 600), (1600, 500)), ((402, 0), (2, 600), (b.x - 2, b.y))]);
        assert!(f.divider.iter().all(|d| d.1[1] + d.1[3] <= 200 || d.1[1] >= 400), "{:?}", f.divider);
        // Horizontal: the same across.
        let mut h = arranged(&settings(|s| s.split = S::Horizontal1To1));
        assert_eq!(h.open_spans(), [(266, 534)]);
        let f = frame(&mut h, vec![]);
        assert!(f.cameras.iter().any(|c| c.origin == (0, 300) && c.viewport == (800, 2)), "{:?}", f.cameras);
    }

    #[test]
    fn the_divider_holds_the_pointer_back_and_scrolls_the_view_instead() {
        let mut l = split_1to1();
        mv(&mut l, (100, 100));
        assert_eq!(l.side, Some(Side::A));
        // Across the closed upper third: held at A's last pixel, the
        // system pointer moved back. The viewer scrolls A locally at
        // the same speed as B.
        let out = mv(&mut l, (600, 100));
        assert_eq!(out, [Input::MouseMove { x: 599, y: 100 }]);
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
        assert_eq!(mv(&mut l, (300, 100)), [Input::MouseMove { x: 500, y: 100 }]);
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
        l.menu_follows_pointer = false;
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
    fn the_walls_the_system_holds_the_pointer_out_of_are_the_gaps_closed_stretches() {
        let mut l = split_1to1();
        assert_eq!(l.pointer_walls(), Some(vec![[400, 0, 4, 200], [400, 400, 4, 200]]));
        // Always reachable unit controls open a gap with the same margin
        // as pointer routing, even when only part overlaps the divider.
        l.reachable = Some([398, 100, 20, 30]);
        assert_eq!(l.pointer_walls(), Some(vec![[400, 0, 4, 96], [400, 134, 4, 66], [400, 400, 4, 200]]));
        l.reachable = None;
        let h = arranged(&settings(|s| s.split = S::Horizontal1To1));
        assert_eq!(h.pointer_walls(), Some(vec![[0, 300, 266, 4], [534, 300, 266, 4]]));
        // None where the divider holds nothing back.
        assert_eq!(arranged(&settings(|_| {})).pointer_walls(), None);
        l.locked = true;
        assert_eq!(l.pointer_walls(), None);
        l.locked = false;
        l.set_snapping(true);
        assert_eq!(l.pointer_walls(), None);
        l.set_snapping(false);
        l.drag = Some(Drag::Divider);
        assert_eq!(l.pointer_walls(), None);
    }

    #[test]
    fn the_walls_the_system_holds_open_where_the_drivers_interface_lies_over_the_divider() {
        let mut l = split_1to1();
        let mut s = with_parts(None);
        let part = |id, origin, viewport| scene::ViewState { id, view: scene::View { interface_only: true, origin, viewport, ..Default::default() }, overlay: vec![] };
        s.views.push(part(257, (300, 450), (450, 100)));
        s.interface.popup = Some(257);
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![[200, 20, 400, 100], [100, 250, 250, 300]], BOUNDS);
        // (The rectangle on A's side only leaves the wall as it is.)
        assert_eq!(l.pointer_walls(), Some(vec![[400, 0, 4, 20], [400, 120, 4, 80], [400, 400, 4, 50], [400, 550, 4, 50]]));
    }

    #[test]
    fn where_the_system_holds_the_pointer_the_divider_does_not_move_it_back() {
        let mut l = split_1to1();
        l.set_walls(Walls::Confined);
        mv(&mut l, (100, 100));
        // Held by the system at A's last pixel: it pushes against the wall.
        assert_eq!(mv(&mut l, (399, 100)), [Input::MouseMove { x: 599, y: 100 }]);
        assert_eq!((l.side, l.against, l.take_warp()), (Some(Side::A), Some(Side::A), None));
        // A pixel into the wall (the system's rounding): held there, not
        // moved back.
        mv(&mut l, (400, 120));
        assert_eq!((l.pointer, l.against, l.take_warp()), ((399, 120), Some(Side::A), None));
        // Through the open middle third as ever.
        mv(&mut l, (300, 300));
        mv(&mut l, (600, 300));
        assert_eq!((l.pointer, l.side), ((600, 300), Some(Side::B)));
        // The system not holding it (yet): the pointer passes.
        mv(&mut l, (600, 100));
        l.set_walls(Walls::Open);
        mv(&mut l, (100, 100));
        assert_eq!((l.pointer, l.side, l.take_warp()), ((100, 100), Some(Side::A), None));
        // Placing the pointer still moves the system pointer.
        l.set_walls(Walls::Confined);
        l.place_pointer((250, 250));
        assert_eq!(l.take_warp(), Some((250, 250)));
    }

    #[test]
    fn a_completed_system_placement_replaces_motion_received_while_pending() {
        let mut l = split_1to1();
        l.set_walls(Walls::Confined);
        mv(&mut l, (100, 100));
        l.place_pointer((600, 100));
        assert_eq!(l.take_warp(), Some((600, 100)));
        l.set_walls(Walls::Open);
        mv(&mut l, (120, 100));
        l.set_walls(Walls::Confined);
        l.system_pointer_placed((600, 100));
        assert_eq!((l.pointer, l.raw_pointer, l.side, l.take_warp()), ((600, 100), (600, 100), Some(Side::B), None));
        // A completed placement at the physical boundary also establishes
        // edge scrolling without waiting for another real motion event.
        l.system_pointer_placed((404, 100));
        assert_eq!((l.against, l.take_warp(), l.walls), (Some(Side::B), None, Walls::Confined));
    }

    #[test]
    fn a_stationary_confined_pointer_keeps_scrolling_both_sides() {
        let mut l = split_1to1();
        l.set_walls(Walls::Confined);
        mv(&mut l, (100, 100));
        mv(&mut l, (399, 100));
        let center = l.main_view.center;
        for i in 1..=2 {
            l.advance(std::time::Duration::from_millis(100));
            frame(&mut l, vec![]);
            assert_eq!(l.against, Some(Side::A));
            assert_eq!(l.driver_edge(l.pointer), (0, 0));
            assert_eq!(l.main_view.center, (center.0 + 60 * i, center.1));
            assert_eq!(l.take_warp(), None);
        }
        mv(&mut l, (300, 300));
        mv(&mut l, (600, 300));
        mv(&mut l, (600, 500));
        mv(&mut l, (404, 500));
        let center = l.split.unwrap().view.center;
        for i in 1..=2 {
            l.advance(std::time::Duration::from_millis(100));
            frame(&mut l, vec![]);
            assert_eq!(l.against, Some(Side::B));
            assert_eq!(l.split.unwrap().view.center, (center.0 - 60 * i, center.1));
            assert_eq!(l.take_warp(), None);
        }
    }

    #[test]
    fn spans_less_spans() {
        assert_eq!(subtract_spans((0, 10), &[]), [(0, 10)]);
        assert_eq!(subtract_spans((0, 10), &[(2, 4), (6, 20), (5, 5)]), [(0, 2), (4, 6)]);
        assert_eq!(subtract_spans((0, 10), &[(-5, 15)]), []);
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
        // No glass over their rows.
        assert!(f.divider.iter().all(|d| d.1[1] + d.1[3] <= 520 || d.1[1] >= 584), "{:?}", f.divider);
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
        // Not reaching over the gap (the strip cut at the divider, or all
        // in one view): their rows are open all the same, the pointer
        // crosses from one view to the other there, both ways.
        let mut s = scene();
        s.interface.reachable = Some([100, 520, 128, 64]);
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(l.open_spans(), [(200, 400), (520, 584)]);
        assert!(l.pointer_walls().unwrap_or_default().iter().all(|w| w[1] + w[3] <= 520 - GAP || w[1] >= 584 + GAP), "{:?}", l.pointer_walls());
        mv(&mut l, (300, 540));
        mv(&mut l, (500, 540));
        assert_eq!((l.pointer, l.side), ((500, 540), Some(Side::B)));
        mv(&mut l, (300, 570));
        assert_eq!((l.pointer, l.side), ((300, 570), Some(Side::A)));
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
        // With a zoom strip (120 high), in A's own area below it, not over
        // the strip; clicks there reach them where the driver has them.
        let mut l = arranged(&settings(|s| s.zoom = Zoom::TopStrip));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.cameras[f.view_cameras[2].unwrap() as usize].origin, (0, 124));
        assert_eq!(click(&mut l, (30, 144))[0], Input::MouseButton { button: Button::Left, down: true, x: 30, y: 20 });
        // The same in A as in B with the strip over both views.
        let mut l = arranged(&settings(|s| (s.split, s.zoom, s.zoom_place) = (S::Vertical1To1, Zoom::TopStrip, Place::Floating)));
        mv(&mut l, (100, 380));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.cameras[f.view_cameras[2].unwrap() as usize].origin, (0, 124));
        mv(&mut l, (600, 380));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.cameras[f.view_cameras[2].unwrap() as usize].origin, (404, 124));
    }

    /// Every split, zoom kind, zoom place, zoom height and HUD scale, the
    /// pointer in each view (free, near the corner, and captured with the
    /// hint in the buttons' place): the views' own interface (the corner's
    /// buttons, the hint) lies inside the active view's area
    /// (`view_area`), that area inside its pane; neither meets a zoom
    /// strip (nor its frame), and a zoom box lies under that interface.
    #[test]
    fn per_view_interface_lies_in_its_view_clear_of_the_strip_and_over_a_box() {
        let mut s = with_parts(None);
        s.views.push(ViewState { id: 259, view: View { interface_only: true, origin: (8, 8), viewport: (133, 38), ..Default::default() }, overlay: vec![] });
        s.interface.capture_hint = Some(259);
        let (roaming, hint) = (2, 3);
        let grown = |r: [i32; 4]| [r[0] - GAP / 2, r[1] - GAP / 2, r[2] + GAP, r[3] + GAP];
        let within = |inner: [i32; 4], outer: [i32; 4]| inner[0] >= outer[0] && inner[1] >= outer[1] && inner[0] + inner[2] <= outer[0] + outer[2] && inner[1] + inner[3] <= outer[1] + outer[3];
        let mut checked = 0;
        for split in [S::Standard, S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
            for zoom in [Zoom::Off, Zoom::Floating131, Zoom::Floating171, Zoom::Mouse111, Zoom::Mouse131, Zoom::TopStrip] {
                for place in [Place::Floating, Place::LeftOrBottom, Place::RightOrTop] {
                    for strip in [0.05, 0.25, 0.5] {
                        for hud in [(1, 1), (2, 1)] {
                            for side in [Side::A, Side::B] {
                                for (near_corner, captured) in [(false, false), (true, false), (false, true)] {
                                    let what = format!("{split:?} {zoom:?} {place:?} strip {strip} hud {hud:?} {side:?} near corner {near_corner} captured {captured}");
                                    let mut l = arranged(&settings(|s| {
                                        (s.split, s.zoom, s.zoom_place, s.zoom_strip) = (split, zoom, place, strip);
                                        s.roles = if captured { Roles::UnitMouse } else { Roles::Free };
                                    }));
                                    l.hud_scale = hud;
                                    // (One view: no B. Captured: B, which follows the world cursor.)
                                    if (side == Side::B && split == S::Standard) || (captured && side == Side::A) { continue; }
                                    let (a, b) = l.panes();
                                    let pane = if side == Side::A { a } else { b.unwrap() };
                                    let middle = (pane[0] + pane[2] / 2, pane[1] + pane[3] / 2);
                                    let p = if near_corner { (pane[0] + 170, pane[1] + 20) } else { middle };
                                    // (Into the view through the divider's open stretch.)
                                    let vertical = matches!(split, S::Vertical1To1 | S::Vertical2To1);
                                    let (lo, hi) = if vertical { (pane[1], pane[1] + pane[3]) } else { (pane[0], pane[0] + pane[2]) };
                                    let along = l.open_spans().iter().map(|&(a, b)| (a.max(lo), b.min(hi))).find(|(a, b)| b > a).map_or((lo + hi) / 2, |(a, b)| (a + b) / 2);
                                    mv(&mut l, if vertical { (middle.0, along) } else { (along, middle.1) });
                                    if captured {
                                        // UnitMouse: B captures; A: let go in A.
                                        if side == Side::B { click(&mut l, p); } else { mv(&mut l, p); }
                                    } else {
                                        mv(&mut l, p);
                                    }
                                    let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                                    assert_eq!(l.active_side(), side, "{what}: locked {} side {:?} pointer {:?} pane {pane:?}", l.locked, l.side, l.pointer);
                                    let area = l.view_area(side);
                                    assert!(within(area, pane), "{what}: area {area:?} in its pane {pane:?}");
                                    let shown = if l.locked { hint } else { roaming };
                                    let c = f.cameras[f.view_cameras[shown].unwrap_or_else(|| panic!("{what}: not shown")) as usize];
                                    let ui = l.view_ui_rect(rect(&c), (0, 0), area);
                                    assert!(within(ui, area), "{what}: per-view interface {ui:?} in its view's area {area:?}");
                                    assert_eq!(l.view_ui, vec![ui], "{what}");
                                    if let Some((_, z)) = l.shown.iter().find(|(h, _)| *h == Hit::Magnifier) {
                                        let z = grown(rect(z));
                                        if matches!(zoom, Zoom::TopStrip) {
                                            // The strip: the views give it room.
                                            assert!(!overlap(ui, z), "{what}: per-view interface {ui:?} under the strip {z:?}");
                                            assert!(!overlap(area, z), "{what}: area {area:?} under the strip {z:?}");
                                        } else {
                                            // A box: under the interface, which is drawn over it.
                                            assert!(f.view_cameras[shown].unwrap() >= f.cursor_layer.unwrap(), "{what}: per-view interface drawn under the box");
                                        }
                                    }
                                    checked += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(checked > 1000, "{checked}");
    }

    #[test]
    fn while_the_mouse_is_captured_a_hint_shows_in_the_corners_place() {
        let mut s = with_parts(None);
        let part = |id, origin, viewport| ViewState { id, view: View { interface_only: true, origin, viewport, ..Default::default() }, overlay: vec![] };
        s.views.push(part(259, (8, 8), (133, 38)));
        s.interface.capture_hint = Some(259);
        let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
        // Free: the buttons in A, no hint.
        mv(&mut l, (100, 100));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(f.view_cameras[2].is_some() && f.view_cameras[3].is_none());
        // Captured in B: the hint at B's top-left, no buttons.
        click(&mut l, (600, 300));
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.view_cameras[2], None);
        assert_eq!(f.cameras[f.view_cameras[3].unwrap() as usize].origin, (412, 8));
        // Snapping: the buttons, to be snapped to.
        l.set_snapping(true);
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(f.view_cameras[2].is_some() && f.view_cameras[3].is_none());
        l.set_snapping(false);
        // Let go: the buttons again, in the view the pointer is in.
        key(&mut l, VK_CONTROL, true);
        key(&mut l, VK_CONTROL, false);
        let f = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(f.view_cameras[2].is_some() && f.view_cameras[3].is_none());
    }

    /// A full screen open (`kind`), with the hint view the driver sends
    /// meanwhile (259: its corner buttons are not there).
    fn screen_with_hint(kind: &str) -> Scene {
        let mut s = with_screen(kind);
        s.views.push(ViewState { id: 259, view: View { interface_only: true, origin: (8, 8), viewport: (133, 38), ..Default::default() }, overlay: vec![] });
        s.interface.capture_hint = Some(259);
        // (The game's semantic camera focus: every map point is a world one.)
        s.camera_focus = Some(scene::CameraFocus { serial: 1, center: (3000, 2000) });
        s
    }

    fn screen_layout(place: Place, roles: Roles, ui_in_pane: bool) -> Layout {
        let mut settings = settings(|s| (s.split, s.roles) = (S::Vertical1To1, roles));
        settings.screen_place = ScreenPlace { vertical: place, horizontal: Place::Floating };
        let mut l = Layout::default();
        l.apply_settings(&settings);
        if ui_in_pane {
            l.configure("ui=pane").unwrap();
        }
        frame(&mut l, vec![]);
        l
    }

    fn hint_at(l: &mut Layout, s: &Scene) -> Option<(i32, i32)> {
        let f = l.frame(s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let i = s.views.iter().position(|v| v.id == 259).unwrap();
        f.view_cameras[i].map(|c| f.cameras[c as usize].origin)
    }

    #[test]
    fn beside_a_screen_the_world_takes_world_input_and_the_screen_its_own() {
        for (place, screen_at, world_at) in [(Place::LeftOrBottom, (200, 150), (600, 300)), (Place::RightOrTop, (600, 150), (200, 300))] {
            for kind in ["F1", "City", "Civilopedia"] {
                let mut l = screen_layout(place, Roles::Free, false);
                let s = screen_with_hint(kind);
                l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                // The other view: hover, presses and keys on the world (the
                // driver plays them on its map while the screen stays open).
                let out = mv(&mut l, world_at);
                assert!(out.iter().any(|i| matches!(i, Input::WorldMove { .. })), "{place:?} {kind}: {out:?}");
                let out = click(&mut l, world_at);
                assert!(out.iter().any(|i| matches!(i, Input::WorldButton { button: Button::Left, down: true, .. })), "{place:?} {kind}: {out:?}");
                assert!(!out.iter().any(|i| matches!(i, Input::MouseButton { .. })), "{place:?} {kind}: {out:?}");
                assert!(matches!(key(&mut l, 0x47, true)[..], [Input::WorldKey { vk: 0x47, down: true, .. }]), "{place:?} {kind}");
                key(&mut l, 0x47, false);
                // The screen: its own clicks, at its window pixels.
                mv(&mut l, screen_at);
                let out = click(&mut l, screen_at);
                assert!(matches!(out[..], [Input::MouseButton { button: Button::Left, down: true, .. }, ..]), "{place:?} {kind}: {out:?}");
                assert!(!out.iter().any(|i| matches!(i, Input::WorldButton { .. })), "{place:?} {kind}: {out:?}");
            }
        }
    }

    #[test]
    fn beside_a_screen_a_captured_mouse_shows_how_to_let_it_go() {
        // The screen in A (the placement setting) or the whole window
        // fitted into A (ui=pane); the mouse captured in B beside it (the
        // view following the pointer).
        for (place, ui_in_pane, world_at, hint) in [
            (Place::LeftOrBottom, false, (600, 300), (412, 8)),
            (Place::LeftOrBottom, true, (600, 300), (412, 8)),
        ] {
            for kind in ["F1", "City"] {
                let what = format!("{place:?}, ui in pane {ui_in_pane}, {kind}");
                let mut l = screen_layout(place, Roles::UnitMouse, ui_in_pane);
                let s = screen_with_hint(kind);
                l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                assert_eq!(hint_at(&mut l, &s), None, "{what}: free, no hint");
                mv(&mut l, world_at);
                click(&mut l, world_at);
                assert!(l.captured(), "{what}");
                assert_eq!(hint_at(&mut l, &s), Some(hint), "{what}: the hint at the captured world's top-left");
                // Let go: no hint.
                key(&mut l, VK_CONTROL, true);
                key(&mut l, VK_CONTROL, false);
                assert!(!l.captured(), "{what}");
                assert_eq!(hint_at(&mut l, &s), None, "{what}");
            }
        }
    }

    #[test]
    fn a_popup_opening_while_the_mouse_is_captured_gets_the_pointer() {
        let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
        mv(&mut l, (600, 300));
        frame(&mut l, vec![]);
        click(&mut l, (600, 300));
        assert!(l.captured());
        // A popup opened by a hotkey stays in the captured view B.
        // Opening it lets go of the mouse and places the pointer on it.
        let _ = l.take_warp();
        let f = l.frame(&with_parts(Some(300)), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let c = f.cameras[f.view_cameras[3].unwrap() as usize];
        assert!(!l.captured());
        assert_eq!(l.take_warp(), Some((c.origin.0 + 150, c.origin.1 + 75)));
        let row = (c.origin.0 + 20, c.origin.1 + 30);
        assert_eq!(click(&mut l, row)[0], Input::MouseButton { button: Button::Left, down: true, x: 320, y: 230 });
    }

    #[test]
    fn a_popup_moves_fully_into_a_where_it_fits() {
        let unit_mouse = settings(|s| {
            (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse);
            s.menu_follows_pointer = false;
        });
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
        l.menu_follows_pointer = false;
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
        menus_gone(&mut l, &with_parts(None));
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
        l2.screen_place = ScreenPlace { vertical: Place::LeftOrBottom, horizontal: Place::Floating };
        mv(&mut l2, (600, 300));
        frame(&mut l2, vec![]);
        click(&mut l2, (600, 300));
        l2.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        // (A is 400 wide: the 800x600 screen at 1/2.)
        assert_eq!(l2.take_warp(), Some((250, 200)));
        // Closed: captured again at the world cursor it had.
        menus_gone(&mut l, &scene());
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
        let (w, h) = b.world_size();
        assert_eq!(l.cursor, (b.x + w as i32 / 2, b.y + h as i32 / 2));
    }

    #[test]
    fn the_divider_is_glass_where_it_is_closed() {
        use glass::Shade::{Frost, Light, Shadow};
        let shades = [Light, Frost, Frost, Shadow];
        // Free, edge scrolling: the closed thirds, one strip a column, the
        // light edge on A's side and the shadow on B's.
        let mut l = split_1to1();
        let f = frame(&mut l, vec![]);
        let strips: Vec<([i32; 4], bool, Shade)> = f.divider.iter().map(|d| (d.1, d.2, d.3)).collect();
        let mut expected = Vec::new();
        for y in [(0, 200), (400, 200)] {
            for k in 0..4 {
                expected.push(([400 + k, y.0, 1, y.1], true, shades[k as usize]));
            }
        }
        assert_eq!(strips, expected);
        // Each drawn in the half of the gap it lies in, over that pane's
        // picture going on there (a world camera, the whole gap long).
        for &(c, r, _, _) in &f.divider {
            let cam = f.cameras[c as usize];
            assert!(cam.cells, "the world shows through");
            let half = if r[0] < 402 { (400, 0) } else { (402, 0) };
            assert_eq!((cam.origin, cam.viewport), (half, (2, 600)), "{r:?}");
        }
        // Captured: all of it.
        let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
        click(&mut l, (600, 300));
        let f = frame(&mut l, vec![]);
        assert_eq!(f.divider.iter().map(|d| d.1).collect::<Vec<_>>(), (0..4).map(|k| [400 + k, 0, 1, 600]).collect::<Vec<_>>());
        // Drag: the pointer passes everywhere, the glass is all of it.
        let mut l = split_1to1();
        l.set_map_movement(viewer_settings::MapMovement::Drag);
        let f = frame(&mut l, vec![]);
        assert_eq!(f.divider.iter().map(|d| (d.1, d.3)).collect::<Vec<_>>(), (0..4).map(|k| ([400 + k, 0, 1, 600], shades[k as usize])).collect::<Vec<_>>());
        // Stacked: rows across.
        let mut h = arranged(&settings(|s| s.split = S::Horizontal1To1));
        let f = frame(&mut h, vec![]);
        let rows: Vec<_> = f.divider.iter().map(|d| (d.1, d.2)).collect();
        let mut expected = Vec::new();
        for x in [(0, 266), (534, 266)] {
            for k in 0..4 {
                expected.push(([x.0, 300 + k, x.1, 1], false));
            }
        }
        assert_eq!(rows, expected);
        // The shades of other widths: never a shadow without a light edge.
        assert_eq!((0..1).map(|k| glass::Shade::at(k, 1)).collect::<Vec<_>>(), [Frost]);
        assert_eq!((0..2).map(|k| glass::Shade::at(k, 2)).collect::<Vec<_>>(), [Light, Frost]);
        assert_eq!((0..3).map(|k| glass::Shade::at(k, 3)).collect::<Vec<_>>(), [Light, Frost, Shadow]);
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
    fn capture_sensitivity_preserves_fractional_motion_with_independent_window_and_world_scales() {
        for pixel_scale in [1, 2, 3, 6] {
            let plan = crate::scaling::plan(
                (800 * pixel_scale, 600 * pixel_scale), (800, 600),
                &viewer_settings::Resolution { pixel_scale, ..Default::default() },
            );
            for zoom in [2, ONE, 5, 6] {
                for world_scale in [(1, 1), (3, 2)] {
                    for sensitivity in [0.5, 1.0, 2.5] {
                        let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
                        l.split.as_mut().unwrap().view.zoom = zoom;
                        l.world_scale = world_scale;
                        l.set_capture_sensitivity(sensitivity);
                        frame(&mut l, vec![]);
                        assert!(click(&mut l, (600, 300)).is_empty());
                        let from = l.cursor;
                        // 257 small events cross many whole-world-pixel boundaries.
                        // Physical counts first pass through the presentation scale,
                        // then through the independent world-camera zoom.
                        for _ in 0..257 {
                            let physical = (0.125 * pixel_scale as f64, -0.375 * pixel_scale as f64);
                            l.motion((physical.0 / plan.factor(), physical.1 / plan.factor()));
                        }
                        let (n, d) = times(ZOOMS[zoom], world_scale);
                        let factor = sensitivity as f64 * d as f64 / n as f64;
                        let expected = (257.0 * 0.125 * factor, -257.0 * 0.375 * factor);
                        assert_eq!(l.cursor, (from.0 + expected.0.trunc() as i32, from.1 + expected.1.trunc() as i32),
                            "pixel scale {pixel_scale}, camera {zoom}, world scale {world_scale:?}, sensitivity {sensitivity}");
                        assert!((l.motion_rest.0 - expected.0.fract()).abs() < 1e-9);
                        assert!((l.motion_rest.1 - expected.1.fract()).abs() < 1e-9);
                    }
                }
            }
        }
    }

    #[test]
    fn capture_sensitivity_updates_live_without_losing_fractional_motion() {
        let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
        click(&mut l, (600, 300));
        let from = l.cursor;
        l.set_capture_sensitivity(0.5);
        l.motion((1.0, -1.0));
        assert_eq!(l.cursor, from);
        assert_eq!(l.motion_rest, (0.5, -0.5));
        l.set_capture_sensitivity(2.5);
        l.motion((1.0, -1.0));
        assert_eq!(l.cursor, (from.0 + 3, from.1 - 3));
        assert_eq!(l.motion_rest, (0.0, 0.0));
        l.set_capture_sensitivity(f32::NAN);
        l.motion((1.0, -1.0));
        assert_eq!(l.cursor, (from.0 + 4, from.1 - 4));
    }

    #[test]
    fn capture_sensitivity_leaves_free_pointer_map_gestures_and_edge_pointer_mapping_unchanged() {
        for movement in [viewer_settings::MapMovement::EdgeScroll, viewer_settings::MapMovement::Drag] {
            let mut slow = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
            slow.set_map_movement(movement);
            let mut fast = slow.clone();
            slow.set_capture_sensitivity(0.1);
            fast.set_capture_sensitivity(5.0);
            let from = slow.cursor;
            assert!(slow.motion((300.0, 200.0)).is_empty());
            assert!(fast.motion((300.0, 200.0)).is_empty());
            assert_eq!((slow.cursor, fast.cursor), (from, from));
            for input in [
                Input::MouseMove { x: 100, y: 300 },
                Input::MouseButton { button: Button::Left, down: true, x: 100, y: 300 },
                Input::MouseMove { x: 150, y: 350 },
                Input::MouseButton { button: Button::Left, down: false, x: 150, y: 350 },
                Input::MouseMove { x: 0, y: 100 },
                Input::Wheel { delta: 120, x: 100, y: 100 },
            ] {
                assert_eq!(slow.route(input.clone()), fast.route(input));
            }
            assert_eq!(frame(&mut slow, vec![]).cameras, frame(&mut fast, vec![]).cameras);
            // At a world edge the visible OS pointer directly locates the
            // world cursor. Its absolute mapping must not be multiplied.
            click(&mut slow, (600, 300));
            click(&mut fast, (600, 300));
            if movement == viewer_settings::MapMovement::Drag {
                // (Drag captures nothing: `capture`.)
                assert!(!slow.captured() && !fast.captured());
                continue;
            }
            assert!(slow.captured() && fast.captured());
            assert_eq!(slow.pointer_to((700, 400)), fast.pointer_to((700, 400)));
            assert_eq!(slow.cursor, fast.cursor);
        }
    }

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
        let (sw, sh) = b.world_size();
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
    fn a_floating_zoom_box_moves_clear_of_the_capturing_pointer() {
        let mut l = arranged(&settings(|s| (s.split, s.roles, s.zoom, s.zoom_place) = (S::Vertical1To1, Roles::UnitMouse, Zoom::Floating131, Place::RightOrTop)));
        mv(&mut l, (600, 300));
        frame(&mut l, vec![]);
        assert_eq!(l.hit((600, 300)), Hit::Pane);
        assert_eq!(l.take_pointer_icon(), Some(PointerIcon::Capture));
        click(&mut l, (600, 300));
        assert!(l.locked);
    }

    #[test]
    fn a_zoom_box_at_the_mouse_goes_above_the_world_cursor_while_captured() {
        let mut l = arranged(&settings(|s| (s.split, s.roles, s.zoom, s.zoom_place) = (S::Vertical1To1, Roles::UnitMouse, Zoom::Mouse111, Place::RightOrTop)));
        mv(&mut l, (600, 400));
        frame(&mut l, vec![]);
        // (Above the pointer: never under it.)
        assert_eq!(l.hit((600, 400)), Hit::Pane);
        click(&mut l, (600, 400));
        assert!(l.locked);
        frame(&mut l, vec![]);
        let at = l.cursor_on_window().unwrap();
        let r = l.magnifier_geometry(&l.magnifier.unwrap()).0;
        assert_eq!((r[0] + r[2] / 2, r[1] + r[3]), (at.0.clamp(404 + 66, 800 - 66), at.1 - GAP), "{r:?} {at:?}");
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
        // pixels, with the driver's labels attached to the same world pixels.
        l.advance(std::time::Duration::from_millis(16));
        let f = l.frame(&at(1064), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(f.cameras[0].x > 1200 && f.cameras[0].x < 1264, "{}", f.cameras[0].x);
        assert_eq!(f.cameras[0].screen_shift.0, 1064 - f.cameras[0].x);
        assert!(l.main_unmoved());
        assert!(!mv(&mut l, (2, 300)).iter().any(|i| matches!(i, Input::Edge { .. })));
        mv(&mut l, (100, 300));
        for _ in 0..120 {
            l.advance(std::time::Duration::from_millis(16));
            l.frame(&at(1064), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        }
        let f = l.frame(&at(1064), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        // (A shows the driver's middle in its own: 200 pixels further on.)
        assert_eq!((f.cameras[0].x, f.cameras[0].screen_shift, f.cameras[0].cull), (1264, (-200, 0), None));
    }

    fn captured_edge_fixture(at: (i32, i32), wrap: (bool, bool)) -> Layout {
        let mut l = arranged(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
        assert!(l.capture_at((600, 300)));
        l.cursor = at;
        l.frame(&scene(), (800, 600), (0, 0, 0, 0), vec![], Some(WorldBounds { size: (8000, 4000), wrap }));
        l.take_grab();
        l.set_pointer_at_edge(true);
        l
    }

    #[test]
    fn captured_edge_confinement_has_a_line_for_each_single_axis_and_an_area_at_corners() {
        for (cursor, axes) in [((10, 2000), (true, false)), ((4000, 10), (false, true)), ((10, 10), (true, true)), ((4000, 2000), (false, false))] {
            let l = captured_edge_fixture(cursor, (false, false));
            let (view, at, _) = l.world_cursor_view().unwrap();
            assert_eq!(l.world_cursor_axes(), Some(axes));
            let bounds = l.pointer_bounds().unwrap();
            assert_eq!(bounds, [if axes.0 { view[0] } else { at.0 }, if axes.1 { view[1] } else { at.1 }, if axes.0 { view[2] } else { 1 }, if axes.1 { view[3] } else { 1 }]);
            let walls = l.pointer_walls().unwrap();
            for x in (0..800).step_by(7).chain([bounds[0]]) {
                for y in (0..600).step_by(7).chain([bounds[1]]) {
                    assert_eq!(walls.iter().any(|&r| inside(r, (x, y))), !inside(bounds, (x, y)), "cursor{cursor:?}, pointer{x},{y}, {bounds:?}");
                }
            }
            // The divider's usual middle opening cannot leak a capture.
            assert!(walls.iter().any(|&r| inside(r, (401, 300))));
        }
        let wrapping = captured_edge_fixture((10, 10), (true, false));
        assert_eq!(wrapping.world_cursor_axes(), Some((false, true)));
        assert_eq!(wrapping.pointer_bounds().unwrap()[2], 1);
    }

    #[test]
    fn captured_edge_pointer_warps_at_the_view_and_preserves_relative_motion_on_the_other_axis() {
        for cursor in [(10, 2000), (4000, 10), (10, 10)] {
            let mut l = captured_edge_fixture(cursor, (false, false));
            let axes = l.world_cursor_axes().unwrap();
            let bounds = l.pointer_bounds().unwrap();
            l.pointer_to((-100, 900));
            let expected = (bounds[0], bounds[1] + bounds[3] - 1);
            assert_eq!(l.take_warp(), Some(expected));
            assert_eq!(l.pointer, expected);
            // Test relative movement while the original edge axes remain
            // active; the large absolute move above may leave that edge.
            let mut l = captured_edge_fixture(cursor, (false, false));
            let expected = l.pointer;
            let before = l.cursor;
            l.edge_motion((10.0, 20.0));
            assert_eq!(l.cursor, (before.0 + if axes.0 { 0 } else { 10 }, before.1 + if axes.1 { 0 } else { 20 }));
            // An absolute event in the same frame must not undo that raw
            // movement through the camera from the preceding frame.
            let after = l.cursor;
            l.pointer_to(expected);
            if !axes.0 { assert_eq!(l.cursor.0, after.0); }
            if !axes.1 { assert_eq!(l.cursor.1, after.1); }
        }
        let mut l = captured_edge_fixture((4000, 10), (false, false));
        l.set_capture_sensitivity(0.1);
        let at = l.pointer;
        let start = l.cursor;
        for _ in 0..40 {
            l.edge_motion((0.5, 15.0));
            l.pointer_to(at);
        }
        assert!((l.cursor.0 - start.0 - 2).abs() <= 1);
        assert_eq!(l.cursor.1, start.1);
        assert!(l.motion_rest.0.abs() < 1.0);
    }

    #[test]
    fn captured_edge_constraints_release_for_control_focus_snapping_and_menus() {
        let mut l = captured_edge_fixture((4000, 10), (false, false));
        l.set_snapping(true);
        assert_eq!(l.pointer_bounds(), None);
        assert_eq!(l.pointer_walls(), None);
        l.set_snapping(false);
        l.set_pointer_at_edge(true);
        assert!(l.pointer_bounds().is_some());
        key(&mut l, VK_CONTROL, true);
        key(&mut l, VK_CONTROL, false);
        assert!(!l.captured());
        assert_eq!(l.pointer_bounds(), None);
        for mode in [0, 1, 2] {
            let mut l = captured_edge_fixture((4000, 10), (false, false));
            match mode {
                0 => l.released(),
                1 => { l.frame(&with_parts(Some(300)), (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
                _ => { l.frame(&with_screen("F1"), (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
            }
            assert!(!l.captured());
            assert_eq!(l.pointer_bounds(), None);
            assert!(!l.pointer_at_edge);
        }
    }

    #[test]
    fn captured_edge_confinement_keeps_the_top_zoom_strip_reserved() {
        let mut l = captured_edge_fixture((4000, 10), (false, false));
        l.apply_settings(&settings(|s| (s.split, s.roles, s.zoom) = (S::Vertical1To1, Roles::UnitMouse, Zoom::TopStrip)));
        l.frame(&scene(), (800, 600), (0, 0, 0, 0), vec![], Some(WorldBounds { size: (8000, 4000), wrap: (false, false) }));
        l.set_pointer_at_edge(true);
        let bounds = l.pointer_bounds().unwrap();
        let (strip, _) = l.strip().unwrap();
        assert!(bounds[1] >= strip[1] + strip[3] + GAP);
        assert!(!l.zoom_pointer_walls().is_empty());
        let x = bounds[0];
        l.pointer_to((x, strip[1]));
        assert_eq!(l.take_warp(), Some((x, bounds[1])));
        assert!(l.pointer_walls().unwrap().iter().any(|&r| inside(r, (x, strip[1]))));
    }


    // ---- Popups opened at a world click --------------------------------

    /// The parked game camera's scene (every map click a world click) and
    /// the same with a popup of `size` (driver pixels) the driver anchors
    /// at `anchor`.
    fn parked() -> Scene {
        let mut s = scene();
        s.camera_focus = Some(scene::CameraFocus { serial: 1, center: (3000, 2000) });
        s
    }

    fn with_anchored_popup(anchor: scene::PopupAnchor, size: (u32, u32)) -> Scene {
        let mut s = parked();
        s.interface.popup = Some(257);
        s.interface.popup_anchor = Some(anchor);
        s.views.push(ViewState { id: 257, view: View { interface_only: true, origin: (40, 30), viewport: size, ..Default::default() }, overlay: vec![] });
        s
    }

    fn world_press(l: &mut Layout, button: Button, p: (i32, i32)) -> (i32, i32) {
        let out = l.route(Input::MouseButton { button, down: true, x: p.0, y: p.1 });
        let w = out.iter().find_map(|i| match *i { Input::WorldButton { x, y, down: true, .. } => Some((x, y)), _ => None }).unwrap_or_else(|| panic!("no world press at {p:?}: {out:?}"));
        l.route(Input::MouseButton { button, down: false, x: p.0, y: p.1 });
        w
    }

    /// The canvas pixel `camera` shows world pixel `w` at (in the wrap image
    /// nearest `near`).
    fn shown_at(camera: &Camera, w: (i32, i32), near: (i32, i32), period: (i32, i32)) -> (i32, i32) {
        let (n, d) = (camera.zoom.0 as i32, camera.zoom.1 as i32);
        let axis = |w: i32, c: i32, o: i32, near: i32, period: i32| {
            let mut rel = w - c;
            if period > 0 {
                let want = (near - o) * d / n;
                rel += (want - rel + period / 2).div_euclid(period) * period;
            }
            o + (rel * n).div_euclid(d)
        };
        (axis(w.0, camera.x, camera.origin.0, near.0, period.0), axis(w.1, camera.y, camera.origin.1, near.1, period.1))
    }

    #[test]
    fn a_menu_opened_at_a_world_click_has_its_corner_there_in_that_view() {
        let menu = (160, 120);
        for split in [S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
            for roles in [Roles::Free, Roles::UnitMouse] {
                for zoom in [ONE, ONE + 2, ONE - 2] {
                    for menu_scale in [(1, 1), (2, 1), (3, 2)] {
                        for side in [Side::A, Side::B] {
                            let what = format!("{split:?} {roles:?} zoom {:?} menu {menu_scale:?} {side:?}", ZOOMS[zoom]);
                            let mut l = Layout::default();
                            l.apply_settings(&settings(|s| (s.split, s.roles) = (split, roles)));
                            l.main_view.zoom = zoom;
                            l.split.as_mut().unwrap().view.zoom = zoom;
                            l.menu_scale = menu_scale;
                            let s = parked();
                            for _ in 0..3 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
                            let (a, b) = l.panes();
                            let pane = if side == Side::A { a } else { b.unwrap() };
                            // Near the pane's top-left: the menu fits.
                            let p = (pane[0] + 37, pane[1] + 23);
                            let captures = roles == Roles::UnitMouse && side == Side::B;
                            let (at, w) = if captures {
                                // A click into B captures the mouse; the next goes
                                // to the world cursor (the middle of B).
                                click(&mut l, p);
                                assert!(l.captured(), "{what}");
                                l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                                let at = l.cursor_on_window().unwrap();
                                (at, world_press(&mut l, Button::Right, at))
                            } else {
                                (p, world_press(&mut l, Button::Left, p))
                            };
                            let before = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                            let open = with_anchored_popup(scene::PopupAnchor { world: w, offset: (1, 1), world_scaled: false }, menu);
                            let f = l.frame(&open, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                            let (corner, z) = f.popup_at.unwrap_or_else(|| panic!("{what}: not anchored"));
                            assert_eq!(z, menu_scale, "{what}");
                            let step = |v: i32| (v * z.0 as i32).div_euclid(z.1 as i32);
                            let (n, d) = ZOOMS[zoom];
                            let slack = (n / d).max(1) as i32;
                            // The clicked world pixel as its view shows it now, the
                            // menu one menu pixel past it.
                            let now = camera_at(&f, (pane[0], pane[1]));
                            let q = shown_at(&now, w, at, (8000, 0));
                            assert_eq!(corner, (q.0 + step(1), q.1 + step(1)), "{what}");
                            // Not kept out of the other view, nor anywhere else.
                            assert_eq!(f.popup_region, None, "{what}");
                            let fits = at.0 + step(1) + scaled_up(menu.0 as i32, z) <= 800 && at.1 + step(1) + scaled_up(menu.1 as i32, z) <= 600;
                            if fits {
                                // There: where the click was (to the art pixel).
                                assert!((q.0 - at.0).abs() < slack && (q.1 - at.1).abs() < slack, "{what}: {corner:?} for a click at {at:?}");
                            }
                            // Fitting, no view moved for it; else only the clicked one.
                            for origin in [(a[0], a[1]), (b.unwrap()[0], b.unwrap()[1])] {
                                let (x, y) = (camera_at(&before, origin), camera_at(&f, origin));
                                if fits || origin != (pane[0], pane[1]) {
                                    assert_eq!((x.x, x.y), (y.x, y.y), "{what}: the view at {origin:?} moved");
                                }
                            }
                            assert!(corner.0 + scaled_up(menu.0 as i32, z) <= 800 && corner.1 + scaled_up(menu.1 as i32, z) <= 600, "{what}: {corner:?}");
                            if captures {
                                assert!(!l.captured(), "{what}: the mouse is let go for the menu");
                                assert_eq!(l.pointer, at, "{what}: the pointer stays at the click, the menu's corner");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_tile_box_is_placed_from_its_tiles_world_pixel_across_the_wrap_seam() {
        // A world pixel just past the seam (the game anchors the box half a
        // tile left of its tile): drawn next to the click, not a world away.
        let mut l = Layout::default();
        l.apply_settings(&settings(|s| s.split = S::Vertical1To1));
        let s = parked();
        for _ in 0..3 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
        let p = (100, 100);
        let w = world_press(&mut l, Button::Right, p);
        let anchor = (w.0 - 8000 - 64, w.1 - 20);
        let f = l.frame(&with_anchored_popup(scene::PopupAnchor { world: anchor, offset: (0, 0), world_scaled: false }, (256, 173)), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(f.popup_at, Some(((p.0 - 64, p.1 - 20), (1, 1))));
    }

    /// The parked scene with kept tile boxes (`Interface::detached`): view,
    /// serial and the world pixel of its click, each anchored half a tile
    /// left of and 20 pixels above that click.
    fn with_detached(kept: &[(u32, u64, (i32, i32))]) -> Scene {
        let mut s = parked();
        for &(view, serial, click) in kept {
            let anchor = scene::PopupAnchor { world: (click.0 - 64, click.1 - 20), offset: (0, 0), world_scaled: false };
            s.interface.detached.push(scene::DetachedPopup { view, serial, anchor, click });
            s.views.push(ViewState { id: view, view: View { interface_only: true, viewport: (256, 173), ..Default::default() }, overlay: vec![] });
        }
        s
    }

    #[test]
    fn kept_tile_boxes_stay_in_their_views_while_the_other_view_plays() {
        let mut l = Layout::default();
        l.apply_settings(&settings(|s| s.split = S::Vertical1To1));
        let s = parked();
        for _ in 0..3 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
        let (a, b) = l.panes();
        let b = b.unwrap();
        let (pa, pb) = ((a[0] + 100, a[1] + 100), (b[0] + 100, b[1] + 100));
        let wa = world_press(&mut l, Button::Right, pa);
        let one = with_detached(&[(0x1000, 1, wa)]);
        let f = l.frame(&one, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(f.popup_places.contains(&(0x1000, ((pa.0 - 64, pa.1 - 20), (1, 1)))), "A's box at its click: {:?}", f.popup_places);
        assert!(!l.click_popup_open(), "a kept box holds no map still");
        // B plays on: its presses reach the game.
        let out = l.route(Input::MouseButton { button: Button::Left, down: true, x: pb.0, y: pb.1 });
        assert!(out.iter().any(|i| matches!(i, Input::WorldButton { down: true, .. })), "B's press reaches the game: {out:?}");
        assert!(!out.iter().any(|i| matches!(i, Input::CloseDetached { .. })), "{out:?}");
        l.route(Input::MouseButton { button: Button::Left, down: false, x: pb.0, y: pb.1 });
        // And opens its own box, both shown.
        let wb = world_press(&mut l, Button::Right, pb);
        let both = with_detached(&[(0x1000, 1, wa), (0x1001, 2, wb)]);
        let f = l.frame(&both, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(f.popup_places.contains(&(0x1000, ((pa.0 - 64, pa.1 - 20), (1, 1)))), "{:?}", f.popup_places);
        assert!(f.popup_places.contains(&(0x1001, ((pb.0 - 64, pb.1 - 20), (1, 1)))), "{:?}", f.popup_places);
        // A press in A closes A's box only, and does nothing else.
        let p = (a[0] + 300, a[1] + 400);
        assert_eq!(l.route(Input::MouseButton { button: Button::Left, down: true, x: p.0, y: p.1 }), vec![Input::CloseDetached { serial: 1 }]);
        assert_eq!(l.route(Input::MouseButton { button: Button::Left, down: false, x: p.0, y: p.1 }), vec![]);
        // Not drawn while the driver still sends it.
        let f = l.frame(&both, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let index = both.views.iter().position(|v| v.id == 0x1000).unwrap();
        assert_eq!(f.view_cameras[index], None);
        assert!(f.popup_places.iter().any(|(id, _)| *id == 0x1001));
        // The driver drops it: B's moves into the first slot, still B's.
        let f = l.frame(&with_detached(&[(0x1000, 2, wb)]), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(f.popup_places.contains(&(0x1000, ((pb.0 - 64, pb.1 - 20), (1, 1)))), "{:?}", f.popup_places);
        // Esc with the pointer in B closes it there, never reaching the game.
        l.route(Input::MouseMove { x: pb.0, y: pb.1 });
        assert_eq!(l.route(Input::Key { vk: VK_ESCAPE, down: true, text: None }), vec![Input::CloseDetached { serial: 2 }]);
        assert_eq!(l.route(Input::Key { vk: VK_ESCAPE, down: false, text: None }), vec![]);
        // With none kept, Esc is the game's again.
        l.frame(&parked(), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(matches!(l.route(Input::Key { vk: VK_ESCAPE, down: true, text: None }).as_slice(), [Input::Key { vk: VK_ESCAPE, .. } | Input::WorldKey { vk: VK_ESCAPE, .. }]));
    }

    #[test]
    fn a_new_tile_box_replaces_the_one_its_view_kept() {
        let mut l = Layout::default();
        l.apply_settings(&settings(|s| s.split = S::Vertical1To1));
        let s = parked();
        for _ in 0..3 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
        let (a, _) = l.panes();
        let w1 = world_press(&mut l, Button::Right, (a[0] + 100, a[1] + 100));
        l.frame(&with_detached(&[(0x1000, 1, w1)]), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        // A press there first closes it, as in the game.
        let p = (a[0] + 200, a[1] + 150);
        assert_eq!(l.route(Input::MouseButton { button: Button::Right, down: true, x: p.0, y: p.1 }), vec![Input::CloseDetached { serial: 1 }]);
        assert_eq!(l.route(Input::MouseButton { button: Button::Right, down: false, x: p.0, y: p.1 }), vec![]);
        l.frame(&parked(), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        // One opened again from that click (a menu's "Terrain details")
        // while its first is still kept takes its place.
        let w2 = world_press(&mut l, Button::Right, p);
        l.frame(&with_detached(&[(0x1000, 3, w2)]), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        l.frame(&with_detached(&[(0x1000, 3, w2), (0x1001, 4, w2)]), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let out = l.route(Input::MouseMove { x: a[0] + 10, y: a[1] + 10 });
        assert!(out.contains(&Input::CloseDetached { serial: 3 }), "the older one closes: {out:?}");
        assert!(!out.contains(&Input::CloseDetached { serial: 4 }), "{out:?}");
    }

    #[test]
    fn a_menu_past_the_windows_edge_moves_its_view_just_enough() {
        let menu = (200, 150);
        for roles in [Roles::Free, Roles::UnitMouse] {
            for side in [Side::A, Side::B] {
                for zoom in [ONE, ONE + 2] {
                    let what = format!("{roles:?} {side:?} zoom {:?}", ZOOMS[zoom]);
                    let mut l = Layout::default();
                    l.apply_settings(&settings(|s| (s.split, s.roles) = (S::Horizontal1To1, roles)));
                    l.main_view.zoom = zoom;
                    l.split.as_mut().unwrap().view.zoom = zoom;
                    let s = parked();
                    for _ in 0..3 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
                    let (a, b) = l.panes();
                    let pane = if side == Side::A { a } else { b.unwrap() };
                    // Near the window's bottom-right corner.
                    let p = (pane[0] + pane[2] - 30, pane[1] + pane[3] - 20);
                    let captures = roles == Roles::UnitMouse && side == Side::B;
                    let (at, w) = if captures {
                        click(&mut l, p);
                        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                        let at = l.cursor_on_window().unwrap();
                        (at, world_press(&mut l, Button::Left, at))
                    } else {
                        (p, world_press(&mut l, Button::Left, p))
                    };
                    let before = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                    let other = if side == Side::A { (b.unwrap()[0], b.unwrap()[1]) } else { (a[0], a[1]) };
                    let mine = (pane[0], pane[1]);
                    let open = with_anchored_popup(scene::PopupAnchor { world: w, offset: (1, 1), world_scaled: false }, menu);
                    let f = l.frame(&open, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                    let ((x, y), _) = f.popup_at.unwrap();
                    // Whole in the window, its corner still at the clicked pixel.
                    let need = ((at.0 + 1 + menu.0 as i32 - 800).max(0), (at.1 + 1 + menu.1 as i32 - 600).max(0));
                    let (n, d) = ZOOMS[zoom];
                    let world = (world_steps(need.0, (n, d)), world_steps(need.1, (n, d)));
                    let (c0, c1) = (camera_at(&before, mine), camera_at(&f, mine));
                    if captures {
                        // (The pointer's view keeps the world cursor in its middle:
                        // the move is the popup's, gone once it closes.)
                        assert_eq!((c1.x - c0.x, c1.y - c0.y), world, "{what}");
                    } else {
                        assert_eq!((c1.x - c0.x, c1.y - c0.y), world, "{what}: moved by the least that fits");
                    }
                    assert!(x + menu.0 as i32 <= 800 && y + menu.1 as i32 <= 600 && x >= 0 && y >= 0, "{what}: {:?}", (x, y));
                    assert_eq!((x, y), { let q = shown_at(&c1, w, at, (8000, 0)); (q.0 + 1, q.1 + 1) }, "{what}");
                    assert!(x + menu.0 as i32 > 800 - 2 * (n / d) as i32 || y + menu.1 as i32 > 600 - 2 * (n / d) as i32, "{what}: no further than needed");
                    // The other view stays.
                    let (o0, o1) = (camera_at(&before, other), camera_at(&f, other));
                    assert_eq!((o0.x, o0.y), (o1.x, o1.y), "{what}");
                    // Closed: a view following the pointer is back on the cursor,
                    // a free or focus view stays where it was moved.
                    let after = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                    let c2 = camera_at(&after, mine);
                    if captures { assert_eq!((c2.x, c2.y), (c0.x, c0.y), "{what}"); } else { assert_eq!((c2.x, c2.y), (c1.x, c1.y), "{what}: a view moved for the menu stays moved"); }
                }
            }
        }
    }

    #[test]
    fn opening_any_popup_leaves_every_view_where_it_was() {
        // A generic popup (no anchor) and one at a click: the views do not
        // follow anything to it.
        for roles in [Roles::Free, Roles::UnitMouse] {
            for anchored in [false, true] {
                let mut l = Layout::default();
                l.apply_settings(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, roles)));
                let s = parked();
                for _ in 0..3 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
                let w = world_press(&mut l, Button::Right, (100, 100));
                let before = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                let mut open = with_anchored_popup(scene::PopupAnchor { world: w, offset: (1, 1), world_scaled: false }, (120, 90));
                if !anchored { open.interface.popup_anchor = None; }
                for _ in 0..3 {
                    let f = l.frame(&open, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                    for origin in [(0, 0), (404, 0)] {
                        let (x, y) = (camera_at(&before, origin), camera_at(&f, origin));
                        assert_eq!((x.x, x.y, x.zoom), (y.x, y.y, y.zoom), "{roles:?} anchored {anchored}: {origin:?}");
                    }
                    assert_eq!(f.popup_at.is_some(), anchored);
                }
            }
        }
    }

    #[test]
    fn a_dialog_replacing_a_click_menu_is_not_anchored() {
        let mut l = Layout::default();
        l.apply_settings(&settings(|s| s.split = S::Vertical1To1));
        let s = parked();
        for _ in 0..2 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
        let w = world_press(&mut l, Button::Right, (100, 100));
        let menu = with_anchored_popup(scene::PopupAnchor { world: w, offset: (1, 1), world_scaled: false }, (120, 90));
        assert_eq!(l.frame(&menu, (800, 600), (0, 0, 0, 0), vec![], BOUNDS).popup_at, Some(((101, 101), (1, 1))));
        // An entry chosen: the game's dialog in the same popup view.
        let mut dialog = menu.clone();
        dialog.interface.popup_anchor = None;
        for _ in 0..2 {
            let f = l.frame(&dialog, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            assert_eq!((f.popup_at, f.popup_region), (None, Some(l.panes().0)));
        }
        // An anchor the driver gives with no recent click is not taken.
        for _ in 0..200 { l.advance(std::time::Duration::from_millis(20)); l.frame(&dialog, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
        assert_eq!(l.frame(&menu, (800, 600), (0, 0, 0, 0), vec![], BOUNDS).popup_at, None);
    }

    #[test]
    fn a_tile_box_is_drawn_at_its_views_world_scale_and_a_menu_at_the_menus() {
        // The game draws the box's picture of a tile as large as its map's
        // tiles (1:1). The box opened in a view goes at that view's world
        // zoom (picture, frame and text alike), never below the frame's
        // (HUD) scale; a context menu stays at the menus' scale.
        for side in [Side::A, Side::B] {
            for (world_scale, zoom) in [((3, 2), ONE), ((3, 2), ONE + 2), ((1, 1), ONE - 2)] {
                let what = format!("{side:?} world {world_scale:?} zoom {:?}", ZOOMS[zoom]);
                let mut l = Layout::default();
                l.apply_settings(&settings(|s| s.split = S::Vertical1To1));
                l.world_scale = world_scale;
                l.main_view.zoom = zoom;
                l.split.as_mut().unwrap().view.zoom = zoom;
                (l.menu_scale, l.hud_scale) = ((8, 3), (1, 1));
                let s = parked();
                for _ in 0..3 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
                let (a, b) = l.panes();
                let pane = if side == Side::A { a } else { b.unwrap() };
                let p = (pane[0] + 40, pane[1] + 30);
                let w = world_press(&mut l, Button::Right, p);
                let tile_box = with_anchored_popup(scene::PopupAnchor { world: (w.0 - 64, w.1 - 23), offset: (0, 0), world_scaled: true }, (256, 173));
                let f = l.frame(&tile_box, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                let (_, z) = f.popup_at.unwrap_or_else(|| panic!("{what}: not anchored"));
                let view = camera_at(&f, (pane[0], pane[1])).zoom;
                let want = if view.0 >= view.1 { view } else { l.hud_scale };
                assert_eq!(z.0 as u64 * want.1 as u64, want.0 as u64 * z.1 as u64, "{what}: {z:?}, the view's {view:?}");
                l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                let w = world_press(&mut l, Button::Right, p);
                let menu = with_anchored_popup(scene::PopupAnchor { world: w, offset: (1, 1), world_scaled: false }, (120, 90));
                assert_eq!(l.frame(&menu, (800, 600), (0, 0, 0, 0), vec![], BOUNDS).popup_at.map(|p| p.1), Some((8, 3)), "{what}");
            }
        }
    }

    #[test]
    fn a_world_scaled_popup_fits_the_window_even_when_the_hud_is_larger() {
        let mut l = Layout::default();
        l.window = (400, 200);
        l.hud_scale = (3, 1);
        assert_eq!(l.popup_zoom((256, 173), Some((3, 2))), (200, 173));
        assert_eq!(l.popup_zoom((500, 100), Some((3, 2))), (4, 5));
    }

    #[test]
    fn a_popup_opened_at_a_world_click_holds_every_view_still() {
        // No edge scrolling, wheel, drag or captured motion moves a view
        // while a context menu or the tile box is open; closed, they do.
        let cams = |f: &Frame| [(0, 0), (404, 0)].map(|o| { let c = camera_at(f, o); (c.x, c.y, c.zoom) });
        for movement in [viewer_settings::MapMovement::EdgeScroll, viewer_settings::MapMovement::Drag] {
            for world_scaled in [false, true] {
                let what = format!("{movement:?} tile box {world_scaled}");
                let mut l = Layout::default();
                l.apply_settings(&settings(|s| s.split = S::Vertical1To1));
                l.set_map_movement(movement);
                let s = parked();
                for _ in 0..3 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
                // (A right press waits for its release: it may be a go-to drag.)
                let mut out = l.route(Input::MouseButton { button: Button::Right, down: true, x: 200, y: 200 });
                out.extend(l.route(Input::MouseButton { button: Button::Right, down: false, x: 200, y: 200 }));
                let w = out.iter().find_map(|i| match *i { Input::WorldButton { x, y, down: true, .. } => Some((x, y)), _ => None }).expect("a world press");
                let open = with_anchored_popup(scene::PopupAnchor { world: w, offset: (1, 1), world_scaled }, (120, 90));
                let before = l.frame(&open, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                assert!(l.click_popup_open(), "{what}");
                // At the window's left edge.
                mv(&mut l, (1, 300));
                for _ in 0..30 { l.advance(std::time::Duration::from_millis(20)); l.frame(&open, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
                assert!(l.route(Input::Wheel { delta: -120, x: 300, y: 300 }).is_empty(), "{what}");
                // A middle or left drag on the world grabs nothing: it dismisses.
                for button in [Button::Middle, Button::Left] {
                    assert_eq!(l.route(Input::MouseButton { button, down: true, x: 300, y: 300 }), vec![Input::DismissPopup], "{what} {button:?}");
                    mv(&mut l, (500, 450));
                    assert!(l.route(Input::MouseButton { button, down: false, x: 500, y: 450 }).is_empty(), "{what} {button:?}");
                }
                let c = l.cursor;
                assert!(l.motion((60.0, 40.0)).is_empty() && l.cursor == c, "{what}");
                let after = l.frame(&open, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                assert_eq!(cams(&before), cams(&after), "{what}");
                if movement == viewer_settings::MapMovement::EdgeScroll {
                    // Closed: the edge scrolls A again.
                    l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                    mv(&mut l, (1, 300));
                    for _ in 0..30 { l.advance(std::time::Duration::from_millis(20)); l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
                    let moved = l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                    assert_ne!(cams(&moved)[0], cams(&after)[0], "{what}: the edge scrolls once it is closed");
                }
            }
        }
    }

    #[test]
    fn a_scaled_world_popup_body_accepts_clicks_after_the_final_interface_map_is_installed() {
        let mut layout = Layout::default();
        let base = parked();
        for _ in 0..3 { layout.frame(&base, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
        let world = world_press(&mut layout, Button::Right, (120, 140));
        let parent = with_anchored_popup(scene::PopupAnchor { world, offset: (1, 1), world_scaled: false }, (120, 90));
        let mut frame = layout.frame(&parent, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let (at, _) = frame.popup_at.unwrap();
        let plan = crate::scaling::plan((800, 600), (800, 600), &viewer_settings::Resolution::default());
        let arranged = crate::scaling::arrange(&plan, &parent, &mut frame, |_| None);
        layout.set_interface_shown(arranged.shown);
        let out = click(&mut layout, (at.0 + 30, at.1 + 30));
        assert!(!out.contains(&Input::DismissPopup), "the body's final hit map is not an outside click: {out:?}");
        assert!(out.iter().any(|i| matches!(i, Input::MouseButton { down: true, .. })), "{out:?}");
    }

    #[test]
    fn a_world_anchored_parent_popup_keeps_its_place_and_world_under_a_nested_prompt() {
        let mut layout = Layout::default();
        let base = parked();
        for _ in 0..3 { layout.frame(&base, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
        let world = world_press(&mut layout, Button::Right, (120, 140));
        let parent = with_anchored_popup(scene::PopupAnchor { world, offset: (1, 1), world_scaled: false }, (120, 90));
        let before = layout.frame(&parent, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let place = before.popup_at.unwrap();
        let world_cameras: Vec<_> = layout.shown.iter().map(|(hit, camera)| (*hit, *camera)).collect();
        let mut nested = parent.clone();
        nested.interface.popup_anchor = None;
        nested.interface.popup = Some(259);
        nested.interface.modal_stack = scene::ModalStack([Some(257), Some(259), None, None]);
        nested.views.push(scene::ViewState { id: 259, view: scene::View { interface_only: true, origin: (500, 10), viewport: (240, 120), ..Default::default() }, overlay: vec![scene::FreeSprite::default()] });
        let frame = layout.frame(&nested, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(frame.popup_places.contains(&(257, place)), "the old click remains the parent's anchor");
        assert_eq!(layout.shown, world_cameras, "pushing a child never moves the world");
        let mut exposed = parent.clone();
        exposed.interface.popup_anchor = None; // The click may now be arbitrarily old.
        let frame = layout.frame(&exposed, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert_eq!(frame.popup_at, Some(place), "popping the child keeps the original parent placement");
        assert_eq!(layout.shown, world_cameras);
    }

    #[test]
    fn a_press_on_the_hud_outside_a_click_popup_only_dismisses_it() {
        let mut l = Layout::default();
        let s = parked();
        for _ in 0..3 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
        let w = world_press(&mut l, Button::Right, (100, 100));
        let open = with_anchored_popup(scene::PopupAnchor { world: w, offset: (1, 1), world_scaled: false }, (120, 90));
        l.frame(&open, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        l.moved.push(([600, 400, 100, 100], (0, 0)));
        assert_eq!(l.hit((620, 420)), Hit::Moved(l.moved.len() - 1));
        let zoom = l.main_view.zoom;
        l.ctrl = true;
        assert!(l.route(Input::Wheel { delta: 120, x: 620, y: 420 }).is_empty());
        assert_eq!(l.main_view.zoom, zoom);
        l.ctrl = false;
        assert_eq!(click(&mut l, (620, 420)), vec![Input::DismissPopup]);
        assert!(!click(&mut l, (130, 130)).contains(&Input::DismissPopup));
    }

    #[test]
    fn a_press_beside_a_popup_opened_at_a_world_click_only_dismisses_it() {
        let mut l = Layout::default();
        l.apply_settings(&settings(|s| (s.split, s.roles) = (S::Vertical1To1, Roles::UnitMouse)));
        let s = parked();
        for _ in 0..3 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
        let open_at_cursor = |l: &mut Layout| {
            // Captured in B, a right press at the world cursor opens it there.
            if !l.captured() { click(l, (600, 300)); }
            assert!(l.captured());
            l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            let at = l.cursor_on_window().unwrap();
            let w = world_press(l, Button::Right, at);
            let open = with_anchored_popup(scene::PopupAnchor { world: w, offset: (1, 1), world_scaled: false }, (120, 90));
            l.frame(&open, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            assert!(!l.captured() && l.click_popup_open(), "the mouse is let go for it");
            open
        };
        let open = open_at_cursor(&mut l);
        // A right press on A's world: it is dismissed, nothing else (no
        // world press, no look), its release goes nowhere.
        assert_eq!(l.route(Input::MouseButton { button: Button::Right, down: true, x: 100, y: 300 }), vec![Input::DismissPopup]);
        assert!(l.route(Input::MouseButton { button: Button::Right, down: false, x: 100, y: 300 }).is_empty());
        // Recapturing (a left click into B) dismisses it as well; the mouse
        // is captured once it is gone, never under it (`capture`).
        assert_eq!(click(&mut l, (600, 420)), vec![Input::DismissPopup]);
        assert!(!l.captured());
        menus_gone(&mut l, &s);
        assert!(l.captured() && !l.click_popup_open());
        let c = l.cursor;
        l.motion((40.0, 0.0));
        assert_ne!(l.cursor, c, "closed: the mouse plays again");
        // Ctrl+Shift+Space capturing it again dismisses it too.
        let _ = open;
        open_at_cursor(&mut l);
        mv(&mut l, (600, 300));
        let out = ctrl_shift(&mut l, 0x20);
        assert!(!l.captured());
        assert_eq!(out.iter().filter(|i| **i == Input::DismissPopup).count(), 1, "{out:?}");
        menus_gone(&mut l, &s);
        assert!(l.captured());
        // A dialog the driver does not anchor is no such popup.
        l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        let mut dialog = open_at_cursor(&mut l);
        dialog.interface.popup_anchor = None;
        l.frame(&dialog, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        assert!(!l.click_popup_open());
        assert!(!l.route(Input::MouseButton { button: Button::Right, down: true, x: 100, y: 300 }).contains(&Input::DismissPopup));
    }

    #[test]
    fn terrain_details_from_a_city_menu_open_at_the_menus_tile_however_long_it_was_open() {
        // The city's menu opened at a right click, read for five seconds;
        // its "Terrain details" opens the tile box the driver anchors at the
        // city's tile: there, in the view the menu was opened in (whether
        // the menu closed a frame before or the box took its place).
        for side in [Side::A, Side::B] {
            for zoom in [ONE, ONE - 2, ONE + 2] {
                for gap in [false, true] {
                    let what = format!("{side:?} zoom {:?} gap {gap}", ZOOMS[zoom]);
                    let mut l = Layout::default();
                    l.apply_settings(&settings(|s| s.split = S::Vertical1To1));
                    l.main_view.zoom = zoom;
                    l.split.as_mut().unwrap().view.zoom = zoom;
                    let s = parked();
                    for _ in 0..3 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
                    let (a, b) = l.panes();
                    let pane = if side == Side::A { a } else { b.unwrap() };
                    let p = (pane[0] + 90, pane[1] + 70);
                    let w = world_press(&mut l, Button::Right, p);
                    let menu = with_anchored_popup(scene::PopupAnchor { world: w, offset: (1, 1), world_scaled: false }, (120, 90));
                    for _ in 0..250 { l.advance(std::time::Duration::from_millis(20)); l.frame(&menu, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
                    if gap { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
                    let city = (w.0 - 50, w.1 - 20);
                    let tile_box = with_anchored_popup(scene::PopupAnchor { world: city, offset: (0, 0), world_scaled: true }, (256, 173));
                    let f = l.frame(&tile_box, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
                    let (corner, _) = f.popup_at.unwrap_or_else(|| panic!("{what}: not at the menu's tile"));
                    assert_eq!(corner, shown_at(&camera_at(&f, (pane[0], pane[1])), city, p, (8000, 0)), "{what}");
                }
            }
        }
    }

    #[test]
    fn popups_follow_the_one_screen_place_unless_opened_at_a_click() {
        for (place, menu_follows_pointer, want) in [
            (Place::LeftOrBottom, true, Some(Side::A)),
            (Place::RightOrTop, true, Some(Side::B)),
            (Place::RightOrTop, false, Some(Side::B)),
            (Place::Floating, true, Some(Side::B)),
            (Place::Floating, false, None),
        ] {
            let mut l = Layout::default();
            l.apply_settings(&settings(|s| {
                s.split = S::Vertical1To1;
                s.screen_place = ScreenPlace { vertical: place, horizontal: Place::Floating };
                s.menu_follows_pointer = menu_follows_pointer;
            }));
            let s = parked();
            for _ in 0..2 { l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS); }
            // The mouse in B when it opens.
            mv(&mut l, (600, 300));
            let w = world_press(&mut l, Button::Right, (600, 300));
            let mut dialog = with_anchored_popup(scene::PopupAnchor { world: w, offset: (1, 1), world_scaled: false }, (120, 90));
            dialog.interface.popup_anchor = None;
            let f = l.frame(&dialog, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            let (a, b) = l.panes();
            assert_eq!(f.popup_region, want.map(|s| if s == Side::A { a } else { b.unwrap() }), "{place:?} {menu_follows_pointer}");
            l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            // A context menu: at its click, whatever the setting.
            let w = world_press(&mut l, Button::Right, (600, 300));
            let f = l.frame(&with_anchored_popup(scene::PopupAnchor { world: w, offset: (1, 1), world_scaled: false }, (120, 90)), (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
            assert_eq!((f.popup_region, f.popup_at), (None, Some(((601, 301), (1, 1)))), "{place:?}");
            l.frame(&s, (800, 600), (0, 0, 0, 0), vec![], BOUNDS);
        }
    }

    /// A popup placed in either view of either split takes every click on
    /// it (also between its sprites, e.g. on its body around the OK
    /// button), never capturing the mouse; the capturing view's world not
    /// under it still captures.
    #[test]
    fn clicks_on_a_popup_in_either_view_reach_it_and_never_capture() {
        use viewer_settings::Resolution;
        // The popup's two sprites: a title strip and an OK button, a gap
        // of its plain body between them.
        let rect_of = |s: &scene::FreeSprite| Some(if s.x == 0 { [760, 440, 400, 40] } else { [900, 600, 120, 30] });
        for split in [S::Vertical1To1, S::Horizontal1To1] {
            let mut sides = Vec::new();
            for place in [Place::LeftOrBottom, Place::RightOrTop] {
                let plan = crate::scaling::plan((1920, 1080), (1920, 1080), &Resolution::default());
                let mut l = Layout::default();
                l.apply_settings(&settings(|s| {
                    (s.split, s.roles) = (split, Roles::UnitMouse);
                    s.screen_place = ScreenPlace { vertical: place, horizontal: place };
                }));
                l.driver_screen = Some(plan.screen);
                l.set_walls(Walls::Open);
                l.frame(&scene(), plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                let (a, b) = l.panes();
                let b = b.unwrap();
                // Captured in B when the popup opens.
                mv(&mut l, (b[0] + b[2] / 2, b[1] + b[3] / 2));
                click(&mut l, (b[0] + b[2] / 2, b[1] + b[3] / 2));
                assert!(l.captured());
                let mut popup = scene();
                popup.interface.popup = Some(257);
                let sprite = |x| scene::FreeSprite { x, ..Default::default() };
                popup.views.push(ViewState { id: 257, view: View { interface_only: true, origin: (760, 440), viewport: (400, 200), ..Default::default() }, overlay: vec![sprite(0), sprite(1)] });
                let mut f = l.frame(&popup, plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                let region = f.popup_region.expect("placed in a view");
                let side = if region == a { Side::A } else { assert_eq!(region, b); Side::B };
                sides.push(side);
                let arranged = crate::scaling::arrange_panned(&plan, &popup, &mut f, &mut l.menu_pan, rect_of);
                l.set_interface_shown(arranged.shown);
                let c = f.cameras[f.view_cameras[2].unwrap() as usize];
                let at = |dx: i32, dy: i32| (c.origin.0 + dx * c.viewport.0 as i32 / 400, c.origin.1 + dy * c.viewport.1 as i32 / 200);
                let what = format!("{split:?}, popup in {side:?}");
                assert!(!l.captured(), "{what}: opening the popup let go of the mouse");
                // The body between its sprites, then the OK button.
                for (p, driver) in [(at(20, 120), (780, 560)), (at(200, 175), (960, 615))] {
                    assert!(inside(region, p), "{what}: {p:?}");
                    mv(&mut l, p);
                    let out = click(&mut l, p);
                    assert!(!l.captured(), "{what}: a click on the popup at {p:?} captured the mouse");
                    match out.first() {
                        Some(&Input::MouseButton { button: Button::Left, down: true, x, y }) => {
                            assert!((x - driver.0).abs() <= 2 && (y - driver.1).abs() <= 2, "{what}: ({x}, {y}) vs {driver:?}");
                        }
                        other => panic!("{what}: the click on the popup went to {other:?}"),
                    }
                }
                // B's world beside the popup: the popup in A, a click there
                // captures the mouse (nothing to the game); the popup in B,
                // it does not (`capture`). Once the popup closes the mouse
                // is captured (again).
                let world = (b[0] + 8, b[1] + b[3] - 8);
                assert!(!overlap([c.origin.0, c.origin.1, c.viewport.0 as i32, c.viewport.1 as i32], [world.0, world.1, 1, 1]), "{what}");
                mv(&mut l, world);
                let out = click(&mut l, world);
                if side == Side::A {
                    assert!(l.captured(), "{what}: a click on B's world beside the popup in A captures");
                    assert_eq!(out, [], "{what}");
                    l.frame(&scene(), plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                    assert!(l.captured(), "{what}: stays captured as the popup closes");
                    continue;
                }
                assert!(!l.captured(), "{what}: a click on B's world beside the popup in B does not capture");
                l.frame(&scene(), plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                assert!(!l.captured(), "{what}: not at once");
                for _ in 0..3 {
                    l.advance(std::time::Duration::from_millis(100));
                    l.frame(&scene(), plan.canvas(), (0, 0, 0, 0), vec![], BOUNDS);
                }
                assert!(l.captured(), "{what}: captured again once the popup is gone");
            }
            sides.sort_by_key(|s| *s == Side::B);
            assert_eq!(sides, [Side::A, Side::B], "{split:?}");
        }
    }
}

impl Layout {
    /// Adopt only the routing geometry of the frame just presented. Live
    /// pointer, keys, gestures and desired camera changes may have advanced
    /// while its immutable GPU packet was in flight.
    pub(crate) fn adopt_presented_mapping(&mut self, frame: Self) {
        self.fit = frame.fit;
        self.placed_screen = frame.placed_screen;
        self.screen_content = frame.screen_content;
        self.menu_side = frame.menu_side;
        self.popup_side = frame.popup_side;
        self.menu_cover = frame.menu_cover;
        self.walls = frame.walls;
        self.reachable = frame.reachable;
        self.screen = frame.screen;
        self.popup_shown = frame.popup_shown;
        self.popup_body = frame.popup_body;
        self.popup_view = frame.popup_view;
        self.anchored_popup_layers = frame.anchored_popup_layers;
        self.popup_anchor = frame.popup_anchor;
        self.anchored = frame.anchored;
        self.popup_anchor_seen = frame.popup_anchor_seen;
        self.chain_press = frame.chain_press;
        self.popup_pan = frame.popup_pan;
        self.world_period = frame.world_period;
        self.magnifier_shown = frame.magnifier_shown;
        self.world_shown = frame.world_shown;
        self.beside = frame.beside;
        self.beside_mapping = frame.beside_mapping;
        self.focus = frame.focus;
        self.world_downscale = frame.world_downscale;
        self.map_wheel_step = frame.map_wheel_step;
        self.minimap = frame.minimap;
        self.window = frame.window;
        self.main = frame.main;
        self.shown = frame.shown;
        self.interface = frame.interface;
        self.driver_rects = frame.driver_rects;
        self.insets_shown = frame.insets_shown;
        self.left_out = frame.left_out;
        self.bounds = frame.bounds;
        self.interface_shown = frame.interface_shown;
        self.driver_screen = frame.driver_screen;
        self.menu_scale = frame.menu_scale;
        self.hud_scale = frame.hud_scale;
        self.art_zooms = frame.art_zooms;
    }
}

#[cfg(test)]
mod presented_mapping_tests {
    use super::*;
    #[test]
    fn presenting_a_minimap_installs_and_removes_its_click_area() {
        let mut active = Layout::default();
        let mut submitted = active.clone();
        submitted.minimap = Some(scene::Minimap { at: (30, 960), size: (200, 100), shown: [30, 960, 200, 100], world: (8000, 4000) });
        let expected = submitted.minimap;
        active.adopt_presented_mapping(submitted);
        assert_eq!(active.minimap, expected);
        let mut closed = active.clone();
        closed.minimap = None;
        active.adopt_presented_mapping(closed);
        assert_eq!(active.minimap, None);
    }

    #[test]
    fn presenting_an_older_packet_keeps_newer_pointer_and_keys_but_installs_its_geometry() {
        let mut active = Layout::default();
        active.pointer = (77, 88);
        active.raw_pointer = (79, 90);
        active.keys_down = vec![17, 65];
        let mut submitted = active.clone();
        submitted.pointer = (10, 20);
        submitted.raw_pointer = (10, 20);
        submitted.keys_down.clear();
        submitted.window = (640, 480);
        submitted.interface = vec![[50, 60, 70, 80]];
        active.adopt_presented_mapping(submitted);
        assert_eq!(active.pointer, (77, 88));
        assert_eq!(active.raw_pointer, (79, 90));
        assert_eq!(active.keys_down, [17, 65]);
        assert_eq!(active.window, (640, 480));
        assert_eq!(active.interface, [[50, 60, 70, 80]]);
    }
}
