//! The mouse cursor: the system's pointer showing the driver's cursor art
//! where it can, and the viewer's own sprites of it where the system's
//! cannot show it.
//!
//! - While the mouse is free, the system draws the pointer (smoother than
//!   anything drawn here): the driver's cursor (`scene::Cursor`) as a
//!   custom system cursor made from its sprite, or a standard one; over a
//!   view that a click captures the mouse in, the description's
//!   [`CAPTURE_SPRITES`] (else the system's crosshair).
//! - While the mouse is captured, the system pointer is hidden and the
//!   viewer draws the cursor itself at its logical place: the world cursor
//!   in every view that shows it.
//! - While button snapping is on ([`SnapMode`]), its own pointer and ghost
//!   (`crate::snap`) are the only cursor: no system pointer, nothing drawn
//!   here.
//! - But while the view following the world cursor is held at the edge of
//!   a world that does not wrap (the cursor off its middle), the system
//!   pointer shows at the world cursor and moves it, kept inside the
//!   window; once the view can centre the cursor again, the pointer is
//!   captured and hidden again ([`Machine`]: the system pointer is put
//!   where the drawn cursor was and shows only after the frame that
//!   applies that, so it never jumps).
//! - While the mouse is free, every other view showing the world pixel
//!   under the pointer (the other pane, insets, a zoom box) shows it too,
//!   drawn by the viewer; not where the system draws it.
//!
//! The viewer's drawings are screen sprites in a camera of their own, on
//! top of every view, clipped to the view they mark; they change without
//! the world being rebuilt (only the free sprites are uploaded again).

use std::collections::HashMap;
use std::time::Instant;

use render_desc::Compiled;
use tile_render::{Camera, PlacedSprite, SpriteSetId};

/// The sets whose first frames the viewer keeps the pixels of (for system
/// cursors made from them): its own (`Viewer@...`), among them the
/// driver's cursors (by convention `Viewer@cursors/<n>`).
pub const KEPT_SETS: &str = "Viewer@";

/// The pointer over a view that a click captures the mouse in: the first
/// set whose name starts with this.
pub const CAPTURE_SPRITES: &str = "Viewer@capture/";

/// What the viewer draws for a cursor whose art it does not have (e.g. a
/// system cursor while the mouse is captured): the first of these the
/// description has.
pub const FALLBACK_SPRITES: [&str; 2] = [crate::snap::POINTER_SPRITE, crate::CURSOR_SPRITE];

/// One cursor image: RGBA rows and the hot spot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub rgba: Vec<u8>,
    pub size: (u16, u16),
    pub hotspot: (u16, u16),
}

/// The pixels of the sets [`KEPT_SETS`] names, by set (taken from a
/// description before its pixels are dropped).
#[derive(Clone, Debug, Default)]
pub struct Art {
    images: HashMap<u32, Image>,
    capture: Option<u32>,
}

impl Art {
    /// The kept sets' first frames of `compiled` (which still has its
    /// pixels; without them, none).
    pub fn of(compiled: &Compiled) -> Self {
        let mut images = HashMap::new();
        for (_, set) in compiled.sets_with_prefix(KEPT_SETS) {
            if let Some(image) = image(compiled, set) {
                images.insert(set.raw(), image);
            }
        }
        let capture = compiled.sets_with_prefix(CAPTURE_SPRITES).map(|(_, s)| s.raw()).next();
        Self { images, capture }
    }

    pub fn get(&self, set: u32) -> Option<&Image> {
        self.images.get(&set)
    }

    /// The capture pointer's set, if the description has one.
    pub fn capture(&self) -> Option<u32> {
        self.capture.filter(|s| self.images.contains_key(s))
    }
}

