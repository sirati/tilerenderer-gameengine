//! The frame counter (`viewer_settings::Display::show_fps`): frames per
//! second over the last second, the mean time between them, and the share
//! of frames a software renderer skipped (unchanged, see `Gpu::software`),
//! drawn in the window's top-right corner over everything, from the
//! description's `Viewer@glyphsN` sheets (`render_desc::GLYPHS`).
//!
//! Its text changes a few times a second only; the sprites are made again
//! only then (or when the window or the art changes).

use std::collections::VecDeque;

use render_desc::Compiled;
use tile_render::PlacedSprite;

/// How far back the counter looks, in seconds.
pub const SPAN: f64 = 1.0;
/// How often its text changes, in seconds.
pub const TEXT_EVERY: f64 = 0.3;
/// The largest glyph scale the description has (`Viewer@glyphs1..=4`).
pub const MAX_SCALE: u32 = 4;

/// What the counter counted over the last [`SPAN`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Stats {
    pub fps: f64,
    /// The mean time between two frames, in milliseconds (none with fewer
    /// than two frames).
    pub frame_ms: Option<f64>,
    /// The share of frames skipped, 0..=1.
    pub skipped: f64,
}

/// Frames presented and skipped, by when (seconds on any clock that only
/// moves forward).
#[derive(Clone, Debug, Default)]
pub struct FrameCounter {
    presented: VecDeque<f64>,
    skipped: VecDeque<f64>,
    /// When counting started (the first frame).
    since: Option<f64>,
}

impl FrameCounter {
    pub fn presented(&mut self, now: f64) {
        self.since.get_or_insert(now);
        self.presented.push_back(now);
        self.forget(now);
    }

    pub fn skipped(&mut self, now: f64) {
        self.since.get_or_insert(now);
        self.skipped.push_back(now);
        self.forget(now);
    }

    /// Drops what is older than [`SPAN`].
    fn forget(&mut self, now: f64) {
        for q in [&mut self.presented, &mut self.skipped] {
            while q.front().is_some_and(|&t| t <= now - SPAN) {
                q.pop_front();
            }
        }
    }

    /// The counts of the last [`SPAN`] before `now` (less if counting began
    /// later).
    pub fn stats(&self, now: f64) -> Stats {
        let recent = |q: &VecDeque<f64>| q.iter().filter(|&&t| t > now - SPAN && t <= now).count();
        let (n, s) = (recent(&self.presented), recent(&self.skipped));
        let span = self.since.map_or(SPAN, |since| (now - since).clamp(0.0, SPAN));
        let fps = if span > 0.0 { n as f64 / span } else { 0.0 };
        let times: Vec<f64> = self.presented.iter().copied().filter(|&t| t > now - SPAN && t <= now).collect();
        let frame_ms = (times.len() >= 2).then(|| (times[times.len() - 1] - times[0]) * 1000.0 / (times.len() - 1) as f64);
        let skipped = if n + s > 0 { s as f64 / (n + s) as f64 } else { 0.0 };
        Stats { fps, frame_ms, skipped }
    }
}

/// The counter's line: `"60 fps 16.7 ms"`, with `" 12% skip"` when the
/// renderer skips frames (`software`). Only characters of
/// `render_desc::GLYPHS`.
pub fn text(stats: &Stats, software: bool) -> String {
    let mut out = format!("{:.0} fps", stats.fps);
    if let Some(ms) = stats.frame_ms {
        out += &format!(" {ms:.1} ms");
    }
    if software {
        out += &format!(" {:.0}% skip", stats.skipped * 100.0);
    }
    out
}

/// Recent wall-time build samples and asynchronously returned GPU samples.
/// Host frame intervals are never subtracted to guess CPU starvation.
#[derive(Default)]
pub struct Timings {
    cpu: VecDeque<(f64, f64)>,
    gpu: VecDeque<(f64, f64)>,
    gaps: VecDeque<(f64, f64)>,
    previous: Option<tile_render::timing::FrameStamp>,
    minimum_frame: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimingStats {
    pub cpu_build_ms: Option<f64>,
    pub gpu_ms: Option<f64>,
    pub gpu_gap_ms: Option<f64>,
}

impl Timings {
    /// Reset at a timing/device epoch boundary. Results from older queued
    /// packets must not become samples of the newly enabled counter.
    pub fn reset_from(&mut self, minimum_frame: u64) {
        *self = Self { minimum_frame: Some(minimum_frame), ..Self::default() };
    }

