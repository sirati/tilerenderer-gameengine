//! Reading a recording back: what it shows ([`summarize`]), and the
//! Alt mode's picker run again over its motion and targets ([`run`]) with
//! the constants or other values ([`Tuning`]), to see whether it would
//! have reached the targets the player clicked and how often it would have
//! landed elsewhere first.
//!
//! The player's assumption: motion more or less towards the element they
//! end up clicking, so a landing on another element before that click is
//! the picker getting it wrong.

use crate::snap::{Action, Point, Rates, Snapper, Target, Tuning};

use super::{ClickUse, ModeChange, PointerKind, Record, seconds};

/// How a replay runs.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Options {
    pub tuning: Tuning,
    /// The viewer steps the pointer every frame; a replay steps it every
    /// this many microseconds (on multiples of it), and when the recording
    /// says the pointer came to rest.
    pub tick_us: u64,
    /// Updates per second instead of the recorded rate.
    pub rate: Option<f64>,
}

impl Default for Options {
    fn default() -> Self {
        Self { tuning: Tuning::default(), tick_us: 8333, rate: None }
    }
}

/// The pointer came to rest on the element with key `key` at `at`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Landing {
    pub t: u64,
    pub key: u64,
    pub at: Point,
}

/// A recorded press that went to the element snapped to: the element's key
/// as recorded, and the one the replayed pointer was on then.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ClickOutcome {
    pub t: u64,
    pub recorded: Option<u64>,
    pub replayed: Option<u64>,
}

/// What a replay did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub landings: Vec<Landing>,
    pub clicks: Vec<ClickOutcome>,
    /// Replayed landings motion led to on another element than the one
    /// clicked next.
    pub wrong_landings: usize,
}

impl Run {
    /// Clicks whose element the replayed pointer was on.
    pub fn reached(&self) -> usize {
        self.clicks.iter().filter(|c| c.recorded.is_some() && c.recorded == c.replayed).count()
    }
}

/// The key of the element at `index` of `targets`.
fn key(targets: &[Target], index: Option<u32>) -> Option<u64> {
    targets.get(index? as usize).map(|t| t.id)
}

/// Runs the picker over `records` (one session's, in order).
pub fn run(records: &[Record], options: &Options) -> Run {
    let mut out = Run::default();
    let mut snapper = Snapper::with_tuning(options.tuning);
    let mut rates = Rates { updates_per_second: options.rate.unwrap_or(3.0), ghost_fps: 30.0 };
    let mut targets: Vec<Target> = Vec::new();
    let mut slider = false;
    let mut ticked: Option<u64> = None;
    // Replayed landings motion led to since the last click, and whether
    // motion came since the last landing.
    let mut pending: Vec<u64> = Vec::new();
    let mut moving = false;
    let tick_us = options.tick_us.max(1);
    let step = |snapper: &mut Snapper, t: u64, targets: &[Target], rates: Rates, slider: bool, out: &mut Run, pending: &mut Vec<u64>, moving: &mut bool| {
        let now = seconds(t);
        if slider {
            snapper.pause(now);
            return;
        }
        let arrived = snapper.arrived();
        snapper.tick(now, targets, rates);
        if !arrived && snapper.arrived() {
            if let Some(key) = snapper.snapped_id() {
                out.landings.push(Landing { t, key, at: snapper.flight().to });
                if std::mem::take(moving) {
                    pending.push(key);
                }
            }
        }
        if snapper.take_parked().is_some() {
            snapper.stop();
        }
    };
    for r in records {
        let t = r.t();
        // The steps up to this record.
        if snapper.holds_pointer() {
            let mut g = ticked.map_or(t.div_ceil(tick_us), |k| k / tick_us + 1) * tick_us;
            while g <= t {
                step(&mut snapper, g, &targets, rates, slider, &mut out, &mut pending, &mut moving);
                ticked = Some(g);
                if !snapper.holds_pointer() {
                    break;
                }
                g += tick_us;
            }
        }
        if !snapper.holds_pointer() {
            ticked = Some(ticked.map_or(t, |k| k.max(t)));
        }
        let now = seconds(t);
        match r {
            Record::Rate { updates_per_second, .. } => rates.updates_per_second = options.rate.unwrap_or(*updates_per_second),
            Record::Snapshot { targets: shown, .. } => targets.clone_from(shown),
            Record::Mode { change: ModeChange::On, at, .. } => {
                slider = false;
                pending.clear();
                moving = false;
                snapper.enter(now, *at, &targets, rates);
            }
            Record::Mode { change, .. } => {
                if std::mem::take(&mut slider) {
                    snapper.forget_motion();
                }
                if *change == ModeChange::Land {
                    let _ = snapper.leave();
                } else {
                    snapper.stop();
                }
                if snapper.take_parked().is_some() {
                    snapper.stop();
                }
            }
            Record::Motion { d, .. } => {
                if !slider && snapper.motion(*d) {
                    moving = true;
                }
            }
            Record::Pointer { kind: PointerKind::Rest, .. } => {
                // (The viewer stepped then: so does the replay.)
                if snapper.holds_pointer() && ticked.is_none_or(|k| k < t) {
                    step(&mut snapper, t, &targets, rates, slider, &mut out, &mut pending, &mut moving);
                    ticked = Some(t);
                }
            }
            Record::Click { button, down, used, target, .. } => {
                if !snapper.holds_pointer() {
                    continue;
                }
                if *down {
                    snapper.follow(&targets, now);
                }
                match used {
                    ClickUse::SliderEnd => {
                        slider = false;
                        snapper.forget_motion();
                    }
                    ClickUse::Swallowed => {}
                    ClickUse::SliderBegin => {
                        snapper.hold();
                        slider = true;
                    }
                    ClickUse::Pointer => {
                        if *down {
                            let recorded = key(&targets, *target);
                            out.wrong_landings += pending.iter().filter(|&&k| Some(k) != recorded).count();
                            pending.clear();
                            out.clicks.push(ClickOutcome { t, recorded, replayed: snapper.snapped_id() });
                        }
                        let actions = snapper.click(*button, *down).unwrap_or_default();
                        if actions.iter().any(|a| matches!(a, Action::Capture(_))) {
                            snapper.stop();
                        }
                        if snapper.take_parked().is_some() {
                            snapper.stop();
                        }
                    }
                }
            }
            Record::Pointer { .. } | Record::GaveUp { .. } | Record::Dropped { .. } | Record::End { .. } => {}
        }
    }
    out
}

