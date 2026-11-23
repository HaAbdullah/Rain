//! The visible top-level windows. Used by the projects module to tell which project is open
//! (section 02 6.x). About 1 to 3 ms for 150 windows. It reads window titles, which can hold
//! anything, so nothing here is logged.

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::BOOL;

/// A visible top-level window that has a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// The process that owns the window.
    pub pid: u32,
    pub title: String,
}

/// Which windows are open, behind a trait so that a test says which windows exist (section 02).
pub trait WindowLister: Send + Sync {
    /// Blocking: a few milliseconds.
    fn windows(&self) -> Vec<WindowInfo>;
}

/// The real windows of this PC.
pub struct SystemWindowLister;

impl WindowLister for SystemWindowLister {
    fn windows(&self) -> Vec<WindowInfo> {
        list_windows()
    }
}

/// Every visible top-level window with a non-empty title.
pub fn list_windows() -> Vec<WindowInfo> {
    let mut found: Vec<WindowInfo> = Vec::new();
    // SAFETY: `collect` is called only during this call, and the pointer to `found` that it gets
    // as its parameter stays valid for the whole call.
    unsafe {
        let _ = EnumWindows(
            Some(collect),
            LPARAM(std::ptr::from_mut(&mut found) as isize),
        );
    }
    found
}

unsafe extern "system" fn collect(window: HWND, parameter: LPARAM) -> BOOL {
    // SAFETY: `parameter` is the pointer to the `Vec` from `list_windows`, which is alive and not
    // used by anything else while `EnumWindows` runs.
    let found = unsafe { &mut *(parameter.0 as *mut Vec<WindowInfo>) };
    // SAFETY: `window` is a handle that `EnumWindows` just gave; the buffer has the length that
    // `GetWindowTextW` is told.
    unsafe {
        if !IsWindowVisible(window).as_bool() {
            return BOOL(1);
        }
        let length = GetWindowTextLengthW(window);
        if length <= 0 {
            return BOOL(1);
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(window, &mut buffer);
        if copied > 0 {
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, Some(&mut pid));
            found.push(WindowInfo {
                pid,
                title: String::from_utf16_lossy(&buffer[..copied as usize]),
            });
        }
    }
    BOOL(1) // go on
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_has_only_windows_with_titles_and_an_owner() {
        // The PC may have no window at all (a build server), so only the shape is checked.
        for window in SystemWindowLister.windows() {
            assert!(!window.title.is_empty());
            assert_ne!(window.pid, 0);
        }
    }
}
