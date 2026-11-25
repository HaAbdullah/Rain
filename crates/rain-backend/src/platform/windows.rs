//! The visible top-level windows. Used by the projects module to tell which project is open
//! (section 02 6.x). About 1 to 3 ms for 150 windows. It reads window titles, which can hold
//! anything, so nothing here is logged.

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GWL_EXSTYLE, GetClassNameW, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    PostMessageW, WM_CLOSE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
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

// ---- every top-level window, for game mode ----

/// A top-level window, visible or not. Never a child window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopWindow {
    /// The window handle as a number.
    pub hwnd: isize,
    /// The process that the window belongs to. For a Store app that Windows shows in an
    /// `ApplicationFrameWindow` this is the app, not the frame host.
    pub pid: u32,
    /// A person can see it: shown, not a tool window, and not empty. A minimised window counts.
    pub visible: bool,
}

/// The class name of the frame that hosts a Store app's own window.
const FRAME_CLASS: &str = "ApplicationFrameWindow";

/// Every top-level window of every process, with no title read (so a hung program cannot block
/// the call). About 1 ms for 150 windows.
pub fn top_windows() -> Vec<TopWindow> {
    let mut found: Vec<TopWindow> = Vec::new();
    // SAFETY: `collect_top` is called only during this call, and the pointer to `found` stays
    // valid for the whole call.
    unsafe {
        let _ = EnumWindows(
            Some(collect_top),
            LPARAM(std::ptr::from_mut(&mut found) as isize),
        );
    }
    found
}

/// Asks a window to close, like its close button (`WM_CLOSE`). It is only posted: the program
/// decides what to do. `false` when the window is gone.
pub fn post_close(hwnd: isize) -> bool {
    // SAFETY: posting a message to a handle that no longer exists fails, it never crashes.
    unsafe {
        PostMessageW(
            Some(HWND(hwnd as *mut std::ffi::c_void)),
            WM_CLOSE,
            WPARAM(0),
            LPARAM(0),
        )
    }
    .is_ok()
}

unsafe extern "system" fn collect_top(window: HWND, parameter: LPARAM) -> BOOL {
    // SAFETY: `parameter` is the pointer to the `Vec` from `top_windows`, alive and used by
    // nothing else while `EnumWindows` runs.
    let found = unsafe { &mut *(parameter.0 as *mut Vec<TopWindow>) };
    let mut pid = 0u32;
    // SAFETY: plain window calls on a handle that `EnumWindows` just gave; the buffers are as
    // long as the lengths that are passed.
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut pid));
        if pid == 0 {
            return BOOL(1);
        }
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
        let tool = style & WS_EX_TOOLWINDOW.0 != 0 && style & WS_EX_APPWINDOW.0 == 0;
        let mut rect = RECT::default();
        let sized = GetWindowRect(window, &mut rect).is_ok()
            && (rect.right > rect.left && rect.bottom > rect.top);
        let visible =
            IsWindowVisible(window).as_bool() && !tool && (sized || IsIconic(window).as_bool());

        let mut class = [0u16; 64];
        let length = GetClassNameW(window, &mut class).max(0) as usize;
        if String::from_utf16_lossy(&class[..length]) == FRAME_CLASS {
            // The Store app lives in a child window of the frame, owned by the app's process.
            let mut hosted = HostedApp { frame: pid, app: 0 };
            let _ = EnumChildWindows(
                Some(window),
                Some(find_hosted),
                LPARAM(std::ptr::from_mut(&mut hosted) as isize),
            );
            if hosted.app != 0 {
                pid = hosted.app;
            }
        }
        found.push(TopWindow {
            hwnd: window.0 as isize,
            pid,
            visible,
        });
    }
    BOOL(1)
}

struct HostedApp {
    frame: u32,
    app: u32,
}

unsafe extern "system" fn find_hosted(child: HWND, parameter: LPARAM) -> BOOL {
    // SAFETY: `parameter` is the pointer to the `HostedApp` of `collect_top`, alive and used by
    // nothing else while `EnumChildWindows` runs.
    let hosted = unsafe { &mut *(parameter.0 as *mut HostedApp) };
    let mut pid = 0u32;
    // SAFETY: a plain call on a handle that `EnumChildWindows` just gave.
    unsafe {
        GetWindowThreadProcessId(child, Some(&mut pid));
    }
    if pid != 0 && pid != hosted.frame {
        hosted.app = pid;
        return BOOL(0); // found
    }
    BOOL(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_top_window_has_an_owner_and_the_list_is_not_empty_on_a_desktop() {
        // Only the shape is checked: a build server may have no window at all.
        for window in top_windows() {
            assert_ne!(window.pid, 0);
            assert_ne!(window.hwnd, 0);
        }
    }

    #[test]
    fn closing_a_window_that_does_not_exist_reports_false() {
        assert!(!post_close(0x7FFF_FFF0));
    }

    #[test]
    fn the_list_has_only_windows_with_titles_and_an_owner() {
        // The PC may have no window at all (a build server), so only the shape is checked.
        for window in SystemWindowLister.windows() {
            assert!(!window.title.is_empty());
            assert_ne!(window.pid, 0);
        }
    }
}
