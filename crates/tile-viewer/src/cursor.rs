//! The mouse cursor: the system's pointer showing the driver's cursor art
//! where it can, and the viewer's own sprites of it where the system's
//! cannot show it.
//!
//! - While the mouse is free, the system draws the pointer (smoother than
//!   anything drawn here): the driver's cursor (`scene::Cursor`) as a
//!   custom system cursor made from its sprite, or a standard one; over a
//!   view that a click captures the mouse in, the description's
//!   [`CAPTURE_SPRITES`] (else the system's crosshair).
//! - While the mouse is captured or the pointer snaps to buttons, the
//!   system pointer is hidden and the viewer draws the cursor itself at its
//!   logical place: the world cursor in every view that shows it, the
//!   snapping pointer where it flies.
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

/// The first frame of `set` as a cursor image: its anchor is minus the hot
/// spot (a hot spot outside the image is moved onto its edge).
fn image(compiled: &Compiled, set: SpriteSetId) -> Option<Image> {
    let id = compiled.sets.frame(set, 0)?;
    let s = compiled.atlas.sprites.get(id.0 as usize)?;
    let page = compiled.atlas.pages.get(s.page as usize)?;
    if s.width == 0 || s.height == 0 || s.width > 256 || s.height > 256 {
        return None;
    }
    let cut = page.crop(s.atlas_x, s.atlas_y, s.width, s.height);
    let clamp = |a: i32, len: u32| (-a).clamp(0, len as i32 - 1) as u16;
    Some(Image {
        rgba: cut.as_bytes().to_vec(),
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

    #[test]
    fn latency_percentiles() {
        let mut v = vec![5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(Latency::summary(&mut v), Some((3.0, 5.0, 5.0, 5)));
        assert_eq!(Latency::summary(&mut []), None);
    }
}