/// The first frame of `set` as a cursor image (a colour sprite, or an
/// indexed one through its palette row): its anchor is minus the hot spot
/// (a hot spot outside the image is moved onto its edge).
fn image(compiled: &Compiled, set: SpriteSetId) -> Option<Image> {
    let id = compiled.sets.frame(set, 0)?;
    let atlas = &compiled.atlas;
    let s = atlas.sprites.get(id.0 as usize)?;
    if s.width == 0 || s.height == 0 || s.width > 256 || s.height > 256 {
        return None;
    }
    let rgba: Vec<u8> = if s.palette == 0 {
        atlas.pages.get(s.page as usize)?.crop(s.atlas_x, s.atlas_y, s.width, s.height).as_bytes().to_vec()
    } else {
        let page = atlas.index_pages.get(s.page as usize)?;
        let row = (s.palette * tile_render::PALETTE_SIZE) as usize;
        let mut out = Vec::with_capacity((s.width * s.height * 4) as usize);
        for y in s.atlas_y..s.atlas_y + s.height {
            for x in s.atlas_x..s.atlas_x + s.width {
                let i = page.get(x, y) as usize;
                out.extend_from_slice(&atlas.palettes.get(row + i).copied().unwrap_or(0).to_le_bytes());
            }
        }
        out
    };
    let clamp = |a: i32, len: u32| (-a).clamp(0, len as i32 - 1) as u16;
    Some(Image {
        rgba,
        size: (s.width as u16, s.height as u16),
        hotspot: (clamp(s.anchor_x, s.width), clamp(s.anchor_y, s.height)),
    })
}

/// What the system pointer shows.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    Hidden,
    Icon(winit::window::CursorIcon),
    /// A cursor made from this set's image ([`Art`]).
    Custom(u32),
}

/// The system pointer for the driver's `cursor`: hidden while the viewer
/// draws it itself (`hidden`), the capture pointer where a click captures
/// the mouse (`capture`), else the driver's.
pub fn system_pointer(cursor: scene::Cursor, art: &Art, hidden: bool, capture: bool) -> Shown {
    use winit::window::CursorIcon;
    if hidden {
        return Shown::Hidden;
    }
    if capture {
        return art.capture().map_or(Shown::Icon(CursorIcon::Crosshair), Shown::Custom);
    }
    match cursor {
        scene::Cursor::Sprite(set) if art.get(set).is_some() => Shown::Custom(set),
        scene::Cursor::System(s) => Shown::Icon(icon(s)),
        scene::Cursor::Default | scene::Cursor::Sprite(_) => Shown::Icon(CursorIcon::Default),
    }
}

/// The system's standard pointer.
pub fn icon(c: scene::SystemCursor) -> winit::window::CursorIcon {
    use scene::SystemCursor as S;
    use winit::window::CursorIcon as I;
    match c {
        S::Arrow => I::Default,
        S::Wait => I::Wait,
        S::Progress => I::Progress,
        S::Crosshair => I::Crosshair,
        S::Text => I::Text,
        S::Hand => I::Pointer,
        S::NotAllowed => I::NotAllowed,
        S::Move => I::Move,
        S::ResizeNs => I::NsResize,
        S::ResizeEw => I::EwResize,
        S::ResizeNwse => I::NwseResize,
        S::ResizeNesw => I::NeswResize,
    }
}

/// The set the viewer draws the driver's `cursor` with: its sprite, else
/// the first of [`FALLBACK_SPRITES`] the description has.
pub fn sprite_set(compiled: &Compiled, cursor: scene::Cursor) -> Option<SpriteSetId> {
    let valid = |raw: u32| compiled.sets.frame(SpriteSetId::from_raw(raw), 0).map(|_| SpriteSetId::from_raw(raw));
    match cursor {
        scene::Cursor::Sprite(set) => valid(set),
        _ => None,
    }
    .or_else(|| FALLBACK_SPRITES.iter().find_map(|n| compiled.set(n).map(|(s, _)| s)))
}

/// A pointer aid that, while it is on, is the only cursor (button snapping,
/// `crate::snap`, which draws its own pointer and ghost).
pub trait SnapMode {
    fn snap_mode_active(&self) -> bool;
}

