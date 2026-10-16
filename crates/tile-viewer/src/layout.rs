//! How the viewer arranges the world in its window beyond what the driver
//! asks for: the driver's main view zoomed, a second pane beside it, floating
//! insets and a magnifier following the pointer. The driver's interface
//! (interface-only views) stays on top, unscaled, where the driver put it.
//!
//! Pointer input is routed through the same arrangement: what lands on the
//! interface goes to the driver unchanged, what lands on the world goes to
//! the point of the driver's main view showing the same world pixel, and
//! world pixels the driver's view does not show become [`Input::Look`].
//!
//! Controls (Ctrl+Shift with a key, so they do not collide with a game's):
//! S cycles the split (off, side by side, stacked), I adds an inset at the
//! pointer, X removes the inset under it, M cycles the magnifier (off,
//! everything on screen, the world only), +/− and Ctrl+wheel zoom what is
//! under the pointer, 0 resets its zoom, arrows pan it; a middle-button drag
//! pans a pane or inset, Ctrl+Shift+left drag moves an inset (resizes it at
//! its bottom-right corner), a left drag on the divider moves it.

use scene::{Button, Input, Scene};
use tile_render::{Camera, ScreenCopy};

/// Zoom steps, screen pixels per world pixel.
pub const ZOOMS: [(u32, u32); 11] = [(1, 4), (1, 3), (1, 2), (2, 3), (1, 1), (3, 2), (2, 1), (3, 1), (4, 1), (6, 1), (8, 1)];
const ONE: usize = 4;
/// Most insets at a time.
pub const MAX_INSETS: usize = 3;
/// Gap between the panes, and the frame around insets and the magnifier.
const GAP: i32 = 4;

/// A view the viewer moves itself: the world pixel at its centre and zoom.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FreeView {
    pub center: (i32, i32),
    pub zoom: usize,
}

