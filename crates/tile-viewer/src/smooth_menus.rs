//! Pixel scrolling for the driver's row-based lists. Wheel messages reach
//! the driver normally. When stable row ids confirm a scroll, its old and
//! new drawings slide through the list's clip rectangle. Input follows the
//! visible row's id, including while a click is held across a scroll. The
//! driver's pointer highlight stays on the row under the pointer
//! ([`highlight`]).

use scene::{Button, Input, InteractableKind, Scene};
use tile_render::PlacedSprite;

use crate::looks::Span;

mod highlight;
use highlight::Highlight;

/// Seconds for a confirmed row movement to settle.
const DURATION: f64 = 0.14;
/// Seconds after a slide that the highlight is kept under the pointer
/// until the driver's own drawing shows it there.
const SETTLE: f64 = 0.3;

type Rect = [i32; 4]; // left, top, right, bottom

fn bounds([x, y, w, h]: [i32; 4]) -> Rect { [x, y, x.saturating_add(w), y.saturating_add(h)] }
fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}
fn inside(r: Rect, (x, y): (i32, i32)) -> bool { x >= r[0] && x < r[2] && y >= r[1] && y < r[3] }
fn shifted(mut r: Rect, dy: i32) -> Rect { r[1] += dy; r[3] += dy; r }

