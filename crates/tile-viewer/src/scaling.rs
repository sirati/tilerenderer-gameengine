//! How the viewer shows its picture and the driver's interface in its
//! window, at the user's resolution settings (`viewer_settings::Resolution`).
//!
//! The viewer renders at its window's size divided by the pixel scale `n`
//! (a whole number; Auto: the desktop's scale rounded down), the canvas,
//! and shows each rendered pixel as an exact `n`×`n` box of window pixels
//! (`tile_render::Present`; what is left of the window when it does not
//! divide is a border of fewer than `n` pixels). Nothing is filtered or
//! scaled by a fraction there: on a screen at 200 %, Auto gives 2×2 boxes.
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

use scene::{FreeSprite, Scene};
use tile_render::{Camera, Present};
use viewer_settings::Resolution;

use crate::layout;

/// A fraction (numerator, denominator).
pub type Ratio = (u32, u32);

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

fn reduced(n: u64, d: u64) -> Ratio {
    let g = gcd(n, d);
    ((n / g).max(1) as u32, (d / g).max(1) as u32)
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

    /// Rectangle `r` (x, y, w, h) of the driver's pixels on the canvas.
    pub fn rect(&self, [x, y, w, h]: [i32; 4]) -> [i32; 4] {
        let (x0, y0) = self.apply((x, y));
        let (x1, y1) = self.apply((x + w, y + h));
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
    /// The zoom of its frame's panels (each about its own side or corner).
    pub frame: Ratio,
    /// The world's (`Layout::world_scale`).
    pub world: Ratio,
}

impl Plan {
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

/// The plan for a window of `window` physical pixels on a desktop scaled
/// `desktop` times (for `pixel_scale` Auto), a driver's screen of `screen`
/// ((0, 0): unknown, then the canvas) and `settings`.
pub fn plan(window: (u32, u32), desktop: f64, screen: (u32, u32), settings: &Resolution) -> Plan {
    let window = (window.0.max(1), window.1.max(1));
    let n = settings.pixel_scale_in(window, desktop).max(1);
    let canvas = settings.realised(window, desktop);
    let screen = if screen.0 > 0 && screen.1 > 0 { screen } else { canvas };
    // (What is left of the window: a border of fewer than n pixels.)
    let at = ((window.0 - canvas.0 * n) as i32 / 2, (window.1 - canvas.1 * n) as i32 / 2);
    let present = Present { canvas, zoom: (n, 1), at, target: window };
    let center = |s: (u32, u32)| ((s.0 / 2) as i32, (s.1 / 2) as i32);
    let menu = Affine { from: center(screen), to: center(canvas), zoom: ratio(settings.menu_scale) };
    Plan { present, screen, menu, frame: ratio(settings.frame_scale), world: ratio(settings.world_scale) }
}

/// One piece of the driver's interface (or one of its other views) as
/// shown: where it takes the pointer (the canvas's pixels), where that is
/// in the driver's window, and back.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ShownPiece {
    pub rect: [i32; 4],
    /// Its rectangle in the driver's window.
    pub driver: [i32; 4],
    piece: Affine,
    /// How far the layout moved the piece (`Camera::screen_shift`, e.g. a
    /// popup moved into a pane).
    shift: (i32, i32),
}

impl ShownPiece {
    /// The driver's pixel for canvas pixel `p` on the piece.
    pub fn to_driver(&self, p: (i32, i32)) -> (i32, i32) {
        let (x, y) = self.piece.invert(p);
        (x - self.shift.0, y - self.shift.1)
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

/// The driver's interface and other views (cameras of the layout's
/// `frame` in the driver's pixels) shown on the canvas as `plan` says: the
/// menus and the driver's views within them about the middle (a popup
/// about its own middle, where the layout put it), the frame's panels each
/// at its side or corner (one camera per side or corner in use), the
/// roaming controls at their top-left (`scene::Interface`); no menu or
/// panel larger than the canvas. The viewer's own views (the world, panes, insets) are
/// the canvas's already. `rect_of`: a screen sprite's rectangle.
pub fn arrange(plan: &Plan, scene: &Scene, frame: &mut layout::Frame, rect_of: impl Fn(&FreeSprite) -> Option<[i32; 4]>) -> Arranged {
    let mut out = Arranged::default();
    // A driver's screen fitted into a pane of the layout's (its views
    // stacked, drawn into its picture): the layout placed it all.
    if frame.cameras.iter().any(|c| c.stacked) {
        return out;
    }
    let canvas = plan.canvas();
    let parts = &scene.interface;
    // Per camera: its map, if it is one of the driver's; or (a frame in
    // panels) the scene view and per panel its map and rectangle, per
    // sprite its panel.
    let mut pieces: Vec<Option<Affine>> = vec![None; frame.cameras.len()];
    let mut panelled: Vec<(usize, usize, Vec<(Affine, [i32; 4])>, Vec<usize>)> = Vec::new();
    // What takes the pointer: per rectangle (the driver's pixels) its map
    // and how far the layout moved it.
    let mut shown: Vec<(Affine, [i32; 4], (i32, i32))> = Vec::new();
    // A menu no larger than the canvas: scaled up only as far as it fits
    // (else its edges, and buttons on them, would be off it).
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
    let menu_rects: Vec<[i32; 4]> = scene.views.iter().filter(|v| v.view.interface_only && v.view.menu).flat_map(|v| v.overlay.iter().filter_map(&rect_of)).collect();
    let menu_box = bounding(&menu_rects);
    // (About the middle, but moved as little as needed to be whole on the
    // canvas: e.g. a menu at the top of a screen larger than the canvas.)
    let menu = Affine { zoom: fitting(plan.menu.zoom, menu_box), ..plan.menu };
    let menu = menu_box.map_or(menu, |b| kept(menu, b));
    // The driver's other views outside its menus: about the middle, as
    // large as they are.
    let middle = Affine { zoom: (1, 1), ..plan.menu };
    for (i, v) in scene.views.iter().enumerate() {
        let Some(c) = frame.view_cameras.get(i).copied().flatten().map(|c| c as usize) else { continue };
        let camera = frame.cameras[c];
        if !v.view.interface_only {
            // (The main one is the layout's own arrangement.)
            if v.view.viewport.0 > 0 && v.view.viewport.1 > 0 {
                let r = [camera.origin.0, camera.origin.1, camera.viewport.0 as i32, camera.viewport.1 as i32];
                let within = menu_box.is_some_and(|m| r[0] >= m[0] && r[1] >= m[1] && r[0] + r[2] <= m[0] + m[2] && r[1] + r[3] <= m[1] + m[3]);
                let piece = if within { menu } else { middle };
                pieces[c] = Some(piece);
                shown.push((piece, r, (0, 0)));
            }
            continue;
        }
        let (o, size) = (camera.origin, camera.viewport);
        // (Where the layout put it: moved parts about their own place.)
        let about = |from: (i32, i32), zoom: Ratio| Affine { from, to: from, zoom };
        let piece = if parts.popup == Some(v.id) || parts.screen_view == Some(v.id) {
            // A popup, or a screen part (an advisor): about its middle,
            // placed as the menus are (and moved as the layout moved it),
            // kept on the canvas.
            let d = camera.screen_shift;
            let (w, h) = (size.0 as i32, size.1 as i32);
            let zoom = fitting(plan.menu.zoom, Some([0, 0, w, h]));
            let from = (o.0 - d.0 + w / 2, o.1 - d.1 + h / 2);
            let placed = middle.apply(from);
            let (hw, hh) = (scaled(w as i64 / 2, zoom) as i32, scaled(h as i64 / 2, zoom) as i32);
            let keep = |v: i32, half: i32, side: u32| if 2 * half <= side as i32 { v.clamp(half, side as i32 - half) } else { side as i32 / 2 };
            Affine { from: (from.0 + d.0, from.1 + d.1), to: (keep(placed.0 + d.0, hw, canvas.0), keep(placed.1 + d.1, hh, canvas.1)), zoom }
        } else if parts.roaming == Some(v.id) {
            let r = [o.0, o.1, size.0 as i32, size.1 as i32];
            kept(about(o, fitting(plan.frame, Some(r))), r)
        } else if v.view.menu {
            menu
        } else {
            // The frame's panels, each anchored at its side or corner.
            let (anchors, of_sprite) = panels(plan.screen, &v.overlay, &rect_of);
            // (Each no larger than the canvas.)
            let anchors: Vec<(Affine, [i32; 4])> = anchors.into_iter().map(|(a, r)| {
                let piece = plan.frame_piece(a);
                (kept(Affine { zoom: fitting(piece.zoom, Some(r)), ..piece }, r), r)
            }).collect();
            if anchors.len() > 1 || anchors.first().is_some_and(|a| a.0.zoom != (1, 1) || a.0.from != a.0.to) {
                for (s, &k) in v.overlay.iter().zip(&of_sprite) {
                    shown.extend(rect_of(s).map(|r| (anchors[k].0, r, (0, 0))));
                }
                panelled.push((c, i, anchors, of_sprite));
                continue;
            }
            Affine::IDENTITY
        };
        pieces[c] = Some(piece);
        let d = camera.screen_shift;
        shown.extend(v.overlay.iter().filter_map(&rect_of).map(|r| (piece, [r[0] + d.0, r[1] + d.1, r[2], r[3]], d)));
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
            shown.retain(|(p, _, _)| !anchors.iter().any(|a| a.0 == *p));
            shown.extend(scene.views[i].overlay.iter().filter_map(&rect_of).map(|r| (Affine::IDENTITY, r, (0, 0))));
            pieces[c] = Some(Affine::IDENTITY);
        }
    }
    let mut cameras = Vec::with_capacity(frame.cameras.len() + extra + 1);
    let mut index = vec![0u32; frame.cameras.len()];
    for (k, camera) in frame.cameras.iter().enumerate() {
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
            (None, Some(piece)) if !camera.cells => cameras.push(overlaid(piece.camera(camera))),
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
    // The viewer's own screen sprites: over everything.
    if cameras.len() < tile_render::MAX_VIEWS {
        cameras.push(Camera { cells: false, ..Camera::new(canvas) });
        out.overlay = Some(cameras.len() as u32 - 1);
    }
    frame.cameras = cameras;
    let shown = shown.into_iter().map(|(piece, r, shift)| ShownPiece { rect: piece.rect(r), driver: [r[0] - shift.0, r[1] - shift.1, r[2], r[3]], piece, shift });
    out.shown = Some(shown.filter(|s| s.rect[2] > 0 && s.rect[3] > 0).collect());
    out
}

/// An interface camera as drawn over what is below it: scaled, it is drawn
/// at its zoom straight onto the picture (`Camera::hairline`), not into a
/// picture of its own that would cover its rectangle with the background.
fn overlaid(camera: Camera) -> Camera {
    Camera { hairline: camera.hairline || camera.is_scaled(), ..camera }
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

    fn res(pixel_scale: u32, menu: f32, frame: f32, world: f32) -> Resolution {
        Resolution { pixel_scale, menu_scale: menu, frame_scale: frame, world_scale: world }
    }

    #[test]
    fn the_picture_is_the_window_in_whole_boxes() {
        // A 4K screen at 200 %: Auto renders 1920×1080, each pixel 2×2.
        let p = plan((3840, 2160), 2.0, (1920, 1080), &Resolution::default());
        assert_eq!(p.present, Present { canvas: (1920, 1080), zoom: (2, 1), at: (0, 0), target: (3840, 2160) });
        // At 150 %: rounded down, ×1 (no fractional boxes).
        let p = plan((3840, 2160), 1.5, (2560, 1440), &Resolution::default());
        assert_eq!((p.present.canvas, p.present.zoom), ((3840, 2160), (1, 1)));
        // ×1 and ×3 at 200 %: the same driver's screen, other pictures.
        let p = plan((3840, 2160), 2.0, (1920, 1080), &res(1, 1.0, 1.0, 1.0));
        assert_eq!((p.present.canvas, p.present.zoom), ((3840, 2160), (1, 1)));
        let p = plan((3840, 2160), 2.0, (1920, 1080), &res(3, 1.0, 1.0, 1.0));
        assert_eq!((p.present.canvas, p.present.zoom, p.screen), ((1280, 720), (3, 1), (1920, 1080)));
        // A window that does not divide: a border of fewer than n pixels.
        let p = plan((2001, 1081), 1.0, (1920, 1080), &res(2, 1.0, 1.0, 1.0));
        assert_eq!((p.present.canvas, p.present.at), ((1000, 540), (0, 0)));
        let p = plan((2003, 1083), 1.0, (1920, 1080), &res(4, 1.0, 1.0, 1.0));
        assert_eq!((p.present.canvas, p.present.at), ((500, 270), (1, 1)));
        // Unknown driver screen: the canvas.
        assert_eq!(plan((800, 600), 1.0, (0, 0), &Resolution::default()).screen, (800, 600));
        // Window pixels and canvas pixels: the middle of a box, and back.
        let p = plan((3840, 2160), 2.0, (1920, 1080), &Resolution::default());
        assert_eq!(p.to_window((100, 50)), (201, 101));
        assert_eq!(p.to_screen((201.0, 101.0)), (100, 50));
    }

    #[test]
    fn menus_and_frame_keep_to_the_canvas() {
        // ×1 at 200 %: the game's 1920×1080 interface at its own size on a
        // 3840×2160 picture: menus in its middle, panels at its corners.
        let p = plan((3840, 2160), 2.0, (1920, 1080), &res(1, 1.5, 0.5, 1.0));
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
        let p = plan((3840, 2160), 2.0, (1920, 1080), &res(1, 1.0, 1.0, 1.0));
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
    fn clicks_on_the_frame_reach_its_pixels_at_every_pixel_scale() {
        // A vertical split in a 4K window at 200 %: the minimap (bottom-left),
        // a top-left button and the unit panel (bottom-right) take clicks
        // where they are drawn, at the driver's pixels they show.
        let (scene, rect_of) = game_scene();
        let split = viewer_settings::Layout { split: viewer_settings::Split::Vertical1To1, ..Default::default() };
        for pixel_scale in [1, 2, 3] {
            let mut layout = layout::Layout::default();
            layout.apply_settings(&split);
            let p = plan((3832, 2106), 2.0, (1920, 1080), &res(pixel_scale, 1.0, 1.0, 1.0));
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
                    let p = plan(canvas, 1.0, (1920, 1080), &res(1, 1.0, frame_scale, 1.35));
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
                        assert_eq!(f.cameras.len(), if optional_views { 25 } else { 17 }, "the worker layout exceeds the former 16-view limit: {what}");
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
                        assert_eq!(piece.rect, p.frame_piece(anchor).rect(driver), "{what}: panel {driver:?}");
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
            for pixel_scale in [Resolution::AUTO, 1, 2, 3, 4, 5, 6] {
                for desktop in [1.0, 2.0] {
                    let settings = res(pixel_scale, 1.0, 1.0, 1.0);
                    let what = format!("{window:?} at ×{pixel_scale} on {desktop}");
                    let p = plan(window, desktop, (1920, 1080), &settings);
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
    }

    #[test]
    fn every_setting_keeps_a_whole_picture() {
        // Every pixel scale, desktop scale and part scale from the slider's
        // ends to its middle: whole boxes; the world fills the picture, every part has
        // a camera that draws over what is below it (none blots out the
        // world), takes the pointer where it shows and maps it back to the
        // driver's pixel it shows.
        let (scene, rect_of) = game_scene();
        let scales = [0.5, 0.7, 1.0, 1.2, 2.0, 3.0];
        for pixel_scale in [Resolution::AUTO, 1, 2, 3, 6] {
            for desktop in [1.0, 1.5, 2.0] {
                let mut combos = Vec::new();
                for m in scales {
                    for f in scales {
                        for w in scales {
                            combos.push((m, f, w));
                        }
                    }
                }
                for (menu, frame, world) in combos {
                    let settings = res(pixel_scale, menu, frame, world);
                    let what = format!("{settings:?} at {desktop}");
                    let p = plan((3840, 2160), desktop, (1920, 1080), &settings);
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
                        // The menu (1024×768 in the middle) on the picture whole.
                        let whole = shown.iter().filter_map(|s| {
                            let c = s.rect;
                            (c[2] * 2 >= canvas.0 as i32 / 3).then_some(c)
                        });
                        for c in whole {
                            assert!(c[0] >= -1 && c[1] >= -1 && c[0] + c[2] <= canvas.0 as i32 + 1 && c[1] + c[3] <= canvas.1 as i32 + 1, "{what}: menu at {c:?} on {canvas:?}");
                        }
                        assert!(shown.iter().all(|s| s.rect[2] > 0 && s.rect[3] > 0), "{what}");
                    }
                }
            }
        }
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
