//! A compact binary encoding of scenes and input, for when driver and
//! renderer cannot share Rust objects (another process, or the native side
//! of a Wine program). Little endian, length-prefixed, versioned.

use crate::{Button, Cursor, FreeSprite, Glyph, Input, Interactable, InteractableKind, Interface, LayerCells, Patch, SystemCursor, Update, View, WorldShape};

/// Bumped whenever the encoding changes.
pub const VERSION: u32 = 26;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    #[error("truncated message")]
    Truncated,
    #[error("wire version {0}, expected {VERSION}")]
    Version(u32),
    #[error("bad tag {0}")]
    Tag(u8),
    #[error("settings: {0}")]
    Settings(String),
}

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        if self.bytes.len() < n {
            return Err(WireError::Truncated);
        }
        let (head, tail) = self.bytes.split_at(n);
        self.bytes = tail;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, WireError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i16(&mut self) -> Result<i16, WireError> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn bool(&mut self) -> Result<bool, WireError> {
        Ok(self.u8()? != 0)
    }
    /// A length that cannot exceed what is left (`unit` bytes per item).
    fn len(&mut self, unit: usize) -> Result<usize, WireError> {
        let n = self.u32()? as usize;
        if n.saturating_mul(unit) > self.bytes.len() {
            return Err(WireError::Truncated);
        }
        Ok(n)
    }
}

impl Writer {
    /// A glyph drawn from outlines, if any: a flag, then its fields.
    fn glyph(&mut self, g: Option<&Glyph>) {
        self.bool(g.is_some());
        let Some(g) = g else { return };
        self.u32(g.font);
        self.u32(g.ch as u32);
        self.u32(g.em64);
        self.i32(g.advance);
        self.u8(g.bold as u8 | (g.italic as u8) << 1 | (g.key.is_some() as u8) << 2 | (g.bar.is_some() as u8) << 3);
        for c in g.key.unwrap_or_default() {
            self.u8(c);
        }
        let (top, height) = g.bar.unwrap_or_default();
        self.i32(top);
        self.i32(height);
    }

    fn view(&mut self, v: &View) {
        self.i32(v.camera_x);
        self.i32(v.camera_y);
        self.u32(v.zoom.0);
        self.u32(v.zoom.1);
        self.u32(v.viewport.0);
        self.u32(v.viewport.1);
        self.bool(v.cull.is_some());
        let (x0, y0, x1, y1) = v.cull.unwrap_or_default();
        for c in [x0, y0, x1, y1] {
            self.i32(c);
        }
        self.i32(v.origin.0);
        self.i32(v.origin.1);
        self.bool(v.interface_only);
        self.u32(v.downscale);
    }

    fn sprites(&mut self, sprites: &[FreeSprite]) {
        self.u32(sprites.len() as u32);
        for f in sprites {
            self.u32(f.set);
            self.i32(f.x);
            self.i32(f.y);
            self.bool(f.screen);
            self.u32(f.after_pass);
            self.u32(f.phase);
            self.u32(f.ticks_per_frame);
            self.u32(f.scale_den);
            self.u32(f.tint);
            self.u8(f.zoom_art);
            self.u32(u32::from(f.zoom.0) << 16 | u32::from(f.zoom.1));
            self.i32(f.zoom_offset.0);
            self.i32(f.zoom_offset.1);
            self.bool(f.clip.is_some());
            for c in f.clip.unwrap_or_default() {
                self.i32(c);
            }
            self.bool(f.not_in_view.is_some());
            self.u32(f.not_in_view.unwrap_or(0));
            self.glyph(f.glyph.as_ref());
            self.u32(f.palette);
        }
    }
}

