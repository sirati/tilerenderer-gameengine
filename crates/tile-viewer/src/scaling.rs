//! How the viewer shows its picture and the driver's interface in its
//! window, at the user's resolution settings (`viewer_settings::Resolution`).
//!
//! The viewer renders at its window's size (physical pixels) divided by
//! the pixel scale `n` (a whole number, 1 by default whatever the
//! desktop's scale), the canvas, and shows each rendered pixel as an
//! exact `n`×`n` box of window pixels (`tile_render::Present`; what is
//! left of the window when it does not divide is a border of fewer than
//! `n` pixels). Nothing is filtered or scaled by a fraction there.
//!
//! The world is the viewer's own: the layout arranges it in the canvas's
//! pixels, its main view around the middle of the driver's view and as far
//! out as the canvas reaches (a larger canvas shows more of the world), at
//! `world_scale` (`Layout::world_scale`). The driver's screen (the game's
//! window, set when it starts) is only where its interface is drawn and
//! where its pointer input goes: its interface is drawn at its own pixel
//! size times the frame and menu scales, anchored to the canvas: menus
//! (screens, the popup) about the middle, the frame's panels each at its
//! side or corner. Pointer input maps back through the same pieces. The
//! part scales are nearest-pixel zooms like the viewer's own (crisp, never
//! filtered).
//!
//! The canvas is rendered at its own size and enlarged once onto the
//! window ([`drawn`]): a pixel scale `n` renders `n²` times fewer pixels.
//! Text from outlines is drawn at the canvas's resolution too.

use scene::{FreeSprite, Scene};
use tile_render::{Camera, Present};
use viewer_settings::Resolution;

use crate::layout;

/// A fraction (numerator, denominator).
pub type Ratio = (u32, u32);

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

/// The largest numerator or denominator of a zoom. The renderer multiplies
/// pixel coordinates and picture sizes by them in 32 bits (and packs a
/// zoom into 16 bits a part): an arbitrary percent of an arbitrary canvas
/// height reduces to millions (60.758 % of 632 rows: 4799941/31606180),
/// and such a picture overflowed and vanished.
pub const MAX_ZOOM_TERM: u64 = 4096;

/// `n / d` reduced, or, where that needs larger terms than
/// [`MAX_ZOOM_TERM`], the closest fraction with such terms not above it
/// (a lower semiconvergent of its continued fraction; never larger, so
/// what fits at `n / d` still fits).
fn reduced(n: u64, d: u64) -> Ratio {
    let (n, d) = (n.max(1), d.max(1));
    let g = gcd(n, d);
    let (n, d) = (n / g, d / g);
    if n <= MAX_ZOOM_TERM && d <= MAX_ZOOM_TERM {
        return (n as u32, d as u32);
    }
    let (n, d) = (n as u128, d as u128);
    let limit = MAX_ZOOM_TERM as u128;
    // Stern–Brocot bounds lo ≤ n/d < hi, moved in whole runs of steps.
    let (mut lo, mut hi) = ((0u128, 1u128), (1u128, 0u128));
    loop {
        // Steps towards hi: lo + k·hi stays ≤ n/d for k ≤ kx.
        let below = |k: u128| (lo.0 + k * hi.0) * d <= n * (lo.1 + k * hi.1);
        let bound = |k: u128| lo.0 + k * hi.0 <= limit && lo.1 + k * hi.1 <= limit;
        let mut k = 0u128;
        // (Each run is short in log terms; find it by doubling then halving.)
        let mut step = 1u128;
        while below(k + step) && bound(k + step) { k += step; step *= 2; }
        while step > 1 { step /= 2; if below(k + step) && bound(k + step) { k += step; } }
        let moved_lo = k > 0;
        lo = (lo.0 + k * hi.0, lo.1 + k * hi.1);
        if lo.0 * d == n * lo.1 { break; }
        // Steps towards lo: hi + k·lo stays > n/d.
        let above = |k: u128| (hi.0 + k * lo.0) * d > n * (hi.1 + k * lo.1);
        let bound = |k: u128| hi.0 + k * lo.0 <= limit && hi.1 + k * lo.1 <= limit;
        let (mut k, mut step) = (0u128, 1u128);
        while above(k + step) && bound(k + step) { k += step; step *= 2; }
        while step > 1 { step /= 2; if above(k + step) && bound(k + step) { k += step; } }
        hi = (hi.0 + k * lo.0, hi.1 + k * lo.1);
        if !moved_lo && k == 0 { break; }
    }
    ((lo.0 as u32).max(1), (lo.1 as u32).max(1))
}

/// The product of two fractions, reduced.
pub fn times(a: Ratio, b: Ratio) -> Ratio {
    reduced(a.0.max(1) as u64 * b.0.max(1) as u64, a.1.max(1) as u64 * b.1.max(1) as u64)
}

/// A part scale setting as a fraction (in sixtieths: halves, thirds,
/// quarters, fifths, tenths are exact), between 1/4 and 8.
pub fn ratio(scale: f32) -> Ratio {
    let scale = if scale.is_finite() { scale.clamp(0.25, 8.0) } else { 1.0 };
    reduced((scale * 60.0).round() as u64, 60)
}

/// The largest fraction `inner` can be scaled by to fit into `outer`
/// (keeping its shape), reduced.
fn fit(inner: (u32, u32), outer: (u32, u32)) -> Ratio {
    let (iw, ih) = (inner.0.max(1) as u64, inner.1.max(1) as u64);
    let (ow, oh) = (outer.0.max(1) as u64, outer.1.max(1) as u64);
    if ow * ih <= oh * iw { reduced(ow, iw) } else { reduced(oh, ih) }
}

/// Keep the requested ratio unless the actual content would be clipped.
pub(crate) fn fit_at_most(zoom: Ratio, bounds: Option<[i32; 4]>, region: (u32, u32)) -> Ratio {
    let Some([_, _, w, h]) = bounds.filter(|r| r[2] > 0 && r[3] > 0) else { return zoom };
    let most = fit((w as u32, h as u32), region);
    if zoom.0 as u64 * most.1 as u64 > most.0 as u64 * zoom.1 as u64 { most } else { zoom }
}

/// `v` scaled by `r`, rounded down.
fn scaled(v: i64, r: Ratio) -> i64 {
    (v * r.0 as i64).div_euclid(r.1.max(1) as i64)
}

/// A map from the driver's pixels to the canvas's: `p ↦ to + (p − from) · zoom`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Affine {
    pub from: (i32, i32),
    pub to: (i32, i32),
    pub zoom: Ratio,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { from: (0, 0), to: (0, 0), zoom: (1, 1) };

    /// The canvas pixel showing the driver's pixel `p` (its top-left).
    pub fn apply(&self, p: (i32, i32)) -> (i32, i32) {
        let axis = |v: i32, from: i32, to: i32| to + scaled((v - from) as i64, self.zoom) as i32;
        (axis(p.0, self.from.0, self.to.0), axis(p.1, self.from.1, self.to.1))
    }

    /// The driver's pixel shown at canvas pixel `p`.
    pub fn invert(&self, p: (i32, i32)) -> (i32, i32) {
        let back = (self.zoom.1, self.zoom.0);
        let axis = |v: i32, from: i32, to: i32| from + scaled((v - to) as i64, back) as i32;
        (axis(p.0, self.from.0, self.to.0), axis(p.1, self.from.1, self.to.1))
    }

    /// The same map measured from the driver's pixel `o`: where the
    /// renderer rounds a camera's origin (`Affine::camera`), so for a
    /// camera at `o` each canvas pixel shows exactly the driver's pixel
    /// [`Affine::invert`] gives there (clicks, hover and Alt targets agree
    /// with what is drawn, to the pixel).
    pub fn at(&self, o: (i32, i32)) -> Affine {
        Affine { from: o, to: self.apply(o), zoom: self.zoom }
    }

    /// Rectangle `r` (x, y, w, h) of the driver's pixels on the canvas.
    pub fn rect(&self, [x, y, w, h]: [i32; 4]) -> [i32; 4] {
        let (x0, y0) = self.apply((x, y));
        let (x1, y1) = self.apply((x + w, y + h));
        [x0, y0, (x1 - x0).max(0), (y1 - y0).max(0)]
    }

    /// The canvas pixels showing a pixel of driver rectangle `r`: those
    /// whose [`Affine::invert`] (the pixel the renderer samples there) is
    /// in it. Each edge rounds up, as the renderer's spans do.
    pub fn drawn(&self, [x, y, w, h]: [i32; 4]) -> [i32; 4] {
        let up = |v: i32, from: i32, to: i32| to - scaled(-((v - from) as i64), self.zoom) as i32;
        let (x0, y0) = (up(x, self.from.0, self.to.0), up(y, self.from.1, self.to.1));
        let (x1, y1) = (up(x + w, self.from.0, self.to.0), up(y + h, self.from.1, self.to.1));
        [x0, y0, (x1 - x0).max(0), (y1 - y0).max(0)]
    }

    /// Canvas rectangle `r` in the driver's pixels (the smallest holding it).
    pub fn rect_back(&self, [x, y, w, h]: [i32; 4]) -> [i32; 4] {
        let (x0, y0) = self.invert((x, y));
        let (x1, y1) = self.invert((x + w - 1, y + h - 1));
        [x0, y0, x1 - x0 + 1, y1 - y0 + 1]
    }

    /// `camera` (in the driver's pixels) as it shows on the canvas (its
    /// origin rounded to a pixel). Its world pixels stay what they are; its
    /// screen sprites and clip rectangles move with its origin
    /// (`Camera::screen_shift`).
    pub fn camera(&self, camera: &Camera) -> Camera {
        if *self == Affine::IDENTITY {
            return *camera;
        }
        let origin = self.apply(camera.origin);
        let size = |v: u32| (v as u64 * self.zoom.0 as u64).div_ceil(self.zoom.1.max(1) as u64).max(1) as u32;
        let (sx, sy) = camera.screen_shift;
        Camera {
            zoom: times(camera.zoom, self.zoom),
            origin,
            viewport: (size(camera.viewport.0), size(camera.viewport.1)),
            screen_shift: (sx + origin.0 - camera.origin.0, sy + origin.1 - camera.origin.1),
            ..*camera
        }
    }
}

/// How the viewer shows things (see the module docs).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The canvas (what the layout arranges and the renderer draws) and how
    /// it is shown in the window: `n`×`n` boxes.
    pub present: Present,
    /// The driver's screen: where its interface is and its input goes.
    pub screen: (u32, u32),
    /// Its menus, from the middle of its screen to the middle of the canvas.
    pub menu: Affine,
    /// User menus use a canvas-relative percentage and fit their available
    /// region. With this off, only semantic mandatory-fit screens shrink.
    pub fit_menus: bool,
    /// The zoom of its frame's panels (each about its own side or corner).
    pub frame: Ratio,
    /// The world's (`Layout::world_scale`).
    pub world: Ratio,
    /// The menu content [`Plan::for_scene`] fitted (driver pixels), for
    /// the log.
    pub menu_bounds: Option<[i32; 4]>,
}

impl Plan {
    /// Select the final ratio from the actual menu content before the
    /// layout reserves space for a side-by-side scenario preview.
    pub fn for_scene(mut self, scene: &Scene, rect_of: impl Fn(&FreeSprite) -> Option<[i32; 4]>) -> Self {
        if self.fit_menus || scene.interface.menu_fit {
            self.menu_bounds = menu_bounds(scene, self.screen, rect_of);
            self.menu.zoom = fit_at_most(self.menu.zoom, self.menu_bounds, self.canvas());
        }
        self
    }

    /// The canvas.
    pub fn canvas(&self) -> (u32, u32) {
        self.present.canvas
    }

    /// The map of a frame panel anchored at `anchor` (each way 0: the
    /// start, 1: the middle, 2: the end): that place of the driver's
    /// screen at that place of the canvas.
    pub fn frame_piece(&self, anchor: (u8, u8)) -> Affine {
        let at = |a: u8, size: u32| (a as i64 * size as i64 / 2) as i32;
        let canvas = self.canvas();
        Affine { from: (at(anchor.0, self.screen.0), at(anchor.1, self.screen.1)), to: (at(anchor.0, canvas.0), at(anchor.1, canvas.1)), zoom: self.frame }
    }

    /// Window point `p` (physical pixels) in the canvas's (the layout's):
    /// on the picture's edge for points in the window beside it.
    pub fn to_screen(&self, p: (f64, f64)) -> (i32, i32) {
        self.present.to_canvas(p)
    }

    /// Where canvas pixel `p` (the layout's) shows in the window: the
    /// physical pixel in the middle of its box.
    pub fn to_window(&self, p: (i32, i32)) -> (i32, i32) {
        let (n, d) = (self.present.zoom.0.max(1) as i64, self.present.zoom.1.max(1) as i64);
        let axis = |v: i32, at: i32| at + ((2 * v as i64 + 1) * n / (2 * d)) as i32;
        (axis(p.0, self.present.at.0), axis(p.1, self.present.at.1))
    }

    /// Window pixels per canvas pixel.
    pub fn factor(&self) -> f64 {
        self.present.factor()
    }
}

/// The plan for a window of `window` physical pixels (the desktop's scale
/// plays no part), a driver's screen of `screen` ((0, 0): unknown, then
/// the canvas) and `settings`.
pub fn plan(window: (u32, u32), screen: (u32, u32), settings: &Resolution) -> Plan {
    let window = (window.0.max(1), window.1.max(1));
    let n = settings.pixel_scale_in(window).max(1);
    let canvas = settings.realised(window);
    let reference = if screen.0 > 0 && screen.1 > 0 { (screen.1 as u64).min(768) } else { 768 };
    let screen = if screen.0 > 0 && screen.1 > 0 { screen } else { canvas };
    // (What is left of the window: a border of fewer than n pixels.)
    let at = ((window.0 - canvas.0 * n) as i32 / 2, (window.1 - canvas.1 * n) as i32 / 2);
    let present = Present { canvas, zoom: (n, 1), at, target: window };
    let center = |s: (u32, u32)| ((s.0 / 2) as i32, (s.1 / 2) as i32);
    // A standard menu is 1024×768 native pixels. The setting selects its
    // height relative to the visible canvas, independent of pixel scale.
    // Tiny context menus retain this reference height rather than filling
    // the window by themselves. Actual menu bounds cap the ratio later.
    // A driver's screen shorter than that cuts its menus to its height:
    // there they are that tall (a 720-row screen's title fills the canvas
    // at 100 %).
    let zoom = if settings.scale_menus {
        reduced(canvas.1 as u64 * (settings.effective_menu_scale_percent() * 1000.0).round() as u64, reference * 100_000)
    } else { (1, 1) };
    let menu = Affine { from: center(screen), to: center(canvas), zoom };
    Plan { present, screen, menu, fit_menus: settings.scale_menus, frame: ratio(settings.frame_scale), world: ratio(settings.world_scale), menu_bounds: None }
}

/// Actual menu bounds in driver pixels. A dedicated screen camera or
/// Beside rectangle is authoritative; otherwise use the menu sprite union.
fn menu_bounds(scene: &Scene, screen: (u32, u32), rect_of: impl Fn(&FreeSprite) -> Option<[i32; 4]>) -> Option<[i32; 4]> {
    if let Some(beside) = scene.interface.beside { return Some(beside.screen); }
    if let Some(view) = scene.interface.screen_view.and_then(|id| scene.view(id)).map(|v| v.view)
        .filter(|v| v.viewport.0 > 0 && v.viewport.1 > 0) {
        return Some([view.origin.0, view.origin.1, view.viewport.0 as i32, view.viewport.1 as i32]);
    }
    let rects: Vec<_> = scene.views.iter().filter(|v| v.view.interface_only && v.view.menu)
        .flat_map(|v| v.overlay.iter().filter_map(&rect_of)).collect();
    content_bounds(&rects, screen)
}

/// The bounds of a menu's content. Between games the whole game window is
/// a menu, and the game clears all of it before drawing the 1024×768
/// title or setup screen in its middle. Pieces covering the whole driver's
/// screen are that backdrop, not the menu's size: fitting them would keep
/// the title at the screen's height instead of the menu's. A screen no
/// larger than a standard menu is the menu itself. The game also draws a
/// few small pieces at the window's own edges (the version number in its
/// bottom left corner, a bar at the bottom): only pieces reaching into the
/// standard menu in the middle of the screen count, or the title would be
/// fitted as if it were nearly window-sized.
fn content_bounds(rects: &[[i32; 4]], screen: (u32, u32)) -> Option<[i32; 4]> {
    let (w, h) = (screen.0 as i32, screen.1 as i32);
    let larger = w > 1024 || h > 768;
    let backdrop = |r: &[i32; 4]| larger && r[0] <= 0 && r[1] <= 0 && r[0] + r[2] >= w && r[1] + r[3] >= h;
    let (ml, mt) = ((w - 1024) / 2, (h - 768) / 2);
    let in_menu = |r: &[i32; 4]| !larger || (r[0] + r[2] > ml && r[0] < ml + 1024 && r[1] + r[3] > mt && r[1] < mt + 768);
    let content: Vec<[i32; 4]> = rects.iter().copied().filter(|r| !backdrop(r) && in_menu(r)).collect();
    let b = bounding(if content.is_empty() { rects } else { &content })?;
    // Only what is on the driver's screen is drawn (a 768-row title on a
    // 720-row screen is cut there): fit and centre that.
    let (x0, y0, x1, y1) = (b[0].max(0), b[1].max(0), (b[0] + b[2]).min(w), (b[1] + b[3]).min(h));
    Some(if w > 0 && h > 0 && x1 > x0 && y1 > y0 { [x0, y0, x1 - x0, y1 - y0] } else { b })
}

/// Pans an oversized menu at its requested scale rather than shrinking its
/// pixels to fit. A middle-button drag never reaches the driver. The same
/// offset is used for rendering, hover and click mapping.
#[derive(Clone, Debug, Default)]
pub struct MenuPan {
    key: Option<(Option<String>, Option<u32>)>,
    offset: (i32, i32),
    /// Allowed offset on each axis.
    bounds: [(i32, i32); 2],
    rect: Option<[i32; 4]>,
    clip: Option<[i32; 4]>,
    drag: Option<((i32, i32), (i32, i32))>,
    hover_refresh: bool,
}

impl MenuPan {
    fn place(&mut self, key: (Option<String>, Option<u32>), rect: Option<[i32; 4]>, canvas: (u32, u32)) -> (i32, i32) {
        self.place_in(key, rect, [0, 0, canvas.0 as i32, canvas.1 as i32])
    }

    fn place_in(&mut self, key: (Option<String>, Option<u32>), rect: Option<[i32; 4]>, region: [i32; 4]) -> (i32, i32) {
        if self.key.as_ref() != Some(&key) {
            self.offset = (0, 0);
            self.key = Some(key);
            self.cancel_drag();
        }
        let bounds = rect.map_or([(0, 0); 2], |r| {
            let axis = |at: i32, len: i32, start: i32, span: i32| if len > span {
                (start + span - at - len, start - at)
            } else { (0, 0) };
            [axis(r[0], r[2], region[0], region[2]), axis(r[1], r[3], region[1], region[3])]
        });
        if bounds != self.bounds {
            self.cancel_drag();
        }
        self.bounds = bounds;
        self.clip = Some(region);
        self.offset = (self.offset.0.clamp(self.bounds[0].0, self.bounds[0].1), self.offset.1.clamp(self.bounds[1].0, self.bounds[1].1));
        self.rect = rect.map(|r| [r[0] + self.offset.0, r[1] + self.offset.1, r[2], r[3]]);
        self.offset
    }

    /// Ends a drag when the window loses the mouse or the menu changes.
    pub fn cancel_drag(&mut self) {
        self.drag = None;
        self.hover_refresh = false;
    }

    pub fn is_dragging(&self) -> bool { self.drag.is_some() }

    /// Restores hover after a drag, once the panned input map is drawn.
    pub fn take_hover_refresh(&mut self) -> bool {
        std::mem::take(&mut self.hover_refresh)
    }

    /// Whether the input belongs to panning the menu. Other buttons and
    /// ordinary scrolling retain their driver behaviour.
    pub fn route(&mut self, input: &scene::Input) -> bool {
        use scene::{Button, Input};
        match *input {
            Input::MouseButton { button: Button::Middle, down: false, .. } => {
                let consumed = self.drag.take().is_some();
                self.hover_refresh |= consumed;
                consumed
            }
            Input::MouseButton { button: Button::Middle, down: true, x, y }
                if self.bounds.iter().any(|&(a, b)| a != b)
                    && self.clip.is_none_or(|r| x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3])
                    && self.rect.is_some_and(|r| x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3]) => {
                self.drag = Some(((x, y), self.offset));
                self.hover_refresh = false;
                true
            }
            Input::MouseMove { x, y } => {
                let Some((from, offset)) = self.drag else { return false };
                self.offset = ((offset.0 + x - from.0).clamp(self.bounds[0].0, self.bounds[0].1),
                    (offset.1 + y - from.1).clamp(self.bounds[1].0, self.bounds[1].1));
                true
            }
            _ => false,
        }
    }
}

