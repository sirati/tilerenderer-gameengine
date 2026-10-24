//! Reordering a city's build queue by dragging (`scene::Interface::
//! build_queue`): each queued row gets a handle (three grooved bars, the
//! city screen's brown on its parchment) before its text, which moves
//! right to make room. Pressed on a handle, the row is picked up: a
//! translucent copy follows the pointer, its own place is dimmed, a line
//! shows where it goes between the rows (above or below the middle of the
//! row under the pointer), and over the production box the box lights up
//! with two arrows: the row will be built now and what is built goes to
//! the front of the queue. Let go, the driver gets `Input::BuildQueue`;
//! let go anywhere else, nothing happens. Nothing of the press, the drag
//! or the release reaches the driver.
//!
//! A click on a handle (no motion while it was held) picks the row up
//! too, until the next click: on a row it takes that row's place, on the
//! production box it swaps, anywhere else it is put back. That is how the
//! Alt mode moves rows: the handles are interactables of their own
//! (buttons), the snapping pointer clicks one, then the target. Esc or the
//! right button puts the row back.

use scene::{BuildQueue, Button, Input, Scene};
use tile_render::{Blend, PlacedSprite, SolidRect, SpriteSetId};

const VK_ESCAPE: u32 = 0x1B;
/// Pixels the pointer moves while held before the press is a drag.
const DRAG: i32 = 4;

/// The handle's bars: dark groove, light edge below (as the screen's
/// engraved lines), on the city screen's parchment.
const GROOVE: [u8; 4] = [92, 64, 34, 255];
const EDGE: [u8; 4] = [250, 240, 214, 255];
/// The insertion line and the production box's swap hint.
const LINE: [u8; 4] = [176, 18, 8, 255];
/// The light edges either side of the insertion line, so it reads on dark
/// text and on the translucent copy alike.
const LINE_EDGE: [u8; 4] = [255, 248, 226, 255];
const HINT_FILL: [u8; 4] = [255, 214, 90, 80];
const HINT_FRAME: [u8; 4] = [214, 150, 20, 255];
const ARROW: [u8; 4] = [120, 60, 10, 255];
/// The picked-up row's own place, and the copy's backing.
const DIM: [u8; 4] = [240, 230, 205, 170];
const GHOST_BACK: [u8; 4] = [246, 236, 210, 110];
const GHOST_FRAME: [u8; 4] = [120, 84, 44, 150];
const GHOST_OPACITY: u8 = 130;

type Rect = [i32; 4]; // x, y, w, h

fn inside([x, y, w, h]: Rect, (px, py): (i32, i32)) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

/// Where a picked-up row goes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Before the row of queue slot `n` (one past the last: the end).
    Before(u8),
    /// Onto the production box.
    Current,
}

