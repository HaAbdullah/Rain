//! `focusSession`: bring the window of a session to the front. Section 03 6.6.
//!
//! The result says what happened and is never an error: `window` (a window of the session's host
//! was brought to the front and verified), `app` (the host application came to the front, but not
//! the session's own tab or conversation: no way to select a Windows Terminal tab or a
//! conversation of the desktop app exists), `none` (nothing could be done).

use std::sync::Arc;

use rain_types::{FocusKind, SessionHost};

use super::seams::{ProcessProbe, WindowApi};
use crate::platform::windows_focus::FocusWindow;

/// The parents of the session's process that are followed, at most.
const CHAIN_LEVELS: usize = 12;

/// What focus needs to know about a session.
pub struct FocusRequest {
    pub host: SessionHost,
    pub pid: u32,
    pub title: String,
    /// The name of the working folder, which terminals show in their window title.
    pub folder: String,
}

/// The window that fits best, and whether its title matched the session.
pub fn choose<'a>(
    candidates: &'a [FocusWindow],
    folder: &str,
    title: &str,
) -> Option<(&'a FocusWindow, bool)> {
    let contains = |window: &FocusWindow, needle: &str| {
        !needle.trim().is_empty() && window.title.to_lowercase().contains(&needle.to_lowercase())
    };
    candidates
        .iter()
        .find(|window| contains(window, folder) || contains(window, title))
        .map(|window| (window, true))
        .or_else(|| candidates.first().map(|window| (window, false)))
}

/// `window` for a terminal session whose window was identified, `app` for everything else.
pub fn kind_of(host: SessionHost, title_matched: bool, candidate_count: usize) -> FocusKind {
    if host == SessionHost::Terminal && (title_matched || candidate_count == 1) {
        FocusKind::Window
    } else {
        FocusKind::App
    }
}

/// Runs the steps of 6.6. Blocking: call it through `core::blocking::run`.
pub fn focus(
    request: &FocusRequest,
    probe: &Arc<dyn ProcessProbe>,
    windows: &Arc<dyn WindowApi>,
) -> FocusKind {
    let chain: Vec<u32> = probe
        .chain(request.pid, CHAIN_LEVELS)
        .into_iter()
        .map(|process| process.pid)
        .collect();
    if chain.is_empty() {
        return FocusKind::None;
    }
    let candidates: Vec<FocusWindow> = windows
        .windows()
        .into_iter()
        .filter(|window| chain.contains(&window.pid))
        .collect();
    let Some((chosen, matched)) = choose(&candidates, &request.folder, &request.title) else {
        return FocusKind::None;
    };
    if windows.bring_to_front(chosen.hwnd) {
        kind_of(request.host, matched, candidates.len())
    } else {
        // Windows refused. The taskbar button flashes so that the user finds the window.
        windows.flash(chosen.hwnd);
        FocusKind::None
    }
}

#[cfg(test)]
mod tests {
    use super::super::seams::ProcInfo;
    use super::*;
    use std::sync::Mutex;

    struct Probe(Vec<u32>);
    impl ProcessProbe for Probe {
        fn creation_filetime(&self, _: u32) -> Option<u64> {
            None
        }
        fn is_running(&self, _: u32) -> bool {
            true
        }
        fn image_name(&self, _: u32) -> Option<String> {
            None
        }
        fn chain(&self, _: u32, _: usize) -> Vec<ProcInfo> {
            self.0
                .iter()
                .map(|pid| ProcInfo {
                    pid: *pid,
                    name: "x.exe".into(),
                })
                .collect()
        }
        fn descendants(&self, _: u32) -> Vec<u32> {
            Vec::new()
        }
        fn terminate(&self, _: u32) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Windows {
        list: Vec<FocusWindow>,
        works: bool,
        brought: Mutex<Vec<isize>>,
        flashed: Mutex<Vec<isize>>,
    }
    impl WindowApi for Windows {
        fn windows(&self) -> Vec<FocusWindow> {
            self.list.clone()
        }
        fn bring_to_front(&self, hwnd: isize) -> bool {
            self.brought.lock().unwrap().push(hwnd);
            self.works
        }
        fn flash(&self, hwnd: isize) {
            self.flashed.lock().unwrap().push(hwnd);
        }
    }

    fn window(hwnd: isize, pid: u32, title: &str) -> FocusWindow {
        FocusWindow {
            hwnd,
            pid,
            title: title.to_owned(),
        }
    }

    fn run(
        chain: Vec<u32>,
        list: Vec<FocusWindow>,
        works: bool,
        host: SessionHost,
    ) -> (FocusKind, Vec<isize>, Vec<isize>) {
        let windows = Arc::new(Windows {
            list,
            works,
            brought: Mutex::default(),
            flashed: Mutex::default(),
        });
        let probe: Arc<dyn ProcessProbe> = Arc::new(Probe(chain));
        let api: Arc<dyn WindowApi> = windows.clone();
        let request = FocusRequest {
            host,
            pid: 10,
            title: "Parser work".into(),
            folder: "demo".into(),
        };
        let kind = focus(&request, &probe, &api);
        let brought = windows.brought.lock().unwrap().clone();
        let flashed = windows.flashed.lock().unwrap().clone();
        (kind, brought, flashed)
    }

    #[test]
    fn a_terminal_window_whose_title_names_the_folder_gives_window() {
        let list = vec![
            window(1, 99, "Other"),
            window(2, 20, "other tab"),
            window(3, 20, "pwsh - demo"),
        ];
        let (kind, brought, _) = run(vec![10, 20], list, true, SessionHost::Terminal);
        assert_eq!(kind, FocusKind::Window);
        assert_eq!(brought, [3]);
    }

    #[test]
    fn one_candidate_is_enough_for_a_terminal_and_several_without_a_match_give_app() {
        let (kind, brought, _) = run(
            vec![10, 20],
            vec![window(2, 20, "only one")],
            true,
            SessionHost::Terminal,
        );
        assert_eq!((kind, brought), (FocusKind::Window, vec![2]));

        let list = vec![window(2, 20, "tab a"), window(4, 20, "tab b")];
        let (kind, brought, _) = run(vec![10, 20], list, true, SessionHost::Terminal);
        assert_eq!(
            (kind, brought),
            (FocusKind::App, vec![2]),
            "the first in z-order is chosen"
        );
    }

    #[test]
    fn a_desktop_session_gives_app_even_with_a_title_match() {
        let list = vec![window(7, 30, "Claude - demo")];
        let (kind, _, _) = run(vec![10, 30], list, true, SessionHost::Desktop);
        assert_eq!(kind, FocusKind::App);
    }

    #[test]
    fn no_window_in_the_chain_or_no_chain_gives_none_without_touching_anything() {
        let (kind, brought, flashed) = run(
            vec![10, 20],
            vec![window(1, 99, "demo")],
            true,
            SessionHost::Terminal,
        );
        assert_eq!((kind, brought, flashed), (FocusKind::None, vec![], vec![]));
        let (kind, brought, _) = run(
            vec![],
            vec![window(1, 10, "demo")],
            true,
            SessionHost::Terminal,
        );
        assert_eq!((kind, brought), (FocusKind::None, vec![]));
    }

    #[test]
    fn when_windows_refuses_the_taskbar_flashes_and_the_result_is_none() {
        let (kind, brought, flashed) = run(
            vec![10, 20],
            vec![window(5, 20, "demo")],
            false,
            SessionHost::Terminal,
        );
        assert_eq!(kind, FocusKind::None);
        assert_eq!((brought, flashed), (vec![5], vec![5]));
    }
}
