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
//!   pointer shows at the world cursor and moves it, kept inside its
//!   view along the axes held at the edge; once the view can centre the
//!   cursor again, the pointer is captured and hidden again ([`Machine`]: the system pointer is put
//!   where the drawn cursor was and shows only after the frame that
//!   applies that, so it never jumps).
//! - While the mouse is free, every other view showing the world pixel
//!   under the pointer (the other pane, insets, a zoom box) shows it too,
//!   drawn by the viewer; not where the system draws it.
//! - With edge scrolling only the zoom box mirrors the pointer (free or
//!   captured): in the other pane a copy reads as a second pointer
//!   ([`Marks::mirror_only_in`]).
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

/// RGBA rows of `size` with each pixel an `n`×`n` box (nearest pixel), and
/// their size.
pub fn upscaled(rgba: &[u8], size: (u16, u16), n: u32) -> (Vec<u8>, (u16, u16)) {
    let n = n.max(1) as usize;
    let (w, h) = (size.0 as usize, size.1 as usize);
    if n == 1 || rgba.len() < w * h * 4 {
        return (rgba.to_vec(), size);
    }
    let mut out = Vec::with_capacity(w * h * 4 * n * n);
    for row in rgba.chunks_exact(w * 4).take(h) {
        let wide: Vec<u8> = row.chunks_exact(4).flat_map(|p| std::iter::repeat_n(p, n).flatten().copied()).collect();
        for _ in 0..n {
            out.extend_from_slice(&wide);
        }
    }
    (out, ((w * n) as u16, (h * n) as u16))
}

/// The pixels of the sets [`KEPT_SETS`] names, by set (taken from a
/// description before its pixels are dropped).
#[derive(Clone, Debug, Default)]
pub struct Art {
    images: HashMap<u32, Image>,
    capture: Option<u32>,
    /// Sets whose first frame shows nothing (e.g. the game's go-to
    /// pointer, a blank cell of its cursor art): the arrow stands in.
    blank: std::collections::HashSet<u32>,
    arrow: Option<u32>,
    busy: Vec<u32>,
}

impl Art {
    /// The kept sets' first frames of `compiled` (which still has its
    /// pixels; without them, none).
    pub fn of(compiled: &Compiled) -> Self {
        let mut images = HashMap::new();
        let mut blank = std::collections::HashSet::new();
        for (_, set) in compiled.sets_with_prefix(KEPT_SETS) {
            match image(compiled, set) {
                Some(image) if image.rgba.chunks_exact(4).any(|p| p[3] != 0) => {
                    images.insert(set.raw(), image);
                }
                Some(_) => {
                    blank.insert(set.raw());
                }
                None if empty_frame(compiled, set) => {
                    blank.insert(set.raw());
                }
                None => {}
            }
        }
        let capture = compiled.sets_with_prefix(CAPTURE_SPRITES).map(|(_, s)| s.raw()).next();
        let arrow = compiled.set(crate::snap::ARROW_SPRITE).map(|(s, _)| s.raw()).filter(|s| images.contains_key(s));
        let busy: Vec<_> = busy_frames(compiled).into_iter().map(|image| {
            let mut id = u32::MAX;
            while images.contains_key(&id) { id -= 1; }
            images.insert(id, image);
            id
        }).collect();
        Self { images, capture, blank, arrow, busy }
    }

    /// The cursor to show for the driver's `cursor`: a sprite that shows
    /// nothing (the game hides its pointer that way, e.g. while it waits
    /// for a go-to target) becomes the arrow, so that the pointer never
    /// disappears.
    pub fn shown(&self, cursor: scene::Cursor) -> scene::Cursor {
        match cursor {
            scene::Cursor::Sprite(set) if self.blank.contains(&set) => self.arrow.map_or(scene::Cursor::Default, scene::Cursor::Sprite),
            c => c,
        }
    }

    /// The normal OS pointer uses our arrow art too.
    pub fn system_shown(&self, cursor: scene::Cursor) -> scene::Cursor {
        match self.shown(cursor) {
            scene::Cursor::Default | scene::Cursor::System(scene::SystemCursor::Arrow) =>
                self.arrow.map_or(cursor, scene::Cursor::Sprite),
            shown => shown,
        }
    }

    pub(crate) fn busy_frames(&self) -> usize { self.busy.len() }

    /// CPU animation uses elapsed host time, independently of presented frames.
    pub fn busy_cursor(&self, elapsed: std::time::Duration) -> Option<u32> {
        if self.busy.is_empty() { return self.arrow; }
        Some(self.busy[scene::loading_cursor::frame_at(elapsed) as usize % self.busy.len()])
    }

    pub fn get(&self, set: u32) -> Option<&Image> {
        self.images.get(&set)
    }

    pub fn badged(&self, set: u32, elapsed: std::time::Duration) -> Option<Shown> {
        if self.busy.is_empty() { return None; }
        let hourglass = self.busy[scene::loading_cursor::frame_at(elapsed) as usize % self.busy.len()];
        Some(Shown::Badged { base: set, hourglass })
    }

    /// The requested native cursor image; badges are composed on demand.
    pub fn cursor_image(&self, shown: Shown) -> Option<Image> {
        match shown {
            Shown::Custom(set) => self.get(set).cloned(),
            Shown::Badged { base, hourglass } => Some(with_busy_badge(self.get(base)?, self.get(hourglass)?)),
            _ => None,
        }
    }

    /// Only the ordinary arrow may give way to the full stuck hourglass.
    pub fn special(&self, cursor: scene::Cursor) -> bool {
        match self.shown(cursor) {
            scene::Cursor::Default | scene::Cursor::System(scene::SystemCursor::Arrow | scene::SystemCursor::Wait | scene::SystemCursor::Progress) => false,
            scene::Cursor::Sprite(set) => Some(set) != self.arrow,
            _ => true,
        }
    }

    /// The capture pointer's set, if the description has one.
    pub fn capture(&self) -> Option<u32> {
        self.capture.filter(|s| self.images.contains_key(s))
    }
}

/// Busy keeps the pointer's meaning; only a stuck ordinary arrow is replaced.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BusyEffect { None, Badge, Hourglass }

pub fn busy_effect(status: crate::host_status::Responsiveness, special: bool) -> BusyEffect {
    use crate::host_status::Responsiveness as R;
    match (status, special) {
        (R::Answering, _) => BusyEffect::None,
        (R::Stuck, false) => BusyEffect::Hourglass,
        _ => BusyEffect::Badge,
    }
}

pub const BUSY_BADGE_SIDE: u32 = 16;

/// Art pixels from the pointer's hot spot to the badge's top-left.
pub fn badge_offset(size: (u32, u32), hotspot: (u16, u16)) -> (i32, i32) {
    ((size.0 as i32 - hotspot.0 as i32 + 2).max(12), (size.1 as i32 - hotspot.1 as i32 + 2).max(12))
}

/// A single native cursor image, retaining the original hot spot and pixels.
fn with_busy_badge(base: &Image, hourglass: &Image) -> Image {
    let offset = badge_offset((base.size.0.into(), base.size.1.into()), base.hotspot);
    let (x, y) = (base.hotspot.0 as u32 + offset.0 as u32, base.hotspot.1 as u32 + offset.1 as u32);
    let (w, h) = ((base.size.0 as u32).max(x + BUSY_BADGE_SIDE), (base.size.1 as u32).max(y + BUSY_BADGE_SIDE));
    let mut rgba = vec![0; (w * h * 4) as usize];
    for by in 0..base.size.1 as usize {
        let start = by * base.size.0 as usize * 4;
        let dest = by * w as usize * 4;
        rgba[dest..dest + base.size.0 as usize * 4].copy_from_slice(&base.rgba[start..start + base.size.0 as usize * 4]);
    }
    for dy in 0..BUSY_BADGE_SIDE {
        for dx in 0..BUSY_BADGE_SIDE {
            let src = ((dy * hourglass.size.1 as u32 / BUSY_BADGE_SIDE * hourglass.size.0 as u32 + dx * hourglass.size.0 as u32 / BUSY_BADGE_SIDE) * 4) as usize;
            let dest = (((y + dy) * w + x + dx) * 4) as usize;
            rgba[dest..dest + 4].copy_from_slice(&hourglass.rgba[src..src + 4]);
        }
    }
    Image { rgba, size: (w as u16, h as u16), hotspot: base.hotspot }
}

