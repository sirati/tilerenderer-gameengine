//! F1-F12 for the game while its window is in front.
//!
//! On macOS the system takes function keys before the window sees them
//! (`function_keys_macos` tells how): F11 shows the desktop, Ctrl+F1..F8
//! move the keyboard focus, and in the default media mode the top row
//! changes brightness and volume. While the game's window is the key window
//! (and the session is the user's: not asleep, locked or switched away
//! from), [`Controller`] pauses the system shortcuts bound to F1-F12, with
//! any modifiers, and on request turns the top row's media keys into
//! F1-F12. Everything else, Cmd+Tab first of all, stays the system's.
//!
//! Restoring is the point: a shortcut paused stays paused for the whole
//! login session, past the end of the process, until it is turned on again.
//! So
//! * before any is paused, their numbers are written to a journal file
//!   ([`FileJournal`], beside the settings: [`journal_path`]); it is
//!   removed once they are all on again;
//! * they are turned on again when the window stops being the key window
//!   (another application, Cmd+Tab, a click elsewhere), is hidden
//!   (minimised, another Space), the session goes away, the setting is
//!   turned off, the viewer ends, or [`Controller`] is dropped;
//! * a journal left behind by a process that could not (a crash, a kill) is
//!   replayed by the next [`Controller`] and by [`Recovery`], which the
//!   launcher holds around the game's process: it restores them as soon as
//!   the game is gone, however it ended.
//!
//! Only shortcuts that were on are paused, so only those are turned on
//! again. The state machine is [`Capture`], over a [`Backend`] (the system)
//! and a [`Journal`]; elsewhere than macOS [`Controller`] and [`Recovery`]
//! do nothing.

#![forbid(unsafe_code)]

use std::io;
use std::path::{Path, PathBuf};

pub use function_keys_macos::{HotKey, keys};

/// Cmd+Tab, Cmd+Shift+Tab and Cmd+` (the shortcuts that switch
/// applications and windows): never paused, whatever key they are on.
pub const PROTECTED: [u32; 3] = [1, 2, 6];

/// What the capture takes besides the system shortcuts on F1-F12.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Mode {
    /// The top row's media keys (brightness, volume, Mission Control, ...)
    /// act as F1-F12, as if Fn were held.
    pub media_keys: bool,
}

/// The system's switches.
pub trait Backend {
    /// The system shortcuts that are on and whose key code is one of `keys`.
    fn enabled_hotkeys(&mut self, keys: &[u16]) -> Vec<HotKey>;
    /// Turns shortcut `id` on or off; whether that worked.
    fn set_hotkey_enabled(&mut self, id: u32, enabled: bool) -> bool;
    /// Turns the media keys' translation on or off.
    fn set_media_keys(&mut self, on: bool) -> Result<(), String>;
}

/// Where the paused shortcuts are written down until they are on again.
pub trait Journal {
    /// The shortcuts written down (none if there is no journal).
    fn read(&self) -> Vec<u32>;
    /// Writes down exactly `ids`, durably, before they are paused.
    fn write(&mut self, ids: &[u32]) -> io::Result<()>;
    /// No shortcut is paused any more.
    fn clear(&mut self) -> io::Result<()>;
}

/// The journal file beside the settings at `settings`.
pub fn journal_path(settings: &Path) -> PathBuf {
    settings.with_file_name("paused-system-shortcuts.txt")
}

/// A journal in a file: one shortcut number per line.
#[derive(Clone, Debug)]
pub struct FileJournal(pub PathBuf);