impl Reader<'_> {
    fn glyph(&mut self) -> Result<Option<Glyph>, WireError> {
        if !self.bool()? {
            return Ok(None);
        }
        let (font, ch, em64, advance) = (self.u32()?, self.u32()?, self.u32()?, self.i32()?);
        let style = self.u8()?;
        let key = [self.u8()?, self.u8()?, self.u8()?];
        let bar = (self.i32()?, self.i32()?);
        Ok(Some(Glyph {
            font,
            ch: char::from_u32(ch).unwrap_or('?'),
            em64,
            advance,
            bold: style & 1 != 0,
            italic: style & 2 != 0,
            key: (style & 4 != 0).then_some(key),
            bar: (style & 8 != 0).then_some(bar),
        }))
    }

    fn view(&mut self) -> Result<View, WireError> {
        let (camera_x, camera_y) = (self.i32()?, self.i32()?);
        let zoom = (self.u32()?, self.u32()?);
        let viewport = (self.u32()?, self.u32()?);
        let has_cull = self.bool()?;
        let cull = (self.i32()?, self.i32()?, self.i32()?, self.i32()?);
        let origin = (self.i32()?, self.i32()?);
        let interface_only = self.bool()?;
        let downscale = self.u32()?;
        Ok(View { camera_x, camera_y, zoom, viewport, cull: has_cull.then_some(cull), origin, interface_only, downscale })
    }

    fn sprites(&mut self) -> Result<Vec<FreeSprite>, WireError> {
        let n = self.len(72)?;
        (0..n)
            .map(|_| {
                Ok(FreeSprite {
                    set: self.u32()?,
                    x: self.i32()?,
                    y: self.i32()?,
                    screen: self.bool()?,
                    after_pass: self.u32()?,
                    phase: self.u32()?,
                    ticks_per_frame: self.u32()?,
                    scale_den: self.u32()?,
                    tint: self.u32()?,
                    zoom_art: self.u8()?,
                    zoom: {
                        let z = self.u32()?;
                        ((z >> 16) as u16, z as u16)
                    },
                    zoom_offset: (self.i32()?, self.i32()?),
                    clip: {
                        let has = self.bool()?;
                        let c = [self.i32()?, self.i32()?, self.i32()?, self.i32()?];
                        has.then_some(c)
                    },
                    not_in_view: {
                        let has = self.bool()?;
                        let v = self.u32()?;
                        has.then_some(v)
                    },
                    glyph: self.glyph()?,
                    palette: self.u32()?,
                })
            })
            .collect()
    }
}

