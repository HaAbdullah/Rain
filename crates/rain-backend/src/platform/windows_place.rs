//! The real desktop for window placement: the windows, the monitors, and moving a window so that
//! its visible part fills a rectangle. Used by `core::placement` (section 02 6.17).
//!
//! Two facts of Windows shape this file:
//! - A window of Windows 10 and 11 has an invisible resize border. `GetWindowRect` includes it and
//!   the DWM extended frame bounds do not, so the move measures both and grows the rectangle by
//!   the border (`outer_for_visible`), then looks once more: after a move to a monitor of another
//!   scale the border has another width.
//! - Coordinates are only physical pixels when the thread is per-monitor DPI aware. Every call of
//!   this file runs inside `DpiScope`, which makes the calling thread aware and puts the old
//!   awareness back, whatever the process manifest says.
//!
//! Whether SetWindowPos moves windows of every program the way this file expects was proved on
//! the owner's PC only by the one test that opens its own window (UNPROVEN for other programs).
//! Window titles can hold anything, so nothing here is logged.

use std::collections::HashMap;
use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor,
    MDT_EFFECTIVE_DPI, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetCursorPos, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    IsZoomed, SW_RESTORE, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER, SetWindowPos,
    ShowWindow, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use super::process::list_processes;
use crate::core::placement::{Desktop, MonitorInfo, Rect, SeenWindow, outer_for_visible};

/// The program of Rain's own window (the Tauri shell). Its monitor is "this screen".
const SHELL_IMAGE: &str = "rain.exe";
/// `MONITORINFO.dwFlags` of the primary monitor.
const MONITORINFOF_PRIMARY: u32 = 1;
/// How many times a move is measured and corrected.
const TRIES: usize = 3;

/// The real windows and monitors of this PC.
pub struct SystemDesktop;

/// Makes the calling thread per-monitor DPI aware until it is dropped.
struct DpiScope(DPI_AWARENESS_CONTEXT);

impl DpiScope {
    fn enter() -> Self {
        // SAFETY: the call takes a constant and changes only the awareness of this thread.
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}

impl Drop for DpiScope {
    fn drop(&mut self) {
        // SAFETY: puts back the context that the same call returned.
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

fn handle(hwnd: isize) -> HWND {
    HWND(hwnd as *mut c_void)
}

fn rect_of(rect: RECT) -> Rect {
    Rect::new(rect.left, rect.top, rect.right, rect.bottom)
}

/// The outer rectangle of a window, with its invisible border.
fn outer_rect(window: HWND) -> Option<Rect> {
    let mut rect = RECT::default();
    // SAFETY: `rect` is a valid out parameter; a window that is gone makes the call fail.
    unsafe { GetWindowRect(window, &mut rect) }.ok()?;
    Some(rect_of(rect))
}

/// What the eye sees of a window: the DWM extended frame bounds.
fn visible_rect(window: HWND) -> Option<Rect> {
    let mut rect = RECT::default();
    // SAFETY: `rect` is a `RECT`, which is what `DWMWA_EXTENDED_FRAME_BOUNDS` writes, and its size
    // is passed.
    unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            std::ptr::from_mut(&mut rect).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    }
    .ok()?;
    Some(rect_of(rect))
}

fn is_cloaked(window: HWND) -> bool {
    let mut cloaked = 0u32;
    // SAFETY: `DWMWA_CLOAKED` writes one `DWORD`; its size is passed.
    let asked = unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED,
            std::ptr::from_mut(&mut cloaked).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    asked.is_ok() && cloaked != 0
}

struct Collected {
    windows: Vec<(isize, u32, String)>,
}

unsafe extern "system" fn collect(window: HWND, parameter: LPARAM) -> BOOL {
    // SAFETY: `parameter` is the pointer to the `Collected` of `list_visible`, alive and used by
    // nothing else while `EnumWindows` runs.
    let found = unsafe { &mut *(parameter.0 as *mut Collected) };
    // SAFETY: `window` is a handle that `EnumWindows` just gave; the buffer is as long as the
    // length that `GetWindowTextW` is told.
    unsafe {
        if !IsWindowVisible(window).as_bool() {
            return BOOL(1);
        }
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
        // A tool window is no window for a person, unless it says it is an app window. A window
        // that never takes the focus is a popup, a tooltip, or an overlay.
        if (style & WS_EX_TOOLWINDOW.0 != 0 && style & WS_EX_APPWINDOW.0 == 0)
            || style & WS_EX_NOACTIVATE.0 != 0
        {
            return BOOL(1);
        }
        // A window with an owner is a dialog of another window.
        if GetWindow(window, GW_OWNER).is_ok_and(|owner| !owner.0.is_null()) {
            return BOOL(1);
        }
        if is_cloaked(window) {
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
            found.windows.push((
                window.0 as isize,
                pid,
                String::from_utf16_lossy(&buffer[..copied as usize]),
            ));
        }
    }
    BOOL(1)
}

/// The visible windows, topmost first, with the image name of their owners.
fn list_visible() -> Vec<SeenWindow> {
    let mut collected = Collected {
        windows: Vec::new(),
    };
    // SAFETY: `collect` runs only during this call, and the pointer to `collected` that it gets
    // stays valid for the whole call.
    unsafe {
        let _ = EnumWindows(
            Some(collect),
            LPARAM(std::ptr::from_mut(&mut collected) as isize),
        );
    }
    let images: HashMap<u32, String> = list_processes()
        .into_iter()
        .map(|process| (process.pid, process.name.to_lowercase()))
        .collect();
    collected
        .windows
        .into_iter()
        .map(|(hwnd, pid, title)| SeenWindow {
            hwnd,
            pid,
            image: images.get(&pid).cloned().unwrap_or_default(),
            title,
        })
        .collect()
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    parameter: LPARAM,
) -> BOOL {
    // SAFETY: `parameter` is the pointer to the `Vec` of `list_monitors`, alive and used by
    // nothing else while `EnumDisplayMonitors` runs.
    let found = unsafe { &mut *(parameter.0 as *mut Vec<MonitorInfo>) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid `MONITORINFO` with its size set; the handle came from the
    // enumeration.
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        // SAFETY: two valid out parameters. A failure leaves the 96 that was put in.
        unsafe {
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        }
        found.push(MonitorInfo {
            work: rect_of(info.rcWork),
            primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
            dpi: dpi_x,
        });
    }
    BOOL(1)
}

fn list_monitors() -> Vec<MonitorInfo> {
    let mut found: Vec<MonitorInfo> = Vec::new();
    // SAFETY: `collect_monitor` runs only during this call, and the pointer to `found` stays
    // valid for the whole call.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(std::ptr::from_mut(&mut found) as isize),
        );
    }
    found
}