impl Target {
    fn wire(self) -> u8 {
        match self {
            Target::Before(n) => n,
            Target::Current => scene::BUILD_QUEUE_CURRENT,
        }
    }
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

/// The drag of a queue row (a part of the viewer's input state).
#[derive(Clone, Debug, Default)]
pub struct BuildQueueDrag {
    held: Option<Held>,
    pointer: Option<(i32, i32)>,
    /// The release of a press that dropped (or put back) a row: swallowed.
    swallow_release: bool,
}

/// What `point` drops queue row `from` onto: the production box, or a
/// place between the rows (a drag: above or below the middle of the row
/// under it; a click: that row's own place). `None`: elsewhere, or where
/// it is already.
pub fn target(queue: &BuildQueue, from: u8, point: (i32, i32), click: bool) -> Option<Target> {
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

impl BuildQueueDrag {
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
    pub fn route(&mut self, input: &Input, scene: &Scene) -> Option<Vec<Input>> {
        let queue = scene.interface.build_queue.as_ref();
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

    /// The driver's drawing (`free`, the city screen's interface at
    /// `scene`'s rectangles) with the handles, and while a row is picked
    /// up the copy, the line and the hint. `None`: no queue shown.
    pub fn draw(&self, scene: &Scene, free: &[PlacedSprite], rect_of: impl Fn(&PlacedSprite) -> Option<Rect>) -> Option<Vec<PlacedSprite>> {
        let queue = scene.interface.build_queue.as_ref()?;
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
            let grip = grip(row.grip, &solid);
            out.extend(grip.iter().copied());
            sprites.extend(grip);
            row_sprites.push((row.slot, sprites));
        }
        let Some(h) = self.held.filter(|h| h.stamp == queue.stamp) else { return Some(out) };
        let Some(row) = queue.rows.iter().find(|r| r.slot == h.slot) else { return Some(out) };
        let [x, y, w, rh] = row.rect;
        out.push(solid(x, y, w, rh, DIM));
        let pointer = self.pointer.unwrap_or(h.at);
        // The copy following the pointer (picked up by a click: beside it).
        let (dx, dy) = if h.sticky { (pointer.0 - x + 12, pointer.1 - y + 8) } else { (pointer.0 - h.at.0, pointer.1 - h.at.1) };
        let (gx, gy) = (x + dx, y + dy);
        // (Within the queue panel's columns: never over the screen beside it.)
        let (left, right) = (queue.rows.iter().map(|r| r.rect[0]).min().unwrap_or(x), queue.rows.iter().map(|r| r.rect[0] + r.rect[2]).max().unwrap_or(x + w));
        let (from, to) = (gx.max(left), (gx + w).min(right));
        if to > from {
            out.push(solid(from - 1, gy - 1, to - from + 2, rh + 2, GHOST_FRAME));
            out.push(solid(from, gy, to - from, rh, GHOST_BACK));
        }
        let ghost_clip = [from, gy, to.max(from), gy + rh];
        for s in row_sprites.iter().find(|(slot, _)| *slot == h.slot).map(|(_, s)| s.as_slice()).unwrap_or_default() {
            let mut s = *s;
            s.x += dx;
            s.y += dy;
            s.clip = Some(ghost_clip);
            if s.solid.is_none() && s.blend == Blend::Alpha {
                s.blend = Blend::AlphaOpacity { opacity: GHOST_OPACITY };
            }
            out.push(s);
        }
        // Where it goes, over the copy.
        match target(queue, h.slot, pointer, h.sticky) {
            Some(Target::Current) => out.extend(swap_hint(queue.current, &solid)),
            Some(Target::Before(n)) => {
                let line_y = match queue.rows.iter().find(|r| r.slot == n) {
                    Some(r) => r.rect[1] - 1,
                    None => queue.rows.last().map_or(y, |r| r.rect[1] + r.rect[3] - 1),
                };
                out.push(solid(left, line_y - 1, right - left, 1, LINE_EDGE));
                out.push(solid(left, line_y + 2, right - left, 1, LINE_EDGE));
                out.push(solid(left, line_y, right - left, 2, LINE));
                out.push(solid(left, line_y - 3, 3, 8, LINE));
                out.push(solid(right - 3, line_y - 3, 3, 8, LINE));
            }
            None => {}
        }
        Some(out)
    }
}

fn overlap(a: Rect, b: Rect) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// The edit for a drop of `h` at `point`: none where it changes nothing.
fn drop(queue: &BuildQueue, h: Held, point: (i32, i32), click: bool) -> Vec<Input> {
    target(queue, h.slot, point, click).map(|t| Input::BuildQueue { stamp: h.stamp, from: h.slot, to: t.wire() }).into_iter().collect()
}

/// Three engraved bars in the middle of `rect`.
fn grip(rect: Rect, solid: &impl Fn(i32, i32, i32, i32, [u8; 4]) -> PlacedSprite) -> Vec<PlacedSprite> {
    let [x, y, w, h] = rect;
    let (bar, left) = ((w - 6).max(4), x + 3);
    let middle = y + h / 2 - 1;
    [-4, 0, 4].into_iter().flat_map(|d| [solid(left, middle + d, bar, 1, GROOVE), solid(left, middle + d + 1, bar, 1, EDGE)]).collect()
}

/// The production box lit, a frame round it, and arrows up and down: what
/// is dropped is built, what was built goes into the queue.
fn swap_hint(rect: Rect, solid: &impl Fn(i32, i32, i32, i32, [u8; 4]) -> PlacedSprite) -> Vec<PlacedSprite> {
    let [x, y, w, h] = rect;
    let mut out = vec![
        solid(x, y, w, h, HINT_FILL),
        solid(x, y, w, 3, HINT_FRAME),
        solid(x, y + h - 3, w, 3, HINT_FRAME),
        solid(x, y, 3, h, HINT_FRAME),
        solid(x + w - 3, y, 3, h, HINT_FRAME),
    ];
    // An arrow: a shaft and a stepped head, up (into the box) at the left,
    // down (out of it) at the right.
    let (len, cy) = ((h / 3).max(8), y + h / 2);
    for (cx, up) in [(x + w / 4, true), (x + w - w / 4, false)] {
        out.push(solid(cx - 1, cy - len / 2, 3, len, ARROW));
        for k in 0..5 {
            let row = if up { cy - len / 2 + k } else { cy + len / 2 - 1 - k };
            out.push(solid(cx - k - 1, row, 2 * k + 3, 1, ARROW));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use scene::BuildQueueRow;

    fn queue() -> BuildQueue {
        let rows = (0..4).map(|k| BuildQueueRow { slot: k, rect: [100, 200 + 20 * k as i32, 160, 20], grip: [100, 200 + 20 * k as i32, 14, 20], grip_id: 0x10 + u64::from(k) }).collect();
        BuildQueue { stamp: 77, current: [400, 300, 114, 95], rows }
    }

    fn scene() -> Scene {
        let mut s = Scene::default();
        s.interface.build_queue = Some(queue());
        s
    }

    fn button(down: bool, x: i32, y: i32) -> Input {
        Input::MouseButton { button: Button::Left, down, x, y }
    }

    #[test]
    fn drags_drop_between_the_rows_above_or_below_the_middle() {
        let q = queue();
        // Row 0 dragged: into row 2's upper half: before 2; its lower half: before 3.
        assert_eq!(target(&q, 0, (150, 245), false), Some(Target::Before(2)));
        assert_eq!(target(&q, 0, (150, 255), false), Some(Target::Before(3)));
        assert_eq!(target(&q, 0, (150, 278), false), Some(Target::Before(4)), "below the last row's middle: the end");
        assert_eq!(target(&q, 3, (150, 192), false), Some(Target::Before(0)), "half a row above the first");
        // Where it is already: nothing.
        assert_eq!(target(&q, 1, (150, 215), false), None);
        assert_eq!(target(&q, 1, (150, 235), false), None);
        assert_eq!(target(&q, 0, (150, 400), false), None, "outside the rows");
        assert_eq!(target(&q, 0, (99, 245), false), None);
        assert_eq!(target(&q, 2, (450, 340), false), Some(Target::Current));
    }

    #[test]
    fn a_click_drop_takes_the_clicked_rows_place() {
        let q = queue();
        assert_eq!(target(&q, 0, (150, 250), true), Some(Target::Before(3)), "down: after that row");
        assert_eq!(target(&q, 3, (150, 210), true), Some(Target::Before(0)), "up: before it");
        assert_eq!(target(&q, 1, (150, 230), true), None, "its own row");
    }

    #[test]
    fn a_drag_from_a_handle_never_reaches_the_driver_and_drops_on_release() {
        let mut d = BuildQueueDrag::default();
        let s = scene();
        assert_eq!(d.route(&button(true, 105, 210), &s), Some(vec![]), "the press on the handle is the drag's");
        assert!(d.active());
        assert_eq!(d.route(&Input::MouseMove { x: 120, y: 272 }, &s), None, "motion still goes on");
        assert_eq!(d.route(&button(false, 120, 272), &s), Some(vec![Input::BuildQueue { stamp: 77, from: 0, to: 4 }]));
        assert!(!d.active());
        // A press on the row's text is the driver's.
        assert_eq!(d.route(&button(true, 200, 210), &s), None);
        assert_eq!(d.route(&button(false, 200, 210), &s), None);
    }

    #[test]
    fn dropping_on_the_production_box_swaps_and_elsewhere_puts_back() {
        let s = scene();
        let mut d = BuildQueueDrag::default();
        d.route(&button(true, 105, 250), &s);
        d.route(&Input::MouseMove { x: 420, y: 320 }, &s);
        assert_eq!(d.route(&button(false, 420, 320), &s), Some(vec![Input::BuildQueue { stamp: 77, from: 2, to: scene::BUILD_QUEUE_CURRENT }]));
        d.route(&button(true, 105, 250), &s);
        d.route(&Input::MouseMove { x: 700, y: 20 }, &s);
        assert_eq!(d.route(&button(false, 700, 20), &s), Some(vec![]), "let go elsewhere: nothing");
    }

    #[test]
    fn a_click_picks_up_until_the_next_click_as_the_alt_mode_does_it() {
        let s = scene();
        let mut d = BuildQueueDrag::default();
        assert_eq!(d.route(&button(true, 106, 230), &s), Some(vec![]));
        assert_eq!(d.route(&button(false, 106, 230), &s), Some(vec![]));
        assert!(d.active(), "picked up by the click");
        assert_eq!(d.route(&Input::MouseMove { x: 457, y: 347 }, &s), None);
        assert_eq!(d.route(&button(true, 457, 347), &s), Some(vec![Input::BuildQueue { stamp: 77, from: 1, to: scene::BUILD_QUEUE_CURRENT }]));
        assert_eq!(d.route(&button(false, 457, 347), &s), Some(vec![]), "its release too");
        assert!(!d.active());
        // Esc puts it back.
        d.route(&button(true, 106, 230), &s);
        d.route(&button(false, 106, 230), &s);
        assert_eq!(d.route(&Input::Key { vk: VK_ESCAPE, down: true, text: None }, &s), Some(vec![]));
        assert!(!d.active());
    }

    #[test]
    fn a_new_queue_lets_go_of_the_row() {
        let mut d = BuildQueueDrag::default();
        d.route(&button(true, 105, 210), &scene());
        let mut other = scene();
        other.interface.build_queue.as_mut().unwrap().stamp = 78;
        d.route(&Input::MouseMove { x: 0, y: 0 }, &other);
        assert!(!d.active());
        assert_eq!(d.route(&button(false, 120, 262), &other), None);
    }

    fn text(x: i32, y: i32) -> PlacedSprite {
        PlacedSprite { screen: true, view: Some(3), solid: Some(SolidRect { width: 40, height: 10, rgba: [0, 0, 0, 255] }), ..PlacedSprite::new(SpriteSetId::from_raw(9), x, y) }
    }

    fn rect(s: &PlacedSprite) -> Option<Rect> {
        let r = s.solid?;
        let (mut x0, mut y0, mut x1, mut y1) = (s.x, s.y, s.x + r.width as i32, s.y + r.height as i32);
        if let Some(c) = s.clip { (x0, y0, x1, y1) = (x0.max(c[0]), y0.max(c[1]), x1.min(c[2]), y1.min(c[3])) }
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
    }

    #[test]
    fn handles_come_before_the_text_which_moves_right() {
        let s = scene();
        let free: Vec<PlacedSprite> = (0..4).map(|k| text(102, 205 + 20 * k)).collect();
        let out = BuildQueueDrag::default().draw(&s, &free, rect).unwrap();
        assert_eq!(out[0].x, 100 + 14 + 2, "text right of the handle");
        assert_eq!(out[0].view, Some(3));
        let bars: Vec<_> = out[4..].iter().filter(|p| p.solid.unwrap().rgba == GROOVE).collect();
        assert_eq!(bars.len(), 3 * 4, "three bars a row");
        assert!(bars.iter().all(|b| b.view == Some(3) && b.x >= 100 && b.x + b.solid.unwrap().width as i32 <= 114));
        assert!(BuildQueueDrag::default().draw(&Scene::default(), &free, rect).is_none());
    }

    #[test]
    fn a_drag_shows_its_copy_the_line_and_over_the_box_the_hint() {
        let s = scene();
        let free: Vec<PlacedSprite> = (0..4).map(|k| text(102, 205 + 20 * k)).collect();
        let mut d = BuildQueueDrag::default();
        d.route(&button(true, 105, 210), &s);
        d.route(&Input::MouseMove { x: 110, y: 272 }, &s);
        let out = d.draw(&s, &free, rect).unwrap();
        assert!(out.iter().any(|p| p.solid.is_some_and(|r| r.rgba == LINE && r.height == 2) && p.y == 279), "line below the last row");
        assert!(out.iter().any(|p| p.solid.is_some_and(|r| r.rgba == GHOST_BACK) && (p.x, p.y) == (105, 262)), "the copy where the pointer took it");
        d.route(&Input::MouseMove { x: 420, y: 320 }, &s);
        let out = d.draw(&s, &free, rect).unwrap();
        assert!(out.iter().any(|p| p.solid.is_some_and(|r| r.rgba == HINT_FRAME)));
        assert!(!out.iter().any(|p| p.solid.is_some_and(|r| r.rgba == LINE && r.height == 2)));
    }

    #[test]
    fn the_copy_stays_in_the_queue_panel_and_lets_the_hints_show_through() {
        let s = scene();
        let panel = s.interface.build_queue.as_ref().unwrap().rows.iter().map(|r| r.rect[0] + r.rect[2]).max().unwrap();
        let free: Vec<PlacedSprite> = (0..4).map(|k| text(102, 205 + 20 * k)).collect();
        let mut d = BuildQueueDrag::default();
        d.route(&button(true, 105, 210), &s);
        for (x, y) in [(170, 272), (420, 320)] {
            d.route(&Input::MouseMove { x, y }, &s);
            let out = d.draw(&s, &free, rect).unwrap();
            for p in out.iter().filter(|p| p.solid.is_some_and(|r| matches!(r.rgba, GHOST_BACK | GHOST_FRAME))) {
                assert!(p.x + p.solid.unwrap().width as i32 <= panel + 1, "copy past the panel at {x}: {p:?}");
            }
            assert!(out.iter().filter(|p| p.clip.is_some() && p.solid.is_none() && p.y > 230).all(|p| p.clip.unwrap()[2] <= panel));
        }
        assert!(GHOST_BACK[3] <= 128 && GHOST_OPACITY <= 140, "translucent enough for the line and arrows");
    }
}
