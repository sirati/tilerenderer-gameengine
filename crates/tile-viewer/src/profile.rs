//! Where a program's time goes, for profiling (`TILE_VIEWER_PROFILE=1` in
//! the viewer; a driver may keep one too): named spans timed per round,
//! counts, and the time between rounds, summarised as percentiles and
//! logged every few seconds.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Whether `var` is set to something other than empty or `0`.
pub fn env_on(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Timings and counts, logged as a summary every `every`.
pub struct Profile {
    name: &'static str,
    every: Duration,
    since: Instant,
    /// Milliseconds per span, one sample per round it ran in (summed
    /// within a round).
    spans: BTreeMap<String, Vec<f64>>,
    /// The spans of the round being recorded.
    round: BTreeMap<String, f64>,
    /// Counts per round (e.g. sprites drawn), one sample per round.
    counts: BTreeMap<&'static str, Vec<f64>>,
    /// Milliseconds between the starts of rounds.
    intervals: Vec<f64>,
    last_round: Option<Instant>,
    rounds: u32,
    /// Events counted over the period (e.g. scenes received).
    events: BTreeMap<&'static str, u64>,
}

/// p50, p95, p99 and the maximum of `v` (sorted in place).
pub fn percentiles(v: &mut [f64]) -> (f64, f64, f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() as f64 - 1.0) * q).round() as usize];
    (at(0.5), at(0.95), at(0.99), v[v.len() - 1])
}

impl Profile {
    pub fn new(name: &'static str, every: Duration) -> Profile {
        Profile {
            name,
            every,
            since: Instant::now(),
            spans: BTreeMap::new(),
            round: BTreeMap::new(),
            counts: BTreeMap::new(),
            intervals: Vec::new(),
            last_round: None,
            rounds: 0,
            events: BTreeMap::new(),
        }
    }

    /// A round (a frame, a publish) starts: the time since the last one is
    /// recorded.
    pub fn round_start(&mut self) {
        let now = Instant::now();
        if let Some(t) = self.last_round.replace(now) {
            self.intervals.push((now - t).as_secs_f64() * 1000.0);
        }
    }

    /// Adds `d` to span `name` of this round.
    pub fn add(&mut self, name: &str, d: Duration) {
        self.add_ms(name, d.as_secs_f64() * 1000.0);
    }

    pub fn add_ms(&mut self, name: &str, ms: f64) {
        match self.round.get_mut(name) {
            Some(v) => *v += ms,
            None => {
                self.round.insert(name.to_owned(), ms);
            }
        }
    }

    /// Times `f` as span `name`.
    pub fn time<R>(&mut self, name: &str, f: impl FnOnce() -> R) -> R {
        let t = Instant::now();
        let r = f();
        self.add(name, t.elapsed());
        r
    }

    /// Records count `name` of this round.
    pub fn count(&mut self, name: &'static str, n: usize) {
        self.counts.entry(name).or_default().push(n as f64);
    }

    /// Counts an event over the period.
    pub fn event(&mut self, name: &'static str, n: u64) {
        *self.events.entry(name).or_default() += n;
    }

    /// Ends the round: its spans become samples.
    pub fn round_end(&mut self) {
        self.rounds += 1;
        for (k, v) in std::mem::take(&mut self.round) {
            self.spans.entry(k).or_default().push(v);
        }
    }

    /// The summary, once `every` has passed since the last (then starts
    /// afresh).
    pub fn report(&mut self) -> Option<String> {
        let span = self.since.elapsed();
        if span < self.every || self.rounds == 0 {
            return None;
        }
        self.since = Instant::now();
        let secs = span.as_secs_f64();
        let fmt = |(p50, p95, p99, max): (f64, f64, f64, f64)| format!("p50 {p50:.2} p95 {p95:.2} p99 {p99:.2} max {max:.2}");
        let mut out = format!("{} profile over {secs:.1} s: {} rounds ({:.1}/s)", self.name, self.rounds, self.rounds as f64 / secs);
        if !self.intervals.is_empty() {
            out += &format!("; interval ms {}", fmt(percentiles(&mut self.intervals)));
        }
        for (k, v) in &self.events {
            out += &format!("; {k} {v} ({:.1}/s)", *v as f64 / secs);
        }
        let rounds = self.rounds as usize;
        for (k, v) in &mut self.spans {
            // (Rounds without the span count as 0 in the mean.)
            let mean = v.iter().sum::<f64>() / rounds as f64;
            out += &format!("\n  {k:<28} ms: n {:>5} mean/round {mean:>7.3} {}", v.len(), fmt(percentiles(v)));
        }
        for (k, v) in &mut self.counts {
            let (p50, _, _, max) = percentiles(v);
            out += &format!("\n  {k:<28} count p50 {p50:.0} max {max:.0}");
        }
        self.spans.clear();
        self.counts.clear();
        self.intervals.clear();
        self.events.clear();
        self.rounds = 0;
        Some(out)
    }
}