/// What a recording shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// Times the mode came on.
    pub sessions: usize,
    pub snapshots: usize,
    pub motions: usize,
    /// Presses that went to the element snapped to (on one, or on none).
    pub clicks: usize,
    pub clicks_on_target: usize,
    /// Times the pointer came to rest.
    pub landings: usize,
    /// Landings motion led to on another element than the one clicked
    /// next in the same mode session.
    pub wrong_landings: usize,
    /// Of those, the ones whose motion pointed at the element clicked
    /// (within the cone, from where the motion started).
    pub aimed_wrong_landings: usize,
    /// Clicks with the free pointer on a box of the mode within ten
    /// seconds of its end ("gave up" on the mode).
    pub gave_up: usize,
    /// Landings of the mode before it ended on other elements than the
    /// one then clicked with the free pointer.
    pub wrong_before_giving_up: usize,
    pub dropped: u64,
    /// From the first record to the last (µs).
    pub span_us: u64,
}

/// A landing remembered until the next click: the element, where the
/// motion leading there started, and how far it went (window pixels).
#[derive(Copy, Clone, Debug)]
struct Pending {
    key: u64,
    origin: Point,
    moved: Point,
}

/// Counts what `records` (one session's, in order) show; `tuning` gives
/// the motion's gain and the cone for "pointed at".
pub fn summarize(records: &[Record], tuning: &Tuning) -> Summary {
    let mut s = Summary::default();
    let mut targets: Vec<Target> = Vec::new();
    let mut origin: Point = (0.0, 0.0);
    let mut moved: Point = (0.0, 0.0);
    let mut pending: Vec<Pending> = Vec::new();
    let mut exited: Vec<Pending> = Vec::new();
    // Motion came since the last landing (else a landing is the mode
    // snapping on, or the element moving).
    let mut burst = false;
    let wrong = |pending: &[Pending], targets: &[Target], clicked: u64| {
        let center = targets.iter().find(|t| t.id == clicked).map(Target::center);
        let mut n = (0, 0);
        for p in pending.iter().filter(|p| p.key != clicked) {
            n.0 += 1;
            if let Some(c) = center {
                let to = (c.0 - p.origin.0, c.1 - p.origin.1);
                let len = to.0.hypot(to.1) * p.moved.0.hypot(p.moved.1);
                if len > 0.0 && ((to.0 * p.moved.0 + to.1 * p.moved.1) / len).clamp(-1.0, 1.0).acos() <= tuning.cone {
                    n.1 += 1;
                }
            }
        }
        n
    };
    if let (Some(first), Some(last)) = (records.first(), records.last()) {
        s.span_us = last.t().saturating_sub(first.t());
    }
    for r in records {
        match r {
            Record::Rate { .. } | Record::End { .. } => {}
            Record::Snapshot { targets: shown, .. } => {
                s.snapshots += 1;
                targets.clone_from(shown);
            }
            Record::Mode { change: ModeChange::On, .. } => {
                s.sessions += 1;
                pending.clear();
                exited.clear();
                burst = false;
            }
            Record::Mode { .. } => exited = std::mem::take(&mut pending),
            Record::Motion { d, .. } => {
                s.motions += 1;
                moved = (moved.0 + d.0 * tuning.motion_gain, moved.1 + d.1 * tuning.motion_gain);
            }
            Record::Pointer { kind: PointerKind::Origin, at, .. } => {
                origin = *at;
                moved = (0.0, 0.0);
                burst = true;
            }
            Record::Pointer { kind: PointerKind::Rest, target, .. } => {
                s.landings += 1;
                let led = std::mem::take(&mut burst);
                if let Some(k) = key(&targets, *target).filter(|_| led) {
                    pending.push(Pending { key: k, origin, moved });
                }
            }
            Record::Click { down: true, used: ClickUse::Pointer, target, .. } => {
                s.clicks += 1;
                if let Some(clicked) = key(&targets, *target) {
                    s.clicks_on_target += 1;
                    let (all, aimed) = wrong(&pending, &targets, clicked);
                    s.wrong_landings += all;
                    s.aimed_wrong_landings += aimed;
                }
                pending.clear();
            }
            Record::Click { .. } => {}
            Record::GaveUp { target, .. } => {
                s.gave_up += 1;
                if let Some(clicked) = key(&targets, Some(*target)) {
                    s.wrong_before_giving_up += wrong(&exited, &targets, clicked).0;
                }
                exited.clear();
            }
            Record::Dropped { count, .. } => s.dropped += count,
        }
    }
    s
}

