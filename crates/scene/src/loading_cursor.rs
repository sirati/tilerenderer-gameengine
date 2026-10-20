//! Shared cadence for the game's animated gold cursor motif.
use std::time::Duration;

pub const SET: &str = "Viewer@loading_cursor/0";
pub const FRAME_MS: u64 = 83;
pub const FRAME_COUNT: u32 = 30;
/// Viewer-clock flag and the FLC period, interpreted by the GPU only.
pub const TICKS_PER_FRAME: u32 = (1 << 31) | FRAME_MS as u32;

/// The FLC's thirty displayed frames, excluding its closing delta frame.
pub fn frame_at(elapsed: Duration) -> u32 {
    ((elapsed.as_millis() / u128::from(FRAME_MS)) % u128::from(FRAME_COUNT)) as u32
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
        assert_eq!(frame_at(Duration::MAX), ((Duration::MAX.as_millis() / 83) % 30) as u32);
    }
}
