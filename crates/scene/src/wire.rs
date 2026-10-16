//! A compact binary encoding of scenes and input, for when driver and
//! renderer cannot share Rust objects (another process, or the native side
//! of a Wine program). Little endian, length-prefixed, versioned.

use crate::{Button, FreeSprite, Input, LayerCells, Patch, Update, View, WorldShape};

/// Bumped whenever the encoding changes.
pub const VERSION: u32 = 13;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    #[error("truncated message")]
    Truncated,
    #[error("wire version {0}, expected {VERSION}")]
    Version(u32),
    #[error("bad tag {0}")]
    Tag(u8),
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
        }
    }
}

impl Reader<'_> {
    fn view(&mut self) -> Result<View, WireError> {
        let (camera_x, camera_y) = (self.i32()?, self.i32()?);
        let zoom = (self.u32()?, self.u32()?);
        let viewport = (self.u32()?, self.u32()?);
        let has_cull = self.bool()?;
        let cull = (self.i32()?, self.i32()?, self.i32()?, self.i32()?);
        let origin = (self.i32()?, self.i32()?);
        let interface_only = self.bool()?;
        Ok(View { camera_x, camera_y, zoom, viewport, cull: has_cull.then_some(cull), origin, interface_only })
    }

    fn sprites(&mut self) -> Result<Vec<FreeSprite>, WireError> {
        let n = self.len(63)?;
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
        }
    }
    w.0
}

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
                    layers.push(LayerCells { values, phases });
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
            Input::Zooms(zooms) => {
                w.u8(7);
                for (n, d) in zooms {
                    w.u8(n);
                    w.u8(d);
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
    for _ in 0..n {
        out.push(match r.u8()? {
            0 => Input::MouseMove { x: r.i32()?, y: r.i32()? },
            1 => {
                let button = match r.u8()? {
                    0 => Button::Left,
                    1 => Button::Right,
                    2 => Button::Middle,
                    t => return Err(WireError::Tag(t)),
                };
                Input::MouseButton { button, down: r.bool()?, x: r.i32()?, y: r.i32()? }
            }
            2 => Input::Wheel { delta: r.i32()?, x: r.i32()?, y: r.i32()? },
            3 => Input::Key { vk: r.u32()?, down: r.bool()?, text: char::from_u32(r.u32()?) },
            4 => Input::Resized { width: r.u32()?, height: r.u32()? },
            5 => Input::Quit,
            6 => Input::Look { x: r.i32()?, y: r.i32()? },
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
    /// Compile the description this many times smaller (1: as it is; 2: a
    /// zoomed-out view).
    pub scale_down: u32,
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
    w.u32(a.scale_down);
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
    let scale_down = r.u32()?;
    let generation = r.u32()? as u64 | (r.u32()? as u64) << 32;
    Ok(Assets { description, roots, scale_down, generation })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scene;

    #[test]
    fn updates_and_input_round_trip() {
        let mut s = Scene::empty(WorldShape { cols: 3, rows: 2, wrap_x: true, wrap_y: false }, 2);
        s.tick = 7;
        s.view_mut(0).view = View { camera_x: -5, camera_y: 12, zoom: (1, 2), viewport: (640, 480), cull: Some((1, 2, 3, 4)), origin: (3, 4), interface_only: false };
        s.set(1, 2, 1, 0xABCDEF);
        s.layers[0].phases = Some(vec![1, 2, 3, 4, 5, 6]);
        s.sprites.push(FreeSprite { set: 4, x: -3, y: 9, screen: false, after_pass: u32::MAX, phase: 2, ticks_per_frame: 3, scale_den: 16, tint: 0x123456, zoom_art: 2, zoom: (3, 2), zoom_offset: (-4, 7), clip: None });
        s.view_mut(2).overlay.push(FreeSprite { set: 5, x: 1, y: 2, screen: true, after_pass: 0, phase: 0, ticks_per_frame: 1, scale_den: 0, tint: 0, zoom_art: 0, zoom: (0, 0), zoom_offset: (0, 0), clip: Some([1, -2, 300, 40]) });
        s.patches.push(Patch { sprite: "Screen@x/0".into(), width: 1, height: 2, rgba: vec![1, 2, 3, 4, 5, 6, 7, 8] });
        s.focus = Some((-64, 96));
        s.assets = 3;
        s.zooms = vec![(3, 2)];
        let mut updates = s.updates_since(&Scene::default());
        updates.push(Update::Cells(vec![(1, 0, 9)]));
        updates.push(Update::View { id: 2, view: None });
        assert_eq!(decode_updates(&encode_updates(&updates)).unwrap(), updates);
        let mut got = Scene::default();
        for u in decode_updates(&encode_updates(&updates)).unwrap() {
            got.apply(u);
        }
        assert_eq!(got.layers[1].values[0], 9);
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
        ];
        assert_eq!(decode_input(&encode_input(&events)).unwrap(), events);
        let a = Assets { description: "[grid]\n".into(), roots: vec!["C:\\Games".into(), "Z:\\x".into()], scale_down: 2, generation: 7 };
        assert_eq!(decode_assets(&encode_assets(&a)).unwrap(), a);
        assert_eq!(decode_updates(&[VERSION as u8, 0, 0, 0, 5]), Err(WireError::Truncated));
        assert_eq!(decode_input(&[0xFE, 0, 0, 0]), Err(WireError::Version(0xFE)));
    }
}
