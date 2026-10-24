//! The viewer's pointer thread: samples the pointer on a steady clock of
//! its own, about twice the display's refresh rate, whatever the main
//! thread (window events, layout, frame preparation) is doing.
//!
//! The main thread hands it the pointer from each `CursorMoved`. A backend
//! that tracks the pointer off the main thread (macOS's event tap,
//! `pointer_api::Backend::feed`) is read directly, so the pointer is known
//! while the main thread is stalled too. Whatever samples it (edge
//! scrolling, `edge_clock`) reads it from here.
//!
//! Nothing here waits on the main thread or the renderer, nor they on it:
//! values go through `handoff::Latest` and atomics. The thread sleeps
//! to absolute deadlines (`std::thread::sleep`, high-resolution on every
//! platform), never spins, and checks once a tick whether to end; dropping
//! [`PointerThread`] ends and joins it within one tick.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use handoff::Latest;

/// A reader of the pointer's position (physical window pixels) callable
/// from any thread (`pointer_control::Feed`).
pub type Feed = Arc<dyn Fn() -> Option<(f64, f64)> + Send + Sync>;

/// The tick for a display refreshing `refresh_mhz` times a thousand
/// seconds: twice its rate, 120 Hz assumed when unknown, between 120 and
/// 1000 ticks a second.
pub fn tick_for(refresh_mhz: Option<u32>) -> Duration {
    let refresh = refresh_mhz.filter(|&r| r > 0).map_or(120.0, |r| r as f64 / 1000.0);
    let rate = (2.0 * refresh).clamp(120.0, 1000.0);
    Duration::from_secs_f64(1.0 / rate)
}

/// What one tick saw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick {
    /// The pointer in physical window pixels, if known.
    pub pointer: Option<(f64, f64)>,
    /// Seconds since the previous tick.
    pub dt: f64,
    /// When it ticked.
    pub at: Instant,
}

/// Work done on every tick, on the pointer thread.
pub trait OnTick: Send + Sync {
    fn tick(&self, tick: Tick);
}

/// How regularly the thread ticked since last asked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rate {
    pub ticks: u64,
    pub mean: Duration,
    pub max: Duration,
}

#[derive(Default)]
struct Stats {
    ticks: AtomicU64,
    total_ns: AtomicU64,
    max_ns: AtomicU64,
}

impl Stats {
    fn record(&self, interval: Duration) {
        let ns = interval.as_nanos().min(u64::MAX as u128) as u64;
        self.ticks.fetch_add(1, Ordering::AcqRel);
        self.total_ns.fetch_add(ns, Ordering::AcqRel);
        self.max_ns.fetch_max(ns, Ordering::AcqRel);
    }

    fn take(&self) -> Rate {
        let ticks = self.ticks.swap(0, Ordering::AcqRel);
        let total = self.total_ns.swap(0, Ordering::AcqRel);
        let max = self.max_ns.swap(0, Ordering::AcqRel);
        Rate { ticks, mean: Duration::from_nanos(total.checked_div(ticks).unwrap_or(0)), max: Duration::from_nanos(max) }
    }
}

struct Shared {
    stop: AtomicBool,
    tick_ns: AtomicU64,
    /// The pointer as the main thread last saw it; `None`: gone.
    reported: Latest<Option<(f64, f64)>>,
    feed: Option<Feed>,
    /// The newest position a tick used (for readers on other threads).
    sampled: Latest<(f64, f64)>,
    stats: Stats,
    work: Vec<Arc<dyn OnTick>>,
}

/// The pointer thread (see the module). Ends when dropped.
pub struct PointerThread {
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
}

impl PointerThread {
    /// Starts ticking every `tick`, reading `feed` if there is one, doing
    /// `work` on each tick.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn spawn(tick: Duration, feed: Option<Feed>, work: Vec<Arc<dyn OnTick>>) -> std::io::Result<Self> {
        Self::spawn_with(tick, feed, work, None)
    }

    /// [`Self::spawn`], running the host's `thread_init` first on the thread
    /// (`ViewerConfig::native_thread_init`).
    pub fn spawn_with(tick: Duration, feed: Option<Feed>, work: Vec<Arc<dyn OnTick>>, thread_init: Option<fn()>) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            tick_ns: AtomicU64::new(tick.as_nanos().max(1) as u64),
            reported: Latest::new(),
            feed,
            sampled: Latest::new(),
            stats: Stats::default(),
            work,
        });
        let thread = shared.clone();
        let handle = std::thread::Builder::new().name("viewer-pointer".into()).spawn(move || crate::thread_start::initialized(thread_init, || run(&thread)))?;
        Ok(Self { shared, handle: Some(handle) })
    }

    /// The display's refresh rate changed (the window moved to another).
    pub fn set_tick(&self, tick: Duration) {
        self.shared.tick_ns.store(tick.as_nanos().max(1) as u64, Ordering::Release);
    }

    pub fn tick(&self) -> Duration {
        Duration::from_nanos(self.shared.tick_ns.load(Ordering::Acquire))
    }

    /// The pointer moved to `at` (physical window pixels); `None`: it left
    /// the window.
    pub fn report(&self, at: Option<(f64, f64)>) {
        self.shared.reported.publish(at);
    }

    /// Where the last tick saw the pointer.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn sampled(&self) -> Option<(f64, f64)> {
        self.shared.sampled.get()
    }

    /// How regularly it ticked since the last call.
    pub fn take_rate(&self) -> Rate {
        self.shared.stats.take()
    }
}

