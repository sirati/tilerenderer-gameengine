//! Key codes: the function keys, the top-row keys that stand for them in
//! media mode, and the media events that do.

/// macOS virtual key codes of F1..F12 (`kVK_F1`..`kVK_F12`), in order.
pub const FUNCTION_KEYS: [u16; 12] = [0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F];

/// Top-row keys of Apple keyboards in media mode that arrive as key events
/// with codes of their own, and the function key each stands on.
pub const TOP_ROW_KEYS: [(u16, u8); 5] = [
    (0xA0, 3), // Mission Control
    (0x83, 4), // Launchpad (older keyboards)
    (0xB1, 4), // Spotlight
    (0xB0, 5), // Dictation
    (0xB2, 6), // Do Not Disturb
];

/// The virtual key code of F`n` (1..=12).
pub fn function_key(n: u8) -> Option<u16> {
    FUNCTION_KEYS.get(usize::from(n).checked_sub(1)?).copied()
}

/// F`n` for a top-row key code ([`TOP_ROW_KEYS`]).
pub fn top_row_as_function(key_code: u16) -> Option<u16> {
    TOP_ROW_KEYS.iter().find(|(code, _)| *code == key_code).and_then(|(_, n)| function_key(*n))
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