/// Encodes a batch of updates.
pub fn encode_updates(updates: &[Update]) -> Vec<u8> {
    let mut w = Writer::default();
    w.u32(VERSION);
    w.u32(updates.len() as u32);
    for u in updates {
        match u {
            Update::World { shape, layers } => {
                w.u8(0);
                w.u32(shape.cols);
                w.u32(shape.rows);
                w.bool(shape.wrap_x);
                w.bool(shape.wrap_y);
                w.u32(layers.len() as u32);
                for l in layers {
                    w.u32(l.values.len() as u32);
                    for &v in &l.values {
                        w.u32(v);
                    }
                    w.bool(l.phases.is_some());
                    if let Some(p) = &l.phases {
                        w.u32(p.len() as u32);
                        w.0.extend_from_slice(p);
                    }
                    w.bool(l.offsets.is_some());
                    if let Some(o) = &l.offsets {
                        w.u32(o.len() as u32);
                        for &(x, y) in o {
                            w.i16(x);
                            w.i16(y);
                        }
                    }
                }
            }
            Update::Cells(cells) => {
                w.u8(1);
                w.u32(cells.len() as u32);
                for &(l, i, v) in cells {
                    w.u32(l);
                    w.u32(i);
                    w.u32(v);
                }
            }
            Update::Phases(phases) => {
                w.u8(13);
                w.u32(phases.len() as u32);
                for &(l, i, p) in phases {
                    w.u32(l);
                    w.u32(i);
                    w.u8(p);
                }
            }
            Update::Offsets(offsets) => {
                w.u8(14);
                w.u32(offsets.len() as u32);
                for &(l, i, (x, y)) in offsets {
                    w.u32(l);
                    w.u32(i);
                    w.i16(x);
                    w.i16(y);
                }
            }
            Update::Sprites(sprites) => {
                w.u8(2);
                w.sprites(sprites);
            }
            Update::View { id, view } => {
                w.u8(3);
                w.u32(*id);
                w.bool(view.is_some());
                if let Some(v) = view {
                    w.view(v);
                }
            }
            Update::Overlay { view, sprites } => {
                w.u8(4);
                w.u32(*view);
                w.sprites(sprites);
            }
            Update::Patches(patches) => {
                w.u8(5);
                w.u32(patches.len() as u32);
                for p in patches {
                    w.u32(p.sprite.len() as u32);
                    w.0.extend_from_slice(p.sprite.as_bytes());
                    w.u32(p.width);
                    w.u32(p.height);
                    w.u32(p.rgba.len() as u32);
                    w.0.extend_from_slice(&p.rgba);
                }
            }
            Update::Tick(t) => {
                w.u8(6);
                w.u32(*t);
            }
            Update::Focus(f) => {
                w.u8(7);
                w.bool(f.is_some());
                let (x, y) = f.unwrap_or_default();
                w.i32(x);
                w.i32(y);
            }
            Update::Assets(n) => {
                w.u8(8);
                w.u32(*n as u32);
                w.u32((*n >> 32) as u32);
            }
            Update::Zooms(z) => {
                w.u8(9);
                w.u32(z.len() as u32);
                for &(n, d) in z {
                    w.u32(n);
                    w.u32(d);
                }
            }
            // As their TOML (as they are saved).
            Update::Settings(s) => {
                w.u8(10);
                let text = toml::to_string(s).unwrap_or_default();
                w.u32(text.len() as u32);
                w.0.extend_from_slice(text.as_bytes());
            }
            Update::Interface(i) => {
                w.u8(11);
                let screen = i.screen.as_deref().unwrap_or_default();
                w.bool(i.screen.is_some());
                w.u32(screen.len() as u32);
                w.0.extend_from_slice(screen.as_bytes());
                for view in [i.roaming, i.popup, i.screen_view] {
                    w.bool(view.is_some());
                    w.u32(view.unwrap_or_default());
                }
                for rect in [i.reachable, i.focus_control] {
                    w.bool(rect.is_some());
                    for v in rect.unwrap_or_default() {
                        w.i32(v);
                    }
                }
            }
            Update::Interactables(items) => {
                w.u8(12);
                w.u32(items.len() as u32);
                for i in items {
                    for v in i.rect {
                        w.i32(v);
                    }
                    w.u32(i.id as u32);
                    w.u32((i.id >> 32) as u32);
                    let (tag, scroll, step) = match i.kind {
                        InteractableKind::Button => (0, [0; 4], 0),
                        InteractableKind::ListItem => (1, [0; 4], 0),
                        InteractableKind::List { scroll, step } => (2, scroll, step),
                    };
                    w.u8(tag);
                    for v in scroll {
                        w.i32(v);
                    }
                    w.i32(step);
                }
            }
            Update::Cursor(c) => {
                w.u8(15);
                let (tag, value) = match *c {
                    Cursor::Default => (0, 0),
                    Cursor::Sprite(set) => (1, set),
                    Cursor::System(s) => (2, SystemCursor::ALL.iter().position(|&k| k == s).unwrap_or(0) as u32),
                };
                w.u8(tag);
                w.u32(value);
            }
        }
    }
    w.0
}

/// Bytes of one encoded [`Interactable`].
const INTERACTABLE_BYTES: usize = 16 + 8 + 1 + 16 + 4;

