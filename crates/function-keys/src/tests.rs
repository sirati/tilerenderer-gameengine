use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use super::*;
use keys::{COMMAND, CONTROL, FN, SHIFT, function_key};

/// The system as a table of shortcuts, shared with the test after the
/// capture took it.
#[derive(Default)]
struct World {
    hotkeys: BTreeMap<u32, (HotKey, bool)>,
    media: bool,
    media_refused: bool,
    refuse_restore: Vec<u32>,
    calls: Vec<(u32, bool)>,
    journal: Option<Vec<u32>>,
    journal_broken: bool,
}

#[derive(Clone, Default)]
struct Fake(Rc<RefCell<World>>);

impl Fake {
    fn macos() -> Fake {
        let fake = Fake::default();
        {
            let mut w = fake.0.borrow_mut();
            let mut add = |id: u32, key: u16, modifiers: u32, on: bool| {
                w.hotkeys.insert(id, (HotKey { id, key_code: key, modifiers }, on));
            };
            add(1, 48, COMMAND, true); // Cmd+Tab
            add(2, function_key(1).unwrap(), COMMAND | SHIFT, true); // (protected, even on an F key)
            add(7, function_key(2).unwrap(), CONTROL, true); // focus the menu bar
            add(32, 0xA0, 0, true); // Mission Control key
            add(36, function_key(11).unwrap(), FN, true); // Show Desktop
            add(37, function_key(11).unwrap(), FN | SHIFT, true);
            add(59, function_key(5).unwrap(), COMMAND, false); // VoiceOver, turned off by the user
            add(64, 49, COMMAND, true); // Spotlight
        }
        fake
    }

    fn on(&self, id: u32) -> bool {
        self.0.borrow().hotkeys[&id].1
    }

    fn off(&self) -> Vec<u32> {
        self.0.borrow().hotkeys.iter().filter(|(_, (_, on))| !on).map(|(id, _)| *id).collect()
    }

    fn journal(&self) -> Option<Vec<u32>> {
        self.0.borrow().journal.clone()
    }
}

impl Backend for Fake {
    fn enabled_hotkeys(&mut self, keys: &[u16]) -> Vec<HotKey> {
        self.0.borrow().hotkeys.values().filter(|(h, on)| *on && keys.contains(&h.key_code)).map(|(h, _)| *h).collect()
    }

    fn set_hotkey_enabled(&mut self, id: u32, enabled: bool) -> bool {
        let mut w = self.0.borrow_mut();
        w.calls.push((id, enabled));
        if enabled && w.refuse_restore.contains(&id) {
            return false;
        }
        match w.hotkeys.get_mut(&id) {
            Some((_, on)) => {
                *on = enabled;
                true
            }
            None => enabled,
        }
    }

    fn set_media_keys(&mut self, on: bool) -> Result<(), String> {
        let mut w = self.0.borrow_mut();
        if on && w.media_refused {
            return Err("no permission".into());
        }
        w.media = on;
        Ok(())
    }
}

impl Journal for Fake {
    fn read(&self) -> Vec<u32> {
        self.0.borrow().journal.clone().unwrap_or_default()
    }

    fn write(&mut self, ids: &[u32]) -> io::Result<()> {
        let mut w = self.0.borrow_mut();
        if w.journal_broken {
            return Err(io::Error::other("read-only"));
        }
        w.journal = Some(ids.to_vec());
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        self.0.borrow_mut().journal = None;
        Ok(())
    }
}

fn capture(fake: &Fake) -> Capture<Fake, Fake> {
    Capture::new(fake.clone(), fake.clone())
}

const PLAIN: Option<Mode> = Some(Mode { media_keys: false });
const MEDIA: Option<Mode> = Some(Mode { media_keys: true });

