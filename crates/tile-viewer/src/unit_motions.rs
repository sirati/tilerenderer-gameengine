//! Units on their walks (`scene::Walk`), moved on the viewer's own clock at
//! its frame rate.
//!
//! The driver tells a unit's walk the moment each tile starts, straight to
//! the viewer and not with its next scene: the tiles started so far (when,
//! which way, how long at least) and how many more its path plans. The
//! scene draws the unit as free sprites at an anchor (`scene::Motion`); the
//! viewer moves them to where the walk has the unit now.
//!
//! A tile plays from when the game started it (on the driver's clock,
//! mapped through the earliest arrival seen: never ahead of the game).
//! Each tile ends when the next one starts if that is known, else, with
//! more planned, after the time the walk's tiles have been taking (the game
//! adds its own per-tile work to each 100 ms slide), and it eases in
//! towards the tile's end rather than stop there if the next one is late.
//! A known next start re-aims the tile from where it is, at most a little
//! faster than the slide ([`SPRINT`]): never back, never a jump. The last
//! tile (nothing more planned: a single step too) takes its slide's own
//! length. A walk that ends early stops where the game stopped, easing in.
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// How much faster than its slide a tile may be played to catch up.
pub(crate) const SPRINT: f64 = 1.3;
/// A tile is never shorter than the game's own slide: its last step comes
/// at 8/9 of the slide's length (`civ3_sys::unit_slide`).
const SLIDE_PART: f64 = 8.0 / 9.0;
/// A tile whose next start is not known yet is aimed at this much more
/// than the walk's tiles have been taking.
const SAFETY: f64 = 1.1;
/// A walk's tile before any walk was seen: this many times its slide.
const FIRST_TILE: f64 = 1.3;
/// The part of a tile played evenly before easing in towards its end,
/// while the next tile's start is not known.
const EVEN_PART: f64 = 0.8;
/// How long a walk's last tile waits for a late next tile before it ends,
/// in the walk's tiles.
const GIVE_UP: f64 = 4.0;
/// How many recent arrivals the clock is measured from.
const RECENT: usize = 64;
/// A walk whose last tile began this long before (ms) is not begun.
const STALE_MS: f64 = 1500.0;

/// A tile's progress from `p0` at `t0` to 1 at `end` (driver ms): evenly,
/// or, `open` (the next tile not known yet), evenly up to [`EVEN_PART`] of
/// the way and then easing in, never quite there.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Seg {
    t0: f64,
    p0: f64,
    end: f64,
    open: bool,
}

impl Seg {
    fn at(&self, t: f64) -> f64 {
        if t <= self.t0 {
            return self.p0;
        }
        let x = if self.end > self.t0 { (t - self.t0) / (self.end - self.t0) } else { 1.0 };
        let f = if !self.open || x <= EVEN_PART { x.min(1.0) } else { 1.0 - (1.0 - EVEN_PART) * (-(x - EVEN_PART) / (1.0 - EVEN_PART)).exp() };
        self.p0 + (1.0 - self.p0) * f
    }
}

/// A unit's walk as the viewer plays it.
#[derive(Debug)]
struct Play {
    serial: u64,
    /// The tile played and its progress.
    k: usize,
    seg: Seg,
    /// When tile `k`'s playing began (driver ms).
    tile_t0: f64,
    /// What `seg` was aimed with: tile `k + 1` known, more planned.
    aimed: (bool, bool),
    /// How far along the walk it was shown last, in tiles (never back).
    along: f64,
    /// Played to its end, and since when (driver ms).
    finished: Option<f64>,
    /// No longer applied (the scene has the unit at its end, or long after).
    done: bool,
}

#[derive(Default)]
pub(crate) struct UnitMotions {
    /// Recent `arrival - start_ms` (viewer ms minus driver ms).
    samples: VecDeque<f64>,
    /// Recent times from one walk tile's start to the next's, by slide
    /// length (driver ms).
    tile_ms: HashMap<u32, f64>,
    plays: HashMap<u32, Play>,
    /// The tiles' arrivals counted into `samples`, per walk: (serial, count).
    counted: HashMap<u32, (u64, usize)>,
    epoch: Option<Instant>,
    /// The offset of each moving unit's sprites this frame, by key.
    moved: HashMap<u32, (i32, i32)>,
    /// The offsets applied last: (sprite range, offset).
    applied: Vec<((u32, u32), (i32, i32))>,
}

/// A walk's point `along` tiles into it (world pixels, full size).
fn point(walk: &scene::Walk, along: f64) -> (i32, i32) {
    let (mut x, mut y) = (f64::from(walk.origin.0), f64::from(walk.origin.1));
    for (k, tile) in walk.tiles.iter().enumerate() {
        let p = (along - k as f64).clamp(0.0, 1.0);
        x += f64::from(tile.way.0) * p;
        y += f64::from(tile.way.1) * p;
    }
    (x.round() as i32, y.round() as i32)
}