impl Journal for FileJournal {
    fn read(&self) -> Vec<u32> {
        let text = std::fs::read_to_string(&self.0).unwrap_or_default();
        let mut ids: Vec<u32> = text.lines().filter_map(|line| line.trim().parse().ok()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn write(&mut self, ids: &[u32]) -> io::Result<()> {
        use std::io::Write;
        if let Some(dir) = self.0.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let mut name = self.0.file_name().unwrap_or_default().to_owned();
        name.push(".new");
        let new = self.0.with_file_name(name);
        let mut file = std::fs::File::create(&new)?;
        file.write_all(ids.iter().map(|id| format!("{id}\n")).collect::<String>().as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&new, &self.0)
    }

    fn clear(&mut self) -> io::Result<()> {
        match std::fs::remove_file(&self.0) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

fn listed(hotkeys: &[HotKey]) -> String {
    if hotkeys.is_empty() {
        return "none".into();
    }
    hotkeys.iter().map(HotKey::to_string).collect::<Vec<_>>().join(", ")
}

/// Turns on again the shortcuts `journal` holds (left by a process that
/// paused them and ended before it turned them on); how many.
pub fn recover(backend: &mut impl Backend, journal: &mut impl Journal) -> usize {
    let ids = journal.read();
    if ids.is_empty() {
        return 0;
    }
    let failed: Vec<u32> = ids.iter().copied().filter(|&id| !backend.set_hotkey_enabled(id, true)).collect();
    let result = if failed.is_empty() { journal.clear() } else { journal.write(&failed) };
    if let Err(e) = result {
        log::warn!("function keys: journal of paused system shortcuts: {e}");
    }
    log::warn!("function keys: turned on again {} system shortcut(s) an earlier run left paused: {ids:?}{}", ids.len() - failed.len(),
        if failed.is_empty() { String::new() } else { format!("; could not turn on {failed:?}, kept for the next try") });
    ids.len() - failed.len()
}

/// The capture: on with a [`Mode`] or off; see the crate's documentation.
pub struct Capture<B: Backend, J: Journal> {
    backend: B,
    journal: J,
    active: Option<Mode>,
    /// The shortcuts this capture paused and has not turned on again.
    paused: Vec<HotKey>,
    media: bool,
}

impl<B: Backend, J: Journal> Capture<B, J> {
    /// Off; what an earlier run left paused turned on again first.
    pub fn new(mut backend: B, mut journal: J) -> Self {
        recover(&mut backend, &mut journal);
        Capture { backend, journal, active: None, paused: Vec::new(), media: false }
    }

    pub fn active(&self) -> Option<Mode> {
        self.active
    }

    /// The shortcuts paused now.
    pub fn paused(&self) -> &[HotKey] {
        &self.paused
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn journal(&self) -> &J {
        &self.journal
    }

    /// On in `mode`, or off (`None`). Nothing happens if it already is.
    pub fn set(&mut self, mode: Option<Mode>) {
        if mode == self.active {
            return;
        }
        self.release();
        if let Some(mode) = mode {
            self.engage(mode);
        }
    }

    fn engage(&mut self, mode: Mode) {
        let mut keys = keys::FUNCTION_KEYS.to_vec();
        if mode.media_keys {
            keys.extend(keys::TOP_ROW_KEYS.iter().map(|(code, _)| *code));
        }
        let mut found: Vec<HotKey> = self.backend.enabled_hotkeys(&keys).into_iter()
            .filter(|h| !PROTECTED.contains(&h.id) && keys.contains(&h.key_code) && !self.paused.iter().any(|p| p.id == h.id))
            .collect();
        // Written down first: a crash between pausing and writing would
        // leave them paused for good.
        let mut journal: Vec<u32> = self.paused.iter().chain(&found).map(|h| h.id).collect();
        journal.sort_unstable();
        journal.dedup();
        if !found.is_empty() {
            if let Err(e) = self.journal.write(&journal) {
                log::warn!("function keys: cannot write down the system shortcuts to pause ({e}); they stay the system's");
                found.clear();
            }
        }
        let mut refused = Vec::new();
        for hotkey in found {
            if self.backend.set_hotkey_enabled(hotkey.id, false) {
                self.paused.push(hotkey);
            } else {
                refused.push(hotkey);
            }
        }
        if !refused.is_empty() {
            log::warn!("function keys: the system did not pause {}", listed(&refused));
        }
        if mode.media_keys {
            match self.backend.set_media_keys(true) {
                Ok(()) => self.media = true,
                Err(e) => log::warn!("function keys: the top row's media keys stay media keys: {e}"),
            }
        }
        log::info!("function keys: F1-F12 go to the game; system shortcuts paused: {}{}", listed(&self.paused),
            if self.media { "; the top row's media keys act as F1-F12" } else { "" });
        self.active = Some(mode);
    }

    /// Off: every shortcut it paused on again. Safe to call any number of
    /// times.
    pub fn release(&mut self) {
        if self.active.is_none() && self.paused.is_empty() && !self.media {
            return;
        }
        if self.media {
            if let Err(e) = self.backend.set_media_keys(false) {
                log::warn!("function keys: media keys: {e}");
            }
            self.media = false;
        }
        let paused = std::mem::take(&mut self.paused);
        let (restored, failed): (Vec<HotKey>, Vec<HotKey>) = paused.into_iter().partition(|h| self.backend.set_hotkey_enabled(h.id, true));
        let result = if failed.is_empty() { self.journal.clear() } else { self.journal.write(&failed.iter().map(|h| h.id).collect::<Vec<_>>()) };
        if let Err(e) = result {
            log::warn!("function keys: journal of paused system shortcuts: {e}");
        }
        if failed.is_empty() {
            log::info!("function keys: F1-F12 are the system's again; system shortcuts turned on: {}", listed(&restored));
        } else {
            log::warn!("function keys: F1-F12 are the system's again; turned on {}, could not turn on {} (kept for the next try)", listed(&restored), listed(&failed));
        }
        self.paused = failed;
        self.active = None;
    }
}

impl<B: Backend, J: Journal> Drop for Capture<B, J> {
    fn drop(&mut self) {
        self.release();
    }
}

/// What the viewer wants.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Want {
    /// The setting: function keys go to the game while it is in front.
    pub capture: bool,
    /// The setting: the top row's media keys act as F1-F12 as well.
    pub media_keys: bool,
    /// The game's window is the key window and visible.
    pub window_active: bool,
}

impl Want {
    /// The capture's mode for this, while the session is `available`.
    pub fn mode(&self, available: bool) -> Option<Mode> {
        (self.capture && self.window_active && available).then_some(Mode { media_keys: self.media_keys })
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Controller, Recovery};

#[cfg(not(target_os = "macos"))]
mod elsewhere {
    use super::Want;
    use std::path::PathBuf;

    /// Nothing to capture here: the system keeps no function keys.
    pub struct Controller;

    impl Controller {
        pub fn new(_journal: PathBuf) -> Controller {
            Controller
        }
        pub fn set(&mut self, _want: Want) {}
        pub fn release(&mut self) {}
    }

    /// Nothing to restore here.
    pub struct Recovery;

    impl Recovery {
        pub fn new(_journal: PathBuf) -> Recovery {
            Recovery
        }
    }
}
#[cfg(not(target_os = "macos"))]
pub use elsewhere::{Controller, Recovery};

#[cfg(test)]
mod tests;