/// (The snap mode is on, or its pointer lands after it: `SnapInput::active`.)
impl SnapMode for crate::snap::SnapInput {
    fn snap_mode_active(&self) -> bool {
        self.active()
    }
}

/// What decides how the cursor shows, as of a frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// The mouse is captured (its motion moves the world cursor).
    pub captured: bool,
    /// Button snapping is on ([`SnapMode`]): its pointer is the only cursor.
    pub snap_mode: bool,
    /// The system pointer waits for a frame to be let go of (or to be
    /// shown at the world cursor): it is still hidden.
    pub unlocking: bool,
    /// The layout's world cursor markers (world pixels as its markers).
    pub world_cursor: Vec<(i32, i32)>,
    /// The view following the world cursor: its rectangle, where the world
    /// cursor is in it, whether it is held at the world's edge
    /// (`Layout::world_cursor_view`).
    pub cursor_view: Option<([i32; 4], (i32, i32), bool)>,
    /// The system pointer (window pixels), whether it is in the window,
    /// and the world pixel under it (while free).
    pub pointer: (i32, i32),
    pub pointer_inside: bool,
    pub pointer_world: Option<(i32, i32)>,
    /// Seconds (any clock).
    pub now: f64,
}

/// How the cursor shows this frame, and what changes for the system
/// pointer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// The system pointer is hidden.
    pub hidden: bool,
    pub marks: Marks,
    /// The view following the world cursor is held at the world's edge:
    /// the system pointer is to show at the world cursor (this window
    /// pixel) instead of the drawn cursor, and to move it from now on.
    pub enter_edge: Option<(i32, i32)>,
    /// The view can centre the world cursor again: the system pointer is
    /// to be captured and hidden again, the cursor drawn.
    pub leave_edge: bool,
}

/// The cursor's states: free (the system pointer), captured (hidden, the
/// world cursor drawn in the middle of the view following it and wherever
/// else the world shows it), captured at the world's edge (the system
/// pointer at the world cursor, moving it, which then moves freely in its
/// view: drawn only in the other views), snapping (nothing of ours: only
/// the snapping pointer
/// drawn). At any moment the cursor shows once per place: where the
/// system pointer shows, nothing is drawn; the world cursor is drawn only
/// while the mouse is captured.
#[derive(Clone, Debug, Default)]
pub struct Machine {
    edge: bool,
    /// No edge mode before this time (the pointer left the view while in
    /// it).
    edge_after: f64,
}

/// How long the edge mode waits to start again after the pointer left the
/// view following the world cursor (seconds).
const EDGE_PAUSE: f64 = 0.3;

impl Machine {
    /// Whether the system pointer shows at the world cursor and moves it
    /// (the view following it held at the world's edge).
    pub fn edge(&self) -> bool {
        self.edge
    }

    /// Takes the facts of a frame: how the cursor shows.
    pub fn step(&mut self, f: &Facts) -> Plan {
        let mut plan = Plan::default();
        let active = f.captured && !f.snap_mode;
        match f.cursor_view.filter(|_| active) {
            Some((rect, at, held)) => {
                let inside = |(x, y): (i32, i32)| x >= rect[0] && y >= rect[1] && x < rect[0] + rect[2] && y < rect[1] + rect[3];
                if self.edge && !f.unlocking && (!held || !inside(f.pointer)) {
                    self.edge = false;
                    plan.leave_edge = true;
                    if !inside(f.pointer) {
                        self.edge_after = f.now + EDGE_PAUSE;
                    }
                } else if !self.edge && held && !f.unlocking && f.now >= self.edge_after {
                    self.edge = true;
                    plan.enter_edge = Some(at);
                }
            }
            None if self.edge => {
                self.edge = false;
                // (Still captured, snapping: hidden again.)
                plan.leave_edge = f.captured;
            }
            None => {}
        }
        // (Entering the edge mode the system pointer shows only once it is
        // at the world cursor, after this frame: the cursor is drawn till
        // then.)
        let entering = plan.enter_edge.is_some();
        plan.hidden = (f.captured && !self.edge) || f.snap_mode || f.unlocking || entering;
        plan.marks = if f.snap_mode {
            Marks::default()
        } else if f.captured {
            let at = f.cursor_view.map(|(_, at, _)| at);
            Marks { world: f.world_cursor.clone(), window: vec![], system: if self.edge && !f.unlocking && !entering { at } else { None } }
        } else {
            let world = f.pointer_world.filter(|_| f.pointer_inside).into_iter().collect();
            Marks { world, window: vec![], system: (!f.unlocking).then_some(f.pointer) }
        };
        plan
    }
}