impl UnitMotions {
    fn ms(&mut self, at: Instant) -> f64 {
        let epoch = *self.epoch.get_or_insert(at);
        if at >= epoch { at.duration_since(epoch).as_secs_f64() * 1000.0 } else { -(epoch.duration_since(at).as_secs_f64() * 1000.0) }
    }

    /// The time a walk's tile of slide length `ms` has been taking.
    fn typical(&self, ms: u32) -> f64 {
        self.tile_ms.get(&ms).copied().unwrap_or(f64::from(ms) * FIRST_TILE)
    }

    /// Where tile `k` of `walk` ends, aimed at driver time `t` with the tile
    /// at `p` (begun at `tile_t0`): (end, open).
    fn aim(&self, walk: &scene::Walk, k: usize, tile_t0: f64, t: f64, p: f64) -> (f64, bool) {
        let tile = walk.tiles[k];
        let ms = f64::from(tile.ms);
        let soonest = (tile.start_ms as f64 + ms * SLIDE_PART).max(t + (1.0 - p) * ms / SPRINT);
        match walk.tiles.get(k + 1) {
            Some(next) => ((next.start_ms as f64).max(soonest), false),
            None if walk.planned > 0 => ((tile_t0 + self.typical(tile.ms) * SAFETY).max(soonest), true),
            None => ((tile_t0 + ms).max(soonest), false),
        }
    }

    /// Plays the walk of `key` at driver time `t`: where it has the unit, if
    /// it is still played.
    fn play(&mut self, key: u32, walk: &scene::Walk, t: f64) -> Option<(i32, i32)> {
        if walk.tiles.is_empty() {
            return None;
        }
        if self.plays.get(&key).is_none_or(|p| p.serial != walk.serial) {
            // (A walk long over, met again when its unit is drawn to be
            // moved again, is not played again.)
            let last = walk.tiles[walk.tiles.len() - 1];
            if t > last.start_ms as f64 + f64::from(last.ms) + STALE_MS {
                return None;
            }
            // From the game's start, or from now if the unit was not drawn
            // to be moved before (its first tile then trails a little).
            let first = walk.tiles[0].start_ms as f64;
            let t0 = first.max(t - 17.0);
            let (end, open) = self.aim(walk, 0, t0, t0, 0.0);
            let aimed = (walk.tiles.len() > 1, walk.planned > 0);
            self.plays.insert(key, Play { serial: walk.serial, k: 0, seg: Seg { t0, p0: 0.0, end, open }, tile_t0: t0, aimed, along: 0.0, finished: None, done: false });
        }
        let mut play = self.plays.remove(&key)?;
        loop {
            let k = play.k;
            let knows = (walk.tiles.len() > k + 1, walk.planned > 0);
            if knows != play.aimed {
                // The next tile started, or the plan changed: re-aimed from
                // where the tile is now.
                let now = t.max(play.seg.t0);
                let p = play.seg.at(now);
                let (end, open) = self.aim(walk, k, play.tile_t0, now, p);
                play.seg = Seg { t0: now, p0: p, end, open };
                play.aimed = knows;
            }
            if !play.seg.open && t >= play.seg.end && walk.tiles.len() > k + 1 {
                // On to the next tile, where this one ended (or when the
                // game started it, if that is later).
                let t0 = play.seg.end.max(walk.tiles[k + 1].start_ms as f64);
                play.k += 1;
                play.tile_t0 = t0;
                play.aimed = (walk.tiles.len() > k + 2, walk.planned > 0);
                let (end, open) = self.aim(walk, k + 1, t0, t0, 0.0);
                play.seg = Seg { t0, p0: 0.0, end, open };
                continue;
            }
            if play.seg.open && t > play.tile_t0 + GIVE_UP * self.typical(walk.tiles[k].ms) {
                // The walk ended early: to the end of this tile, gently.
                let p = play.seg.at(t);
                play.seg = Seg { t0: t, p0: p, end: t + (1.0 - p) * f64::from(walk.tiles[k].ms), open: false };
                play.aimed = (false, false);
            }
            break;
        }
        play.along = play.along.max(play.k as f64 + play.seg.at(t));
        let at_end = !play.seg.open && t >= play.seg.end && walk.tiles.len() == play.k + 1;
        if at_end && play.finished.is_none() {
            play.finished = Some(t);
        }
        let shown = (!play.done).then(|| point(walk, play.along));
        self.plays.insert(key, play);
        shown
    }

