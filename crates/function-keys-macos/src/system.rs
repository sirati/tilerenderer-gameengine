//! Whether the login session is the user's to work in: not asleep, its
//! screens not off, not locked, not switched away from (fast user
//! switching).

use std::cell::Cell;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidSleepNotification, NSWorkspaceScreensDidWakeNotification,
    NSWorkspaceSessionDidBecomeActiveNotification, NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSOperationQueue, NSString};

const ASLEEP: u8 = 1;
const SCREENS_OFF: u8 = 2;
const AWAY: u8 = 4;
const LOCKED: u8 = 8;

/// Observes the session; reports changes on the main thread. Dropping it
/// stops the reports.
pub struct SystemWatch {
    observers: Vec<(Retained<NSNotificationCenter>, Retained<ProtocolObject<dyn NSObjectProtocol>>)>,
}

impl SystemWatch {
    /// Calls `changed(available)` when the session stops being available to
    /// work in (sleep, screens off, screen locked, another user's session)
    /// and when it is again. Main thread only.
    pub fn new(changed: impl Fn(bool) + 'static) -> SystemWatch {
        let state = Rc::new(Cell::new(0u8));
        let changed: Rc<dyn Fn(bool)> = Rc::new(changed);
        let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
        let distributed: Retained<NSNotificationCenter> = Retained::into_super(NSDistributedNotificationCenter::defaultCenter());
        let locked = NSString::from_str("com.apple.screenIsLocked");
        let unlocked = NSString::from_str("com.apple.screenIsUnlocked");
        // SAFETY: AppKit's notification names, constants for the process.
        let workspace_names = unsafe {
            [
                (NSWorkspaceWillSleepNotification, ASLEEP, true),
                (NSWorkspaceDidWakeNotification, ASLEEP, false),
                (NSWorkspaceScreensDidSleepNotification, SCREENS_OFF, true),
                (NSWorkspaceScreensDidWakeNotification, SCREENS_OFF, false),
                (NSWorkspaceSessionDidResignActiveNotification, AWAY, true),
                (NSWorkspaceSessionDidBecomeActiveNotification, AWAY, false),
            ]
        };
        let queue = NSOperationQueue::mainQueue();
        let mut observers = Vec::new();
        let names = workspace_names.iter().map(|&(name, bit, set)| (&workspace, name, bit, set))
            .chain([(&distributed, &*locked, LOCKED, true), (&distributed, &*unlocked, LOCKED, false)]);
        for (center, name, bit, set) in names {
            let (state, changed) = (state.clone(), changed.clone());
            let block = RcBlock::new(move |_: NonNull<NSNotification>| {
                let before = state.get();
                let after = if set { before | bit } else { before & !bit };
                state.set(after);
                if (before == 0) != (after == 0) {
                    changed(after == 0);
                }
            });
            // SAFETY: no object filter; the main queue runs the block on
            // the main thread only, where its `Rc`s live.
            let token = unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, Some(&queue), &block) };
            observers.push((center.clone(), token));
        }
        SystemWatch { observers }
    }
}

impl Drop for SystemWatch {
    fn drop(&mut self) {
        for (center, token) in self.observers.drain(..) {
            // SAFETY: a token this center returned.
            unsafe { center.removeObserver(token.as_ref()) };
        }
    }
}