impl FreeView {
    fn camera(&self, rect: [i32; 4], overhang: (u32, u32, u32, u32)) -> Camera {
        let zoom = ZOOMS[self.zoom];
        let (w, h) = (rect[2].max(1) as u32, rect[3].max(1) as u32);
        let (sw, sh) = ((w * zoom.1).div_ceil(zoom.0) as i32, (h * zoom.1).div_ceil(zoom.0) as i32);
        Camera {
            x: self.center.0 - sw / 2,
            y: self.center.1 - sh / 2,
            zoom,
            origin: (rect[0], rect[1]),
            overhang,
            ..Camera::new((w, h))
        }
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

/// What one frame draws: cameras in drawing order, for each scene view the
/// camera that shows its overlay, and screen copies on top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub cameras: Vec<Camera>,
    pub view_cameras: Vec<Option<u32>>,
    pub copies: Vec<ScreenCopy>,
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
    pub main_zoom: usize,
    pub split: Option<Split>,
    pub insets: Vec<Inset>,
    pub magnifier: Option<Magnifier>,
    window: (u32, u32),
    pointer: (i32, i32),
    ctrl: bool,
    shift: bool,
    drag: Option<Drag>,
    /// Per button: whether its press went to the driver, and where.
    pressed: [Option<(i32, i32)>; 3],
    /// From the last frame: the driver's main view (scene camera, unzoomed)
    /// and where its world is shown, the interface rectangles.
    main: Option<Camera>,
    shown: Vec<(Hit, Camera)>,
    interface: Vec<[i32; 4]>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            main_zoom: ONE,
            split: None,
            insets: Vec::new(),
            magnifier: None,
            window: (1, 1),
            pointer: (0, 0),
            ctrl: false,
            shift: false,
            drag: None,
            pressed: [None; 3],
            main: None,
            shown: Vec::new(),
            interface: Vec::new(),
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

impl Layout {
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

    /// The frame for `scene` in a window of `window`. `interface`: the
    /// rectangles the driver's interface covers (window pixels).
    pub fn frame(&mut self, scene: &Scene, window: (u32, u32), overhang: (u32, u32, u32, u32), interface: Vec<[i32; 4]>) -> Frame {
        self.window = window;
        self.interface = interface;
        self.shown.clear();
        let mut frame = Frame { view_cameras: vec![None; scene.views.len()], ..Default::default() };
        let base = crate::scene_cameras(scene, window, overhang);
        // The driver's main view: its first world view filling the window.
        let main = scene.views.iter().position(|v| !v.view.interface_only && (v.view.viewport.0 == 0 || v.view.viewport.1 == 0));
        self.main = main.map(|i| base[i]);
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
            let camera = if Some(i) == main { self.arranged_main(camera, main_rect) } else { *camera };
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
            let camera = split.view.camera(rect, overhang);
            push(&mut frame, camera);
            self.shown.push((Hit::Pane, camera));
        }
        for (k, inset) in self.insets.iter().enumerate() {
            push(&mut frame, frame_of(inset.rect));
            let camera = inset.view.camera(inset.rect, overhang);
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
                    let center = self.world_under(self.pointer).unwrap_or((0, 0));
                    push(&mut frame, frame_of(rect));
                    let camera = FreeView { center, zoom: m.zoom }.camera(rect, overhang);
                    push(&mut frame, camera);
                }
            }
        }
        frame
    }

    /// The driver's main camera as shown in `rect`: zoomed about its centre.
    fn arranged_main(&self, camera: &Camera, rect: [i32; 4]) -> Camera {
        let zoom = ZOOMS[self.main_zoom];
        let (w, h) = (rect[2].max(1) as u32, rect[3].max(1) as u32);
        let (sw, sh) = ((w * zoom.1).div_ceil(zoom.0) as i32, (h * zoom.1).div_ceil(zoom.0) as i32);
        let (cx, cy) = (camera.x + camera.viewport.0 as i32 / 2, camera.y + camera.viewport.1 as i32 / 2);
        let (x, y) = if zoom == (1, 1) && rect[0] == camera.origin.0 && rect[1] == camera.origin.1 { (camera.x, camera.y) } else { (cx - sw / 2, cy - sh / 2) };
        // The driver's screen sprites (labels) stay on their world pixels.
        let shift = (camera.x - x + camera.origin.0 - rect[0], camera.y - y + camera.origin.1 - rect[1]);
        Camera { x, y, zoom, viewport: (w, h), origin: (rect[0], rect[1]), screen_shift: shift, ..*camera }
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

    /// Where a pointer event at `p` goes in the driver's window: `Ok(q)`, or
    /// `Err(world pixel)` if the driver does not show it, `Err` of `None` if
    /// nothing.
    fn route_point(&self, p: (i32, i32)) -> Result<(i32, i32), Option<(i32, i32)>> {
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
                        let w = (c.0 - sw / 2 + off.0, c.1 - sh / 2 + off.1);
                        self.in_main(w).ok_or(Some(w))
                    }
                }
            }
            hit => {
                let (_, camera) = self.shown.iter().find(|(h, _)| *h == hit).ok_or(None)?;
                let w = world_at(camera, p);
                match self.in_main(w) {
                    // On the driver's interface there: it would take it.
                    Some(q) if self.interface.iter().any(|&r| inside(r, q)) => Err(Some(w)),
                    Some(q) => Ok(q),
                    None => Err(Some(w)),
                }
            }
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
            if self.drag.take().is_some() && self.pressed[b].is_none() {
                return vec![];
            }
            // The release goes where the press went.
            return match self.pressed[b].take() {
                Some(_) => {
                    let q = self.route_point(p).unwrap_or_else(|_| self.clamp_to_main(p));
                    vec![Input::MouseButton { button, down, x: q.0, y: q.1 }]
                }
                None => vec![],
            };
        }
        let hit = self.hit(p);
        // Viewer gestures.
        match (button, hit) {
            (Button::Middle, Hit::Pane | Hit::Inset(_)) => {
                if let Some(view) = self.free_view(hit) {
                    self.drag = Some(Drag::Pan { hit, from: p, center: view.center });
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
            _ => {}
        }
        match self.route_point(p) {
            Ok(q) => {
                self.pressed[b] = Some(q);
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
        match self.hit(p) {
            Hit::Magnifier => {
                if let Some(m) = &mut self.magnifier {
                    m.zoom = next(m.zoom).max(ONE);
                }
            }
            hit @ (Hit::Pane | Hit::Inset(_)) => {
                if let Some(v) = self.free_view(hit) {
                    v.zoom = next(v.zoom);
                }
            }
            _ => self.main_zoom = next(self.main_zoom),
        }
    }

    /// A Ctrl+Shift command key; whether it was one.
    fn command(&mut self, vk: u32, down: bool) -> bool {
        const KEYS: [u32; 12] = [0x53, 0x49, 0x58, 0x4D, 0x30, 0xBB, 0xBD, 0x6B, 0x6D, 0x25, 0x26, 0x27];
        if !KEYS.contains(&vk) && vk != 0x28 {
            return false;
        }
        if !down {
            return true;
        }
        let p = self.pointer;
        let start = self.world_under(p).or_else(|| self.main.map(|m| (m.x + m.viewport.0 as i32 / 2, m.y + m.viewport.1 as i32 / 2))).unwrap_or((0, 0));
        match vk {
            // S: split off → side by side → stacked → off.
            0x53 => {
                self.split = match self.split {
                    None => Some(Split { axis: Axis::Vertical, fraction: 0.5, view: FreeView { center: start, zoom: ONE } }),
                    Some(s) if s.axis == Axis::Vertical => Some(Split { axis: Axis::Horizontal, ..s }),
                    Some(_) => None,
                }
            }
            // I: an inset at the pointer.
            0x49 if self.insets.len() < MAX_INSETS => {
                let (w, h) = (self.window.0 as i32 / 4, self.window.1 as i32 / 4);
                self.insets.push(Inset { rect: [p.0 - w / 2, p.1 - h / 2, w.max(48), h.max(32)], view: FreeView { center: start, zoom: ONE + 2 } });
            }
            // X: remove the inset under the pointer (else the last one).
            0x58 => {
                let k = match self.hit(p) {
                    Hit::Inset(k) => Some(k),
                    _ => self.insets.len().checked_sub(1),
                };
                if let Some(k) = k {
                    self.insets.remove(k);
                }
            }
            // M: magnifier off → everything → world → off.
            0x4D => {
                self.magnifier = match self.magnifier {
                    None => Some(Magnifier { mode: MagnifierMode::Screen, zoom: ONE + 3, size: (240, 160) }),
                    Some(m) if m.mode == MagnifierMode::Screen => Some(Magnifier { mode: MagnifierMode::World, ..m }),
                    Some(_) => None,
                }
            }
            // 0: zoom 1 for what is under the pointer.
            0x30 => match self.hit(p) {
                hit @ (Hit::Pane | Hit::Inset(_)) => {
                    if let Some(v) = self.free_view(hit) {
                        v.zoom = ONE;
                    }
                }
                Hit::Magnifier => {}
                _ => self.main_zoom = ONE,
            },
            0xBB | 0x6B => self.zoom_at(p, 1),
            0xBD | 0x6D => self.zoom_at(p, -1),
            // Arrows: pan what is under the pointer by an eighth of it.
            0x25..=0x28 => {
                let hit = self.hit(p);
                let size = self.shown.iter().find(|(h, _)| *h == hit).map(|(_, c)| c.source_size());
                if let (Some(v), Some((w, h))) = (self.free_view(hit), size) {
                    let (dx, dy) = match vk {
                        0x25 => (-(w as i32) / 8, 0),
                        0x26 => (0, -(h as i32) / 8),
                        0x27 => (w as i32 / 8, 0),
                        _ => (0, h as i32 / 8),
                    };
                    v.center = (v.center.0 + dx, v.center.1 + dy);
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
            ..Default::default()
        }
    }

    fn click(l: &mut Layout, p: (i32, i32)) -> Vec<Input> {
        let mut out = l.route(Input::MouseButton { button: Button::Left, down: true, x: p.0, y: p.1 });
        out.extend(l.route(Input::MouseButton { button: Button::Left, down: false, x: p.0, y: p.1 }));
        out
    }

    #[test]
    fn unzoomed_main_view_passes_input_through() {
        let mut l = Layout::default();
        let f = l.frame(&scene(), (800, 600), (0, 0, 0, 0), vec![]);
        assert_eq!(f.cameras.len(), 2);
        assert_eq!((f.cameras[0].x, f.cameras[0].y, f.cameras[0].screen_shift), (1000, 500, (0, 0)));
        assert_eq!(click(&mut l, (10, 20)), [
            Input::MouseButton { button: Button::Left, down: true, x: 10, y: 20 },
            Input::MouseButton { button: Button::Left, down: false, x: 10, y: 20 }
        ]);
    }

    #[test]
    fn zoomed_main_view_maps_clicks_to_the_same_world_pixel() {
        let mut l = Layout { main_zoom: ONE + 2, ..Default::default() }; // 2/1
        let f = l.frame(&scene(), (800, 600), (0, 0, 0, 0), vec![[0, 500, 800, 100]]);
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
        l.route(Input::Key { vk: VK_CONTROL, down: true, text: None });
        l.route(Input::Key { vk: VK_SHIFT, down: true, text: None });
        l.frame(&scene(), (800, 600), (0, 0, 0, 0), vec![]);
        assert!(l.route(Input::Key { vk: 0x53, down: true, text: None }).is_empty());
        l.split.as_mut().unwrap().view.center = (5000, 5000);
        let f = l.frame(&scene(), (800, 600), (0, 0, 0, 0), vec![]);
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
        let mut l = Layout { magnifier: Some(Magnifier { mode: MagnifierMode::Screen, zoom: ONE + 2, size: (200, 100) }), ..Default::default() };
        l.route(Input::MouseMove { x: 300, y: 300 });
        let f = l.frame(&scene(), (800, 600), (0, 0, 0, 0), vec![]);
        assert_eq!(f.copies, [ScreenCopy { source: [250, 275, 100, 50], at: (324, 250), zoom: (2, 1) }]);
        // A click on the magnifier goes to what it shows.
        assert_eq!(click(&mut l, (334, 260))[0], Input::MouseButton { button: Button::Left, down: true, x: 255, y: 280 });
    }
}