    /// What to apply for the scene's motions at `now`: per sprite range,
    /// its offset, after the ranges moved last frame that no longer move,
    /// at (0, 0) (back where the scene has them). `None` when that is what
    /// was applied last and `rebuilt` (the world's sprites made again from
    /// the scene's) is false. `walk` gives a unit's walk and when each of
    /// its tiles arrived (`scene::Display::walk`).
    pub(crate) fn offsets<'w>(&mut self, scene: &scene::Scene, now: Instant, rebuilt: bool, walk: impl Fn(u32) -> Option<(&'w scene::Walk, &'w [Instant])>) -> Option<Vec<((u32, u32), (i32, i32))>> {
        let offsets = self.positions(scene, now, walk);
        if !rebuilt && offsets == self.applied {
            return None;
        }
        let mut out: Vec<_> = if rebuilt { Vec::new() } else {
            self.applied.iter().filter(|(range, _)| !offsets.iter().any(|(r, _)| r == range)).map(|&(range, _)| (range, (0, 0))).collect()
        };
        out.extend_from_slice(&offsets);
        self.applied = offsets;
        Some(out)
    }

    /// The offset of each of the scene's motions at `now`.
    pub(crate) fn positions<'w>(&mut self, scene: &scene::Scene, now: Instant, walk: impl Fn(u32) -> Option<(&'w scene::Walk, &'w [Instant])>) -> Vec<((u32, u32), (i32, i32))> {
        let now_ms = self.ms(now);
        // The clock: every tile's arrival, once.
        for m in &scene.motions {
            let Some((w, arrived)) = walk(m.key) else { continue };
            let counted = self.counted.get(&m.key).filter(|c| c.0 == w.serial).map_or(0, |c| c.1);
            for (i, at) in arrived.iter().enumerate().skip(counted).take(w.tiles.len().saturating_sub(counted)) {
                let at = self.ms(*at);
                if self.samples.len() == RECENT { self.samples.pop_front(); }
                self.samples.push_back(at - w.tiles[i].start_ms as f64);
                if i > 0 {
                    let gap = (w.tiles[i].start_ms - w.tiles[i - 1].start_ms) as f64;
                    let seen = self.tile_ms.entry(w.tiles[i].ms).or_insert(gap);
                    *seen = 0.7 * *seen + 0.3 * gap;
                }
            }
            self.counted.insert(m.key, (w.serial, arrived.len().min(w.tiles.len())));
        }
        let Some(offset) = self.samples.iter().copied().reduce(f64::min) else {
            self.moved.clear();
            return scene.motions.iter().map(|m| (m.sprites, (0, 0))).collect();
        };
        let t = now_ms - offset;
        self.plays.retain(|key, _| scene.motions.iter().any(|m| m.key == *key));
        self.moved.clear();
        let n = scene.sprites.len() as u32;
        let mut out = Vec::with_capacity(scene.motions.len());
        for m in &scene.motions {
            let range = (m.sprites.0.min(n), m.sprites.1.min(n));
            let at = walk(m.key).and_then(|(w, _)| self.play(m.key, w, t));
            let offset = match at {
                Some(p) => {
                    let play = self.plays.get_mut(&m.key).expect("played above");
                    // Over once the scene has the unit where the walk ended
                    // (or a while after: it moved on by other means).
                    if play.finished.is_some_and(|f| p == m.anchor || t > f + 2000.0) {
                        play.done = true;
                    }
                    (p.0 - m.anchor.0, p.1 - m.anchor.1)
                }
                None => (0, 0),
            };
            if offset != (0, 0) {
                self.moved.insert(m.key, offset);
            }
            out.push((range, offset));
        }
        out
    }

    /// The offset unit `key`'s sprites were moved by this frame, if any.
    pub(crate) fn moved(&self, key: u32) -> Option<(i32, i32)> {
        self.moved.get(&key).copied()
    }

    /// Where a marker over the scene's point of interest goes
    /// (`scene::Scene::focus_motion`): moved as its unit's sprites were
    /// this frame, in the pixels of views showing the world `downscale`
    /// times smaller (the focus's); else the focus.
    pub(crate) fn focus_at(&self, scene: &scene::Scene, downscale: u32) -> Option<(i32, i32)> {
        let Some(m) = scene.focus_motion else { return scene.focus };
        let n = downscale.max(1) as i32;
        let (dx, dy) = self.moved.get(&m.key).copied().filter(|_| m.key != 0).unwrap_or((0, 0));
        Some((m.from.0 + dx / n, m.from.1 + dy / n))
    }
}

/// Moves the world's sprites (`free`, the scene's sprites first, in order)
/// to where the scene puts them plus their motions' offsets.
pub(crate) fn apply(free: &mut [tile_render::PlacedSprite], sprites: &[scene::FreeSprite], offsets: &[((u32, u32), (i32, i32))]) {
    for &((start, end), (dx, dy)) in offsets {
        for i in start as usize..end as usize {
            if let (Some(placed), Some(s)) = (free.get_mut(i), sprites.get(i)) {
                placed.x = s.x + dx;
                placed.y = s.y + dy;
            }
        }
    }
}

#[cfg(test)]
mod tests;