/// Decodes a batch of updates.
pub fn decode_updates(bytes: &[u8]) -> Result<Vec<Update>, WireError> {
    let mut r = Reader { bytes };
    let version = r.u32()?;
    if version != VERSION {
        return Err(WireError::Version(version));
    }
    let n = r.len(1)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(match r.u8()? {
            0 => {
                let shape = WorldShape { cols: r.u32()?, rows: r.u32()?, wrap_x: r.bool()?, wrap_y: r.bool()? };
                let n = r.len(5)?;
                let mut layers = Vec::with_capacity(n);
                for _ in 0..n {
                    let m = r.len(4)?;
                    let values = (0..m).map(|_| r.u32()).collect::<Result<_, _>>()?;
                    let phases = if r.bool()? {
                        let k = r.len(1)?;
                        Some(r.take(k)?.to_vec())
                    } else {
                        None
                    };
                    let offsets = if r.bool()? {
                        let k = r.len(4)?;
                        Some((0..k).map(|_| Ok((r.i16()?, r.i16()?))).collect::<Result<_, WireError>>()?)
                    } else {
                        None
                    };
                    layers.push(LayerCells { values, phases, offsets });
                }
                Update::World { shape, layers }
            }
            1 => {
                let n = r.len(12)?;
                Update::Cells((0..n).map(|_| Ok((r.u32()?, r.u32()?, r.u32()?))).collect::<Result<_, WireError>>()?)
            }
            2 => Update::Sprites(r.sprites()?),
            3 => {
                let id = r.u32()?;
                let view = if r.bool()? { Some(r.view()?) } else { None };
                Update::View { id, view }
            }
            4 => Update::Overlay { view: r.u32()?, sprites: r.sprites()? },
            5 => {
                let n = r.len(16)?;
                let mut patches = Vec::with_capacity(n);
                for _ in 0..n {
                    let n = r.len(1)?;
                    let sprite = String::from_utf8_lossy(r.take(n)?).into_owned();
                    let (width, height) = (r.u32()?, r.u32()?);
                    let k = r.len(1)?;
                    patches.push(Patch { sprite, width, height, rgba: r.take(k)?.to_vec() });
                }
                Update::Patches(patches)
            }
            6 => Update::Tick(r.u32()?),
            7 => {
                let some = r.bool()?;
                let p = (r.i32()?, r.i32()?);
                Update::Focus(some.then_some(p))
            }
            8 => Update::Assets(r.u32()? as u64 | (r.u32()? as u64) << 32),
            9 => {
                let n = r.len(8)?;
                Update::Zooms((0..n).map(|_| Ok((r.u32()?, r.u32()?))).collect::<Result<_, WireError>>()?)
            }
            10 => {
                let n = r.len(1)?;
                let text = String::from_utf8_lossy(r.take(n)?).into_owned();
                Update::Settings(toml::from_str(&text).map_err(|e| WireError::Settings(e.to_string()))?)
            }
            11 => {
                let named = r.bool()?;
                let n = r.len(1)?;
                let screen = String::from_utf8_lossy(r.take(n)?).into_owned();
                let mut view = || -> Result<Option<u32>, WireError> {
                    let some = r.bool()?;
                    let id = r.u32()?;
                    Ok(some.then_some(id))
                };
                let (roaming, popup, screen_view) = (view()?, view()?, view()?);
                let mut rect = || -> Result<Option<[i32; 4]>, WireError> {
                    let some = r.bool()?;
                    let rect = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                    Ok(some.then_some(rect))
                };
                let (reachable, focus_control) = (rect()?, rect()?);
                Update::Interface(Interface { screen: named.then_some(screen), roaming, popup, screen_view, reachable, focus_control })
            }
            12 => {
                let n = r.len(INTERACTABLE_BYTES)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    let rect = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                    let id = r.u32()? as u64 | (r.u32()? as u64) << 32;
                    let tag = r.u8()?;
                    let scroll = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                    let step = r.i32()?;
                    let kind = match tag {
                        0 => InteractableKind::Button,
                        1 => InteractableKind::ListItem,
                        2 => InteractableKind::List { scroll, step },
                        t => return Err(WireError::Tag(t)),
                    };
                    items.push(Interactable { rect, kind, id });
                }
                Update::Interactables(items)
            }
            13 => {
                let n = r.len(9)?;
                Update::Phases((0..n).map(|_| Ok((r.u32()?, r.u32()?, r.u8()?))).collect::<Result<_, WireError>>()?)
            }
            14 => {
                let n = r.len(12)?;
                Update::Offsets((0..n).map(|_| Ok((r.u32()?, r.u32()?, (r.i16()?, r.i16()?)))).collect::<Result<_, WireError>>()?)
            }
            15 => {
                let tag = r.u8()?;
                let value = r.u32()?;
                Update::Cursor(match tag {
                    0 => Cursor::Default,
                    1 => Cursor::Sprite(value),
                    2 => Cursor::System(*SystemCursor::ALL.get(value as usize).ok_or(WireError::Tag(tag))?),
                    t => return Err(WireError::Tag(t)),
                })
            }
            t => return Err(WireError::Tag(t)),
        });
    }
    Ok(out)
}