/// The first frame of `set` as a cursor image (a colour sprite, or an
/// indexed one through its palette row): its anchor is minus the hot spot
/// (a hot spot outside the image is moved onto its edge).
fn image(compiled: &Compiled, set: SpriteSetId) -> Option<Image> {
    image_frame(compiled, set, 0)
}

fn image_frame(compiled: &Compiled, set: SpriteSetId, frame: u32) -> Option<Image> {
    let id = compiled.sets.frame(set, frame)?;
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
    Some(Image {
        rgba,
        size: (s.width as u16, s.height as u16),
        hotspot: hotspot((s.anchor_x, s.anchor_y), (s.width, s.height)),
    })
}

/// The busy pointer's pictures: the loading cursor's turned hourglass
/// frames (`scene::loading_cursor::SET`, pre-rendered rotations), each its
/// own system cursor with the hot spot in its middle. None where the
/// description lacks them or they show nothing (no system wait cursor to
/// read): the arrow then stands in.
fn busy_frames(compiled: &Compiled) -> Vec<Image> {
    let Some((set, _)) = compiled.set(scene::loading_cursor::SET) else { return Vec::new() };
    let frames: Vec<Image> = (0..scene::loading_cursor::FRAME_COUNT).map_while(|frame| image_frame(compiled, set, frame)).collect();
    if frames.iter().all(|f| f.rgba.chunks_exact(4).all(|p| p[3] == 0)) {
        return Vec::new();
    }
    frames
}

/// A sprite's hot spot: minus its anchor, moved onto its edge if outside.
fn hotspot(anchor: (i32, i32), size: (u32, u32)) -> (u16, u16) {
    let clamp = |a: i32, len: u32| (-a).clamp(0, len.max(1) as i32 - 1) as u16;
    (clamp(anchor.0, size.0), clamp(anchor.1, size.1))
}

/// Whether `set`'s first frame has no pixels at all.
fn empty_frame(compiled: &Compiled, set: SpriteSetId) -> bool {
    compiled.sets.frame(set, 0).and_then(|id| compiled.atlas.sprites.get(id.0 as usize)).is_some_and(|s| s.width == 0 || s.height == 0)
}

/// What the viewer needs of a cursor set to draw it at a size of its own:
/// the set, its first frame's size, hot spot (as [`Image::hotspot`]) and
/// anchor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SetArt {
    pub set: SpriteSetId,
    pub size: (u32, u32),
    pub hotspot: (u16, u16),
    pub anchor: (i32, i32),
}

impl SetArt {
    pub fn of(compiled: &Compiled, set: SpriteSetId) -> Option<SetArt> {
        let id = compiled.sets.frame(set, 0)?;
        let s = compiled.atlas.sprites.get(id.0 as usize)?;
        Some(SetArt { set, size: (s.width, s.height), hotspot: hotspot((s.anchor_x, s.anchor_y), (s.width, s.height)), anchor: (s.anchor_x, s.anchor_y) })
    }
}

/// Add the small hourglass beside a drawn pointer. Its own camera keeps
/// the badge at half the nominal cursor size, including fractional scales.
pub fn compose_busy_badge(out: &mut Composed, compiled: &Compiled, at: (i32, i32), offset: (i32, i32), clip: Option<[i32; 4]>, scale: Scale, canvas: (u32, u32), next: u32) {
    let Some(art) = compiled.set(scene::loading_cursor::SET).and_then(|(set, _)| SetArt::of(compiled, set)) else { return };
    if art.size.0 == 0 || art.size.1 == 0 { return; }
    let view = next + out.top.len() as u32;
    if view as usize >= tile_render::MAX_VIEWS { return; }
    let origin = (at.0 + scale.span(offset.0.max(0) as u32) as i32, at.1 + scale.span(offset.1.max(0) as u32) as i32);
    let side = scale.span(BUSY_BADGE_SIDE).max(1);
    out.top.push(Camera {
        cells: false, zoom: (scale.zoom.0 * BUSY_BADGE_SIDE, scale.zoom.1 * art.size.0),
        origin, screen_shift: origin, hairline: true, target_clip: clip,
        ..Camera::new((side.min(canvas.0), side.min(canvas.1)))
    });
    out.sprites.push(PlacedSprite {
        screen: true, view: Some(view), ticks_per_frame: scene::loading_cursor::TICKS_PER_FRAME,
        ..PlacedSprite::new(art.set, -art.anchor.0, -art.anchor.1)
    });
}

/// The art cell every cursor size is measured against: the game's cursor
/// cells are 32×32, so at a cursor size of 32 they show 1:1.
pub const NOMINAL: u32 = 32;

/// How big the cursor shows (`viewer_settings::Display::cursor_size`):
/// canvas pixels per art pixel (`zoom`, reduced) and window pixels per
/// canvas pixel (`present`, the pixel scale). The viewer draws its cursor
/// sprites at `zoom` on the canvas (nearest pixel, as every scaled view) and
/// the canvas shows `present`×`present` boxes; the system pointer made from
/// the same art ([`Scale::native`]) is that picture pixel for pixel. A size
/// is rounded to whole canvas pixels, so both are always the same size.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Scale {
    pub zoom: (u32, u32),
    pub present: u32,
}

impl Default for Scale {
    fn default() -> Self {
        Scale { zoom: (1, 1), present: 1 }
    }
}

impl Scale {
    /// [`NOMINAL`] art pixels `size` window pixels across, at pixel scale
    /// `present` (the nearest whole number of canvas pixels, at least one).
    pub fn new(size: u32, present: u32) -> Scale {
        let p = present.clamp(1, 64);
        let canvas = ((size.clamp(1, 1024) + p / 2) / p).max(1);
        let g = gcd(canvas, NOMINAL);
        Scale { zoom: (canvas / g, NOMINAL / g), present: p }
    }

    /// Whether the art shows 1:1 on the canvas.
    pub fn unscaled(&self) -> bool {
        self.zoom.0 == self.zoom.1
    }

    /// The window pixels [`NOMINAL`] art pixels span: the size in effect.
    pub fn size(&self) -> u32 {
        self.present * self.span(NOMINAL)
    }

    /// The canvas pixels `art` art pixels span, which is also the first
    /// canvas pixel that shows art pixel `art` (canvas pixel `c` shows art
    /// pixel ⌊c·zoom.1/zoom.0⌋, as the renderer scales a view).
    pub fn span(&self, art: u32) -> u32 {
        (art as u64 * self.zoom.0.max(1) as u64).div_ceil(self.zoom.1.max(1) as u64) as u32
    }

    fn art_of(&self, canvas: u32) -> u32 {
        (canvas as u64 * self.zoom.1.max(1) as u64 / self.zoom.0.max(1) as u64) as u32
    }

    /// The system pointer's hot spot (window pixels) for art hot spot `h`:
    /// in the middle of the canvas pixel showing it, as the window puts the
    /// pointer in the middle of a canvas pixel.
    pub fn native_hotspot(&self, h: (u16, u16), size: (u16, u16)) -> (u16, u16) {
        let axis = |h: u16, len: u16| (self.present * self.span(h as u32) + self.present / 2).min((self.present * self.span(len as u32)).max(1) - 1) as u16;
        (axis(h.0, size.0), axis(h.1, size.1))
    }

