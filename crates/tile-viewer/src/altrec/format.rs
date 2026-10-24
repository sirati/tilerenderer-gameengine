//! The recording's file format (`docs/alt-mode-recording.md`): a header,
//! then length-prefixed records, little-endian, with varints for counts,
//! indices and time steps and a compact form for coordinates.
//!
//! Header: the magic `C3RALTRC`, `u16` version, `u16` length of the rest
//! of the header (20 in version 1: `u64` wall clock at the session's
//! start in Unix microseconds, `u64` the time base of the file's first
//! record on the viewer's clock in microseconds, `u32` part number).
//! Record: `u8` type, varint payload length, payload. A payload starts
//! with a varint: microseconds since the previous record of the file (the
//! first: since the header's base). A reader skips types it does not know.

use crate::snap::{Target, TargetKind};
use scene::Button;

use super::{ClickUse, ModeChange, PointerKind, Reason, Record};

pub const MAGIC: [u8; 8] = *b"C3RALTRC";
pub const VERSION: u16 = 1;
/// The header's length after magic, version and this length.
const HEADER_REST: u16 = 20;
pub const HEADER_LEN: usize = 8 + 2 + 2 + HEADER_REST as usize;

pub const RATE: u8 = 1;
pub const SNAPSHOT: u8 = 2;
pub const MOTION: u8 = 3;
pub const POINTER: u8 = 4;
pub const CLICK: u8 = 5;
pub const MODE: u8 = 6;
pub const GAVE_UP: u8 = 7;
pub const DROPPED: u8 = 8;
pub const END: u8 = 9;

const KIND_BUTTON: u8 = 0;
const KIND_SLIDER: u8 = 1;
const KIND_LIST_ITEM: u8 = 2;
const KIND_LIST: u8 = 3;
const KIND_CAPTURE: u8 = 4;

const SPREAD: u8 = 1;
const AREA: u8 = 2;
const ANCHOR: u8 = 4;
/// A list entry lies in a list; a list has entries.
const IN_LIST: u8 = 8;
const HAS_ITEMS: u8 = 16;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// When the session started, Unix microseconds.
    pub wall_us: u64,
    /// The time the file's first record counts from (viewer clock, µs).
    pub base_us: u64,
    /// The file's number in its session, from 0.
    pub part: u32,
}

impl Header {
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&HEADER_REST.to_le_bytes());
        out.extend_from_slice(&self.wall_us.to_le_bytes());
        out.extend_from_slice(&self.base_us.to_le_bytes());
        out.extend_from_slice(&self.part.to_le_bytes());
    }
}

pub fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(v: u64) -> i64 {
    (v >> 1) as i64 ^ -((v & 1) as i64)
}