/// Where the viewer draws the cursor this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marks {
    /// World pixels (in the pixels of views showing the world downscaled,
    /// as the layout's markers) to mark in every view showing them.
    pub world: Vec<(i32, i32)>,
    /// Window pixels to draw it at as they are (the snapping pointer).
    pub window: Vec<(i32, i32)>,
    /// Where the system draws its pointer (none while it is hidden): a
    /// view showing a marked point there is left to it.
    pub system: Option<(i32, i32)>,
}

/// The cursor sprites for `marks`, all in camera `overlay` (a camera over
/// the whole window at 1:1 drawn after `cameras`; see [`overlay_camera`]),
/// each clipped to the view it marks. `period`: how far the world repeats
/// (0: not), in the views' pixels.
pub fn sprites(set: SpriteSetId, marks: &Marks, cameras: &[Camera], overlay: u32, period: (i32, i32)) -> Vec<PlacedSprite> {
    let mut out = Vec::new();
    let place = |(x, y): (i32, i32), clip: Option<[i32; 4]>| PlacedSprite { screen: true, view: Some(overlay), clip, ..PlacedSprite::new(set, x, y) };
    let shifts = |p: i32| if p > 0 { vec![-p, 0, p] } else { vec![0] };
    for (k, c) in cameras.iter().enumerate() {
        if !c.cells || c.stacked {
            continue;
        }
        let rect = [c.origin.0, c.origin.1, c.origin.0 + c.viewport.0 as i32, c.origin.1 + c.viewport.1 as i32];
        let (n, d) = (c.zoom.0.max(1) as i32, c.zoom.1.max(1) as i32);
        // How far a world pixel spans on the screen (the system's pointer
        // is on the one under it).
        let span = (n + d - 1) / d;
        for &(wx, wy) in &marks.world {
            for sx in shifts(period.0) {
                for sy in shifts(period.1) {
                    let (px, py) = (wx + sx - c.x, wy + sy - c.y);
                    let q = (c.origin.0 + (px * n).div_euclid(d), c.origin.1 + (py * n).div_euclid(d));
                    if q.0 < rect[0] || q.1 < rect[1] || q.0 >= rect[2] || q.1 >= rect[3] {
                        continue;
                    }
                    // Covered by a later view showing the world there.
                    let covered = cameras[k + 1..].iter().any(|o| {
                        o.cells && !o.stacked && q.0 >= o.origin.0 && q.1 >= o.origin.1 && q.0 < o.origin.0 + o.viewport.0 as i32 && q.1 < o.origin.1 + o.viewport.1 as i32
                    });
                    let system = marks.system.is_some_and(|s| (s.0 - q.0).abs() <= span && (s.1 - q.1).abs() <= span);
                    if !covered && !system {
                        out.push(place(q, Some(rect)));
                    }
                }
            }
        }
    }
    out.extend(marks.window.iter().map(|&p| place(p, None)));
    out
}

/// The camera the cursor sprites are drawn in: the whole window at 1:1,
/// no world.
pub fn overlay_camera(window: (u32, u32)) -> Camera {
    Camera { cells: false, ..Camera::new(window) }
}

/// Latency from mouse motion to the cursor drawn by the viewer: from the
/// first motion event not yet drawn to the start of the frame showing it
/// (`start`: the event loop's part), to that frame handed to the system
/// (`present`: with the wait for a surface texture), and to the GPU having
/// finished it (`done`), in milliseconds.
#[derive(Debug, Default)]
pub struct Latency {
    pub start: Vec<f64>,
    pub present: Vec<f64>,
    /// When the last sample was taken.
    pub last: Option<Instant>,
    pub done: std::sync::Arc<std::sync::Mutex<Vec<f64>>>,
}

