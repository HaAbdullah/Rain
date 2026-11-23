//! How long the user has not touched the keyboard or the mouse. Foundation section 11.6.
//!
//! `GetLastInputInfo` works from a process without a window. It reports the tick count of the
//! last input as 32 bits, which wrap every 49.7 days. The wrap is by design: the difference
//! between two 32-bit tick counts is right across it.

use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

/// Milliseconds since the last input of this session, or `None` when Windows refuses.
pub fn idle_ms() -> Option<u64> {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is a valid `LASTINPUTINFO` whose `cbSize` is set, as the call requires.
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return None;
    }
    // SAFETY: `GetTickCount64` takes no arguments.
    let now = unsafe { GetTickCount64() };
    Some(elapsed_since(now, info.dwTime))
}

/// The difference between the current tick count and a 32-bit last-input tick count.
fn elapsed_since(now_ticks: u64, last_input_ticks: u32) -> u64 {
    // Truncating `now` to 32 bits matches `last_input_ticks`, so the subtraction wraps correctly.
    u64::from((now_ticks as u32).wrapping_sub(last_input_ticks))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_difference_is_right_across_the_32_bit_wrap() {
        assert_eq!(elapsed_since(5_000, 4_000), 1_000);
        // 49.7 days of uptime: the 64-bit count is past 2^32, the input tick count is just below.
        let now = (1u64 << 32) + 500;
        assert_eq!(elapsed_since(now, u32::MAX - 499), 1_000);
        assert_eq!(elapsed_since(7, 7), 0);
    }

    #[test]
    fn windows_answers_with_a_plausible_idle_time() {
        // The value depends on whoever sits at the PC, so only its existence is checked.
        let idle = idle_ms().expect("GetLastInputInfo works from a process without a window");
        assert!(idle < 365 * 24 * 3600 * 1000);
    }
}