/// One piece of the driver's interface (or one of its other views) as
/// shown: where it takes the pointer (the canvas's pixels), where that is
/// in the driver's window, and back.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ShownPiece {
    pub rect: [i32; 4],
    /// The view's visible viewport, independent of this sprite's ink bounds.
    pub clip: [i32; 4],
    /// Its rectangle in the driver's window.
    pub driver: [i32; 4],
    piece: Affine,
    /// How far the layout moved the piece (`Camera::screen_shift`, e.g. a
    /// popup moved into a pane).
    shift: (i32, i32),
    /// Under a modal popup shown elsewhere ([`MODAL_TOP`]): that popup's
    /// rectangle in the driver's window. The pointer on this piece never
    /// lands there (the popup is no longer where the driver put it).
    under_modal: Option<[i32; 4]>,
    /// A piece of the modal popup over a menu ([`MODAL_TOP`]).
    modal: bool,
}

impl ShownPiece {
    /// Whether it is a piece of the modal popup over a menu: over a screen
    /// the layout fitted into a pane too, it takes the pointer first.
    pub fn is_modal(&self) -> bool {
        self.modal
    }

    /// The driver's pixel `(x, y)` the pointer reaches beside a modal popup
    /// over this piece at `modal` (the driver's window): the game takes a
    /// click off its modal form as nothing.
    pub fn beside_modal((x, y): (i32, i32), [mx, my, mw, mh]: [i32; 4]) -> (i32, i32) {
        if x >= mx && y >= my && x < mx + mw && y < my + mh { (if mx > 0 { mx - 1 } else { mx + mw }, y) } else { (x, y) }
    }

    /// The driver's pixel for canvas pixel `p` on the piece.
    pub fn to_driver(&self, p: (i32, i32)) -> (i32, i32) {
        let (x, y) = self.piece.invert(p);
        (x - self.shift.0, y - self.shift.1)
    }

    /// Where the pointer at canvas pixel `p` on the piece goes in the
    /// driver's window: its pixel there, but never onto a modal popup over
    /// it that is shown elsewhere.
    pub fn pointer_to_driver(&self, p: (i32, i32)) -> (i32, i32) {
        let (x, y) = self.to_driver(p);
        // (Just beside it: the game takes a click off its modal form as
        // nothing.)
        self.under_modal.map_or((x, y), |m| Self::beside_modal((x, y), m))
    }

    /// Driver rectangle `r` (within the piece) on the canvas, in the
    /// canvas's pixels (fractions kept).
    pub fn to_canvas(&self, [x, y, w, h]: [i32; 4]) -> [f64; 4] {
        let s = self.piece.zoom.0 as f64 / self.piece.zoom.1.max(1) as f64;
        let x0 = self.piece.to.0 as f64 + (x + self.shift.0 - self.piece.from.0) as f64 * s;
        let y0 = self.piece.to.1 as f64 + (y + self.shift.1 - self.piece.from.1) as f64 * s;
        [x0, y0, w as f64 * s, h as f64 * s]
    }
}

/// What [`arrange`] adds to the layout's frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arranged {
    /// Per scene view (its index) whose overlay is drawn by several
    /// cameras: each overlay sprite's camera.
    pub overlay_cameras: Vec<(usize, Vec<u32>)>,
    /// The interface as shown, for the layout (`Layout::interface_shown`);
    /// `None` where the driver's screen is fitted into a pane of the
    /// layout's (it maps that itself).
    pub shown: Option<Vec<ShownPiece>>,
    /// A camera over the whole canvas with nothing of its own, for the
    /// viewer's own screen sprites (e.g. the snapping pointer, in the
    /// canvas's pixels).
    pub overlay: Option<u32>,
}

/// Interface sprites this close belong to the same panel.
const PANEL_GAP: i32 = 6;

/// A modal popup over a menu (the Preferences screen, the title, a setup
/// screen; not one anchored at a click) has its top this far down the
/// menu's region (the window, unless the menu keeps to a pane): in the
/// top third, across in the middle.
pub const MODAL_TOP: Ratio = (1, 6);

/// Where a modal popup of `size` (canvas pixels) over a menu goes in
/// `region`: across in its middle, its top [`MODAL_TOP`] of the way down,
/// moved up as far as needed to be whole there (in the middle where it is
/// taller than the region).
fn modal_place((w, h): (i32, i32), [x, y, rw, rh]: [i32; 4]) -> (i32, i32) {
    let top = (rh as i64 * MODAL_TOP.0 as i64 / MODAL_TOP.1.max(1) as i64) as i32;
    (x + (rw - w) / 2, y + if h <= rh { top.min(rh - h) } else { (rh - h) / 2 })
}

/// The driver's interface and other views (cameras of the layout's
/// `frame` in the driver's pixels) shown on the canvas as `plan` says: the
/// menus and the driver's views within them about the middle (a popup
/// about its own middle, where the layout put it), the frame's panels each
/// at its side or corner (one camera per side or corner in use), the
/// roaming controls at their top-left (`scene::Interface`). Oversized menus
/// retain their requested scale; their viewport can be panned with the
/// middle button. The viewer's own views (the world, panes, insets) are
/// the canvas's already. `rect_of`: a screen sprite's rectangle.
pub fn arrange(plan: &Plan, scene: &Scene, frame: &mut layout::Frame, rect_of: impl Fn(&FreeSprite) -> Option<[i32; 4]>) -> Arranged {
    arrange_panned(plan, scene, frame, &mut MenuPan::default(), rect_of)
}

/// Arranges the interface with a persistent oversized-menu pan position.
pub fn arrange_panned(plan: &Plan, scene: &Scene, frame: &mut layout::Frame, pan: &mut MenuPan, rect_of: impl Fn(&FreeSprite) -> Option<[i32; 4]>) -> Arranged {
    if let Some(beside) = frame.beside_screen {
        return arrange_beside(plan, scene, frame, pan, beside, rect_of);
    }
    let mut out = Arranged::default();
    // A driver's screen fitted into a pane of the layout's (its views
    // stacked, drawn into its picture): the layout placed it all.
    if frame.placed.is_empty() && frame.cameras.iter().any(|c| c.stacked) {
        pan.place((None, None), None, plan.canvas());
        return out;
    }
    let canvas = plan.canvas();
    let parts = &scene.interface;
    let menu_region = frame.menu_region;
    let popup_region = frame.popup_region;
    let popup_at = frame.popup_at;
    let canvas_rect = [0, 0, canvas.0 as i32, canvas.1 as i32];
    // Per camera: its map, if it is one of the driver's; or (a frame in
    // panels) the scene view and per panel its map and rectangle, per
    // sprite its panel.
    let mut pieces: Vec<Option<Affine>> = vec![None; frame.cameras.len()];
    let mut clips: Vec<Option<[i32; 4]>> = vec![None; frame.cameras.len()];
    let mut panelled: Vec<(usize, usize, Vec<(Affine, [i32; 4])>, Vec<usize>)> = Vec::new();
    // What takes the pointer: per rectangle (the driver's pixels) its map
    // and how far the layout moved it.
    let mut shown: Vec<(Affine, [i32; 4], (i32, i32), Option<[i32; 4]>)> = Vec::new();
    // HUD panels stay on the canvas; menus retain their requested scale
    // and use a pannable viewport when larger than the canvas.
    let fitting = |zoom: Ratio, rect: Option<[i32; 4]>| match rect {
        Some([_, _, w, h]) if w > 0 && h > 0 => {
            let most = fit((w as u32, h as u32), canvas);
            if zoom.0 as u64 * most.1 as u64 > most.0 as u64 * zoom.1 as u64 { most } else { zoom }
        }
        _ => zoom,
    };
    // A piece moved as little as needed for `r` (the driver's pixels) to be
    // shown whole on the canvas.
    let kept = |piece: Affine, r: [i32; 4]| {
        let [x, y, w, h] = piece.rect(r);
        let into = |at: i32, len: i32, side: u32| if len <= side as i32 { at.clamp(0, side as i32 - len) - at } else { 0 };
        Affine { to: (piece.to.0 + into(x, w, canvas.0), piece.to.1 + into(y, h, canvas.1)), ..piece }
    };
    // (The same, inside region `[x, y, w, h]` of the canvas.)
    let kept_in = |piece: Affine, r: [i32; 4], region: [i32; 4]| {
        let [x, y, w, h] = piece.rect(r);
        let into = |at: i32, len: i32, lo: i32, side: i32| if len <= side { at.clamp(lo, lo + side - len) - at } else { 0 };
        Affine { to: (piece.to.0 + into(x, w, region[0], region[2]), piece.to.1 + into(y, h, region[1], region[3])), ..piece }
    };
    let menu_rects: Vec<[i32; 4]> = scene.views.iter().enumerate().filter(|(i, v)| v.view.interface_only && v.view.menu && frame.view_cameras.get(*i).copied().flatten().is_some_and(|c| !frame.placed.contains(&c))).map(|(_, v)| v).flat_map(|v| v.overlay.iter().filter_map(&rect_of)).collect();
    let menu_box = content_bounds(&menu_rects, plan.screen);
    // (About the middle, but moved as little as needed to be whole on the
    // canvas: e.g. a menu at the top of a screen larger than the canvas.)
    let fits = plan.fit_menus || parts.menu_fit;
    let region_size = |region: Option<[i32; 4]>| region.map_or(canvas, |r| (r[2].max(1) as u32, r[3].max(1) as u32));
    let menu_base = Affine { zoom: if fits { fit_at_most(plan.menu.zoom, menu_box, region_size(menu_region)) } else { plan.menu.zoom }, ..plan.menu };
    let menu = menu_box.map_or(menu_base, |b| match menu_region {
        Some([x, y, w, h]) => Affine { from: (b[0] + b[2] / 2, b[1] + b[3] / 2), to: (x + w / 2, y + h / 2), zoom: menu_base.zoom },
        None => kept(menu_base, b),
    });
    // Measured from the content's top-left corner: the renderer rounds the
    // menu camera's origin from there, so the pixels start on the same row
    // and column as the rectangle used for input and Alt targets (a 100 %
    // menu begins at the canvas's top, without a dark first row).
    let menu = menu_box.map_or(menu, |b| Affine { from: (b[0], b[1]), to: menu.apply((b[0], b[1])), ..menu });
    // The driver's other views outside its menus: about the middle, as
    // large as they are.
    let middle = Affine { zoom: (1, 1), ..plan.menu };
    // A modal popup over a menu (over the Preferences screen, the title or
    // a setup screen; not one anchored at a click): the menu keeps its own
    // fit and place, as without it; the popup goes over it at its zoom,
    // across in the middle of its region, its top `MODAL_TOP` down.
    // A popup the driver opened at a click on its named screen (a city
    // screen garrison unit's menu) is no modal: it stays where the driver
    // put it on that screen, placed and scaled with it.
    let on_screen = parts.popup_on_screen && parts.screen.is_some() && parts.screen_view.is_none();
    // (The popup placed: the dialog on top, with what is attached to it,
    // its combo box's open list: `scene::Interface::placed_popup`.)
    let placed_popup = parts.placed_popup();
    let modal = placed_popup.filter(|_| popup_at.is_none() && !on_screen && (parts.screen.is_some() || parts.screen_view.is_some() || menu_box.is_some()));
    let camera_of = |id: u32| scene.views.iter().position(|v| v.id == id)
        .and_then(|i| frame.view_cameras.get(i).copied().flatten())
        .map(|c| frame.cameras[c as usize])
        .filter(|c| c.viewport.0 > 0 && c.viewport.1 > 0);
    let modal_region = menu_region.unwrap_or(canvas_rect);
    // (Over a screen the layout fitted into a pane: at that screen's zoom,
    // its group's head's.)
    let placed_zoom = modal.and_then(|_| frame.placed.first())
        .map(|&head| frame.cameras[head as usize].zoom);
    let modal_zoom = match (placed_zoom, parts.screen_view.filter(|_| modal.is_some()).and_then(camera_of)) {
        (Some(zoom), _) => zoom,
        (None, Some(c)) if fits => fit_at_most(plan.menu.zoom, Some([c.origin.0, c.origin.1, c.viewport.0 as i32, c.viewport.1 as i32]), region_size(menu_region)),
        (None, Some(_)) => plan.menu.zoom,
        (None, None) => menu.zoom,
    };
    let modal_piece = |camera: Camera| {
        let (o, (w, h)) = (camera.origin, (camera.viewport.0 as i32, camera.viewport.1 as i32));
        let zoom = fit_at_most(modal_zoom, Some([o.0, o.1, w, h]), region_size(Some(modal_region)));
        Affine { from: o, to: modal_place((scaled(w as i64, zoom) as i32, scaled(h as i64, zoom) as i32), modal_region), zoom }
    };
    let anchored = |id| frame.popup_places.iter().find(|(view, _)| *view == id).map(|(_, place)| *place)
        .or_else(|| popup_at.filter(|_| placed_popup == Some(id)));
    let screen_piece = |camera: Camera, popup: bool, id: u32| {
        // A popup the layout anchored at a click: exactly where it says.
        if let Some((at, zoom)) = anchored(id).filter(|_| popup) {
            return Affine { from: camera.origin, to: at, zoom };
        }
        if popup && modal.is_some() {
            return modal_piece(camera);
        }
        let (o, size, d) = (camera.origin, camera.viewport, camera.screen_shift);
        let (w, h) = (size.0 as i32, size.1 as i32);
        let region = if popup { popup_region.or(menu_region) } else { menu_region };
        let zoom = if fits { fit_at_most(plan.menu.zoom, Some([o.0, o.1, w, h]), region_size(region)) } else { plan.menu.zoom };
        let from = (o.0 - d.0 + w / 2, o.1 - d.1 + h / 2);
        let placed = menu_region.map_or_else(|| middle.apply(from), |[x, y, w, h]| (x + w / 2 - d.0, y + h / 2 - d.1));
        let (hw, hh) = (scaled(w as i64 / 2, zoom) as i32, scaled(h as i64 / 2, zoom) as i32);
        let [rx, ry, rw, rh] = if popup { popup_region.or(menu_region) } else { menu_region }.unwrap_or(canvas_rect);
        let keep = |v: i32, half: i32, origin: i32, span: i32| if 2 * half <= span { v.clamp(origin + half, origin + span - half) } else { origin + span / 2 };
        Affine { from: (from.0 + d.0, from.1 + d.1), to: (keep(placed.0 + d.0, hw, rx, rw), keep(placed.1 + d.1, hh, ry, rh)), zoom }
    };
    // (An anchored popup may reach past the canvas: nothing pans for it. A
    // modal one over a menu fits: the menu under it keeps its pan.)
    let active = placed_popup.filter(|_| popup_at.is_none() && modal.is_none() && !on_screen).or(parts.screen_view);
    let active_rect = active.and_then(camera_of)
        .map(|c| screen_piece(c, active == placed_popup, active.unwrap_or(u32::MAX)).rect([c.origin.0, c.origin.1, c.viewport.0 as i32, c.viewport.1 as i32]))
        .or_else(|| menu_box.map(|r| menu.rect(r)));
    let offset = if frame.placed.is_empty() {
        pan.place_in((parts.screen.clone(), active), active_rect, popup_region.or(menu_region).unwrap_or(canvas_rect))
    } else {
        pan.place((None, None), None, canvas)
    };
    let panned = |piece: Affine| Affine { to: (piece.to.0 + offset.0, piece.to.1 + offset.1), ..piece };
    let menu = panned(menu);
    // The modal popup's map and its rectangle in the driver's window.
    let mut modal_shown: Option<(Affine, [i32; 4])> = None;
    for (i, v) in scene.views.iter().enumerate() {
        let Some(c) = frame.view_cameras.get(i).copied().flatten().map(|c| c as usize) else { continue };
        let camera = frame.cameras[c];
        if frame.placed.contains(&(c as u32)) { continue; }
        if !v.view.interface_only {
            // (The main one is the layout's own arrangement.)
            if v.view.viewport.0 > 0 && v.view.viewport.1 > 0 {
                let r = [camera.origin.0, camera.origin.1, camera.viewport.0 as i32, camera.viewport.1 as i32];
                let within = menu_box.is_some_and(|m| r[0] >= m[0] && r[1] >= m[1] && r[0] + r[2] <= m[0] + m[2] && r[1] + r[3] <= m[1] + m[3]);
                let piece = if within { menu } else { middle }.at(camera.origin);
                pieces[c] = Some(piece);
                shown.push((piece, r, (0, 0), None));
            }
            continue;
        }
        let (o, size) = (camera.origin, camera.viewport);
        // A tile box the driver closed, its picture kept by the viewer
        // (`Interface::detached`): exactly where the layout placed it, at
        // its click. It takes no pointer input (the layout closes it).
        if parts.detached.iter().any(|d| d.view == v.id) {
            if let Some((at, zoom)) = anchored(v.id) {
                pieces[c] = Some(Affine { from: o, to: at, zoom }.at(o));
            }
            continue;
        }
        // (Where the layout put it: moved parts about their own place.)
        let about = |from: (i32, i32), zoom: Ratio| Affine { from, to: from, zoom };
        // A popup attached to the one under it (a combo box's open list) is
        // placed as that one: the very same map, so that it shows, and takes
        // the pointer, exactly where that dialog's place puts it, and is no
        // piece apart from the modal (`ShownPiece::under_modal`).
        let (place_id, place_camera) = Some(parts.popup_root(v.id)).filter(|&r| r != v.id && parts.is_popup_view(v.id))
            .and_then(|r| camera_of(r).map(|c| (r, c))).unwrap_or((v.id, camera));
        let piece = if on_screen && placed_popup == Some(parts.popup_root(v.id)) && parts.is_popup_view(v.id) {
            // As the screen it was opened on: the game's place on it.
            menu
        } else if parts.is_popup_view(v.id) || parts.screen_view == Some(v.id) {
            // A popup, or a screen part (an advisor): about its middle,
            // placed as the menus are (and moved as the layout moved it),
            // kept on the canvas.
            let piece = screen_piece(place_camera, parts.is_popup_view(v.id), place_id);
            if parts.is_popup_view(v.id) && (anchored(place_id).is_some() || modal.is_some()) { piece } else { panned(piece) }
        } else if parts.roaming == Some(v.id) || parts.capture_hint == Some(v.id) {
            // The view's own interface (the corner's buttons, the hint in
            // their place): at the frame's scale where it fits the area of
            // the view it is in, inside that area.
            let r = [o.0, o.1, size.0 as i32, size.1 as i32];
            let region = frame.view_ui_region.unwrap_or(canvas_rect);
            kept_in(about(o, fit_at_most(fitting(plan.frame, Some(r)), Some(r), (region[2].max(1) as u32, region[3].max(1) as u32))), r, region)
        } else if v.view.menu {
            menu
        } else {
            // The frame's panels, each anchored at its side or corner.
            let (anchors, mut of_sprite) = panels(plan.screen, &v.overlay, &rect_of);
            // (Each no larger than the canvas.)
            let mut anchors: Vec<(Affine, [i32; 4])> = anchors.into_iter().map(|(a, r)| {
                let piece = plan.frame_piece(a);
                (kept(Affine { zoom: fitting(piece.zoom, Some(r)), ..piece }, r).at((r[0], r[1])), r)
            }).collect();
            // The unit's order buttons where the layout puts them.
            if let (Some(place), Some(orders)) = (frame.unit_orders, parts.reachable) {
                place_unit_orders(place, orders, &v.overlay, &rect_of, &mut anchors, &mut of_sprite);
            }
            if anchors.len() > 1 || anchors.first().is_some_and(|a| a.0.zoom != (1, 1) || a.0.from != a.0.to) {
                for (s, &k) in v.overlay.iter().zip(&of_sprite) {
                    shown.extend(rect_of(s).map(|r| (anchors[k].0, r, (0, 0), None)));
                }
                panelled.push((c, i, anchors, of_sprite));
                continue;
            }
            Affine::IDENTITY
        };
        let piece = piece.at(place_camera.origin);
        pieces[c] = Some(piece);
        clips[c] = if parts.is_popup_view(v.id) { if anchored(place_id).is_some() { None } else if modal.is_some() { menu_region } else { popup_region.or(menu_region) } } else if parts.screen_view == Some(v.id) || v.view.menu { menu_region } else { None };
        let d = camera.screen_shift;
        // A popup or screen part takes the pointer on all of its body, not
        // only on its sprites: a click between them (its plain body, around
        // a button) must not reach the world below and capture the mouse.
        // (Its view's rectangle, else its sprites' bounds.)
        if parts.is_popup_view(v.id) || parts.screen_view == Some(v.id) {
            let full = (size.0 as i32, size.1 as i32) == (plan.screen.0 as i32, plan.screen.1 as i32);
            let body = if size.0 > 0 && size.1 > 0 && !full {
                Some([o.0, o.1, size.0 as i32, size.1 as i32])
            } else {
                let rects: Vec<[i32; 4]> = v.overlay.iter().filter_map(&rect_of).collect();
                content_bounds(&rects, plan.screen).map(|r| [r[0] + d.0, r[1] + d.1, r[2], r[3]])
            };
            shown.extend(body.map(|r| (piece, r, d, clips[c])));
            if modal == Some(v.id) {
                modal_shown = body.map(|r| (piece, [r[0] - d.0, r[1] - d.1, r[2], r[3]]));
            }
        }
        shown.extend(v.overlay.iter().filter_map(&rect_of).map(|r| (piece, [r[0] + d.0, r[1] + d.1, r[2], r[3]], d, clips[c])));
    }
    // A frame in panels needs a camera per side or corner in use: if that
    // is more than the renderer draws, its panels go to the top-left one.
    let extra: usize = panelled.iter().map(|p| p.2.len() - 1).sum();
    let required = frame.cameras.len() + extra + 1;
    if required > tile_render::MAX_VIEWS {
        // A driver can supply arbitrarily many views. Report this bounded
        // renderer's fallback once, rather than on every rendered frame.
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| log::warn!("viewer: the frame and panels need {required} views, limit {}: panels drawn as one (not repeated)", tile_render::MAX_VIEWS));
        for (c, i, anchors, _) in panelled.drain(..) {
            shown.retain(|(p, _, _, _)| !anchors.iter().any(|a| a.0 == *p));
            shown.extend(scene.views[i].overlay.iter().filter_map(&rect_of).map(|r| (Affine::IDENTITY, r, (0, 0), None)));
            pieces[c] = Some(Affine::IDENTITY);
        }
    }
    let intersection = |a: [i32; 4], b: [i32; 4]| {
        let (x, y) = (a[0].max(b[0]), a[1].max(b[1]));
        [x, y, (a[0] + a[2]).min(b[0] + b[2]) - x, (a[1] + a[3]).min(b[1] + b[3]) - y]
    };
    // Mirrored cursors' cameras under the interface: one for each world
    // view below it, while the renderer draws that many (leaving room for
    // the cursor and frame counter cameras on top).
    let layer = frame.cursor_layer.map(|l| l as usize).filter(|&l| l <= frame.cameras.len());
    // (Not the divider's narrow fillers: a mirror there is a sliver.)
    let below = layer.map_or(0, |l| frame.cameras[..l].iter().filter(|c| c.cells && !c.stacked && c.viewport.0 >= 16 && c.viewport.1 >= 16).count()).min(4);
    let slots = below.min(tile_render::MAX_VIEWS.saturating_sub(required.min(tile_render::MAX_VIEWS) + 3));
    let mut cameras = Vec::with_capacity(frame.cameras.len() + extra + 1 + slots);
    let mut index = vec![0u32; frame.cameras.len()];
    frame.cursor_slots.clear();
    for (k, camera) in frame.cameras.iter().enumerate() {
        if Some(k) == layer {
            for _ in 0..slots {
                frame.cursor_slots.push(cameras.len() as u32);
                cameras.push(crate::cursor::empty_camera());
            }
        }
        index[k] = cameras.len() as u32;
        match (panelled.iter().find(|p| p.0 == k), pieces[k]) {
            (Some((_, i, anchors, of_sprite)), _) => {
                // Each panel's part of the driver's screen, at its place.
                let first = cameras.len() as u32;
                for (piece, [x, y, w, h]) in anchors {
                    let part = Camera { origin: (*x, *y), viewport: ((*w).max(1) as u32, (*h).max(1) as u32), ..*camera };
                    cameras.push(overlaid(piece.camera(&part)));
                }
                out.overlay_cameras.push((*i, of_sprite.iter().map(|&a| first + a as u32).collect()));
            }
            (None, Some(piece)) if !camera.cells => {
                let mut camera = overlaid(piece.camera(camera));
                if let Some([x, y, w, h]) = clips[k] { camera.target_clip = Some([x, y, x + w, y + h]); }
                cameras.push(camera);
            },
            (None, Some(piece)) => cameras.push(piece.camera(camera)),
            (None, None) => cameras.push(*camera),
        }
    }
    for v in frame.view_cameras.iter_mut().flatten() {
        *v = index[*v as usize];
    }
    for d in &mut frame.divider {
        d.0 = index[d.0 as usize];
    }
    for c in &mut frame.placed {
        *c = index[*c as usize];
    }
    if layer == Some(frame.cameras.len()) {
        for _ in 0..slots {
            frame.cursor_slots.push(cameras.len() as u32);
            cameras.push(crate::cursor::empty_camera());
        }
    }
    frame.cursor_layer = frame.cursor_slots.first().copied();
    // The viewer's own screen sprites: over everything.
    if cameras.len() < tile_render::MAX_VIEWS {
        cameras.push(Camera { cells: false, ..Camera::new(canvas) });
        out.overlay = Some(cameras.len() as u32 - 1);
    }
    frame.cameras = cameras;
    let shown = shown.into_iter().map(|(piece, r, shift, clip)| {
        let rect = piece.drawn(r);
        let under_modal = modal_shown.filter(|(m, _)| *m != piece).map(|(_, r)| r);
        let modal = modal_shown.is_some_and(|(m, _)| m == piece);
        ShownPiece { rect: clip.map_or(rect, |clip| intersection(rect, clip)), clip: clip.unwrap_or(canvas_rect), driver: [r[0] - shift.0, r[1] - shift.1, r[2], r[3]], piece, shift, under_modal, modal }
    });
    out.shown = Some(shown.filter(|s| s.rect[2] > 0 && s.rect[3] > 0).collect());
    out
}

