//! Shared cadence and geometry of the loading (busy) cursor: the wait
//! cursor's hourglass (`IDC_WAIT`, see `civ3_assets::syscursor`), turned
//! in discrete steps. The art has a single still frame, so the loader
//! pre-renders [`FRAME_COUNT`] rotations of it (`rotations` on its image,
//! `civ3_assets::rotate`), each [`STEP_DEGREES`] further clockwise.
use std::time::Duration;

/// The rotated frames, in order (frame `k`: `k` × [`STEP_DEGREES`]).
pub const SET: &str = "Viewer@loading_cursor/0";
pub const FRAME_MS: u64 = 83;
/// Steps a full turn takes.
pub const FRAME_COUNT: u32 = 12;
pub const STEP_DEGREES: u32 = 360 / FRAME_COUNT;
/// Viewer-clock flag and the step period, interpreted by the GPU only.
pub const TICKS_PER_FRAME: u32 = (1 << 31) | FRAME_MS as u32;

/// The still cursor's side (`system-cursor:32514/32`).
pub const SOURCE_SIDE: u32 = 32;
/// A rotated frame's side: the source's diagonal, rounded up to even, so
/// that no rotation is cut and the centre is a pixel corner.
pub const FRAME_SIDE: u32 = frame_side(SOURCE_SIDE);
/// The square the turning hourglass stays inside, centred in its frame:
/// the slot it takes beside the loading bar and in the busy panel.
pub const ENVELOPE: u32 = SOURCE_SIDE;

/// The side of `side`'s rotated frames: ⌈side·√2⌉, rounded up to even.
pub const fn frame_side(side: u32) -> u32 {
    // ⌈side·√2⌉ in integers: the least n with n² ≥ 2·side².
    let target = 2 * (side as u64) * (side as u64);
    let mut n = side as u64;
    while n * n < target { n += 1; }
    (n + (n & 1)) as u32
}

/// The frame shown after `elapsed`.
pub fn frame_at(elapsed: Duration) -> u32 {
    ((elapsed.as_millis() / u128::from(FRAME_MS)) % u128::from(FRAME_COUNT)) as u32
}

/// The clockwise turn, in degrees, frame `frame` shows.
pub fn angle_of(frame: u32) -> u32 {
    (frame % FRAME_COUNT) * STEP_DEGREES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_cursor_cadence_keeps_frame_order_and_wraps() {
        for frame in 0..FRAME_COUNT {
            let start = u64::from(frame) * FRAME_MS;
            assert_eq!(frame_at(Duration::from_millis(start)), frame);
            assert_eq!(frame_at(Duration::from_millis(start + FRAME_MS - 1)), frame);
        }
        assert_eq!(frame_at(Duration::from_millis(FRAME_MS * u64::from(FRAME_COUNT))), 0);
        assert_eq!(frame_at(Duration::MAX), ((Duration::MAX.as_millis() / 83) % 12) as u32);
    }

    #[test]
    fn the_hourglass_turns_thirty_degrees_a_step_and_a_full_turn_a_cycle() {
        assert_eq!(STEP_DEGREES, 30);
        let angles: Vec<u32> = (0..FRAME_COUNT).map(|f| angle_of(frame_at(Duration::from_millis(u64::from(f) * FRAME_MS)))).collect();
        assert_eq!(angles, [0, 30, 60, 90, 120, 150, 180, 210, 240, 270, 300, 330]);
        assert_eq!(angle_of(FRAME_COUNT), 0, "a full turn is back upright");
        assert_eq!(STEP_DEGREES * FRAME_COUNT, 360);
    }

    #[test]
    fn rotated_frames_hold_the_whole_cursor() {
        assert_eq!(FRAME_SIDE, 46);
        assert_eq!(frame_side(1), 2);
        for side in 1..=256u32 {
            let n = frame_side(side);
            assert_eq!(n % 2, 0);
            assert!(u64::from(n) * u64::from(n) >= 2 * u64::from(side) * u64::from(side), "{side}: {n}");
            assert!(n <= side * 3 / 2 + 2, "{side}: {n} is no larger than needed");
        }
        assert!(ENVELOPE <= FRAME_SIDE);
    }
}
