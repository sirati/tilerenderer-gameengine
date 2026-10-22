//! A compact binary encoding of scenes and input, for when driver and
//! renderer cannot share Rust objects (another process, or the native side
//! of a Wine program). Little endian, length-prefixed, versioned.

use crate::{Beside, Binding, Button, DetachedPopup, PopupAnchor, CameraFocus, Cursor, Edge, FreeSprite, Glyph, Input, Interactable, InteractableKind, Interface, LayerCells, LookState, Looks, Patch, PointerState, Select, SliderAction, SystemCursor, UnitOrderAction, Update, View, ViewLooks, WorldShape};

/// Bumped whenever the encoding changes.
pub const VERSION: u32 = 52;

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
        self.bool(v.menu);
        self.u32(v.downscale);
        self.u32(v.hidden_categories);
        self.u32(v.binding);
    }

    fn layer_cells(&mut self, l: &LayerCells) {
        self.u32(l.values.len() as u32);
        for &v in &l.values {
            self.u32(v);
        }
        self.bool(l.phases.is_some());
        if let Some(p) = &l.phases {
            self.u32(p.len() as u32);
            self.0.extend_from_slice(p);
        }
        self.bool(l.offsets.is_some());
        if let Some(o) = &l.offsets {
            self.u32(o.len() as u32);
            for &(x, y) in o {
                self.i16(x);
                self.i16(y);
            }
        }
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
            self.u8(match f.blend { crate::SpriteBlend::Alpha => 0, crate::SpriteBlend::PaletteAlpha256 => 1 });
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
            self.u32(f.category);
            self.bool(f.solid.is_some());
            if let Some(solid) = f.solid {
                self.u32(solid.width);
                self.u32(solid.height);
                for channel in solid.rgba { self.u8(channel); }
            }
        }
    }

    fn look_state(&mut self, s: LookState) {
        let pointer = match s.pointer {
            PointerState::Normal => 0,
            PointerState::Hover => 1,
            PointerState::Pressed => 2,
        };
        self.u8(pointer | u8::from(s.selected) << 2);
    }
}