/// Encodes input events.
pub fn encode_input(events: &[Input]) -> Vec<u8> {
    let mut w = Writer::default();
    w.u32(VERSION);
    w.u32(events.len() as u32);
    let button = |b: Button| match b {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
    };
    for e in events {
        match *e {
            Input::MouseMove { x, y } => {
                w.u8(0);
                w.i32(x);
                w.i32(y);
            }
            Input::MouseButton { button: b, down, x, y } => {
                w.u8(1);
                w.u8(button(b));
                w.bool(down);
                w.i32(x);
                w.i32(y);
            }
            Input::Wheel { delta, x, y } => {
                w.u8(2);
                w.i32(delta);
                w.i32(x);
                w.i32(y);
            }
            Input::Key { vk, down, text } => {
                w.u8(3);
                w.u32(vk);
                w.bool(down);
                w.u32(text.map_or(u32::MAX, |c| c as u32));
            }
            Input::Resized { width, height } => {
                w.u8(4);
                w.u32(width);
                w.u32(height);
            }
            Input::Quit => w.u8(5),
            Input::Look { x, y } => {
                w.u8(6);
                w.i32(x);
                w.i32(y);
            }
            Input::Views(views) => {
                w.u8(8);
                for (x, y, ww, h) in views {
                    for v in [x, y, ww, h] {
                        w.i32(v);
                    }
                }
            }
            Input::Zooms(zooms) => {
                w.u8(7);
                for (n, d) in zooms {
                    w.u8(n);
                    w.u8(d);
                }
            }
            Input::WorldButton { button: b, down, x, y } => {
                w.u8(9);
                w.u8(button(b));
                w.bool(down);
                w.i32(x);
                w.i32(y);
            }
            Input::WorldKey { vk, down, text, x, y } => {
                w.u8(10);
                w.u32(vk);
                w.bool(down);
                w.u32(text.map_or(u32::MAX, |c| c as u32));
                w.i32(x);
                w.i32(y);
            }
            Input::Edge { x, y } => {
                w.u8(11);
                w.i32(x);
                w.i32(y);
            }
            Input::MainView { x, y, w: ww, h } => {
                w.u8(12);
                for v in [x, y, ww, h] {
                    w.i32(v);
                }
            }
        }
    }
    w.0
}

/// Decodes input events.
pub fn decode_input(bytes: &[u8]) -> Result<Vec<Input>, WireError> {
    let mut r = Reader { bytes };
    let version = r.u32()?;
    if version != VERSION {
        return Err(WireError::Version(version));
    }
    let n = r.len(1)?;
    let mut out = Vec::with_capacity(n);
    let button = |r: &mut Reader| match r.u8()? {
        0 => Ok(Button::Left),
        1 => Ok(Button::Right),
        2 => Ok(Button::Middle),
        t => Err(WireError::Tag(t)),
    };
    for _ in 0..n {
        out.push(match r.u8()? {
            0 => Input::MouseMove { x: r.i32()?, y: r.i32()? },
            1 => {
                let button = button(&mut r)?;
                Input::MouseButton { button, down: r.bool()?, x: r.i32()?, y: r.i32()? }
            }
            9 => {
                let button = button(&mut r)?;
                Input::WorldButton { button, down: r.bool()?, x: r.i32()?, y: r.i32()? }
            }
            10 => Input::WorldKey { vk: r.u32()?, down: r.bool()?, text: char::from_u32(r.u32()?), x: r.i32()?, y: r.i32()? },
            2 => Input::Wheel { delta: r.i32()?, x: r.i32()?, y: r.i32()? },
            3 => Input::Key { vk: r.u32()?, down: r.bool()?, text: char::from_u32(r.u32()?) },
            4 => Input::Resized { width: r.u32()?, height: r.u32()? },
            5 => Input::Quit,
            6 => Input::Look { x: r.i32()?, y: r.i32()? },
            11 => Input::Edge { x: r.i32()?, y: r.i32()? },
            12 => Input::MainView { x: r.i32()?, y: r.i32()?, w: r.i32()?, h: r.i32()? },
            8 => {
                let mut views = [(0, 0, 0, 0); 4];
                for v in &mut views {
                    *v = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
                }
                Input::Views(views)
            }
            7 => {
                let mut zooms = [(0, 0); 8];
                for z in &mut zooms {
                    *z = (r.u8()?, r.u8()?);
                }
                Input::Zooms(zooms)
            }
            t => return Err(WireError::Tag(t)),
        });
    }
    Ok(out)
}

