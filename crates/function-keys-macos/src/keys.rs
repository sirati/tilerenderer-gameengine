//! Key codes: the function keys, the top-row keys that stand for them in
//! media mode, and the media events that do.

/// macOS virtual key codes of F1..F12 (`kVK_F1`..`kVK_F12`), in order.
pub const FUNCTION_KEYS: [u16; 12] = [0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F];

/// Top-row keys of Apple keyboards in media mode that arrive as key events
/// with codes of their own, and the function key each stands on.
///
/// The keyboard sends HID usages for them (on Apple silicon laptops: F3
/// Apple vendor 0xFF01:0x10, F4 consumer 0x221 AC Search, F5 consumer 0xCF
/// Voice Command, F6 generic desktop 0x9B Do Not Disturb: the keyboard's
/// `FnFunctionUsageMap` in `ioreg`), which the HID system turns into these
/// virtual key codes; the system shortcuts for Mission Control, Spotlight,
/// Dictation and Do Not Disturb are bound to the codes.
pub const TOP_ROW_KEYS: [(u16, u8); 7] = [
    (0xA0, 3), // Mission Control
    (0x83, 4), // Launchpad (older keyboards)
    (0x81, 4), // Spotlight (some keyboards)
    (0xB1, 4), // Spotlight
    (0xB0, 5), // Dictation
    (0xCF, 5), // Dictation (as its HID usage, some keyboards)
    (0xB2, 6), // Do Not Disturb
];

/// The virtual key code of F`n` (1..=12).
pub fn function_key(n: u8) -> Option<u16> {
    FUNCTION_KEYS.get(usize::from(n).checked_sub(1)?).copied()
}

/// `n` of the F`n` a top-row key code ([`TOP_ROW_KEYS`]) stands on.
pub fn top_row_number(key_code: u16) -> Option<u8> {
    TOP_ROW_KEYS.iter().find(|(code, _)| *code == key_code).map(|(_, n)| *n)
}

/// F`n` for a top-row key code ([`TOP_ROW_KEYS`]).
pub fn top_row_as_function(key_code: u16) -> Option<u16> {
    top_row_number(key_code).and_then(function_key)
}

/// The `NX_KEYTYPE_*` media keys of the top row and the function key each
/// sits on, on Apple's laptop and Magic keyboards (older keyboards: the
/// keyboard backlight on F5 and F6; next and previous are sent by some as
/// fast forward and rewind).
const MEDIA_KEYS: [(u32, u8); 12] = [
    (3, 1),   // NX_KEYTYPE_BRIGHTNESS_DOWN
    (2, 2),   // NX_KEYTYPE_BRIGHTNESS_UP
    (22, 5),  // NX_KEYTYPE_ILLUMINATION_DOWN
    (21, 6),  // NX_KEYTYPE_ILLUMINATION_UP
    (20, 7),  // NX_KEYTYPE_REWIND
    (18, 7),  // NX_KEYTYPE_PREVIOUS
    (16, 8),  // NX_KEYTYPE_PLAY
    (19, 9),  // NX_KEYTYPE_FAST
    (17, 9),  // NX_KEYTYPE_NEXT
    (7, 10),  // NX_KEYTYPE_MUTE
    (1, 11),  // NX_KEYTYPE_SOUND_DOWN
    (0, 12),  // NX_KEYTYPE_SOUND_UP
];

/// The `NSEvent` subtype of media key events (`NX_SUBTYPE_AUX_CONTROL_BUTTONS`).
pub const AUX_CONTROL_BUTTONS: i16 = 8;

/// A media key press or release, from a system-defined event's `data1`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MediaKey {
    /// The `NX_KEYTYPE_*`.
    pub key: u32,
    pub down: bool,
    pub repeat: bool,
}

impl MediaKey {
    /// From `data1` of an [`AUX_CONTROL_BUTTONS`] event: the key type in
    /// bits 16..32, the state (0xA down, 0xB up) in 8..16, repeat in bit 0.
    pub fn from_data1(data1: isize) -> Option<MediaKey> {
        let data1 = data1 as u64;
        let key = ((data1 >> 16) & 0xFFFF) as u32;
        let flags = data1 & 0xFFFF;
        let down = match (flags >> 8) & 0xFF {
            0xA => true,
            0xB => false,
            _ => return None,
        };
        Some(MediaKey { key, down, repeat: flags & 1 != 0 })
    }

    /// The function key it stands on.
    pub fn as_function(&self) -> Option<u16> {
        MEDIA_KEYS.iter().find(|(key, _)| *key == self.key).and_then(|(_, n)| function_key(*n))
    }
}