/// A preview menu has already been placed at its requested edge by the
/// layout. Preserve that placement, including when the driver's original
/// screen is larger than the render canvas. Only oversized-menu panning
/// remains, and the same map drives Alt targets and pointer routing.
fn arrange_beside(plan: &Plan, scene: &Scene, frame: &mut layout::Frame, pan: &mut MenuPan, beside: layout::BesideScreen, _rect_of: impl Fn(&FreeSprite) -> Option<[i32; 4]>) -> Arranged {
    let offset = pan.place_in((scene.interface.screen.clone(), scene.interface.screen_view), Some(beside.mapping.rect(beside.source)), beside.region);
    let piece = Affine { to: (beside.mapping.to.0 + offset.0, beside.mapping.to.1 + offset.1), ..beside.mapping };
    let mut has_menu = false;
    for (i, _) in scene.views.iter().enumerate().filter(|(_, view)| view.view.interface_only && Some(view.id) != scene.interface.capture_hint && !scene.interface.is_popup_view(view.id)) {
        let Some(index) = frame.view_cameras[i] else { continue };
        let camera = &mut frame.cameras[index as usize];
        camera.origin.0 += offset.0;
        camera.origin.1 += offset.1;
        camera.screen_shift.0 += offset.0;
        camera.screen_shift.1 += offset.1;
        camera.target_clip = Some([beside.region[0], beside.region[1], beside.region[0] + beside.region[2], beside.region[1] + beside.region[3]]);
        *camera = overlaid(*camera);
        has_menu = true;
    }
    // The driver supplied the complete menu rectangle. Use that same map
    // for controls in blank space and text links, independent of glyph ink.
    let rect = piece.rect(beside.source);
    let (x, y) = (rect[0].max(beside.region[0]), rect[1].max(beside.region[1]));
    let (right, bottom) = ((rect[0] + rect[2]).min(beside.region[0] + beside.region[2]), (rect[1] + rect[3]).min(beside.region[1] + beside.region[3]));
    let mut shown = if has_menu && right > x && bottom > y {
        vec![ShownPiece { rect: [x, y, right - x, bottom - y], clip: beside.region, driver: beside.source, piece, shift: (0, 0), under_modal: None, modal: false }]
    } else { Vec::new() };
    let mut top_driver = None;
    // Each popup's body before any is placed: one attached to the popup
    // under it (a combo box's open list) is placed as that one.
    let bodies: Vec<(u32, usize, [i32; 4])> = scene.interface.popup_views().filter_map(|id| {
        let index = scene.views.iter().position(|view| view.id == id)?;
        let camera = frame.cameras[frame.view_cameras[index]? as usize];
        Some((id, index, [camera.origin.0, camera.origin.1, camera.viewport.0 as i32, camera.viewport.1 as i32]))
    }).collect();
    let placed_popup = scene.interface.placed_popup();
    for &(id, index, body) in &bodies {
        let Some(camera) = frame.view_cameras[index] else { continue };
        let camera = &mut frame.cameras[camera as usize];
        let root = scene.interface.popup_root(id);
        let place = bodies.iter().find(|b| b.0 == root).map_or(body, |b| b.2);
        let zoom = fit_at_most(piece.zoom, Some(place), (beside.region[2].max(1) as u32, beside.region[3].max(1) as u32));
        let at = modal_place((scaled(place[2] as i64, zoom) as i32, scaled(place[3] as i64, zoom) as i32), beside.region);
        let modal_piece = Affine { from: (place[0], place[1]), to: at, zoom };
        *camera = overlaid(modal_piece.camera(camera));
        camera.target_clip = Some([beside.region[0], beside.region[1], beside.region[0] + beside.region[2], beside.region[1] + beside.region[3]]);
        let modal = placed_popup == Some(root);
        if modal && root == id { top_driver = Some(body); }
        shown.push(ShownPiece { rect: modal_piece.rect(body), clip: beside.region, driver: body, piece: modal_piece, shift: (0, 0), under_modal: None, modal });
    }
    for shown in shown.iter_mut().filter(|shown| !shown.modal) { shown.under_modal = top_driver; }
    let overlay = (frame.cameras.len() < tile_render::MAX_VIEWS).then(|| {
        frame.cameras.push(Camera { cells: false, ..Camera::new(plan.canvas()) });
        frame.cameras.len() as u32 - 1
    });
    Arranged { shown: Some(shown), overlay, ..Default::default() }
}

/// An interface camera as drawn over what is below it: scaled, it is drawn
/// at its zoom straight onto the picture (`Camera::hairline`), not into a
/// picture of its own that would cover its rectangle with the background.
fn overlaid(camera: Camera) -> Camera {
    Camera { hairline: camera.hairline || camera.is_scaled(), ..camera }
}

/// What a frame is drawn with: its cameras, screen copies and presentation.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    pub cameras: Vec<Camera>,
    pub copies: Vec<tile_render::ScreenCopy>,
    pub present: Present,
    /// Window pixels per canvas pixel the canvas was composed at (1: the
    /// canvas's own; see [`drawn`]).
    pub pixel_box: u32,
}

/// The cameras, screen copies and presentation that draw the layout's
/// `cameras` (and `copies`, in the canvas's pixels) into the canvas at its
/// own size, shown on the window in whole boxes as `present` says: at
/// pixel scale `n` the GPU renders the window's pixels divided by `n²`,
/// enlarged by one nearest-pixel copy (`TileRenderer::render_layered`).
/// `outlines`: whether a view at a zoom shows text drawn from outlines
/// (`Layout::zoomed_art`); the driver's interface views show it wherever
/// their zoom in the canvas is not 1 (a menu scaled by the menu scale or
/// to fit), at the canvas's resolution.
pub fn drawn(cameras: &[Camera], copies: &[tile_render::ScreenCopy], present: Present, outlines: impl Fn(Ratio) -> bool) -> Drawn {
    drawn_boxed(cameras, copies, present, outlines, 1)
}

fn drawn_boxed(cameras: &[Camera], copies: &[tile_render::ScreenCopy], present: Present, outlines: impl Fn(Ratio) -> bool, k: u32) -> Drawn {
    // (A stacked view is drawn at its group's zoom.)
    let mut group = None;
    let cameras = cameras.iter().map(|c| {
        let boxed = c.boxed(k);
        let (canvas, window) = match group {
            Some(zooms) if c.stacked => zooms,
            _ => (c.zoom, boxed.zoom),
        };
        if !c.stacked { group = Some((canvas, window)); }
        let zoomed_art = if !c.cells || (k > 1 && c.zoomed_art == outlines(canvas)) { outlines(window) } else { c.zoomed_art };
        Camera { zoomed_art, ..boxed }
    }).collect();
    let (copies, present) = if k > 1 { (copies.iter().map(|c| c.boxed(k)).collect(), present.at_target()) } else { (copies.to_vec(), present) };
    Drawn { cameras, copies, present, pixel_box: k }
}

/// The unit's order buttons (the sprites inside `orders`, the driver's
/// pixels) placed as the layout says (`layout::UnitOrdersPlace`): panels of
/// their own added to the frame's `anchors` (each sprite's in
/// `of_sprite`). Split at a side by side split's gap, those left of its
/// middle are moved left of it and the others right of it, as little as
/// that takes; or all of them go into the active view's area, as large as
/// they are elsewhere where that fits it, centred across, at the same
/// height where that is inside it.
fn place_unit_orders(place: layout::UnitOrdersPlace, orders: [i32; 4], sprites: &[FreeSprite], rect_of: &impl Fn(&FreeSprite) -> Option<[i32; 4]>, anchors: &mut Vec<(Affine, [i32; 4])>, of_sprite: &mut [usize]) {
    let within = |r: &[i32; 4]| r[2] > 0 && r[3] > 0 && r[0] >= orders[0] && r[1] >= orders[1] && r[0] + r[2] <= orders[0] + orders[2] && r[1] + r[3] <= orders[1] + orders[3];
    let members: Vec<(usize, [i32; 4])> = sprites.iter().enumerate().filter_map(|(i, s)| rect_of(s).filter(within).map(|r| (i, r))).collect();
    if members.is_empty() { return; }
    let shown: Vec<[i32; 4]> = members.iter().map(|&(i, r)| anchors[of_sprite[i]].0.rect(r)).collect();
    let zoom_before = anchors[of_sprite[members[0].0]].0.zoom;
    // A group of them (indices into `members`) as one panel more, its map
    // from the driver's pixels made from the one of its panel so far.
    let mut add = |group: Vec<usize>, map: &dyn Fn(Affine) -> Affine, anchors: &mut Vec<(Affine, [i32; 4])>| {
        let mut bases: Vec<usize> = group.iter().map(|&m| of_sprite[members[m].0]).collect();
        bases.sort_unstable();
        bases.dedup();
        for k in bases {
            let part: Vec<usize> = group.iter().copied().filter(|&m| of_sprite[members[m].0] == k).collect();
            let Some(r) = bounding(&part.iter().map(|&m| members[m].1).collect::<Vec<_>>()) else { continue };
            let at = anchors.len();
            anchors.push((map(anchors[k].0).at((r[0], r[1])), r));
            for m in part {
                of_sprite[members[m].0] = at;
            }
        }
    };
    match place {
        layout::UnitOrdersPlace::Gap((g0, g1)) => {
            let (left, right): (Vec<usize>, Vec<usize>) = (0..members.len()).partition(|&m| 2 * shown[m][0] + shown[m][2] < g0 + g1);
            let dl = left.iter().map(|&m| shown[m][0] + shown[m][2]).max().map_or(0, |end| (g0 - end).min(0));
            let dr = right.iter().map(|&m| shown[m][0]).min().map_or(0, |start| (g1 - start).max(0));
            for (group, d) in [(left, dl), (right, dr)] {
                if d != 0 && !group.is_empty() {
                    add(group, &|a: Affine| Affine { to: (a.to.0 + d, a.to.1), ..a }, anchors);
                }
            }
        }
        layout::UnitOrdersPlace::In(area) => {
            let (Some(drawn), Some(driver)) = (bounding(&shown), bounding(&members.iter().map(|m| m.1).collect::<Vec<_>>())) else { return };
            let zoom = fit_at_most(zoom_before, Some(driver), (area[2].max(1) as u32, area[3].max(1) as u32));
            let (w, h) = (scaled(driver[2] as i64, zoom) as i32, scaled(driver[3] as i64, zoom) as i32);
            let x = if w <= area[2] { area[0] + (area[2] - w) / 2 } else { area[0] };
            let y = drawn[1].clamp(area[1], (area[1] + area[3] - h).max(area[1]));
            let map = Affine { from: (driver[0], driver[1]), to: (x, y), zoom };
            add((0..members.len()).collect(), &|_| map, anchors);
        }
    }
}

/// The smallest rectangle holding `rects`.
fn bounding(rects: &[[i32; 4]]) -> Option<[i32; 4]> {
    let first = rects.first()?;
    let mut b = [first[0], first[1], first[0] + first[2], first[1] + first[3]];
    for r in rects {
        b = [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[0] + r[2]), b[3].max(r[1] + r[3])];
    }
    Some([b[0], b[1], b[2] - b[0], b[3] - b[1]])
}

