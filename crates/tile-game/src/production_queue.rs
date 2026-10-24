//! A site's production queue, reordered by dragging.
//!
//! The driver sends the queue where its screen shows it ([`ProductionQueue`],
//! a scene extension of kind [`QUEUE`]): each queued row, the handle it
//! is dragged by, and the box of what is produced now. The viewer
//! ([`QueueDrag`]) draws a handle (three grooved bars) before each row's
//! text, which moves right to make room. Pressed on a handle, the row is
//! picked up: a translucent copy follows the pointer, its own place is
//! dimmed, a line shows where it goes between the rows (above or below the
//! middle of the row under the pointer), and over the production box the
//! box lights up with two arrows: the row will be produced now and what is
//! produced goes to the front of the queue. Let go, the driver gets a
//! [`QueueEdit`] (kind [`QUEUE_EDIT`]); let go anywhere else, nothing
//! happens. Nothing of the press, the drag or the release reaches the
//! driver.
//!
//! A click on a handle (no motion while it was held) picks the row up
//! too, until the next click: on a row it takes that row's place, on the
//! production box it swaps, anywhere else it is put back. That is how the
//! Alt mode moves rows: the handles are interactables of their own
//! (buttons), the snapping pointer clicks one, then the target. Esc or the
//! right button puts the row back.
//!
//! The colours are the game's ([`QueueStyle`]).

use scene::{Button, Extension, Input, Scene};
use tile_render::{Blend, PlacedSprite, SolidRect, SpriteSetId};
use tile_viewer::extension::{InterfaceExtension, Rect};

/// The extension kind of a [`ProductionQueue`] in `scene::Interface::extensions`.
pub const QUEUE: u32 = u32::from_be_bytes(*b"PQue");
/// The extension kind of a [`QueueEdit`] in `scene::Input::Extension`.
pub const QUEUE_EDIT: u32 = u32::from_be_bytes(*b"PQEd");

const VK_ESCAPE: u32 = 0x1B;
/// Pixels the pointer moves while held before the press is a drag.
const DRAG: i32 = 4;

/// A production queue a renderer may let the player reorder by dragging
/// ([`QueueEdit`]): a site's queued orders where its screen lists them,
/// and what the site produces now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProductionQueue {
    /// The queue as it is now (an edit names it).
    pub stamp: u32,
    /// What is produced now (x, y, w, h), a place to drop an entry on.
    pub current: Rect,
    /// The entries shown, in queue order.
    pub rows: Vec<QueueRow>,
}

/// An entry of a [`ProductionQueue`] where it is shown.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct QueueRow {
    /// Its place in the queue (0: produced next).
    pub slot: u8,
    /// The row (x, y, w, h) and, at its left, the handle it is dragged by:
    /// also an interactable of its own, `grip_id` (a button), for pointing
    /// aids.
    pub rect: Rect,
    pub grip: Rect,
    pub grip_id: u64,
}

/// Where a picked-up row goes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Before the row of queue slot `n` (one past the last: the end).
    Before(u8),
    /// Onto the production box (produced now instead, what was produced
    /// going to the front of the queue).
    Current,
}

/// Move entry `from` of the queue as of `stamp` (a driver drops the edit
/// for another) to `to`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct QueueEdit {
    pub stamp: u32,
    pub from: u8,
    pub to: Target,
}

/// `Target::Current` on the wire.
const CURRENT: u8 = u8::MAX;
/// One row on the wire: slot, rect, grip, grip id.
const ROW_BYTES: usize = 1 + 8 * 4 + 8;

fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn rect_at(b: &[u8], at: usize) -> Option<Rect> {
    Some([i32_at(b, at)?, i32_at(b, at + 4)?, i32_at(b, at + 8)?, i32_at(b, at + 12)?])
}

