//! The viewer thread's side of the recording: it turns what the Alt mode
//! does into [`Record`]s and queues them for the writer
//! ([`super::writer`]). Cheap: a snapshot of the targets is compared
//! element by element and built only when it changed; the other records
//! are small values.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use scene::Button;

use crate::snap::{Point, Target, TargetKind, IDLE_RESET};

use super::writer::{self, Limits, Sink};
use super::{ClickUse, ModeChange, PointerKind, Reason, Record, micros};

/// How long after the mode ends a click with the free pointer on a box of
/// the last snapshot counts as the player giving up on the mode (seconds).
pub const AFTER_EXIT: f64 = 10.0;

/// How long [`Recorder::finish`] waits for the writer on close.
pub const FINISH_WAIT: Duration = Duration::from_millis(100);

/// The watch after the mode ended ([`AFTER_EXIT`]).
#[derive(Clone, Debug, PartialEq)]
struct Exit {
    at: f64,
    /// The last snapshot written while the mode was on.
    snapshot: u32,
}

/// Records the Alt mode's interactions (see [`super`]).
pub struct Recorder {
    sink: Arc<Sink>,
    /// The writer's queue is ours alone (tests): nobody writes it out.
    finished: bool,
    /// Each element's key (by the driver's id): the same through the
    /// session.
    keys: HashMap<u64, u32>,
    /// The last snapshot written (ids replaced by keys), and its number
    /// (0: none yet).
    last: Vec<Target>,
    number: u32,
    rate: Option<f64>,
    /// When raw motion last came, and whether the pointer rested since.
    last_motion: Option<f64>,
    rested: bool,
    exit: Option<Exit>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder").field("snapshot", &self.number).field("queued", &self.sink.queue.len()).field("dropped", &self.sink.queue.dropped()).finish()
    }
}

impl Recorder {
    fn with_sink(sink: Arc<Sink>) -> Self {
        Self { sink, finished: false, keys: HashMap::new(), last: Vec::new(), number: 0, rate: None, last_motion: None, rested: true, exit: None }
    }