/// The frame's panels: sprites close to each other belong together, and a
/// panel keeps to the side or corner of `screen` its middle is nearest
/// (thirds each way). Per side or corner in use: its anchor and the
/// rectangle of its panels; per sprite: its index among those.
fn panels(screen: (u32, u32), sprites: &[FreeSprite], rect_of: &impl Fn(&FreeSprite) -> Option<[i32; 4]>) -> (Vec<((u8, u8), [i32; 4])>, Vec<usize>) {
    let rects: Vec<[i32; 4]> = sprites.iter().map(|s| rect_of(s).unwrap_or([s.x, s.y, 1, 1])).collect();
    // Union-find over sprites whose rectangles (grown by the gap) touch.
    let mut parent: Vec<usize> = (0..rects.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let near = |a: &[i32; 4], b: &[i32; 4]| {
        a[0] - PANEL_GAP < b[0] + b[2] && b[0] - PANEL_GAP < a[0] + a[2] && a[1] - PANEL_GAP < b[1] + b[3] && b[1] - PANEL_GAP < a[1] + a[3]
    };
    for i in 0..rects.len() {
        for j in 0..i {
            if near(&rects[i], &rects[j]) {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    let mut panel_rects: std::collections::BTreeMap<usize, Vec<[i32; 4]>> = Default::default();
    for (i, r) in rects.iter().enumerate() {
        let k = root(&mut parent, i);
        panel_rects.entry(k).or_default().push(*r);
    }
    let third = |v: i32, size: u32| match v as i64 * 3 / size.max(1) as i64 {
        ..=0 => 0,
        1 => 1,
        _ => 2,
    };
    // Per panel: its anchor.
    let anchor_of: std::collections::BTreeMap<usize, (u8, u8)> = panel_rects
        .iter()
        .map(|(&k, rs)| {
            let b = bounding(rs).unwrap_or_default();
            (k, (third(b[0] + b[2] / 2, screen.0), third(b[1] + b[3] / 2, screen.1)))
        })
        .collect();
    let mut anchors: Vec<((u8, u8), Vec<[i32; 4]>)> = Vec::new();
    let mut of_sprite = Vec::with_capacity(rects.len());
    for (i, r) in rects.iter().enumerate() {
        let a = anchor_of[&root(&mut parent, i)];
        let k = match anchors.iter().position(|(x, _)| *x == a) {
            Some(k) => k,
            None => {
                anchors.push((a, Vec::new()));
                anchors.len() - 1
            }
        };
        anchors[k].1.push(*r);
        of_sprite.push(k);
    }
    let anchors = anchors.into_iter().map(|(a, rs)| (a, bounding(&rs).unwrap_or_default())).collect();
    (anchors, of_sprite)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mechanism tests can exercise arbitrary affine ratios independently
    // of the screen-percentage policy exposed in the settings.
    fn fixed_plan(window: (u32, u32), screen: (u32, u32), scales: (u32, f32, f32, f32)) -> Plan {
        let (pixel_scale, menu, frame_scale, world_scale) = scales;
        let mut p = plan(window, screen, &Resolution { pixel_scale, frame_scale, world_scale, ..Default::default() });
        p.menu.zoom = ratio(menu);
        p
    }

    #[test]
    fn title_and_preferences_fit_with_scaling_off_or_screen_percent_at_x1_and_x3() {
        use scene::{Button, Input, Interactable, InteractableKind, View, ViewState};
        use tile_render::{AtlasBuilder, Grid, PlacedSprite, RgbaImage, SpriteSets, TileWorld};
        let source = [448, 156, 1024, 768];
        let buttons = [[source[0] + 16, source[1] + 16, 32, 24], [source[0] + 976, source[1] + 728, 32, 24]];
        let mut atlas = AtlasBuilder::new(2048);
        let background = atlas.add(&RgbaImage::from_fn(1024, 768, |_, _| [0, 0, 180, 255]), (0, 0)).unwrap();
        let button = atlas.add(&RgbaImage::from_fn(32, 24, |_, _| [255; 4]), (0, 0)).unwrap();
        let atlas = atlas.finish();
        let mut sets = SpriteSets::default();
        let background = sets.single(background);
        let button = sets.single(button);
        let mut world = TileWorld::new(Grid { cols: 1, rows: 1, layout: tile_render::Layout::Orthogonal, wrap_x: false, wrap_y: false, step: (8, 8) }, sets);
        let rect_of = |s: &FreeSprite| Some([s.x, s.y, if s.set == background.raw() { 1024 } else { 32 }, if s.set == background.raw() { 768 } else { 24 }]);
        for preferences in [false, true] {
            let mut scene = Scene {
                screen_size: (1920, 1080),
                interface: scene::Interface { screen: preferences.then(|| "Menu".into()), menu_fit: true,
                    screen_view: preferences.then_some(258), ..Default::default() },
                views: vec![ViewState { id: if preferences { 258 } else { 255 }, view: View {
                    interface_only: true, menu: !preferences,
                    origin: if preferences { (source[0], source[1]) } else { (0, 0) },
                    viewport: if preferences { (1024, 768) } else { (0, 0) }, ..Default::default()
                }, overlay: vec![FreeSprite { set: background.raw(), x: source[0], y: source[1], screen: true, ..Default::default() }] }],
                interactables: buttons.iter().enumerate().map(|(i, &rect)| Interactable { region: None, alt_right_click: false, id: i as u64, rect, anchor: None, kind: InteractableKind::Button }).collect(),
                ..Default::default()
            };
            scene.views[0].overlay.extend(buttons.iter().map(|r| FreeSprite { set: button.raw(), x: r[0], y: r[1], screen: true, ..Default::default() }));
            for window in [(1536, 1152), (1920, 900), (960, 1440)] {
                for pixel_scale in [1, 3] {
                    for scale_menus in [false, true] {
                        for menu_scale_percent in [25.0, 60.0, 100.0] {
                            let settings = Resolution { pixel_scale, scale_menus, menu_scale_percent, ..Default::default() };
                            let p = plan(window, scene.screen_size, &settings).for_scene(&scene, rect_of);
                            assert_eq!(p.present.zoom, (pixel_scale, 1));
                            let canvas = p.canvas();
                            let expected = if scale_menus { canvas.1 as f64 / 768.0 * menu_scale_percent as f64 / 100.0 } else { 1.0 }
                                .min(canvas.0 as f64 / 1024.0).min(canvas.1 as f64 / 768.0);
                            assert!((p.menu.zoom.0 as f64 / p.menu.zoom.1 as f64 - expected).abs() < 0.000001);
                            let mut layout = layout::Layout::default();
                            layout.configure("split=vertical,pane=pointer,magnifier=world,ui=pane").unwrap();
                            layout.driver_screen = Some(p.screen);
                            layout.menu_scale = p.menu.zoom;
                            let mut f = layout.frame(&scene, canvas, (0, 0, 0, 0), vec![], None);
                            let shown = arrange_panned(&p, &scene, &mut f, &mut layout.menu_pan, rect_of);
                            let full = shown.shown.as_ref().unwrap().iter().find(|s| s.driver == source).unwrap();
                            assert!(full.rect[0] >= 0 && full.rect[1] >= 0 && full.rect[0] + full.rect[2] <= canvas.0 as i32 && full.rect[1] + full.rect[3] <= canvas.1 as i32,
                                "{preferences}, {settings:?}, {window:?}: {full:?}");
                            layout.set_interface_shown(shown.shown);
                            let camera = f.view_cameras[0].unwrap();
                            world.free = scene.views[0].overlay.iter().map(|s| {
                                let mut placed = PlacedSprite::new(tile_render::SpriteSetId::from_raw(s.set), s.x, s.y);
                                placed.screen = true;
                                placed.view = Some(camera);
                                placed
                            }).collect();
                            let image = tile_render::reference::render_views(&world, &atlas, &f.cameras, canvas, 0, [0, 0, 0, 255]);
                            let targets = crate::snap::targets_mapped(&scene.interactables, |r| layout.driver_rect_to_window(r), canvas);
                            assert_eq!(targets.len(), buttons.len());
                            for target in targets {
                                let center = target.center();
                                let point = (center.0 as i32, center.1 as i32);
                                let window_point = p.to_window(point);
                                assert_eq!(p.to_screen((window_point.0 as f64, window_point.1 as f64)), point);
                                assert_eq!(image.get(point.0 as u32, point.1 as u32), [255; 4], "{settings:?}, {window:?}, {target:?}");
                                let driver = layout.route_window(point).unwrap();
                                let [x, y, w, h] = buttons[target.id as usize];
                                assert!(driver.0 >= x && driver.0 < x + w && driver.1 >= y && driver.1 < y + h);
                                assert_eq!(layout.route(Input::MouseButton { button: Button::Left, down: true, x: point.0, y: point.1 }),
                                    vec![Input::MouseButton { button: Button::Left, down: true, x: driver.0, y: driver.1 }]);
                                layout.route(Input::MouseButton { button: Button::Left, down: false, x: point.0, y: point.1 });
                            }
                        }
                    }
                }
            }
        }
    }

    /// TS Neo's case: the 3274×2048 title in a 3258×1904 window, at pixel
    /// scales 2 and 1. The canvas is composed at its own size (never the
    /// window's: a pixel scale renders fewer pixels), the title's text
    /// drawn from outlines at its size in the canvas wherever the menu is
    /// scaled there, its 1:1 raster twins left out. Without outlines (or
    /// with sharp text off), or at 1:1, the 1:1 twins.
    #[test]
    fn driver_text_is_drawn_from_outlines_at_its_size_in_the_canvas() {
        use scene::{Glyph, View, ViewState};
        let screen = (3274u32, 2048u32);
        let (mx, my) = ((3274 - 1024) / 2, (2048 - 768) / 2);
        let glyph = Glyph { font: 0, ch: 'N', em64: 18 * 64, advance: 12, bold: false, italic: false, key: None, bar: None };
        let mut overlay = vec![FreeSprite { set: 7, x: mx, y: my, screen: true, ..Default::default() }];
        for row in 0..11 {
            let (x, y) = (mx + 213, my + 311 + 38 * row);
            overlay.push(FreeSprite { set: 9, x, y, screen: true, zoom_art: 1, ..Default::default() });
            overlay.push(FreeSprite { x, y, screen: true, zoom_art: 2, glyph: Some(glyph), ..Default::default() });
        }
        let rect_of = |s: &FreeSprite| Some(if s.set == 7 { [s.x, s.y, 1024, 768] } else { [s.x, s.y - 18, 12, 24] });
        let title = Scene {
            screen_size: screen,
            zooms: vec![(0, 0)],
            interface: scene::Interface { menu_fit: true, ..Default::default() },
            views: vec![ViewState { id: 255, view: View { interface_only: true, menu: true, ..Default::default() }, overlay }],
            ..Default::default()
        };
        for (window, pixel_scale, scale_menus, outlines, sharp) in [
            ((3258, 1904), 2, false, true, true),
            ((3258, 1904), 2, true, true, true),
            ((3258, 1904), 1, true, true, true),
            ((3258, 1904), 2, true, false, true),
            ((3258, 1904), 2, true, true, false),
            ((1024, 768), 1, false, true, true),
        ] {
            let mut scene = title.clone();
            if !outlines { scene.zooms.clear(); }
            let settings = Resolution { pixel_scale, scale_menus, ..Default::default() };
            let p = plan(window, scene.screen_size, &settings).for_scene(&scene, rect_of);
            let mut layout = layout::Layout::default();
            layout.sharp_text = sharp;
            layout.driver_screen = Some(p.screen);
            layout.menu_scale = p.menu.zoom;
            let mut f = layout.frame(&scene, p.canvas(), (0, 0, 0, 0), vec![], None);
            arrange_panned(&p, &scene, &mut f, &mut layout.menu_pan, rect_of);
            let d = drawn(&f.cameras, &f.copies, p.present, |zoom| layout.zoomed_art(zoom));
            let what = format!("window {window:?}, pixel scale {pixel_scale}, menus scaled {scale_menus}, outlines {outlines}, sharp {sharp}");
            let menu = f.view_cameras[0].unwrap() as usize;
            let (canvas, shown) = (f.cameras[menu], d.cameras[menu]);
            let n = p.present.zoom.0;
            assert_eq!(n, pixel_scale, "{what}");
            let text = outlines && sharp && canvas.zoom != (1, 1);
            assert_eq!(shown.zoomed_art, text, "{what}: text from outlines");
            assert_eq!((d.pixel_box, d.present, &d.copies), (1, p.present, &f.copies), "{what}: composed in the canvas");
            assert_eq!(d.present.canvas, (window.0 / n, window.1 / n), "{what}");
            assert_eq!(shown, Camera { zoomed_art: text, ..canvas }, "{what}");
            for (a, b) in f.cameras.iter().zip(&d.cameras) {
                assert_eq!((b.origin, b.viewport, b.zoom), (a.origin, a.viewport, a.zoom), "{what}");
            }
        }
    }

    /// The game clears its whole window before drawing the 1024×768 title
    /// in its middle (TS Neo: a 3274×2048 game window). That backdrop is
    /// not the menu's size: at 100 % the title touches the top and bottom
    /// (or, in a narrower window, both sides), as other menus do; with the
    /// setting off it keeps its own size, within the canvas.
    #[test]
    fn title_over_full_window_backdrop_follows_menu_scale_and_fills_height_at_100() {
        use scene::{Button, Input, Interactable, InteractableKind, View, ViewState};
        use tile_render::{AtlasBuilder, Grid, PlacedSprite, RgbaImage, SpriteSets, TileWorld};
        let mut atlas = AtlasBuilder::new(2048);
        let backdrop = atlas.add(&RgbaImage::from_fn(8, 8, |_, _| [0, 0, 0, 255]), (0, 0)).unwrap();
        let background = atlas.add(&RgbaImage::from_fn(1024, 768, |_, _| [0, 0, 180, 255]), (0, 0)).unwrap();
        let button = atlas.add(&RgbaImage::from_fn(32, 24, |_, _| [255; 4]), (0, 0)).unwrap();
        let atlas = atlas.finish();
        let mut sets = SpriteSets::default();
        let backdrop = sets.single(backdrop);
        let background = sets.single(background);
        let button = sets.single(button);
        let mut world = TileWorld::new(Grid { cols: 1, rows: 1, layout: tile_render::Layout::Orthogonal, wrap_x: false, wrap_y: false, step: (8, 8) }, sets);
        let blue = [0, 0, 180, 255];
        for screen in [(3274u32, 2048u32), (1920, 1080), (1024, 768), (1280, 800), (1280, 720), (1366, 768), (1600, 900)] {
            let source = [(screen.0 as i32 - 1024) / 2, (screen.1 as i32 - 768) / 2, 1024, 768];
            // (On a screen shorter than the title, the part of it on the screen.)
            let visible = [source[0].max(0), source[1].max(0), 1024, 768.min(screen.1 as i32)];
            let vh = visible[3] as f64;
            let buttons = [[source[0] + 16, source[1] + 16, 32, 24], [source[0] + 976, source[1] + 728, 32, 24]];
            // (The backdrop is a full-window fill; only its rectangle matters here.)
            let rect_of = |s: &FreeSprite| Some(match s.set {
                x if x == backdrop.raw() => [0, 0, screen.0 as i32, screen.1 as i32],
                x if x == background.raw() => [s.x, s.y, 1024, 768],
                _ => [s.x, s.y, 32, 24],
            });
            let mut scene = Scene {
                screen_size: screen,
                interface: scene::Interface { menu_fit: true, ..Default::default() },
                views: vec![ViewState { id: 255, view: View { interface_only: true, menu: true, ..Default::default() },
                    overlay: vec![FreeSprite { set: backdrop.raw(), x: 0, y: 0, screen: true, ..Default::default() },
                        FreeSprite { set: background.raw(), x: source[0], y: source[1], screen: true, ..Default::default() }] }],
                interactables: buttons.iter().enumerate().map(|(i, &rect)| Interactable { region: None, alt_right_click: false, id: i as u64, rect, anchor: None, kind: InteractableKind::Button }).collect(),
                ..Default::default()
            };
            scene.views[0].overlay.extend(buttons.iter().map(|r| FreeSprite { set: button.raw(), x: r[0], y: r[1], screen: true, ..Default::default() }));
            // The version number in the window's bottom left corner and a bar
            // at its bottom middle (seen on TS Neo): not part of the menu.
            // (Where the window leaves room for them beside the title.)
            for (x, y) in [(1, screen.1 as i32 - 24), (screen.0 as i32 / 2 - 16, screen.1 as i32 - 24)] {
                let clear = x + 32 <= source[0] || y >= source[1] + 768;
                if clear {
                    scene.views[0].overlay.push(FreeSprite { set: button.raw(), x, y, screen: true, ..Default::default() });
                }
            }
            // (TS Neo's 3258×1904 window first.)
            for window in [(3258, 1904), (2946, 1844), (1920, 1080), (1280, 1024), (1012, 632), (3840, 1600), (1001, 777)] {
                for pixel_scale in [1, 2, 3] {
                    for scale_menus in [false, true] {
                        for menu_scale_percent in [25.0, 60.0, 100.0] {
                            let settings = Resolution { pixel_scale, scale_menus, menu_scale_percent, ..Default::default() };
                            let p = plan(window, scene.screen_size, &settings).for_scene(&scene, rect_of);
                            let canvas = p.canvas();
                            let (cw, ch) = (canvas.0 as f64, canvas.1 as f64);
                            let most = (cw / 1024.0).min(ch / vh);
                            let expected = if scale_menus { ch / vh * menu_scale_percent as f64 / 100.0 } else { 1.0 }.min(most);
                            let what = format!("screen {screen:?}, window {window:?}, {settings:?}, canvas {canvas:?}");
                            assert!((p.menu.zoom.0 as f64 / p.menu.zoom.1 as f64 - expected).abs() < 1e-9, "{what}: {:?}", p.menu.zoom);
                            for configured in [false, true] {
                                let mut layout = layout::Layout::default();
                                if configured {
                                    layout.configure("split=vertical,pane=pointer,magnifier=world,ui=pane").unwrap();
                                }
                                layout.driver_screen = Some(p.screen);
                                layout.menu_scale = p.menu.zoom;
                                let mut f = layout.frame(&scene, canvas, (0, 0, 0, 0), vec![], None);
                                let shown = arrange_panned(&p, &scene, &mut f, &mut layout.menu_pan, rect_of);
                                let s = shown.shown.as_ref().unwrap().iter().find(|s| s.driver == source).unwrap();
                                let title = if visible == source { s.rect } else { s.piece.drawn(visible) };
                                let (cw, ch) = (canvas.0 as i32, canvas.1 as i32);
                                assert!(title[0] >= 0 && title[1] >= 0 && title[0] + title[2] <= cw && title[1] + title[3] <= ch, "{what}: {title:?}");
                                if scale_menus && menu_scale_percent == 100.0 {
                                    // No black above or below; a narrower window letterboxes
                                    // above and below only because the width is used up.
                                    if ch as f64 / vh <= cw as f64 / 1024.0 {
                                        assert_eq!((title[1], title[1] + title[3]), (0, ch), "{what}: {title:?}");
                                    } else {
                                        assert_eq!((title[0], title[0] + title[2]), (0, cw), "{what}: {title:?}");
                                    }
                                } else {
                                    assert!((title[3] as f64 - vh * expected).abs() <= 1.0, "{what}: {title:?}");
                                }
                                layout.set_interface_shown(shown.shown);
                                let camera = f.view_cameras[0].unwrap();
                                world.free = scene.views[0].overlay.iter().map(|s| {
                                    let mut placed = PlacedSprite::new(tile_render::SpriteSetId::from_raw(s.set), s.x, s.y);
                                    placed.screen = true;
                                    placed.view = Some(camera);
                                    placed
                                }).collect();
                                let image = tile_render::reference::render_views(&world, &atlas, &f.cameras, canvas, 0, [0, 0, 0, 255]);
                                // (At 100 % the edge rows themselves: no black line above
                                // or below. Elsewhere the inner rows, as the rectangle's
                                // rounding may differ by a row at fractional zooms.)
                                let full = scale_menus && menu_scale_percent == 100.0;
                                let tall = ch as f64 / vh <= cw as f64 / 1024.0;
                                let inset = if full && tall { 0 } else { 1 };
                                let (mid_x, mid_y) = ((title[0] + title[2] / 2) as u32, (title[1] + title[3] / 2) as u32);
                                assert_eq!(image.get(mid_x, (title[1] + inset) as u32), blue, "{what}: top of {title:?}, configured {configured}, camera {:?}, column {:?}", f.cameras[camera as usize], (0..4).map(|y| image.get(mid_x, y)).collect::<Vec<_>>());
                                assert_eq!(image.get(mid_x, (title[1] + title[3] - 1 - inset) as u32), blue, "{what}: bottom of {title:?}");
                                if full && !tall {
                                    assert_eq!(image.get(0, mid_y), blue, "{what}: left of {title:?}");
                                    assert_eq!(image.get(cw as u32 - 1, mid_y), blue, "{what}: right of {title:?}");
                                }
                                let targets = crate::snap::targets_mapped(&scene.interactables, |r| layout.driver_rect_to_window(r), canvas);
                                assert_eq!(targets.len(), buttons.len(), "{what}");
                                for target in targets {
                                    let center = target.center();
                                    let point = (center.0 as i32, center.1 as i32);
                                    let window_point = p.to_window(point);
                                    assert_eq!(p.to_screen((window_point.0 as f64, window_point.1 as f64)), point);
                                    assert_eq!(image.get(point.0 as u32, point.1 as u32), [255; 4], "{what}, {target:?}");
                                    let driver = layout.route_window(point).unwrap();
                                    let [x, y, w, h] = buttons[target.id as usize];
                                    assert!(driver.0 >= x && driver.0 < x + w && driver.1 >= y && driver.1 < y + h, "{what}: {driver:?}");
                                    assert_eq!(layout.route(Input::MouseButton { button: Button::Left, down: true, x: point.0, y: point.1 }),
                                        vec![Input::MouseButton { button: Button::Left, down: true, x: driver.0, y: driver.1 }]);
                                    layout.route(Input::MouseButton { button: Button::Left, down: false, x: point.0, y: point.1 });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// An arbitrary decimal percent of an arbitrary canvas height used to
    /// reduce to millions (60.758 % of 632 rows), which overflowed the
    /// renderer's 32-bit maths: the picture vanished. Zooms keep small terms
    /// (never above the exact value), and menus at hand-edited percents are
    /// drawn over many canvas sizes.
    #[test]
    fn decimal_menu_percents_keep_zoom_terms_bounded_and_menus_drawn() {
        use scene::{View, ViewState};
        use tile_render::{AtlasBuilder, Grid, PlacedSprite, RgbaImage, SpriteSets, TileWorld};
        let small = |(n, d): Ratio| n as u64 <= MAX_ZOOM_TERM && d as u64 <= MAX_ZOOM_TERM;
        // (The fraction that vanished: 4799941/15360000 before.)
        assert!(small(reduced(632 * 60_758, 76_800_000)));
        assert_eq!(reduced(1080 * 25_000, 76_800_000), (45, 128));
        assert_eq!(reduced(4096, 4095), (4096, 4095));
        let mut zooms = 0;
        for h in (120u64..=2400).step_by(7) {
            for k in 0..80 {
                let percent = 25.0 + k as f64 * 0.94137;
                let (n, d) = (h * (percent * 1000.0).round() as u64, 76_800_000u64);
                let zoom = reduced(n, d);
                let (exact, got) = (n as f64 / d as f64, zoom.0 as f64 / zoom.1 as f64);
                // (A standard menu less than a canvas row shorter.)
                assert!(small(zoom) && got <= exact * (1.0 + 1e-12) && (exact - got) * 768.0 < 1.0, "{h} rows at {percent}: {zoom:?}");
                // (Composed with a camera's own zoom, still small.)
                for camera in [(1, 1), (2, 1), (1, 2), (3, 2), (59, 60)] {
                    assert!(small(times(zoom, camera)), "{zoom:?} × {camera:?}");
                }
                zooms += 1;
            }
        }
        assert!(zooms > 20_000);
        let mut atlas = AtlasBuilder::new(2048);
        let background = atlas.add(&RgbaImage::from_fn(1024, 768, |_, _| [0, 0, 180, 255]), (0, 0)).unwrap();
        let atlas = atlas.finish();
        let mut sets = SpriteSets::default();
        let background = sets.single(background);
        let mut world = TileWorld::new(Grid { cols: 1, rows: 1, layout: tile_render::Layout::Orthogonal, wrap_x: false, wrap_y: false, step: (8, 8) }, sets);
        let blue = [0, 0, 180, 255];
        let screen = (1920u32, 1080u32);
        let source = [448, 156, 1024, 768];
        let rect_of = |_: &FreeSprite| Some(source);
        let scene = Scene {
            screen_size: screen,
            views: vec![ViewState { id: 255, view: View { interface_only: true, menu: true, ..Default::default() },
                overlay: vec![FreeSprite { set: background.raw(), x: source[0], y: source[1], screen: true, ..Default::default() }] }],
            ..Default::default()
        };
        // (The heights the title vanished at, with 60.758 %, and others.)
        let heights = [632u32, 616, 584, 571, 558, 546, 518, 656, 648, 640, 608, 600, 592, 576, 504, 424, 344, 264, 777, 1001, 1440];
        for (i, &height) in heights.iter().enumerate() {
            for percent in [60.758f32, 25.001, 33.333, 49.95, 77.777, 99.999, 100.0 - 1e-4] {
                for width in [1012u32, 1366 + i as u32, 2560] {
                    let settings = Resolution { pixel_scale: 1, scale_menus: true, menu_scale_percent: percent, ..Default::default() };
                    let p = plan((width, height), screen, &settings).for_scene(&scene, rect_of);
                    let canvas = p.canvas();
                    let what = format!("{percent} % of {canvas:?}: {:?}", p.menu.zoom);
                    assert!(small(p.menu.zoom), "{what}");
                    let expected = (canvas.1 as f64 / 768.0 * percent.round() as f64 / 100.0).min(canvas.0 as f64 / 1024.0);
                    assert!((p.menu.zoom.0 as f64 / p.menu.zoom.1 as f64 - expected).abs() < 1e-5, "{what}");
                    let mut layout = layout::Layout::default();
                    layout.driver_screen = Some(p.screen);
                    layout.menu_scale = p.menu.zoom;
                    let mut f = layout.frame(&scene, canvas, (0, 0, 0, 0), vec![], None);
                    let shown = arrange_panned(&p, &scene, &mut f, &mut layout.menu_pan, rect_of);
                    let title = shown.shown.as_ref().unwrap().iter().find(|s| s.driver == source).unwrap().rect;
                    assert!(title[2] > 0 && title[3] > 0, "{what}");
                    let camera = f.view_cameras[0].unwrap() as usize;
                    // What the shaders compute in 32 bits stays in range.
                    for c in &f.cameras {
                        let (n, d) = (c.zoom.0 as u64, c.zoom.1 as u64);
                        assert!(n <= MAX_ZOOM_TERM && d <= MAX_ZOOM_TERM, "{what}: {c:?}");
                        let (sw, sh) = c.source_size();
                        assert!((c.viewport.0.max(sw).max(c.viewport.1).max(sh) as u64 + 4096) * n.max(d) < 1 << 31, "{what}: {c:?}");
                    }
                    world.free = vec![{
                        let mut placed = PlacedSprite::new(tile_render::SpriteSetId::from_raw(background.raw()), source[0], source[1]);
                        placed.screen = true;
                        placed.view = Some(camera as u32);
                        placed
                    }];
                    let image = tile_render::reference::render_views(&world, &atlas, &f.cameras, canvas, 0, [0, 0, 0, 255]);
                    let [x, y, w, h] = title;
                    for (px, py) in [(x, y), (x + w - 1, y), (x, y + h - 1), (x + w - 1, y + h - 1), (x + w / 2, y + h / 2)] {
                        assert_eq!(image.get(px as u32, py as u32), blue, "{what}: {title:?} at {:?}", (px, py));
                    }
                }
            }
        }
    }

    /// At a heavy downscale (×3, menus at 25 %: about ten menu pixels per
    /// canvas pixel) every canvas pixel drawn of a control routes (click
    /// and hover) into that control, and its Alt target's middle is drawn
    /// of it, wherever the menu and its controls start.
    #[test]
    fn heavy_downscale_drawn_control_pixels_route_into_the_control() {
        use scene::{Interactable, InteractableKind, View, ViewState};
        use tile_render::{AtlasBuilder, Grid, PlacedSprite, RgbaImage, SpriteSets, TileWorld};
        let sizes = [(32u32, 24u32), (101, 13), (57, 31), (17, 40), (230, 22)];
        let colour = |i: usize| [255, 40 * i as u8, 0, 255];
        let mut atlas = AtlasBuilder::new(2048);
        let background = atlas.add(&RgbaImage::from_fn(1024, 768, |_, _| [0, 0, 180, 255]), (0, 0)).unwrap();
        let art: Vec<_> = sizes.iter().enumerate().map(|(i, &(w, h))| atlas.add(&RgbaImage::from_fn(w, h, |_, _| colour(i)), (0, 0)).unwrap()).collect();
        let atlas = atlas.finish();
        let mut sets = SpriteSets::default();
        let background = sets.single(background);
        let art: Vec<_> = art.into_iter().map(|a| sets.single(a)).collect();
        let mut world = TileWorld::new(Grid { cols: 1, rows: 1, layout: tile_render::Layout::Orthogonal, wrap_x: false, wrap_y: false, step: (8, 8) }, sets);
        let mut checked = 0;
        for screen in [(1920u32, 1080u32), (1280, 800), (3274, 2048), (1024, 768)] {
            for shift in [0, 3, 7] {
                let source = [(screen.0 as i32 - 1024) / 2 + shift, (screen.1 as i32 - 768) / 2 + shift / 2, 1024, 768];
                let controls: Vec<[i32; 4]> = sizes.iter().enumerate().map(|(i, &(w, h))|
                    [source[0] + 37 + 151 * i as i32 + shift, source[1] + 29 + 113 * i as i32 + 2 * shift, w as i32, h as i32]).collect();
                let rect_of = |s: &FreeSprite| Some(if s.set == background.raw() { [s.x, s.y, 1024, 768] } else {
                    let k = art.iter().position(|a| a.raw() == s.set).unwrap();
                    [s.x, s.y, sizes[k].0 as i32, sizes[k].1 as i32]
                });
                let mut scene = Scene {
                    screen_size: screen,
                    interface: scene::Interface { menu_fit: true, ..Default::default() },
                    views: vec![ViewState { id: 255, view: View { interface_only: true, menu: true, ..Default::default() },
                        overlay: vec![FreeSprite { set: background.raw(), x: source[0], y: source[1], screen: true, ..Default::default() }] }],
                    interactables: controls.iter().enumerate().map(|(i, &rect)| Interactable { region: None, alt_right_click: false, id: i as u64, rect, anchor: None, kind: InteractableKind::Button }).collect(),
                    ..Default::default()
                };
                scene.views[0].overlay.extend(controls.iter().zip(&art).map(|(r, a)| FreeSprite { set: a.raw(), x: r[0], y: r[1], screen: true, ..Default::default() }));
                for window in [(1920, 1080), (3258, 1904), (1012, 632), (2560, 1440), (1366, 768)] {
                    for (pixel_scale, percent) in [(3, 25.0), (3, 31.0), (2, 25.0), (1, 25.0), (3, 47.0)] {
                        let settings = Resolution { pixel_scale, scale_menus: true, menu_scale_percent: percent, ..Default::default() };
                        let p = plan(window, screen, &settings).for_scene(&scene, rect_of);
                        let canvas = p.canvas();
                        let mut layout = layout::Layout::default();
                        layout.driver_screen = Some(p.screen);
                        layout.menu_scale = p.menu.zoom;
                        let mut f = layout.frame(&scene, canvas, (0, 0, 0, 0), vec![], None);
                        let shown = arrange_panned(&p, &scene, &mut f, &mut layout.menu_pan, rect_of);
                        layout.set_interface_shown(shown.shown);
                        let camera = f.view_cameras[0].unwrap();
                        world.free = scene.views[0].overlay.iter().map(|s| {
                            let mut placed = PlacedSprite::new(tile_render::SpriteSetId::from_raw(s.set), s.x, s.y);
                            placed.screen = true;
                            placed.view = Some(camera);
                            placed
                        }).collect();
                        let image = tile_render::reference::render_views(&world, &atlas, &f.cameras, canvas, 0, [0, 0, 0, 255]);
                        let what = format!("screen {screen:?}, shift {shift}, window {window:?}, {settings:?}, canvas {canvas:?}, zoom {:?}", p.menu.zoom);
                        let mut drawn = vec![0; controls.len()];
                        for y in 0..canvas.1 {
                            for x in 0..canvas.0 {
                                let Some(k) = (0..controls.len()).find(|&k| image.get(x, y) == colour(k)) else { continue };
                                drawn[k] += 1;
                                let point = (x as i32, y as i32);
                                let driver = layout.route_window(point).unwrap();
                                let [cx, cy, cw, ch] = controls[k];
                                assert!(driver.0 >= cx && driver.0 < cx + cw && driver.1 >= cy && driver.1 < cy + ch,
                                    "{what}: control {k} {:?} drawn at {point:?} routes to {driver:?}", controls[k]);
                                checked += 1;
                            }
                        }
                        let targets = crate::snap::targets_mapped(&scene.interactables, |r| layout.driver_rect_to_window(r), canvas);
                        for target in targets {
                            let k = target.id as usize;
                            if drawn[k] == 0 { continue; }
                            let c = target.center();
                            let point = (c.0 as i32, c.1 as i32);
                            assert_eq!(image.get(point.0 as u32, point.1 as u32), colour(k), "{what}: target {target:?}");
                        }
                    }
                }
            }
        }
        assert!(checked > 1000, "{checked}");
    }

    #[test]
    fn scale_menus_off_ignores_percent_and_keeps_other_dialogs_native_and_pannable() {
        let (mut scene, rect_of) = game_scene();
        scene.views.retain(|v| v.id == 0 || v.id == 255);
        scene.interface = scene::Interface::default();
        for menu_scale_percent in [25.0, 100.0] {
            let p = plan((1920, 1080), scene.screen_size,
                &Resolution { pixel_scale: 3, scale_menus: false, menu_scale_percent, ..Default::default() }).for_scene(&scene, &rect_of);
            assert_eq!(p.menu.zoom, (1, 1));
            let mut layout = layout::Layout::default();
            layout.driver_screen = Some(p.screen);
            let mut f = layout.frame(&scene, p.canvas(), (0, 0, 0, 0), vec![], None);
            let shown = arrange_panned(&p, &scene, &mut f, &mut layout.menu_pan, &rect_of);
            let menu = shown.shown.unwrap().into_iter().find(|s| s.driver == [448, 156, 1024, 768]).unwrap();
            assert_eq!(menu.piece.zoom, (1, 1));
            assert!(menu.rect[2] > p.canvas().0 as i32 && menu.rect[3] > p.canvas().1 as i32);
            assert!(layout.menu_pan.bounds.iter().any(|&(a, b)| a != b));
        }
    }

    #[test]
    fn fitted_preferences_and_scaled_menus_stay_in_the_pointer_view_with_correct_alt_input() {
        use scene::{Input, Interactable, InteractableKind};
        use viewer_settings::Split;
        for split in [Split::Vertical1To1, Split::Horizontal1To1] {
            for pixel_scale in [1, 3] {
                for pane in [false, true] {
                    for scale_menus in [false, true] {
                        for menu_scale_percent in [25.0, 100.0] {
                            let (mut scene, rect_of) = game_scene();
                            scene.views.retain(|v| v.id == 0 || v.id == 255);
                            scene.interface = Default::default();
                            let menu = scene.views.pop().unwrap();
                            let resolution = Resolution { pixel_scale, scale_menus, menu_scale_percent, ..Default::default() };
                            let initial = plan((1920, 1080), scene.screen_size, &resolution);
                            let mut layout = layout::Layout::default();
                            layout.apply_settings(&viewer_settings::Layout { split, menu_follows_pointer: true, ..Default::default() });
                            layout.driver_screen = Some(initial.screen);
                            let initial_frame = layout.frame(&scene, initial.canvas(), (0, 0, 0, 0), vec![], None);
                            let main = initial_frame.view_cameras[0].unwrap() as usize;
                            let camera = if pane {
                                initial_frame.cameras.iter().enumerate().find(|(i, c)| *i != main && c.cells).unwrap().1
                            } else { &initial_frame.cameras[main] };
                            let expected_region = [camera.origin.0, camera.origin.1, camera.viewport.0 as i32, camera.viewport.1 as i32];
                            layout.route(Input::MouseMove { x: expected_region[0] + expected_region[2] / 2, y: expected_region[1] + expected_region[3] / 2 });
                            scene.views.push(menu);
                            scene.interface.screen = Some("Menu".into());
                            scene.interface.menu_fit = !scale_menus;
                            let control = [448 + 480, 156 + 360, 64, 48];
                            scene.interactables = vec![Interactable { region: None, alt_right_click: false, id: 1, rect: control, anchor: None, kind: InteractableKind::Button }];
                            let p = initial.for_scene(&scene, &rect_of);
                            let mut frame = layout.frame(&scene, p.canvas(), (0, 0, 0, 0), vec![], None);
                            // (Preferences, `menu_fit`: fitted as at the
                            // title, into the window.)
                            let expected_region = if scene.interface.menu_fit { [0, 0, p.canvas().0 as i32, p.canvas().1 as i32] } else { expected_region };
                            assert_eq!(frame.menu_region, (!scene.interface.menu_fit).then_some(expected_region));
                            let shown = arrange_panned(&p, &scene, &mut frame, &mut layout.menu_pan, &rect_of);
                            let full = shown.shown.as_ref().unwrap().iter().find(|s| s.driver == [448, 156, 1024, 768]).unwrap();
                            assert!(full.rect[0] >= expected_region[0] && full.rect[1] >= expected_region[1]
                                && full.rect[0] + full.rect[2] <= expected_region[0] + expected_region[2]
                                && full.rect[1] + full.rect[3] <= expected_region[1] + expected_region[3]);
                            layout.set_interface_shown(shown.shown);
                            let targets = crate::snap::targets_mapped(&scene.interactables, |r| layout.driver_rect_to_window(r), p.canvas());
                            assert_eq!(targets.len(), 1);
                            let at = targets[0].center();
                            let driver = layout.route_window((at.0 as i32, at.1 as i32)).unwrap();
                            assert!(driver.0 >= control[0] && driver.0 < control[0] + control[2] && driver.1 >= control[1] && driver.1 < control[1] + control[3]);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn screen_percent_scenario_beside_reserves_scaled_space_once_and_maps_pixels_and_alt() {
        use scene::{Interactable, InteractableKind, View, ViewState, WorldShape};
        use tile_render::{AtlasBuilder, Grid, PlacedSprite, RgbaImage, SpriteSets, TileWorld};
        let source = [1125, 640, 1024, 768];
        let button_rect = [source[0] + 976, source[1] + 728, 32, 24];
        let mut atlas = AtlasBuilder::new(64);
        let sprite = atlas.add(&RgbaImage::from_fn(32, 24, |_, _| [255; 4]), (0, 0)).unwrap();
        let atlas = atlas.finish();
        let mut sets = SpriteSets::default();
        let set = sets.single(sprite);
        let mut world = TileWorld::new(Grid { cols: 1, rows: 1, layout: tile_render::Layout::Orthogonal, wrap_x: false, wrap_y: false, step: (8, 8) }, sets);
        let mut scene = Scene {
            screen_size: (3274, 2048),
            views: vec![ViewState { id: 0, view: View::default(), overlay: vec![] },
                ViewState { id: 255, view: View { interface_only: true, menu: true, ..Default::default() },
                    overlay: vec![FreeSprite { set: set.raw(), x: button_rect[0], y: button_rect[1], screen: true, ..Default::default() }] }],
            interface: scene::Interface { screen: Some("Load Scenario".into()), beside: Some(scene::Beside { screen: source, edge: scene::Edge::Left, world: None }), ..Default::default() },
            interactables: vec![Interactable { region: None, alt_right_click: false, id: 11, rect: button_rect, anchor: None, kind: InteractableKind::Button }],
            ..Default::default()
        };
        let rect_of = |s: &FreeSprite| Some([s.x, s.y, 32, 24]);
        for window in [(2946, 1844), (1920, 1080), (960, 1440)] {
            for pixel_scale in [1, 3] {
                for menu_scale_percent in [25.0, 100.0] {
                    let resolution = Resolution { pixel_scale, scale_menus: true, menu_scale_percent, ..Default::default() };
                    for ready in [false, true] {
                        scene.shape = if ready { WorldShape { cols: 62, rows: 125, wrap_x: true, wrap_y: false } } else { WorldShape::default() };
                        scene.interface.beside.as_mut().unwrap().world = ready.then_some([0, 0, 8000, 4000]);
                        let p = plan(window, scene.screen_size, &resolution).for_scene(&scene, rect_of);
                        let mut layout = layout::Layout::default();
                        layout.driver_screen = Some(p.screen);
                        layout.menu_scale = p.menu.zoom;
                        let mut frame = layout.frame(&scene, p.canvas(), (0, 0, 0, 0), vec![],
                            Some(layout::WorldBounds { size: (8000, 4000), wrap: (true, false) }));
                        let beside = frame.beside_screen.unwrap();
                        let mapped = beside.mapping.rect(source);
                        assert!(mapped[0] >= 0 && mapped[1] >= 0 && mapped[0] + mapped[2] <= p.canvas().0 as i32 && mapped[1] + mapped[3] <= p.canvas().1 as i32);
                        let camera = frame.view_cameras[1].unwrap();
                        let before = frame.cameras[camera as usize];
                        let shown = arrange_panned(&p, &scene, &mut frame, &mut layout.menu_pan, rect_of);
                        let camera = frame.view_cameras[1].unwrap();
                        assert_eq!(frame.cameras[camera as usize].origin, before.origin);
                        layout.set_interface_shown(shown.shown);
                        let targets = crate::snap::targets_mapped(&scene.interactables, |r| layout.driver_rect_to_window(r), p.canvas());
                        assert_eq!(targets.len(), 1);
                        let target = targets[0].center();
                        let point = (target.0 as i32, target.1 as i32);
                        let driver = layout.route_window(point).unwrap();
                        assert!(driver.0 >= button_rect[0] && driver.0 < button_rect[0] + button_rect[2] && driver.1 >= button_rect[1] && driver.1 < button_rect[1] + button_rect[3]);
                        let mut placed = PlacedSprite::new(set, button_rect[0], button_rect[1]);
                        placed.screen = true;
                        placed.view = Some(camera);
                        world.free = vec![placed];
                        let image = tile_render::reference::render_views(&world, &atlas, &frame.cameras, p.canvas(), 0, [0, 0, 0, 255]);
                        assert_eq!(image.get(point.0 as u32, point.1 as u32), [255; 4], "{window:?}, {resolution:?}, ready {ready}");
                        assert_eq!(layout.take_main_view_change(), None);
                        assert_eq!(layout.take_zooms_change(), None);
                    }
                }
            }
        }
    }

    #[test]
    fn optional_preview_keeps_the_menu_visible_and_interactive_on_a_smaller_canvas() {
        use scene::{Button, Input, Interactable, InteractableKind, View, ViewState, WorldShape};
        use tile_render::{AtlasBuilder, Grid, PlacedSprite, RgbaImage, SpriteSets, TileWorld};
        let source = [1125, 640, 1024, 768];
        let button = [source[0] + 900, source[1] + 20, 16, 12];
        let mut atlas = AtlasBuilder::new(64);
        let sprite = atlas.add(&RgbaImage::from_fn(16, 12, |_, _| [255; 4]), (0, 0)).unwrap();
        let atlas = atlas.finish();
        let mut sets = SpriteSets::default();
        let set = sets.single(sprite);
        let mut world = TileWorld::new(Grid { cols: 1, rows: 1, layout: tile_render::Layout::Orthogonal, wrap_x: false, wrap_y: false, step: (8, 8) }, sets);
        for menu_scale in [0.5, 1.0, 1.5] {
            let p = fixed_plan((2946, 1844), (3274, 2048), (2, menu_scale, 1.0, 1.0));
            let mut l = layout::Layout::default();
            l.configure("split=vertical,pane=pointer,magnifier=world,ui=pane").unwrap();
            l.driver_screen = Some(p.screen);
            l.menu_scale = p.menu.zoom;
            let mut s = Scene {
                views: vec![
                    ViewState { id: 0, view: View::default(), overlay: vec![] },
                    ViewState { id: 255, view: View { interface_only: true, menu: true, ..Default::default() }, overlay: vec![FreeSprite { x: button[0], y: button[1], screen: true, ..Default::default() }] },
                ],
                interface: scene::Interface { screen: Some("Load Scenario".into()), beside: Some(scene::Beside { screen: source, edge: scene::Edge::Left, world: None }), ..Default::default() },
                interactables: vec![Interactable { region: None, alt_right_click: false, id: 99, rect: button, anchor: None, kind: InteractableKind::Button }],
                screen_size: p.screen,
                ..Default::default()
            };
            // Pending, loaded, failed/cancelled, then another loaded preview.
            for ready in [false, true, false, true] {
                s.shape = if ready { WorldShape { cols: 62, rows: 125, wrap_x: true, wrap_y: false } } else { WorldShape::default() };
                s.interface.beside.as_mut().unwrap().world = ready.then_some([0, 0, 8000, 4000]);
                let mut f = l.frame(&s, p.canvas(), (0, 0, 0, 0), vec![], Some(layout::WorldBounds { size: (8000, 4000), wrap: (true, false) }));
                let beside = f.beside_screen.unwrap();
                let ui = f.view_cameras[1].unwrap() as usize;
                let before = f.cameras[ui];
                assert_eq!(before.viewport, ((1024.0 * menu_scale).ceil() as u32, (768.0 * menu_scale).ceil() as u32));
                let shown = arrange_panned(&p, &s, &mut f, &mut l.menu_pan, |sprite| Some([sprite.x, sprite.y, 16, 12]));
                l.set_interface_shown(shown.shown);
                let ui = f.view_cameras[1].unwrap() as usize;
                assert_eq!(f.cameras[ui].origin, before.origin, "preview placement is applied once");
                let x = (908.0 * menu_scale) as i32;
                let y = before.origin.1 + (26.0 * menu_scale) as i32;
                assert_eq!(l.route_window((x, y)), Some((button[0] + 8, button[1] + 6)));
                let target = l.driver_rect_to_window(button);
                assert_eq!(target[0], 900.0 * menu_scale as f64);
                assert!(l.driver_target_clip(button).is_some());
                assert_eq!(l.route(Input::MouseButton { button: Button::Left, down: true, x, y }), [Input::MouseButton { button: Button::Left, down: true, x: button[0] + 8, y: button[1] + 6 }]);
                l.route(Input::MouseButton { button: Button::Left, down: false, x, y });
                assert_eq!(l.take_zooms_change(), None, "saved gameplay zooms never activate for a preview");
                assert_eq!(l.pointer_walls(), None);
                assert_eq!(f.view_cameras[0].is_some(), ready && beside.region[2] + 4 < p.canvas().0 as i32);
                let mut placed = PlacedSprite::new(set, button[0], button[1]);
                placed.screen = true;
                placed.view = Some(ui as u32);
                world.free = vec![placed];
                let image = tile_render::reference::render_views(&world, &atlas, &f.cameras, p.canvas(), 0, [0, 0, 0, 255]);
                assert_eq!(image.get(x as u32, y as u32), [255; 4], "scenario menu pixels survive preview lifecycle at scale {menu_scale}");
            }
            // Returning to the title clears both the preview and its menu map.
            s.shape = Default::default();
            s.interface = Default::default();
            let mut f = l.frame(&s, p.canvas(), (0, 0, 0, 0), vec![], None);
            assert!(f.beside_screen.is_none());
            let shown = arrange_panned(&p, &s, &mut f, &mut l.menu_pan, |sprite| Some([sprite.x, sprite.y, 16, 12]));
            l.set_interface_shown(shown.shown);
            assert!(!l.captured());
            assert_eq!(l.capture_view(), None);
        }
    }

    #[test]
    fn the_picture_is_the_window_in_whole_boxes() {
        // A 4K screen at 200 %: ×2 renders 1920×1080, each pixel 2×2.
        let two = Resolution { pixel_scale: 2, ..Default::default() };
        let p = plan((3840, 2160), (1920, 1080), &two);
        assert_eq!(p.present, Present { canvas: (1920, 1080), zoom: (2, 1), at: (0, 0), target: (3840, 2160) });
        // The default, whatever the desktop's scale: ×1, every window pixel.
        let p = plan((3840, 2160), (1920, 1080), &Resolution::default());
        assert_eq!((p.present.canvas, p.present.zoom), ((3840, 2160), (1, 1)));
        // ×1 and ×3 at 200 %: the same driver's screen, other pictures.
        let p = fixed_plan((3840, 2160), (1920, 1080), (1, 1.0, 1.0, 1.0));
        assert_eq!((p.present.canvas, p.present.zoom), ((3840, 2160), (1, 1)));
        let p = fixed_plan((3840, 2160), (1920, 1080), (3, 1.0, 1.0, 1.0));
        assert_eq!((p.present.canvas, p.present.zoom, p.screen), ((1280, 720), (3, 1), (1920, 1080)));
        // A window that does not divide: a border of fewer than n pixels.
        let p = fixed_plan((2001, 1081), (1920, 1080), (2, 1.0, 1.0, 1.0));
        assert_eq!((p.present.canvas, p.present.at), ((1000, 540), (0, 0)));
        let p = fixed_plan((2003, 1083), (1920, 1080), (4, 1.0, 1.0, 1.0));
        assert_eq!((p.present.canvas, p.present.at), ((500, 270), (1, 1)));
        // Unknown driver screen: the canvas.
        assert_eq!(plan((800, 600), (0, 0), &Resolution::default()).screen, (800, 600));
        // Window pixels and canvas pixels: the middle of a box, and back.
        let p = plan((3840, 2160), (1920, 1080), &two);
        assert_eq!(p.to_window((100, 50)), (201, 101));
        assert_eq!(p.to_screen((201.0, 101.0)), (100, 50));
    }

    #[test]
    fn menus_and_frame_keep_to_the_canvas() {
        // ×1 at 200 %: the game's 1920×1080 interface at its own size on a
        // 3840×2160 picture: menus in its middle, panels at its corners.
        let p = fixed_plan((3840, 2160), (1920, 1080), (1, 1.5, 0.5, 1.0));
        assert_eq!((p.present.canvas, p.menu.zoom, p.frame), ((3840, 2160), (3, 2), (1, 2)));
        assert_eq!(p.menu.apply((960, 540)), (1920, 1080));
        assert_eq!(p.menu.apply((960 + 100, 540)), (1920 + 150, 1080));
        let br = p.frame_piece((2, 2));
        assert_eq!(br.apply((1920, 1080)), (3840, 2160));
        assert_eq!(br.apply((1820, 980)), (3840 - 50, 2160 - 50));
        assert_eq!(br.invert((3840 - 50, 2160 - 50)), (1820, 980));
        let tl = p.frame_piece((0, 0));
        assert_eq!(tl.apply((100, 40)), (50, 20));
        // A camera of the driver's moved and scaled with its piece.
        let p = fixed_plan((3840, 2160), (1920, 1080), (1, 1.0, 1.0, 1.0));
        let c = p.frame_piece((2, 2)).camera(&Camera { origin: (1600, 900), ..Camera::new((320, 180)) });
        assert_eq!((c.origin, c.viewport, c.zoom, c.screen_shift), ((3520, 1980), (320, 180), (1, 1), (1920, 1080)));
    }

    #[test]
    fn panels_keep_to_their_corner() {
        let s = |x, y| FreeSprite { x, y, screen: true, ..Default::default() };
        // A bottom-left box made of two touching pieces, a top-left button,
        // a bottom-right box and bottom-centre buttons.
        let sprites = [s(5, 949), s(100, 1000), s(21, 12), s(1621, 942), s(836, 1030), s(900, 1030)];
        let rect_of = |f: &FreeSprite| {
            Some(match (f.x, f.y) {
                (5, 949) => [5, 949, 280, 130],
                (100, 1000) => [100, 1000, 50, 50],
                (21, 12) => [21, 12, 36, 30],
                (1621, 942) => [1621, 942, 294, 137],
                (x, y) => [x, y, 64, 32],
            })
        };
        let (anchors, of) = panels((1920, 1080), &sprites, &rect_of);
        let anchor = |i: usize| anchors[of[i]].0;
        assert_eq!([anchor(0), anchor(1), anchor(2), anchor(3), anchor(4), anchor(5)], [(0, 2), (0, 2), (0, 0), (2, 2), (1, 2), (1, 2)]);
        assert_eq!(anchors[of[0]].1, [5, 949, 280, 130]);
        assert_eq!(anchors[of[4]].1, [836, 1030, 128, 32]);
    }

    /// A game's scene at 1920×1080 as the bridge sends it: the world, the
    /// frame (panels at three corners and the bottom middle), a menu in the
    /// middle, the roaming corner buttons and a popup.
    fn game_scene() -> (Scene, impl Fn(&FreeSprite) -> Option<[i32; 4]>) {
        use scene::{View, ViewState};
        let sprite = |x, y| FreeSprite { x, y, screen: true, after_pass: u32::MAX, ..Default::default() };
        let frame = vec![sprite(5, 949), sprite(100, 1000), sprite(21, 12), sprite(1621, 942), sprite(836, 1030)];
        let menu = vec![sprite(448, 156)];
        let ui = |id, menu: bool, overlay: Vec<FreeSprite>, origin, viewport| ViewState { id, view: View { interface_only: true, menu, origin, viewport, ..Default::default() }, overlay };
        let world = View { camera_x: 1000, camera_y: 500, zoom: (1, 1), ..Default::default() };
        let scene = Scene {
            views: vec![
                ViewState { id: 0, view: world, overlay: vec![] },
                ui(254, false, frame, (0, 0), (0, 0)),
                ui(255, true, menu, (0, 0), (0, 0)),
                ui(256, false, vec![sprite(21, 12)], (0, 0), (140, 54)),
                ui(257, false, vec![sprite(800, 400)], (800, 400), (320, 200)),
            ],
            shape: scene::WorldShape { cols: 60, rows: 62, wrap_x: true, wrap_y: false },
            interface: scene::Interface { roaming: Some(256), popup: Some(257), ..Default::default() },
            screen_size: (1920, 1080),
            ..Default::default()
        };
        let rect_of = |s: &FreeSprite| {
            Some(match (s.x, s.y) {
                (5, 949) => [5, 949, 280, 130],
                (100, 1000) => [100, 1000, 50, 50],
                (21, 12) => [21, 12, 107, 30],
                (1621, 942) => [1621, 942, 294, 137],
                (448, 156) => [448, 156, 1024, 768],
                (800, 400) => [800, 400, 320, 200],
                (x, y) => [x, y, 256, 32],
            })
        };
        (scene, rect_of)
    }

    #[test]
    fn snapping_highlights_the_row_of_an_anchored_menu_it_points_at() {
        // A context menu opened at a world click in B (parked game camera:
        // every map click a world click), at several pixel and menu scales:
        // each row's snap target lies on that row as drawn, and the pointer
        // the driver gets there is over that row (its hover is that row's).
        use scene::{Button, Input, Interactable, InteractableKind, View, ViewState};
        let (mut scene, rect_of) = game_scene();
        scene.camera_focus = Some(scene::CameraFocus { serial: 1, center: (3000, 1500) });
        // (No game menu over the world.)
        scene.views.retain(|v| v.id != 255);
        let menu = [800, 400, 320, 200];
        let rows: Vec<Interactable> = (0..8).map(|k| Interactable { region: None, alt_right_click: false, rect: [menu[0], menu[1] + 4 + 24 * k, menu[2], 24], anchor: None, kind: InteractableKind::Button, id: 0x900 + k as u64 }).collect();
        let split = viewer_settings::Layout { split: viewer_settings::Split::Vertical1To1, ..Default::default() };
        let bounds = layout::WorldBounds { size: (8000, 4000), wrap: (true, false) };
        for pixel_scale in [1, 2, 3] {
            for menu_scale in [1.0, 1.5, 2.0] {
                let what = format!("×{pixel_scale}, menus {menu_scale}");
                let mut l = layout::Layout::default();
                l.apply_settings(&split);
                let p = fixed_plan((3832, 2106), (1920, 1080), (pixel_scale, menu_scale, 1.0, 1.0));
                l.driver_screen = Some(p.screen);
                l.menu_scale = p.menu.zoom;
                let mut closed = scene.clone();
                closed.interface.popup = None;
                closed.views.retain(|v| v.id != 257);
                for _ in 0..3 {
                    let mut f = l.frame(&closed, p.present.canvas, (0, 0, 0, 0), vec![], Some(bounds));
                    l.interface_shown = arrange(&p, &closed, &mut f, &rect_of).shown;
                }
                let c = (p.present.canvas.0 as i32, p.present.canvas.1 as i32);
                let at = (c.0 / 2 + 40, c.1 / 3);
                let out = l.route(Input::MouseButton { button: Button::Right, down: true, x: at.0, y: at.1 });
                let w = out.iter().find_map(|i| match *i { Input::WorldButton { x, y, .. } => Some((x, y)), _ => None }).expect("a world click");
                l.route(Input::MouseButton { button: Button::Right, down: false, x: at.0, y: at.1 });
                let mut open = scene.clone();
                open.interactables = rows.clone();
                open.interface.popup_anchor = Some(scene::PopupAnchor { world: w, offset: (1, 1), world_scaled: false });
                if let Some(v) = open.views.iter_mut().find(|v| v.id == 257) {
                    v.view = View { interface_only: true, origin: (menu[0], menu[1]), viewport: (menu[2] as u32, menu[3] as u32), ..Default::default() };
                }
                let _: &ViewState = &open.views[0];
                let mut f = l.frame(&open, p.present.canvas, (0, 0, 0, 0), vec![], Some(bounds));
                let ((x0, y0), z) = f.popup_at.expect("anchored");
                l.interface_shown = arrange(&p, &open, &mut f, &rect_of).shown;
                let s = z.0 as f64 / z.1 as f64;
                let targets = crate::snap::targets_mapped(&open.interactables, |r| l.driver_rect_to_window(r), l.window());
                for row in &rows {
                    let t = targets.iter().find(|t| t.id == row.id).unwrap_or_else(|| panic!("{what}: no target for {:#x}", row.id));
                    // Drawn there: the popup's corner plus the row's place in it.
                    let want = [x0 as f64 + (row.rect[0] - menu[0]) as f64 * s, y0 as f64 + (row.rect[1] - menu[1]) as f64 * s, row.rect[2] as f64 * s, row.rect[3] as f64 * s];
                    assert!(t.rect.iter().zip(want).all(|(a, b)| (a - b).abs() < 1.0), "{what}: {:?} drawn at {want:?}", t.rect);
                    // The pointer the driver gets at the target is over that row.
                    let centre = t.center();
                    let q = l.route_window((centre.0.round() as i32, centre.1.round() as i32)).expect("reaches the driver");
                    let under = open.interactables.iter().find(|i| q.0 >= i.rect[0] && q.1 >= i.rect[1] && q.0 < i.rect[0] + i.rect[2] && q.1 < i.rect[1] + i.rect[3]).map(|i| i.id);
                    assert_eq!(under, Some(row.id), "{what}: the driver highlights {under:?} for the target of {:#x}", row.id);
                }
            }
        }
    }

    #[test]
    fn clicks_on_the_frame_reach_its_pixels_at_every_pixel_scale() {
        // A vertical split in a 4K window at 200 %: the minimap (bottom-left),
        // a top-left button and the unit panel (bottom-right) take clicks
        // where they are drawn, at the driver's pixels they show.
        let (scene, rect_of) = game_scene();
        let split = viewer_settings::Layout { split: viewer_settings::Split::Vertical1To1, ..Default::default() };
        for pixel_scale in [1, 2, 3] {
            let mut layout = layout::Layout::default();
            layout.apply_settings(&split);
            let p = fixed_plan((3832, 2106), (1920, 1080), (pixel_scale, 1.0, 1.0, 1.0));
            layout.driver_screen = Some(p.screen);
            for _ in 0..3 {
                let mut f = layout.frame(&scene, p.present.canvas, (0, 0, 0, 0), vec![], None);
                layout.interface_shown = arrange(&p, &scene, &mut f, &rect_of).shown;
            }
            let c = (p.present.canvas.0 as i32, p.present.canvas.1 as i32);
            for (at, driver) in [((100, c.1 - 80), (100, 1000)), ((60, 20), (60, 20)), ((c.0 - 100, c.1 - 60), (1820, 1020))] {
                let routed = layout.route(scene::Input::MouseButton { button: scene::Button::Left, down: true, x: at.0, y: at.1 });
                assert_eq!(routed, vec![scene::Input::MouseButton { button: scene::Button::Left, down: true, x: driver.0, y: driver.1 }], "×{pixel_scale} at {at:?}");
                layout.route(scene::Input::MouseButton { button: scene::Button::Left, down: false, x: at.0, y: at.1 });
            }
        }
    }

    /// The unit's order buttons (eight 32-pixel buttons across the middle
    /// of a 1920 × 1080 screen) for every split, both settings, frame
    /// scales, zoom kinds and active views: with one view (and spanning a
    /// stacked split) where the frame's scale puts them; spanning a side
    /// by side split, split at the divider with its gap clear of them and
    /// open where they are drawn (the pointer crosses there, also at
    /// rows they reach only scaled); in the active view, inside its area,
    /// the divider open alongside them. Never under the zoom box or strip,
    /// nor the corner's buttons; clicks reach each button.
    #[test]
    fn unit_buttons_leave_the_divider_open_or_go_into_the_active_view() {
        use viewer_settings::{Split as S, UnitButtons, Zoom};
        let (mut scene, base_rect) = game_scene();
        scene.views[1].overlay.retain(|s| (s.x, s.y) != (836, 1030));
        let buttons: Vec<[i32; 4]> = (0..8).map(|k| [832 + 32 * k, 1030, 32, 32]).collect();
        for b in &buttons {
            scene.views[1].overlay.push(FreeSprite { x: b[0], y: b[1], screen: true, after_pass: u32::MAX, ..Default::default() });
        }
        scene.interface.reachable = Some([832, 1030, 256, 32]);
        let rect_of = move |s: &FreeSprite| if s.y == 1030 { Some([s.x, s.y, 32, 32]) } else { base_rect(s) };
        let canvas = (1920, 1080);
        let overlaps = |a: [i32; 4], b: [i32; 4]| a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3];
        let mut checked = 0;
        for split in [S::Standard, S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
            for mode in [UnitButtons::Spanning, UnitButtons::ActiveView] {
                for frame_scale in [1.0, 1.5] {
                    for (zoom, box_height) in [(Zoom::Off, 0.0), (Zoom::TopStrip, 0.0), (Zoom::Floating131, 1.0), (Zoom::Mouse131, 0.0)] {
                        for b_active in [false, true] {
                            if b_active && split == S::Standard { continue; }
                            let what = format!("{split:?} {mode:?} frame ×{frame_scale} {zoom:?} {}", if b_active { "B" } else { "A" });
                            let p = fixed_plan(canvas, (1920, 1080), (1, 1.0, frame_scale, 1.0));
                            let mut layout = layout::Layout::default();
                            layout.apply_settings(&viewer_settings::Layout { split, unit_buttons: mode, zoom, zoom_box_height: box_height, ..Default::default() });
                            layout.driver_screen = Some(p.screen);
                            layout.frame(&scene, canvas, (0, 0, 0, 0), vec![], None);
                            let area = layout.view_area(if b_active { layout::Side::B } else { layout::Side::A });
                            let start = (area[0] + area[2] / 2, area[1] + area[3] / 2);
                            layout.route(scene::Input::MouseMove { x: start.0, y: start.1 });
                            let mut shown = Vec::new();
                            for _ in 0..3 {
                                let mut f = layout.frame(&scene, canvas, (0, 0, 0, 0), vec![], None);
                                shown = arrange(&p, &scene, &mut f, &rect_of).shown.unwrap();
                                layout.set_interface_shown(Some(shown.clone()));
                            }
                            assert_eq!(layout.active_side(), if b_active { layout::Side::B } else { layout::Side::A }, "{what}");
                            let drawn: Vec<_> = buttons.iter().map(|b| *shown.iter().find(|s| s.driver == *b).unwrap_or_else(|| panic!("{what}: {b:?} shown"))).collect();
                            let all = drawn.iter().map(|s| s.rect).reduce(|a, b| {
                                let (x, y) = (a[0].min(b[0]), a[1].min(b[1]));
                                [x, y, (a[0] + a[2]).max(b[0] + b[2]) - x, (a[1] + a[3]).max(b[1] + b[3]) - y]
                            }).unwrap();
                            for (b, s) in buttons.iter().zip(&drawn) {
                                // Clicks reach each one.
                                let back = s.to_driver((s.rect[0] + s.rect[2] / 2, s.rect[1] + s.rect[3] / 2));
                                assert!((back.0 - (b[0] + 16)).abs() <= 1 && (back.1 - (b[1] + 16)).abs() <= 1, "{what}: {b:?} -> {back:?}");
                                if let Some(z) = layout.zoom_area() {
                                    let z = [z[0] - 2, z[1] - 2, z[2] + 4, z[3] + 4];
                                    assert!(!overlaps(s.rect, z), "{what}: {b:?} at {:?} under the zoom area {z:?}", s.rect);
                                }
                            }
                            let spanning_unmoved = split == S::Standard || split == S::Horizontal1To1 && mode == UnitButtons::Spanning;
                            if spanning_unmoved {
                                for (b, s) in buttons.iter().zip(&drawn) {
                                    let at = p.frame_piece((1, 2)).rect(*b);
                                    assert!((0..4).all(|i| (s.rect[i] - at[i]).abs() <= 1), "{what}: {b:?} at {:?}, not {at:?}", s.rect);
                                }
                            } else if mode == UnitButtons::Spanning {
                                let ((g0, g1), _) = layout.gap().unwrap();
                                for s in &drawn {
                                    assert!(s.rect[0] + s.rect[2] <= g0 || s.rect[0] >= g1, "{what}: {:?} in the gap {g0}..{g1}", s.rect);
                                }
                                // (Across the middle of a 1:1 split: on both sides of it.)
                                if split == S::Vertical1To1 {
                                    assert!(drawn.iter().any(|s| s.rect[0] < g0) && drawn.iter().any(|s| s.rect[0] >= g1), "{what}: on both sides");
                                }
                            } else {
                                assert!(all[0] >= area[0] && all[1] >= area[1] && all[0] + all[2] <= area[0] + area[2] && all[1] + all[3] <= area[1] + area[3], "{what}: {all:?} in {area:?}");
                            }
                            // (Spanning a 2:1 split they stay in A, clear of its gap.)
                            if split == S::Vertical1To1 || (split == S::Vertical2To1 && mode == UnitButtons::ActiveView) {
                                // The divider is open at their rows as drawn: the pointer crosses there.
                                let row = all[1] + all[3] / 2;
                                assert!(layout.open_spans().iter().any(|&(a, b)| a <= all[1] && b >= all[1] + all[3]), "{what}: rows {all:?} open in {:?}", layout.open_spans());
                                let ((g0, g1), _) = layout.gap().unwrap();
                                let (from, to) = if b_active { ((g1 + 10, row), (g0 - 10, row)) } else { ((g0 - 10, row), (g1 + 10, row)) };
                                layout.route(scene::Input::MouseMove { x: from.0, y: from.1 });
                                layout.route(scene::Input::MouseMove { x: to.0, y: to.1 });
                                assert_eq!(layout.pointer_in_window(), to, "{what}: crossing at row {row}");
                            }
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked >= 100, "{checked}");
    }

    #[test]
    fn a_scaled_split_keeps_worker_panels_and_optional_views_separate() {
        // Worker orders cross the divider near the bottom, opening a
        // second passage. Each passage needs two fill views and each
        // closed stretch its line. The frame still needs four HUD anchors.
        let (mut scene, rect_of) = game_scene();
        scene.interface.reachable = Some([836, 1030, 256, 32]);
        let settings = viewer_settings::Layout {
            split: viewer_settings::Split::Vertical1To1,
            roles: viewer_settings::Roles::UnitMouse,
            ..Default::default()
        };
        for canvas in [(1920, 1080), (2560, 1440)] {
            for frame_scale in [1.0, 1.35] {
                let mut base_views = 0;
                for optional_views in [false, true] {
                    let p = fixed_plan(canvas, (1920, 1080), (1, 1.0, frame_scale, 1.35));
                    let mut layout = layout::Layout::default();
                    layout.apply_settings(&settings);
                    layout.set_map_movement(viewer_settings::MapMovement::Drag);
                    layout.world_scale = p.world;
                    layout.driver_screen = Some(p.screen);
                    layout.frame(&scene, canvas, (0, 0, 0, 0), vec![], None);
                    if optional_views {
                        layout.configure("magnifier=world").unwrap();
                        for (x, y) in [(100, 100), (500, 200), (300, 400)] {
                            layout.route(scene::Input::MouseMove { x, y });
                            for (vk, down) in [(0x11, true), (0x10, true), (0x49, true), (0x49, false), (0x10, false), (0x11, false)] {
                                layout.route(scene::Input::Key { vk, down, text: None });
                            }
                        }
                    }
                    let mut f = layout.frame(&scene, canvas, (0, 0, 0, 0), vec![], None);
                    if optional_views {
                        assert_eq!(f.cameras.len(), base_views + 8, "all three framed insets and the magnifier remain visible");
                    } else {
                        base_views = f.cameras.len();
                    }
                    let arranged = arrange(&p, &scene, &mut f, &rect_of);
                    let what = format!("{canvas:?}, frame scale {frame_scale}, optional views {optional_views}");
                    if canvas == (1920, 1080) {
                        // (Besides the cameras kept for mirrored cursors under the
                        // interface, one per world view below it; the divider takes
                        // two, its halves.)
                        // (One more for the orders, moved right of the gap they crossed.)
                        assert_eq!(f.cameras.len() - f.cursor_slots.len(), if optional_views { 21 } else { 13 }, "the worker layout: {what}");
                        assert!((2..=4).contains(&f.cursor_slots.len()), "{what}: {:?}", f.cursor_slots);
                    }
                    let (_, cameras) = arranged.overlay_cameras.iter().find(|(view, _)| *view == 1).expect("the HUD keeps its panel cameras");
                    assert_eq!(cameras.len(), scene.views[1].overlay.len(), "{what}");
                    assert_eq!(cameras.iter().copied().collect::<std::collections::BTreeSet<_>>().len(), 4, "{what}");
                    assert_eq!(cameras[0], cameras[1], "pieces of the minimap share its camera: {what}");
                    // The cursor and FPS views are added after arrangement.
                    assert!(f.cameras.len() + 2 <= tile_render::MAX_VIEWS, "{what}: {} arranged cameras", f.cameras.len());
                    assert!(arranged.overlay.is_some(), "viewer overlays retained: {what}");
                    let shown = arranged.shown.as_ref().unwrap();
                    for (driver, anchor) in [([5, 949, 280, 130], (0, 2)), ([21, 12, 107, 30], (0, 0)), ([1621, 942, 294, 137], (2, 2)), ([836, 1030, 256, 32], (1, 2))] {
                        let piece = shown.iter().find(|s| s.driver == driver).expect("every HUD panel remains interactive");
                        // (The pixels drawn of it, from its camera's rounded
                        // origin: within a pixel of its anchor's map.)
                        let mut anchored = p.frame_piece(anchor).rect(driver);
                        if driver == [836, 1030, 256, 32] {
                            // The orders (one piece here) start right of the divider's gap.
                            anchored[0] = layout.gap().unwrap().0.1;
                        }
                        assert!((0..4).all(|i| (piece.rect[i] - anchored[i]).abs() <= 1), "{what}: panel {driver:?}: {:?} vs {anchored:?}", piece.rect);
                        let at = (piece.rect[0] + piece.rect[2] / 2, piece.rect[1] + piece.rect[3] / 2);
                        let back = piece.to_driver(at);
                        assert!((back.0 - (driver[0] + driver[2] / 2)).abs() <= 1 && (back.1 - (driver[1] + driver[3] / 2)).abs() <= 1, "{what}: panel input {driver:?} -> {back:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn any_window_and_pixel_scale_with_a_split() {
        // Windows of any size (odd, tiny, wide, tall) at every pixel scale,
        // split side by side, the world moving smoothly: a picture of at
        // least 320×240 where the window allows, whole boxes, cameras that
        // draw, the menu's middle mapping back, the layout run for a while.
        let (scene, rect_of) = game_scene();
        let split = viewer_settings::Layout { split: viewer_settings::Split::Vertical1To1, smooth_world_movement: true, ..Default::default() };
        let windows = [(1, 1), (7, 5), (319, 239), (333, 251), (641, 479), (1001, 667), (1737, 1006), (3840, 2160), (3841, 2161), (5000, 400), (400, 3000)];
        for window in windows {
            for pixel_scale in [1, 2, 3, 4, 5, 6] {
                let settings = Resolution { pixel_scale, ..Default::default() };
                let what = format!("{window:?} at ×{pixel_scale}");
                let p = plan(window, (1920, 1080), &settings);
                let (canvas, n) = (p.present.canvas, p.present.zoom.0);
                assert_eq!(p.present.zoom.1, 1, "{what}");
                assert!(canvas.0 * n <= window.0.max(1) && canvas.1 * n <= window.1.max(1), "{what}: {:?}", p.present);
                assert!(window.0.max(1) - canvas.0 * n < n && window.1.max(1) - canvas.1 * n < n, "{what}: {:?}", p.present);
                if window.0 >= 320 && window.1 >= 240 {
                    assert!(canvas.0 >= 320 && canvas.1 >= 240, "{what}: {canvas:?}");
                }
                let mut layout = layout::Layout::default();
                layout.apply_settings(&split);
                layout.world_scale = p.world;
                layout.driver_screen = Some(p.screen);
                for _ in 0..3 {
                    layout.advance(std::time::Duration::from_millis(16));
                    let mut f = layout.frame(&scene, canvas, (0, 0, 0, 0), vec![], None);
                    let arranged = arrange(&p, &scene, &mut f, &rect_of);
                    assert!(f.cameras.len() <= tile_render::MAX_VIEWS, "{what}");
                    assert!(f.cameras.iter().all(|c| c.viewport.0 > 0 && c.viewport.1 > 0), "{what}: {:?}", f.cameras);
                    let shown = arranged.shown.clone().unwrap_or_default();
                    assert!(shown.iter().all(|s| s.rect[2] > 0 && s.rect[3] > 0), "{what}");
                    layout.interface_shown = arranged.shown;
                    // The pointer anywhere in the window: somewhere in
                    // the driver's screen or nowhere, never a panic.
                    for (x, y) in [(0.0, 0.0), (window.0 as f64 - 1.0, window.1 as f64 - 1.0), (window.0 as f64 / 2.0, window.1 as f64 / 2.0)] {
                        let q = p.to_screen((x, y));
                        assert!(q.0 >= 0 && q.1 >= 0 && q.0 <= canvas.0 as i32 && q.1 <= canvas.1 as i32, "{what}: {q:?}");
                        let _ = layout.route(scene::Input::MouseMove { x: q.0, y: q.1 });
                    }
                }
            }
        }
    }

    #[test]
    fn every_setting_preserves_scale_and_reachable_interface() {
        // Every pixel scale and part scale from the slider's
        // ends to its middle: whole pixels; the world fills the picture,
        // every part has a camera that draws over what is below it (none blots out the
        // world), takes the pointer where it shows and maps it back to the
        // driver's pixel it shows.
        let (scene, rect_of) = game_scene();
        let scales = [0.5, 0.7, 1.0, 1.2, 2.0, 3.0];
        for pixel_scale in [1, 2, 3, 6] {
            let mut combos = Vec::new();
            for m in scales {
                for f in scales {
                    for w in scales {
                        combos.push((m, f, w));
                    }
                }
            }
            for (menu, frame, world) in combos {
                let scales = (pixel_scale, menu, frame, world);
                let what = format!("{scales:?}");
                let p = fixed_plan((3840, 2160), (1920, 1080), scales);
                let mut layout = layout::Layout::default();
                layout.world_scale = p.world;
                layout.driver_screen = Some(p.screen);
                let mut f = layout.frame(&scene, p.present.canvas, (0, 0, 0, 0), vec![], None);
                let arranged = arrange(&p, &scene, &mut f, &rect_of);
                let canvas = p.present.canvas;
                assert_eq!(p.present.zoom.1, 1, "{what}: whole boxes");
                assert!((3840 - canvas.0 * p.present.zoom.0) < p.present.zoom.0 && (2160 - canvas.1 * p.present.zoom.0) < p.present.zoom.0, "{what}");
                assert!(f.cameras.len() <= tile_render::MAX_VIEWS, "{what}");
                assert!(f.cameras.iter().all(|c| c.viewport.0 > 0 && c.viewport.1 > 0), "{what}");
                let main = f.cameras[f.view_cameras[0].unwrap() as usize];
                assert!(main.cells && main.origin.0 <= 0 && main.origin.1 <= 0, "{what}: {main:?}");
                assert!(main.origin.0 + main.viewport.0 as i32 >= canvas.0 as i32 - 1 && main.origin.1 + main.viewport.1 as i32 >= canvas.1 as i32 - 1, "{what}: {main:?}");
                for (v, c) in scene.views.iter().zip(&f.view_cameras).filter(|(v, _)| v.view.interface_only) {
                    let Some(c) = c else { continue };
                    let c = f.cameras[*c as usize];
                    assert!(!c.is_scaled() || c.hairline, "{what}: view {} drawn into a picture of its own", v.id);
                }
                for (_, cams) in &arranged.overlay_cameras {
                    assert!(cams.iter().all(|&c| (c as usize) < f.cameras.len() && !f.cameras[c as usize].cells), "{what}");
                }
                {
                    assert!(arranged.overlay.is_some_and(|o| (o as usize) < f.cameras.len()), "{what}");
                    let shown = arranged.shown.as_ref().expect("the interface as shown");
                    // The menu: its middle maps back near its middle (a
                    // layout pixel may be several driver pixels).
                    let menu_piece = shown.iter().find(|s| {
                        let m = s.to_driver((s.rect[0] + s.rect[2] / 2, s.rect[1] + s.rect[3] / 2));
                        (m.0 - 960).abs() <= 8 && (m.1 - 540).abs() <= 8
                    });
                    assert!(menu_piece.is_some(), "{what}: {shown:?}");
                    // Fitting axes remain wholly visible. Oversized
                    // axes cover the canvas and retain their scale.
                    for piece in shown {
                        let c = piece.rect;
                        for (at, len, side) in [(c[0], c[2], canvas.0 as i32), (c[1], c[3], canvas.1 as i32)] {
                            if len <= side {
                                assert!(at >= -1 && at + len <= side + 1, "{what}: {c:?} on {canvas:?}");
                            } else {
                                assert!(at <= 0 && at + len >= side, "{what}: {c:?} on {canvas:?}");
                            }
                        }
                        if piece.driver == [448, 156, 1024, 768] {
                            assert_eq!(piece.piece.zoom, ratio(menu), "{what}");
                        }
                        // (The popup is a modal over the menu: at its zoom,
                        // smaller only where it would not be whole.)
                        if piece.driver == [800, 400, 320, 200] {
                            assert_eq!(piece.piece.zoom, fit_at_most(ratio(menu), Some(piece.driver), canvas), "{what}");
                        }
                    }
                    assert!(shown.iter().all(|s| s.rect[2] > 0 && s.rect[3] > 0), "{what}");
                }
            }
        }
    }

    #[test]
    fn pixel_scale_grows_full_menus_screens_and_popups_through_six() {
        for kind in ["menu", "screen", "popup"] {
            for menu_scale in [1.0, 1.5] {
                for pixel_scale in 1..=6 {
                    let (mut scene, _) = game_scene();
                    scene.views.retain(|v| v.id == 0 || v.id == 255);
                    scene.interface = scene::Interface::default();
                    let v = &mut scene.views[1];
                    v.view.origin = (448, 156);
                    v.view.viewport = (1024, 768);
                    v.view.menu = kind == "menu";
                    if kind == "screen" {
                        scene.interface.screen_view = Some(v.id);
                        scene.interface.screen = Some("Preferences".into());
                    } else if kind == "popup" {
                        scene.interface.popup = Some(v.id);
                    }
                    let p = fixed_plan((3840, 2160), scene.screen_size, (pixel_scale, menu_scale, 1.0, 1.0));
                    let mut layout = layout::Layout::default();
                    layout.driver_screen = Some(p.screen);
                    let mut f = layout.frame(&scene, p.canvas(), (0, 0, 0, 0), vec![], None);
                    let mut pan = MenuPan::default();
                    let a = arrange_panned(&p, &scene, &mut f, &mut pan, |_| Some([448, 156, 1024, 768]));
                    let piece = a.shown.unwrap()[0];
                    assert_eq!(piece.piece.zoom, ratio(menu_scale), "{kind} ×{pixel_scale}");
                    assert_eq!(piece.rect[2] as u32 * p.present.zoom.0, (1024.0 * menu_scale) as u32 * pixel_scale);
                    assert_eq!(piece.rect[3] as u32 * p.present.zoom.0, (768.0 * menu_scale) as u32 * pixel_scale);
                    let c = f.cameras[f.view_cameras[1].unwrap() as usize];
                    assert_eq!(c.zoom, ratio(menu_scale));
                }
            }
        }
    }

    #[test]
    fn oversized_menu_pans_to_every_corner_and_clicks_follow_its_pixels() {
        use scene::{Button, Input};
        let (mut scene, rect_of) = game_scene();
        scene.views.retain(|v| v.id == 0 || v.id == 255);
        scene.interface = scene::Interface::default();
        let p = fixed_plan((3840, 2160), scene.screen_size, (6, 1.0, 1.0, 1.0));
        let mut layout = layout::Layout::default();
        layout.driver_screen = Some(p.screen);
        let draw = |layout: &mut layout::Layout| {
            let mut f = layout.frame(&scene, p.canvas(), (0, 0, 0, 0), vec![], None);
            let a = arrange_panned(&p, &scene, &mut f, &mut layout.menu_pan, &rect_of);
            let piece = a.shown.as_ref().unwrap()[0];
            layout.interface_shown = a.shown;
            piece
        };
        draw(&mut layout);
        for (dx, dy) in [(-10000, -10000), (10000, -10000), (-10000, 10000), (10000, 10000)] {
            assert!(layout.route(Input::MouseButton { button: Button::Middle, down: true, x: 320, y: 180 }).is_empty());
            assert!(layout.route(Input::MouseMove { x: 320 + dx, y: 180 + dy }).is_empty());
            assert!(layout.route(Input::MouseButton { button: Button::Middle, down: false, x: 320, y: 180 }).is_empty());
            let piece = draw(&mut layout);
            assert_eq!(piece.rect[0], if dx > 0 { 0 } else { 640 - 1024 });
            assert_eq!(piece.rect[1], if dy > 0 { 0 } else { 360 - 768 });
            let expected = piece.to_driver((320, 180));
            // Release requests hover only after this displayed map is
            // current. The same coordinates then reach hover and clicks.
            assert!(layout.menu_pan.take_hover_refresh());
            assert!(!layout.menu_pan.take_hover_refresh());
            assert_eq!(layout.route_window(layout.pointer_in_window()), Some(expected));
            assert_eq!(layout.route(Input::MouseMove { x: 320, y: 180 }),
                vec![Input::MouseMove { x: expected.0, y: expected.1 }]);
            for down in [true, false] {
                assert_eq!(layout.route(Input::MouseButton { button: Button::Left, down, x: 320, y: 180 }),
                    vec![Input::MouseButton { button: Button::Left, down, x: expected.0, y: expected.1 }]);
            }
        }
        // Losing focus stops the gesture; moving afterwards cannot pan.
        layout.route(Input::MouseButton { button: Button::Middle, down: true, x: 320, y: 180 });
        layout.released();
        let before = draw(&mut layout).rect;
        layout.route(Input::MouseMove { x: 100, y: 100 });
        assert_eq!(draw(&mut layout).rect, before);
        // Leaving the window can lose the middle-button release. Coming
        // back moves the pointer without continuing the old gesture.
        layout.route(Input::MouseButton { button: Button::Middle, down: true, x: 320, y: 180 });
        layout.pointer_gone();
        let before = draw(&mut layout).rect;
        layout.route(Input::MouseMove { x: 420, y: 250 });
        assert_eq!(draw(&mut layout).rect, before);
        assert!(!layout.menu_pan.take_hover_refresh());
        // Resizing clamps the offset and cancels an old gesture.
        layout.menu_pan.route(&Input::MouseButton { button: Button::Middle, down: true, x: 320, y: 180 });
        layout.menu_pan.place((None, None), Some([0, 0, 1024, 768]), (1920, 1080));
        assert_eq!(layout.menu_pan.offset, (0, 0));
        assert!(!layout.menu_pan.route(&Input::MouseMove { x: 10, y: 10 }));
        // A new screen resets navigation even when its bounds match.
        layout.menu_pan.place((Some("F1".into()), None), Some([-192, -204, 1024, 768]), (640, 360));
        layout.menu_pan.route(&Input::MouseButton { button: Button::Middle, down: true, x: 320, y: 180 });
        layout.menu_pan.route(&Input::MouseMove { x: 100, y: 100 });
        layout.menu_pan.place((Some("F2".into()), None), Some([-192, -204, 1024, 768]), (640, 360));
        assert_eq!(layout.menu_pan.offset, (0, 0));
        assert!(!layout.menu_pan.route(&Input::MouseMove { x: 20, y: 20 }));
        assert!(!layout.menu_pan.route(&Input::MouseButton { button: Button::Right, down: true, x: 320, y: 180 }));
        assert!(!layout.menu_pan.route(&Input::Wheel { delta: 120, x: 320, y: 180 }));
    }

    /// A modal popup over a menu (C3X's info box over the Preferences
    /// screen, a popup over the title) leaves the menu exactly as without
    /// it. The popup shows across in the middle of the window, its top in
    /// the top third ([`MODAL_TOP`]), at the menu's zoom, and takes the
    /// clicks on its button. TS Neo first: the game's 3274×2048 screen in a
    /// 3258×1904 window at pixel scale 2 (a 1629×952 canvas).
    #[test]
    fn a_modal_over_a_menu_keeps_the_menu_and_shows_in_the_top_third() {
        use scene::{Interactable, InteractableKind, View, ViewState};
        let (background, info, button) = (1, 2, 3);
        let cases = [((3274u32, 2048u32), (3258u32, 1904u32), 2u32), ((1280, 800), (1280, 800), 1), ((1920, 1080), (1920, 1080), 1), ((1024, 768), (1024, 768), 1)];
        for (screen, window, pixel_scale) in cases {
            let menu = [(screen.0 as i32 - 1024) / 2, (screen.1 as i32 - 768) / 2, 1024, 768];
            // Where the game put the info box: at the top of its screen.
            let popup = [(screen.0 as i32 - 500) / 2, 0, 500, 300];
            let ok = [popup[0] + 220, popup[1] + 250, 60, 24];
            let rect_of = |s: &FreeSprite| Some(match s.set {
                x if x == background => [s.x, s.y, 1024, 768],
                x if x == info => [s.x, s.y, 500, 300],
                _ => [s.x, s.y, 60, 24],
            });
            let sprite = |set: u32, (x, y): (i32, i32)| FreeSprite { set, x, y, screen: true, ..Default::default() };
            for preferences in [true, false] {
                let base = Scene {
                    screen_size: screen,
                    interface: scene::Interface { screen: preferences.then(|| "Menu".into()), menu_fit: true, screen_view: preferences.then_some(258), ..Default::default() },
                    views: vec![ViewState { id: if preferences { 258 } else { 255 }, view: View {
                        interface_only: true, menu: !preferences,
                        origin: if preferences { (menu[0], menu[1]) } else { (0, 0) },
                        viewport: if preferences { (1024, 768) } else { (0, 0) }, ..Default::default()
                    }, overlay: vec![sprite(background, (menu[0], menu[1]))] }],
                    ..Default::default()
                };
                let mut with = base.clone();
                with.interface.popup = Some(257);
                with.views.push(ViewState { id: 257, view: View { interface_only: true, origin: (popup[0], popup[1]), viewport: (500, 300), ..Default::default() },
                    overlay: vec![sprite(info, (popup[0], popup[1])), sprite(button, (ok[0], ok[1]))] });
                with.interactables = vec![Interactable { region: None, alt_right_click: false, id: 7, rect: ok, anchor: None, kind: InteractableKind::Button }];
                for scale_menus in [false, true] {
                    for menu_scale_percent in [60.0, 100.0] {
                        let settings = Resolution { pixel_scale, scale_menus, menu_scale_percent, ..Default::default() };
                        let what = format!("{screen:?} in {window:?}, preferences {preferences}, {settings:?}");
                        let arranged = |scene: &Scene| {
                            let p = plan(window, screen, &settings).for_scene(scene, rect_of);
                            let mut layout = layout::Layout::default();
                            layout.driver_screen = Some(p.screen);
                            layout.menu_scale = p.menu.zoom;
                            let mut f = layout.frame(scene, p.canvas(), (0, 0, 0, 0), vec![], None);
                            let shown = arrange_panned(&p, scene, &mut f, &mut layout.menu_pan, rect_of).shown.unwrap();
                            layout.set_interface_shown(Some(shown.clone()));
                            (p, layout, shown)
                        };
                        let (p0, _, shown0) = arranged(&base);
                        let (p, layout, shown) = arranged(&with);
                        if screen == (3274, 2048) {
                            assert_eq!(p.canvas(), (1629, 952), "{what}");
                        }
                        // The menu: as without the popup.
                        assert_eq!(p.menu.zoom, p0.menu.zoom, "{what}");
                        let menu_of = |shown: &[ShownPiece]| *shown.iter().find(|s| s.driver == menu).expect("the menu shown");
                        let (m0, m) = (menu_of(&shown0), menu_of(&shown));
                        assert_eq!((m.rect, m.piece.zoom), (m0.rect, m0.piece.zoom), "{what}: the menu");
                        // The popup: at the menu's zoom, across in the middle,
                        // its top a sixth of the way down.
                        let modal = *shown.iter().find(|s| s.driver == popup).expect("the popup shown");
                        assert_eq!(modal.piece.zoom, m.piece.zoom, "{what}: the menu's zoom");
                        let (cw, ch) = (p.canvas().0 as i32, p.canvas().1 as i32);
                        let [x, y, w, h] = modal.rect;
                        assert!((2 * x + w - cw).abs() <= 2, "{what}: across in the middle: {:?}", modal.rect);
                        assert!((y - (ch * MODAL_TOP.0 as i32 / MODAL_TOP.1 as i32).min(ch - h)).abs() <= 1, "{what}: top: {:?} on {ch}", modal.rect);
                        assert!(y >= 0 && y <= ch / 3 && y + h <= ch, "{what}: in the top third, whole: {:?} on {ch}", modal.rect);
                        // Clicks on its button reach the button.
                        let targets = crate::snap::targets_mapped(&with.interactables, |r| layout.driver_rect_to_window(r), p.canvas());
                        assert_eq!(targets.len(), 1, "{what}");
                        let c = targets[0].center();
                        let driver = layout.route_window((c.0 as i32, c.1 as i32)).expect("the button takes the pointer");
                        assert!(driver.0 >= ok[0] && driver.0 < ok[0] + ok[2] && driver.1 >= ok[1] && driver.1 < ok[1] + ok[3], "{what}: {c:?} -> {driver:?}");
                        // A click on the menu where the driver put the popup
                        // does not reach it there.
                        let [bx, by, bw, bh] = m.to_canvas(ok);
                        let at = ((bx + bw / 2.0) as i32, (by + bh / 2.0) as i32);
                        if inside_rect(m.rect, at) && !inside_rect(modal.rect, at) {
                            let driver = layout.route_window(at).expect("the menu takes the pointer");
                            assert!(!inside_rect(popup, driver), "{what}: {at:?} -> {driver:?}");
                        }
                    }
                }
            }
        }
    }

    /// A full menu-sized modal screen over the title (the Hall of Fame:
    /// the driver sends its 1024×768 board as the modal part) is shown as
    /// large as the title under it, at 100 % from the canvas's top to its
    /// bottom. A part as large as the driver's whole window would instead
    /// be shrunk to fit: TS Neo's 3274×2048 into 1629×952, the board under
    /// a third of the canvas wide.
    #[test]
    fn a_menu_sized_modal_screen_over_the_title_is_as_large_as_the_title() {
        use scene::{View, ViewState};
        let (screen, window) = ((3274u32, 2048u32), (3258u32, 1904u32));
        let menu = [(3274 - 1024) / 2, (2048 - 768) / 2, 1024, 768];
        let rect_of = |s: &FreeSprite| Some(if s.set == 1 { [s.x, s.y, 1024, 768] } else { [s.x, s.y, 3274, 2048] });
        let sprite = |set: u32, (x, y): (i32, i32)| FreeSprite { set, x, y, screen: true, ..Default::default() };
        let scene = |part: [i32; 4]| Scene {
            screen_size: screen,
            interface: scene::Interface { popup: Some(257), ..Default::default() },
            views: vec![
                ViewState { id: 255, view: View { interface_only: true, menu: true, ..Default::default() }, overlay: vec![sprite(1, (menu[0], menu[1]))] },
                ViewState { id: 257, view: View { interface_only: true, origin: (part[0], part[1]), viewport: (part[2] as u32, part[3] as u32), ..Default::default() },
                    overlay: vec![sprite(2, (0, 0)), sprite(1, (menu[0], menu[1]))] },
            ],
            ..Default::default()
        };
        let settings = Resolution { pixel_scale: 2, scale_menus: true, menu_scale_percent: 100.0, ..Default::default() };
        let shown = |part: [i32; 4]| {
            let scene = scene(part);
            let p = plan(window, screen, &settings).for_scene(&scene, rect_of);
            let mut layout = layout::Layout::default();
            layout.driver_screen = Some(p.screen);
            layout.menu_scale = p.menu.zoom;
            let mut f = layout.frame(&scene, p.canvas(), (0, 0, 0, 0), vec![], None);
            let shown = arrange_panned(&p, &scene, &mut f, &mut layout.menu_pan, rect_of).shown.unwrap();
            let of = |r: [i32; 4]| *shown.iter().find(|s| s.driver == r).expect("shown");
            (p.canvas(), of(menu), of(part))
        };
        let (canvas, title, board) = shown(menu);
        assert_eq!(canvas, (1629, 952));
        assert_eq!((title.rect[1], title.rect[3]), (0, 952), "the title fills the height: {:?}", title.rect);
        assert_eq!((board.piece.zoom, board.rect), (title.piece.zoom, title.rect), "as the title");
        let (_, _, window_sized) = shown([0, 0, 3274, 2048]);
        assert!(window_sized.rect[2] <= 1629 && board.rect[2] > 2 * scaled(1024, window_sized.piece.zoom) as i32,
            "a window-sized part is shrunk: {:?} vs {:?}", window_sized.rect, board.rect);
    }

    /// In a game (split views, the user's macOS setup: the game's 3274×2048
    /// screen in a 3274×2048 window at pixel scale ×3, menus scaled to
    /// 100 %): a modal popup over Preferences (C3X's info box at the top of
    /// the game's screen, the "discard changes?" prompt over its middle)
    /// leaves Preferences as without it. Floating, Preferences is fitted as
    /// at the title (the window, not the pointer's view); with the one
    /// screen place setting naming a pane, it fits that pane. The popup
    /// shows across in the middle of that region, its top `MODAL_TOP` down,
    /// never at its place in the game's screen over the world, and takes
    /// the clicks on its button; Preferences takes those on it.
    #[test]
    fn a_modal_over_preferences_in_a_game_keeps_preferences_and_shows_in_the_top_third() {
        use scene::{Interactable, InteractableKind, View, ViewState};
        use viewer_settings::{Place, ScreenPlace, Split};
        let (screen, window) = ((3274u32, 2048u32), (3274u32, 2048u32));
        let (hud, background, info, button) = (1, 2, 3, 4);
        let prefs = [(screen.0 as i32 - 1024) / 2, (screen.1 as i32 - 768) / 2, 1024, 768];
        let control = [prefs[0] + 40, prefs[1] + 700, 64, 24];
        let rect_of = |s: &FreeSprite| Some(match s.set {
            x if x == hud => [s.x, s.y, 300, 120],
            x if x == background => [s.x, s.y, 1024, 768],
            x if x == info => [s.x, s.y, 650, 300],
            _ => [s.x, s.y, 60, 24],
        });
        let sprite = |set: u32, (x, y): (i32, i32)| FreeSprite { set, x, y, screen: true, after_pass: u32::MAX, ..Default::default() };
        let ui = |id: u32, [x, y, w, h]: [i32; 4], overlay: Vec<FreeSprite>| ViewState { id, view: View { interface_only: true, origin: (x, y), viewport: (w as u32, h as u32), ..Default::default() }, overlay };
        // C3X's info box at the top of the game's screen; the prompt over
        // the middle of Preferences.
        let popups = [[(screen.0 as i32 - 650) / 2, 0, 650, 300], [prefs[0] + 187, prefs[1] + 234, 650, 300]];
        for popup in popups {
            let ok = [popup[0] + 295, popup[1] + 250, 60, 24];
            let game = |with: bool| {
                let mut views = vec![
                    ViewState { id: 0, view: View { camera_x: 1000, camera_y: 500, zoom: (1, 1), ..Default::default() }, overlay: vec![] },
                    ViewState { id: 254, view: View { interface_only: true, ..Default::default() }, overlay: vec![sprite(hud, (0, screen.1 as i32 - 120))] },
                    ui(258, prefs, vec![sprite(background, (prefs[0], prefs[1])), sprite(button, (control[0], control[1]))]),
                ];
                if with {
                    views.push(ui(257, popup, vec![sprite(info, (popup[0], popup[1])), sprite(button, (ok[0], ok[1]))]));
                }
                Scene {
                    screen_size: screen,
                    views,
                    shape: scene::WorldShape { cols: 60, rows: 62, wrap_x: true, wrap_y: false },
                    interface: scene::Interface { screen: Some("Menu".into()), menu_fit: true, screen_view: Some(258), popup: with.then_some(257), ..Default::default() },
                    interactables: vec![
                        Interactable { region: None, alt_right_click: false, id: 1, rect: control, anchor: None, kind: InteractableKind::Button },
                        Interactable { region: None, alt_right_click: false, id: 2, rect: ok, anchor: None, kind: InteractableKind::Button },
                    ].into_iter().take(if with { 2 } else { 1 }).collect(),
                    ..Default::default()
                }
            };
            let title = |with: bool| Scene { shape: Default::default(), views: game(with).views.into_iter().filter(|v| v.view.interface_only && v.id != 254).collect(), ..game(with) };
            let settings = Resolution { pixel_scale: 3, scale_menus: true, menu_scale_percent: 100.0, ..Default::default() };
            for (split, axis) in [(Split::Vertical1To1, "vertical"), (Split::Horizontal1To1, "horizontal")] {
                for place in [Place::Floating, Place::LeftOrBottom, Place::RightOrTop] {
                    for menu_follows_pointer in [true, false] {
                        let what = format!("popup {popup:?}, {axis} split, {place:?}, menu follows pointer {menu_follows_pointer}");
                        // The layout as the user's settings make it, the
                        // pointer in the pane on the other side from A.
                        let arranged = |scene: &Scene| {
                            let p = plan(window, screen, &settings).for_scene(scene, rect_of);
                            assert_eq!(p.canvas(), (1091, 682), "{what}");
                            let mut layout = layout::Layout::default();
                            layout.configure(&format!("split={axis}")).unwrap();
                            layout.apply_settings(&viewer_settings::Layout { split, menu_follows_pointer, screen_place: ScreenPlace { vertical: place, horizontal: place }, ..Default::default() });
                            layout.driver_screen = Some(p.screen);
                            layout.menu_scale = p.menu.zoom;
                            let mut closed = scene.clone();
                            closed.interface = scene::Interface::default();
                            closed.views.retain(|v| v.id == 0 || v.id == 254);
                            if scene.shape.cols > 0 {
                                layout.frame(&closed, p.canvas(), (0, 0, 0, 0), vec![], None);
                                let b = layout.panes().1.unwrap();
                                layout.route(scene::Input::MouseMove { x: b[0] + b[2] / 2, y: b[1] + b[3] / 2 });
                            }
                            let mut shown = None;
                            for _ in 0..2 {
                                layout.advance(std::time::Duration::from_millis(16));
                                let mut f = layout.frame(scene, p.canvas(), (0, 0, 0, 0), vec![], None);
                                let a = arrange_panned(&p, scene, &mut f, &mut layout.menu_pan, rect_of).shown;
                                shown = a.clone();
                                layout.set_interface_shown(a);
                            }
                            (p, layout, shown)
                        };
                        let (_, l0, _) = arranged(&game(false));
                        let (p, l, shown) = arranged(&game(true));
                        let (canvas, rect) = ((p.canvas().0 as i32, p.canvas().1 as i32), |l: &layout::Layout, r: [i32; 4]| l.driver_rect_to_window(r).map(|v| v.round() as i32));
                        // (Preferences by its corners: the prompt covers its middle.)
                        let shown_prefs = |l: &layout::Layout| {
                            let ([x, y, ..], [r, b, w, h]) = (l.driver_rect_to_window([prefs[0], prefs[1], 1, 1]), l.driver_rect_to_window([prefs[0] + prefs[2] - 1, prefs[1] + prefs[3] - 1, 1, 1]));
                            [x.round() as i32, y.round() as i32, (r + w - x).round() as i32, (b + h - y).round() as i32]
                        };
                        let region = match place {
                            Place::Floating => [0, 0, canvas.0, canvas.1],
                            Place::LeftOrBottom if split == Split::Horizontal1To1 => l.panes().1.unwrap(),
                            Place::RightOrTop if split == Split::Vertical1To1 => l.panes().1.unwrap(),
                            _ => l.panes().0,
                        };
                        // Preferences: as without the popup; floating, as at
                        // the title; in a pane, fitted into it.
                        let m = shown_prefs(&l);
                        assert_eq!(m, shown_prefs(&l0), "{what}: Preferences as without the popup");
                        if place == Place::Floating {
                            let (_, t, _) = arranged(&title(false));
                            assert_eq!(m, shown_prefs(&t), "{what}: Preferences as at the title");
                            let (_, t, _) = arranged(&title(true));
                            let (a, b) = (rect(&l, popup), rect(&t, popup));
                            assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 1), "{what}: the popup {a:?} as at the title {b:?}");
                        }
                        let within = |r: [i32; 4], g: [i32; 4]| r[0] >= g[0] - 1 && r[1] >= g[1] - 1 && r[0] + r[2] <= g[0] + g[2] + 1 && r[1] + r[3] <= g[1] + g[3] + 1;
                        assert!(within(m, region), "{what}: Preferences {m:?} in {region:?}");
                        assert!((m[2] - region[2]).abs() <= 2 || (m[3] - region[3]).abs() <= 2, "{what}: Preferences {m:?} fills {region:?}");
                        assert!((2 * m[0] + m[2] - 2 * region[0] - region[2]).abs() <= 2, "{what}: Preferences {m:?} across in the middle of {region:?}");
                        // The popup: across in the middle of the region, its
                        // top a sixth of the way down, whole.
                        let [x, y, w, h] = rect(&l, popup);
                        let [rx, ry, rw, rh] = region;
                        assert!((2 * x + w - 2 * rx - rw).abs() <= 2, "{what}: across in the middle: {:?} in {region:?}", [x, y, w, h]);
                        assert!((y - ry - (rh * MODAL_TOP.0 as i32 / MODAL_TOP.1 as i32).min(rh - h)).abs() <= 1, "{what}: top: {:?} in {region:?}", [x, y, w, h]);
                        assert!(within([x, y, w, h], region) && y - ry <= rh / 3, "{what}: in the top third, whole: {:?} in {region:?}", [x, y, w, h]);
                        if let Some(shown) = &shown {
                            let modal = shown.iter().find(|s| s.driver == popup).expect("the popup shown");
                            assert!(modal.rect.iter().zip([x, y, w, h]).all(|(a, b)| (a - b).abs() <= 1), "{what}: {:?} drawn at {:?}", modal.rect, [x, y, w, h]);
                        }
                        // Clicks on its button reach the button; those on
                        // Preferences' control (beside the popup) that one.
                        let with = game(true);
                        let targets = crate::snap::targets_mapped(&with.interactables, |r| l.driver_rect_to_window(r), p.canvas());
                        assert_eq!(targets.len(), 2, "{what}");
                        for (t, want) in targets.iter().zip([control, ok]) {
                            let c = t.center();
                            let driver = l.route_window((c.0 as i32, c.1 as i32)).expect("the control takes the pointer");
                            assert!(inside_rect(want, driver), "{what}: {c:?} -> {driver:?}, not on {want:?}");
                        }
                        // Preferences beside the popup never reaches the
                        // popup where the game has it.
                        for i in 0..=8 {
                            for j in 0..=8 {
                                let at = (m[0] + 1 + (m[2] - 3) * i / 8, m[1] + 1 + (m[3] - 3) * j / 8);
                                if inside_rect([x, y, w, h], at) { continue; }
                                let driver = l.route_window(at).expect("Preferences takes the pointer");
                                assert!(inside_rect(prefs, driver) && !inside_rect(popup, driver), "{what}: {at:?} -> {driver:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    fn inside_rect([x, y, w, h]: [i32; 4], (px, py): (i32, i32)) -> bool {
        px >= x && py >= y && px < x + w && py < y + h
    }

    #[test]
    fn scales_become_fractions() {
        assert_eq!(ratio(1.0), (1, 1));
        assert_eq!(ratio(1.5), (3, 2));
        assert_eq!(ratio(0.75), (3, 4));
        assert_eq!(ratio(1.333), (4, 3));
        assert_eq!(ratio(100.0), (8, 1));
        assert_eq!(ratio(f32::NAN), (1, 1));
    }
}