impl ProductionQueue {
    /// The scene extension that sends it.
    pub fn extension(&self) -> Extension {
        let mut data = Vec::with_capacity(4 + 16 + 4 + self.rows.len() * ROW_BYTES);
        data.extend_from_slice(&self.stamp.to_le_bytes());
        for v in self.current {
            data.extend_from_slice(&v.to_le_bytes());
        }
        data.extend_from_slice(&(self.rows.len() as u32).to_le_bytes());
        for row in &self.rows {
            data.push(row.slot);
            for v in row.rect.into_iter().chain(row.grip) {
                data.extend_from_slice(&v.to_le_bytes());
            }
            data.extend_from_slice(&row.grip_id.to_le_bytes());
        }
        Extension { kind: QUEUE, data }
    }

    /// The queue of an extension's data (`None`: malformed).
    pub fn decode(data: &[u8]) -> Option<Self> {
        let stamp = u32_at(data, 0)?;
        let current = rect_at(data, 4)?;
        let n = u32_at(data, 20)? as usize;
        let rows = data.get(24..)?;
        if rows.len() != n.checked_mul(ROW_BYTES)? {
            return None;
        }
        let rows = rows
            .chunks_exact(ROW_BYTES)
            .map(|r| {
                Some(QueueRow { slot: r[0], rect: rect_at(r, 1)?, grip: rect_at(r, 17)?, grip_id: u64::from_le_bytes(r[33..41].try_into().ok()?) })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { stamp, current, rows })
    }

    /// The queue `scene` shows, if any.
    pub fn of(scene: &Scene) -> Option<Self> {
        Self::decode(scene.interface.extension(QUEUE)?)
    }
}

impl QueueEdit {
    /// The input that sends it to the driver.
    pub fn input(self) -> Input {
        let mut data = self.stamp.to_le_bytes().to_vec();
        data.push(self.from);
        data.push(match self.to {
            Target::Before(n) => n,
            Target::Current => CURRENT,
        });
        Input::Extension { kind: QUEUE_EDIT, data }
    }

    /// The edit `input` is, if it is one.
    pub fn of(input: &Input) -> Option<Self> {
        let Input::Extension { kind: QUEUE_EDIT, data } = input else { return None };
        let [a, b, c, d, from, to] = data.as_slice().try_into().ok()?;
        let to = if to == CURRENT { Target::Current } else { Target::Before(to) };
        Some(Self { stamp: u32::from_le_bytes([a, b, c, d]), from, to })
    }
}

/// The colours a game draws the handles and the drag in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct QueueStyle {
    /// The handle's bars: dark groove, light edge below.
    pub groove: [u8; 4],
    pub edge: [u8; 4],
    /// The insertion line and the production box's swap hint.
    pub line: [u8; 4],
    /// The light edges either side of the insertion line, so it reads on
    /// dark text and on the translucent copy alike.
    pub line_edge: [u8; 4],
    pub hint_fill: [u8; 4],
    pub hint_frame: [u8; 4],
    pub arrow: [u8; 4],
    /// The picked-up row's own place, and the copy's backing.
    pub dim: [u8; 4],
    pub ghost_back: [u8; 4],
    pub ghost_frame: [u8; 4],
    /// How opaque the copy's art is.
    pub ghost_opacity: u8,
}

impl Default for QueueStyle {
    /// Neutral greys, for a game that chooses none.
    fn default() -> Self {
        Self {
            groove: [70, 70, 70, 255],
            edge: [235, 235, 235, 255],
            line: [200, 30, 30, 255],
            line_edge: [250, 250, 250, 255],
            hint_fill: [255, 220, 100, 80],
            hint_frame: [210, 160, 40, 255],
            arrow: [90, 60, 20, 255],
            dim: [230, 230, 230, 170],
            ghost_back: [240, 240, 240, 110],
            ghost_frame: [110, 110, 110, 150],
            ghost_opacity: 130,
        }
    }
}

fn inside([x, y, w, h]: Rect, (px, py): (i32, i32)) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Held {
    slot: u8,
    stamp: u32,
    /// Where it was pressed (the copy keeps that point under the pointer).
    at: (i32, i32),
    /// Moved beyond [`DRAG`] while the button was down.
    dragged: bool,
    /// Picked up by a click: dropped by the next.
    sticky: bool,
}