impl Desktop for SystemDesktop {
    fn windows(&self) -> Vec<SeenWindow> {
        let _scope = DpiScope::enter();
        list_visible()
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        let _scope = DpiScope::enter();
        list_monitors()
    }

    fn anchor(&self) -> Option<(i32, i32)> {
        let _scope = DpiScope::enter();
        let own = list_visible()
            .into_iter()
            .find(|window| window.image == SHELL_IMAGE)
            .and_then(|window| outer_rect(handle(window.hwnd)));
        if let Some(rect) = own {
            return Some(((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2));
        }
        let mut point = POINT::default();
        // SAFETY: `point` is a valid out parameter.
        unsafe { GetCursorPos(&mut point) }.ok()?;
        Some((point.x, point.y))
    }

    fn place(&self, hwnd: isize, visible: Rect) -> Result<(), String> {
        let _scope = DpiScope::enter();
        let window = handle(hwnd);
        // SAFETY: plain window calls on a handle; a handle that is gone makes them fail.
        unsafe {
            if IsZoomed(window).as_bool() || IsIconic(window).as_bool() {
                let _ = ShowWindow(window, SW_RESTORE);
            }
        }
        for attempt in 0..TRIES {
            let outer = outer_rect(window).ok_or("the window is gone")?;
            let seen = visible_rect(window).unwrap_or(outer);
            if attempt > 0 && seen == visible {
                break;
            }
            let wanted = outer_for_visible(visible, outer, seen);
            // SAFETY: a plain window call. The size is the rectangle's own; no z-order change and
            // no activation, so the window does not take the focus.
            unsafe {
                SetWindowPos(
                    window,
                    None,
                    wanted.left,
                    wanted.top,
                    wanted.width(),
                    wanted.height(),
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                )
            }
            .map_err(|e| {
                // 5 is "access denied": the window belongs to a program that runs as administrator.
                if e.code().0 as u32 & 0xFFFF == 5 {
                    "it runs as administrator".to_owned()
                } else {
                    "Windows refused".to_owned()
                }
            })?;
        }
        Ok(())
    }

    fn visible_frame(&self, hwnd: isize) -> Option<Rect> {
        let _scope = DpiScope::enter();
        let window = handle(hwnd);
        visible_rect(window).or_else(|| outer_rect(window))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::launch_contract::{Placement, WindowQuery};
    use crate::core::placement::{Placer, number_monitors, zone_rect};
    use rain_types::LayoutZone;
    use std::sync::Arc;
    use std::time::Duration;
    use windows::Win32::Foundation::WPARAM;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW, MSG, PostThreadMessageW,
        TranslateMessage, WINDOW_EX_STYLE, WM_QUIT, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };
    use windows::core::{PCWSTR, w};

    #[test]
    fn this_pc_has_a_monitor_when_it_has_a_desktop() {
        // A build server may have no display at all, so only the shape is checked.
        for monitor in SystemDesktop.monitors() {
            assert!(monitor.work.width() > 0 && monitor.work.height() > 0);
            assert!(monitor.dpi >= 96);
        }
    }

    #[test]
    fn the_listed_windows_have_titles_and_owners() {
        for window in SystemDesktop.windows() {
            assert!(!window.title.is_empty());
            assert_ne!(window.pid, 0);
        }
    }

    /// A window that the test makes and owns, on a thread of its own with a message loop. It is
    /// closed when this value is dropped, also when an assertion failed.
    struct TestWindow {
        hwnd: isize,
        thread_id: u32,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl TestWindow {
        fn open(title: &str) -> Option<Self> {
            let (sender, receiver) = std::sync::mpsc::channel();
            let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
            let thread = std::thread::spawn(move || {
                // SAFETY: a window of the predefined `STATIC` class is made on this thread, which
                // then runs its message loop until `WM_QUIT`, and destroys the window itself.
                unsafe {
                    let made = CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        w!("STATIC"),
                        PCWSTR(wide.as_ptr()),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                        200,
                        200,
                        500,
                        400,
                        None,
                        None,
                        None,
                        None,
                    );
                    let Ok(window) = made else {
                        let _ = sender.send(None);
                        return;
                    };
                    let _ = sender.send(Some((window.0 as isize, GetCurrentThreadId())));
                    let mut message = MSG::default();
                    while GetMessageW(&mut message, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                    let _ = DestroyWindow(window);
                }
            });
            let (hwnd, thread_id) = receiver.recv_timeout(Duration::from_secs(3)).ok()??;
            Some(Self {
                hwnd,
                thread_id,
                thread: Some(thread),
            })
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            // SAFETY: asks the message loop of the test window to end.
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// The one test of this feature that uses a real window: its own, made here, moved, and
    /// closed again. No window of the user is looked at for more than its title, or touched.
    #[tokio::test]
    async fn a_window_of_its_own_is_found_moved_into_its_zone_and_closed() {
        let desktop = Arc::new(SystemDesktop);
        let numbered = number_monitors(desktop.monitors());
        if numbered.is_empty() {
            eprintln!("skipped: no display on this machine");
            return;
        }
        let title = format!("rain-placement-test-{}", std::process::id());
        let placer = Placer::with_timing(
            desktop.clone(),
            Duration::from_secs(3),
            Duration::from_millis(50),
            Duration::from_millis(300),
        );
        let mut run = placer.begin().await;
        let before = run.snapshot().await;
        let Some(window) = TestWindow::open(&title) else {
            eprintln!("skipped: the session cannot show windows");
            return;
        };

        let placement = Placement {
            zone: LayoutZone::BottomRight,
            monitor: Some(1),
            find: WindowQuery {
                images: Vec::new(),
                title_has: Some(title.to_lowercase()),
                allow_existing: false,
            },
        };
        let note = run.place(&before, &placement, None).await;
        assert_eq!(note, None);

        let target = zone_rect(LayoutZone::BottomRight, numbered[0].work).unwrap();
        let seen = desktop
            .visible_frame(window.hwnd)
            .expect("the window is there");
        let near = |a: i32, b: i32| (a - b).abs() <= 2;
        assert!(
            near(seen.left, target.left)
                && near(seen.top, target.top)
                && near(seen.right, target.right)
                && near(seen.bottom, target.bottom),
            "the window is at {seen:?}, the zone is {target:?}"
        );
        eprintln!("placed at {seen:?}");
        drop(window);
    }
}
