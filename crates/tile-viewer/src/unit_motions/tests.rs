use super::*;
use std::time::Duration;

/// The viewer's clock is the driver's plus this.
const VIEWER_AHEAD_MS: f64 = 5_000.0;
const FRAME_MS: f64 = 1000.0 / 60.0;
/// The longest the immediate push of a tile's start takes (ms).
const PUSH_MS: (f64, f64) = (0.5, 3.0);

fn lcg(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (*seed >> 33) as f64 / (1u64 << 31) as f64
}

fn between(seed: &mut u64, (a, b): (f64, f64)) -> f64 {
    a + (b - a) * lcg(seed)
}

/// A walk of 64-px tiles east as the game plays it: when each slide starts
/// (`ms` slides, then the game's own per-tile work), when its push reaches
/// the viewer, how many tiles the path planned at each, and the publisher's
/// rounds (each a scene with the unit's anchor as the game had it then,
/// published `publish` ms apart).
struct Walk {
    starts: Vec<f64>,
    pushed: Vec<f64>,
    planned: Vec<u32>,
    rounds: Vec<f64>,
    ms: f64,
    serial: u64,
}

struct Timing {
    work: (f64, f64),
    publish: (f64, f64),
    ms: f64,
    /// The path known up front (else each step is told alone).
    path: bool,
    /// The game stops after this many tiles (the path planned more).
    stops_after: Option<usize>,
}

const MAC: Timing = Timing { work: (9.0, 50.0), publish: (40.0, 130.0), ms: 100.0, path: true, stops_after: None };
const HARSH: Timing = Timing { work: (30.0, 150.0), ..MAC };

fn walk_at(from: f64, serial: u64, tiles: usize, seed: u64, timing: &Timing) -> Walk {
    let mut seed = seed;
    let walked = timing.stops_after.unwrap_or(tiles).min(tiles);
    let mut t = from;
    let (mut starts, mut pushed, mut planned) = (Vec::new(), Vec::new(), Vec::new());
    for k in 0..walked {
        starts.push(t);
        pushed.push(t + between(&mut seed, PUSH_MS));
        planned.push(if timing.path { (tiles - 1 - k) as u32 } else { 0 });
        t += timing.ms + between(&mut seed, timing.work);
    }
    let mut rounds = Vec::new();
    let mut r = from - 200.0;
    while r < t + 2000.0 {
        rounds.push(r);
        r += between(&mut seed, timing.publish);
    }
    Walk { starts, pushed, planned, rounds, ms: timing.ms, serial }
}

impl Walk {
    /// Where the game has the unit at driver time `t` (its slide's last
    /// step at 8/9 of its length).
    fn game(&self, t: f64) -> f64 {
        let mut x = 0.0;
        for (k, &s) in self.starts.iter().enumerate() {
            if t >= s {
                x = 64.0 * k as f64 + 64.0 * ((t - s) / (self.ms * SLIDE_PART)).min(1.0);
            }
        }
        x
    }

    /// The anchor the scene draws the unit at, as published at `t`: the
    /// tile the game has it on (moved there at each slide's end).
    fn anchor(&self, t: f64) -> f64 {
        64.0 * self.starts.iter().filter(|&&s| s + self.ms * SLIDE_PART <= t).count() as f64
    }

    /// The walk as told by `t`, and when its tiles arrived (viewer instants).
    fn told(&self, t: f64, at: impl Fn(f64) -> Instant) -> Option<(scene::Walk, Vec<Instant>)> {
        let n = self.pushed.iter().filter(|&&p| p <= t).count();
        (n > 0).then(|| {
            let tiles = self.starts[..n].iter().map(|&s| scene::WalkTile { start_ms: s as u64, way: (64, 0), ms: self.ms as u32 }).collect();
            (scene::Walk { key: 7, serial: self.serial, origin: (0, 0), tiles, planned: self.planned[n - 1] }, self.pushed[..n].iter().map(|&p| at(p)).collect())
        })
    }