    /// `image` as the system pointer at this size (window pixels): what the
    /// viewer draws of it on the canvas, each canvas pixel a
    /// `present`×`present` box.
    pub fn native(&self, image: &Image) -> Image {
        let (w, h) = (image.size.0 as u32, image.size.1 as u32);
        if image.rgba.len() < (w * h * 4) as usize || w == 0 || h == 0 {
            return image.clone();
        }
        let p = self.present;
        let (nw, nh) = (p * self.span(w), p * self.span(h));
        let mut rgba = Vec::with_capacity((nw * nh * 4) as usize);
        for y in 0..nh {
            let ay = self.art_of(y / p).min(h - 1);
            for x in 0..nw {
                let ax = self.art_of(x / p).min(w - 1);
                let i = ((ay * w + ax) * 4) as usize;
                rgba.extend_from_slice(&image.rgba[i..i + 4]);
            }
        }
        Image { rgba, size: (nw as u16, nh as u16), hotspot: self.native_hotspot(image.hotspot, image.size) }
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

/// The centre of the map tile whose diamond holds world pixel `p` on a
/// staggered grid with half-tile `step` (Civ III: tile (x, y), x + y even,
/// its 128×64 box at (64·x, 32·y), its centre at (64·x + 64, 32·y + 32)),
/// or of the cell holding it on an orthogonal grid.
pub fn tile_centre(p: (i32, i32), step: (u32, u32), staggered: bool) -> (i32, i32) {
    let (sx, sy) = (step.0.max(1) as i64, step.1.max(1) as i64);
    let (x, y) = (p.0 as i64, p.1 as i64);
    if !staggered {
        return ((x.div_euclid(sx) * sx + sx / 2) as i32, (y.div_euclid(sy) * sy + sy / 2) as i32);
    }
    let (cx, cy) = ((x - sx).div_euclid(sx), (y - sy).div_euclid(sy));
    let mut best: Option<(i64, (i64, i64))> = None;
    for ty in cy - 1..=cy + 2 {
        for tx in cx - 1..=cx + 2 {
            if (tx + ty).rem_euclid(2) != 0 {
                continue;
            }
            let centre = (sx * tx + sx, sy * ty + sy);
            // Diamond distance from the centre (at most sx·sy inside).
            let d = (x - centre.0).abs() * sy + (y - centre.1).abs() * sx;
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, centre));
            }
        }
    }
    let (_, (x, y)) = best.expect("candidates");
    (x as i32, y as i32)
}

/// While the driver waits for a target tile (`scene::Interface::targeting`):
/// the cursor shown snapped to the centre of the tile it points at, so it
/// moves tile by tile, in the view the pointer is in and every other view
/// showing that tile. The system pointer still moves freely; it is hidden
/// meanwhile and the viewer draws the cursor.
pub fn snap_to_tiles(marks: Marks, f: &Facts, centre: impl Fn((i32, i32)) -> (i32, i32)) -> Marks {
    if f.snap_mode || (!f.captured && f.interface_open) {
        return marks;
    }
    if f.captured {
        let input = f.cursor_view.map(|(_, at, _)| at);
        return Marks { world: f.world_cursor.iter().copied().map(&centre).collect(), system: None, input, ..marks };
    }
    match f.pointer_world.filter(|_| f.pointer_inside && !f.unlocking) {
        Some(w) => Marks { world: vec![centre(w)], system: None, input: Some(f.pointer), ..marks },
        None => marks,
    }
}

/// Whether the cursor is shown snapped to tiles ([`snap_to_tiles`]) this
/// frame: only while the driver waits for a move's target tile
/// (`scene::Interface::snap_to_tiles`: go-to, a stack's move), the cursor
/// over the world; for every other target (bombard, rebase, airlift, air
/// drop, ...) the driver's cursor stays at the pointer.
pub fn snaps_to_tiles(interface: &scene::Interface, f: &Facts) -> bool {
    interface.targeting && interface.snap_to_tiles && !f.snap_mode && (f.captured || (f.pointer_inside && f.pointer_world.is_some() && !f.interface_open))
}

/// An empty camera: a place in the drawing order kept for the cursor
/// (`layout::Frame::cursor_slots`), drawing nothing until filled.
pub fn empty_camera() -> Camera {
    Camera { cells: false, target_clip: Some([0, 0, 0, 0]), ..Camera::new((1, 1)) }
}

/// What the system pointer shows.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Shown {
    Hidden,
    Icon(winit::window::CursorIcon),
    /// A cursor made from this set's image ([`Art`]).
    Custom(u32),
    /// The base pointer and a small hourglass in one native cursor image.
    Badged { base: u32, hourglass: u32 },
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
    /// Which camera axes are held at a world edge. Only those axes use
    /// the visible pointer; the others retain relative capture motion.
    pub cursor_axes: (bool, bool),
    /// The system pointer (window pixels), whether it is in the window,
    /// and the world pixel under it (while free).
    pub pointer: (i32, i32),
    pub pointer_inside: bool,
    pub pointer_world: Option<(i32, i32)>,
    /// The driver shows more than its plain map ([`interface_open`]: a
    /// title, setup or preview screen, a menu, a popup): the free pointer
    /// is the system's alone, mirrored nowhere.
    pub interface_open: bool,
    /// Seconds (any clock).
    pub now: f64,
}

/// Whether `scene` shows more than the driver's plain map: no world (the
/// title and setup screens), a named screen (an advisor, Preferences), the
/// Load Scenario screen with its preview beside it, a popup, menus, or
/// menus fitted to the window. The free pointer is then not mirrored in any
/// view ([`Facts::interface_open`]): what is under it is the interface, not
/// a place of a world other views show.
pub fn interface_open(scene: &scene::Scene) -> bool {
    let i = &scene.interface;
    scene.shape.cols == 0
        || scene.shape.rows == 0
        || i.screen.is_some()
        || i.beside.is_some()
        || i.popup.is_some()
        || i.menu_fit
        || scene.views.iter().any(|v| v.view.menu && !v.overlay.is_empty())
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
    /// Active view, or its one-pixel line when only one axis is unlocked.
    edge_rect: Option<[i32; 4]>,
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

    /// A deliberate capture/release supersedes any edge fallback before
    /// the next frame, including a release waiting to be presented.
    pub fn cancel_edge(&mut self) {
        self.edge = false;
        self.edge_rect = None;
        self.edge_after = 0.0;
    }

    /// Takes the facts of a frame: how the cursor shows.
    pub fn step(&mut self, f: &Facts) -> Plan {
        let mut plan = Plan::default();
        let active = f.captured && !f.snap_mode;
        match f.cursor_view.filter(|_| active) {
            Some((rect, at, held)) => {
                let mut edge_rect = rect;
                if !f.cursor_axes.0 { edge_rect[0] = at.0; edge_rect[2] = 1; }
                if !f.cursor_axes.1 { edge_rect[1] = at.1; edge_rect[3] = 1; }
                let inside = |(x, y): (i32, i32)| x >= rect[0] && y >= rect[1] && x < rect[0] + rect[2] && y < rect[1] + rect[3];
                if self.edge && !f.unlocking && (!held || !inside(f.pointer)) {
                    self.edge = false;
                    self.edge_rect = None;
                    plan.leave_edge = true;
                    if !inside(f.pointer) {
                        self.edge_after = f.now + EDGE_PAUSE;
                    }
                } else if held && !f.unlocking && ((!self.edge && f.now >= self.edge_after) || (self.edge && self.edge_rect != Some(edge_rect))) {
                    self.edge = true;
                    self.edge_rect = Some(edge_rect);
                    plan.enter_edge = Some(at);
                }
            }
            None if self.edge => {
                self.edge = false;
                self.edge_rect = None;
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
            let system = if self.edge && !f.unlocking && !entering { at } else { None };
            Marks { world: f.world_cursor.clone(), window: vec![], system, input: if system.is_none() { at } else { None }, mirror_only_in: None }
        } else {
            // (Mirrored in the other views only over the plain map.)
            let world = f.pointer_world.filter(|_| f.pointer_inside && !f.interface_open).into_iter().collect();
            Marks { world, window: vec![], system: (!f.unlocking).then_some(f.pointer), input: None, mirror_only_in: None }
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
    /// Where the pointer the input goes by is (window pixels) when the
    /// viewer draws it itself (none: the system pointer is that one, or
    /// none shows): the mark in the view there is the cursor, on top of
    /// everything; the marks in other views mirror it, drawn under the
    /// driver's interface (a click there would not reach that interface).
    pub input: Option<(i32, i32)>,
    /// Where marks that only mirror the pointer may show (window rectangles
    /// x, y, w, h; a mark shows in a view inside one of them): none, every
    /// view. With edge scrolling only the zoom box mirrors it, a copy in
    /// the other pane being a second pointer to the player.
    pub mirror_only_in: Option<Vec<[i32; 4]>>,
}

/// A cursor mark as drawn: where (window pixels, the hot spot), clipped to
/// which rectangle (left, top, right, bottom), in which camera's view (none:
/// a window mark), and whether it is the pointer itself rather than its
/// mirror in another view.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub at: (i32, i32),
    pub clip: Option<[i32; 4]>,
    pub view: Option<usize>,
    pub own: bool,
}

/// The cursor sprites for `marks`, all in camera `overlay` (a camera over
/// the whole window at 1:1 drawn after `cameras`; see [`overlay_camera`]),
/// each clipped to the view it marks. `period`: how far the world repeats
/// (0: not), in the views' pixels.
pub fn sprites(set: SpriteSetId, marks: &Marks, cameras: &[Camera], overlay: u32, period: (i32, i32)) -> Vec<PlacedSprite> {
    placements(marks, cameras, period).into_iter().map(|p| PlacedSprite { screen: true, view: Some(overlay), clip: p.clip, ..PlacedSprite::new(set, p.at.0, p.at.1) }).collect()
}

/// Where the cursor shows for `marks` (see [`sprites`]).
pub fn placements(marks: &Marks, cameras: &[Camera], period: (i32, i32)) -> Vec<Placed> {
    let mut out = Vec::new();
    let inside = |c: &Camera, q: (i32, i32)| c.cells && !c.stacked && q.0 >= c.origin.0 && q.1 >= c.origin.1 && q.0 < c.origin.0 + c.viewport.0 as i32 && q.1 < c.origin.1 + c.viewport.1 as i32;
    // The view the input goes to: the topmost showing the world there.
    let input_view = marks.input.and_then(|p| cameras.iter().rposition(|c| inside(c, p)));
    let shifts = |p: i32| if p > 0 { vec![-p, 0, p] } else { vec![0] };
    for (k, c) in cameras.iter().enumerate() {
        if !c.cells || c.stacked {
            continue;
        }
        let rect = [c.origin.0, c.origin.1, c.origin.0 + c.viewport.0 as i32, c.origin.1 + c.viewport.1 as i32];
        // (Whether a mark only mirroring the pointer may show in this view.)
        let mirrors_here = marks.mirror_only_in.as_ref().is_none_or(|only| {
            only.iter().any(|r| rect[0] >= r[0] && rect[1] >= r[1] && rect[2] <= r[0] + r[2] && rect[3] <= r[1] + r[3])
        });
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
                    let own = Some(k) == input_view;
                    if !covered && !system && (own || mirrors_here) {
                        out.push(Placed { at: q, clip: Some(rect), view: Some(k), own });
                    }
                }
            }
        }
    }
    out.extend(marks.window.iter().map(|&p| Placed { at: p, clip: None, view: None, own: true }));
    out
}

