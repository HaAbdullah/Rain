//! Bringing a window to the front. Used by `focusSession` of the sessions module (section 03 6.6).
//!
//! `SetForegroundWindow` is honoured only for the foreground process, or for a process that
//! received the last input. The backend is neither, so a plain call may be refused. This file
//! tries the three known ways in turn and checks after each one whether the window really is in
//! front, so the caller can report the truth. Whether any of them works from the backend on a
//! real desktop was never seen (UNPROVEN); the hand check F1 of section 03 settles it.
//!
//! Window titles can hold anything, so nothing here is logged.

use std::ffi::c_void;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_MENU, keybd_event};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, FLASHW_ALL, FLASHW_TIMERNOFG, FLASHWINFO, FlashWindowEx,
    GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE, SetForegroundWindow,
    ShowWindow, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

/// How long a window gets to arrive in front after each try.
const VERIFY_WITHIN: Duration = Duration::from_millis(150);
const VERIFY_STEP: Duration = Duration::from_millis(10);

/// A visible top-level window that a person would call a window: it has a title and is not a
/// tool window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusWindow {
    /// The window handle as a number.
    pub hwnd: isize,
    /// The process that owns the window.
    pub pid: u32,
    pub title: String,
}

fn handle(hwnd: isize) -> HWND {
    HWND(hwnd as *mut c_void)
}

/// The windows in z-order, the one in front first.
pub fn top_level_windows() -> Vec<FocusWindow> {
    let mut found: Vec<FocusWindow> = Vec::new();
    // SAFETY: `collect` runs only during this call, and the pointer to `found` that it receives
    // stays valid for the whole call.
    unsafe {
        let _ = EnumWindows(
            Some(collect),
            LPARAM(std::ptr::from_mut(&mut found) as isize),
        );
    }
    found
}

unsafe extern "system" fn collect(window: HWND, parameter: LPARAM) -> BOOL {
    // SAFETY: `parameter` is the pointer to the `Vec` of `top_level_windows`, alive and used by
    // nothing else while `EnumWindows` runs.
    let found = unsafe { &mut *(parameter.0 as *mut Vec<FocusWindow>) };
    // SAFETY: `window` is a handle that `EnumWindows` just gave; the buffer is as long as the
    // length that `GetWindowTextW` is told.
    unsafe {
        if !IsWindowVisible(window).as_bool() {
            return BOOL(1);
        }
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
        if style & WS_EX_TOOLWINDOW.0 != 0 {
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
            found.push(FocusWindow {
                hwnd: window.0 as isize,
                pid,
                title: String::from_utf16_lossy(&buffer[..copied as usize]),
            });
        }
    }
    BOOL(1)
}

/// The window in front, as a number (0 when there is none).
pub fn foreground() -> isize {
    // SAFETY: the call has no pointer parameter.
    unsafe { GetForegroundWindow() }.0 as isize
}

/// Tries to bring the window to the front and says whether it really is in front afterwards.
/// Blocking: up to about half a second. The three ways, in this order: restore a minimised window
/// and ask plainly; share the input queue of the window that is in front; press the Alt key
/// first, which Windows counts as input.
pub fn bring_to_front(hwnd: isize) -> bool {
    let target = handle(hwnd);
    // SAFETY: plain window calls on a handle; a handle that no longer exists makes them fail,
    // never crash.
    unsafe {
        if IsIconic(target).as_bool() {
            let _ = ShowWindow(target, SW_RESTORE);
        }
        let _ = SetForegroundWindow(target);
        if arrived(hwnd) {
            return true;
        }

        // Way 2: borrow the right of the window that is in front by joining its input queue.
        let front = GetForegroundWindow();
        let front_thread = GetWindowThreadProcessId(front, None);
        let own_thread = GetCurrentThreadId();
        if front_thread != 0 && front_thread != own_thread {
            let _ = AttachThreadInput(own_thread, front_thread, true);
            let _ = BringWindowToTop(target);
            let _ = SetForegroundWindow(target);
            let _ = AttachThreadInput(own_thread, front_thread, false);
            if arrived(hwnd) {
                return true;
            }
        }

        // Way 3: a synthetic Alt key press makes Windows treat this process as the one that got
        // the last input.
        keybd_event(VK_MENU.0 as u8, 0, Default::default(), 0);
        keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
        let _ = SetForegroundWindow(target);
    }
    arrived(hwnd)
}

/// Waits a moment and asks whether `hwnd` is in front.
fn arrived(hwnd: isize) -> bool {
    let mut waited = Duration::ZERO;
    loop {
        if foreground() == hwnd {
            return true;
        }
        if waited >= VERIFY_WITHIN {
            return false;
        }
        // This runs on a blocking thread (`core::blocking::run`), so sleeping blocks no worker.
        #[allow(clippy::disallowed_methods)]
        std::thread::sleep(VERIFY_STEP);
        waited += VERIFY_STEP;
    }
}

/// Flashes the taskbar button of the window until the user looks at it. The last resort when the
/// window cannot be brought to the front.
pub fn flash(hwnd: isize) {
    let info = FLASHWINFO {
        cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
        hwnd: handle(hwnd),
        dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
        uCount: 3,
        dwTimeout: 0,
    };
    // SAFETY: `info` is a valid `FLASHWINFO` with its size set.
    unsafe {
        let _ = FlashWindowEx(&info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_have_titles_and_owners() {
        // A build server may have no window at all, so only the shape is checked.
        for window in top_level_windows() {
            assert!(!window.title.is_empty());
            assert_ne!(window.pid, 0);
        }
    }
}