#[test]
fn focus_pauses_the_shortcuts_on_function_keys_and_written_down_first() {
    let fake = Fake::macos();
    let mut c = capture(&fake);
    assert!(fake.off() == vec![59] && fake.journal().is_none());
    c.set(PLAIN);
    // F2, F11 and Shift+F11; not Cmd+Tab, not the protected one on F1,
    // not Mission Control's own key (media mode off), not Spotlight.
    assert_eq!(fake.off(), vec![7, 36, 37, 59]);
    assert_eq!(fake.journal(), Some(vec![7, 36, 37]));
    assert!(fake.on(1) && fake.on(2) && fake.on(32) && fake.on(64));
    assert_eq!(c.active(), PLAIN);
    assert!(!fake.0.borrow().media);
}

#[test]
fn resigning_turns_on_exactly_what_was_paused() {
    let fake = Fake::macos();
    let mut c = capture(&fake);
    c.set(PLAIN);
    c.set(None);
    // The user's own VoiceOver choice stays off.
    assert_eq!(fake.off(), vec![59]);
    assert_eq!(fake.journal(), None);
    assert!(c.paused().is_empty() && c.active().is_none());
    assert!(!fake.0.borrow().calls.contains(&(59, true)));
}

#[test]
fn releasing_twice_or_setting_the_same_again_does_nothing_more() {
    let fake = Fake::macos();
    let mut c = capture(&fake);
    c.set(PLAIN);
    c.set(PLAIN);
    let after_on = fake.0.borrow().calls.len();
    assert_eq!(after_on, 3);
    c.set(None);
    c.release();
    c.set(None);
    assert_eq!(fake.0.borrow().calls.len(), after_on + 3);
    drop(c);
    assert_eq!(fake.0.borrow().calls.len(), after_on + 3);
    assert_eq!(fake.off(), vec![59]);
}

#[test]
fn dropping_the_capture_restores() {
    let fake = Fake::macos();
    let mut c = capture(&fake);
    c.set(MEDIA);
    drop(c);
    assert_eq!(fake.off(), vec![59]);
    assert!(!fake.0.borrow().media);
    assert_eq!(fake.journal(), None);
}

#[test]
fn a_crashed_run_is_restored_by_the_next_or_by_the_launcher() {
    let fake = Fake::macos();
    let mut c = capture(&fake);
    c.set(PLAIN);
    // The process dies: nothing runs.
    std::mem::forget(c);
    assert_eq!(fake.off(), vec![7, 36, 37, 59]);
    // The launcher's recovery (or the next run's capture).
    assert_eq!(recover(&mut fake.clone(), &mut fake.clone()), 3);
    assert_eq!(fake.off(), vec![59]);
    assert_eq!(fake.journal(), None);
    // Nothing more to do the next time.
    assert_eq!(recover(&mut fake.clone(), &mut fake.clone()), 0);

    let mut c = capture(&fake);
    c.set(PLAIN);
    std::mem::forget(c);
    let c = capture(&fake);
    assert_eq!(fake.off(), vec![59]);
    assert!(c.active().is_none());
}

#[test]
fn no_journal_no_pausing() {
    let fake = Fake::macos();
    fake.0.borrow_mut().journal_broken = true;
    let mut c = capture(&fake);
    c.set(PLAIN);
    assert_eq!(fake.off(), vec![59]);
    assert!(c.paused().is_empty());
    c.set(None);
    assert!(fake.0.borrow().calls.is_empty());
}

#[test]
fn a_shortcut_that_would_not_come_back_stays_written_down() {
    let fake = Fake::macos();
    fake.0.borrow_mut().refuse_restore = vec![36];
    let mut c = capture(&fake);
    c.set(PLAIN);
    c.set(None);
    assert_eq!(fake.off(), vec![36, 59]);
    assert_eq!(fake.journal(), Some(vec![36]));
    // Tried again on the next release, and by the next run.
    c.set(PLAIN);
    assert_eq!(fake.journal(), Some(vec![7, 36, 37]));
    fake.0.borrow_mut().refuse_restore.clear();
    c.set(None);
    assert_eq!(fake.off(), vec![59]);
    assert_eq!(fake.journal(), None);
}