/// The cursor as drawn this frame: its sprites, the cameras filling the
/// frame's cursor slots (`slots`, cameras under the driver's interface:
/// marks mirroring the pointer in views drawn below them), and cameras to
/// add on top after camera `overlay` (a 1:1 camera over the canvas, the
/// next free index `overlay + 1`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Composed {
    pub sprites: Vec<PlacedSprite>,
    pub slots: Vec<(u32, Camera)>,
    pub top: Vec<Camera>,
}

/// The cursor `art` for `marks` at `scale` (see [`Composed`]): 1:1 in the
/// overlay (or a slot made an overlay), else each mark in a camera of its
/// own scaled to the cursor's size, its hot spot on the marked canvas
/// pixel. Marks in views drawn below the slots that are not the pointer
/// itself go into the slots; the rest on top (as long as the renderer
/// draws that many views; beyond that they are left out).
pub fn compose(art: SetArt, marks: &Marks, cameras: &[Camera], slots: &[u32], scale: Scale, canvas: (u32, u32), overlay: u32, period: (i32, i32)) -> Composed {
    let mut out = Composed::default();
    let below = |p: &Placed| !p.own && slots.first().is_some_and(|&first| p.view.is_some_and(|v| (v as u32) < first));
    let (mut free_slots, mut next) = (slots.iter().copied(), overlay + 1);
    let mut overlay_slot: Option<u32> = None;
    for p in placements(marks, cameras, period) {
        let under = below(&p);
        if scale.unscaled() {
            let view = if under {
                match overlay_slot.or_else(|| free_slots.next()) {
                    Some(slot) => {
                        if overlay_slot.replace(slot).is_none() {
                            out.slots.push((slot, overlay_camera(canvas)));
                        }
                        slot
                    }
                    None => overlay,
                }
            } else {
                overlay
            };
            out.sprites.push(PlacedSprite { screen: true, view: Some(view), clip: p.clip, ..PlacedSprite::new(art.set, p.at.0, p.at.1) });
            continue;
        }
        let origin = (p.at.0 - scale.span(art.hotspot.0 as u32) as i32, p.at.1 - scale.span(art.hotspot.1 as u32) as i32);
        let viewport = (scale.span(art.size.0).max(1), scale.span(art.size.1).max(1));
        let camera = Camera { cells: false, zoom: scale.zoom, origin, screen_shift: origin, hairline: true, target_clip: p.clip, ..Camera::new(viewport) };
        let view = match under.then(|| free_slots.next()).flatten() {
            Some(slot) => {
                out.slots.push((slot, camera));
                slot
            }
            None if (next as usize) < tile_render::MAX_VIEWS => {
                out.top.push(camera);
                next += 1;
                next - 1
            }
            None => continue,
        };
        out.sprites.push(PlacedSprite { screen: true, view: Some(view), ..PlacedSprite::new(art.set, -art.anchor.0, -art.anchor.1) });
    }
    out
}

/// World cursors retain their command/capture meaning even if the host is
/// stuck. Only the actual pointer gets a badge, not its mirrored marks.
pub fn compose_with_busy(compiled: &Compiled, art: SetArt, marks: &Marks, cameras: &[Camera], slots: &[u32], scale: Scale, canvas: (u32, u32), overlay: u32, period: (i32, i32), status: crate::host_status::Responsiveness) -> Composed {
    let mut out = compose(art, marks, cameras, slots, scale, canvas, overlay, period);
    if busy_effect(status, true) == BusyEffect::Badge {
        for p in placements(marks, cameras, period).into_iter().filter(|p| p.own) {
            compose_busy_badge(&mut out, compiled, p.at, badge_offset(art.size, art.hotspot), p.clip, scale, canvas, overlay + 1);
        }
    }
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
#[derive(Default)]
pub struct Latency {
    pub pending: std::collections::BTreeMap<u64, Instant>,
    pub start: Vec<f64>,
    pub present: Vec<f64>,
    /// When the last sample was taken.
    pub last: Option<Instant>,
    /// Sent by the GPU's completion callbacks, whichever thread runs them.
    pub done: std::sync::Arc<civ3r_sync::Mailbox<f64>>,
}

impl std::fmt::Debug for Latency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Latency").field("start", &self.start).field("present", &self.present).field("last", &self.last).field("done", &self.done.len()).finish()
    }
}

impl Latency {
    pub fn begin(&mut self, id: u64, since: Instant, started: Instant) {
        self.start.push(started.saturating_duration_since(since).as_secs_f64() * 1000.0);
        self.pending.insert(id, since);
        while self.pending.len() > 64 { self.pending.pop_first(); }
    }
    pub fn completed(&self, id: u64, at: Instant) {
        if let Some(since) = self.pending.get(&id) {
            self.done.send(at.saturating_duration_since(*since).as_secs_f64() * 1000.0);
        }
    }
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

    #[test]
    fn latency_tokens_keep_out_of_order_completions_and_bound_stale_frames() {
        let mut latency = Latency::default();
        let now = Instant::now();
        for id in 0..66 { latency.begin(id, now + std::time::Duration::from_millis(id), now + std::time::Duration::from_millis(id + 1)); }
        assert_eq!(latency.pending.len(), 64);
        latency.completed(0, now + std::time::Duration::from_millis(100));
        assert!(latency.done.try_recv().is_none());
        latency.completed(65, now + std::time::Duration::from_millis(70));
        latency.completed(64, now + std::time::Duration::from_millis(72));
        assert_eq!(latency.done.try_recv(), Some(5.0));
        assert_eq!(latency.done.try_recv(), Some(8.0));
        // GPU completion can precede bitmap presentation. Keep its input time.
        assert!(latency.pending.contains_key(&65));
    }