/// A coordinate: a whole number of 1/256 pixels as a zigzag varint shifted
/// left by one (low bit 0), else the low bit 1 and the `f64` itself.
/// Exact either way.
pub fn put_coord(out: &mut Vec<u8>, v: f64) {
    let k = v * 256.0;
    if k.is_finite() && k.abs() < (1u64 << 52) as f64 {
        let whole = k as i64;
        if (whole as f64 / 256.0).to_bits() == v.to_bits() {
            put_varint(out, zigzag(whole) << 1);
            return;
        }
    }
    out.push(1);
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_point(out: &mut Vec<u8>, (x, y): (f64, f64)) {
    put_coord(out, x);
    put_coord(out, y);
}

fn put_rect(out: &mut Vec<u8>, r: [f64; 4]) {
    r.iter().for_each(|&v| put_coord(out, v));
}

fn put_target(out: &mut Vec<u8>, index: Option<u32>) {
    put_varint(out, index.map_or(0, |k| k as u64 + 1));
}

fn button_code(b: Button) -> u8 {
    match b {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
    }
}

fn button_of(code: u8) -> Result<Button, String> {
    Ok(match code {
        0 => Button::Left,
        1 => Button::Right,
        2 => Button::Middle,
        _ => return Err(format!("button {code}")),
    })
}

/// Encodes records, each as a step in time from the one before.
#[derive(Debug, Default)]
pub struct Encoder {
    last: u64,
    payload: Vec<u8>,
}

impl Encoder {
    /// Records from `base_us` on (the header's base).
    pub fn new(base_us: u64) -> Self {
        Self { last: base_us, payload: Vec::new() }
    }

    pub fn record(&mut self, r: &Record, out: &mut Vec<u8>) {
        let p = &mut self.payload;
        p.clear();
        // (Time never runs back in a file: a record late into the queue
        // counts as at the one before.)
        let t = r.t().max(self.last);
        put_varint(p, t - self.last);
        self.last = t;
        let kind = match r {
            Record::Rate { updates_per_second, .. } => {
                p.extend_from_slice(&updates_per_second.to_le_bytes());
                RATE
            }
            Record::Snapshot { number, targets, .. } => {
                put_varint(p, *number as u64);
                put_varint(p, targets.len() as u64);
                for t in targets {
                    put_snapshot_target(p, t);
                }
                SNAPSHOT
            }
            Record::Motion { d, .. } => {
                put_point(p, *d);
                MOTION
            }
            Record::Pointer { kind, at, snapshot, target, .. } => {
                p.push(match kind {
                    PointerKind::Rest => 0,
                    PointerKind::Origin => 1,
                });
                put_point(p, *at);
                put_varint(p, *snapshot as u64);
                put_target(p, *target);
                POINTER
            }
            Record::Click { button, down, used, snapshot, target, .. } => {
                let used = match used {
                    ClickUse::Pointer => 0,
                    ClickUse::SliderBegin => 1,
                    ClickUse::SliderEnd => 2,
                    ClickUse::Swallowed => 3,
                };
                p.push(button_code(*button) | (*down as u8) << 2 | used << 3);
                put_varint(p, *snapshot as u64);
                put_target(p, *target);
                CLICK
            }
            Record::Mode { change, reason, at, snapshot, .. } => {
                p.push(match change {
                    ModeChange::On => 0,
                    ModeChange::Land => 1,
                    ModeChange::Stop => 2,
                });
                p.push(Reason::ALL.iter().position(|r| r == reason).unwrap_or(0) as u8);
                if *change == ModeChange::On {
                    put_point(p, *at);
                    put_varint(p, *snapshot as u64);
                }
                MODE
            }
            Record::GaveUp { since_exit_us, snapshot, target, at, button, .. } => {
                put_varint(p, *since_exit_us);
                put_varint(p, *snapshot as u64);
                put_varint(p, *target as u64);
                put_point(p, *at);
                p.push(button_code(*button));
                GAVE_UP
            }
            Record::Dropped { count, .. } => {
                put_varint(p, *count);
                DROPPED
            }
            Record::End { .. } => END,
        };
        out.push(kind);
        put_varint(out, p.len() as u64);
        out.extend_from_slice(p);
    }
}

fn put_snapshot_target(p: &mut Vec<u8>, t: &Target) {
    let (kind, extra) = match t.kind {
        TargetKind::Button => (KIND_BUTTON, 0),
        TargetKind::Slider { .. } => (KIND_SLIDER, 0),
        TargetKind::ListItem { list } => (KIND_LIST_ITEM, if list.is_some() { IN_LIST } else { 0 }),
        TargetKind::List { has_items, .. } => (KIND_LIST, if has_items { HAS_ITEMS } else { 0 }),
        TargetKind::Capture { .. } => (KIND_CAPTURE, 0),
    };
    let flags = extra | if t.spread { SPREAD } else { 0 } | if t.area != t.rect { AREA } else { 0 } | if t.anchor.is_some() { ANCHOR } else { 0 };
    put_varint(p, t.id);
    p.push(kind);
    p.push(flags);
    put_rect(p, t.rect);
    if t.area != t.rect {
        put_rect(p, t.area);
    }
    if let Some(a) = t.anchor {
        put_point(p, a);
    }
    match t.kind {
        TargetKind::Slider { track, value } => {
            put_rect(p, track);
            put_varint(p, zigzag(value as i64));
        }
        TargetKind::ListItem { list: Some(k) } => put_varint(p, k as u64),
        TargetKind::List { scroll, step, .. } => {
            put_rect(p, scroll);
            put_coord(p, step);
        }
        TargetKind::Capture { at } => put_point(p, at),
        TargetKind::Button | TargetKind::ListItem { list: None } => {}
    }
}

/// Reads bytes in order.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len()).ok_or("cut short")?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn varint(&mut self) -> Result<u64, String> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            v |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("varint too long".into())
    }

    fn small(&mut self) -> Result<u32, String> {
        u32::try_from(self.varint()?).map_err(|_| "number too large".to_string())
    }

    fn coord(&mut self) -> Result<f64, String> {
        let v = self.varint()?;
        if v == 1 {
            return self.f64();
        }
        if v & 1 != 0 {
            return Err("bad coordinate".into());
        }
        Ok(unzigzag(v >> 1) as f64 / 256.0)
    }

    fn point(&mut self) -> Result<(f64, f64), String> {
        Ok((self.coord()?, self.coord()?))
    }

    fn rect(&mut self) -> Result<[f64; 4], String> {
        Ok([self.coord()?, self.coord()?, self.coord()?, self.coord()?])
    }

    fn target(&mut self) -> Result<Option<u32>, String> {
        Ok(self.small()?.checked_sub(1))
    }
}