    /// The viewer's unit x at each frame from 50 ms before the walk to 1.5 s
    /// after: (driver time, x).
    fn played_by(&self, motions: &mut UnitMotions, base: Instant) -> Vec<(f64, f64)> {
        let at = |t: f64| base + Duration::from_secs_f64((t + VIEWER_AHEAD_MS) / 1000.0);
        let mut out = Vec::new();
        let mut t = self.starts[0] - 50.0;
        while t < self.starts.last().unwrap() + 1500.0 {
            let published = self.rounds.iter().rev().find(|&&r| r <= t).copied().unwrap_or(f64::MIN);
            let anchor = self.anchor(published);
            let scene = scene::Scene { sprites: vec![scene::FreeSprite::default()], motions: vec![scene::Motion { key: 7, sprites: (0, 1), anchor: (anchor as i32, 0) }], ..Default::default() };
            let told = self.told(t, at);
            let offsets = motions.positions(&scene, at(t), |_| told.as_ref().map(|(w, a)| (w, &a[..])));
            out.push((t, anchor + f64::from(offsets[0].1.0)));
            t += FRAME_MS;
        }
        out
    }

    fn played(&self) -> Vec<(f64, f64)> {
        self.played_by(&mut UnitMotions::default(), Instant::now())
    }

    /// As played before: each tile from when its scene arrived, over its own
    /// length.
    fn played_from_arrival(&self) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        let mut t = self.starts[0] - 50.0;
        while t < self.starts.last().unwrap() + 1500.0 {
            let published = self.rounds.iter().rev().find(|&&r| r <= t).copied().unwrap_or(f64::MIN);
            let x = match self.starts.iter().rposition(|&s| s <= published) {
                Some(k) => {
                    let arrived = self.rounds.iter().find(|&&r| r >= self.starts[k]).copied().unwrap_or(t);
                    64.0 * k as f64 + (64.0 * (t - arrived) / self.ms).clamp(0.0, 64.0).trunc()
                }
                None => 0.0,
            };
            out.push((t, x));
            t += FRAME_MS;
        }
        out
    }
}

/// From the frame after the first push the unit moves every frame (never
/// stops longer than one) until it reaches where the game ends it, never
/// faster than [`SPRINT`] times its slide's pace (and a pixel), never back,
/// never ahead of the game.
fn check(w: &Walk, played: &[(f64, f64)], what: &str) {
    let end = 64.0 * w.starts.len() as f64;
    let first = played.iter().position(|p| p.1 > 0.0).unwrap_or_else(|| panic!("{what}: never moves"));
    assert!(played[first].0 <= w.pushed[0] + FRAME_MS, "{what}: starts at {:.0} ms, pushed at {:.0}", played[first].0, w.pushed[0]);
    let last = played.iter().position(|p| p.1 >= end).unwrap_or_else(|| panic!("{what}: never arrives: {:?}", played.last()));
    let fastest = SPRINT * 64.0 * FRAME_MS / w.ms + 1.0;
    for (i, pair) in played[first - 1..=last].windows(2).enumerate() {
        let step = pair[1].1 - pair[0].1;
        assert!(step >= 0.0, "{what}: backwards at {:.0} ms: {pair:?}", pair[1].0);
        assert!(step <= fastest, "{what}: a sprint of {step} px (at most {fastest:.1}) at {:.0} ms", pair[1].0);
        if i > 0 {
            let before = pair[0].1 - played[first - 2 + i].1;
            assert!(step > 0.0 || before > 0.0, "{what}: stopped two frames at {:.0} ms ({} px)", pair[1].0, pair[1].1);
        }
    }
    for &(t, x) in played {
        assert!(x <= w.game(t) + 0.5, "{what}: ahead of the game at {t:.0} ms: {x} > {}", w.game(t));
    }
    assert!(played[last..].iter().all(|p| p.1 == end), "{what}: does not stay at the end");
}

