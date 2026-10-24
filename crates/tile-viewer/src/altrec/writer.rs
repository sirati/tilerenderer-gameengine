//! The recording's writer thread: it takes the records the viewer's
//! thread queued ([`Sink`]), encodes them and appends them to the
//! session's file, a new part once a file is [`Limits::part`] bytes, and
//! deletes the oldest recordings beyond [`Limits::total`].
//!
//! The viewer's thread never waits for it: pushes onto the queue drop and
//! count when it is full; the writer polls the queue, woken early by a
//! [`civ3r_sync::Wake`] when it fills up, and on close, which waits for
//! the writer at most a bounded time.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use civ3r_sync::{Bounded, Wake};

use super::Record;
use super::format::{Encoder, Header};

/// Recordings are files `alt-mode-<Unix seconds>-<pid>-<part>.altrec`.
pub const PREFIX: &str = "alt-mode-";
pub const EXTENSION: &str = "altrec";

/// How large recordings grow.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Limits {
    /// A file is closed and the next part begun past this many bytes.
    pub part: u64,
    /// All recordings in the folder together: the oldest files are
    /// deleted beyond this (never the one being written).
    pub total: u64,
    /// Records the queue holds before it drops.
    pub queue: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { part: 8 << 20, total: 64 << 20, queue: 16 * 1024 }
    }
}

/// How often the writer looks at the queue when nobody wakes it.
const POLL: Duration = Duration::from_millis(250);

/// What the viewer's thread and the writer share.
pub struct Sink {
    pub queue: Bounded<Record>,
    /// Rung when the queue is half full, and on close.
    pub wake: Wake,
    /// The viewer closes the recording.
    pub closing: AtomicBool,
    /// The writer wrote everything and closed the file.
    pub done: AtomicBool,
    /// Rung by the writer once done.
    pub finished: Wake,
}

impl Sink {
    pub fn new(queue: usize) -> Self {
        Self { queue: Bounded::new(queue), wake: Wake::new(), closing: AtomicBool::new(false), done: AtomicBool::new(false), finished: Wake::new() }
    }

    /// Queues `r` (dropped and counted if the queue is full); wakes the
    /// writer once the queue is half full.
    pub fn push(&self, r: Record) {
        self.queue.push(r);
        if self.queue.len() * 2 >= self.queue.capacity() {
            self.wake.ring();
        }
    }
}

/// The recordings in `dir`, oldest first (their names sort by time).
pub fn recordings(dir: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(PathBuf, u64)> = entries
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with(PREFIX) && n.ends_with(&format!(".{EXTENSION}"))))
        .filter_map(|e| Some((e.path(), e.metadata().ok().filter(|m| m.is_file())?.len())))
        .collect();
    out.sort();
    out
}

/// Deletes the oldest recordings in `dir` until they total at most
/// `total` bytes, keeping `current`.
pub fn prune(dir: &Path, total: u64, current: Option<&Path>) {
    let files = recordings(dir);
    let mut sum: u64 = files.iter().map(|(_, n)| n).sum();
    for (path, len) in files {
        if sum <= total {
            break;
        }
        if Some(path.as_path()) == current {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => sum -= len,
            Err(e) => log::warn!("altrec: deleting the old recording {}: {e}", path.display()),
        }
    }
}

/// The session's files.
struct Output {
    dir: PathBuf,
    stem: String,
    wall_us: u64,
    limits: Limits,
    part: u32,
    file: Option<(std::fs::File, PathBuf)>,
    written: u64,
    encoder: Encoder,
    /// What a new part states first: the rate and the targets in effect.
    rate: Option<Record>,
    snapshot: Option<Record>,
    buf: Vec<u8>,
    failed: bool,
}

