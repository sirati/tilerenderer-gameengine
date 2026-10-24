//! The top-row media keys as F1-F12: an event tap where events enter the
//! window server from the HID system ([`keys::tap::HID`]) that, while it
//! is on, rewrites them before the system or any application sees them.
//!
//! * Brightness, keyboard light, media and volume keys arrive as
//!   system-defined events (`NX_SYSDEFINED`, subtype 8). Each is replaced
//!   with a key press or release of its function key
//!   ([`crate::keys::MediaKey::as_function`]); the system never sees it,
//!   so the volume or brightness does not change.
//! * Mission Control, Spotlight, Dictation, Do Not Disturb (and Launchpad)
//!   arrive as key events of their own codes; each is replaced with a key
//!   press or release of its function key
//!   ([`crate::keys::top_row_as_function`]). A tap in the session does not
//!   get these: on an Apple silicon MacBook they went past one to the
//!   window in front with their own codes, unchanged, so the tap sits at
//!   the HID level (in the session only if the system refuses that).
//!
//! Each top-row key the tap rewrites, and each special key it does not
//! know, is logged once (`function keys: tap: ...`).
//!
//! A tap that changes events needs the Accessibility permission (System
//! Settings > Privacy & Security > Accessibility), granted to the
//! application the process belongs to. Without it no tap is made
//! ([`MediaKeysError::NotTrusted`]); the system's own prompt is shown once
//! per process.
//!
//! The tap sees every application's events: it is only on while the game's
//! window is in front, and turned off (not merely ignored) otherwise. It
//! ends with the process.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};

use objc2_app_kit::NSEvent;
use objc2_core_graphics::CGEvent;

use crate::keys;
use crate::keys::tap::{self, Seen};

type CFTypeRef = *const c_void;
type CFMachPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CGEventRef = *mut c_void;
type TapCallback = extern "C" fn(*mut c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, events: u64, callback: TapCallback, info: *mut c_void) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventCreateKeyboardEvent(source: *mut c_void, key: u16, down: bool) -> CGEventRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(allocator: CFTypeRef, port: CFMachPortRef, order: isize) -> CFRunLoopSourceRef;
    fn CFMachPortInvalidate(port: CFMachPortRef);
    fn CFRunLoopGetMain() -> CFRunLoopRef;
    fn CFRunLoopAddSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFTypeRef);
    fn CFRunLoopRemoveSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFTypeRef);
    fn CFDictionaryCreate(allocator: CFTypeRef, keys: *const CFTypeRef, values: *const CFTypeRef, count: isize, key_callbacks: *const c_void, value_callbacks: *const c_void) -> CFTypeRef;
    fn CFRelease(object: CFTypeRef);
    static kCFRunLoopCommonModes: CFTypeRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> u8;
    static kAXTrustedCheckOptionPrompt: CFTypeRef;
}

const HEAD_INSERT_EVENT_TAP: u32 = 0;
const TAP_OPTION_DEFAULT: u32 = 0;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
const KEYBOARD_EVENT_AUTOREPEAT: u32 = 8;
const KEYBOARD_EVENT_KEYCODE: u32 = 9;

/// Why the tap is not there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaKeysError {
    /// The application lacks the Accessibility permission.
    NotTrusted,
    /// The system made no tap (or no run loop source for it).
    Refused,
}

impl std::fmt::Display for MediaKeysError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MediaKeysError::NotTrusted => "the application needs the Accessibility permission (System Settings > Privacy & Security > Accessibility)",
            MediaKeysError::Refused => "the system refused the event tap",
        })
    }
}

/// What the tap's callback reads: whether it is on, and the tap itself (to
/// turn it back on when the system turned it off for being slow).
struct Shared {
    on: AtomicBool,
    port: AtomicPtr<c_void>,
    /// Key codes (0..256) and media key types (256..512) already logged.
    logged: [AtomicU64; 8],
}