    fn accepts(&self, frame: Option<u64>) -> bool {
        self.minimum_frame.is_none_or(|minimum| frame.is_some_and(|frame| frame >= minimum))
    }

    pub fn cpu_frame(&mut self, frame: u64, now: f64, ms: f64) {
        if self.accepts(Some(frame)) { self.cpu_build(now, ms); }
    }

    fn note(queue: &mut VecDeque<(f64, f64)>, now: f64, ms: f64) {
        if ms.is_finite() && ms >= 0.0 { queue.push_back((now, ms)); }
        while queue.front().is_some_and(|&(at, _)| at <= now - SPAN) { queue.pop_front(); }
    }

    pub fn cpu_build(&mut self, now: f64, ms: f64) {
        Self::note(&mut self.cpu, now, ms);
    }

    pub fn gpu_frame(&mut self, now: f64, frame: &tile_render::timing::FrameTimes) {
        if !self.accepts(frame.frame_token) { return; }
        let stamp = frame.stamp;
        // Never move the anchor backwards if a backend returns late results.
        if self.previous.is_some_and(|old| stamp.frame_id <= old.frame_id) { return; }
        Self::note(&mut self.gpu, now, frame.span);
        if let Some(gap) = self.previous.and_then(|old| stamp.gap_after(old)) {
            Self::note(&mut self.gaps, now, gap);
        } else {
            // A missing instrumented frame cannot contribute an idle gap.
            self.gaps.clear();
        }
        self.previous = Some(stamp);
    }