/// A file read: its header, its records, and why reading stopped early
/// (the file cut short, e.g. the viewer killed while writing).
#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub header: Header,
    pub records: Vec<Record>,
    pub cut: Option<String>,
}

/// Reads a recording's file.
pub fn decode(bytes: &[u8]) -> Result<File, String> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(8).map_err(|_| "not a recording")? != MAGIC {
        return Err("not a recording (no C3RALTRC header)".into());
    }
    let version = r.u16()?;
    if version != VERSION {
        return Err(format!("recording version {version}, this reads {VERSION}"));
    }
    let rest = r.u16()? as usize;
    let start = r.at;
    let header = Header { wall_us: r.u64()?, base_us: r.u64()?, part: r.u32()? };
    r.at = start + rest.max(HEADER_REST as usize);
    let mut records = Vec::new();
    let mut t = header.base_us;
    let mut cut = None;
    while r.at < bytes.len() {
        let read = (|| -> Result<Option<Record>, String> {
            let kind = r.u8()?;
            let len = r.varint()? as usize;
            let payload = r.take(len)?;
            let mut p = Reader { bytes: payload, at: 0 };
            t += p.varint()?;
            decode_payload(kind, t, &mut p)
        })();
        match read {
            Ok(Some(record)) => records.push(record),
            Ok(None) => {}
            Err(e) => {
                cut = Some(format!("at byte {}: {e}", r.at));
                break;
            }
        }
    }
    Ok(File { header, records, cut })
}

fn decode_payload(kind: u8, t: u64, p: &mut Reader) -> Result<Option<Record>, String> {
    Ok(Some(match kind {
        RATE => Record::Rate { t, updates_per_second: p.f64()? },
        SNAPSHOT => {
            let number = p.small()?;
            let count = p.varint()? as usize;
            let mut targets = Vec::with_capacity(count.min(4096));
            for _ in 0..count {
                targets.push(snapshot_target(p)?);
            }
            Record::Snapshot { t, number, targets }
        }
        MOTION => Record::Motion { t, d: p.point()? },
        POINTER => {
            let kind = match p.u8()? {
                0 => PointerKind::Rest,
                1 => PointerKind::Origin,
                k => return Err(format!("pointer kind {k}")),
            };
            Record::Pointer { t, kind, at: p.point()?, snapshot: p.small()?, target: p.target()? }
        }
        CLICK => {
            let bits = p.u8()?;
            let used = match bits >> 3 {
                0 => ClickUse::Pointer,
                1 => ClickUse::SliderBegin,
                2 => ClickUse::SliderEnd,
                3 => ClickUse::Swallowed,
                u => return Err(format!("click use {u}")),
            };
            Record::Click { t, button: button_of(bits & 3)?, down: bits & 4 != 0, used, snapshot: p.small()?, target: p.target()? }
        }
        MODE => {
            let change = match p.u8()? {
                0 => ModeChange::On,
                1 => ModeChange::Land,
                2 => ModeChange::Stop,
                c => return Err(format!("mode change {c}")),
            };
            let reason = Reason::ALL.get(p.u8()? as usize).copied().unwrap_or(Reason::Unknown);
            let (at, snapshot) = if change == ModeChange::On { (p.point()?, p.small()?) } else { ((0.0, 0.0), 0) };
            Record::Mode { t, change, reason, at, snapshot }
        }
        GAVE_UP => Record::GaveUp { t, since_exit_us: p.varint()?, snapshot: p.small()?, target: p.small()?, at: p.point()?, button: button_of(p.u8()?)? },
        DROPPED => Record::Dropped { t, count: p.varint()? },
        END => Record::End { t },
        _ => return Ok(None),
    }))
}

fn snapshot_target(p: &mut Reader) -> Result<Target, String> {
    let id = p.varint()?;
    let kind = p.u8()?;
    let flags = p.u8()?;
    let rect = p.rect()?;
    let area = if flags & AREA != 0 { p.rect()? } else { rect };
    let anchor = if flags & ANCHOR != 0 { Some(p.point()?) } else { None };
    let kind = match kind {
        KIND_BUTTON => TargetKind::Button,
        KIND_SLIDER => TargetKind::Slider { track: p.rect()?, value: unzigzag(p.varint()?) as i32 },
        KIND_LIST_ITEM => TargetKind::ListItem { list: if flags & IN_LIST != 0 { Some(p.varint()? as usize) } else { None } },
        KIND_LIST => TargetKind::List { scroll: p.rect()?, step: p.coord()?, has_items: flags & HAS_ITEMS != 0 },
        KIND_CAPTURE => TargetKind::Capture { at: p.point()? },
        k => return Err(format!("target kind {k}")),
    };
    Ok(Target { rect, area, spread: flags & SPREAD != 0, anchor, kind, id, alt_right_click: false })
}
