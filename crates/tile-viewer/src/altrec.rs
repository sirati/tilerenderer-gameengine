//! Recording the Alt mode's interactions, to tune its picking from real
//! use (`docs/alt-mode-recording.md`).
//!
//! While the mode holds the pointer the viewer records the mouse's raw
//! motion, where the Alt pointer rests and where each new motion starts
//! from, the clicks, the mode turning on and off, the Alt mode's update
//! rate, and the targets ([`crate::snap::Target`]) as boxes: their
//! rectangles and what picking reads of them, never which button of the
//! game they are. A snapshot of the targets is written only when it
//! differs from the last one; records name a target by its index in it.
//! For ten seconds after the mode ends, the first click with the free
//! pointer on a box of the last snapshot is recorded too ("gave up").
//!
//! The viewer's thread builds [`Record`]s ([`Recorder`]) and hands them
//! to a writer thread through a [`civ3r_sync::Bounded`] queue: nothing on
//! the viewer's thread waits for the disk, and a record that does not fit
//! is dropped and counted. The writer encodes them ([`format`]) into an
//! append-only file next to the game's log, a new part every few
//! megabytes, the oldest deleted beyond a total ([`writer`]).
//! [`replay`] reads a recording and runs the picker over it again.

pub mod format;
pub mod recorder;
pub mod replay;
pub mod writer;

pub use recorder::Recorder;

use crate::snap::{Point, Target};
use scene::Button;

/// What a click did in the mode.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClickUse {
    /// It went to the element snapped to (or the attractor).
    Pointer,
    /// A left press on a slider snapped to: an Alt-mode slider edit
    /// starts (motion edits the slider until the next press).
    SliderBegin,
    /// A press ending a slider edit (it goes nowhere else).
    SliderEnd,
    /// The release of a press that went nowhere (an attractor's, a slider
    /// edit's).
    Swallowed,
}

/// How the mode changed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ModeChange {
    On,
    /// Off: the pointer lands on the element it heads for.
    Land,
    /// Off at once (an attractor clicked, focus lost, a shortcut, a
    /// command waiting for a place of the world).
    Stop,
}

/// Why the mode changed, as the viewer knows it anyway.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    Unknown,
    Alt,
    Esc,
    Ctrl,
    MenuOpened,
    MenusClosed,
    FocusLost,
    Attractor,
    Shortcut,
    WorldTarget,
}

impl Reason {
    pub const ALL: [Reason; 10] = [Reason::Unknown, Reason::Alt, Reason::Esc, Reason::Ctrl, Reason::MenuOpened, Reason::MenusClosed, Reason::FocusLost, Reason::Attractor, Reason::Shortcut, Reason::WorldTarget];

    pub fn name(self) -> &'static str {
        match self {
            Reason::Unknown => "unknown",
            Reason::Alt => "Alt",
            Reason::Esc => "Esc",
            Reason::Ctrl => "Ctrl",
            Reason::MenuOpened => "menu opened",
            Reason::MenusClosed => "menus closed",
            Reason::FocusLost => "focus lost",
            Reason::Attractor => "attractor",
            Reason::Shortcut => "shortcut",
            Reason::WorldTarget => "command targets the world",
        }
    }
}

/// Which Alt pointer position a [`Record::Pointer`] is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PointerKind {
    /// The flight landed: the pointer rests on its target.
    Rest,
    /// A new motion starts here (the first after a rest, or after the
    /// mouse was still for [`crate::snap::IDLE_RESET`]).
    Origin,
}

/// One recorded event. `t`: microseconds on the viewer's clock (the
/// snapping pointer's, since the viewer started). `snapshot`: the number
/// of a [`Record::Snapshot`] written before; `target`: an index into its
/// targets.
#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    /// The Alt mode's updates per second (`snap::Rates`), once at the
    /// start of each file and whenever it changes.
    Rate { t: u64, updates_per_second: f64 },
    /// Everything the Alt pointer can snap to, in window pixels. Each
    /// target's `id` is a key that stays the same for the same element
    /// through the session (an arbitrary counter, not the game's id); its
    /// `alt_right_click` is not recorded (false).
    Snapshot { t: u64, number: u32, targets: Vec<Target> },
    /// Raw mouse motion as the Alt mode gets it (before
    /// [`crate::snap::MOTION_GAIN`]).
    Motion { t: u64, d: Point },
    /// Where the Alt pointer is drawn and the target it rests on or heads
    /// for.
    Pointer { t: u64, kind: PointerKind, at: Point, snapshot: u32, target: Option<u32> },
    /// A mouse button in the mode, and the target snapped to then.
    Click { t: u64, button: Button, down: bool, used: ClickUse, snapshot: u32, target: Option<u32> },
    /// The mode on (the pointer at `at` before it snapped, the targets of
    /// `snapshot`) or off (`at` and `snapshot` 0).
    Mode { t: u64, change: ModeChange, reason: Reason, at: Point, snapshot: u32 },
    /// After the mode ended, the first press with the free pointer (at
    /// `at`) on a box of the last snapshot taken in the mode.
    GaveUp { t: u64, since_exit_us: u64, snapshot: u32, target: u32, at: Point, button: Button },
    /// Records the writer never got (its queue was full).
    Dropped { t: u64, count: u64 },
    /// The recording was closed (the viewer quit, or recording turned off).
    End { t: u64 },
}

impl Record {
    pub fn t(&self) -> u64 {
        match *self {
            Record::Rate { t, .. }
            | Record::Snapshot { t, .. }
            | Record::Motion { t, .. }
            | Record::Pointer { t, .. }
            | Record::Click { t, .. }
            | Record::Mode { t, .. }
            | Record::GaveUp { t, .. }
            | Record::Dropped { t, .. }
            | Record::End { t } => t,
        }
    }
}

/// Where a host records the Alt mode: `CIV3R_ALTREC_DIR` if set, else
/// `default` (the folder of the game's log, `civ3-bridge.log`).
pub fn recording_dir(default: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    std::env::var_os("CIV3R_ALTREC_DIR").filter(|d| !d.is_empty()).map(Into::into).or(default)
}

/// Seconds on the viewer's clock as a record's microseconds.
pub fn micros(now: f64) -> u64 {
    if now.is_finite() && now > 0.0 { (now * 1e6).round() as u64 } else { 0 }
}

/// A record's microseconds as seconds on the viewer's clock.
pub fn seconds(t: u64) -> f64 {
    t as f64 / 1e6
}

#[cfg(test)]
mod tests;