    pub fn stats(&self, now: f64) -> TimingStats {
        let mean = |queue: &VecDeque<(f64, f64)>| {
            let (sum, count) = queue.iter().filter(|&&(at, _)| at > now - SPAN && at <= now)
                .fold((0.0, 0), |(sum, count), &(_, ms)| (sum + ms, count + 1));
            (count > 0).then(|| sum / count as f64)
        };
        TimingStats { cpu_build_ms: mean(&self.cpu), gpu_ms: mean(&self.gpu), gpu_gap_ms: mean(&self.gaps) }
    }
}

pub fn detailed_text(stats: &Stats, software: bool, timings: TimingStats) -> String {
    let value = |ms: Option<f64>| ms.map_or_else(|| "n/a".to_owned(), |ms| format!("{ms:.1} ms"));
    let gap = timings.gpu_gap_ms.map_or_else(|| "n/a".to_owned(), |ms| format!("~{ms:.1} ms"));
    format!("{}\nCPU build {}\nGPU {}\nGPU gap {gap}", text(stats, software), value(timings.cpu_build_ms), value(timings.gpu_ms))
}

/// How many times their 5 × 7 size the glyphs are drawn in a window of
/// `window`: about one glyph pixel per 540 window rows (1 at 720 rows, 2
/// at 1080, 4 at 2160), smaller where a line of `chars` characters would
/// take more than half the window's width.
pub fn scale(window: (u32, u32), chars: usize) -> u32 {
    let by_height = (window.1 / 540).clamp(1, MAX_SCALE);
    (1..=by_height).rev().find(|&n| render_desc::glyph_cell(n).0 * chars as u32 <= window.0 / 2).unwrap_or(1)
}

/// The sprites of `line` at the window's top-right corner, `n` times their
/// size (a smaller size if the description lacks it), drawn by camera
/// `camera` (one showing the whole window 1:1). A character the glyphs
/// lack shows as a space; none reach into the left half of the window
/// (where a game keeps its buttons).
pub fn line_sprites(compiled: &Compiled, line: &str, window: (u32, u32), n: u32, camera: u32) -> Vec<PlacedSprite> {
    let Some(n) = (1..=n.max(1)).rev().find(|n| compiled.set(&format!("Viewer@glyphs{n}/0")).is_some()) else { return vec![] };
    let (cw, _) = render_desc::glyph_cell(n);
    // A space before and after: the box's margin.
    let chars: Vec<usize> = std::iter::once(' ')
        .chain(line.chars())
        .chain(std::iter::once(' '))
        .map(|c| render_desc::GLYPHS.chars().position(|g| g == c).unwrap_or(0))
        .collect();
    let margin = 2 * n as i32;
    let left = window.0 as i32 - margin - (cw as usize * chars.len()) as i32;
    chars
        .iter()
        .enumerate()
        .filter_map(|(i, &g)| {
            let x = left + (i as u32 * cw) as i32;
            if x < window.0 as i32 / 2 {
                return None;
            }
            let (set, _) = compiled.set(&format!("Viewer@glyphs{n}/{g}"))?;
            Some(PlacedSprite { screen: true, view: Some(camera), ..PlacedSprite::new(set, x, margin) })
        })
        .collect()
}

fn rows_sprites(compiled: &Compiled, text: &str, window: (u32, u32), n: u32, camera: u32) -> Vec<PlacedSprite> {
    // Use the actual available scale for row spacing, just as line_sprites does.
    let Some(n) = (1..=n.max(1)).rev().find(|n| compiled.set(&format!("Viewer@glyphs{n}/0")).is_some()) else { return vec![] };
    let height = render_desc::glyph_cell(n).1 as i32;
    text.lines().enumerate().flat_map(|(row, line)| {
        line_sprites(compiled, line, window, n, camera).into_iter().map(move |mut sprite| {
            sprite.y += row as i32 * height;
            sprite
        })
    }).filter(|sprite| sprite.y < window.1 as i32).collect()
}

/// The counter and its sprites as last made.
#[derive(Default)]
pub struct Overlay {
    pub counter: FrameCounter,
    pub timings: Timings,
    detailed: bool,
    /// The line shown and when it was made.
    line: Option<(f64, String)>,
    /// What the sprites were made for: line, window, art version, camera.
    made_for: Option<(String, (u32, u32), u64, u32)>,
    sprites: Vec<PlacedSprite>,
}

impl Overlay {
    /// The counter's sprites at `now` (seconds; the line changes every
    /// [`TEXT_EVERY`]) for a window of `window` drawn with art `version` of
    /// `compiled`, by camera `camera`.
    pub fn sprites(&mut self, compiled: &Compiled, version: u64, now: f64, window: (u32, u32), camera: u32, software: bool, detailed: bool) -> &[PlacedSprite] {
        if self.detailed != detailed { self.line = None; self.detailed = detailed; }
        let stale = self.line.as_ref().is_none_or(|(at, _)| now - at >= TEXT_EVERY || now < *at);
        if stale {
            let stats = self.counter.stats(now);
            let text = if detailed { detailed_text(&stats, software, self.timings.stats(now)) } else { text(&stats, software) };
            self.line = Some((now, text));
        }
        let line = &self.line.as_ref().expect("made above").1;
        let key = (line.clone(), window, version, camera);
        if self.made_for.as_ref() != Some(&key) {
            let n = scale(window, line.lines().map(|row| row.chars().count() + 2).max().unwrap_or(0));
            self.sprites = rows_sprites(compiled, line, window, n, camera);
            self.made_for = Some(key);
        }
        &self.sprites
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn timing_reset_rejects_old_or_unidentified_worker_packets() {
        use tile_render::timing::{FrameStamp, FrameTimes};
        let mut timings = Timings::default();
        timings.reset_from(100);
        timings.cpu_frame(99, 0.0, 50.0);
        timings.gpu_frame(0.0, &FrameTimes { span: 50.0, frame_token: Some(99), ..Default::default() });
        timings.gpu_frame(0.0, &FrameTimes { span: 50.0, ..Default::default() });
        assert_eq!(timings.stats(0.0), TimingStats::default());
        timings.cpu_frame(100, 0.0, 3.0);
        timings.gpu_frame(0.0, &FrameTimes { span: 4.0, frame_token: Some(100), stamp: FrameStamp { frame_id: 1, start: 10, end: 14, period_ns: 1_000_000.0 }, ..Default::default() });
        assert_eq!(timings.stats(0.0), TimingStats { cpu_build_ms: Some(3.0), gpu_ms: Some(4.0), gpu_gap_ms: None });
    }

    #[test]
    fn delayed_gpu_samples_use_timestamp_gaps_and_expire() {
        use tile_render::timing::{FrameStamp, FrameTimes};
        let mut timings = Timings::default();
        let sample = |frame_id, start, end| FrameTimes { span: (end - start) as f64,
            stamp: FrameStamp { frame_id, start, end, period_ns: 1_000_000.0 }, ..Default::default() };
        timings.cpu_build(0.0, 2.0);
        timings.gpu_frame(0.1, &sample(1, 10, 14));
        assert_eq!(timings.stats(0.1).gpu_gap_ms, None);
        timings.gpu_frame(0.2, &sample(2, 20, 26));
        assert_eq!(timings.stats(0.2), TimingStats { cpu_build_ms: Some(2.0), gpu_ms: Some(5.0), gpu_gap_ms: Some(6.0) });
        timings.gpu_frame(0.3, &sample(4, 80, 90));
        assert_eq!(timings.stats(0.3).gpu_gap_ms, None, "untimed frame 3 is not idle GPU time");
        timings.gpu_frame(0.4, &sample(3, 40, 50));
        assert_eq!(timings.stats(0.4).gpu_gap_ms, None, "late result cannot move the anchor backwards");
        assert_eq!(timings.stats(1.5), TimingStats::default());
        let reset = Timings::default();
        assert_eq!(reset.stats(0.0), TimingStats::default());
    }

    #[test]
    fn detailed_metrics_label_build_gap_estimates_and_unavailable_values() {
        let stats = Stats { fps: 60.0, frame_ms: Some(16.7), skipped: 0.0 };
        let missing = detailed_text(&stats, false, TimingStats::default());
        assert_eq!(missing, "60 fps 16.7 ms\nCPU build n/a\nGPU n/a\nGPU gap n/a");
        let present = detailed_text(&stats, false, TimingStats { cpu_build_ms: Some(2.0), gpu_ms: Some(8.0), gpu_gap_ms: Some(6.7) });
        assert!(present.contains("GPU gap ~6.7 ms"));
        assert!(present.chars().filter(|&c| c != '\n').all(|c| render_desc::GLYPHS.contains(c)));
        let compiled = compiled();
        let sprites = rows_sprites(&compiled, &present, (1920, 1080), 2, 5);
        assert!(sprites.iter().all(|sprite| sprite.view == Some(5) && sprite.x >= 960));
        for row in 0..4 { assert!(sprites.iter().any(|sprite| sprite.y == 4 + row * 18)); }
    }

    #[test]
    fn a_steady_rate_counts_as_itself_over_the_last_second() {
        let mut c = FrameCounter::default();
        for i in 0..300 {
            c.presented(i as f64 / 60.0);
        }
        let s = c.stats(299.0 / 60.0);
        assert!((s.fps - 60.0).abs() <= 1.0, "{s:?}");
        assert!(close(s.frame_ms.unwrap(), 1000.0 / 60.0), "{s:?}");
        assert_eq!(s.skipped, 0.0);
        // Old frames are forgotten.
        assert!(c.presented.len() <= 61);
    }

    #[test]
    fn the_rate_follows_a_change_within_a_second() {
        let mut c = FrameCounter::default();
        let mut t = 0.0;
        while t < 2.0 {
            c.presented(t);
            t += 1.0 / 60.0;
        }
        while t < 3.5 {
            c.presented(t);
            t += 1.0 / 20.0;
        }
        let s = c.stats(t);
        assert!((s.fps - 20.0).abs() <= 1.0, "{s:?}");
        assert!(close(s.frame_ms.unwrap(), 50.0), "{s:?}");
        // No frames for a second: nothing.
        let s = c.stats(t + 1.0);
        assert_eq!((s.fps, s.frame_ms), (0.0, None));
    }

    #[test]
    fn the_first_second_is_counted_over_the_time_so_far() {
        let mut c = FrameCounter::default();
        for i in 0..15 {
            c.presented(10.0 + i as f64 / 30.0);
        }
        let s = c.stats(10.5);
        assert!((s.fps - 30.0).abs() <= 2.0, "{s:?}");
        assert_eq!(FrameCounter::default().stats(5.0).fps, 0.0);
    }

    #[test]
    fn skipped_frames_are_a_share_of_all() {
        let mut c = FrameCounter::default();
        for i in 0..100 {
            let t = i as f64 / 100.0;
            if i % 4 == 0 { c.presented(t) } else { c.skipped(t) }
        }
        let s = c.stats(0.99);
        assert!(close(s.skipped, 0.75), "{s:?}");
        assert!((s.fps - 25.0).abs() <= 1.0, "{s:?}");
    }

    #[test]
    fn the_line_uses_only_glyphs_there_are() {
        let s = Stats { fps: 59.6, frame_ms: Some(16.78), skipped: 0.123 };
        assert_eq!(text(&s, false), "60 fps 16.8 ms");
        assert_eq!(text(&s, true), "60 fps 16.8 ms 12% skip");
        assert_eq!(text(&Stats { fps: 0.0, frame_ms: None, skipped: 0.0 }, true), "0 fps 0% skip");
        assert!(text(&s, true).chars().all(|c| render_desc::GLYPHS.contains(c)));
    }

    #[test]
    fn glyphs_grow_with_the_window_but_keep_to_its_right_half() {
        assert_eq!(scale((1280, 720), 16), 1);
        assert_eq!(scale((1920, 1080), 16), 2);
        assert_eq!(scale((3840, 2160), 25), 4);
        assert_eq!(scale((99999, 99999), 25), MAX_SCALE);
        // A narrow, tall window: smaller.
        assert_eq!(scale((400, 2160), 25), 1);
        assert_eq!(scale((10, 10), 25), 1);
    }

    struct NoFiles;
    impl render_desc::ImageLoader for NoFiles {
        fn load(&self, source: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
            Err(format!("no file {source}"))
        }
    }

    fn compiled() -> Compiled {
        let mut desc = String::from("grid = { layout = \"orthogonal\", step = [8, 8] }\n");
        for n in 1..=2 {
            let (w, h) = render_desc::glyph_cell(n);
            desc += &format!(
                "[images.g{n}]\nsource = \"glyphs:255,255,255,255:0,0,0,176:{n}\"\n[sheets.g{n}]\nimage = \"g{n}\"\ncell = [{w}, {h}]\ngrid = [34, 1]\ngroup = \"Viewer@glyphs{n}\"\n"
            );
        }
        let d = render_desc::Description::parse(&desc).unwrap();
        d.compile(&NoFiles, 256).unwrap()
    }

    #[test]
    fn the_line_is_placed_top_right_in_its_camera() {
        let c = compiled();
        let sprites = line_sprites(&c, "60 fps", (1920, 1080), 2, 5);
        // Six characters and a space either side, 12 pixels each, 4 from the
        // corner.
        assert_eq!(sprites.len(), 8);
        assert!(sprites.iter().all(|s| s.screen && s.view == Some(5) && s.y == 4 && s.after_pass == usize::MAX));
        assert_eq!(sprites.last().unwrap().x, 1920 - 4 - 12);
        assert_eq!(sprites[0].x, 1920 - 4 - 8 * 12);
        assert_eq!(sprites[1].set, c.set(&format!("Viewer@glyphs2/{}", render_desc::GLYPHS.find('6').unwrap())).unwrap().0);
        // Asked for a size the description lacks: the largest it has.
        let big = line_sprites(&c, "60 fps", (1920, 1080), 4, 5);
        assert_eq!(big, sprites);
        // Never into the left half.
        let narrow = line_sprites(&c, "60 fps 16.7 ms 12% skip", (200, 100), 1, 0);
        assert!(!narrow.is_empty() && narrow.iter().all(|s| s.x >= 100));
        // Without the glyphs, nothing.
        let none = render_desc::Description::parse("grid = { layout = \"orthogonal\", step = [8, 8] }\n").unwrap().compile(&NoFiles, 256).unwrap();
        assert!(line_sprites(&none, "60 fps", (1920, 1080), 2, 0).is_empty());
    }

    #[test]
    fn the_overlay_changes_its_line_a_few_times_a_second_only() {
        let c = compiled();
        let mut o = Overlay::default();
        let mut t = 0.0;
        let mut changes = 0;
        let mut last = Vec::new();
        while t < 3.0 {
            o.counter.presented(t);
            let s = o.sprites(&c, 1, t, (1920, 1080), 3, false, false).to_vec();
            if s != last {
                changes += 1;
                last = s;
            }
            // A rate that keeps changing (so would every frame's text).
            t += if (t * 10.0) as u32 % 2 == 0 { 1.0 / 90.0 } else { 1.0 / 25.0 };
        }
        // (At most one change every TEXT_EVERY; equal lines are no change.)
        assert!((3..=11).contains(&changes), "{changes} changes in 3 s");
        // A new window or new art: new sprites at once.
        let moved = o.sprites(&c, 1, t, (1280, 720), 3, false, false).to_vec();
        assert!(moved != last && moved.iter().all(|s| s.x >= 640));
    }
}

/// Frame times for the log (`TILE_VIEWER_FRAME_TIMES=1`): every
/// [`FrameLog::EVERY`], how far apart the frames presented were and how
/// long the longest took to draw: a stall shows as a long gap (the viewer
/// presents at least once a beat, see `FRAME`).
#[derive(Clone, Debug, Default)]
pub struct FrameLog {
    last: Option<f64>,
    since: Option<f64>,
    /// Gaps between presented frames, and the time each took to draw, in
    /// milliseconds.
    gaps: Vec<f64>,
    worst_draw: f64,
}

impl FrameLog {
    /// How often a line is logged, in seconds.
    pub const EVERY: f64 = 2.0;
    /// A gap this long (in milliseconds) counts as a stall.
    pub const STALL_MS: f64 = 50.0;