/// The Mac's measured walks (9-50 ms of the game's own work a tile,
/// `civ3-bridge.log`), path known, on a viewer that saw nothing before.
#[test]
fn a_mac_walk_plays_from_its_first_push_without_stopping_and_never_ahead() {
    for seed in 1..=60u64 {
        let w = walk_at(1000.0, 1, 8, seed, &MAC);
        check(&w, &w.played(), &format!("seed {seed}"));
    }
}

/// Harsh variation (the game's work 30-150 ms a tile), on a viewer that saw
/// a walk before.
#[test]
fn a_harsh_walk_plays_without_stopping_and_never_ahead() {
    for seed in 1..=60u64 {
        let base = Instant::now();
        let mut viewer = UnitMotions::default();
        walk_at(1000.0, 1, 8, seed + 1000, &HARSH).played_by(&mut viewer, base);
        let w = walk_at(20_000.0, 2, 8, seed, &HARSH);
        check(&w, &w.played_by(&mut viewer, base), &format!("seed {seed}"));
    }
}

/// Shift (10 ms slides) and native timing (12 ms of work), path known.
#[test]
fn short_and_native_walks_play_evenly() {
    for timing in [Timing { work: (5.0, 60.0), ms: 10.0, ..MAC }, Timing { work: (10.0, 14.0), ..MAC }] {
        for seed in 1..=20 {
            let w = walk_at(1000.0, 1, 6, seed, &timing);
            let played = w.played();
            // (A 10 ms slide may cross a whole tile in a frame.)
            if timing.ms >= 100.0 {
                check(&w, &played, &format!("{} ms slides, seed {seed}", timing.ms));
            }
            assert_eq!(played.last().unwrap().1, 64.0 * 6.0);
            assert!(played.windows(2).all(|p| p[1].1 >= p[0].1));
            assert!(played.iter().all(|&(t, x)| x <= w.game(t) + 0.5));
        }
    }
}

/// A single step (nothing planned) plays its slide from its push.
#[test]
fn a_single_step_plays_its_slide_at_once() {
    for seed in 1..=20 {
        let w = walk_at(1000.0, 1, 1, seed, &MAC);
        let played = w.played();
        check(&w, &played, &format!("seed {seed}"));
        let arrived = played.iter().find(|p| p.1 >= 64.0).unwrap().0;
        assert!(arrived <= w.pushed[0] + 100.0 + 2.0 * FRAME_MS, "seed {seed}: the step took until {arrived:.0} ms");
    }
}

/// Keyboard steps told one by one (no path): from each push, never ahead,
/// never back, at the end in the end.
#[test]
fn steps_without_a_path_play_from_each_push() {
    for seed in 1..=20 {
        let w = walk_at(1000.0, 1, 5, seed, &Timing { path: false, ..MAC });
        let played = w.played();
        let first = played.iter().position(|p| p.1 > 0.0).unwrap();
        assert!(played[first].0 <= w.pushed[0] + FRAME_MS);
        assert!(played.windows(2).all(|p| p[1].1 >= p[0].1));
        assert!(played.iter().all(|&(t, x)| x <= w.game(t) + 0.5));
        assert_eq!(played.last().unwrap().1, 64.0 * 5.0);
    }
}

/// The path planned 8 tiles, the game stopped after 3 (blocked): the unit
/// stops where the game stopped, easing in, never past it.
#[test]
fn a_walk_that_ends_early_stops_where_the_game_stopped() {
    for seed in 1..=20 {
        let w = walk_at(1000.0, 1, 8, seed, &Timing { stops_after: Some(3), ..MAC });
        let played = w.played();
        assert!(played.windows(2).all(|p| p[1].1 >= p[0].1));
        assert!(played.iter().all(|&(t, x)| x <= w.game(t) + 0.5 && x <= 192.0));
        assert_eq!(played.last().unwrap().1, 192.0, "seed {seed}");
        let last = played.iter().position(|p| p.1 >= 192.0).unwrap();
        let fastest = SPRINT * 64.0 * FRAME_MS / w.ms + 1.0;
        assert!(played[last].1 - played[last - 1].1 <= fastest, "seed {seed}: no snap at the end");
    }
}