impl Latency {
    /// The percentiles 50, 95, 100 of `v`, and how many.
    pub fn summary(v: &mut [f64]) -> Option<(f64, f64, f64, usize)> {
        if v.is_empty() {
            return None;
        }
        v.sort_by(f64::total_cmp);
        let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        Some((at(0.5), at(0.95), v[v.len() - 1], v.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(origin: (i32, i32), viewport: (u32, u32), at: (i32, i32), zoom: (u32, u32)) -> Camera {
        Camera { x: at.0, y: at.1, origin, zoom, ..Camera::new(viewport) }
    }

    #[test]
    fn a_world_point_is_marked_in_every_view_but_where_the_system_draws_it() {
        let set = SpriteSetId::from_raw(3);
        // A at 1:1 showing world (0, 0)..; B at 2:1 showing (100, 100)...
        let a = cam((0, 0), (400, 300), (0, 0), (1, 1));
        let b = cam((400, 0), (400, 300), (100, 100), (2, 1));
        let interface = Camera { cells: false, ..Camera::new((800, 300)) };
        let cameras = [a, b, interface];
        // The pointer over A at world (150, 120): drawn in B only, clipped.
        let marks = Marks { world: vec![(150, 120)], window: vec![], system: Some((150, 120)) };
        let s = sprites(set, &marks, &cameras, 3, (0, 0));
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].x, s[0].y, s[0].view, s[0].screen), (400 + 100, 40, Some(3), true));
        assert_eq!(s[0].clip, Some([400, 0, 800, 300]));
        // Captured (no system pointer): in both.
        let s = sprites(set, &Marks { system: None, ..marks.clone() }, &cameras, 3, (0, 0));
        assert_eq!(s.len(), 2);
        // A world that wraps every 1000 pixels: (1150, 120) is the same.
        let s = sprites(set, &Marks { world: vec![(1150, 120)], window: vec![(5, 6)], system: None }, &cameras, 3, (1000, 0));
        assert_eq!(s.iter().map(|s| (s.x, s.y)).collect::<Vec<_>>(), [(150, 120), (500, 40), (5, 6)]);
        assert_eq!(s[2].clip, None);
    }

    #[test]
    fn a_view_on_top_hides_the_mark_below_it() {
        let set = SpriteSetId::from_raw(3);
        let a = cam((0, 0), (400, 300), (0, 0), (1, 1));
        let inset = cam((100, 100), (50, 50), (1000, 1000), (1, 1));
        let s = sprites(set, &Marks { world: vec![(120, 120)], ..Default::default() }, &[a, inset], 2, (0, 0));
        assert!(s.is_empty());
    }

    #[test]
    fn the_system_pointer_follows_the_driver_unless_hidden_or_capturing() {
        use winit::window::CursorIcon;
        let mut art = Art::default();
        art.images.insert(7, Image { rgba: vec![0; 4], size: (1, 1), hotspot: (0, 0) });
        art.capture = Some(7);
        assert_eq!(system_pointer(scene::Cursor::Sprite(7), &art, true, false), Shown::Hidden);
        assert_eq!(system_pointer(scene::Cursor::Sprite(7), &art, false, false), Shown::Custom(7));
        assert_eq!(system_pointer(scene::Cursor::Sprite(8), &art, false, false), Shown::Icon(CursorIcon::Default));
        assert_eq!(system_pointer(scene::Cursor::System(scene::SystemCursor::Wait), &art, false, false), Shown::Icon(CursorIcon::Wait));
        assert_eq!(system_pointer(scene::Cursor::Default, &art, false, true), Shown::Custom(7));
        assert_eq!(system_pointer(scene::Cursor::Default, &Art::default(), false, true), Shown::Icon(CursorIcon::Crosshair));
    }

