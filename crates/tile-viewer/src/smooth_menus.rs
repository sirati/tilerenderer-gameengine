//! Continuous scrolling for the driver's row-based lists. The wheel and the
//! trackpad move a list's own position in pixels (`List::pos`) at once and
//! additively; it stops anywhere. Only after the input has been still for
//! [`IDLE`] seconds, and only with "Snap lists to items", does the list
//! glide to an item's edge ([`snap_point`]).
//!
//! The driver can only show whole items (`step` pixels a wheel notch). The
//! viewer keeps the drawings the driver made at each of its positions on
//! the way and draws the list at its own position from them: each row's
//! band from the newest drawing that has the row ([`List::compose`]). The
//! driver's position is the viewer's, rounded towards where the list is
//! going, one item ahead while it moves ([`List::wanted`]): the viewer
//! sends the wheel notches that take it there, so the next rows are drawn
//! before they show. A position no drawing covers yet is waited for.
//!
//! Input goes to the row the pointer is over as shown: a point is taken to
//! the list's content and back to where the driver will have that row
//! once it has taken the notches sent before it. A press on a row the
//! driver does not hold yet sends the notches that bring it in first. The
//! driver's pointer highlight is moved to the row under the pointer
//! ([`highlight`]).

use scene::{Button, Input, InteractableKind, Scene};
use tile_render::PlacedSprite;

use crate::looks::Span;

mod highlight;
use highlight::Highlight;

/// Seconds without scroll input after which a list settles: it snaps to an
/// item (with snapping) and the driver gets its position rounded.
pub const IDLE: f64 = 0.15;
/// The time constant (seconds) of a list's glide towards where the input
/// asks it to be: the Alt mode's list scrolling's
/// ([`crate::snap::SCROLL_SMOOTHING`]).
pub const GLIDE: f64 = crate::snap::SCROLL_SMOOTHING;
/// How far past an item's edge (a fraction of the item), in the direction
/// the list moved last, a list must be to snap on to the next edge rather
/// than back to that one.
pub const SNAP_FORWARD: f64 = 1.0 / 3.0;
/// Seconds after the last notch by which the driver has drawn the position
/// it was sent to; if it has not, it did not go there (an end of the list).
const SETTLE: f64 = 0.35;
/// One notch, as `Input::Wheel` counts them.
const NOTCH: i32 = 120;
/// Drawings kept per list.
const MAX_DRAWINGS: usize = 64;
/// Notches sent to the driver in one frame, at most.
const MAX_NOTCHES: i64 = 16;

type Rect = [i32; 4]; // left, top, right, bottom

fn bounds([x, y, w, h]: [i32; 4]) -> Rect { [x, y, x.saturating_add(w), y.saturating_add(h)] }
fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}
fn inside(r: Rect, (x, y): (i32, i32)) -> bool { x >= r[0] && x < r[2] && y >= r[1] && y < r[3] }
fn shifted(mut r: Rect, dy: i32) -> Rect { r[1] += dy; r[3] += dy; r }

const UNCLIPPED: Rect = [-tile_render::world::CLIP_LIMIT, -tile_render::world::CLIP_LIMIT, tile_render::world::CLIP_LIMIT, tile_render::world::CLIP_LIMIT];

fn clipped(mut s: PlacedSprite, r: Rect) -> Option<PlacedSprite> {
    s.clip = Some(intersect(s.clip.unwrap_or(UNCLIPPED), r)?);
    Some(s)
}