/// The drag of a queue row (the viewer's extension for production queues).
#[derive(Clone, Debug, Default)]
pub struct QueueDrag {
    style: QueueStyle,
    held: Option<Held>,
    pointer: Option<(i32, i32)>,
    /// The release of a press that dropped (or put back) a row: swallowed.
    swallow_release: bool,
}

/// What `point` drops queue row `from` onto: the production box, or a
/// place between the rows (a drag: above or below the middle of the row
/// under it; a click: that row's own place). `None`: elsewhere, or where
/// it is already.
pub fn target(queue: &ProductionQueue, from: u8, point: (i32, i32), click: bool) -> Option<Target> {
    if inside(queue.current, point) {
        return Some(Target::Current);
    }
    let rows = &queue.rows;
    let (first, last) = (rows.first()?, rows.last()?);
    let left = rows.iter().map(|r| r.rect[0]).min()?;
    let right = rows.iter().map(|r| r.rect[0] + r.rect[2]).max()?;
    let half = first.rect[3] / 2;
    let (top, bottom) = (first.rect[1] - half, last.rect[1] + last.rect[3] + half);
    if point.0 < left || point.0 >= right || point.1 < top || point.1 >= bottom {
        return None;
    }
    let before = if click {
        let row = rows.iter().find(|r| point.1 >= r.rect[1] && point.1 < r.rect[1] + r.rect[3])?;
        if row.slot > from { row.slot + 1 } else { row.slot }
    } else {
        match rows.iter().find(|r| point.1 < r.rect[1] + r.rect[3] / 2) {
            Some(r) => r.slot,
            None => last.slot + 1,
        }
    };
    (before != from && before != from + 1).then_some(Target::Before(before))
}

impl QueueDrag {
    /// Drawn in `style`.
    pub fn new(style: QueueStyle) -> Self {
        Self { style, ..Self::default() }
    }

    /// A row is picked up (frames follow the pointer).
    pub fn active(&self) -> bool {
        self.held.is_some()
    }

    /// Lets go of a picked-up row (the window lost the focus).
    pub fn cancel(&mut self) {
        self.held = None;
    }

    /// An input on its way to the driver (its pixels): `Some` with what goes
    /// to the driver instead if it is the drag's.
    pub fn route(&mut self, input: &Input, queue: Option<&ProductionQueue>) -> Option<Vec<Input>> {
        if let Some(h) = self.held {
            if queue.is_none_or(|q| q.stamp != h.stamp) {
                // The queue changed under it (another edit, the screen closed).
                self.held = None;
            }
        }
        match *input {
            Input::MouseMove { x, y } => {
                self.pointer = Some((x, y));
                if let Some(h) = &mut self.held {
                    h.dragged |= (x - h.at.0).abs() > DRAG || (y - h.at.1).abs() > DRAG;
                }
                None
            }
            Input::MouseButton { button: Button::Left, down: true, x, y } => {
                self.pointer = Some((x, y));
                let queue = queue?;
                if let Some(h) = self.held.take() {
                    // The click that drops a row picked up by a click.
                    self.swallow_release = true;
                    return Some(drop(queue, h, (x, y), true));
                }
                let row = queue.rows.iter().find(|r| inside(r.grip, (x, y)))?;
                self.held = Some(Held { slot: row.slot, stamp: queue.stamp, at: (x, y), dragged: false, sticky: false });
                Some(vec![])
            }
            Input::MouseButton { button: Button::Left, down: false, x, y } => {
                self.pointer = Some((x, y));
                if std::mem::take(&mut self.swallow_release) {
                    return Some(vec![]);
                }
                let h = self.held?;
                if !h.dragged && !h.sticky {
                    // A click on the handle: picked up until the next click.
                    self.held = Some(Held { sticky: true, ..h });
                    return Some(vec![]);
                }
                self.held = None;
                Some(queue.map_or_else(Vec::new, |q| drop(q, h, (x, y), false)))
            }
            Input::MouseButton { button: Button::Right, down: true, .. } | Input::Key { vk: VK_ESCAPE, down: true, .. } if self.held.is_some() => {
                self.held = None;
                Some(vec![])
            }
            _ => None,
        }
    }

