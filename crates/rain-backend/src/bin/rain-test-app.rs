//! A harmless program for the hand checks of the launcher (section 01 11.1, check 6).
//!
//! It has no window and does nothing for five minutes, then ends. A throwaway Start Menu
//! shortcut to it (`scripts/make-test-shortcut.ps1`) lets you see that Rain lists, starts and
//! tracks a classic program, and that the program stays alive when the backend stops.
//! Nothing in the automated tests starts it.
#![windows_subsystem = "windows"]

use std::time::Duration;

fn main() {
    // A plain program with no runtime: blocking is the whole point, so the backend's rule against
    // it does not apply here.
    #[allow(clippy::disallowed_methods)]
    std::thread::sleep(Duration::from_secs(5 * 60));
}