/// `rect` (a list's scroll rectangle) without the scroll bars reaching
/// into it (the game's `Scroll_Bar`s, sliders to the scene): only the
/// list's content glides; a bar's track, arrows and thumb stay where the
/// driver draws them. Each bar cuts off the side of `rect` it is on.
fn without_scroll_bars(scene: &Scene, mut rect: Rect) -> Rect {
    for bar in scene.interactables.iter().filter(|i| matches!(i.kind, InteractableKind::Slider { .. })) {
        let bar = bounds(bar.rect);
        if intersect(bar, rect).is_none() { continue }
        if bar[3] - bar[1] >= bar[2] - bar[0] {
            if bar[0] + bar[2] >= rect[0] + rect[2] { rect[2] = rect[2].min(bar[0]) } else { rect[0] = rect[0].max(bar[2]) }
        } else if bar[1] + bar[3] >= rect[1] + rect[3] { rect[3] = rect[3].min(bar[1]) } else { rect[1] = rect[1].max(bar[3]) }
    }
    rect
}

/// Where the vertical scroll bar beside `rect` stands, if there is one:
/// `Some(0)` at the top, `Some(1_000_000)` at the bottom.
fn bar_value(scene: &Scene, wheel: Rect) -> Option<i32> {
    scene.interactables.iter().find_map(|bar| match bar.kind {
        InteractableKind::Slider { value, .. } => {
            let r = bounds(bar.rect);
            (r[3] - r[1] > r[2] - r[0] && intersect(r, wheel).is_some()).then_some(value)
        }
        _ => None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row { id: u64, rect: Rect }

#[derive(Clone, Debug)]
struct Picture {
    id: u64,
    rect: Rect,
    rows: Vec<Row>,
    sprites: Vec<PlacedSprite>,
    /// Each sprite's extent (unclipped), to tell the rows' own sprites.
    extents: Vec<Rect>,
    cameras: Vec<u32>,
}

/// A list as the driver shows it in one scene: its picture, the rectangle
/// the wheel scrolls it in, its step and its scroll bar.
struct Seen {
    picture: Picture,
    wheel: Rect,
    step: i32,
    bar: Option<i32>,
}

impl Picture {
    fn take(scene: &Scene, free: &[PlacedSprite], spans: &[Span], rect_of: &impl Fn(&PlacedSprite) -> Option<[i32; 4]>) -> Vec<Seen> {
        let mut overlay_cameras = Vec::new();
        for camera in spans.iter().flat_map(|s| s.cameras.iter().copied()) {
            if !overlay_cameras.contains(&camera) { overlay_cameras.push(camera); }
        }
        scene.interactables.iter().filter_map(|list| {
            let InteractableKind::List { scroll, step } = list.kind else { return None };
            let rect = without_scroll_bars(scene, bounds(scroll));
            if rect[2] <= rect[0] || rect[3] <= rect[1] { return None }
            let rows: Vec<Row> = scene.interactables.iter().filter(|row| row.kind == InteractableKind::ListItem)
                .filter(|row| {
                    let r = bounds(row.rect);
                    let center = ((r[0] + r[2]) / 2, (r[1] + r[3]) / 2);
                    r[2] > r[0] && r[3] > r[1] && inside(rect, center)
                        && scene.interactables.iter().filter_map(|candidate| match candidate.kind {
                            InteractableKind::List { scroll, .. } if inside(bounds(scroll), center) => Some((candidate.id, i64::from(scroll[2]) * i64::from(scroll[3]))),
                            _ => None,
                        }).min_by_key(|(_, area)| *area).is_some_and(|(id, _)| id == list.id)
                }).map(|row| Row { id: row.id, rect: bounds(row.rect) }).collect();
            // ListItem hit areas can cover only a row's name. The native
            // scroll rectangle covers every column, including nonclickable
            // statistics and cells represented as buttons.
            if rows.is_empty() { return None }
            let mut sprites = Vec::new();
            let mut extents = Vec::new();
            let mut cameras = Vec::new();
            // Overlays are the driver's screen pixels; world sprites are
            // excluded, even when the city map lies behind its list.
            for s in free.iter().filter(|s| s.screen && !s.except_view && s.view.is_some_and(|v| overlay_cameras.contains(&v))) {
                if let Some(extent) = rect_of(s).map(bounds).filter(|r| intersect(*r, rect).is_some()) {
                    if let Some(s) = clipped(*s, rect) {
                        if let Some(camera) = s.view {
                            if !cameras.contains(&camera) { cameras.push(camera); }
                        }
                        sprites.push(s);
                        extents.push(extent);
                    }
                }
            }
            let picture = Picture { id: list.id, rect, rows, sprites, extents, cameras };
            Some(Seen { picture, wheel: bounds(list.rect), step, bar: bar_value(scene, bounds(list.rect)) })
        }).collect()
    }

    fn row_at(&self, point: (i32, i32)) -> Option<&Row> {
        if !inside(self.rect, point) { return None }
        if let Some(row) = self.rows.iter().find(|row| inside(row.rect, point)) { return Some(row) }
        // Tables often expose only the name as ListItem. Other columns
        // share that row's vertical transform. Ambiguous multicolumn lists
        // keep their independent native hit areas.
        let mut rows = self.rows.iter().filter(|row| point.1 >= row.rect[1] && point.1 < row.rect[3]);
        let row = rows.next()?;
        rows.next().is_none().then_some(row)
    }

    /// How far the rows moved down since `old` (0: not at all), if all of
    /// them that both show moved alike and some do.
    fn offset_from(&self, old: &Self) -> Option<i32> {
        if self.rect != old.rect || self.cameras != old.cameras { return None }
        let mut movement = None;
        for row in &self.rows {
            let Some(previous) = old.rows.iter().find(|r| r.id == row.id) else { continue };
            if row.rect[0] != previous.rect[0] || row.rect[2] != previous.rect[2]
                || row.rect[3] - row.rect[1] != previous.rect[3] - previous.rect[1] { return None }
            let dy = row.rect[1] - previous.rect[1];
            if movement.is_some_and(|m| m != dy) { return None }
            movement = Some(dy);
        }
        movement
    }

    /// The band its rows take (top, bottom), relative to the list's top.
    fn band(&self) -> (i32, i32) {
        let top = self.rows.iter().map(|r| r.rect[1]).min().unwrap_or(self.rect[1]).max(self.rect[1]);
        let bottom = self.rows.iter().map(|r| r.rect[3]).max().unwrap_or(self.rect[3]).min(self.rect[3]);
        (top - self.rect[1], bottom.max(top) - self.rect[1])
    }

    /// Its sprites and rows moved down by `dy`, inside `clip` only.
    fn piece(&self, dy: i32, clip: Rect, out: &mut Picture) {
        for (s, extent) in self.sprites.iter().zip(&self.extents) {
            let mut s = *s;
            s.y += dy;
            s.clip = s.clip.map(|r| shifted(r, dy));
            if let Some(s) = clipped(s, clip) {
                out.sprites.push(s);
                out.extents.push(shifted(*extent, dy));
            }
        }
        for row in &self.rows {
            let rect = shifted(row.rect, dy);
            if intersect(rect, clip).is_some() && !out.rows.iter().any(|r| r.id == row.id) {
                out.rows.push(Row { id: row.id, rect });
            }
        }
    }
}

/// A drawing of the driver's kept: the list's content offset (pixels) it
/// shows.
#[derive(Clone, Debug)]
struct Drawing { scroll: i64, picture: Picture }

/// Where a list settles from `pos` (content pixels) on items `step` apart
/// from `origin`, having moved last in direction `dir` (+1 down, −1 up, 0
/// not known): the item edge it is nearest to, but the next one in the
/// direction of travel once [`SNAP_FORWARD`] of an item past the last.
pub fn snap_point(pos: f64, origin: f64, step: f64, dir: f64) -> f64 {
    let k = (pos - origin) / step;
    let (back, ahead) = (k.floor(), k.ceil());
    let f = k - back;
    let edge = if dir > 0.0 { if f >= SNAP_FORWARD { ahead } else { back } }
        else if dir < 0.0 { if 1.0 - f >= SNAP_FORWARD { back } else { ahead } }
        else { k.round() };
    origin + edge * step
}

/// One list's own scrolling.
#[derive(Clone, Debug)]
struct List {
    id: u64,
    rect: Rect,
    /// Where the wheel scrolls it (the driver's list with its bar).
    wheel: Rect,
    step: i64,
    cameras: Vec<u32>,
    /// The driver's drawings, newest last.
    drawings: Vec<Drawing>,
    /// The content offset of the driver's newest drawing (its scroll).
    drawn: i64,
    /// Where the driver is once it has taken the notches sent.
    expected: i64,
    /// When the last notch was sent, and its direction (+1 down).
    notched: (f64, i64),
    /// The list's position (content pixels; `drawn` is the driver's
    /// position in the same pixels), what the input asks for, and the
    /// whole pixel the last frame showed.
    pos: f64,
    target: f64,
    shown: i64,
    /// When scroll input came last, which way it went (+1 down, −1 up).
    input_at: f64,
    dir: f64,
    /// Whether it has settled since that input.
    settled: bool,
    /// Whether it glides to the target (else it is at the target as soon
    /// as the drawings allow: a trackpad's input).
    glide: bool,
    /// The ends found (content offsets), if known.
    top: Option<i64>,
    bottom: Option<i64>,
    /// Buttons pressed on it, held: the driver's position stays.
    held: u8,
    highlight: Option<Highlight>,
}

impl List {
    fn new(seen: &Seen) -> Self {
        let p = &seen.picture;
        let mut list = List {
            id: p.id, rect: p.rect, wheel: seen.wheel, step: i64::from(seen.step.max(1)), cameras: p.cameras.clone(),
            drawings: vec![Drawing { scroll: 0, picture: p.clone() }], drawn: 0, expected: 0, notched: (f64::NEG_INFINITY, 0),
            pos: 0.0, target: 0.0, shown: 0, input_at: f64::NEG_INFINITY, dir: 0.0, settled: true, glide: true,
            top: None, bottom: None, held: 0, highlight: None,
        };
        list.ends(seen.bar);
        list
    }

    fn origin(&self) -> f64 { self.drawn.rem_euclid(self.step) as f64 }

    fn current(&self) -> &Picture { &self.drawings.last().expect("a list has a drawing").picture }

    /// What the scroll bar says of the ends, at the driver's position.
    fn ends(&mut self, bar: Option<i32>) {
        if self.drawn != self.expected { return }
        match bar {
            Some(v) if v <= 0 => self.top = Some(self.drawn),
            Some(v) if v >= 1_000_000 => self.bottom = Some(self.drawn),
            _ => {}
        }
    }

    fn clamp_to_ends(&self, v: f64) -> f64 {
        let v = self.top.map_or(v, |t| v.max(t as f64));
        self.bottom.map_or(v, |b| v.min(b as f64))
    }

    /// The driver drew the list again (`seen`): kept at its offset. False
    /// if it is not the same list scrolled (start anew).
    fn observe(&mut self, seen: &Seen, now: f64, hover: Option<(i32, i32)>) -> bool {
        let picture = &seen.picture;
        if i64::from(seen.step.max(1)) != self.step || picture.rect != self.rect || picture.cameras != self.cameras { return false }
        let Some(dy) = picture.offset_from(self.current()) else { return false };
        if i64::from(dy) % self.step != 0 { return false }
        let scroll = self.drawn - i64::from(dy);
        if dy == 0 {
            // (The driver lit the row its pointer moved onto: its highlight.)
            if let Some(h) = hover.and_then(|hover| highlight::learn(self.current(), picture, hover)) { self.highlight = Some(h) }
        }
        self.drawn = scroll;
        self.wheel = seen.wheel;
        self.drawings.retain(|d| d.scroll != scroll);
        self.drawings.push(Drawing { scroll, picture: picture.clone() });
        if self.drawings.len() > MAX_DRAWINGS { self.drawings.remove(0); }
        if self.drawn != self.expected && now - self.notched.0 >= SETTLE {
            // The driver did not go where it was sent (an end of a list
            // without a scroll bar), or went elsewhere by itself (its
            // scroll bar's arrows): it is where it is, the list glides
            // there.
            self.expected = self.drawn;
            self.target = self.drawn as f64;
            self.glide = true;
        }
        self.ends(seen.bar);
        self.target = self.clamp_to_ends(self.target);
        true
    }

    /// Scroll input of `d` content pixels (positive: further down) at
    /// `now`: `direct` (a trackpad's) moves the list at once, else it
    /// glides there.
    fn scroll(&mut self, d: f64, direct: bool, now: f64) {
        if d == 0.0 { return }
        if self.settled && self.pos == self.drawn as f64 {
            // A new movement from rest: the drawings kept from before may
            // show what the list no longer holds.
            let current = self.drawings.pop().expect("a list has a drawing");
            self.drawings = vec![current];
        }
        self.target = self.clamp_to_ends(self.target + d);
        if direct { self.pos = self.clamp_to_ends(self.pos + d) }
        self.glide = !direct;
        self.input_at = now;
        self.dir = d.signum();
        self.settled = false;
    }

    /// The offsets the drawings kept can show the list at: (first, last).
    fn coverage(&self) -> (f64, f64) {
        let (a, b) = self.current().band();
        let (a, b) = (i64::from(a), i64::from(b));
        let mut spans: Vec<(i64, i64)> = self.drawings.iter().map(|d| {
            let (da, db) = d.picture.band();
            (d.scroll + i64::from(da), d.scroll + i64::from(db))
        }).collect();
        spans.sort_unstable();
        let (mut lo, mut hi) = (self.drawn + a, self.drawn + b);
        // Grow the run of spans that contains the driver's own.
        loop {
            let before = (lo, hi);
            for &(s, e) in &spans {
                if s <= hi && e >= lo { lo = lo.min(s); hi = hi.max(e); }
            }
            if (lo, hi) == before { break }
        }
        ((lo - a) as f64, (hi - b) as f64)
    }

    /// Time `now`, `dt` after the last frame: the list moves on.
    fn advance(&mut self, now: f64, dt: f64, snap: bool) {
        if !self.settled && now - self.input_at >= IDLE {
            self.settled = true;
            if snap {
                let to = snap_point(self.target, self.origin(), self.step as f64, self.dir);
                self.target = self.clamp_to_ends(to);
                self.glide = true;
            }
        }
        if self.glide {
            let k = 1.0 - (-dt.max(0.0) / GLIDE).exp();
            self.pos += (self.target - self.pos) * k;
        } else {
            self.pos = self.target;
        }
        if (self.target - self.pos).abs() < 0.05 { self.pos = self.target }
        let (lo, hi) = self.coverage();
        self.pos = self.pos.clamp(lo.min(hi), hi.max(lo));
    }

    /// Where the driver should be: the list's position rounded towards
    /// where it is going, an item ahead while it moves; at rest the item
    /// edge nearest it.
    fn wanted(&self) -> i64 {
        let (o, s) = (self.origin(), self.step as f64);
        let grid = |v: f64, f: fn(f64) -> f64| (o + f((v - o) / s) * s).round() as i64;
        let moving = (self.target - self.pos).abs() >= 0.5;
        let w = if moving && self.target > self.pos {
            (grid(self.pos, f64::ceil) + self.step).min(grid(self.target, f64::ceil))
        } else if moving {
            (grid(self.pos, f64::floor) - self.step).max(grid(self.target, f64::floor))
        } else {
            grid(self.pos, f64::round)
        };
        let w = self.top.map_or(w, |t| w.max(t));
        self.bottom.map_or(w, |b| w.min(b))
    }

    /// Wheel notches that take the driver to `to` (from where it will be).
    fn notches_to(&mut self, to: i64, now: f64) -> Vec<Input> {
        let n = ((to - self.expected) / self.step).clamp(-MAX_NOTCHES, MAX_NOTCHES);
        if n == 0 { return vec![] }
        self.expected += n * self.step;
        self.notched = (now, n.signum());
        let (x, y) = ((self.wheel[0] + self.wheel[2]) / 2, (self.wheel[1] + self.wheel[3]) / 2);
        let delta = if n > 0 { -NOTCH } else { NOTCH };
        (0..n.abs()).map(|_| Input::Wheel { delta, x, y }).collect()
    }

    fn moving(&self) -> bool {
        !self.settled || self.pos != self.target || self.expected != self.drawn
    }

    /// The list drawn at offset `p` from the drawings kept: each row band
    /// from the newest drawing that has it, the rest (above and below the
    /// rows) from the driver's own.
    fn compose(&self, p: i64) -> Picture {
        let current = self.current();
        let r = self.rect;
        let (a, b) = current.band();
        let mut out = Picture { id: self.id, rect: r, rows: vec![], sprites: vec![], extents: vec![], cameras: self.cameras.clone() };
        let band = (r[1] + a, r[1] + b);
        for (top, bottom) in [(r[1], band.0), (band.1, r[3])] {
            if bottom > top { current.piece(0, [r[0], top, r[2], bottom], &mut out) }
        }
        let mut open = vec![band];
        for d in self.drawings.iter().rev() {
            let dy = (d.scroll - p) as i32;
            let (da, db) = d.picture.band();
            let (from, to) = (r[1] + da + dy, r[1] + db + dy);
            let mut rest = Vec::new();
            for (s, e) in open {
                let (cs, ce) = (s.max(from), e.min(to));
                if ce > cs {
                    d.picture.piece(dy, [r[0], cs, r[2], ce], &mut out);
                    if cs > s { rest.push((s, cs)) }
                    if e > ce { rest.push((ce, e)) }
                } else {
                    rest.push((s, e));
                }
            }
            open = rest;
            if open.is_empty() { break }
        }
        out
    }

    /// Where the driver will have the content shown at `y` (driver
    /// pixels) once it has taken the notches sent (beside the rows: `y`).
    fn to_driver(&self, y: i32) -> i32 {
        let r = self.rect;
        let (a, b) = self.current().band();
        if y < r[1] + a || y >= r[1] + b { return y }
        let content = i64::from(y) + self.shown;
        (content - self.expected) as i32
    }

    /// Whether the driver will show its row at `y` (its pixels).
    fn holds(&self, y: i32) -> bool {
        let (a, b) = self.current().band();
        y >= self.rect[1] + a && y < self.rect[1] + b
    }
}

/// Hover refresh while a list moves under a stationary mouse.
pub enum PointerRefresh {
    Unchanged,
    Move { x: i32, y: i32 },
    Gone,
}

/// The state of smooth scrolling of all lists shown.
#[derive(Default)]
pub struct SmoothMenus {
    lists: Vec<List>,
    key: Option<(u64, Option<String>, Option<u32>)>,
    /// Where the driver's pointer was sent last (its pixels).
    hover: Option<(i32, i32)>,
    /// The last frame's time.
    drawn_at: Option<f64>,
    /// The last `Input::Scroll` moved a list: the `Wheel` notches the
    /// window made of the same motion are not sent.
    scrolled: bool,
    /// Notches for the driver (taken by [`SmoothMenus::take_inputs`]).
    out: Vec<Input>,
    enabled: bool,
    /// The last frame showed a list elsewhere than the driver drew it.
    patched: bool,
}

impl SmoothMenus {
    /// Whether a list moves, or waits for the driver.
    pub fn running(&self) -> bool { self.lists.iter().any(List::moving) }

    /// Whether the last frame showed a list where the driver did not draw
    /// it: the next frame must start from the driver's own sprites again.
    pub fn patched(&self) -> bool { self.patched }

    /// Clear a held gesture on focus loss.
    pub fn focus_lost(&mut self) {
        for l in &mut self.lists { l.held = 0 }
        self.hover = None;
    }

    /// What the lists send the driver (wheel notches), in order, before any
    /// later input.
    pub fn take_inputs(&mut self) -> Vec<Input> { std::mem::take(&mut self.out) }

    /// For each list that does not show where the driver's scene has its
    /// rows: its rectangle (driver pixels, x, y, w, h) and how far down
    /// its rows show (the button snapping's targets follow).
    pub fn shifts(&self) -> Vec<([i32; 4], i32)> {
        self.lists.iter().filter(|l| l.shown != l.drawn)
            .map(|l| ([l.rect[0], l.rect[1], l.rect[2] - l.rect[0], l.rect[3] - l.rect[1]], (l.drawn - l.shown) as i32)).collect()
    }

    /// Draws the current free sprites with each list at its own position.
    /// `generation` invalidates saved sprite ids when the renderer replaces
    /// its atlas. `pointer` is where the pointer reaches the driver's
    /// window (its pixels), if it does. `snap`: lists settle on items.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, scene: &Scene, free: &[PlacedSprite], spans: &[Span], now: f64, enabled: bool, snap: bool, generation: u64, pointer: Option<(i32, i32)>, rect_of: impl Fn(&PlacedSprite) -> Option<[i32; 4]>) -> Option<Vec<PlacedSprite>> {
        let key = (generation, scene.interface.screen.clone(), scene.interface.popup);
        if !enabled || self.key.as_ref() != Some(&key) {
            self.lists.clear();
            self.scrolled = false;
            self.key = Some(key);
        }
        self.enabled = enabled;
        let dt = self.drawn_at.replace(now).map_or(0.0, |at| (now - at).max(0.0));
        self.patched = false;
        if !enabled { return None }
        let seen = Picture::take(scene, free, spans, &rect_of);
        let mut lists = Vec::new();
        for s in &seen {
            let list = match self.lists.iter().position(|l| l.id == s.picture.id) {
                Some(k) => {
                    let mut l = self.lists.swap_remove(k);
                    if l.observe(s, now, self.hover) { l } else { List::new(s) }
                }
                None => List::new(s),
            };
            lists.push(list);
        }
        self.lists = lists;
        let mut patches: Vec<(Rect, Vec<u32>, Vec<PlacedSprite>)> = Vec::new();
        for l in &mut self.lists {
            l.advance(now, dt, snap);
            if l.held == 0 {
                let to = l.wanted();
                self.out.extend(l.notches_to(to, now));
            }
            l.shown = l.pos.round() as i64;
            if l.shown == l.drawn { continue }
            let mut shown = l.compose(l.shown);
            // The driver's highlight on the row under the pointer.
            if let (Some(h), Some(p)) = (&l.highlight, pointer) {
                if let Some(row) = shown.row_at(p).map(|r| r.id) {
                    if let Some(restyled) = highlight::restyle(&shown, h, row) { shown = restyled }
                }
            }
            patches.push((l.rect, l.cameras.clone(), shown.sprites));
        }
        if patches.is_empty() { return None }
        self.patched = true;
        let mut result = free.to_vec();
        for (rect, cameras, shown) in patches {
            let mut outside = Vec::new();
            for s in result {
                if !s.screen || s.except_view || !s.view.is_some_and(|v| cameras.contains(&v))
                    || !rect_of(&s).is_some_and(|r| intersect(bounds(r), rect).is_some()) {
                    outside.push(s);
                    continue;
                }
                // Preserve the same sprite outside the list. Rectangular
                // subtraction also retains a spanning popup background.
                let r = s.clip.unwrap_or(UNCLIPPED);
                let Some(cut) = intersect(r, rect) else { outside.push(s); continue };
                for keep in [[r[0], r[1], r[2], cut[1]], [r[0], cut[3], r[2], r[3]],
                    [r[0], cut[1], cut[0], cut[3]], [cut[2], cut[1], r[2], cut[3]]] {
                    if rect_of(&s).is_some_and(|actual| intersect(bounds(actual), keep).is_some()) {
                        if let Some(s) = clipped(s, keep) { outside.push(s); }
                    }
                }
            }
            outside.extend(shown);
            result = outside;
        }
        Some(result)
    }

    /// Input for the driver: scroll input over a list moves it (and goes
    /// no further); pointer input over a list goes to the row shown there.
    /// `alt`: button snapping holds the pointer (its notches are the
    /// wheel's; a window scroll is not the list's).
    pub fn route(&mut self, input: Input, alt: bool) -> Vec<Input> {
        if !self.enabled { return vec![input] }
        let now = self.drawn_at.unwrap_or(0.0);
        match input {
            Input::Scroll { dy, pixels, x, y, .. } => {
                self.scrolled = false;
                if alt { return vec![] }
                let Some(l) = self.lists.iter_mut().find(|l| inside(l.wheel, (x, y))) else { return vec![] };
                // (Positive `dy` moves the content down: up the list.)
                let d = if pixels { -f64::from(dy) / 1000.0 } else { -f64::from(dy) / 1000.0 * l.step as f64 };
                l.scroll(d, pixels, now);
                self.scrolled = true;
                vec![]
            }
            Input::Wheel { delta, x, y } => {
                if self.scrolled && !alt { return vec![] }
                let Some(l) = self.lists.iter_mut().find(|l| inside(l.wheel, (x, y))) else { return vec![input] };
                l.scroll(-f64::from(delta) / f64::from(NOTCH) * l.step as f64, false, now);
                vec![]
            }
            Input::MouseMove { x, y } => {
                self.scrolled = false;
                match self.map(x, y) {
                    Some((x, y)) => {
                        self.hover = Some((x, y));
                        vec![Input::MouseMove { x, y }]
                    }
                    None => vec![],
                }
            }
            Input::MouseButton { button, down, x, y } => {
                self.scrolled = false;
                let mut out = Vec::new();
                if let Some(l) = self.lists.iter_mut().find(|l| inside(l.rect, (x, y))) {
                    let bit = 1 << button_index(button);
                    if down {
                        // A row the driver does not hold yet: brought in.
                        let at = l.to_driver(y);
                        if !l.holds(at) && l.holds(y) {
                            let (a, b) = l.current().band();
                            let rows = if at < l.rect[1] + a {
                                -(i64::from(l.rect[1] + a - at) + l.step - 1) / l.step
                            } else {
                                (i64::from(at - (l.rect[1] + b)) + l.step) / l.step
                            };
                            let to = l.expected + rows * l.step;
                            out.extend(l.notches_to(to, now));
                        }
                        l.held |= bit;
                    } else {
                        l.held &= !bit;
                    }
                }
                let (x, y) = self.map(x, y).unwrap_or((x, y));
                out.push(Input::MouseButton { button, down, x, y });
                out
            }
            Input::Key { .. } => {
                self.scrolled = false;
                vec![input]
            }
            other => vec![other],
        }
    }

    /// Where a point the pointer is at (driver pixels) goes in the driver:
    /// `None` on a row the driver does not hold.
    fn map(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let Some(l) = self.lists.iter().find(|l| inside(l.rect, (x, y))) else { return Some((x, y)) };
        if l.shown == l.expected { return Some((x, y)) }
        let at = l.to_driver(y);
        (!l.holds(y) || l.holds(at)).then_some((x, at))
    }

    /// The same hit map without waiting for another physical mouse move.
    pub fn refresh_pointer(&mut self, point: (i32, i32), _now: f64) -> PointerRefresh {
        let Some((x, y)) = self.map(point.0, point.1) else {
            self.hover = None;
            return PointerRefresh::Gone;
        };
        if self.hover == Some((x, y)) { return PointerRefresh::Unchanged }
        self.hover = Some((x, y));
        PointerRefresh::Move { x, y }
    }
}

fn button_index(button: Button) -> u8 {
    match button {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
    }
}

#[cfg(test)]
mod tests;