fn clipped(mut s: PlacedSprite, r: Rect) -> Option<PlacedSprite> {
    s.clip = Some(intersect(s.clip.unwrap_or([-tile_render::world::CLIP_LIMIT, -tile_render::world::CLIP_LIMIT, tile_render::world::CLIP_LIMIT, tile_render::world::CLIP_LIMIT]), r)?);
    Some(s)
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

impl Picture {
    fn take(scene: &Scene, free: &[PlacedSprite], spans: &[Span], rect_of: &impl Fn(&PlacedSprite) -> Option<[i32; 4]>) -> Vec<Self> {
        let mut overlay_cameras = Vec::new();
        for camera in spans.iter().flat_map(|s| s.cameras.iter().copied()) {
            if !overlay_cameras.contains(&camera) { overlay_cameras.push(camera); }
        }
        scene.interactables.iter().filter_map(|list| {
            let InteractableKind::List { scroll, .. } = list.kind else { return None };
            let rect = bounds(scroll);
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
            Some(Picture { id: list.id, rect, rows, sprites, extents, cameras })
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

    fn displacement_from(&self, old: &Self) -> Option<i32> {
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
        movement.filter(|dy| *dy != 0 && dy.abs() < self.rect[3] - self.rect[1])
    }
}

#[derive(Clone, Debug)]
struct Slide { from: Picture, to: Picture, dy: i32, at: f64 }

impl Slide {
    fn offsets(&self, now: f64) -> (i32, i32) {
        let t = ((now - self.at) / DURATION).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let old = (self.dy as f64 * t).round() as i32;
        (old, old - self.dy)
    }

    /// (old drawing's region, new drawing's region). The new drawing fills
    /// the scroll rectangle except for the edge whose rows have already
    /// left the driver's list; the old drawing fills only that edge,
    /// without alpha overlap. What the driver redraws during a slide (its
    /// highlight) shows at once.
    fn regions(&self, now: f64) -> (Option<Rect>, Option<Rect>) {
        let (_, new) = self.offsets(now);
        let r = self.to.rect;
        if self.dy < 0 {
            let seam = r[1] + new;
            (intersect(r, [r[0], r[1], r[2], seam]), intersect(r, [r[0], seam, r[2], r[3]]))
        } else {
            let seam = r[3] + new;
            (intersect(r, [r[0], seam, r[2], r[3]]), intersect(r, [r[0], r[1], r[2], seam]))
        }
    }

    /// The drawing shown at `now`; with `style`, the driver's highlight
    /// moved to that row in both drawings.
    fn picture(&self, now: f64, style: Option<(&Highlight, u64)>) -> Picture {
        let (old, new) = self.offsets(now);
        let (old_clip, new_clip) = self.regions(now);
        let restyled = |p: &Picture| style.and_then(|(h, want)| highlight::restyle(p, h, want));
        let (from, to) = (restyled(&self.from), restyled(&self.to));
        let mut sprites = Vec::new();
        let mut extents = Vec::new();
        let mut rows = Vec::new();
        for (picture, dy, clip) in [(from.as_ref().unwrap_or(&self.from), old, old_clip), (to.as_ref().unwrap_or(&self.to), new, new_clip)] {
            let Some(clip) = clip else { continue };
            for (s, extent) in picture.sprites.iter().zip(&picture.extents) {
                let mut s = *s;
                s.y += dy;
                s.clip = s.clip.map(|r| shifted(r, dy));
                if let Some(s) = clipped(s, clip) {
                    sprites.push(s);
                    extents.push(shifted(*extent, dy));
                }
            }
            for row in &picture.rows {
                let rect = shifted(row.rect, dy);
                if intersect(rect, clip).is_some() && !rows.iter().any(|r: &Row| r.id == row.id) {
                    rows.push(Row { id: row.id, rect });
                }
            }
        }
        Picture { id: self.to.id, rect: self.to.rect, cameras: self.to.cameras.clone(), sprites, extents, rows }
    }

    fn row_at(&self, point: (i32, i32), now: f64) -> Option<&Row> {
        let (old, new) = self.offsets(now);
        let (old_clip, new_clip) = self.regions(now);
        let (picture, offset) = if old_clip.is_some_and(|r| inside(r, point)) { (&self.from, old) }
            else if new_clip.is_some_and(|r| inside(r, point)) { (&self.to, new) }
            else { return None };
        picture.row_at((point.0, point.1 - offset))
    }
}

struct Press {
    button: Button,
    list: u64,
    row: Option<u64>,
    /// Preserve ordinary coordinates when a static list never moved.
    original_rect: Option<Rect>,
}

/// State of independently enabled smooth menu scrolling. No wheel message
/// is subdivided or delayed; animation starts only on a confirmed movement.
#[derive(Default)]
pub struct SmoothMenus {
    pictures: Vec<Picture>,
    slides: Vec<Slide>,
    key: Option<(u64, Option<String>, Option<u32>)>,
    pressed: Vec<Press>,
    hover: Option<(i32, i32)>,
    /// Input uses the pixels of the last drawn frame, even with a low FPS cap.
    shown_at: f64,
    /// Each list's pointer highlight, as learned from the driver's drawing.
    highlights: Vec<(u64, Highlight)>,
    /// Lists whose highlight is kept under the pointer after a slide, until
    /// (seconds) or until the driver shows it there.
    settling: Vec<(u64, f64)>,
    /// A button or key reached the driver since the last frame: a change
    /// of its highlight may be a selection rather than the pointer's.
    interacted: bool,
}

/// The highlight of `slide`'s list, moved to the row its drawing shows
/// under `pointer` at `now`.
fn style<'a>(highlights: &'a [(u64, Highlight)], slide: &Slide, pointer: Option<(i32, i32)>, now: f64) -> Option<(&'a Highlight, u64)> {
    let (_, h) = highlights.iter().find(|(id, _)| *id == slide.to.id)?;
    Some((h, slide.row_at(pointer?, now)?.id))
}

/// Hover refresh while a list moves under a stationary mouse.
pub enum PointerRefresh {
    Unchanged,
    Move { x: i32, y: i32 },
    Gone,
}

impl SmoothMenus {
    /// Includes the last frame that restores the unmodified drawing.
    pub fn running(&self) -> bool { !self.slides.is_empty() || !self.settling.is_empty() }

    /// Clear a held gesture on focus loss. List pictures are retained so
    /// the next frame can restore an animation's ordinary drawing.
    pub fn focus_lost(&mut self) { self.pressed.clear(); self.hover = None; }

    /// Interpolate the current, unanimated free sprites. `generation`
    /// invalidates saved sprite ids when the renderer replaces its atlas.
    /// `pointer` is where the pointer reaches the driver's window (its
    /// pixels), if it does.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, scene: &Scene, free: &[PlacedSprite], spans: &[Span], now: f64, enabled: bool, generation: u64, pointer: Option<(i32, i32)>, rect_of: impl Fn(&PlacedSprite) -> Option<[i32; 4]>) -> Option<Vec<PlacedSprite>> {
        self.shown_at = now;
        let key = (generation, scene.interface.screen.clone(), scene.interface.popup);
        if !enabled || self.key.as_ref() != Some(&key) {
            self.pictures.clear();
            self.slides.clear();
            self.pressed.clear();
            self.highlights.clear();
            self.settling.clear();
            self.key = Some(key);
        }
        if !enabled { return None }
        let current = Picture::take(scene, free, spans, &rect_of);
        // The driver lit the row its pointer moved onto: its highlight.
        if let (false, Some(hover)) = (std::mem::take(&mut self.interacted), self.hover) {
            for picture in &current {
                let Some(old) = self.pictures.iter().find(|p| p.id == picture.id) else { continue };
                if let Some(h) = highlight::learn(old, picture, hover) {
                    self.highlights.retain(|(id, _)| *id != picture.id);
                    self.highlights.push((picture.id, h));
                }
            }
        }
        let mut slides = Vec::new();
        for picture in &current {
            let Some(old) = self.pictures.iter().find(|p| p.id == picture.id) else { continue };
            let previous = self.slides.iter().find(|s| s.to.id == picture.id);
            if let Some(dy) = picture.displacement_from(old) {
                let from = previous.map_or_else(|| old.clone(), |s| s.picture(now, style(&self.highlights, s, pointer, now)));
                // Repeated notches keep the currently visible position.
                if let Some(displacement) = picture.displacement_from(&from) {
                    slides.push(Slide { from, to: picture.clone(), dy: displacement, at: now });
                } else {
                    slides.push(Slide { from: old.clone(), to: picture.clone(), dy, at: now });
                }
            } else if let Some(previous) = previous.filter(|s| now - s.at < DURATION && s.to.rect == picture.rect && s.to.cameras == picture.cameras && picture.rows == old.rows) {
                // A redraw of hover/text does not restart the clock.
                slides.push(Slide { to: picture.clone(), ..previous.clone() });
            }
        }
        self.pictures = current;
        self.slides = slides;
        // What each list shows in place of the driver's drawing.
        let mut patches: Vec<(Rect, Vec<u32>, Vec<PlacedSprite>)> = Vec::new();
        for slide in &self.slides {
            let shown = slide.picture(now, style(&self.highlights, slide, pointer, now));
            patches.push((slide.to.rect, slide.to.cameras.clone(), shown.sprites));
            if self.highlights.iter().any(|(id, _)| *id == slide.to.id) {
                self.settling.retain(|(id, _)| *id != slide.to.id);
                self.settling.push((slide.to.id, slide.at + DURATION + SETTLE));
            }
        }
        // After a slide the highlight stays under the pointer until the
        // driver, which got the pointer over that row, shows it there too.
        let mut settled = Vec::new();
        for &(id, until) in &self.settling {
            if self.slides.iter().any(|s| s.to.id == id) { continue }
            let held = (now < until).then_some(()).and_then(|()| {
                let picture = self.pictures.iter().find(|p| p.id == id)?;
                let (_, h) = self.highlights.iter().find(|(list, _)| *list == id)?;
                let want = picture.row_at(pointer?)?.id;
                highlight::restyle(picture, h, want)
            });
            match held {
                Some(shown) => patches.push((shown.rect, shown.cameras, shown.sprites)),
                None => settled.push(id),
            }
        }
        self.settling.retain(|(id, _)| !settled.contains(id));
        if patches.is_empty() { return None }
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
                let r = s.clip.unwrap_or([-tile_render::world::CLIP_LIMIT, -tile_render::world::CLIP_LIMIT, tile_render::world::CLIP_LIMIT, tile_render::world::CLIP_LIMIT]);
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

    /// Map a displayed row to its current driver row. Rows that have
    /// already left the driver's list cannot be selected accidentally.
    /// Button releases retain the id chosen on the corresponding press.
    pub fn route(&mut self, input: Input, _now: f64) -> Option<Input> {
        if matches!(input, Input::MouseButton { .. } | Input::Key { .. }) { self.interacted = true; }
        let result = self.map_input(input, self.shown_at);
        if let Some(Input::MouseMove { x, y }) = result { self.hover = Some((x, y)); }
        result
    }

    /// The same hit map without waiting for another physical mouse move.
    pub fn refresh_pointer(&mut self, point: (i32, i32), _now: f64) -> PointerRefresh {
        let Some(Input::MouseMove { x, y }) = self.map_input(Input::MouseMove { x: point.0, y: point.1 }, self.shown_at) else {
            self.hover = None;
            return PointerRefresh::Gone;
        };
        if self.hover == Some((x, y)) { return PointerRefresh::Unchanged }
        self.hover = Some((x, y));
        PointerRefresh::Move { x, y }
    }

    fn map_input(&mut self, input: Input, now: f64) -> Option<Input> {
        let (x, y) = match input {
            Input::MouseMove { x, y } | Input::MouseButton { x, y, .. } => (x, y),
            _ => return Some(input),
        };
        if let Input::MouseButton { button, down: false, .. } = input {
            if let Some(k) = self.pressed.iter().position(|p| p.button == button) {
                let Press { list, row, original_rect, .. } = self.pressed.remove(k);
                let row = row?; // A press on a departed row was suppressed.
                let Some(picture) = self.pictures.iter().find(|p| p.id == list) else {
                    return Some(Input::MouseButton { button, down: false, x: -1, y: -1 });
                };
                let slide = self.slides.iter().find(|s| s.to.id == list && now - s.at < DURATION);
                let Some(current) = picture.rows.iter().find(|r| r.id == row) else {
                    return Some(Input::MouseButton { button, down: false, x: -1, y: -1 });
                };
                if slide.is_none() && original_rect == Some(current.rect) { return Some(input); }
                let shown = slide.and_then(|s| s.row_at((x, y), now)).map(|r| r.id)
                    .or_else(|| slide.is_none().then(|| picture.row_at((x, y)).map(|r| r.id)).flatten());
                let (x, y) = if shown == Some(row) {
                    (x.clamp(picture.rect[0], picture.rect[2] - 1), (current.rect[1] + current.rect[3]) / 2)
                } else {
                    // Release outside the pressed row cancels its click.
                    (current.rect[0] - 1, current.rect[1] - 1)
                };
                return Some(Input::MouseButton { button, down: false, x, y });
            }
        }
        let Some(slide) = self.slides.iter().find(|s| now - s.at < DURATION && inside(s.to.rect, (x, y))) else {
            if let Input::MouseButton { button, down: true, .. } = input {
                self.pressed.retain(|p| p.button != button);
                if let Some((picture, row)) = self.pictures.iter().filter(|p| inside(p.rect, (x, y)))
                    .find_map(|p| p.row_at((x, y)).map(|r| (p, r))) {
                    // A wheel event may start a slide while this button is
                    // held. Remember the row before that can happen.
                    self.pressed.push(Press { button, list: picture.id, row: Some(row.id), original_rect: Some(row.rect) });
                }
            }
            return Some(input);
        };
        let shown = slide.row_at((x, y), now);
        if shown.is_none() {
            // A gap in the displayed rows must not click a different row
            // that already occupies this coordinate in the native list.
            if let Input::MouseButton { button, down: true, .. } = input {
                self.pressed.retain(|p| p.button != button);
                self.pressed.push(Press { button, list: slide.to.id, row: None, original_rect: None });
            }
            return None;
        }
        let current = shown.and_then(|shown| slide.to.rows.iter().find(|row| row.id == shown.id));
        if let Input::MouseButton { button, down: true, .. } = input {
            self.pressed.retain(|p| p.button != button);
            self.pressed.push(Press { button, list: slide.to.id, row: current.map(|row| row.id), original_rect: None });
        }
        let shown = shown?;
        let current = current?;
        // Keep the horizontal point and the within-row vertical fraction.
        let height = (shown.rect[3] - shown.rect[1]).max(1);
        let (old, new) = slide.offsets(now);
        let offset = if slide.regions(now).0.is_some_and(|r| inside(r, (x, y))) { old } else { new };
        let fraction = (y - shown.rect[1] - offset).clamp(0, height - 1);
        let y = current.rect[1] + fraction.min(current.rect[3] - current.rect[1] - 1);
        Some(match input {
            Input::MouseMove { .. } => Input::MouseMove { x, y },
            Input::MouseButton { button, down, .. } => Input::MouseButton { button, down, x, y },
            _ => unreachable!(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scene::{Interactable, ViewState};
    use tile_render::{SpriteSetId, ZoomArt};

    fn frame(first: u64) -> (Scene, Vec<PlacedSprite>, Vec<Span>) {
        let mut scene = Scene::default();
        scene.views.push(ViewState { id: 7, ..Default::default() });
        scene.interactables.push(Interactable { region: None, alt_right_click: false, anchor: None, id: 99, rect: [10, 10, 100, 60], kind: InteractableKind::List { scroll: [10, 10, 100, 60], step: 20 } });
        let mut free = Vec::new();
        for k in 0..3 {
            let id = first + k;
            let y = 10 + k as i32 * 20;
            scene.interactables.push(Interactable { region: None, alt_right_click: false, anchor: None, id, rect: [10, y, 100, 20], kind: InteractableKind::ListItem });
            free.push(PlacedSprite { screen: true, view: Some(0), clip: Some([10, 10, 110, 70]), ..PlacedSprite::new(SpriteSetId::from_raw(id as u32), 10, y) });
        }
        let spans = vec![Span { view: 7, start: 0, len: free.len(), cameras: vec![0; free.len()] }];
        (scene, free, spans)
    }

    fn rect(s: &PlacedSprite) -> Option<[i32; 4]> {
        let r = bounds([s.x, s.y, 100, 20]);
        let r = s.clip.map_or(Some(r), |c| intersect(r, c))?;
        Some([r[0], r[1], r[2] - r[0], r[3] - r[1]])
    }

    fn draw(state: &mut SmoothMenus, first: u64, now: f64, enabled: bool) -> Vec<PlacedSprite> {
        let (scene, free, spans) = frame(first);
        state.draw(&scene, &free, &spans, now, enabled, 1, None, rect).unwrap_or(free)
    }

    fn pixel(sprites: &[PlacedSprite], at: (i32, i32)) -> Option<u32> {
        sprites.iter().filter(|s| s.screen && rect(s).is_some_and(|r| inside(bounds(r), at))).last().map(|s| s.set.raw())
    }

    #[test]
    fn down_scroll_moves_actual_pixels_between_confirmed_rows() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        let first = draw(&mut state, 2, 1.0, true);
        assert_eq!(pixel(&first, (20, 15)), Some(1));
        let halfway = draw(&mut state, 2, 1.0 + DURATION / 2.0, true);
        assert_eq!(pixel(&halfway, (20, 15)), Some(1));
        assert_eq!(pixel(&halfway, (20, 25)), Some(2));
        assert_eq!(pixel(&halfway, (20, 45)), Some(3));
        assert_eq!(pixel(&halfway, (20, 65)), Some(4));
        let end = draw(&mut state, 2, 1.0 + DURATION, true);
        assert_eq!(pixel(&end, (20, 15)), Some(2));
        assert_eq!(pixel(&end, (20, 65)), Some(4));
        assert!(!state.running());
    }

    #[test]
    fn up_scroll_reveals_new_rows_at_the_top_and_clips_both_edges() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 2, 0.0, true);
        draw(&mut state, 1, 1.0, true);
        let halfway = draw(&mut state, 1, 1.0 + DURATION / 2.0, true);
        assert_eq!(pixel(&halfway, (20, 15)), Some(1));
        assert_eq!(pixel(&halfway, (20, 25)), Some(2));
        assert_eq!(pixel(&halfway, (20, 65)), Some(4));
        assert_eq!(pixel(&halfway, (20, 9)), None);
        assert_eq!(pixel(&halfway, (20, 70)), None);
    }

    #[test]
    fn repeated_wheel_steps_start_from_the_pixels_currently_visible() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        draw(&mut state, 2, 1.0, true);
        let at = 1.0 + DURATION / 2.0;
        let before = draw(&mut state, 2, at, true);
        let after = draw(&mut state, 3, at, true);
        for y in 10..70 {
            assert_eq!(pixel(&before, (20, y)), pixel(&after, (20, y)), "jumped at y={y}");
        }
        let end = draw(&mut state, 3, at + DURATION, true);
        assert_eq!(pixel(&end, (20, 15)), Some(3));
    }

    #[test]
    fn pointer_maps_to_the_row_the_animation_actually_shows() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        draw(&mut state, 2, 1.0, true);
        let at = 1.0 + DURATION / 2.0;
        draw(&mut state, 2, at, true);
        assert_eq!(state.route(Input::MouseMove { x: 20, y: 25 }, at), Some(Input::MouseMove { x: 20, y: 15 }));
        assert_eq!(state.route(Input::MouseMove { x: 20, y: 65 }, at), Some(Input::MouseMove { x: 20, y: 55 }));
        // The partly departing row is gone in the driver. Its neighbour
        // must never receive a click intended for that row.
        assert_eq!(state.route(Input::MouseButton { button: Button::Left, down: true, x: 20, y: 15 }, at), None);
        assert_eq!(state.route(Input::MouseButton { button: Button::Left, down: false, x: 20, y: 15 }, at), None);
    }

    #[test]
    fn click_release_keeps_the_pressed_row_and_dragging_off_cancels_it() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        draw(&mut state, 2, 1.0, true);
        let at = 1.0 + DURATION / 2.0;
        draw(&mut state, 2, at, true);
        assert_eq!(state.route(Input::MouseButton { button: Button::Left, down: true, x: 20, y: 25 }, at),
            Some(Input::MouseButton { button: Button::Left, down: true, x: 20, y: 15 }));
        let release = state.route(Input::MouseButton { button: Button::Left, down: false, x: 20, y: 25 }, at).unwrap();
        assert_eq!(release, Input::MouseButton { button: Button::Left, down: false, x: 20, y: 20 });
        state.route(Input::MouseButton { button: Button::Left, down: true, x: 20, y: 25 }, at);
        let release = state.route(Input::MouseButton { button: Button::Left, down: false, x: 200, y: 200 }, at).unwrap();
        let Input::MouseButton { x, y, .. } = release else { panic!() };
        assert!(!inside([10, 10, 110, 30], (x, y)));
    }

    #[test]
    fn wheel_messages_are_never_delayed_changed_or_subdivided() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        draw(&mut state, 2, 1.0, true);
        for delta in [-360, -120, -1, 1, 120, 360] {
            let input = Input::Wheel { delta, x: 20, y: 25 };
            assert_eq!(state.route(input.clone(), 1.01), Some(input));
        }
    }

    #[test]
    fn option_is_independent_and_disabling_restores_the_drivers_drawing() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, false);
        let plain = draw(&mut state, 2, 1.0, false);
        assert_eq!(pixel(&plain, (20, 15)), Some(2));
        assert!(!state.running());
        draw(&mut state, 2, 2.0, true);
        draw(&mut state, 3, 3.0, true);
        assert!(state.running());
        let plain = draw(&mut state, 3, 3.01, false);
        assert_eq!(pixel(&plain, (20, 15)), Some(3));
        assert!(!state.running());
    }

    #[test]
    fn unchanged_driver_redraws_do_not_restart_the_animation() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        draw(&mut state, 2, 1.0, true);
        for k in 1..20 { draw(&mut state, 2, 1.0 + DURATION * k as f64 / 20.0, true); }
        draw(&mut state, 2, 1.0 + DURATION, true);
        assert!(!state.running());
    }

    #[test]
    fn popup_and_alt_mode_changes_drop_saved_animation_and_held_rows() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        draw(&mut state, 2, 1.0, true);
        state.route(Input::MouseButton { button: Button::Left, down: true, x: 20, y: 35 }, 1.0);
        assert!(!state.pressed.is_empty());
        state.focus_lost();
        assert!(state.pressed.is_empty());
        let (mut scene, free, spans) = frame(2);
        scene.interface.popup = Some(123);
        assert!(state.draw(&scene, &free, &spans, 1.01, true, 1, None, rect).is_none());
        assert!(!state.running());
        let (mut scene, free, spans) = frame(3);
        scene.interface.popup = Some(123);
        assert!(state.draw(&scene, &free, &spans, 1.02, true, 1, None, rect).is_some());
        // App disables menu interpolation while Alt snapping owns input.
        assert!(state.draw(&scene, &free, &spans, 1.03, false, 1, None, rect).is_none());
        assert!(!state.running());
        assert!(state.draw(&scene, &free, &spans, 1.04, true, 1, None, rect).is_none());
    }

    #[test]
    fn scene_changes_and_unrelated_row_replacement_do_not_replay_old_rows() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        draw(&mut state, 2, 1.0, true);
        let (mut scene, free, spans) = frame(20);
        assert!(state.draw(&scene, &free, &spans, 1.01, true, 1, None, rect).is_none());
        scene.interface.screen = Some("Different menu".into());
        assert!(state.draw(&scene, &free, &spans, 1.02, true, 1, None, rect).is_none());
        assert!(state.draw(&scene, &free, &spans, 1.03, true, 2, None, rect).is_none());
        scene.interactables.clear();
        assert!(state.draw(&scene, &free, &spans, 1.04, true, 2, None, rect).is_none());
    }

    #[test]
    fn map_sprites_and_controls_outside_the_list_stay_unchanged() {
        let mut state = SmoothMenus::default();
        let (scene, mut free, spans) = frame(1);
        let world = PlacedSprite { view: None, screen: false, ..free[0] };
        let button = PlacedSprite { x: 150, y: 150, clip: Some([150, 150, 250, 170]), zoom_art: ZoomArt::Any, ..free[0] };
        free.extend([world, button]);
        state.draw(&scene, &free, &spans, 0.0, true, 1, None, rect);
        let (scene, mut free, spans) = frame(2);
        free.extend([world, button]);
        let animated = state.draw(&scene, &free, &spans, 1.0, true, 1, None, rect).unwrap();
        assert!(animated.contains(&world));
        assert!(animated.contains(&button));
    }

    #[test]
    fn a_press_before_scrolling_gets_a_cancelled_release_if_its_row_leaves() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        let press = Input::MouseButton { button: Button::Left, down: true, x: 20, y: 15 };
        assert_eq!(state.route(press.clone(), 0.0), Some(press));
        draw(&mut state, 2, 1.0, true);
        // Row 1 remains visible at the start of the animation, but has
        // left the native list. The native press still needs a release.
        assert_eq!(state.route(Input::MouseButton { button: Button::Left, down: false, x: 20, y: 15 }, 1.0),
            Some(Input::MouseButton { button: Button::Left, down: false, x: -1, y: -1 }));
    }

    #[test]
    fn a_press_before_scrolling_keeps_its_row_when_the_release_is_over_another_row() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        state.route(Input::MouseButton { button: Button::Left, down: true, x: 20, y: 45 }, 0.0);
        draw(&mut state, 2, 1.0, true);
        draw(&mut state, 2, 1.0 + DURATION / 2.0, true);
        // The pointer now covers row 3, not the originally pressed row 2.
        assert_eq!(state.route(Input::MouseButton { button: Button::Left, down: false, x: 20, y: 45 }, 1.1),
            Some(Input::MouseButton { button: Button::Left, down: false, x: 9, y: 9 }));
    }

    #[test]
    fn a_stationary_list_keeps_ordinary_button_coordinates_unchanged() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        for (down, x, y) in [(true, 23, 35), (false, 25, 39), (true, 23, 35), (false, 200, 200)] {
            let input = Input::MouseButton { button: Button::Left, down, x, y };
            assert_eq!(state.route(input.clone(), 0.0), Some(input));
        }
    }

    #[test]
    fn a_forwarded_press_is_released_if_another_scroll_removes_its_row() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        draw(&mut state, 2, 1.0, true);
        let at = 1.0 + DURATION / 2.0;
        draw(&mut state, 2, at, true);
        assert!(state.route(Input::MouseButton { button: Button::Left, down: true, x: 20, y: 25 }, at).is_some());
        draw(&mut state, 3, at + 0.01, true);
        assert_eq!(state.route(Input::MouseButton { button: Button::Left, down: false, x: 20, y: 25 }, at + 0.01),
            Some(Input::MouseButton { button: Button::Left, down: false, x: -1, y: -1 }));
    }

    #[test]
    fn hit_maps_follow_the_last_drawn_pixels_with_a_low_frame_rate_cap() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        draw(&mut state, 2, 1.0, true);
        // No subsequent frame has been drawn. Although the animation's
        // duration has elapsed, the user still sees the initial pixels.
        assert_eq!(state.route(Input::MouseMove { x: 20, y: 35 }, 2.0), Some(Input::MouseMove { x: 20, y: 15 }));
    }

    #[test]
    fn stationary_hover_is_refreshed_during_the_slide_and_when_it_settles() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        state.route(Input::MouseMove { x: 20, y: 35 }, 0.0);
        draw(&mut state, 2, 1.0, true);
        draw(&mut state, 2, 1.0 + DURATION / 2.0, true);
        assert!(matches!(state.refresh_pointer((20, 35), 1.0), PointerRefresh::Move { x: 20, y: 25 }));
        assert!(matches!(state.refresh_pointer((20, 35), 1.0), PointerRefresh::Unchanged));
        draw(&mut state, 2, 1.0 + DURATION, true);
        assert!(matches!(state.refresh_pointer((20, 35), 1.0), PointerRefresh::Move { x: 20, y: 35 }));
    }

    #[test]
    #[ignore = "requires a GPU; exercises actual scaled and clipped menu pixels"]
    fn gpu_draws_intermediate_menu_rows_with_matching_scaled_clips() {
        use tile_render::{AtlasBuilder, Camera, Grid, Headless, Layout, RgbaImage, SpriteSets, TileRenderer, TileWorld};
        let gpu = Headless::new().expect("AMD GPU");
        let mut atlas = AtlasBuilder::new(256);
        let mut sets = SpriteSets::default();
        let mut art = Vec::new();
        for id in 0..5u8 {
            art.push(sets.single(atlas.add(&RgbaImage::from_fn(100, 20, |x, y| [id * 40, y as u8 * 8, x as u8 * 2, 255]), (0, 0)).unwrap()));
        }
        let atlas = atlas.finish();
        let mut world = TileWorld::new(Grid { cols: 1, rows: 1, layout: Layout::Orthogonal, step: (1, 1), wrap_x: false, wrap_y: false }, sets);
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        draw(&mut state, 2, 1.0, true);
        world.free = draw(&mut state, 2, 1.0 + DURATION / 2.0, true);
        for sprite in &mut world.free { sprite.set = art[sprite.set.raw() as usize]; }
        let mut renderer = TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
        renderer.set_atlas(&gpu.device, &gpu.queue, &atlas);
        renderer.set_world(&gpu.device, &world);
        for zoom in [(1, 1), (2, 1), (3, 2)] {
            let camera = Camera { zoom, ..Camera::new((240, 160)) };
            let actual = gpu.render(&mut renderer, &camera, 0, [0, 0, 0, 255]);
            let expected = tile_render::reference::render(&world, &atlas, &camera, 0, [0, 0, 0, 255]);
            assert_eq!(actual, expected, "intermediate list pixels at zoom {zoom:?}");
            // At 1:1 row 2 is halfway between its old and new slots. Its
            // original source y=5, neither endpoint's source pixel.
            if zoom == (1, 1) {
                assert_eq!(actual.get(20, 25), [80, 40, 20, 255]);
                assert_eq!(actual.get(20, 65), [160, 40, 20, 255]);
                assert_eq!(actual.get(20, 9), [0, 0, 0, 255]);
                assert_eq!(actual.get(20, 70), [0, 0, 0, 255]);
            }
        }
    }


    #[test]
    fn every_table_column_moves_and_routes_to_its_visible_native_row() {
        let mut state = SmoothMenus::default();
        let table = |first| {
            let (mut scene, mut sprites, spans) = frame(first);
            for item in &mut scene.interactables {
                if item.kind == InteractableKind::ListItem { item.rect[2] = 25; }
            }
            for row in 0..3 {
                scene.interactables.push(Interactable { region: None, alt_right_click: false, anchor: None,
                    id: 1000 + first + row, rect: [65, 10 + row as i32 * 20, 30, 20], kind: InteractableKind::Button });
            }
            // Each row sprite spans names, statistics and clickable cells.
            for sprite in &mut sprites { sprite.clip = Some([10, 10, 110, 70]); }
            (scene, sprites, spans)
        };
        let (scene, free, spans) = table(1);
        state.draw(&scene, &free, &spans, 0.0, true, 1, None, rect);
        let (scene, free, spans) = table(2);
        state.draw(&scene, &free, &spans, 1.0, true, 1, None, rect);
        let now = 1.0 + DURATION / 2.0;
        let shown = state.draw(&scene, &free, &spans, now, true, 1, None, rect).unwrap();
        for x in [20, 50, 80, 105] {
            assert_eq!(pixel(&shown, (x, 25)), Some(2), "column {x}");
            assert_eq!(state.route(Input::MouseMove { x, y: 25 }, now), Some(Input::MouseMove { x, y: 15 }));
            assert_eq!(state.route(Input::MouseButton { button: Button::Right, down: true, x, y: 15 }, now), None);
            assert_eq!(state.route(Input::MouseButton { button: Button::Right, down: false, x, y: 15 }, now), None);
        }
    }

    #[test]
    fn animated_row_gaps_do_not_hit_a_native_row_at_another_offset() {
        let mut state = SmoothMenus::default();
        for (first, now) in [(1, 0.0), (2, 1.0), (2, 1.0 + DURATION / 2.0)] {
            let (mut scene, free, spans) = frame(first);
            for item in &mut scene.interactables {
                if item.kind == InteractableKind::ListItem { item.rect[3] = 16; }
            }
            state.draw(&scene, &free, &spans, now, true, 1, None, rect);
        }
        // Old row 2 occupies y20..36, then a four-pixel gap. Native
        // row 3 already occupies y30..46 at this same coordinate.
        for down in [true, false] {
            assert_eq!(state.route(Input::MouseButton { button: Button::Left, down, x: 20, y: 38 }, 1.07), None);
        }
    }

    #[test]
    fn multicolumn_lists_keep_distinct_native_row_targets() {
        let (mut scene, free, spans) = frame(1);
        for row in scene.interactables.iter_mut().filter(|i| i.kind == InteractableKind::ListItem) { row.rect[2] = 40; }
        scene.interactables.push(Interactable { region: None, alt_right_click: false, anchor: None, id: 500,
            rect: [65, 10, 40, 20], kind: InteractableKind::ListItem });
        let picture = Picture::take(&scene, &free, &spans, &rect).remove(0);
        assert_eq!(picture.row_at((20, 15)).map(|r| r.id), Some(1));
        assert_eq!(picture.row_at((80, 15)).map(|r| r.id), Some(500));
        assert!(picture.row_at((60, 15)).is_none());
        assert_eq!(picture.row_at((80, 35)).map(|r| r.id), Some(2));
    }

    #[test]
    fn coalesced_whole_pages_and_endpoints_do_not_leave_an_animation_backlog() {
        let mut state = SmoothMenus::default();
        draw(&mut state, 1, 0.0, true);
        let shown = draw(&mut state, 8, 1.0, true);
        assert_eq!(pixel(&shown, (20, 15)), Some(8));
        assert!(!state.running());
        draw(&mut state, 9, 2.0, true);
        draw(&mut state, 9, 2.0 + DURATION, true);
        for at in [3.0, 4.0, 5.0] {
            assert_eq!(pixel(&draw(&mut state, 9, at, true), (20, 15)), Some(9));
            assert!(!state.running());
        }
    }

    #[test]
    fn the_lists_side_controls_stay_still_and_keep_their_input() {
        let mut state = SmoothMenus::default();
        let side = PlacedSprite { screen: true, view: Some(0), clip: Some([95, 15, 105, 25]),
            ..PlacedSprite::new(SpriteSetId::from_raw(8), 95, 15) };
        for (first, at) in [(1, 0.0), (2, 1.0)] {
            let (mut scene, mut free, spans) = frame(first);
            for item in &mut scene.interactables {
                if item.kind == InteractableKind::ListItem { item.rect[2] = 80; }
                if let InteractableKind::List { scroll, .. } = &mut item.kind { scroll[2] = 80; }
            }
            free.push(side);
            let output = state.draw(&scene, &free, &spans, at, true, 1, None, rect);
            if first == 2 { assert!(output.unwrap().contains(&side)); }
        }
        let input = Input::MouseButton { button: Button::Left, down: true, x: 100, y: 20 };
        assert_eq!(state.route(input.clone(), 1.01), Some(input));
    }

    const PLAIN: [u8; 3] = [0, 0, 0];
    const LIT: [u8; 3] = [0, 112, 184];

    /// [`frame`] with the driver's highlight on row `lit`: a fill under it
    /// and its text in another colour.
    fn lit_frame(first: u64, lit: Option<u64>) -> (Scene, Vec<PlacedSprite>, Vec<Span>) {
        let (scene, free, _) = frame(first);
        let mut out = Vec::new();
        for s in free {
            let on = lit == Some(u64::from(s.set.raw()));
            if on {
                let fill = tile_render::SolidRect { width: 100, height: 20, rgba: [40, 40, 120, 255] };
                out.push(PlacedSprite { set: SpriteSetId::from_raw(0), solid: Some(fill), ..s });
            }
            out.push(PlacedSprite { blend: tile_render::Blend::Subpixel { color: if on { LIT } else { PLAIN } }, ..s });
        }
        let spans = vec![Span { view: 7, start: 0, len: out.len(), cameras: vec![0; out.len()] }];
        (scene, out, spans)
    }

    fn lit_draw(state: &mut SmoothMenus, first: u64, lit: Option<u64>, now: f64, pointer: Option<(i32, i32)>) -> Option<Vec<PlacedSprite>> {
        let (scene, free, spans) = lit_frame(first, lit);
        state.draw(&scene, &free, &spans, now, true, 1, pointer, rect)
    }

    /// Where the highlight shows: the fills' visible rows, and the rows
    /// whose text has the highlight's colour.
    fn highlight_shown(sprites: &[PlacedSprite]) -> (Vec<[i32; 2]>, Vec<u32>) {
        let fills = sprites.iter().filter(|s| s.solid.is_some()).filter_map(rect).map(|r| [r[1], r[1] + r[3]]).collect();
        let mut text: Vec<u32> = sprites.iter().filter(|s| s.blend == tile_render::Blend::Subpixel { color: LIT }).map(|s| s.set.raw()).collect();
        text.dedup();
        (fills, text)
    }

    /// The driver lights row 2 as its pointer reaches it (y 35).
    fn learned() -> SmoothMenus {
        let mut state = SmoothMenus::default();
        lit_draw(&mut state, 1, None, 0.0, Some((20, 35)));
        state.route(Input::MouseMove { x: 20, y: 35 }, 0.0);
        assert!(lit_draw(&mut state, 1, Some(2), 0.1, Some((20, 35))).is_none());
        assert_eq!(state.highlights.len(), 1);
        state
    }

    #[test]
    fn the_highlight_stays_under_a_stationary_pointer_in_every_frame_of_a_slide() {
        for (pointer, step) in [((20, 35), 2), ((20, 35), 0), ((20, 61), 2), ((20, 61), 0), ((20, 14), 2), ((20, 14), 0)] {
            let mut state = learned();
            // A wheel notch down: rows move up by one. The driver still
            // lights row 2, now one slot higher.
            let (first, lit) = if step == 2 { (2, Some(2)) } else { (0, Some(2)) };
            for k in 0..=20 {
                let now = 1.0 + DURATION * k as f64 / 20.0;
                let shown = lit_draw(&mut state, first, lit, now, Some(pointer)).unwrap_or_else(|| lit_frame(first, lit).1);
                let (fills, text) = highlight_shown(&shown);
                let under = state.slides.first().and_then(|s| s.row_at(pointer, now)).map(|r| r.id)
                    .or_else(|| state.pictures[0].row_at(pointer).map(|r| r.id));
                let under = under.expect("the pointer is over a row");
                assert_eq!(text, vec![under as u32], "pointer {pointer:?}, frame {k}");
                assert!(!fills.is_empty() && fills.iter().all(|f| f[0] <= pointer.1 && pointer.1 < f[1]),
                    "pointer {pointer:?}, frame {k}: fills {fills:?}");
            }
        }
    }

    #[test]
    fn after_a_slide_the_highlight_waits_for_the_driver_then_hands_back_without_a_jump() {
        let mut state = learned();
        lit_draw(&mut state, 2, Some(2), 1.0, Some((20, 35)));
        let end = 1.0 + DURATION + 0.01;
        // The driver has not drawn its pointer's new row yet.
        let shown = lit_draw(&mut state, 2, Some(2), end, Some((20, 35))).unwrap();
        assert_eq!(highlight_shown(&shown), (vec![[30, 50]], vec![3]));
        assert!(state.running());
        // Clicks go where the highlight is: native row 3.
        assert_eq!(state.route(Input::MouseMove { x: 20, y: 35 }, end), Some(Input::MouseMove { x: 20, y: 35 }));
        // The driver shows it: its own drawing again, the same pixels.
        assert!(lit_draw(&mut state, 2, Some(3), end + 0.02, Some((20, 35))).is_none());
        assert!(!state.running());
        // A driver that never catches up gets its drawing back in time.
        let mut state = learned();
        lit_draw(&mut state, 2, Some(2), 1.0, Some((20, 35)));
        assert!(lit_draw(&mut state, 2, Some(2), end, Some((20, 35))).is_some());
        assert!(lit_draw(&mut state, 2, Some(2), 1.0 + DURATION + SETTLE + 0.01, Some((20, 35))).is_none());
        assert!(!state.running());
    }

    #[test]
    fn a_highlight_changed_by_a_click_or_away_from_the_pointer_is_not_moved() {
        // Selected by a click: a selection, which need not follow the pointer.
        let mut state = SmoothMenus::default();
        lit_draw(&mut state, 1, None, 0.0, Some((20, 35)));
        state.route(Input::MouseMove { x: 20, y: 35 }, 0.0);
        state.route(Input::MouseButton { button: Button::Left, down: true, x: 20, y: 35 }, 0.0);
        lit_draw(&mut state, 1, Some(2), 0.1, Some((20, 35)));
        assert!(state.highlights.is_empty());
        // Lit away from the driver's pointer (keyboard or the driver's own choice).
        let mut state = SmoothMenus::default();
        lit_draw(&mut state, 1, None, 0.0, Some((20, 35)));
        state.route(Input::MouseMove { x: 20, y: 55 }, 0.0);
        lit_draw(&mut state, 1, Some(2), 0.1, Some((20, 55)));
        assert!(state.highlights.is_empty());
        // Unlearned, a slide shows the driver's newest drawing as it is.
        lit_draw(&mut state, 2, Some(2), 1.0, Some((20, 35)));
        let shown = lit_draw(&mut state, 2, Some(2), 1.0 + DURATION * 0.75, Some((20, 35))).unwrap();
        let (fills, text) = highlight_shown(&shown);
        assert_eq!(text, vec![2]);
        assert!(fills.iter().all(|f| f[1] <= 35), "the driver's highlight moves with its row: {fills:?}");
    }

    #[test]
    fn a_pointer_outside_the_rows_leaves_the_drivers_highlight_alone() {
        let mut state = learned();
        lit_draw(&mut state, 2, Some(2), 1.0, Some((200, 35)));
        let shown = lit_draw(&mut state, 2, Some(2), 1.0 + DURATION / 2.0, Some((200, 35))).unwrap();
        assert_eq!(highlight_shown(&shown).1, vec![2]);
        let shown = lit_draw(&mut state, 2, Some(2), 1.0 + DURATION / 2.0, None).unwrap();
        assert_eq!(highlight_shown(&shown).1, vec![2]);
    }
}