#[test]
fn media_mode_takes_the_top_row_keys_and_the_tap() {
    let fake = Fake::macos();
    let mut c = capture(&fake);
    c.set(MEDIA);
    assert_eq!(fake.off(), vec![7, 32, 36, 37, 59]);
    assert!(fake.0.borrow().media);
    // Changing the mode lets go first, then takes the new one.
    c.set(PLAIN);
    assert_eq!(fake.off(), vec![7, 36, 37, 59]);
    assert!(!fake.0.borrow().media);
    c.set(None);
    assert_eq!(fake.off(), vec![59]);
}

#[test]
fn a_refused_tap_still_pauses_the_shortcuts() {
    let fake = Fake::macos();
    fake.0.borrow_mut().media_refused = true;
    let mut c = capture(&fake);
    c.set(MEDIA);
    assert_eq!(fake.off(), vec![7, 32, 36, 37, 59]);
    assert!(!fake.0.borrow().media);
    c.set(None);
    assert_eq!(fake.off(), vec![59]);
}

#[test]
fn wanting_needs_the_setting_the_window_and_the_session() {
    let on = Want { capture: true, media_keys: false, window_active: true };
    assert_eq!(on.mode(true), PLAIN);
    assert_eq!(on.mode(false), None);
    assert_eq!(Want { window_active: false, ..on }.mode(true), None);
    assert_eq!(Want { capture: false, ..on }.mode(true), None);
    assert_eq!(Want { media_keys: true, ..on }.mode(true), MEDIA);
    // Focus gained, lost (Cmd+Tab), gained, the setting turned off.
    let fake = Fake::macos();
    let mut c = capture(&fake);
    for (want, paused) in [(on, true), (Want { window_active: false, ..on }, false), (on, true), (Want { capture: false, ..on }, false)] {
        c.set(want.mode(true));
        assert_eq!(fake.off().len() > 1, paused, "{want:?}");
        assert!(fake.on(1));
    }
}

#[test]
fn file_journal_round_trip() {
    let dir = std::env::temp_dir().join(format!("function-keys-test-{}", std::process::id()));
    let path = journal_path(&dir.join("settings.toml"));
    assert_eq!(path, dir.join("paused-system-shortcuts.txt"));
    let mut journal = FileJournal(path.clone());
    assert!(journal.read().is_empty());
    journal.clear().unwrap();
    journal.write(&[37, 36, 7]).unwrap();
    assert_eq!(journal.read(), vec![7, 36, 37]);
    journal.write(&[36]).unwrap();
    assert_eq!(journal.read(), vec![36]);
    journal.clear().unwrap();
    assert!(!path.exists());
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
fn media_mode_pauses_the_shortcuts_on_mission_control_spotlight_dictation_and_do_not_disturb() {
    // As on an Apple silicon MacBook (macOS 26): the system shortcuts bound
    // to the top row's own key codes, with the Fn flag the system gives them.
    let fake = Fake::macos();
    {
        let mut w = fake.0.borrow_mut();
        let mut add = |id: u32, key: u16, modifiers: u32| {
            w.hotkeys.insert(id, (HotKey { id, key_code: key, modifiers }, true));
        };
        add(108, 0xA0, FN); // Mission Control
        add(110, 0xA0, FN | COMMAND);
        add(173, 0x83, FN); // Launchpad
        add(186, 0xB0, FN); // Dictation
        add(187, 0xB1, FN); // Spotlight
        add(188, 0xCF, FN); // Dictation (the usage's code)
        add(189, 0xB2, FN); // Do Not Disturb
        add(190, 0xB3, FN); // a code the tables do not know: left alone
    }
    let mut c = capture(&fake);
    c.set(PLAIN);
    assert_eq!(fake.off(), vec![7, 36, 37, 59]);
    c.set(MEDIA);
    assert_eq!(fake.off(), vec![7, 32, 36, 37, 59, 108, 110, 173, 186, 187, 188, 189]);
    assert_eq!(fake.journal(), Some(vec![7, 32, 36, 37, 108, 110, 173, 186, 187, 188, 189]));
    // Cmd+Tab and Spotlight's Cmd+Space stay on.
    assert!(fake.on(1) && fake.on(64) && fake.on(190));
    c.set(None);
    assert_eq!(fake.off(), vec![59]);
    assert_eq!(fake.journal(), None);
}