/// The trace of one Mac walk, before and after (`--ignored --nocapture`):
/// driver ms, the game's x, x played from each scene's arrival, x played
/// from the pushed walk.
#[test]
#[ignore]
fn walk_trace() {
    let w = walk_at(1000.0, 1, 8, 11, &MAC);
    let (before, after) = (w.played_from_arrival(), w.played());
    println!("# starts {:?}\n# pushed {:?}", w.starts, w.pushed);
    println!("t,game,before,after");
    for (b, a) in before.iter().zip(&after) {
        println!("{:.1},{:.1},{:.1},{:.1}", a.0, w.game(a.0), b.1, a.1);
    }
}

#[test]
fn a_range_beyond_the_sprites_is_cut_and_only_the_range_moves() {
    let scene = scene::Scene { sprites: vec![scene::FreeSprite { x: 10, y: 20, ..Default::default() }; 4], ..Default::default() };
    let mut free: Vec<_> = scene.sprites.iter().map(|s| tile_render::PlacedSprite::new(tile_render::SpriteSetId::EMPTY, s.x, s.y)).collect();
    apply(&mut free, &scene.sprites, &[((2, 4), (5, -3))]);
    assert_eq!(free.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(), vec![(10, 20), (10, 20), (15, 17), (15, 17)]);
}

#[test]
fn the_marker_over_a_walking_unit_goes_with_it() {
    let w = walk_at(1000.0, 1, 2, 5, &MAC);
    let base = Instant::now();
    let at = |t: f64| base + Duration::from_secs_f64((t + VIEWER_AHEAD_MS) / 1000.0);
    let mut m = UnitMotions::default();
    let mut scene = scene::Scene { sprites: vec![scene::FreeSprite::default()], motions: vec![scene::Motion { key: 7, sprites: (0, 1), anchor: (0, 0) }], ..Default::default() };
    scene.focus = Some((300, 200));
    assert_eq!(m.focus_at(&scene, 1), Some((300, 200)), "no motion of its own: the focus");
    scene.focus_motion = Some(scene::FocusMotion { key: 7, from: (100, 80) });
    let told = w.told(1050.0, at);
    let moved = m.positions(&scene, at(1050.0), |_| told.as_ref().map(|(w, a)| (w, &a[..])))[0].1;
    assert!(moved.0 > 0);
    assert_eq!(m.focus_at(&scene, 1), Some((100 + moved.0, 80)));
    assert_eq!(m.focus_at(&scene, 2), Some((100 + moved.0 / 2, 80)), "in the pixels of a half-size world");
}

/// What a frame of a walk costs the viewer (`--ignored --nocapture`).
#[test]
#[ignore]
fn motion_frame_bench() {
    let w = walk_at(1000.0, 1, 8, 5, &MAC);
    let base = Instant::now();
    let at = |t: f64| base + Duration::from_secs_f64((t + VIEWER_AHEAD_MS) / 1000.0);
    let mut scene = scene::Scene { motions: vec![scene::Motion { key: 7, sprites: (4000, 4012), anchor: (0, 0) }], ..Default::default() };
    scene.sprites = vec![scene::FreeSprite::default(); 6000];
    let mut free: Vec<_> = scene.sprites.iter().map(|s| tile_render::PlacedSprite::new(tile_render::SpriteSetId::EMPTY, s.x, s.y)).collect();
    let told = w.told(5000.0, at);
    let mut m = UnitMotions::default();
    let n = 100_000u32;
    let started = Instant::now();
    for i in 0..n {
        if let Some(offsets) = m.offsets(&scene, at(1000.0 + f64::from(i) * 0.01), false, |_| told.as_ref().map(|(w, a)| (w, &a[..]))) {
            apply(&mut free, &scene.sprites, &offsets);
        }
    }
    eprintln!("{n} walk frames: {:?} each", started.elapsed() / n);
}