    #[test]
    fn a_cursor_upscaled_is_whole_boxes() {
        let px = |v: u8| [v, v, v, 255];
        let rgba: Vec<u8> = [px(1), px(2), px(3), px(4)].concat();
        let (out, size) = upscaled(&rgba, (2, 2), 3);
        assert_eq!(size, (6, 6));
        let at = |x: usize, y: usize| out[(y * 6 + x) * 4];
        assert_eq!((at(0, 0), at(2, 2), at(3, 0), at(5, 2), at(0, 3), at(5, 5)), (1, 1, 2, 2, 3, 4));
        assert_eq!(upscaled(&rgba, (2, 2), 1), (rgba.clone(), (2, 2)));
    }

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
        let marks = Marks { world: vec![(150, 120)], window: vec![], system: Some((150, 120)), input: None, mirror_only_in: None };
        let s = sprites(set, &marks, &cameras, 3, (0, 0));
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].x, s[0].y, s[0].view, s[0].screen), (400 + 100, 40, Some(3), true));
        assert_eq!(s[0].clip, Some([400, 0, 800, 300]));
        // Captured (no system pointer): in both.
        let s = sprites(set, &Marks { system: None, ..marks.clone() }, &cameras, 3, (0, 0));
        assert_eq!(s.len(), 2);
        // A world that wraps every 1000 pixels: (1150, 120) is the same.
        let s = sprites(set, &Marks { world: vec![(1150, 120)], window: vec![(5, 6)], system: None, input: None, mirror_only_in: None }, &cameras, 3, (1000, 0));
        assert_eq!(s.iter().map(|s| (s.x, s.y)).collect::<Vec<_>>(), [(150, 120), (500, 40), (5, 6)]);
        assert_eq!(s[2].clip, None);
    }

    #[test]
    fn with_edge_scrolling_only_the_zoom_box_mirrors_the_pointer() {
        let set = SpriteSetId::from_raw(3);
        let a = cam((0, 0), (400, 300), (0, 0), (1, 1));
        let b = cam((400, 0), (400, 300), (100, 80), (1, 1));
        let zoom = cam((500, 200), (100, 60), (130, 110), (2, 1));
        let cameras = [a, b, zoom];
        let only = Some(vec![[500, 200, 100, 60]]);
        // Free over A at world (150, 120): not in B, still in the zoom box.
        let free = Marks { world: vec![(150, 120)], window: vec![], system: Some((150, 120)), input: None, mirror_only_in: only.clone() };
        let s = sprites(set, &free, &cameras, 3, (0, 0));
        assert_eq!(s.iter().map(|s| s.clip).collect::<Vec<_>>(), [Some([500, 200, 600, 260])]);
        // Captured in A: drawn there (the pointer itself) and in the zoom box.
        let captured = Marks { system: None, input: Some((150, 120)), ..free.clone() };
        let s = sprites(set, &captured, &cameras, 3, (0, 0));
        assert_eq!(s.iter().map(|s| s.clip).collect::<Vec<_>>(), [Some([0, 0, 400, 300]), Some([500, 200, 600, 260])]);
        // No restriction: mirrored in B as well.
        let s = sprites(set, &Marks { mirror_only_in: None, ..captured }, &cameras, 3, (0, 0));
        assert_eq!(s.len(), 3);
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
    fn normal_system_pointer_uses_our_arrow_and_keeps_busy_and_invalid_icons() {
        let mut art = Art::default();
        art.arrow = Some(7);
        assert_eq!(art.system_shown(scene::Cursor::Default), scene::Cursor::Sprite(7));
        assert_eq!(art.system_shown(scene::Cursor::System(scene::SystemCursor::Arrow)), scene::Cursor::Sprite(7));
        for icon in [scene::SystemCursor::Wait, scene::SystemCursor::Progress, scene::SystemCursor::NotAllowed] {
            assert_eq!(art.system_shown(scene::Cursor::System(icon)), scene::Cursor::System(icon));
        }
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
        Facts { captured: true, world_cursor: vec![cursor], cursor_view: Some(([400, 0, 400, 300], at, held)), cursor_axes: (held, held), pointer, pointer_inside: true, now, ..Default::default() }
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

    #[test]
    fn edge_axis_changes_reposition_before_showing_the_pointer_and_recapture_when_centred() {
        let mut m = Machine::default();
        let mut f = captured((20, 120), (420, 150), true, (420, 150), 1.0);
        f.cursor_axes = (true, false);
        assert_eq!(m.step(&f).enter_edge, Some((420, 150)));
        assert_eq!(m.edge_rect, Some([400, 150, 400, 1]));
        assert_eq!(m.step(&f).enter_edge, None);
        // Reaching a corner unlocks the second dimension. Hint the new
        // drawn cursor position while hidden before releasing into it.
        f.cursor_axes = (true, true);
        f.cursor_view = Some(([400, 0, 400, 300], (420, 20), true));
        let corner = m.step(&f);
        assert!(corner.hidden && m.edge());
        assert_eq!(corner.enter_edge, Some((420, 20)));
        assert_eq!(m.edge_rect, Some([400, 0, 400, 300]));
        f.unlocking = true;
        assert!(m.step(&f).hidden);
        f.unlocking = false;
        f.pointer = (420, 20);
        assert!(!m.step(&f).hidden);
        // Leaving that corner becomes a vertical line, not a full-window
        // winit grab, and changing the view updates that line too.
        f.cursor_axes = (false, true);
        assert_eq!(m.step(&f).enter_edge, Some((420, 20)));
        assert_eq!(m.edge_rect, Some([420, 0, 1, 300]));
        f.cursor_view = Some(([300, 0, 500, 300], (550, 20), true));
        assert_eq!(m.step(&f).enter_edge, Some((550, 20)));
        f.cursor_axes = (false, false);
        f.cursor_view = Some(([300, 0, 500, 300], (550, 150), false));
        let centred = m.step(&f);
        assert!(centred.leave_edge && centred.hidden && !m.edge());
        assert_eq!(m.edge_rect, None);
    }

    struct NoFiles;
    impl render_desc::ImageLoader for NoFiles {
        fn load(&self, source: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
            Err(format!("no file {source}"))
        }
    }

    /// The game's arrow and a blank cell (as its go-to pointer is), both
    /// kept cursor sets.
    fn arrow_and_blank() -> Compiled {
        render_desc::Description::parse(
            r#"
            grid = { layout = "orthogonal", step = [8, 8] }
            [images.arrow]
            source = "pointer:255,255,255,255:14x22"
            [images.blank]
            source = "solid:0,0,0,0:32x32"
            [images.target]
            source = "solid:200,40,40,255:32x32"
            [sheets.arrow]
            image = "arrow"
            cell = [14, 22]
            group = "Viewer@cursors"
            [sheets.blank]
            image = "blank"
            cell = [32, 32]
            group = "Viewer@blank"
            [sheets.target]
            image = "target"
            cell = [32, 32]
            anchor = [-16, -16]
            group = "Viewer@bombard"
            "#,
        )
        .unwrap()
        .compile(&NoFiles, 256)
        .unwrap()
    }

    #[test]
    fn a_targeting_pointer_that_shows_nothing_becomes_the_arrow_everywhere() {
        let compiled = arrow_and_blank();
        let art = Art::of(&compiled);
        let raw = |name: &str| compiled.set(name).map(|(s, _)| s.raw());
        let (arrow, blank, target) = (raw("Viewer@cursors/0"), raw("Viewer@blank/0"), raw("Viewer@bombard/0"));
        let (arrow, target) = (arrow.unwrap(), target.unwrap());
        // A blank cell, or one the description dropped: the arrow, as the
        // system pointer and as the sprite the viewer draws.
        if let Some(blank) = blank {
            assert_eq!(art.shown(scene::Cursor::Sprite(blank)), scene::Cursor::Sprite(arrow));
            assert_eq!(system_pointer(art.shown(scene::Cursor::Sprite(blank)), &art, false, false), Shown::Custom(arrow));
            assert_eq!(sprite_set(&compiled, art.shown(scene::Cursor::Sprite(blank))), Some(SpriteSetId::from_raw(arrow)));
        }
        // The game's own targeting cursors show as they are.
        assert_eq!(art.shown(scene::Cursor::Sprite(target)), scene::Cursor::Sprite(target));
        assert_eq!(system_pointer(art.shown(scene::Cursor::Sprite(target)), &art, false, false), Shown::Custom(target));
        assert_eq!(sprite_set(&compiled, scene::Cursor::Sprite(target)), Some(SpriteSetId::from_raw(target)));
        assert_eq!(SetArt::of(&compiled, SpriteSetId::from_raw(target)).map(|a| a.hotspot), Some((16, 16)));
        // Never hidden while the viewer does not draw it.
        assert_eq!(art.shown(scene::Cursor::System(scene::SystemCursor::Wait)), scene::Cursor::System(scene::SystemCursor::Wait));
        assert_ne!(system_pointer(art.shown(scene::Cursor::Sprite(target)), &art, false, false), Shown::Hidden);
    }

    #[test]
    fn cursor_sizes_round_to_whole_canvas_pixels_and_the_system_pointer_matches() {
        for present in 1..=4 {
            for size in [16, 24, 32, 36, 40, 48, 64, 72, 96, 128, 192] {
                let s = Scale::new(size, present);
                // The size in effect: the nearest multiple of the pixel scale.
                assert_eq!(s.size() % present, 0, "{size} {present}");
                assert!((s.size() as i64 - size as i64).abs() <= present as i64 / 2 + (size < present) as i64 * present as i64, "{size} {present}: {}", s.size());
                let image = Image { rgba: (0..32 * 32).flat_map(|i| [i as u8, (i >> 8) as u8, 7, 255]).collect(), size: (32, 32), hotspot: (5, 9) };
                let native = s.native(&image);
                assert_eq!((native.size.0 as u32, native.size.1 as u32), (s.size(), s.size()), "{size} {present}");
                // Each canvas pixel a present×present box of one art pixel.
                for y in 0..native.size.1 as usize {
                    for x in 0..native.size.0 as usize {
                        let (bx, by) = (x / present as usize * present as usize, y / present as usize * present as usize);
                        assert_eq!(native.rgba[(y * native.size.0 as usize + x) * 4..][..4], native.rgba[(by * native.size.0 as usize + bx) * 4..][..4]);
                    }
                }
                // The hot spot in the middle of the canvas pixel showing it.
                let h = native.hotspot;
                assert_eq!((h.0 as u32 / present, h.1 as u32 / present), (s.span(5), s.span(9)));
                let at = |x: u32, y: u32| native.rgba[((y * s.size() + x) * 4) as usize];
                if s.zoom.0 >= s.zoom.1 {
                    assert_eq!(at(h.0 as u32, h.1 as u32), image.rgba[((9 * 32 + 5) * 4) as usize], "{size} {present}");
                }
            }
        }
        // At 32 and the pixel scale 1: the art as it is.
        let s = Scale::new(32, 1);
        assert!(s.unscaled() && s == Scale::default());
        assert_eq!(Scale::new(64, 2), Scale { zoom: (1, 1), present: 2 });
        assert_eq!(Scale::new(48, 1).zoom, (3, 2));
        assert_eq!(Scale::new(24, 1).zoom, (3, 4));
    }

    /// The drawn cursor at every size is, pixel for pixel, the system
    /// pointer made from the same art: both the same size, the same hot
    /// spot, the same pixels (the canvas's at the pixel scale).
    #[test]
    fn the_drawn_cursor_is_the_system_pointer_pixel_for_pixel() {
        use tile_render::{AtlasBuilder, Grid, RgbaImage, SpriteSets, TileWorld};
        let pixel = |x: u32, y: u32| if x == 3 { [0, 0, 0, 0] } else { [(x * 8) as u8, (y * 8) as u8, 200, 255] };
        let picture = RgbaImage::from_fn(32, 32, pixel);
        let hot = (5u16, 9u16);
        let mut builder = AtlasBuilder::new(64);
        let id = builder.add(&picture, (-(hot.0 as i32), -(hot.1 as i32))).unwrap();
        let atlas = builder.finish();
        let mut sets = SpriteSets::default();
        let set = sets.single(id);
        let mut world = TileWorld::new(Grid { cols: 4, rows: 4, layout: tile_render::Layout::Orthogonal, wrap_x: false, wrap_y: false, step: (8, 8) }, sets);
        let art = SetArt { set, size: (32, 32), hotspot: hot, anchor: (-(hot.0 as i32), -(hot.1 as i32)) };
        let image = Image { rgba: (0..32).flat_map(|y| (0..32).flat_map(move |x| pixel(x, y))).collect(), size: (32, 32), hotspot: hot };
        let canvas = (240, 200);
        for present in [1, 2, 3] {
            for size in [24, 32, 48, 64, 80, 96] {
                let scale = Scale::new(size, present);
                let q = (97, 61);
                let marks = Marks { window: vec![q], ..Default::default() };
                let base = vec![overlay_camera(canvas)];
                let composed = compose(art, &marks, &[], &[], scale, canvas, 0, (0, 0));
                let mut cameras = base.clone();
                cameras.extend(composed.top.iter().copied());
                world.free = composed.sprites.clone();
                let picture = tile_render::reference::render_views(&world, &atlas, &cameras, canvas, 0, [1, 2, 3, 255]);
                let native = scale.native(&image);
                let origin = (q.0 - (native.hotspot.0 as u32 / present) as i32, q.1 - (native.hotspot.1 as u32 / present) as i32);
                let span = scale.span(32);
                for cy in 0..span {
                    for cx in 0..span {
                        let n = &native.rgba[(((cy * present) * native.size.0 as u32 + cx * present) * 4) as usize..][..4];
                        let drawn = picture.get(origin.0 as u32 + cx, origin.1 as u32 + cy);
                        let expected = if n[3] == 0 { [1, 2, 3, 255] } else { [n[0], n[1], n[2], n[3]] };
                        assert_eq!(drawn, expected, "size {size} present {present} at {cx},{cy}");
                    }
                }
                // Nothing beside it.
                for (x, y) in [(origin.0 - 1, origin.1), (origin.0, origin.1 - 1), (origin.0 + span as i32, origin.1), (origin.0, origin.1 + span as i32)] {
                    assert_eq!(picture.get(x as u32, y as u32), [1, 2, 3, 255], "size {size} present {present}");
                }
            }
        }
    }

    #[test]
    fn mirrored_cursors_go_under_the_interface_and_the_pointer_on_top() {
        let set = SpriteSetId::from_raw(3);
        let art = SetArt { set, size: (32, 32), hotspot: (1, 1), anchor: (-1, -1) };
        // A (left) and B (right) show the world; slot 2 is kept under the
        // interface (3); the overlay on top is 4.
        let a = cam((0, 0), (400, 300), (0, 0), (1, 1));
        let b = cam((400, 0), (400, 300), (0, 0), (1, 1));
        let interface = Camera { cells: false, ..Camera::new((800, 300)) };
        let cameras = [a, b, empty_camera(), interface];
        // The mouse captured, the world cursor drawn in A (where the input
        // goes) and mirrored in B.
        let marks = Marks { world: vec![(100, 50)], window: vec![], system: None, input: Some((100, 50)), mirror_only_in: None };
        for scale in [Scale::default(), Scale::new(48, 1)] {
            let c = compose(art, &marks, &cameras, &[2], scale, (800, 300), 4, (0, 0));
            assert_eq!(c.sprites.len(), 2, "{scale:?}");
            let views: Vec<u32> = c.sprites.iter().map(|s| s.view.unwrap()).collect();
            // The mirror in B: in the slot, before the interface.
            assert!(views.contains(&2), "{views:?}");
            assert_eq!(c.slots.len(), 1);
            assert_eq!(c.slots[0].0, 2);
            assert!(scale.unscaled() || c.slots[0].1.target_clip.is_some_and(|r| r[0] >= 400), "{:?}", c.slots);
            // The pointer itself in A: on top of everything.
            let top = views.iter().copied().find(|&v| v != 2).unwrap();
            assert!(top >= 4 && top > 3, "{views:?}");
            assert_eq!(c.top.len(), if scale.unscaled() { 0 } else { 1 });
        }
        // The pointer free (the system's): every drawn mark is a mirror.
        let free = Marks { world: vec![(100, 50)], window: vec![], system: Some((100, 50)), input: None, mirror_only_in: None };
        let c = compose(art, &free, &cameras, &[2], Scale::default(), (800, 300), 4, (0, 0));
        assert_eq!(c.sprites.iter().map(|s| s.view).collect::<Vec<_>>(), [Some(2)]);
        // No slot (e.g. the world beside a screen): on top as before.
        let c = compose(art, &free, &cameras, &[], Scale::default(), (800, 300), 4, (0, 0));
        assert_eq!(c.sprites.iter().map(|s| s.view).collect::<Vec<_>>(), [Some(4)]);
    }

    #[test]
    fn tile_centres_are_the_games_diamonds_at_every_world_zoom() {
        // Civ III: tile (x, y), x + y even, its centre at (64x + 64, 32y + 32).
        let game = |(wx, wy): (i32, i32)| -> (i32, i32) {
            let mut best = (i64::MAX, (0, 0));
            for y in -2..40 {
                for x in -2..40 {
                    if (x + y) % 2 != 0 { continue; }
                    let c = (64 * x + 64, 32 * y + 32);
                    let d = ((wx - c.0).abs() * 32 + (wy - c.1).abs() * 64) as i64;
                    if d < best.0 { best = (d, c); }
                }
            }
            best.1
        };
        for p in [(64, 32), (0, 0), (100, 40), (127, 63), (128, 64), (191, 47), (300, 211), (513, 777), (63, 31), (65, 33)] {
            assert_eq!(tile_centre(p, (64, 32), true), game(p), "{p:?}");
        }
        // The centre of a tile is its own; points inside its diamond go to it.
        for (x, y) in [(0, 0), (1, 1), (4, 2), (7, 9)] {
            let c = (64 * x + 64, 32 * y + 32);
            for d in [(0, 0), (60, 0), (-60, 0), (0, 28), (0, -28), (30, 14), (-30, -14)] {
                assert_eq!(tile_centre((c.0 + d.0, c.1 + d.1), (64, 32), true), c, "{c:?} {d:?}");
            }
        }
        // Zoomed out (Z): half the steps, half the centres.
        for p in [(32, 16), (50, 20), (150, 105)] {
            let full = game((2 * p.0, 2 * p.1));
            assert_eq!(tile_centre(p, (32, 16), true), (full.0 / 2, full.1 / 2), "{p:?}");
        }
        // An orthogonal grid: the cell's middle.
        assert_eq!(tile_centre((17, 3), (8, 8), false), (20, 4));
        assert_eq!(tile_centre((-1, -9), (8, 8), false), (-4, -12));
    }

    #[test]
    fn while_targeting_the_drawn_cursor_moves_tile_by_tile() {
        let centre = |p: (i32, i32)| tile_centre(p, (64, 32), true);
        // The mouse free over the world: drawn (the system pointer hidden)
        // at the middle of the tile, in the pointer's view and mirrored.
        for (pointer, world) in [((150, 120), (150, 120)), ((151, 121), (151, 121)), ((200, 90), (200, 90))] {
            let f = free(pointer, Some(world));
            let p = Machine::default().step(&f);
            let m = snap_to_tiles(p.marks, &f, centre);
            assert_eq!(m.world, vec![centre(world)]);
            assert_eq!((m.system, m.input), (None, Some(pointer)));
        }
        // Over invalid tiles too: the driver's cursor (its "cannot" pointer)
        // is what shows there, at the tile's middle.
        let f = free((10, 10), Some((60, 30)));
        assert_eq!(snap_to_tiles(Machine::default().step(&f).marks, &f, centre).world, vec![(64, 32)]);
        // Captured: the world cursor snapped in every view.
        let f = captured((150, 120), (600, 150), false, (600, 150), 1.0);
        let m = snap_to_tiles(Machine::default().step(&f).marks, &f, centre);
        assert_eq!(m.world, vec![centre((150, 120))]);
        assert_eq!((m.system, m.input), (None, Some((600, 150))));
        // Off the world: as it was.
        let f = free((10, 10), None);
        let p = Machine::default().step(&f);
        assert_eq!(snap_to_tiles(p.marks.clone(), &f, centre), p.marks);
    }

    #[test]
    fn only_a_moves_target_snaps_the_cursor_to_tiles() {
        // Go-to and the stack moves: snapped, free or captured.
        let go_to = scene::Interface { targeting: true, snap_to_tiles: true, ..Default::default() };
        // Rebase, airlift, bombard, air drop, ...: the cursor at the pointer.
        let other = scene::Interface { targeting: true, ..Default::default() };
        for f in [free((150, 120), Some((150, 120))), captured((150, 120), (600, 150), false, (600, 150), 1.0)] {
            assert!(snaps_to_tiles(&go_to, &f));
            assert!(!snaps_to_tiles(&other, &f));
            // No command waiting (a stale flag alone does nothing).
            assert!(!snaps_to_tiles(&scene::Interface { snap_to_tiles: true, ..Default::default() }, &f));
        }
        // Off the world, or button snapping on: never.
        assert!(!snaps_to_tiles(&go_to, &free((10, 10), None)));
        assert!(!snaps_to_tiles(&go_to, &Facts { snap_mode: true, ..free((150, 120), Some((150, 120))) }));
    }

    #[test]
    fn intentional_release_cancels_pending_edge_state_before_the_next_frame() {
        let mut m = Machine::default();
        let f = captured((20, 120), (420, 150), true, (420, 150), 1.0);
        assert!(m.step(&f).enter_edge.is_some());
        m.cancel_edge();
        assert!(!m.edge());
        assert_eq!(m.edge_rect, None);
        let p = m.step(&free((250, 100), Some((250, 100))));
        assert!(!p.hidden && !p.leave_edge);
        assert_eq!(p.marks.system, Some((250, 100)));
    }

}

#[cfg(test)]
mod busy_tests {
    use super::*;

    /// The game's arrow and the real loading-cursor description, its wait
    /// cursor's turned frames made here: frame `k` marked with red `k` in
    /// its middle (`clear`: the system has no wait cursor).
    fn with_hourglass(clear: bool) -> Compiled {
        struct Hourglass { clear: bool }
        impl render_desc::ImageLoader for Hourglass {
            fn load(&self, source: &str, options: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
                assert_eq!(source, "system-cursor:32514/32", "the busy cursor is the wait cursor's hourglass");
                assert_eq!(options.get("rotations").and_then(toml::Value::as_integer), Some(12));
                let side = scene::loading_cursor::FRAME_SIDE;
                Ok((0..12u8).map(|k| tile_render::RgbaImage::from_fn(side, side, |x, y| {
                    if self.clear { [0; 4] } else if (x, y) == (side / 2, side / 2) { [k, 0, 0, 255] } else { [0, 0, 0, 255] }
                })).collect())
            }
        }
        let text = format!(r#"
            grid = {{ layout = "orthogonal", step = [8, 8] }}
            [images.arrow]
            source = "pointer:255,255,255,255:14x22"
            [sheets.arrow]
            image = "arrow"
            cell = [14, 22]
            group = "Viewer@cursors"
            [images.capture]
            source = "solid:100,200,80,255:24x24"
            [sheets.capture]
            image = "capture"
            cell = [24, 24]
            anchor = [-12, -12]
            group = "Viewer@capture"
            [images.command]
            source = "solid:220,40,50,255:32x32"
            [sheets.command]
            image = "command"
            cell = [32, 32]
            anchor = [-16, -16]
            group = "Viewer@command"
            {}"#, include_str!("../../../assets/civ3/loading-cursor.toml"));
        assert!(!text.contains("Cursor.flc"), "never the unit-selection ring");
        let desc = render_desc::Description::parse(&text).unwrap();
        // (The arrow is generated; only the hourglass goes to the loader.)
        desc.compile(&Hourglass { clear }, 256).unwrap()
    }

    #[test]
    fn the_full_stuck_pointer_is_the_turning_hourglass_with_its_hot_spot_in_the_middle() {
        let compiled = with_hourglass(false);
        let art = Art::of(&compiled);
        assert_eq!(art.busy_frames(), 12);
        let arrow = art.arrow.unwrap();
        let side = scene::loading_cursor::FRAME_SIDE as usize;
        for k in 0..12u32 {
            let elapsed = std::time::Duration::from_millis(u64::from(k) * scene::loading_cursor::FRAME_MS);
            let id = art.busy_cursor(elapsed).unwrap();
            assert_ne!(id, arrow, "not the arrow with a badge");
            let image = art.get(id).unwrap();
            assert_eq!(image.size, (side as u16, side as u16));
            assert_eq!(image.hotspot, (23, 23), "hot spot at the centre, where the hourglass turns");
            let middle = (side / 2 * side + side / 2) * 4;
            assert_eq!(&image.rgba[middle..middle + 4], &[k as u8, 0, 0, 255], "frame {k}: {}°", scene::loading_cursor::angle_of(k));
            assert_eq!(crate::cursor::system_pointer(scene::Cursor::Sprite(id), &art, false, false), Shown::Custom(id));
        }
        // A full turn later: upright again.
        let turn = std::time::Duration::from_millis(12 * scene::loading_cursor::FRAME_MS);
        assert_eq!(art.busy_cursor(turn), art.busy_cursor(std::time::Duration::ZERO));
    }

    #[test]
    fn without_a_system_hourglass_the_busy_pointer_is_the_arrow() {
        let art = Art::of(&with_hourglass(true));
        assert_eq!(art.busy_frames(), 0);
        assert_eq!(art.busy_cursor(std::time::Duration::from_millis(500)), art.arrow);
    }

    #[test]
    fn loading_frames_use_host_time_and_loop_at_native_cadence() {
        let art = Art { busy: vec![90,91,92], arrow: Some(5), ..Default::default() };
        for (ms,id) in [(0,90),(82,90),(83,91),(165,91),(166,92),(249,90)] {
            assert_eq!(art.busy_cursor(std::time::Duration::from_millis(ms)),Some(id));
        }
        assert_eq!(Art { arrow: Some(5), ..Default::default() }.busy_cursor(std::time::Duration::ZERO),Some(5));
    }

    #[test]
    fn busy_and_stuck_preserve_capture_command_and_native_special_pointers() {
        use crate::host_status::Responsiveness as R;
        let compiled = with_hourglass(false);
        let art = std::sync::Arc::new(Art::of(&compiled));
        let command = compiled.set("Viewer@command/0").unwrap().0.raw();
        let elapsed = std::time::Duration::from_millis(83);
        for (cursor, capture, targeting, preserve, base, special) in [
            (scene::Cursor::Default, false, false, false, art.arrow.unwrap(), false),
            (scene::Cursor::System(scene::SystemCursor::Progress), false, false, false, art.arrow.unwrap(), false),
            (scene::Cursor::Default, true, false, false, art.capture().unwrap(), true),
            (scene::Cursor::Sprite(command), false, false, false, command, true),
            (scene::Cursor::Sprite(command), true, true, false, command, true),
            // At a captured map edge the OS draws the world cursor, even
            // if its shape happens to be the ordinary arrow.
            (scene::Cursor::Default, false, false, true, art.arrow.unwrap(), true),
        ] {
            let mut system = crate::system_cursor::SystemCursor::default();
            system.refresh(&scene::Scene { seq: 1, assets: 1, cursor, ..Default::default() }, 1, 1, art.clone());
            for status in [R::Answering, R::Busy, R::Stuck] {
                let effective = system.status(status);
                assert_eq!(system.next_animation_wake(false, status, elapsed).is_some(), effective != R::Answering);
                assert_eq!(system.next_animation_wake(true, status, elapsed), None);
                let expected = match busy_effect(effective, special) {
                    BusyEffect::None => Shown::Custom(base),
                    BusyEffect::Badge => art.badged(base, elapsed).unwrap(),
                    BusyEffect::Hourglass => Shown::Custom(art.busy_cursor(elapsed).unwrap()),
                };
                assert_eq!(system.shown_busy(false, capture, targeting, preserve, status, elapsed), expected, "{cursor:?} capture={capture} {status:?}");
                assert_eq!(system.shown_busy(true, capture, targeting, preserve, status, elapsed), Shown::Hidden);
            }
        }
        // Native cursors without CPU image pixels keep their shape. The
        // viewer draws just the badge beside them in its overlay camera.
        let mut native = crate::system_cursor::SystemCursor::default();
        native.refresh(&scene::Scene { seq: 1, assets: 1, cursor: scene::Cursor::System(scene::SystemCursor::Text), ..Default::default() }, 1, 1, art);
        for status in [R::Answering, R::Busy, R::Stuck] {
            assert_eq!(native.shown_busy(false, false, false, false, status, elapsed), Shown::Icon(winit::window::CursorIcon::Text));
        }
    }

    #[test]
    fn a_busy_native_image_keeps_the_hotspot_and_adds_a_small_hourglass_below_right() {
        let art = Art::of(&with_hourglass(false));
        for base in [art.arrow.unwrap(), art.capture().unwrap()] {
            let image = art.get(base).unwrap();
            let badge = art.cursor_image(art.badged(base, std::time::Duration::ZERO).unwrap()).unwrap();
            assert_eq!(badge.hotspot, image.hotspot);
            let offset = badge_offset((image.size.0.into(), image.size.1.into()), image.hotspot);
            let start = (image.hotspot.0 as u32 + offset.0 as u32, image.hotspot.1 as u32 + offset.1 as u32);
            assert!(start.0 > image.hotspot.0 as u32 && start.1 > image.hotspot.1 as u32);
            assert_eq!(badge.size, ((image.size.0 as u32).max(start.0 + BUSY_BADGE_SIDE) as u16, (image.size.1 as u32).max(start.1 + BUSY_BADGE_SIDE) as u16));
            // Every original pointer pixel survives unchanged.
            for y in 0..image.size.1 as usize {
                let source = y * image.size.0 as usize * 4;
                let dest = y * badge.size.0 as usize * 4;
                let len = image.size.0 as usize * 4;
                assert_eq!(&image.rgba[source..source + len], &badge.rgba[dest..dest + len]);
            }
            assert_ne!(art.badged(base, std::time::Duration::ZERO), art.badged(base, std::time::Duration::from_millis(83)), "the small hourglass turns too");
        }
    }

    #[test]
    fn captured_drawn_cursor_keeps_its_shape_with_a_scaled_busy_or_stuck_badge() {
        use crate::host_status::Responsiveness as R;
        let compiled = with_hourglass(false);
        let set = compiled.set("Viewer@command/0").unwrap().0;
        let art = SetArt::of(&compiled, set).unwrap();
        let hourglass = compiled.set(scene::loading_cursor::SET).unwrap().0;
        let camera = Camera { cells: true, ..Camera::new((400, 300)) };
        let marks = Machine::default().step(&Facts {
            captured: true, world_cursor: vec![(100, 100)],
            cursor_view: Some(([0, 0, 400, 300], (100, 100), false)), ..Default::default()
        }).marks;
        for scale in [Scale::default(), Scale::new(48, 1), Scale::new(64, 2)] {
            for status in [R::Answering, R::Busy, R::Stuck] {
                let out = compose_with_busy(&compiled, art, &marks, &[camera], &[], scale, (400, 300), 1, (0, 0), status);
                assert_eq!(out.sprites.iter().filter(|s| s.set == set).count(), 1, "keep the captured cursor for {status:?}");
                let badges: Vec<_> = out.sprites.iter().filter(|s| s.set == hourglass).collect();
                assert_eq!(badges.len(), usize::from(status != R::Answering));
                if let Some(badge) = badges.first() {
                    assert_eq!(badge.ticks_per_frame, scene::loading_cursor::TICKS_PER_FRAME);
                    let c = out.top.last().unwrap();
                    assert_eq!(c.viewport, (scale.span(BUSY_BADGE_SIDE), scale.span(BUSY_BADGE_SIDE)));
                    assert!(c.origin.0 > 100 && c.origin.1 > 100);
                    assert_eq!(c.target_clip, Some([0, 0, 400, 300]));
                }
            }
        }
    }
}