impl Reader<'_> {
    fn look_state(&mut self) -> Result<LookState, WireError> {
        let b = self.u8()?;
        let pointer = match b & 3 {
            0 => PointerState::Normal,
            1 => PointerState::Hover,
            2 => PointerState::Pressed,
            _ => return Err(WireError::Tag(b)),
        };
        Ok(LookState { pointer, selected: b & 4 != 0 })
    }

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
        let menu = self.bool()?;
        let downscale = self.u32()?;
        let hidden_categories = self.u32()?;
        let binding = self.u32()?;
        Ok(View { camera_x, camera_y, zoom, viewport, cull: has_cull.then_some(cull), origin, interface_only, menu, downscale, hidden_categories, binding })
    }

    fn layer_cells(&mut self) -> Result<LayerCells, WireError> {
        let m = self.len(4)?;
        let values = (0..m).map(|_| self.u32()).collect::<Result<_, _>>()?;
        let phases = if self.bool()? {
            let k = self.len(1)?;
            Some(self.take(k)?.to_vec())
        } else {
            None
        };
        let offsets = if self.bool()? {
            let k = self.len(4)?;
            Some((0..k).map(|_| Ok((self.i16()?, self.i16()?))).collect::<Result<_, WireError>>()?)
        } else {
            None
        };
        Ok(LayerCells { values, phases, offsets })
    }

    fn sprites(&mut self) -> Result<Vec<FreeSprite>, WireError> {
        let n = self.len(79)?;
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
                    blend: match self.u8()? {
                        0 => crate::SpriteBlend::Alpha,
                        1 => crate::SpriteBlend::PaletteAlpha256,
                        value => return Err(WireError::Tag(value)),
                    },
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
                    category: self.u32()?,
                    solid: if self.bool()? {
                        Some(crate::SolidRect { width: self.u32()?, height: self.u32()?, rgba: [self.u8()?, self.u8()?, self.u8()?, self.u8()?] })
                    } else { None },
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
                    w.layer_cells(l);
                }
            }
            Update::Slot { id, slot } => {
                w.u8(20);
                w.u32(*id);
                w.bool(slot.is_some());
                if let Some((layer, cells)) = slot {
                    w.u32(*layer);
                    w.layer_cells(cells);
                }
            }
            Update::SlotCells(cells) => {
                w.u8(21);
                w.u32(cells.len() as u32);
                for &(s, i, v) in cells {
                    w.u32(s);
                    w.u32(i);
                    w.u32(v);
                }
            }
            Update::Bindings(bindings) => {
                w.u8(22);
                w.u32(bindings.len() as u32);
                for b in bindings {
                    w.u32(b.id);
                    w.u32(b.slots.len() as u32);
                    for &(layer, slot) in &b.slots {
                        w.u32(layer);
                        w.u32(slot);
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
            Update::SpriteLifetimes(lifetimes) => {
                w.u8(27);
                w.u32(lifetimes.len() as u32);
                for life in lifetimes {
                    w.u32(life.serial as u32);
                    w.u32((life.serial >> 32) as u32);
                    w.u32(life.sprites.0);
                    w.u32(life.sprites.1);
                    w.u32(life.duration_ms);
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
            Update::CameraArrival(arrival) => {
                w.u8(25);
                w.bool(arrival.is_some());
                if let Some(arrival) = arrival {
                    w.u32(arrival.serial as u32); w.u32((arrival.serial >> 32) as u32);
                    for n in arrival.bounds { w.i32(n); }
                }
            }
            Update::CameraFocus(focus) => {
                w.u8(18);
                w.bool(focus.is_some());
                let focus = focus.unwrap_or_default();
                w.u32(focus.serial as u32);
                w.u32((focus.serial >> 32) as u32);
                w.i32(focus.center.0);
                w.i32(focus.center.1);
            }
            Update::MapWheelStep((x, y)) => {
                w.u8(19);
                w.i32(*x);
                w.i32(*y);
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
            Update::SettingsPreview(p) => {
                w.u8(24);
                w.bool(p.is_some());
                if let Some(p) = p {
                    for n in [p.session, p.id] { w.u32(n as u32); w.u32((n >> 32) as u32); }
                    w.u32(p.pixel_scale); w.u32(p.previous_pixel_scale);
                }
            }
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
                w.bool(i.menu_fit);
                for view in [i.roaming, i.popup, i.screen_view, i.capture_hint] {
                    w.bool(view.is_some());
                    w.u32(view.unwrap_or_default());
                }
                for view in i.modal_stack.0 {
                    w.bool(view.is_some());
                    w.u32(view.unwrap_or_default());
                }
                for attached in i.modal_attached {
                    w.bool(attached);
                }
                for rect in [i.reachable, i.focus_control] {
                    w.bool(rect.is_some());
                    for v in rect.unwrap_or_default() {
                        w.i32(v);
                    }
                }
                w.bool(i.beside.is_some());
                let beside = i.beside.unwrap_or(Beside { screen: [0; 4], edge: Edge::Left, world: None });
                for v in beside.screen {
                    w.i32(v);
                }
                w.u8(match beside.edge {
                    Edge::Left => 0,
                    Edge::Right => 1,
                    Edge::Top => 2,
                    Edge::Bottom => 3,
                });
                w.bool(beside.world.is_some());
                for v in beside.world.unwrap_or_default() {
                    w.i32(v);
                }
                w.bool(i.popup_anchor.is_some());
                let a = i.popup_anchor.unwrap_or_default();
                for v in [a.world.0, a.world.1, a.offset.0, a.offset.1] {
                    w.i32(v);
                }
                w.bool(a.world_scaled);
                w.bool(i.popup_on_screen);
                w.u32(i.detached.len() as u32);
                for d in &i.detached {
                    w.u32(d.view);
                    w.u32(d.serial as u32);
                    w.u32((d.serial >> 32) as u32);
                    for v in [d.anchor.world.0, d.anchor.world.1, d.anchor.offset.0, d.anchor.offset.1, d.click.0, d.click.1] {
                        w.i32(v);
                    }
                    w.bool(d.anchor.world_scaled);
                }
                w.bool(i.targeting);
                w.bool(i.snap_to_tiles);
            }
            Update::Interactables(items) => {
                w.u8(12);
                w.u32(items.len() as u32);
                for i in items {
                    for v in i.rect {
                        w.i32(v);
                    }
                    // Bit 0: an anchor follows; bit 1: Alt left click is right.
                    w.u8(u8::from(i.anchor.is_some()) | u8::from(i.alt_right_click) << 1);
                    for v in i.anchor.unwrap_or_default() {
                        w.i32(v);
                    }
                    w.u32(i.id as u32);
                    w.u32((i.id >> 32) as u32);
                    let (tag, scroll, step) = match i.kind {
                        InteractableKind::Button => (0, [0; 4], 0),
                        InteractableKind::ListItem => (1, [0; 4], 0),
                        InteractableKind::List { scroll, step } => (2, scroll, step),
                        InteractableKind::Slider { track, value } => (3, track, value),
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
            Update::ScreenSize((width, height)) => {
                w.u8(16);
                w.u32(*width);
                w.u32(*height);
            }
            Update::Host(h) => {
                w.u8(23);
                w.u32(h.stale_ms);
                w.u32(h.serial);
            }
            Update::Looks(looks) => {
                w.u8(17);
                w.u32(looks.len() as u32);
                for l in looks {
                    w.u32(l.id as u32);
                    w.u32((l.id >> 32) as u32);
                    let (tag, group) = match l.select {
                        Select::None => (0, 0),
                        Select::Toggle => (1, 0),
                        Select::Pick { group } => (2, group),
                    };
                    w.u8(tag);
                    w.u32(group as u32);
                    w.u32((group >> 32) as u32);
                    w.look_state(l.shown);
                    w.u32(l.views.len() as u32);
                    for v in &l.views {
                        w.u32(v.view);
                        w.u32(v.sprites.0);
                        w.u32(v.sprites.1);
                        w.u32(v.states.len() as u32);
                        for (state, sprites) in &v.states {
                            w.look_state(*state);
                            w.sprites(sprites);
                        }
                    }
                }
            }
        }
    }
    w.0
}

/// Bytes of one encoded [`Interactable`].
const INTERACTABLE_BYTES: usize = 16 + 1 + 8 + 8 + 1 + 16 + 4;
/// One `DetachedPopup`: view, serial, anchor and click, `world_scaled`.
const DETACHED_BYTES: usize = 4 + 8 + 6 * 4 + 1;

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
                    layers.push(r.layer_cells()?);
                }
                Update::World { shape, layers }
            }
            20 => {
                let id = r.u32()?;
                let slot = if r.bool()? { Some((r.u32()?, r.layer_cells()?)) } else { None };
                Update::Slot { id, slot }
            }
            21 => {
                let n = r.len(12)?;
                Update::SlotCells((0..n).map(|_| Ok((r.u32()?, r.u32()?, r.u32()?))).collect::<Result<_, WireError>>()?)
            }
            22 => {
                let n = r.len(8)?;
                let mut bindings = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = r.u32()?;
                    let m = r.len(8)?;
                    let slots = (0..m).map(|_| Ok((r.u32()?, r.u32()?))).collect::<Result<_, WireError>>()?;
                    bindings.push(Binding { id, slots });
                }
                Update::Bindings(bindings)
            }
            1 => {
                let n = r.len(12)?;
                Update::Cells((0..n).map(|_| Ok((r.u32()?, r.u32()?, r.u32()?))).collect::<Result<_, WireError>>()?)
            }
            27 => {
                let n = r.len(20)?;
                let mut lifetimes = Vec::with_capacity(n);
                for _ in 0..n {
                    let serial = u64::from(r.u32()?) | (u64::from(r.u32()?) << 32);
                    lifetimes.push(crate::SpriteLifetime { serial, sprites: (r.u32()?, r.u32()?), duration_ms: r.u32()? });
                }
                Update::SpriteLifetimes(lifetimes)
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
            18 => {
                let some = r.bool()?;
                let serial = r.u32()? as u64 | (r.u32()? as u64) << 32;
                let center = (r.i32()?, r.i32()?);
                Update::CameraFocus(some.then_some(CameraFocus { serial, center }))
            }
            25 => Update::CameraArrival(if r.bool()? {
                Some(crate::CameraArrival { serial: r.u32()? as u64 | (r.u32()? as u64) << 32, bounds: [r.i32()?, r.i32()?, r.i32()?, r.i32()?] })
            } else { None }),
            19 => Update::MapWheelStep((r.i32()?, r.i32()?)),
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
                let menu_fit = r.bool()?;
                let mut view = || -> Result<Option<u32>, WireError> {
                    let some = r.bool()?;
                    let id = r.u32()?;
                    Ok(some.then_some(id))
                };
                let (roaming, popup, screen_view, capture_hint) = (view()?, view()?, view()?, view()?);
                let modal_stack = crate::ModalStack([view()?, view()?, view()?, view()?]);
                let modal_attached = [r.bool()?, r.bool()?, r.bool()?, r.bool()?];
                let mut rect = || -> Result<Option<[i32; 4]>, WireError> {
                    let some = r.bool()?;
                    let rect = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                    Ok(some.then_some(rect))
                };
                let (reachable, focus_control) = (rect()?, rect()?);
                let beside = r.bool()?;
                let screen_rect = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                let edge = match r.u8()? {
                    0 => Edge::Left,
                    1 => Edge::Right,
                    2 => Edge::Top,
                    3 => Edge::Bottom,
                    t => return Err(WireError::Tag(t)),
                };
                let world = {
                    let some = r.bool()?;
                    let rect = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                    some.then_some(rect)
                };
                let popup_anchor = {
                    let some = r.bool()?;
                    let v = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                    let world_scaled = r.bool()?;
                    some.then_some(PopupAnchor { world: (v[0], v[1]), offset: (v[2], v[3]), world_scaled })
                };
                let popup_on_screen = r.bool()?;
                let detached = (0..r.len(DETACHED_BYTES)?).map(|_| -> Result<DetachedPopup, WireError> {
                    let view = r.u32()?;
                    let serial = r.u32()? as u64 | (r.u32()? as u64) << 32;
                    let v = [r.i32()?, r.i32()?, r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                    let world_scaled = r.bool()?;
                    Ok(DetachedPopup { view, serial, anchor: PopupAnchor { world: (v[0], v[1]), offset: (v[2], v[3]), world_scaled }, click: (v[4], v[5]) })
                }).collect::<Result<Vec<_>, _>>()?;
                let targeting = r.bool()?;
                let snap_to_tiles = r.bool()?;
                Update::Interface(Interface {
                    screen: named.then_some(screen),
                    menu_fit,
                    roaming,
                    capture_hint,
                    popup,
                    modal_stack,
                    modal_attached,
                    screen_view,
                    reachable,
                    focus_control,
                    beside: beside.then_some(Beside { screen: screen_rect, edge, world }),
                    popup_anchor,
                    popup_on_screen,
                    detached,
                    targeting,
                    snap_to_tiles,
                })
            }
            12 => {
                let n = r.len(INTERACTABLE_BYTES)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    let rect = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                    let flags = r.u8()?;
                    if flags > 3 {
                        return Err(WireError::Tag(flags));
                    }
                    let (has_anchor, alt_right_click) = (flags & 1 != 0, flags & 2 != 0);
                    let point = [r.i32()?, r.i32()?];
                    let anchor = has_anchor.then_some(point);
                    let id = r.u32()? as u64 | (r.u32()? as u64) << 32;
                    let tag = r.u8()?;
                    let scroll = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                    let step = r.i32()?;
                    let kind = match tag {
                        0 => InteractableKind::Button,
                        1 => InteractableKind::ListItem,
                        2 => InteractableKind::List { scroll, step },
                        3 => InteractableKind::Slider { track: scroll, value: step },
                        t => return Err(WireError::Tag(t)),
                    };
                    items.push(Interactable { alt_right_click, anchor, rect, kind, id });
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
            16 => Update::ScreenSize((r.u32()?, r.u32()?)),
            24 => Update::SettingsPreview(if r.bool()? {
                Some(crate::SettingsPreview {
                    session: r.u32()? as u64 | (r.u32()? as u64) << 32,
                    id: r.u32()? as u64 | (r.u32()? as u64) << 32,
                    pixel_scale: r.u32()?, previous_pixel_scale: r.u32()?,
                })
            } else { None }),
            23 => Update::Host(crate::HostStatus { stale_ms: r.u32()?, serial: r.u32()? }),
            17 => {
                let n = r.len(22)?;
                let mut looks = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = r.u32()? as u64 | (r.u32()? as u64) << 32;
                    let tag = r.u8()?;
                    let group = r.u32()? as u64 | (r.u32()? as u64) << 32;
                    let select = match tag {
                        0 => Select::None,
                        1 => Select::Toggle,
                        2 => Select::Pick { group },
                        t => return Err(WireError::Tag(t)),
                    };
                    let shown = r.look_state()?;
                    let m = r.len(16)?;
                    let mut views = Vec::with_capacity(m);
                    for _ in 0..m {
                        let view = r.u32()?;
                        let sprites = (r.u32()?, r.u32()?);
                        let k = r.len(5)?;
                        let states = (0..k).map(|_| Ok((r.look_state()?, r.sprites()?))).collect::<Result<_, WireError>>()?;
                        views.push(ViewLooks { view, sprites, states });
                    }
                    looks.push(Looks { id, select, shown, views });
                }
                Update::Looks(looks)
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
            Input::Slider { id, action, value } => {
                w.u8(16);
                w.u32(id as u32);
                w.u32((id >> 32) as u32);
                w.u8(match action {
                    SliderAction::Begin => 0,
                    SliderAction::Preview => 1,
                    SliderAction::Commit => 2,
                    SliderAction::Cancel => 3,
                    SliderAction::Step => 4,
                });
                w.i32(value);
            }
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
            Input::CameraArrived { serial } | Input::CameraUnavailable { serial } => {
                w.u8(if matches!(e, Input::CameraArrived { .. }) { 19 } else { 20 });
                w.u32(serial as u32); w.u32((serial >> 32) as u32);
            }
            Input::SettingsPreviewDecision { session, id, keep } => {
                w.u8(18);
                for n in [session, id] { w.u32(n as u32); w.u32((n >> 32) as u32); }
                w.bool(keep);
            }
            Input::DismissPopup => w.u8(17),
            Input::CloseDetached { serial } => {
                w.u8(21);
                w.u32(serial as u32);
                w.u32((serial >> 32) as u32);
            }
            Input::WorldMove { x, y } => {
                w.u8(14);
                w.i32(x);
                w.i32(y);
            }
            Input::UnitOrder { action, x, y } => {
                w.u8(15);
                w.u8(match action {
                    UnitOrderAction::Preview => 0,
                    UnitOrderAction::Confirm => 1,
                    UnitOrderAction::Cancel => 2,
                });
                w.i32(x);
                w.i32(y);
            }
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
            Input::Window { width, height, scale_milli } => {
                w.u8(13);
                w.u32(width);
                w.u32(height);
                w.u32(scale_milli);
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
            16 => {
                let id = r.u32()? as u64 | (r.u32()? as u64) << 32;
                let action = match r.u8()? {
                    0 => SliderAction::Begin,
                    1 => SliderAction::Preview,
                    2 => SliderAction::Commit,
                    3 => SliderAction::Cancel,
                    4 => SliderAction::Step,
                    tag => return Err(WireError::Tag(tag)),
                };
                Input::Slider { id, action, value: r.i32()? }
            }
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
            19 => Input::CameraArrived { serial: r.u32()? as u64 | (r.u32()? as u64) << 32 },
            20 => Input::CameraUnavailable { serial: r.u32()? as u64 | (r.u32()? as u64) << 32 },
            18 => Input::SettingsPreviewDecision {
                session: r.u32()? as u64 | (r.u32()? as u64) << 32,
                id: r.u32()? as u64 | (r.u32()? as u64) << 32, keep: r.bool()?,
            },
            17 => Input::DismissPopup,
            21 => Input::CloseDetached { serial: r.u32()? as u64 | (r.u32()? as u64) << 32 },
            6 => Input::Look { x: r.i32()?, y: r.i32()? },
            14 => Input::WorldMove { x: r.i32()?, y: r.i32()? },
            15 => {
                let action = match r.u8()? {
                    0 => UnitOrderAction::Preview,
                    1 => UnitOrderAction::Confirm,
                    2 => UnitOrderAction::Cancel,
                    tag => return Err(WireError::Tag(tag)),
                };
                Input::UnitOrder { action, x: r.i32()?, y: r.i32()? }
            }
            11 => Input::Edge { x: r.i32()?, y: r.i32()? },
            12 => Input::MainView { x: r.i32()?, y: r.i32()?, w: r.i32()?, h: r.i32()? },
            13 => Input::Window { width: r.u32()?, height: r.u32()?, scale_milli: r.u32()? },
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
    /// An addition to the description of the generation before (see
    /// `render_desc::Growing`; `roots` as there), not a whole description.
    pub append: bool,
    /// Index pages to make room for in the atlas (art expected soon, see
    /// `tile_viewer::Assets::reserve`; 0: none).
    pub reserve: u32,
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
    w.bool(a.append);
    w.u32(a.reserve);
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
    let append = r.bool()?;
    let reserve = r.u32()?;
    Ok(Assets { description, roots, generation, append, reserve })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scene;

    #[test]
    fn semantic_camera_repeats_and_shutdown_cross_the_wire() {
        let mut before = Scene::default();
        before.camera_focus = Some(CameraFocus { serial: 0x1_0000_0001, center: (-64, 4096) });
        let mut after = before.clone();
        after.camera_focus.as_mut().unwrap().serial += 1;
        let updates = after.updates_since(&before);
        assert_eq!(updates, vec![Update::CameraFocus(after.camera_focus)]);
        let decoded = decode_updates(&encode_updates(&updates)).unwrap();
        for update in decoded { before.apply(update); }
        assert_eq!(before.camera_focus, after.camera_focus);
        after.camera_focus = None;
        let decoded = decode_updates(&encode_updates(&after.updates_since(&before))).unwrap();
        for update in decoded { before.apply(update); }
        assert_eq!(before.camera_focus, None);
    }

    #[test]
    fn map_wheel_preferences_update_without_refocusing_the_camera() {
        let mut before = Scene::default();
        before.camera_focus = Some(CameraFocus { serial: 7, center: (3000, 2000) });
        before.map_wheel_step = (128, 64);
        let mut after = before.clone();
        after.map_wheel_step = (64, 32);
        let updates = after.updates_since(&before);
        assert_eq!(updates, vec![Update::MapWheelStep((64, 32))]);
        for update in decode_updates(&encode_updates(&updates)).unwrap() { before.apply(update); }
        assert_eq!(before.camera_focus, after.camera_focus);
        assert_eq!(before.map_wheel_step, after.map_wheel_step);
        after.map_wheel_step = (0, 0);
        let updates = after.updates_since(&before);
        for update in decode_updates(&encode_updates(&updates)).unwrap() { before.apply(update); }
        assert_eq!(before.map_wheel_step, (0, 0));
    }

    #[test]
    fn sprite_lifetimes_round_trip_diff_and_apply() {
        let mut after = crate::Scene::default();
        after.sprite_lifetimes = vec![crate::SpriteLifetime { serial: 0x1234_5678_9ABC_DEF0, sprites: (12, 20), duration_ms: 2500 }];
        let updates = after.updates_since(&crate::Scene::default());
        assert_eq!(decode_updates(&encode_updates(&updates)).unwrap(), updates);
        let mut received = crate::Scene::default();
        for update in updates { received.apply(update); }
        assert_eq!(received.sprite_lifetimes, after.sprite_lifetimes);
        let clear = crate::Scene::default().updates_since(&received);
        assert!(clear.contains(&Update::SpriteLifetimes(Vec::new())));
    }

    #[test]
    fn native_palette_blend_round_trips_in_commands_and_look_recipes() {
        let smoke = FreeSprite { set: 7, x: -11, y: 9, blend: crate::SpriteBlend::PaletteAlpha256, clip: Some([-2, 3, 40, 50]), ..Default::default() };
        let normal = FreeSprite { set: 8, ..Default::default() };
        let updates = vec![Update::Sprites(vec![normal, smoke]), Update::Looks(vec![Looks {
            id: 12, views: vec![ViewLooks { view: 4, sprites: (0, 1), states: vec![(LookState::default(), vec![smoke])] }], ..Default::default()
        }])];
        assert_eq!(decode_updates(&encode_updates(&updates)).unwrap(), updates);
    }

    #[test]
    fn updates_and_input_round_trip() {
        let mut s = Scene::empty(WorldShape { cols: 3, rows: 2, wrap_x: true, wrap_y: false }, 2);
        s.tick = 7;
        s.view_mut(0).view = View { camera_x: -5, camera_y: 12, zoom: (1, 2), viewport: (640, 480), cull: Some((1, 2, 3, 4)), origin: (3, 4), interface_only: false, menu: true, downscale: 2, hidden_categories: 0x123, binding: 9 };
        s.slots.push(crate::LayerSlot { id: 3, layer: 1, cells: LayerCells { values: vec![1, 0, 2, 0, 0, 7], phases: Some(vec![0, 1, 2, 3, 4, 5]), offsets: Some(vec![(-128, 0); 6]) } });
        s.slots.push(crate::LayerSlot { id: 8, layer: 0, cells: LayerCells { values: vec![0; 6], phases: None, offsets: None } });
        s.bindings.push(Binding { id: 9, slots: vec![(0, 8), (1, 3)] });
        s.set(1, 2, 1, 0xABCDEF);
        s.layers[0].phases = Some(vec![1, 2, 3, 4, 5, 6]);
        s.layers[1].offsets = Some(vec![(0, 0), (-300, 7), (0, 0), (5, -1), (0, 0), (0, 0)]);
        s.sprites.push(FreeSprite { set: 4, x: -3, y: 9, screen: false, after_pass: u32::MAX, phase: 2, ticks_per_frame: 3, scale_den: 16, blend: crate::SpriteBlend::Alpha, tint: 0x123456, zoom_art: 2, zoom: (3, 2), zoom_offset: (-4, 7), clip: None, not_in_view: Some(3), glyph: None, solid: None, palette: 7, category: 0x456 });
        s.sprites.push(FreeSprite { x: -4, y: 8, solid: Some(crate::SolidRect { width: 13, height: 7, rgba: [1, 2, 3, 128] }), ..Default::default() });
        s.sprites.push(FreeSprite { x: 7, y: 30, zoom_art: 2, tint: 0xFFFF00, glyph: Some(Glyph { font: 1, ch: 'é', em64: 640, advance: 6, bold: true, italic: false, key: Some([255, 0, 255]), bar: None }), ..Default::default() });
        s.sprites.push(FreeSprite { x: 13, y: 30, zoom_art: 2, glyph: Some(Glyph { font: 0, ch: ' ', em64: 768, advance: 4, bold: false, italic: true, key: None, bar: Some((1, 2)) }), ..Default::default() });
        s.view_mut(2).overlay.push(FreeSprite { set: 5, x: 1, y: 2, screen: true, after_pass: 0, phase: 0, ticks_per_frame: 1, scale_den: 0, blend: crate::SpriteBlend::Alpha, tint: 0, zoom_art: 0, zoom: (0, 0), zoom_offset: (0, 0), clip: Some([1, -2, 300, 40]), not_in_view: None, glyph: None, solid: None, palette: 0, category: 0x789 });
        s.patches.push(Patch { sprite: "Screen@x/0".into(), width: 1, height: 2, rgba: vec![1, 2, 3, 4, 5, 6, 7, 8] });
        s.focus = Some((-64, 96));
        s.camera_focus = Some(CameraFocus { serial: 0x1_0000_0002, center: (-4096, 1234) });
        s.assets = 3;
        s.zooms = vec![(3, 2)];
        let mut settings = viewer_settings::Settings::default();
        settings.resolution.pixel_scale = 3;
        settings.layout.split = viewer_settings::Split::Horizontal1To1;
        settings.layout.screen_place = viewer_settings::ScreenPlace { vertical: viewer_settings::Place::RightOrTop, horizontal: viewer_settings::Place::LeftOrBottom };
        settings.input.snap_updates_per_second = 4.5;
        s.settings = Some(settings);
        s.interface = Interface {
            screen: Some("F4".into()),
            menu_fit: true,
            roaming: Some(256),
            capture_hint: Some(259),
            popup: Some(261),
            modal_stack: crate::ModalStack([Some(257), Some(259), Some(260), Some(261)]),
            modal_attached: [false, false, true, false],
            screen_view: Some(258),
            reachable: Some([-1, 2, 300, 40]),
            focus_control: Some([5, 6, 7, 8]),
            beside: Some(Beside { screen: [448, 156, 1024, 768], edge: Edge::Bottom, world: Some([-64, 32, 1280, 640]) }),
            popup_anchor: Some(PopupAnchor { world: (-3, 70_000), offset: (-128, 1), world_scaled: true }),
            popup_on_screen: true,
            detached: vec![
                DetachedPopup { view: 0xC9_5314, serial: 0x1_0000_0007, anchor: PopupAnchor { world: (-64, 9), offset: (0, 0), world_scaled: true }, click: (-40, 30) },
                DetachedPopup { view: 300, serial: 8, anchor: PopupAnchor { world: (5, 6), offset: (1, 1), world_scaled: false }, click: (7, 8) },
            ],
            targeting: true,
            snap_to_tiles: true,
        };
        s.interactables = vec![
            Interactable { alt_right_click: false, anchor: None, rect: [10, 20, 100, 24], kind: InteractableKind::Button, id: 0x1_0000_0002 },
            Interactable { alt_right_click: false, anchor: None, rect: [5, 50, 200, 300], kind: InteractableKind::List { scroll: [5, 60, 190, 280], step: 20 }, id: 7 },
            Interactable { alt_right_click: true, anchor: Some([193, 69]), rect: [5, 60, 190, 20], kind: InteractableKind::ListItem, id: 8 },
            Interactable { alt_right_click: false, anchor: None, rect: [10, 20, 200, 24], kind: InteractableKind::Slider { track: [20, 32, 180, 1], value: 123_456 }, id: 9 },
        ];
        s.screen_size = (1920, 1080);
        let hover = LookState { pointer: PointerState::Hover, selected: false };
        let picked = LookState { pointer: PointerState::Pressed, selected: true };
        s.looks = vec![
            Looks {
                id: 0x1_0000_0002,
                select: Select::None,
                shown: LookState::default(),
                views: vec![ViewLooks { view: 2, sprites: (0, 1), states: vec![(hover, vec![FreeSprite { set: 9, x: 10, y: 20, screen: true, ..Default::default() }]), (picked, vec![])] }],
            },
            Looks { id: 8, select: Select::Pick { group: 0x7_0000_0000 }, shown: picked, views: vec![] },
            Looks { id: 9, select: Select::Toggle, shown: hover, views: vec![ViewLooks { view: 0, sprites: (3, 3), states: vec![] }] },
        ];
        let mut updates = s.updates_since(&Scene::default());
        assert!(updates.iter().any(|u| matches!(u, Update::Settings(_))));
        updates.push(Update::Cells(vec![(1, 0, 9)]));
        updates.push(Update::Phases(vec![(0, 5, 200)]));
        updates.push(Update::Offsets(vec![(1, 2, (-1, 32767))]));
        updates.push(Update::SlotCells(vec![(3, 5, 0xFFFF_FFFF), (8, 0, 1)]));
        updates.push(Update::Slot { id: 3, slot: None });
        updates.push(Update::Bindings(Vec::new()));
        updates.push(Update::View { id: 2, view: None });
        updates.push(Update::Interface(Interface { popup: Some(7), ..Default::default() }));
        updates.push(Update::Cursor(Cursor::Sprite(42)));
        updates.push(Update::Cursor(Cursor::System(SystemCursor::ResizeNesw)));
        updates.push(Update::Cursor(Cursor::Default));
        updates.push(Update::Host(crate::HostStatus { stale_ms: 3_250, serial: 77 }));
        updates.push(Update::SettingsPreview(Some(crate::SettingsPreview { session: 0x1_0000_0001, id: 0x2_0000_0002, pixel_scale: 3, previous_pixel_scale: 1 })));
        updates.push(Update::SettingsPreview(None));
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
            Input::Slider { id: 0xfedc_ba98_7654_3210, action: SliderAction::Begin, value: 123_456 },
            Input::Slider { id: 2, action: SliderAction::Preview, value: 1_000_000 },
            Input::Slider { id: 2, action: SliderAction::Commit, value: 1_000_000 },
            Input::Slider { id: 2, action: SliderAction::Cancel, value: -1 },
            Input::Slider { id: 2, action: SliderAction::Step, value: -10 },
            Input::MouseMove { x: 1, y: -2 },
            Input::MouseButton { button: Button::Right, down: true, x: 3, y: 4 },
            Input::Wheel { delta: -120, x: 0, y: 0 },
            Input::Key { vk: 0x41, down: false, text: Some('a') },
            Input::Key { vk: 0x10, down: true, text: None },
            Input::Resized { width: 800, height: 600 },
            Input::Quit,
            Input::Look { x: -5, y: 7000 },
            Input::DismissPopup,
            Input::CloseDetached { serial: 0x2_0000_0009 },
            Input::SettingsPreviewDecision { session: 0x3_0000_0003, id: 0x4_0000_0004, keep: false },
            Input::WorldMove { x: -64, y: 12345 },
            Input::UnitOrder { action: UnitOrderAction::Preview, x: -100, y: 9999 },
            Input::UnitOrder { action: UnitOrderAction::Confirm, x: 1234, y: -555 },
            Input::UnitOrder { action: UnitOrderAction::Cancel, x: 0, y: 0 },
            Input::Zooms([(3, 2), (2, 1), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)]),
            Input::Views([(10, -20, 300, 200), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)]),
            Input::MainView { x: 5, y: -6, w: 700, h: 400 },
            Input::WorldButton { button: Button::Right, down: true, x: -64, y: 4096 },
            Input::WorldButton { button: Button::Middle, down: false, x: 0, y: 0 },
            Input::WorldKey { vk: 0x66, down: true, text: Some('6'), x: 130, y: -2 },
            Input::WorldKey { vk: 0x25, down: false, text: None, x: 1, y: 1 },
            Input::Edge { x: 16, y: -1 },
            Input::Edge { x: 0, y: 0 },
            Input::Window { width: 3840, height: 2160, scale_milli: 1500 },
        ];
        assert_eq!(decode_input(&encode_input(&events)).unwrap(), events);
        let a = Assets { description: "[grid]\n".into(), roots: vec!["C:\\Games".into(), "Z:\\x".into()], generation: 7, append: true, reserve: 300 };
        assert_eq!(decode_assets(&encode_assets(&a)).unwrap(), a);
        assert_eq!(decode_updates(&[VERSION as u8, 0, 0, 0, 5]), Err(WireError::Truncated));
        assert_eq!(decode_input(&[0xFE, 0, 0, 0]), Err(WireError::Version(0xFE)));
    }

    #[test]
    fn all_message_types_reject_a_mismatched_runtime_before_decoding_payloads() {
        // A new sprite blend must never be interpreted by an older paired
        // runtime. Reject the epoch even for otherwise valid empty messages.
        for version in [VERSION - 1, VERSION + 1] {
            let replace_version = |mut bytes: Vec<u8>| {
                bytes[..4].copy_from_slice(&version.to_le_bytes());
                bytes
            };
            assert_eq!(decode_updates(&replace_version(encode_updates(&[]))), Err(WireError::Version(version)));
            assert_eq!(decode_input(&replace_version(encode_input(&[]))), Err(WireError::Version(version)));
            assert_eq!(decode_assets(&replace_version(encode_assets(&Assets::default()))), Err(WireError::Version(version)));
        }
    }

    #[test]
    fn menu_fit_roundtrips_and_rejects_the_previous_protocol() {
        let updates = [Update::Interface(Interface { menu_fit: true, ..Default::default() }),
            Update::Interface(Interface::default())];
        let mut bytes = encode_updates(&updates);
        assert_eq!(decode_updates(&bytes).unwrap(), updates);
        bytes[..4].copy_from_slice(&38u32.to_le_bytes());
        assert_eq!(decode_updates(&bytes), Err(WireError::Version(38)));
    }

    #[test]
    fn minimal_control_looks_roundtrip() {
        let hidden = Looks { id: 1, ..Default::default() };
        let empty_state = Looks {
            id: 2,
            views: vec![ViewLooks {
                states: vec![(LookState::default(), Vec::new())],
                ..Default::default()
            }],
            ..Default::default()
        };
        for look in [hidden, empty_state] {
            let updates = vec![Update::Looks(vec![look])];
            assert_eq!(decode_updates(&encode_updates(&updates)).unwrap(), updates);
        }
    }

}

#[cfg(test)]
mod camera_arrival_tests {
    use super::*;
    #[test]
    fn combat_area_and_receipts_round_trip_without_losing_serial_or_bounds() {
        let request = crate::CameraArrival { serial: 0x1_0000_0009, bounds: [-64, 32, 1024, 777] };
        let updates = vec![Update::CameraArrival(Some(request)), Update::CameraArrival(None)];
        assert_eq!(decode_updates(&encode_updates(&updates)).unwrap(), updates);
        let inputs = vec![Input::CameraArrived { serial: request.serial }, Input::CameraUnavailable { serial: request.serial + 1 }];
        assert_eq!(decode_input(&encode_input(&inputs)).unwrap(), inputs);
        let before = crate::Scene::default();
        let mut after = before.clone(); after.camera_arrival = Some(request);
        assert_eq!(after.updates_since(&before), vec![Update::CameraArrival(Some(request))]);
    }
}
