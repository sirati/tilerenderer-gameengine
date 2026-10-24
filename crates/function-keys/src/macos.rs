//! The capture on macOS: the system's shortcuts and media keys
//! (`function_keys_macos`), the session's state, the journal file.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use function_keys_macos::{MediaKeys, SymbolicHotKeys, SystemWatch};

use crate::{Backend, Capture, FileJournal, HotKey, Want};

struct System {
    hotkeys: SymbolicHotKeys,
    media: MediaKeys,
}

impl Backend for System {
    fn enabled_hotkeys(&mut self, keys: &[u16]) -> Vec<HotKey> {
        self.hotkeys.enabled_on(keys)
    }

    fn set_hotkey_enabled(&mut self, id: u32, enabled: bool) -> bool {
        match self.hotkeys.set_enabled(id, enabled) {
            Ok(()) => true,
            Err(error) => {
                log::debug!("function keys: system shortcut {id} on={enabled}: CoreGraphics error {error}");
                false
            }
        }
    }

    fn set_media_keys(&mut self, on: bool) -> Result<(), String> {
        self.media.set(on).map_err(|e| e.to_string())
    }
}

fn system() -> Option<System> {
    let hotkeys = SymbolicHotKeys::load();
    if hotkeys.is_none() {
        log::warn!("function keys: this macOS has no CGS symbolic hot key calls; its shortcuts on F1-F12 stay the system's");
    }
    Some(System { hotkeys: hotkeys?, media: MediaKeys::new() })
}

struct State {
    capture: Option<Capture<System, FileJournal>>,
    want: Want,
    /// The session is the user's (`SystemWatch`).
    available: Rc<Cell<bool>>,
}

impl State {
    fn apply(&mut self) {
        let mode = self.want.mode(self.available.get());
        if let Some(capture) = &mut self.capture {
            capture.set(mode);
        }
    }
}

/// The viewer's capture. Main thread only.
pub struct Controller {
    state: Rc<RefCell<State>>,
    _watch: SystemWatch,
}

impl Controller {
    /// Off, with its journal at `journal` (what it holds is turned on
    /// again first).
    pub fn new(journal: PathBuf) -> Controller {
        let capture = system().map(|system| Capture::new(system, FileJournal(journal)));
        let available = Rc::new(Cell::new(true));
        let state = Rc::new(RefCell::new(State { capture, want: Want::default(), available: available.clone() }));
        let watched = Rc::downgrade(&state);
        let watch = SystemWatch::new(move |now| {
            log::info!("function keys: the session is {}", if now { "back" } else { "away (asleep, locked or switched)" });
            available.set(now);
            if let Some(state) = watched.upgrade() {
                // (Busy only if called back from inside `set`, which then
                // applies it itself the next time it is called.)
                if let Ok(mut state) = state.try_borrow_mut() {
                    state.apply();
                }
            }
        });
        Controller { state, _watch: watch }
    }

    /// Takes what the viewer wants now: cheap when nothing changed.
    pub fn set(&mut self, want: Want) {
        let mut state = self.state.borrow_mut();
        state.want = want;
        state.apply();
    }

    /// Off, everything restored (the viewer is ending).
    pub fn release(&mut self) {
        let mut state = self.state.borrow_mut();
        state.want = Want::default();
        if let Some(capture) = &mut state.capture {
            capture.release();
        }
    }
}

/// Turns on again the shortcuts the journal at `journal` holds (a run that
/// paused them ended before it turned them on); how many. The launcher
/// calls it when it starts and once the game is gone.
pub fn replay(journal: &Path) -> usize {
    let mut journal = FileJournal(journal.to_path_buf());
    if crate::Journal::read(&journal).is_empty() {
        return 0;
    }
    match system() {
        Some(mut system) => crate::recover(&mut system, &mut journal),
        None => 0,
    }
}