    /// The views: A (left) at 1:1 showing world (0, 0)..; B (right) at 2:1
    /// following the world cursor, showing world `b_at`..; an interface on
    /// top.
    fn views(b_at: (i32, i32)) -> Vec<Camera> {
        let a = cam((0, 0), (400, 300), (0, 0), (1, 1));
        let b = cam((400, 0), (400, 300), b_at, (2, 1));
        vec![a, b, Camera { cells: false, ..Camera::new((800, 300)) }]
    }

    /// What must hold of every frame: the cursor shows once per place.
    fn check(f: &Facts, p: &Plan, cameras: &[Camera]) {
        let s = sprites(SpriteSetId::from_raw(1), &p.marks, cameras, cameras.len() as u32, (0, 0));
        // The system pointer shows exactly when it is not hidden, and then
        // nothing is drawn where it is.
        assert_eq!(p.hidden, p.marks.system.is_none(), "{f:?} {p:?}");
        if let Some((sx, sy)) = p.marks.system {
            assert!(s.iter().all(|s| (s.x - sx).abs() > 2 || (s.y - sy).abs() > 2), "drawn under the system pointer: {f:?} {s:?}");
        }
        // No two drawings at one place.
        for (i, a) in s.iter().enumerate() {
            assert!(s[i + 1..].iter().all(|b| (a.x, a.y) != (b.x, b.y)), "twice at {:?}", (a.x, a.y));
        }
        if f.snap_mode {
            // Snapping: only its own pointer, nothing of ours.
            assert!(p.hidden && s.is_empty() && p.marks == Marks::default());
        } else if f.captured {
            // The world cursor, and only it, is drawn while captured.
            assert_eq!(p.marks.world, f.world_cursor);
        } else {
            // Free: no world cursor markers, only the pointer's own spot.
            assert!(p.marks.world.iter().all(|w| Some(*w) == f.pointer_world), "{p:?}");
        }
    }

    fn free(pointer: (i32, i32), world: Option<(i32, i32)>) -> Facts {
        Facts { pointer, pointer_inside: true, pointer_world: world, ..Default::default() }
    }

    fn captured(cursor: (i32, i32), at: (i32, i32), held: bool, pointer: (i32, i32), now: f64) -> Facts {
        Facts { captured: true, world_cursor: vec![cursor], cursor_view: Some(([400, 0, 400, 300], at, held)), pointer, pointer_inside: true, now, ..Default::default() }
    }

