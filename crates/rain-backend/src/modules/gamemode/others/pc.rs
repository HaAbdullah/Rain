//! What closing other programs asks of the PC, behind one trait. Section 06 6.11.
//!
//! The real version forwards to `platform::process` and `platform::windows`. A test puts a fake
//! in its place, so that no real program is ever closed by a test (`FakePc` in
//! `tests/common/fakes.rs`, and the fake in `closing.rs`).

use crate::platform::process::{self, ProcessFacts, Terminated};
use crate::platform::windows::{self, TopWindow};

/// The processes and the windows at one moment.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub procs: Vec<ProcessFacts>,
    pub wins: Vec<TopWindow>,
}

pub trait Pc: Send + Sync {
    /// Every process and every top-level window. Blocking: about 20 ms.
    fn scan(&self) -> Scan;
    /// Every top-level window. Blocking: a few milliseconds.
    fn windows(&self) -> Vec<TopWindow>;
    /// Posts `WM_CLOSE` to one top-level window. `false` when the window is gone.
    fn post_close(&self, hwnd: isize) -> bool;
    /// Whether the process with this pid and start time still runs.
    fn alive(&self, pid: u32, created: u64) -> bool;
    /// Ends the process with this pid and start time. One attempt.
    fn terminate(&self, pid: u32, created: u64) -> Terminated;
}

/// The real PC.
pub struct SystemPc;

impl Pc for SystemPc {
    fn scan(&self) -> Scan {
        Scan {
            procs: process::list_facts(),
            wins: windows::top_windows(),
        }
    }

    fn windows(&self) -> Vec<TopWindow> {
        windows::top_windows()
    }

    fn post_close(&self, hwnd: isize) -> bool {
        windows::post_close(hwnd)
    }

    fn alive(&self, pid: u32, created: u64) -> bool {
        process::alive_since(pid, created)
    }

    fn terminate(&self, pid: u32, created: u64) -> Terminated {
        process::terminate_if_same(pid, created)
    }
}

/// A PC with nothing on it. It is what a backend gets when a test replaced the other modules but
/// not this part, so that such a test can never close a real program.
pub struct NoPc;

impl Pc for NoPc {
    fn scan(&self) -> Scan {
        Scan::default()
    }

    fn windows(&self) -> Vec<TopWindow> {
        Vec::new()
    }

    fn post_close(&self, _hwnd: isize) -> bool {
        false
    }

    fn alive(&self, _pid: u32, _created: u64) -> bool {
        false
    }

    fn terminate(&self, _pid: u32, _created: u64) -> Terminated {
        Terminated::Gone
    }
}