    /// The driver's drawing (`free`, the site screen's interface at
    /// `queue`'s rectangles) with the handles, and while a row is picked
    /// up the copy, the line and the hint. `None`: no rows shown.
    pub fn draw(&self, queue: &ProductionQueue, free: &[PlacedSprite], rect_of: &dyn Fn(&PlacedSprite) -> Option<Rect>) -> Option<Vec<PlacedSprite>> {
        let style = &self.style;
        let first = queue.rows.first()?;
        // The camera of the interface the rows are drawn in.
        let camera = free.iter().rev().filter(|s| s.screen && !s.except_view && s.view.is_some())
            .find(|s| rect_of(s).is_some_and(|r| overlap(r, first.rect)))?.view?;
        let solid = |x: i32, y: i32, w: i32, h: i32, rgba: [u8; 4]| PlacedSprite {
            screen: true,
            view: Some(camera),
            solid: Some(SolidRect { width: w.max(0) as u32, height: h.max(0) as u32, rgba }),
            ..PlacedSprite::new(SpriteSetId::from_raw(0), x, y)
        };
        let mut out = free.to_vec();
        // Each row's text, right of its handle.
        let mut row_sprites: Vec<(u8, Vec<PlacedSprite>)> = Vec::new();
        for row in &queue.rows {
            let [x, y, w, h] = row.rect;
            let own: Vec<usize> = out.iter().enumerate().filter(|(_, s)| s.screen && s.view == Some(camera))
                .filter(|(_, s)| rect_of(s).is_some_and(|r| r[1] >= y - 1 && r[1] + r[3] <= y + h + 1 && r[0] >= x && r[0] + r[2] <= x + w && r[2] * 4 < w * 3))
                .map(|(i, _)| i).collect();
            let leftmost = own.iter().filter_map(|&i| rect_of(&out[i]).map(|r| r[0])).min().unwrap_or(x + w);
            let shift = (row.grip[0] + row.grip[2] + 2 - leftmost).max(0);
            let clip = [x, y, x + w, y + h];
            for &i in &own {
                let s = &mut out[i];
                s.x += shift;
                s.clip = Some(s.clip.map_or(clip, |c| [c[0].max(clip[0]), c[1].max(clip[1]), c[2].min(clip[2]), c[3].min(clip[3])]));
            }
            let mut sprites: Vec<PlacedSprite> = own.iter().map(|&i| out[i]).collect();
            let grip = grip(row.grip, style, &solid);
            out.extend(grip.iter().copied());
            sprites.extend(grip);
            row_sprites.push((row.slot, sprites));
        }
        let Some(h) = self.held.filter(|h| h.stamp == queue.stamp) else { return Some(out) };
        let Some(row) = queue.rows.iter().find(|r| r.slot == h.slot) else { return Some(out) };
        let [x, y, w, rh] = row.rect;
        out.push(solid(x, y, w, rh, style.dim));
        let pointer = self.pointer.unwrap_or(h.at);
        // The copy following the pointer (picked up by a click: beside it).
        let (dx, dy) = if h.sticky { (pointer.0 - x + 12, pointer.1 - y + 8) } else { (pointer.0 - h.at.0, pointer.1 - h.at.1) };
        let (gx, gy) = (x + dx, y + dy);
        // (Within the queue panel's columns: never over the screen beside it.)
        let (left, right) = (queue.rows.iter().map(|r| r.rect[0]).min().unwrap_or(x), queue.rows.iter().map(|r| r.rect[0] + r.rect[2]).max().unwrap_or(x + w));
        let (from, to) = (gx.max(left), (gx + w).min(right));
        if to > from {
            out.push(solid(from - 1, gy - 1, to - from + 2, rh + 2, style.ghost_frame));
            out.push(solid(from, gy, to - from, rh, style.ghost_back));
        }
        let ghost_clip = [from, gy, to.max(from), gy + rh];
        for s in row_sprites.iter().find(|(slot, _)| *slot == h.slot).map(|(_, s)| s.as_slice()).unwrap_or_default() {
            let mut s = *s;
            s.x += dx;
            s.y += dy;
            s.clip = Some(ghost_clip);
            if s.solid.is_none() && s.blend == Blend::Alpha {
                s.blend = Blend::AlphaOpacity { opacity: style.ghost_opacity };
            }
            out.push(s);
        }
        // Where it goes, over the copy.
        match target(queue, h.slot, pointer, h.sticky) {
            Some(Target::Current) => out.extend(swap_hint(queue.current, style, &solid)),
            Some(Target::Before(n)) => {
                let line_y = match queue.rows.iter().find(|r| r.slot == n) {
                    Some(r) => r.rect[1] - 1,
                    None => queue.rows.last().map_or(y, |r| r.rect[1] + r.rect[3] - 1),
                };
                out.push(solid(left, line_y - 1, right - left, 1, style.line_edge));
                out.push(solid(left, line_y + 2, right - left, 1, style.line_edge));
                out.push(solid(left, line_y, right - left, 2, style.line));
                out.push(solid(left, line_y - 3, 3, 8, style.line));
                out.push(solid(right - 3, line_y - 3, 3, 8, style.line));
            }
            None => {}
        }
        Some(out)
    }
}