/// What a renderer needs to build its sprites: a render description and
/// the folders its images are searched in (highest priority first, as the
/// driver names them).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assets {
    pub description: String,
    pub roots: Vec<String>,
    /// Its generation, as scenes refer to it (`Scene::assets`).
    pub generation: u64,
}

pub fn encode_assets(a: &Assets) -> Vec<u8> {
    let mut w = Writer::default();
    w.u32(VERSION);
    w.u32(a.roots.len() as u32);
    for r in &a.roots {
        w.u32(r.len() as u32);
        w.0.extend_from_slice(r.as_bytes());
    }
    w.u32(a.description.len() as u32);
    w.0.extend_from_slice(a.description.as_bytes());
    w.u32(a.generation as u32);
    w.u32((a.generation >> 32) as u32);
    w.0
}

pub fn decode_assets(bytes: &[u8]) -> Result<Assets, WireError> {
    let mut r = Reader { bytes };
    let version = r.u32()?;
    if version != VERSION {
        return Err(WireError::Version(version));
    }
    let string = |r: &mut Reader| -> Result<String, WireError> {
        let n = r.len(1)?;
        Ok(String::from_utf8_lossy(r.take(n)?).into_owned())
    };
    let n = r.len(4)?;
    let roots = (0..n).map(|_| string(&mut r)).collect::<Result<_, _>>()?;
    let description = string(&mut r)?;
    let generation = r.u32()? as u64 | (r.u32()? as u64) << 32;
    Ok(Assets { description, roots, generation })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scene;

    #[test]
    fn updates_and_input_round_trip() {
        let mut s = Scene::empty(WorldShape { cols: 3, rows: 2, wrap_x: true, wrap_y: false }, 2);
        s.tick = 7;
        s.view_mut(0).view = View { camera_x: -5, camera_y: 12, zoom: (1, 2), viewport: (640, 480), cull: Some((1, 2, 3, 4)), origin: (3, 4), interface_only: false, downscale: 2 };
        s.set(1, 2, 1, 0xABCDEF);
        s.layers[0].phases = Some(vec![1, 2, 3, 4, 5, 6]);
        s.layers[1].offsets = Some(vec![(0, 0), (-300, 7), (0, 0), (5, -1), (0, 0), (0, 0)]);
        s.sprites.push(FreeSprite { set: 4, x: -3, y: 9, screen: false, after_pass: u32::MAX, phase: 2, ticks_per_frame: 3, scale_den: 16, tint: 0x123456, zoom_art: 2, zoom: (3, 2), zoom_offset: (-4, 7), clip: None, not_in_view: Some(3), glyph: None, palette: 7 });
        s.sprites.push(FreeSprite { x: 7, y: 30, zoom_art: 2, tint: 0xFFFF00, glyph: Some(Glyph { font: 1, ch: 'é', em64: 640, advance: 6, bold: true, italic: false, key: Some([255, 0, 255]), bar: None }), ..Default::default() });
        s.sprites.push(FreeSprite { x: 13, y: 30, zoom_art: 2, glyph: Some(Glyph { font: 0, ch: ' ', em64: 768, advance: 4, bold: false, italic: true, key: None, bar: Some((1, 2)) }), ..Default::default() });
        s.view_mut(2).overlay.push(FreeSprite { set: 5, x: 1, y: 2, screen: true, after_pass: 0, phase: 0, ticks_per_frame: 1, scale_den: 0, tint: 0, zoom_art: 0, zoom: (0, 0), zoom_offset: (0, 0), clip: Some([1, -2, 300, 40]), not_in_view: None, glyph: None, palette: 0 });
        s.patches.push(Patch { sprite: "Screen@x/0".into(), width: 1, height: 2, rgba: vec![1, 2, 3, 4, 5, 6, 7, 8] });
        s.focus = Some((-64, 96));
        s.assets = 3;
        s.zooms = vec![(3, 2)];
        let mut settings = viewer_settings::Settings::default();
        settings.resolution.pixel_scale = 3;
        settings.layout.split = viewer_settings::Split::Horizontal1To1;
        settings.layout.screens.insert("Spy".into(), viewer_settings::ScreenPlace { vertical: viewer_settings::Place::RightOrTop, horizontal: viewer_settings::Place::LeftOrBottom });
        settings.input.snap_updates_per_second = 4.5;
        s.settings = Some(settings);
        s.interface = Interface { screen: Some("F4".into()), roaming: Some(256), popup: None, screen_view: Some(258), reachable: Some([-1, 2, 300, 40]), focus_control: Some([5, 6, 7, 8]) };
        s.interactables = vec![
            Interactable { rect: [10, 20, 100, 24], kind: InteractableKind::Button, id: 0x1_0000_0002 },
            Interactable { rect: [5, 50, 200, 300], kind: InteractableKind::List { scroll: [5, 60, 190, 280], step: 20 }, id: 7 },
            Interactable { rect: [5, 60, 190, 20], kind: InteractableKind::ListItem, id: 8 },
        ];
        let mut updates = s.updates_since(&Scene::default());
        assert!(updates.iter().any(|u| matches!(u, Update::Settings(_))));
        updates.push(Update::Cells(vec![(1, 0, 9)]));
        updates.push(Update::Phases(vec![(0, 5, 200)]));
        updates.push(Update::Offsets(vec![(1, 2, (-1, 32767))]));
        updates.push(Update::View { id: 2, view: None });
        updates.push(Update::Interface(Interface { screen: None, roaming: None, popup: Some(7), screen_view: None, reachable: None, focus_control: None }));
        updates.push(Update::Cursor(Cursor::Sprite(42)));
        updates.push(Update::Cursor(Cursor::System(SystemCursor::ResizeNesw)));
        updates.push(Update::Cursor(Cursor::Default));
        assert_eq!(decode_updates(&encode_updates(&updates)).unwrap(), updates);
        let mut got = Scene::default();
        for u in decode_updates(&encode_updates(&updates)).unwrap() {
            got.apply(u);
        }
        assert_eq!(got.layers[1].values[0], 9);
        assert_eq!(got.layers[0].phases.as_ref().unwrap()[5], 200);
        assert_eq!(got.layers[1].offsets.as_ref().unwrap()[1..3], [(-300, 7), (-1, 32767)]);
        assert_eq!(got.settings, s.settings);
        // Settings that are no TOML of settings: an error, not a panic.
        let mut bad = Writer::default();
        bad.u32(VERSION);
        bad.u32(1);
        bad.u8(10);
        bad.u32(3);
        bad.0.extend_from_slice(b"x=[");
        assert!(matches!(decode_updates(&bad.0), Err(WireError::Settings(_))));
        assert_eq!(got.views.len(), 1);
        let events = vec![
            Input::MouseMove { x: 1, y: -2 },
            Input::MouseButton { button: Button::Right, down: true, x: 3, y: 4 },
            Input::Wheel { delta: -120, x: 0, y: 0 },
            Input::Key { vk: 0x41, down: false, text: Some('a') },
            Input::Key { vk: 0x10, down: true, text: None },
            Input::Resized { width: 800, height: 600 },
            Input::Quit,
            Input::Look { x: -5, y: 7000 },
            Input::Zooms([(3, 2), (2, 1), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)]),
            Input::Views([(10, -20, 300, 200), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)]),
            Input::MainView { x: 5, y: -6, w: 700, h: 400 },
            Input::WorldButton { button: Button::Right, down: true, x: -64, y: 4096 },
            Input::WorldButton { button: Button::Middle, down: false, x: 0, y: 0 },
            Input::WorldKey { vk: 0x66, down: true, text: Some('6'), x: 130, y: -2 },
            Input::WorldKey { vk: 0x25, down: false, text: None, x: 1, y: 1 },
            Input::Edge { x: 16, y: -1 },
            Input::Edge { x: 0, y: 0 },
        ];
        assert_eq!(decode_input(&encode_input(&events)).unwrap(), events);
        let a = Assets { description: "[grid]\n".into(), roots: vec!["C:\\Games".into(), "Z:\\x".into()], generation: 7 };
        assert_eq!(decode_assets(&encode_assets(&a)).unwrap(), a);
        assert_eq!(decode_updates(&[VERSION as u8, 0, 0, 0, 5]), Err(WireError::Truncated));
        assert_eq!(decode_input(&[0xFE, 0, 0, 0]), Err(WireError::Version(0xFE)));
    }
}