    #[test]
    fn capture_edge_release_and_capture_again_show_one_cursor_a_place() {
        let mut m = Machine::default();
        let step = |m: &mut Machine, f: Facts, cameras: &[Camera]| {
            let p = m.step(&f);
            check(&f, &p, cameras);
            p
        };
        // Free over A at world (150, 120): the system pointer there, B
        // (following the pointer's world spot) draws it in its middle.
        let p = step(&mut m, free((150, 120), Some((150, 120))), &views((50, 45)));
        assert!(!p.hidden);
        assert_eq!(p.marks.system, Some((150, 120)));
        assert_eq!(sprites(SpriteSetId::from_raw(1), &p.marks, &views((50, 45)), 3, (0, 0)).len(), 1);
        // Captured, centred in B: hidden, drawn in A and B.
        let p = step(&mut m, captured((150, 120), (600, 150), false, (600, 150), 1.0), &views((50, 45)));
        assert!(p.hidden && p.enter_edge.is_none());
        // B held at the world's edge, the cursor off its middle: the system
        // pointer is to show at the cursor; this frame still draws it.
        let p = step(&mut m, captured((20, 120), (420, 150), true, (600, 150), 2.0), &views((10, 45)));
        assert_eq!(p.enter_edge, Some((420, 150)));
        assert!(p.hidden);
        assert!(m.edge());
        // Waiting for the frame: still drawn, still hidden.
        let mut f = captured((20, 120), (420, 150), true, (420, 150), 2.1);
        f.unlocking = true;
        let p = step(&mut m, f, &views((10, 45)));
        assert!(p.hidden && p.marks.system.is_none());
        // Shown: the system pointer at the cursor, not drawn in B there (A
        // still draws it where it shows that spot).
        let p = step(&mut m, captured((20, 120), (420, 150), true, (420, 150), 2.2), &views((10, 45)));
        assert!(!p.hidden && !p.leave_edge);
        assert_eq!(p.marks.system, Some((420, 150)));
        let drawn = sprites(SpriteSetId::from_raw(1), &p.marks, &views((10, 45)), 3, (0, 0));
        assert_eq!(drawn.iter().map(|s| (s.x, s.y)).collect::<Vec<_>>(), [(20, 120)]);
        // Back in the middle: captured and drawn again.
        let p = step(&mut m, captured((150, 120), (600, 150), false, (600, 150), 3.0), &views((50, 45)));
        assert!(p.leave_edge && p.hidden && !m.edge());
        // Let go: the pointer waits a frame (drawn where it goes), then the
        // system shows it; no world cursor is drawn any more.
        let mut f = free((600, 150), Some((150, 120)));
        f.unlocking = true;
        let p = step(&mut m, f, &views((50, 45)));
        assert!(p.hidden);
        let p = step(&mut m, free((600, 150), Some((150, 120))), &views((50, 45)));
        assert!(!p.hidden);
        let drawn = sprites(SpriteSetId::from_raw(1), &p.marks, &views((50, 45)), 3, (0, 0));
        assert_eq!(drawn.iter().map(|s| (s.x, s.y)).collect::<Vec<_>>(), [(150, 120)]);
        // Captured again.
        let p = step(&mut m, captured((150, 120), (600, 150), false, (600, 150), 4.0), &views((50, 45)));
        assert!(p.hidden);
    }

    #[test]
    fn snapping_and_the_edge_mode_take_turns() {
        let mut m = Machine::default();
        let cams = views((10, 45));
        // At the edge, shown.
        m.step(&captured((20, 120), (420, 150), true, (420, 150), 1.0));
        let p = m.step(&captured((20, 120), (420, 150), true, (420, 150), 1.1));
        check(&captured((20, 120), (420, 150), true, (420, 150), 1.1), &p, &cams);
        assert!(!p.hidden);
        // Snapping: hidden, nothing drawn, the edge mode over.
        let mut f = captured((20, 120), (420, 150), true, (420, 150), 1.2);
        f.snap_mode = true;
        let p = m.step(&f);
        check(&f, &p, &cams);
        assert!(p.leave_edge && p.hidden && !m.edge());
        // Snapping ends: the edge mode starts again at the world cursor.
        let f = captured((20, 120), (420, 150), true, (30, 280), 1.3);
        let p = m.step(&f);
        check(&f, &p, &cams);
        assert_eq!(p.enter_edge, Some((420, 150)));
    }

    #[test]
    fn a_pointer_leaving_the_held_view_is_captured_again_for_a_while() {
        let mut m = Machine::default();
        let cams = views((10, 45));
        m.step(&captured((20, 120), (420, 150), true, (420, 150), 1.0));
        m.step(&captured((20, 120), (420, 150), true, (420, 150), 1.1));
        // Out of B into A.
        let f = captured((0, 120), (400, 150), true, (390, 150), 1.2);
        let p = m.step(&f);
        check(&f, &p, &cams);
        assert!(p.leave_edge && p.hidden);
        // Not at once again (no flicker), but after the pause.
        let f = captured((0, 120), (400, 150), true, (390, 150), 1.3);
        assert_eq!(m.step(&f).enter_edge, None);
        let f = captured((0, 120), (400, 150), true, (390, 150), 1.6);
        assert_eq!(m.step(&f).enter_edge, Some((400, 150)));
    }

    #[test]
    fn latency_percentiles() {
        let mut v = vec![5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(Latency::summary(&mut v), Some((3.0, 5.0, 5.0, 5)));
        assert_eq!(Latency::summary(&mut []), None);
    }
}