impl InterfaceExtension for QueueDrag {
    fn route(&mut self, input: &Input, scene: &Scene) -> Option<Vec<Input>> {
        QueueDrag::route(self, input, ProductionQueue::of(scene).as_ref())
    }

    fn active(&self) -> bool {
        QueueDrag::active(self)
    }

    fn cancel(&mut self) {
        QueueDrag::cancel(self)
    }

    fn draws(&self, scene: &Scene) -> bool {
        scene.interface.extension(QUEUE).is_some()
    }

    fn draw(&self, scene: &Scene, free: &[PlacedSprite], rect_of: &dyn Fn(&PlacedSprite) -> Option<Rect>) -> Option<Vec<PlacedSprite>> {
        QueueDrag::draw(self, &ProductionQueue::of(scene)?, free, rect_of)
    }
}

fn overlap(a: Rect, b: Rect) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// The edit for a drop of `h` at `point`: none where it changes nothing.
fn drop(queue: &ProductionQueue, h: Held, point: (i32, i32), click: bool) -> Vec<Input> {
    target(queue, h.slot, point, click).map(|to| QueueEdit { stamp: h.stamp, from: h.slot, to }.input()).into_iter().collect()
}

/// Three engraved bars in the middle of `rect`.
fn grip(rect: Rect, style: &QueueStyle, solid: &impl Fn(i32, i32, i32, i32, [u8; 4]) -> PlacedSprite) -> Vec<PlacedSprite> {
    let [x, y, w, h] = rect;
    let (bar, left) = ((w - 6).max(4), x + 3);
    let middle = y + h / 2 - 1;
    [-4, 0, 4].into_iter().flat_map(|d| [solid(left, middle + d, bar, 1, style.groove), solid(left, middle + d + 1, bar, 1, style.edge)]).collect()
}

/// The production box lit, a frame round it, and arrows up and down: what
/// is dropped is produced, what was produced goes into the queue.
fn swap_hint(rect: Rect, style: &QueueStyle, solid: &impl Fn(i32, i32, i32, i32, [u8; 4]) -> PlacedSprite) -> Vec<PlacedSprite> {
    let [x, y, w, h] = rect;
    let mut out = vec![
        solid(x, y, w, h, style.hint_fill),
        solid(x, y, w, 3, style.hint_frame),
        solid(x, y + h - 3, w, 3, style.hint_frame),
        solid(x, y, 3, h, style.hint_frame),
        solid(x + w - 3, y, 3, h, style.hint_frame),
    ];
    // An arrow: a shaft and a stepped head, up (into the box) at the left,
    // down (out of it) at the right.
    let (len, cy) = ((h / 3).max(8), y + h / 2);
    for (cx, up) in [(x + w / 4, true), (x + w - w / 4, false)] {
        out.push(solid(cx - 1, cy - len / 2, 3, len, style.arrow));
        for k in 0..5 {
            let row = if up { cy - len / 2 + k } else { cy + len / 2 - 1 - k };
            out.push(solid(cx - k - 1, row, 2 * k + 3, 1, style.arrow));
        }
    }
    out
}

#[cfg(test)]
mod tests;