impl Drop for PointerThread {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            // (It ends within one tick: it waits on nothing but its sleep.)
            let _ = handle.join();
        }
    }
}

fn run(shared: &Shared) {
    let mut last = Instant::now();
    let mut next = last;
    while !shared.stop.load(Ordering::Acquire) {
        let tick = Duration::from_nanos(shared.tick_ns.load(Ordering::Acquire));
        next += tick;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > tick {
            // (Woken far too late, e.g. the machine slept: no catching up.)
            next = now;
        }
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        let now = Instant::now();
        let interval = now - last;
        last = now;
        shared.stats.record(interval);
        let pointer = shared.feed.as_ref().and_then(|f| f()).or_else(|| shared.reported.get().flatten());
        if let Some(p) = pointer {
            shared.sampled.publish(p);
        }
        let t = Tick { pointer, dt: interval.as_secs_f64(), at: now };
        for work in &shared.work {
            work.tick(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn the_tick_is_twice_the_refresh_rate_within_bounds() {
        let hz = |d: Duration| (1.0 / d.as_secs_f64()).round() as u32;
        assert_eq!(hz(tick_for(Some(120_000))), 240);
        assert_eq!(hz(tick_for(Some(60_000))), 120);
        assert_eq!(hz(tick_for(Some(59_940))), 120);
        assert_eq!(hz(tick_for(None)), 240);
        assert_eq!(hz(tick_for(Some(0))), 240);
        assert_eq!(hz(tick_for(Some(30_000))), 120, "never slower than 120 Hz");
        assert_eq!(hz(tick_for(Some(1_000_000))), 1000, "never faster than 1 kHz");
    }

    struct Count(AtomicUsize);
    impl OnTick for Count {
        fn tick(&self, _: Tick) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// A busy main thread (50 ms frames spinning on the CPU) does not slow
    /// the pointer thread: it keeps its 240 Hz.
    #[test]
    fn it_keeps_its_rate_while_the_render_thread_stalls() {
        let count = Arc::new(Count(AtomicUsize::new(0)));
        let tick = tick_for(Some(120_000));
        let thread = PointerThread::spawn(tick, None, vec![count.clone() as Arc<dyn OnTick>]).unwrap();
        let start = Instant::now();
        // Ten stalled frames, the pointer reported once each.
        for frame in 0..10 {
            let until = Instant::now() + Duration::from_millis(50);
            while Instant::now() < until {
                std::hint::spin_loop();
            }
            thread.report(Some((frame as f64, 0.0)));
        }
        let elapsed = start.elapsed();
        let rate = thread.take_rate();
        drop(thread);
        let expected = elapsed.as_secs_f64() / tick.as_secs_f64();
        assert!(rate.ticks as f64 > 0.6 * expected, "{rate:?} in {elapsed:?}, {expected:.0} expected");
        assert!(rate.max < Duration::from_millis(45), "a tick waited for a 50 ms frame: {rate:?}");
        assert!(count.0.load(Ordering::Acquire) as u64 >= rate.ticks);
    }

    /// The feed wins over the main thread's last report (it is newer).
    #[test]
    fn a_feed_is_read_on_every_tick() {
        let feed: Feed = Arc::new(|| Some((7.0, 8.0)));
        let thread = PointerThread::spawn(Duration::from_millis(2), Some(feed), vec![]).unwrap();
        thread.report(Some((1.0, 2.0)));
        let until = Instant::now() + Duration::from_secs(2);
        while thread.sampled().is_none() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(thread.sampled(), Some((7.0, 8.0)));
    }

    /// Dropping it ends it promptly, at any tick rate: within one tick, plus
    /// however long a loaded machine takes to schedule it (no deadlock).
    #[test]
    fn it_ends_without_waiting_at_shutdown() {
        for tick in [Duration::from_micros(500), tick_for(None), Duration::from_millis(8)] {
            let thread = PointerThread::spawn(tick, None, vec![Arc::new(Count(AtomicUsize::new(0))) as Arc<dyn OnTick>]).unwrap();
            std::thread::sleep(Duration::from_millis(20));
            let start = Instant::now();
            drop(thread);
            assert!(start.elapsed() < Duration::from_secs(1), "{tick:?}: {:?}", start.elapsed());
        }
    }

    /// Printed with `--ignored --nocapture`: pointer update intervals with
    /// the render thread stalled, before (sampled per main-thread frame)
    /// and after (the pointer thread).
    #[test]
    #[ignore]
    fn pointer_intervals_under_render_load() {
        for frame_ms in [8u64, 50] {
            let tick = tick_for(Some(120_000));
            let thread = PointerThread::spawn(tick, None, vec![]).unwrap();
            let start = Instant::now();
            let mut frames = Vec::new();
            let mut last = Instant::now();
            while start.elapsed() < Duration::from_secs(1) {
                let until = Instant::now() + Duration::from_millis(frame_ms);
                while Instant::now() < until {
                    std::hint::spin_loop();
                }
                frames.push(last.elapsed());
                last = Instant::now();
            }
            let rate = thread.take_rate();
            let mean = frames.iter().sum::<Duration>() / frames.len() as u32;
            let max = frames.iter().max().copied().unwrap_or_default();
            println!("render frames of {frame_ms} ms: main thread pointer updates every {mean:.2?} (max {max:.2?}); pointer thread every {:.2?} (max {:.2?}, {} ticks)", rate.mean, rate.max, rate.ticks);
        }
    }
}