impl Shared {
    /// Whether `index` is logged for the first time now.
    fn first(&self, index: usize) -> bool {
        let Some(word) = self.logged.get(index / 64) else { return false };
        let bit = 1u64 << (index % 64);
        word.fetch_or(bit, Ordering::Relaxed) & bit == 0
    }
}

/// The tap, made the first time it is turned on. Main thread only.
pub struct MediaKeys {
    shared: Box<Shared>,
    port: CFMachPortRef,
    source: CFRunLoopSourceRef,
}

impl Default for MediaKeys {
    fn default() -> Self {
        MediaKeys::new()
    }
}

impl MediaKeys {
    pub fn new() -> MediaKeys {
        let shared = Shared { on: AtomicBool::new(false), port: AtomicPtr::new(std::ptr::null_mut()), logged: Default::default() };
        MediaKeys { shared: Box::new(shared), port: std::ptr::null_mut(), source: std::ptr::null_mut() }
    }

    /// Whether the tap is on.
    pub fn is_on(&self) -> bool {
        self.shared.on.load(Ordering::Relaxed)
    }

    /// Turns the tap on or off.
    pub fn set(&mut self, on: bool) -> Result<(), MediaKeysError> {
        if on && self.port.is_null() {
            self.create()?;
        }
        self.shared.on.store(on, Ordering::Relaxed);
        if !self.port.is_null() {
            // SAFETY: a live tap of ours.
            unsafe { CGEventTapEnable(self.port, on) };
        }
        Ok(())
    }

    fn create(&mut self) -> Result<(), MediaKeysError> {
        if !trusted() {
            return Err(MediaKeysError::NotTrusted);
        }
        let info = &*self.shared as *const Shared as *mut c_void;
        let port = tap::LOCATIONS.iter().find_map(|&location| {
            // SAFETY: `info` is the boxed `Shared`, which outlives the tap
            // (`Drop` invalidates the tap first); the callback has the tap
            // callback's signature.
            let port = unsafe { CGEventTapCreate(location, HEAD_INSERT_EVENT_TAP, TAP_OPTION_DEFAULT, tap::EVENTS, tap_callback, info) };
            let place = if location == tap::HID { "the HID level" } else { "the login session" };
            if port.is_null() {
                log::warn!("function keys: the system refused an event tap at {place}");
                None
            } else {
                log::info!("function keys: event tap at {place}");
                Some(port)
            }
        });
        let Some(port) = port else { return Err(MediaKeysError::Refused) };
        // SAFETY: a live tap; the default allocator.
        let source = unsafe { CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0) };
        if source.is_null() {
            // SAFETY: the tap was made above and is not used after.
            unsafe {
                CFMachPortInvalidate(port);
                CFRelease(port);
            }
            return Err(MediaKeysError::Refused);
        }
        // SAFETY: the main run loop and a live source; a CF constant.
        unsafe { CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes) };
        self.shared.port.store(port, Ordering::Relaxed);
        self.port = port;
        self.source = source;
        Ok(())
    }
}

impl Drop for MediaKeys {
    fn drop(&mut self) {
        self.shared.on.store(false, Ordering::Relaxed);
        if self.port.is_null() {
            return;
        }
        // SAFETY: the tap and source made in `create`; once invalidated the
        // tap calls back no more, so `shared` may go after.
        unsafe {
            CGEventTapEnable(self.port, false);
            CFRunLoopRemoveSource(CFRunLoopGetMain(), self.source, kCFRunLoopCommonModes);
            CFMachPortInvalidate(self.port);
            CFRelease(self.source);
            CFRelease(self.port);
        }
        self.shared.port.store(std::ptr::null_mut(), Ordering::Relaxed);
    }
}

