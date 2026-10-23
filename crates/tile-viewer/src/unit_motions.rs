//! World sprites on their way (`scene::Motion`: units sliding to the next
//! tile), moved on the viewer's own clock at its frame rate, not at the rate
//! the driver publishes. Each telling's clock starts when its batch arrived
//! (`scene::Display::motion_arrival`; else when a frame first shows it),
//! less what had passed of it as the driver sent it; resending it changes
//! nothing.
use std::{collections::HashMap, time::{Duration, Instant}};

#[derive(Default)]
pub(crate) struct UnitMotions {
    started: HashMap<u64, Instant>,
    /// The offsets applied last: (sprite range, offset).
    applied: Vec<((u32, u32), (i32, i32))>,
}

/// How far a motion along `way` over `duration_ms` has come after
/// `elapsed` (rounded towards its start; all of it once done).
pub(crate) fn offset_at(way: (i32, i32), duration_ms: u32, elapsed: Duration) -> (i32, i32) {
    let total = u64::from(duration_ms) * 1000;
    let t = (elapsed.as_micros() as u64).min(total);
    if total == 0 || t == total {
        return way;
    }
    let part = |w: i32| (i64::from(w) * t as i64 / total as i64) as i32;
    (part(way.0), part(way.1))
}

impl UnitMotions {
    /// What to apply for the scene's motions at `now`: per sprite range,
    /// how far its motion has moved it, after the ranges moved last frame
    /// that no longer move, at (0, 0) (back where the scene has them).
    /// `None` when that is what was applied last and `rebuilt` (the
    /// world's sprites made again from the scene's) is false.
    pub(crate) fn offsets(&mut self, scene: &scene::Scene, now: Instant, rebuilt: bool, arrival: impl Fn(u64) -> Option<Instant>) -> Option<Vec<((u32, u32), (i32, i32))>> {
        self.started.retain(|serial, _| scene.motions.iter().any(|m| m.serial == *serial));
        let n = scene.sprites.len() as u32;
        let offsets: Vec<_> = scene.motions.iter().map(|m| {
            let started = *self.started.entry(m.serial).or_insert_with(|| {
                let arrived = arrival(m.serial).unwrap_or(now);
                arrived.checked_sub(Duration::from_millis(m.elapsed_ms.into())).unwrap_or(arrived)
            });
            ((m.sprites.0.min(n), m.sprites.1.min(n)), offset_at(m.way, m.duration_ms, now.saturating_duration_since(started)))
        }).collect();
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

    /// Whether some sprites are off where the scene has them.
    #[cfg(test)]
    pub(crate) fn any_applied(&self) -> bool {
        !self.applied.is_empty()
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
mod tests {
    use super::*;

    fn motion(serial: u64) -> scene::Motion {
        scene::Motion { serial, sprites: (1, 3), way: (64, 32), duration_ms: 100, elapsed_ms: 0 }
    }

    fn scene_with(motions: Vec<scene::Motion>) -> scene::Scene {
        scene::Scene { sprites: vec![scene::FreeSprite { x: 10, y: 20, ..Default::default() }; 4], motions, ..Default::default() }
    }

    #[test]
    fn a_motion_moves_evenly_and_stops_at_its_end() {
        assert_eq!(offset_at((64, 32), 100, Duration::ZERO), (0, 0));
        assert_eq!(offset_at((64, 32), 100, Duration::from_millis(25)), (16, 8));
        assert_eq!(offset_at((-64, 32), 100, Duration::from_millis(50)), (-32, 16));
        assert_eq!(offset_at((64, 32), 100, Duration::from_millis(250)), (64, 32));
        assert_eq!(offset_at((64, 32), 0, Duration::ZERO), (64, 32), "no length: there at once");
    }

    #[test]
    fn the_clock_starts_at_first_sight_and_a_resent_serial_does_not_restart_it() {
        let t0 = Instant::now();
        let mut m = UnitMotions::default();
        let scene = scene_with(vec![motion(5)]);
        assert_eq!(m.offsets(&scene, t0, false, |_| None).unwrap(), [((1, 3), (0, 0))]);
        assert_eq!(m.offsets(&scene, t0 + Duration::from_millis(50), false, |_| None).unwrap(), [((1, 3), (32, 16))]);
        // The same telling again (a scene with other changes): same clock.
        let again = scene_with(vec![motion(5)]);
        assert_eq!(m.offsets(&again, t0 + Duration::from_millis(75), false, |_| None).unwrap(), [((1, 3), (48, 24))]);
        // A new telling (the driver's new position and rest): a new clock.
        let next = scene_with(vec![scene::Motion { serial: 6, way: (16, 8), duration_ms: 25, ..motion(6) }]);
        assert_eq!(m.offsets(&next, t0 + Duration::from_millis(80), false, |_| None).unwrap(), [((1, 3), (0, 0))]);
        assert_eq!(m.offsets(&next, t0 + Duration::from_millis(105), false, |_| None).unwrap(), [((1, 3), (16, 8))]);
        // At its end it stays: nothing more to do unless the sprites were
        // made again.
        assert_eq!(m.offsets(&next, t0 + Duration::from_millis(200), false, |_| None), None);
        assert_eq!(m.offsets(&next, t0 + Duration::from_millis(210), true, |_| None).unwrap(), [((1, 3), (16, 8))]);
        // Gone: one more pass to put the sprites back.
        assert!(m.any_applied());
        assert_eq!(m.offsets(&scene_with(Vec::new()), t0 + Duration::from_millis(220), false, |_| None).unwrap(), [((1, 3), (0, 0))], "put back where the scene has them");
        assert!(!m.any_applied());
        assert_eq!(m.offsets(&scene_with(Vec::new()), t0 + Duration::from_millis(230), false, |_| None), None);
    }

    #[test]
    fn the_clock_starts_at_the_batch_s_arrival_less_what_had_passed_when_sent() {
        let arrived = Instant::now();
        let mut m = UnitMotions::default();
        // Sent 40 ms into the slide; first drawn 10 ms after it arrived.
        let scene = scene_with(vec![scene::Motion { elapsed_ms: 40, ..motion(8) }]);
        let at = |ms| arrived + Duration::from_millis(ms);
        assert_eq!(m.offsets(&scene, at(10), false, |s| (s == 8).then_some(arrived)).unwrap(), [((1, 3), (32, 16))]);
        assert_eq!(m.offsets(&scene, at(60), false, |_| None).unwrap(), [((1, 3), (64, 32))]);
    }

    /// What a frame of a slide costs the viewer (`--ignored --nocapture`):
    /// a moving unit's dozen sprites among thousands.
    #[test]
    #[ignore]
    fn motion_frame_bench() {
        let mut scene = scene_with(vec![scene::Motion { sprites: (4000, 4012), ..motion(1) }]);
        scene.sprites = vec![scene::FreeSprite::default(); 6000];
        let mut free: Vec<_> = scene.sprites.iter().map(|s| tile_render::PlacedSprite::new(tile_render::SpriteSetId::EMPTY, s.x, s.y)).collect();
        let mut m = UnitMotions::default();
        let t0 = Instant::now();
        let n = 100_000u32;
        for i in 0..n {
            if let Some(offsets) = m.offsets(&scene, t0 + Duration::from_micros(u64::from(i)), false, |_| Some(t0)) {
                apply(&mut free, &scene.sprites, &offsets);
            }
        }
        eprintln!("{n} motion frames: {:?} each", t0.elapsed() / n);
    }

    #[test]
    fn a_range_beyond_the_sprites_is_cut_and_only_the_range_moves() {
        let t0 = Instant::now();
        let mut m = UnitMotions::default();
        let scene = scene_with(vec![scene::Motion { sprites: (2, 9), ..motion(1) }]);
        let offsets = m.offsets(&scene, t0 + Duration::from_millis(1), false, |_| None).unwrap();
        assert_eq!(offsets[0].0, (2, 4));
        let mut free: Vec<_> = scene.sprites.iter().map(|s| tile_render::PlacedSprite::new(tile_render::SpriteSetId::EMPTY, s.x, s.y)).collect();
        apply(&mut free, &scene.sprites, &[((2, 4), (5, -3))]);
        assert_eq!(free.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(), vec![(10, 20), (10, 20), (15, 17), (15, 17)]);
    }
}