/// Modifier flags (`CGEventFlags`).
pub const SHIFT: u32 = 0x2_0000;
pub const CONTROL: u32 = 0x4_0000;
pub const OPTION: u32 = 0x8_0000;
pub const COMMAND: u32 = 0x10_0000;
pub const FN: u32 = 0x80_0000;
/// Shift, Control, Option and Command.
pub const MODIFIERS: u32 = SHIFT | CONTROL | OPTION | COMMAND;

/// The event tap: where it sits, what it listens to, what it does.
pub mod tap {
    use super::{MediaKey, top_row_as_function};

    /// `kCGHIDEventTap`: where events enter the window server from the HID
    /// system, before the system shortcuts and before anything else in the
    /// session. The top row's Mission Control, Spotlight, Dictation and Do
    /// Not Disturb keys are taken here: by the time they enter the session
    /// they are past a tap there.
    pub const HID: u32 = 0;
    /// `kCGSessionEventTap`: where events enter the login session.
    pub const SESSION: u32 = 1;
    /// The places tried, in order: the session only if the system refuses
    /// a tap at the HID level.
    pub const LOCATIONS: [u32; 2] = [HID, SESSION];

    /// `kCGEventKeyDown`, `kCGEventKeyUp`, `NX_SYSDEFINED`.
    pub const KEY_DOWN: u32 = 10;
    pub const KEY_UP: u32 = 11;
    pub const SYSTEM_DEFINED: u32 = 14;
    /// The events the tap listens to.
    pub const EVENTS: u64 = (1 << KEY_DOWN) | (1 << KEY_UP) | (1 << SYSTEM_DEFINED);

    /// What arrived at the tap.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum Seen {
        /// A key press or release, with its virtual key code.
        Key { code: u16, down: bool, repeat: bool },
        /// A media key (a system-defined event of subtype 8).
        Media(MediaKey),
        /// Anything else.
        Other,
    }

    /// A function key press or release, handed on instead.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub struct Replace {
        pub key_code: u16,
        pub down: bool,
        pub repeat: bool,
    }

    /// What the tap does with `seen`: a function key instead, or nothing
    /// (it goes on as it is).
    pub fn rewrite(seen: Seen) -> Option<Replace> {
        match seen {
            Seen::Key { code, down, repeat } => top_row_as_function(code).map(|key_code| Replace { key_code, down, repeat }),
            Seen::Media(key) => key.as_function().map(|key_code| Replace { key_code, down: key.down, repeat: key.repeat }),
            Seen::Other => None,
        }
    }

    /// `seen` in words if it looks like a top-row key the tables do not
    /// know (a key code past the ordinary ones, or a media key on no
    /// function key), for the log.
    pub fn unknown(seen: Seen) -> Option<String> {
        match seen {
            Seen::Key { code, .. } if code >= 0x80 && top_row_as_function(code).is_none() => Some(format!("key code {code:#04x}")),
            Seen::Media(key) if key.as_function().is_none() => Some(format!("media key NX_KEYTYPE {}", key.key)),
            _ => None,
        }
    }
}

