//! A compact binary encoding of scenes and input, for when driver and
//! renderer cannot share Rust objects (another process, or the native side
//! of a Wine program). Little endian, length-prefixed, versioned.

use crate::{Button, FreeSprite, Input, LayerCells, Scene, View, WorldShape};

/// Bumped whenever the encoding changes.
pub const VERSION: u32 = 1;

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
    fn u64(&mut self, v: u64) {
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
    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
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

/// Encodes a scene.
pub fn encode_scene(s: &Scene) -> Vec<u8> {
    let mut w = Writer::default();
    w.u32(VERSION);
    w.u64(s.seq);
    w.u32(s.tick);
    let v = &s.view;
    w.i32(v.camera_x);
    w.i32(v.camera_y);
    w.u32(v.zoom.0);
    w.u32(v.zoom.1);
    w.u32(v.viewport.0);
    w.u32(v.viewport.1);
    w.bool(v.cull.is_some());
    let (x0, y0, x1, y1) = v.cull.unwrap_or_default();
    for c in [x0, y0, x1, y1] {
        w.i32(c);
    }
    w.u32(s.shape.cols);
    w.u32(s.shape.rows);
    w.bool(s.shape.wrap_x);
    w.bool(s.shape.wrap_y);
    w.u32(s.layers.len() as u32);
    for l in &s.layers {
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
    w.u32(s.sprites.len() as u32);
    for f in &s.sprites {
        w.u32(f.set);
        w.i32(f.x);
        w.i32(f.y);
        w.bool(f.screen);
        w.u32(f.after_pass);
        w.u32(f.phase);
        w.u32(f.ticks_per_frame);
        w.u32(f.scale_den);
    }
    w.0
}

/// Decodes a scene.
pub fn decode_scene(bytes: &[u8]) -> Result<Scene, WireError> {
    let mut r = Reader { bytes };
    let version = r.u32()?;
    if version != VERSION {
        return Err(WireError::Version(version));
    }
    let seq = r.u64()?;
    let tick = r.u32()?;
    let (camera_x, camera_y) = (r.i32()?, r.i32()?);
    let zoom = (r.u32()?, r.u32()?);
    let viewport = (r.u32()?, r.u32()?);
    let has_cull = r.bool()?;
    let cull = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
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
    let n = r.len(29)?;
    let mut sprites = Vec::with_capacity(n);
    for _ in 0..n {
        sprites.push(FreeSprite {
            set: r.u32()?,
            x: r.i32()?,
            y: r.i32()?,
            screen: r.bool()?,
            after_pass: r.u32()?,
            phase: r.u32()?,
            ticks_per_frame: r.u32()?,
            scale_den: r.u32()?,
        });
    }
    Ok(Scene {
        seq,
        tick,
        view: View { camera_x, camera_y, zoom, viewport, cull: has_cull.then_some(cull) },
        shape,
        layers,
        sprites,
    })
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
    Ok(Assets { description: string(&mut r)?, roots })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_and_input_round_trip() {
        let mut s = Scene::empty(WorldShape { cols: 3, rows: 2, wrap_x: true, wrap_y: false }, 2);
        s.seq = 99;
        s.tick = 7;
        s.view = View { camera_x: -5, camera_y: 12, zoom: (1, 2), viewport: (640, 480), cull: Some((1, 2, 3, 4)) };
        s.set(1, 2, 1, 0xABCDEF);
        s.layers[0].phases = Some(vec![1, 2, 3, 4, 5, 6]);
        s.sprites.push(FreeSprite { set: 4, x: -3, y: 9, screen: true, after_pass: u32::MAX, phase: 2, ticks_per_frame: 3, scale_den: 16 });
        assert_eq!(decode_scene(&encode_scene(&s)).unwrap(), s);
        let events = vec![
            Input::MouseMove { x: 1, y: -2 },
            Input::MouseButton { button: Button::Right, down: true, x: 3, y: 4 },
            Input::Wheel { delta: -120, x: 0, y: 0 },
            Input::Key { vk: 0x41, down: false, text: Some('a') },
            Input::Key { vk: 0x10, down: true, text: None },
            Input::Resized { width: 800, height: 600 },
            Input::Quit,
        ];
        assert_eq!(decode_input(&encode_input(&events)).unwrap(), events);
        let a = Assets { description: "[grid]\n".into(), roots: vec!["C:\\Games".into(), "Z:\\x".into()] };
        assert_eq!(decode_assets(&encode_assets(&a)).unwrap(), a);
        assert_eq!(decode_scene(&[1, 0, 0, 0, 5]), Err(WireError::Truncated));
        assert_eq!(decode_input(&[2, 0, 0, 0]), Err(WireError::Version(2)));
    }
}