    /// Records into files in `dir` (next to the game's log), written by a
    /// thread of its own.
    pub fn start(dir: std::path::PathBuf, limits: Limits) -> std::io::Result<Self> {
        let wall = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64);
        std::fs::create_dir_all(&dir)?;
        Ok(Self::with_sink(writer::start(dir, wall, limits)?))
    }

    /// Records into a queue of `capacity` only [`Recorder::drain`] takes
    /// (tests, replays of a live run).
    pub fn in_memory(capacity: usize) -> Self {
        let mut r = Self::with_sink(Arc::new(Sink::new(capacity)));
        r.finished = true;
        r
    }

    /// What was queued so far (an in-memory recorder's records).
    pub fn drain(&self) -> Vec<Record> {
        self.sink.queue.drain().collect()
    }

    /// Records dropped because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.sink.queue.dropped()
    }

    /// Closes the recording: what is queued is written and the file
    /// closed, waited for at most [`FINISH_WAIT`].
    pub fn finish(&mut self) {
        if !std::mem::replace(&mut self.finished, true) && !writer::finish(&self.sink, FINISH_WAIT) {
            log::warn!("altrec: the writer did not finish in {} ms", FINISH_WAIT.as_millis());
        }
    }

    fn push(&self, r: Record) {
        self.sink.push(r);
    }

    fn key(&mut self, id: u64) -> u32 {
        let next = self.keys.len() as u32;
        *self.keys.entry(id).or_insert(next)
    }

    /// The index in the last snapshot of the element with the driver's
    /// `id`.
    fn index(&self, id: Option<u64>) -> Option<u32> {
        let key = *self.keys.get(&id?)?;
        self.last.iter().position(|t| t.id == key as u64).map(|k| k as u32)
    }

    fn same(&self, targets: &[Target]) -> bool {
        self.number != 0
            && self.last.len() == targets.len()
            && self.last.iter().zip(targets).all(|(a, b)| {
                self.keys.get(&b.id).is_some_and(|&k| k as u64 == a.id) && a.rect == b.rect && a.area == b.area && a.spread == b.spread && a.anchor == b.anchor && a.kind == b.kind
            })
    }

    /// The targets as they are at `now`: a snapshot written if they
    /// differ from the last one. Its number.
    pub fn observe(&mut self, now: f64, targets: &[Target]) -> u32 {
        if !self.same(targets) {
            let shown: Vec<Target> = targets.iter().map(|t| Target { id: self.key(t.id) as u64, alt_right_click: false, ..*t }).collect();
            self.number += 1;
            self.last = shown.clone();
            self.push(Record::Snapshot { t: micros(now), number: self.number, targets: shown });
        }
        self.number
    }

    /// The Alt mode's update rate in effect, written when it changes.
    pub fn rate(&mut self, now: f64, updates_per_second: f64) {
        if self.rate != Some(updates_per_second) {
            self.rate = Some(updates_per_second);
            self.push(Record::Rate { t: micros(now), updates_per_second });
        }
    }

    /// The mode turned on, the pointer at `at` before it snapped, the
    /// targets as they are.
    pub fn mode_on(&mut self, now: f64, reason: Reason, at: Point, targets: &[Target], updates_per_second: f64) {
        self.exit = None;
        self.rate(now, updates_per_second);
        let snapshot = self.observe(now, targets);
        self.push(Record::Mode { t: micros(now), change: ModeChange::On, reason, at, snapshot });
        self.rested = true;
    }

    /// The mode turned off: the watch for a click on its targets starts.
    pub fn mode_off(&mut self, now: f64, change: ModeChange, reason: Reason) {
        self.push(Record::Mode { t: micros(now), change, reason, at: (0.0, 0.0), snapshot: 0 });
        self.exit = (self.number != 0).then_some(Exit { at: now, snapshot: self.number });
    }

    /// Raw motion the Alt pointer takes; first, where it starts from if it
    /// is a new motion (`origin`: the pointer drawn, the element snapped
    /// to).
    pub fn motion(&mut self, now: f64, d: Point, origin: impl FnOnce() -> (Point, Option<u64>)) {
        let new = self.rested || self.last_motion.is_none_or(|t| now - t > IDLE_RESET);
        if new {
            let (at, id) = origin();
            self.pointer(now, PointerKind::Origin, at, id);
        }
        self.last_motion = Some(now);
        self.rested = false;
        self.push(Record::Motion { t: micros(now), d });
    }

    /// The Alt pointer at `at`, on (or heading for) the element `id`.
    pub fn pointer(&mut self, now: f64, kind: PointerKind, at: Point, id: Option<u64>) {
        if kind == PointerKind::Rest {
            self.rested = true;
        }
        self.push(Record::Pointer { t: micros(now), kind, at, snapshot: self.number, target: self.index(id) });
    }

    /// A button in the mode, the element `id` snapped to.
    pub fn click(&mut self, now: f64, button: Button, down: bool, used: ClickUse, id: Option<u64>) {
        if down && used == ClickUse::Pointer {
            self.rested = true;
        }
        self.push(Record::Click { t: micros(now), button, down, used, snapshot: self.number, target: self.index(id) });
    }

    /// A press with the free pointer at `at` after the mode, on the element
    /// whose box (as shown now) is `hit`: recorded if within
    /// [`AFTER_EXIT`] of the mode's end and the box is one of the last
    /// snapshot's; that ends the watch.
    pub fn click_after(&mut self, now: f64, button: Button, at: Point, hit: Option<[f64; 4]>) {
        let Some(exit) = &self.exit else { return };
        if now - exit.at > AFTER_EXIT || exit.snapshot != self.number {
            self.exit = None;
            return;
        }
        let Some(rect) = hit else { return };
        let Some(k) = self.last.iter().position(|t| t.rect == rect && !matches!(t.kind, TargetKind::Capture { .. })) else { return };
        let since = micros(now).saturating_sub(micros(exit.at));
        self.push(Record::GaveUp { t: micros(now), since_exit_us: since, snapshot: exit.snapshot, target: k as u32, at, button });
        self.exit = None;
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.finish();
    }
}