/// A key with its modifiers in words, for logs (`Ctrl+F2`, `F11`).
pub fn describe(key_code: u16, modifiers: u32) -> String {
    let mut out = String::new();
    for (flag, name) in [(CONTROL, "Ctrl+"), (OPTION, "Option+"), (SHIFT, "Shift+"), (COMMAND, "Cmd+"), (FN, "Fn+")] {
        if modifiers & flag != 0 {
            out.push_str(name);
        }
    }
    match FUNCTION_KEYS.iter().position(|&k| k == key_code) {
        Some(i) => out.push_str(&format!("F{}", i + 1)),
        None => match TOP_ROW_KEYS.iter().find(|(k, _)| *k == key_code) {
            Some((_, n)) => out.push_str(&format!("top-row key {key_code} on F{n}")),
            None => out.push_str(&format!("key {key_code}")),
        },
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_keys_by_number() {
        assert_eq!(function_key(1), Some(0x7A));
        assert_eq!(function_key(11), Some(0x67));
        assert_eq!(function_key(12), Some(0x6F));
        assert_eq!(function_key(0), None);
        assert_eq!(function_key(13), None);
    }

    #[test]
    fn top_row_keys_stand_on_their_function_keys() {
        assert_eq!(top_row_as_function(0xA0), function_key(3));
        assert_eq!(top_row_as_function(0xB1), function_key(4));
        assert_eq!(top_row_as_function(0xB2), function_key(6));
        assert_eq!(top_row_as_function(0x67), None);
    }

    #[test]
    fn apple_silicon_top_row_f3_to_f6() {
        // Mission Control, Spotlight (and Launchpad), Dictation (both
        // codes), Do Not Disturb.
        for (code, n) in [(0xA0, 3), (0xB1, 4), (0x81, 4), (0x83, 4), (0xB0, 5), (0xCF, 5), (0xB2, 6)] {
            assert_eq!(top_row_number(code), Some(n), "{code:#x}");
            assert_eq!(top_row_as_function(code), function_key(n), "{code:#x}");
        }
        // Each code once, none an ordinary key code.
        let mut codes: Vec<u16> = TOP_ROW_KEYS.iter().map(|(c, _)| *c).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), TOP_ROW_KEYS.len());
        assert!(codes.iter().all(|&c| c >= 0x80));
    }

    #[test]
    fn the_tap_sits_at_the_hid_level_and_hears_keys_and_media_keys() {
        assert_eq!(tap::LOCATIONS, [tap::HID, tap::SESSION]);
        assert_eq!((tap::HID, tap::SESSION), (0, 1));
        assert_eq!(tap::EVENTS, (1 << 10) | (1 << 11) | (1 << 14));
        // Neither flags-changed (12) nor mouse events.
        assert_eq!(tap::EVENTS & ((1 << 12) | 0b11_1111), 0);
    }

    #[test]
    fn the_tap_rewrites_the_top_row_and_lets_the_rest_by() {
        use tap::{Replace, Seen, rewrite, unknown};
        let key = |code, down, repeat| Seen::Key { code, down, repeat };
        assert_eq!(rewrite(key(0xA0, true, false)), Some(Replace { key_code: 0x63, down: true, repeat: false }));
        assert_eq!(rewrite(key(0xB1, false, false)), Some(Replace { key_code: 0x76, down: false, repeat: false }));
        assert_eq!(rewrite(key(0xB0, true, true)), Some(Replace { key_code: 0x60, down: true, repeat: true }));
        assert_eq!(rewrite(key(0xCF, true, false)).map(|r| r.key_code), Some(0x60));
        assert_eq!(rewrite(key(0xB2, true, false)).map(|r| r.key_code), Some(0x61));
        // Letters, Tab (Cmd+Tab) and the F keys themselves go by.
        for code in [0x00, 0x30, 0x63, 0x7A] {
            assert_eq!(rewrite(key(code, true, false)), None);
        }
        let volume_down = MediaKey::from_data1((1 << 16) | 0x0A00).unwrap();
        assert_eq!(rewrite(Seen::Media(volume_down)), Some(Replace { key_code: 0x67, down: true, repeat: false }));
        assert_eq!(rewrite(Seen::Other), None);
        // Special keys the tables lack are worth a line in the log; known
        // and ordinary ones are not.
        assert_eq!(unknown(key(0xB3, true, false)).as_deref(), Some("key code 0xb3"));
        assert_eq!(unknown(key(0xA0, true, false)), None);
        assert_eq!(unknown(key(0x30, true, false)), None);
        let eject = MediaKey::from_data1((14 << 16) | 0x0A00).unwrap();
        assert_eq!(unknown(Seen::Media(eject)).as_deref(), Some("media key NX_KEYTYPE 14"));
        assert_eq!(unknown(Seen::Media(volume_down)), None);
    }

    #[test]
    fn media_events_decode_to_their_function_keys() {
        // Volume down pressed (0xA), then released (0xB), then a repeat.
        let down = MediaKey::from_data1((1 << 16) | 0x0A00).unwrap();
        assert_eq!(down, MediaKey { key: 1, down: true, repeat: false });
        assert_eq!(down.as_function(), function_key(11));
        let up = MediaKey::from_data1((1 << 16) | 0x0B00).unwrap();
        assert!(!up.down);
        assert!(MediaKey::from_data1((0 << 16) | 0x0A01).unwrap().repeat);
        assert_eq!(MediaKey::from_data1(0x0A00).unwrap().as_function(), function_key(12));
        assert_eq!(MediaKey::from_data1((3 << 16) | 0x0A00).unwrap().as_function(), function_key(1));
        assert_eq!(MediaKey::from_data1((16 << 16) | 0x0A00).unwrap().as_function(), function_key(8));
        // Neither down nor up; a key with no place on the top row.
        assert_eq!(MediaKey::from_data1((1 << 16) | 0x0C00), None);
        assert_eq!(MediaKey::from_data1((14 << 16) | 0x0A00).unwrap().as_function(), None);
    }

    #[test]
    fn keys_in_words() {
        assert_eq!(describe(0x67, FN), "Fn+F11");
        assert_eq!(describe(0x78, CONTROL), "Ctrl+F2");
        assert_eq!(describe(0x60, OPTION | COMMAND), "Option+Cmd+F5");
        assert_eq!(describe(0xA0, 0), "top-row key 160 on F3");
        assert_eq!(describe(48, COMMAND), "Cmd+key 48");
    }
}
