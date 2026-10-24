//! What macOS does with F1-F12 before an application sees them, and the
//! switches that hand them to the application instead.
//!
//! A function key reaches the window in front (as a plain `keyDown:`) only
//! if nothing in the system takes it first. Two things do:
//!
//! * **System shortcuts** (the WindowServer's "symbolic hot keys", the ones
//!   in System Settings > Keyboard > Keyboard Shortcuts): F11 shows the
//!   desktop, Ctrl+F1..F8 move the keyboard focus, Cmd+F5 starts VoiceOver,
//!   and any other a user bound to an F key. They are matched before the
//!   event is routed to an application. [`SymbolicHotKeys`] lists them and
//!   turns single ones off and on again (the private CoreGraphics calls
//!   remote-desktop and app-switcher programs use); the change holds for
//!   the login session, whichever process made it, until it is undone.
//! * **The top row as media keys** (System Settings > Keyboard > "Use F1, F2,
//!   etc. keys as standard function keys" off, the default): the keys send
//!   brightness, volume and media events (`NX_SYSDEFINED`) or keys of their
//!   own (Mission Control, Spotlight, Dictation, Do Not Disturb), and only
//!   Fn with them gives F1-F12. [`MediaKeys`] is an event tap that turns
//!   them back into F1-F12 while it is on; it needs the Accessibility
//!   permission.
//!
//! [`SystemWatch`] reports when the session is away (sleep, screens off,
//! screen locked, another user's session in front), when nothing should be
//! held back from the system.
//!
//! [`keys`] holds the key tables and compiles everywhere (for tests).

#![deny(unsafe_op_in_unsafe_fn)]

pub mod keys;
#[cfg(target_os = "macos")]
mod hotkeys;
#[cfg(target_os = "macos")]
mod media;
#[cfg(target_os = "macos")]
mod system;

#[cfg(target_os = "macos")]
pub use {hotkeys::SymbolicHotKeys, media::{MediaKeys, MediaKeysError}, system::SystemWatch};

/// One system shortcut: its number and its key (a virtual key code and the
/// modifier flags, as `CGEventFlags`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HotKey {
    pub id: u32,
    pub key_code: u16,
    pub modifiers: u32,
}

impl std::fmt::Display for HotKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.id, keys::describe(self.key_code, self.modifiers))
    }
}