    /// A frame presented at `now` (seconds) that took `draw_ms` to draw;
    /// the line to log once [`FrameLog::EVERY`] has passed.
    pub fn presented(&mut self, now: f64, draw_ms: f64) -> Option<String> {
        if let Some(last) = self.last.replace(now) {
            self.gaps.push((now - last) * 1000.0);
        }
        self.worst_draw = self.worst_draw.max(draw_ms);
        let since = *self.since.get_or_insert(now);
        if now - since < Self::EVERY || self.gaps.is_empty() {
            return None;
        }
        let mut gaps = std::mem::take(&mut self.gaps);
        gaps.sort_by(f64::total_cmp);
        let n = gaps.len();
        let mean = gaps.iter().sum::<f64>() / n as f64;
        let p99 = gaps[((n as f64 * 0.99) as usize).min(n - 1)];
        let stalls = gaps.iter().filter(|&&g| g >= Self::STALL_MS).count();
        let line = format!(
            "frame times over {:.1} s: {} frames, gaps mean {mean:.1} ms, 99th {p99:.1} ms, worst {:.1} ms, {stalls} over {} ms; longest draw {:.1} ms",
            now - since,
            n + 1,
            gaps[n - 1],
            Self::STALL_MS,
            self.worst_draw
        );
        self.since = Some(now);
        self.worst_draw = 0.0;
        Some(line)
    }
}

#[cfg(test)]
mod frame_log_tests {
    use super::FrameLog;

    #[test]
    fn a_line_every_two_seconds_with_the_stalls() {
        let mut log = FrameLog::default();
        let mut lines = Vec::new();
        let mut t = 0.0;
        for i in 0..200 {
            // One frame of 16 ms each, but the 100th 120 ms after the one before.
            t += if i == 100 { 0.120 } else { 0.016 };
            lines.extend(log.presented(t, 2.0));
        }
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("worst 120.0 ms, 1 over 50 ms"), "{}", lines[0]);
    }
}