/// Whether the process may tap events; the first time it may not, the
/// system is asked to prompt the user.
fn trusted() -> bool {
    static PROMPTED: AtomicBool = AtomicBool::new(false);
    // SAFETY: no arguments.
    if unsafe { AXIsProcessTrusted() } != 0 {
        return true;
    }
    if !PROMPTED.swap(true, Ordering::Relaxed) {
        // SAFETY: a one-entry dictionary of CF constants, released after
        // the call (which does not keep it).
        unsafe {
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(std::ptr::null(), keys.as_ptr(), values.as_ptr(), 1,
                &raw const kCFTypeDictionaryKeyCallBacks as *const c_void, &raw const kCFTypeDictionaryValueCallBacks as *const c_void);
            if !options.is_null() {
                let trusted = AXIsProcessTrustedWithOptions(options) != 0;
                CFRelease(options);
                return trusted;
            }
        }
    }
    false
}

/// The tap: rewrites the top row's keys while on, lets everything else by.
/// Runs on the main run loop; must not panic (it is called from C).
extern "C" fn tap_callback(_proxy: *mut c_void, kind: u32, event: CGEventRef, info: *mut c_void) -> CGEventRef {
    // SAFETY: `info` is the `Shared` the tap was made with (`create`).
    let shared = unsafe { &*(info as *const Shared) };
    let on = shared.on.load(Ordering::Relaxed);
    if kind == TAP_DISABLED_BY_TIMEOUT || kind == TAP_DISABLED_BY_USER_INPUT {
        let port = shared.port.load(Ordering::Relaxed);
        if on && !port.is_null() {
            // SAFETY: our live tap.
            unsafe { CGEventTapEnable(port, true) };
        }
        return event;
    }
    if !on || event.is_null() {
        return event;
    }
    let seen = match kind {
        tap::KEY_DOWN | tap::KEY_UP => {
            // SAFETY: a live keyboard event handed to the tap.
            let (code, repeat) = unsafe { (CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE), CGEventGetIntegerValueField(event, KEYBOARD_EVENT_AUTOREPEAT)) };
            match u16::try_from(code) {
                Ok(code) => Seen::Key { code, down: kind == tap::KEY_DOWN, repeat: repeat != 0 },
                Err(_) => Seen::Other,
            }
        }
        tap::SYSTEM_DEFINED => media_key(event).map_or(Seen::Other, Seen::Media),
        _ => Seen::Other,
    };
    let Some(replace) = tap::rewrite(seen) else {
        if let Some(what) = tap::unknown(seen) {
            if shared.first(log_index(seen)) {
                log::info!("function keys: tap: {what} is no top-row key it knows; passed on");
            }
        }
        return event;
    };
    if let Seen::Key { code, .. } = seen {
        if shared.first(log_index(seen)) {
            log::info!("function keys: tap: top-row key code {code:#04x} becomes {}", keys::describe(replace.key_code, 0));
        }
    }
    // SAFETY: no event source; a key code and a direction.
    let replacement = unsafe { CGEventCreateKeyboardEvent(std::ptr::null_mut(), replace.key_code, replace.down) };
    if replacement.is_null() {
        return event;
    }
    // SAFETY: two live events. The replacement is handed back to the
    // system, which releases it (with the original) once delivered.
    unsafe {
        CGEventSetFlags(replacement, CGEventGetFlags(event) & u64::from(keys::MODIFIERS));
        if replace.repeat {
            CGEventSetIntegerValueField(replacement, KEYBOARD_EVENT_AUTOREPEAT, 1);
        }
    }
    replacement
}

/// Where `seen` is noted as logged (`Shared::logged`).
fn log_index(seen: Seen) -> usize {
    match seen {
        Seen::Key { code, .. } => usize::from(code).min(255),
        Seen::Media(key) => 256 + (key.key as usize).min(255),
        Seen::Other => 511,
    }
}

/// The media key of a system-defined event, if it is one.
fn media_key(event: CGEventRef) -> Option<keys::MediaKey> {
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: a live CGEvent (a CF object of that type) for the call.
        let event = NSEvent::eventWithCGEvent(unsafe { &*(event as *const CGEvent) })?;
        if event.subtype().0 != keys::AUX_CONTROL_BUTTONS {
            return None;
        }
        keys::MediaKey::from_data1(event.data1())
    })
}