/// A session's recording: its files (parts) and their records, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub name: String,
    pub files: Vec<std::path::PathBuf>,
    pub records: Vec<Record>,
    /// Files cut short (the viewer stopped while writing).
    pub cut: Vec<String>,
}

/// Reads recordings: files, or folders of them; parts of one session
/// (`alt-mode-<time>-<pid>-<part>.altrec`) are joined in order.
pub fn sessions(paths: &[std::path::PathBuf]) -> Result<Vec<Session>, String> {
    let mut files = Vec::new();
    for p in paths {
        if p.is_dir() {
            files.extend(super::writer::recordings(p).into_iter().map(|(f, _)| f));
        } else {
            files.push(p.clone());
        }
    }
    files.sort();
    let mut out: Vec<Session> = Vec::new();
    for f in files {
        let bytes = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        let file = super::format::decode(&bytes).map_err(|e| format!("{}: {e}", f.display()))?;
        let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let name = stem.rsplit_once('-').map_or(stem, |(session, _)| session).to_owned();
        if out.last().is_none_or(|s| s.name != name) {
            out.push(Session { name, ..Session::default() });
        }
        let s = out.last_mut().unwrap();
        if let Some(cut) = file.cut {
            s.cut.push(format!("{}: {cut}", f.display()));
        }
        s.files.push(f);
        s.records.extend(file.records);
    }
    Ok(out)
}

fn share(n: usize, of: usize) -> String {
    if of == 0 { "-".into() } else { format!("{:.0} %", 100.0 * n as f64 / of as f64) }
}

/// What the driver's altrec tool prints for a session: what the recording shows,
/// and what the picker does with `options` over the same motion.
pub fn report(session: &Session, options: &Options) -> String {
    use std::fmt::Write;
    let s = summarize(&session.records, &options.tuning);
    let r = run(&session.records, options);
    let mut o = String::new();
    let _ = writeln!(o, "{} ({} file{}, {:.1} s)", session.name, session.files.len(), if session.files.len() == 1 { "" } else { "s" }, s.span_us as f64 / 1e6);
    for cut in &session.cut {
        let _ = writeln!(o, "  cut short: {cut}");
    }
    let _ = writeln!(o, "  recorded: the mode on {} time(s), {} target snapshots, {} motions, {} landings, {} clicks ({} on a target)", s.sessions, s.snapshots, s.motions, s.landings, s.clicks, s.clicks_on_target);
    let _ = writeln!(o, "    landings elsewhere before the click: {} ({:.2} a click), {} of them with the motion pointing at the element clicked", s.wrong_landings, s.wrong_landings as f64 / s.clicks_on_target.max(1) as f64, s.aimed_wrong_landings);
    let _ = writeln!(o, "    gave up (clicked a box of the mode with the free pointer within 10 s): {}, after {} landings elsewhere", s.gave_up, s.wrong_before_giving_up);
    if s.dropped > 0 {
        let _ = writeln!(o, "    records dropped (the writer fell behind): {}", s.dropped);
    }
    let reached = r.reached();
    let _ = writeln!(o, "  replayed: {} landings, {} landings elsewhere before the click; on the clicked element at {} of {} clicks ({})", r.landings.len(), r.wrong_landings, reached, r.clicks.len(), share(reached, r.clicks.len()));
    o
}