impl Output {
    fn open(&mut self, base_us: u64) {
        let path = self.dir.join(format!("{}-{:03}.{EXTENSION}", self.stem, self.part));
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path);
        match file {
            Ok(file) => {
                log::info!("altrec: recording the Alt mode to {}", path.display());
                self.file = Some((file, path));
                self.written = 0;
                self.encoder = Encoder::new(base_us);
                Header { wall_us: self.wall_us, base_us, part: self.part }.write(&mut self.buf);
                // (Each part stands alone.)
                for r in [self.rate.clone(), self.snapshot.clone()].into_iter().flatten() {
                    self.encoder.record(&r, &mut self.buf);
                }
            }
            Err(e) => {
                log::warn!("altrec: opening {}: {e}; not recording", path.display());
                self.failed = true;
            }
        }
    }

    fn add(&mut self, r: Record) {
        if self.failed {
            return;
        }
        if self.file.is_none() {
            self.open(r.t());
        }
        self.encoder.record(&r, &mut self.buf);
        match r {
            Record::Rate { .. } => self.rate = Some(r),
            Record::Snapshot { .. } => self.snapshot = Some(r),
            _ => {}
        }
    }

    /// Writes what was encoded; a new part (and old ones deleted) once
    /// this one is full.
    fn flush(&mut self, last_t: u64) {
        if self.buf.is_empty() || self.failed {
            return;
        }
        let Some((file, path)) = &mut self.file else { return };
        if let Err(e) = file.write_all(&self.buf) {
            log::warn!("altrec: writing {}: {e}; not recording", path.display());
            self.failed = true;
            return;
        }
        self.written += self.buf.len() as u64;
        self.buf.clear();
        if self.written >= self.limits.part {
            self.file = None;
            self.part += 1;
            self.open(last_t);
            self.flush_header();
        }
        let current = self.file.as_ref().map(|(_, p)| p.clone());
        prune(&self.dir, self.limits.total, current.as_deref());
    }

    /// A new part's header and restated records, written at once.
    fn flush_header(&mut self) {
        if let Some((file, _)) = &mut self.file {
            if file.write_all(&self.buf).is_ok() {
                self.written += self.buf.len() as u64;
            }
        }
        self.buf.clear();
    }
}

/// Starts the writer for a session's recording in `dir`: files
/// `alt-mode-<wall seconds>-<pid>-<part>.altrec`, made when the first
/// record comes.
pub fn start(dir: PathBuf, wall_us: u64, limits: Limits) -> std::io::Result<Arc<Sink>> {
    let sink = Arc::new(Sink::new(limits.queue));
    let mut out = Output {
        dir,
        stem: format!("{PREFIX}{:010}-{}", wall_us / 1_000_000, std::process::id()),
        wall_us,
        limits,
        part: 0,
        file: None,
        written: 0,
        encoder: Encoder::default(),
        rate: None,
        snapshot: None,
        buf: Vec::new(),
        failed: false,
    };
    let shared = sink.clone();
    std::thread::Builder::new().name("civ3r-altrec".into()).spawn(move || {
        let mut last_t = 0;
        loop {
            let closing = shared.closing.load(Ordering::Acquire);
            for r in shared.queue.drain() {
                last_t = last_t.max(r.t());
                out.add(r);
            }
            let dropped = shared.queue.take_dropped();
            if dropped > 0 {
                out.add(Record::Dropped { t: last_t, count: dropped });
            }
            if closing {
                if out.file.is_some() {
                    out.add(Record::End { t: last_t });
                }
                out.flush(last_t);
                break;
            }
            out.flush(last_t);
            shared.wake.wait(POLL);
        }
        shared.done.store(true, Ordering::Release);
        shared.finished.ring();
    })?;
    Ok(sink)
}

/// Closes the recording: the writer writes what is queued and an end,
/// waited for at most `wait` (the rest is lost if it takes longer).
/// Whether it finished. Called from one thread only.
pub fn finish(sink: &Sink, wait: Duration) -> bool {
    sink.closing.store(true, Ordering::Release);
    sink.wake.ring();
    let until = std::time::Instant::now() + wait;
    while !sink.done.load(Ordering::Acquire) {
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return false;
        }
        sink.finished.wait(left);
    }
    true
}
