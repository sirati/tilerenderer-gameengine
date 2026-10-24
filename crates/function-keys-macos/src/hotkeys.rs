//! The WindowServer's system shortcuts ("symbolic hot keys"), through the
//! private CoreGraphics calls `CGSIsSymbolicHotKeyEnabled`,
//! `CGSGetSymbolicHotKeyValue` and `CGSSetSymbolicHotKeyEnabled`.
//!
//! They are looked up at run time (`dlsym`), so a macOS without them makes
//! [`SymbolicHotKeys::load`] return `None` rather than the program fail to
//! load.
//!
//! Turning a shortcut off this way changes the login session's live table,
//! not the user's preferences (`com.apple.symbolichotkeys`): it holds until
//! it is turned on again or the user logs out, even past the end of the
//! process that turned it off. Whoever turns one off has to turn it on.
//! (`CGSSetGlobalHotKeyOperatingMode` instead belongs to the process's
//! connection and ends with it, but it takes every shortcut, Cmd+Tab with
//! them.)

use std::ffi::{CStr, c_char, c_int, c_void};

use crate::HotKey;

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

const RTLD_LAZY: c_int = 0x1;
const RTLD_LOCAL: c_int = 0x4;

// The out parameters are `unichar *` and `CGSModifierFlags *` (16 and 32
// bits); they are given 64-bit zeroed cells, so a wider write than the
// headers say still lands inside them, and the low bits are read.
type IsEnabled = unsafe extern "C" fn(u32) -> u8;
type SetEnabled = unsafe extern "C" fn(u32, bool) -> i32;
type GetValue = unsafe extern "C" fn(u32, *mut u64, *mut u64, *mut u64) -> i32;

/// The system shortcuts, read and switched one by one.
pub struct SymbolicHotKeys {
    is_enabled: IsEnabled,
    set_enabled: SetEnabled,
    get_value: GetValue,
}

impl SymbolicHotKeys {
    /// Shortcut numbers looked at: macOS's own go up to about 260.
    pub const IDS: std::ops::Range<u32> = 0..512;

    /// The calls, if this macOS has them.
    pub fn load() -> Option<SymbolicHotKeys> {
        let libraries: [&CStr; 2] = [
            c"/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics",
            c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight",
        ];
        for library in libraries {
            // SAFETY: a NUL-terminated path; a system framework already
            // loaded by AppKit (its initialisers have run).
            let handle = unsafe { dlopen(library.as_ptr(), RTLD_LAZY | RTLD_LOCAL) };
            if handle.is_null() {
                continue;
            }
            // SAFETY: a live handle and NUL-terminated names.
            let symbol = |name: &CStr| unsafe { dlsym(handle, name.as_ptr()) };
            let (is_enabled, set_enabled, get_value) =
                (symbol(c"CGSIsSymbolicHotKeyEnabled"), symbol(c"CGSSetSymbolicHotKeyEnabled"), symbol(c"CGSGetSymbolicHotKeyValue"));
            if is_enabled.is_null() || set_enabled.is_null() || get_value.is_null() {
                continue;
            }
            // SAFETY: these are the functions' C signatures (see the type
            // aliases); the library stays loaded (never `dlclose`d).
            return Some(unsafe {
                SymbolicHotKeys {
                    is_enabled: std::mem::transmute::<*mut c_void, IsEnabled>(is_enabled),
                    set_enabled: std::mem::transmute::<*mut c_void, SetEnabled>(set_enabled),
                    get_value: std::mem::transmute::<*mut c_void, GetValue>(get_value),
                }
            });
        }
        None
    }

    /// Whether shortcut `id` is on.
    pub fn is_enabled(&self, id: u32) -> bool {
        // SAFETY: takes a number, returns a flag; an unknown number is off.
        unsafe { (self.is_enabled)(id) != 0 }
    }

    /// Shortcut `id`'s key, if it has one.
    pub fn value(&self, id: u32) -> Option<HotKey> {
        let (mut character, mut key_code, mut modifiers) = (0u64, 0u64, 0u64);
        // SAFETY: three writable cells, each wider than what is written.
        let error = unsafe { (self.get_value)(id, &mut character, &mut key_code, &mut modifiers) };
        let key_code = key_code as u16;
        (error == 0 && key_code != 0xFFFF).then_some(HotKey { id, key_code, modifiers: modifiers as u32 })
    }

    /// The shortcuts that are on and whose key is one of `keys`.
    pub fn enabled_on(&self, keys: &[u16]) -> Vec<HotKey> {
        Self::IDS
            .filter(|&id| self.is_enabled(id))
            .filter_map(|id| self.value(id))
            .filter(|hotkey| keys.contains(&hotkey.key_code))
            .collect()
    }

    /// Turns shortcut `id` on or off; the CoreGraphics error otherwise.
    pub fn set_enabled(&self, id: u32, enabled: bool) -> Result<(), i32> {
        // SAFETY: takes a number and a flag.
        match unsafe { (self.set_enabled)(id, enabled) } {
            0 => Ok(()),
            error => Err(error),
        }
    }
}
