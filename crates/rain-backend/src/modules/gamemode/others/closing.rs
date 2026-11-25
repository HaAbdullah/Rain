//! Closing the other programs, in order. Section 06 6.11.
//!
//! 1. Every program that shows a window is asked politely: `WM_CLOSE` to each of its visible
//!    top-level windows. Rain waits up to `polite_wait` for them.
//! 2. A program that is still alive and still shows a window is probably asking the user
//!    something (unsaved work). It is left alone and reported as "did not close". A program
//!    with a visible window is never forced.
//! 3. A program with no visible window (it had none, or it went to the tray) gets one polite
//!    attempt (`WM_CLOSE` to its hidden top-level windows, `hidden_wait`). What is still alive
//!    after that is ended with `TerminateProcess`, children first.
//! 4. A process that cannot be ended is reported as "needs administrator rights". Nothing is
//!    tried twice.
//!
//! All the effects go through the `Pc` trait, so the order is a plain test with a fake.

use std::collections::HashSet;
use std::time::Duration;

use tokio::time::Instant;

use super::pc::Pc;
use super::select::ProcId;
use crate::platform::process::Terminated;
use crate::platform::windows::TopWindow;

/// How long each step waits (section 06 6.11). A test replaces them with short ones.
#[derive(Debug, Clone)]
pub struct CloseTimings {
    /// How long a program with windows gets to close after it was asked.
    pub polite_wait: Duration,
    /// How long a program without a visible window gets after its hidden windows were asked.
    pub hidden_wait: Duration,
    /// How often the windows and the processes are looked at while waiting.
    pub poll: Duration,
}

impl Default for CloseTimings {
    fn default() -> Self {
        Self {
            polite_wait: Duration::from_secs(10),
            hidden_wait: Duration::from_secs(3),
            poll: Duration::from_millis(150),
        }
    }
}

/// One program to close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub procs: Vec<ProcId>,
}

/// What happened to a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Result {
    /// It closed after it was asked.
    Closed,
    /// It was ended (it had no window to ask, or it hid in the tray).
    Ended,
    /// It still shows a window after the wait, so it was left alone.
    StillOpen,
    /// Windows refused to end it: it needs administrator rights.
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub id: String,
    pub name: String,
    pub result: Result,
}

fn alive_of(pc: &dyn Pc, target: &Target) -> Vec<ProcId> {
    target
        .procs
        .iter()
        .copied()
        .filter(|p| pc.alive(p.pid, p.created))
        .collect()
}

fn pids(procs: &[ProcId]) -> HashSet<u32> {
    procs.iter().map(|p| p.pid).collect()
}

fn windows_of(windows: &[TopWindow], procs: &[ProcId], visible: bool) -> Vec<TopWindow> {
    let own = pids(procs);
    windows
        .iter()
        .filter(|w| own.contains(&w.pid) && w.visible == visible)
        .copied()
        .collect()
}

/// The processes of one program, the ones that were started last first: a child is ended before
/// the process that started it.
pub fn children_first(procs: &[ProcId]) -> Vec<ProcId> {
    let own = pids(procs);
    let depth = |start: &ProcId| {
        let (mut depth, mut current) = (0usize, *start);
        while depth < procs.len() {
            match procs.iter().find(|p| {
                p.pid == current.parent_pid && p.pid != current.pid && own.contains(&p.pid)
            }) {
                Some(parent) => {
                    depth += 1;
                    current = *parent;
                }
                None => break,
            }
        }
        depth
    };
    let mut ordered: Vec<(usize, ProcId)> = procs.iter().map(|p| (depth(p), *p)).collect();
    ordered.sort_by_key(|(depth, _)| std::cmp::Reverse(*depth));
    ordered.into_iter().map(|(_, p)| p).collect()
}

/// Closes the programs. `progress` is told after every change how many programs are finished.
pub async fn close_all(
    pc: &dyn Pc,
    targets: &[Target],
    timings: &CloseTimings,
    progress: &(dyn Fn(u32) + Sync),
) -> Vec<Outcome> {
    let mut results: Vec<Option<Result>> = vec![None; targets.len()];
    let report = |results: &[Option<Result>]| {
        progress(u32::try_from(results.iter().flatten().count()).unwrap_or(u32::MAX));
    };

    // 1. Ask every program that shows a window.
    let mut asked: Vec<usize> = Vec::new();
    let windows = pc.windows();
    for (index, target) in targets.iter().enumerate() {
        let visible = windows_of(&windows, &target.procs, true);
        for window in &visible {
            pc.post_close(window.hwnd);
        }
        if !visible.is_empty() {
            asked.push(index);
        }
    }

    // 2. Wait until none of them still shows a window, or the time is up.
    if !asked.is_empty() {
        let deadline = Instant::now() + timings.polite_wait;
        loop {
            let windows = pc.windows();
            let mut waiting = false;
            for &index in &asked {
                if results[index].is_some() {
                    continue;
                }
                let alive = alive_of(pc, &targets[index]);
                if alive.is_empty() {
                    results[index] = Some(Result::Closed);
                } else if !windows_of(&windows, &alive, true).is_empty() {
                    waiting = true;
                }
            }
            report(&results);
            if !waiting || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(timings.poll).await;
        }
    }

    // 3. A program that is still alive. With a window: it asks the user, so it stays.
    let mut rest: Vec<usize> = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        if results[index].is_some() {
            continue;
        }
        if alive_of(pc, target).is_empty() {
            results[index] = Some(Result::Closed);
        } else {
            rest.push(index);
        }
    }
    report(&results);

    // 4. One polite attempt at the hidden windows of what is left, then the end.
    let windows = pc.windows();
    let mut hidden_asked: Vec<usize> = Vec::new();
    for &index in &rest {
        let alive = alive_of(pc, &targets[index]);
        if !windows_of(&windows, &alive, true).is_empty() {
            results[index] = Some(Result::StillOpen);
            continue;
        }
        let hidden = windows_of(&windows, &alive, false);
        for window in &hidden {
            pc.post_close(window.hwnd);
        }
        if !hidden.is_empty() {
            hidden_asked.push(index);
        }
    }
    report(&results);
    if !hidden_asked.is_empty() {
        let deadline = Instant::now() + timings.hidden_wait;
        loop {
            for &index in &hidden_asked {
                if results[index].is_none() && alive_of(pc, &targets[index]).is_empty() {
                    results[index] = Some(Result::Closed);
                }
            }
            report(&results);
            let waiting = hidden_asked.iter().any(|&index| results[index].is_none());
            if !waiting || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(timings.poll).await;
        }
    }

    for &index in &rest {
        if results[index].is_some() {
            continue;
        }
        let target = &targets[index];
        let alive = alive_of(pc, target);
        if alive.is_empty() {
            results[index] = Some(Result::Closed);
            continue;
        }
        // The last look before the end: a window that showed up since is a question for the user.
        if !windows_of(&pc.windows(), &alive, true).is_empty() {
            results[index] = Some(Result::StillOpen);
            continue;
        }
        let mut denied = false;
        for process in children_first(&alive) {
            denied |= pc.terminate(process.pid, process.created) == Terminated::Denied;
        }
        results[index] = Some(if denied {
            Result::Denied
        } else {
            Result::Ended
        });
        report(&results);
    }

    targets
        .iter()
        .zip(results)
        .map(|(target, result)| Outcome {
            id: target.id.clone(),
            name: target.name.clone(),
            result: result.unwrap_or(Result::StillOpen),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// What a window does when it is asked to close.
    #[derive(Clone, Copy, PartialEq)]
    enum OnClose {
        /// The whole process ends.
        Exits,
        /// The window goes away but the process stays (a tray program).
        Hides,
        /// Nothing (the program asks the user something).
        Ignores,
    }

    #[derive(Clone)]
    struct FakeWindow {
        hwnd: isize,
        pid: u32,
        visible: bool,
        on_close: OnClose,
    }

    #[derive(Default)]
    struct World {
        /// `(pid, created, alive)`.
        procs: Vec<(u32, u64, bool)>,
        windows: Vec<FakeWindow>,
        /// Processes that Windows refuses to end.
        protected: HashSet<u32>,
        journal: Vec<String>,
    }

    #[derive(Default)]
    struct FakePc {
        world: Mutex<World>,
        reads: AtomicU32,
    }

    impl FakePc {
        fn with(procs: &[u32], windows: Vec<FakeWindow>) -> Self {
            let pc = Self::default();
            {
                let mut world = pc.world.lock().unwrap();
                world.procs = procs
                    .iter()
                    .map(|pid| (*pid, u64::from(*pid) * 10, true))
                    .collect();
                world.windows = windows;
            }
            pc
        }

        fn journal(&self) -> Vec<String> {
            self.world.lock().unwrap().journal.clone()
        }

        fn protect(&self, pid: u32) {
            self.world.lock().unwrap().protected.insert(pid);
        }
    }

    impl Pc for FakePc {
        fn scan(&self) -> super::super::pc::Scan {
            super::super::pc::Scan::default()
        }

        fn windows(&self) -> Vec<TopWindow> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let world = self.world.lock().unwrap();
            world
                .windows
                .iter()
                .filter(|w| {
                    world
                        .procs
                        .iter()
                        .any(|(pid, _, alive)| *pid == w.pid && *alive)
                })
                .map(|w| TopWindow {
                    hwnd: w.hwnd,
                    pid: w.pid,
                    visible: w.visible,
                })
                .collect()
        }

        fn post_close(&self, hwnd: isize) -> bool {
            let mut world = self.world.lock().unwrap();
            let Some(window) = world.windows.iter().find(|w| w.hwnd == hwnd).cloned() else {
                return false;
            };
            world.journal.push(format!(
                "close:{}:{}",
                window.pid,
                if window.visible { "visible" } else { "hidden" }
            ));
            match window.on_close {
                OnClose::Exits => {
                    for proc in world.procs.iter_mut().filter(|p| p.0 == window.pid) {
                        proc.2 = false;
                    }
                }
                OnClose::Hides => {
                    for w in world.windows.iter_mut().filter(|w| w.hwnd == hwnd) {
                        w.visible = false;
                    }
                }
                OnClose::Ignores => {}
            }
            true
        }

        fn alive(&self, pid: u32, created: u64) -> bool {
            let world = self.world.lock().unwrap();
            world
                .procs
                .iter()
                .any(|(p, c, alive)| *p == pid && *c == created && *alive)
        }

        fn terminate(&self, pid: u32, _created: u64) -> Terminated {
            let mut world = self.world.lock().unwrap();
            world.journal.push(format!("terminate:{pid}"));
            if world.protected.contains(&pid) {
                return Terminated::Denied;
            }
            for proc in world.procs.iter_mut().filter(|p| p.0 == pid) {
                proc.2 = false;
            }
            Terminated::Ended
        }
    }

    fn quick() -> CloseTimings {
        CloseTimings {
            polite_wait: Duration::from_millis(120),
            hidden_wait: Duration::from_millis(60),
            poll: Duration::from_millis(5),
        }
    }

    fn proc(pid: u32, parent: u32) -> ProcId {
        ProcId {
            pid,
            parent_pid: parent,
            created: u64::from(pid) * 10,
        }
    }

    fn target(id: &str, procs: &[ProcId]) -> Target {
        Target {
            id: id.to_owned(),
            name: id.to_owned(),
            procs: procs.to_vec(),
        }
    }

    fn window(hwnd: isize, pid: u32, visible: bool, on_close: OnClose) -> FakeWindow {
        FakeWindow {
            hwnd,
            pid,
            visible,
            on_close,
        }
    }

    async fn run(pc: &FakePc, targets: &[Target]) -> Vec<Result> {
        close_all(pc, targets, &quick(), &|_| {})
            .await
            .into_iter()
            .map(|o| o.result)
            .collect()
    }

    #[tokio::test]
    async fn a_window_that_closes_when_asked_is_closed_and_nothing_is_ended() {
        let pc = FakePc::with(&[10], vec![window(1, 10, true, OnClose::Exits)]);
        let results = run(&pc, &[target("a", &[proc(10, 1)])]).await;
        assert_eq!(results, [Result::Closed]);
        assert_eq!(pc.journal(), ["close:10:visible"]);
    }

    #[tokio::test]
    async fn a_program_that_keeps_its_window_is_left_alone_and_never_ended() {
        let pc = FakePc::with(&[10, 11], vec![window(1, 10, true, OnClose::Ignores)]);
        let results = run(&pc, &[target("a", &[proc(10, 1), proc(11, 10)])]).await;
        assert_eq!(results, [Result::StillOpen]);
        assert_eq!(
            pc.journal(),
            ["close:10:visible"],
            "asked once, never ended"
        );
    }

    #[tokio::test]
    async fn a_program_that_goes_to_the_tray_gets_one_hidden_attempt_and_then_the_end() {
        let pc = FakePc::with(
            &[10, 11],
            vec![
                window(1, 10, true, OnClose::Hides),
                window(2, 10, false, OnClose::Ignores),
            ],
        );
        let results = run(&pc, &[target("a", &[proc(10, 1), proc(11, 10)])]).await;
        assert_eq!(results, [Result::Ended]);
        assert_eq!(
            pc.journal(),
            [
                "close:10:visible",
                // The window that hid itself and the one that was hidden all along.
                "close:10:hidden",
                "close:10:hidden",
                "terminate:11", // the child first
                "terminate:10",
            ]
        );
    }

    #[tokio::test]
    async fn a_program_without_a_window_is_asked_through_its_hidden_windows_then_ended() {
        let pc = FakePc::with(&[20], vec![window(5, 20, false, OnClose::Ignores)]);
        let results = run(&pc, &[target("tray", &[proc(20, 1)])]).await;
        assert_eq!(results, [Result::Ended]);
        assert_eq!(pc.journal(), ["close:20:hidden", "terminate:20"]);
    }

    #[tokio::test]
    async fn a_hidden_window_that_closes_the_program_spares_the_terminate() {
        let pc = FakePc::with(&[20], vec![window(5, 20, false, OnClose::Exits)]);
        let results = run(&pc, &[target("tray", &[proc(20, 1)])]).await;
        assert_eq!(results, [Result::Closed]);
        assert_eq!(pc.journal(), ["close:20:hidden"]);
    }

    #[tokio::test]
    async fn a_program_with_no_window_at_all_is_ended_at_once() {
        let pc = FakePc::with(&[30, 31], vec![]);
        let results = run(&pc, &[target("quiet", &[proc(30, 1), proc(31, 30)])]).await;
        assert_eq!(results, [Result::Ended]);
        assert_eq!(pc.journal(), ["terminate:31", "terminate:30"]);
    }

    #[tokio::test]
    async fn a_refused_process_is_reported_once_and_never_retried() {
        let pc = FakePc::with(&[40], vec![]);
        pc.protect(40);
        let results = run(&pc, &[target("admin", &[proc(40, 1)])]).await;
        assert_eq!(results, [Result::Denied]);
        assert_eq!(pc.journal(), ["terminate:40"]);
    }

    #[tokio::test]
    async fn every_polite_ask_comes_before_the_first_end_and_other_programs_are_not_held_up() {
        let pc = FakePc::with(
            &[10, 20, 30],
            vec![
                window(1, 10, true, OnClose::Exits),
                window(2, 30, true, OnClose::Ignores),
            ],
        );
        let targets = [
            target("window", &[proc(10, 1)]),
            target("quiet", &[proc(20, 1)]),
            target("asks", &[proc(30, 1)]),
        ];
        let results = run(&pc, &targets).await;
        assert_eq!(results, [Result::Closed, Result::Ended, Result::StillOpen]);
        let journal = pc.journal();
        let first_end = journal
            .iter()
            .position(|e| e.starts_with("terminate"))
            .unwrap();
        let last_ask = journal
            .iter()
            .rposition(|e| e.starts_with("close"))
            .unwrap();
        assert!(last_ask < first_end, "{journal:?}");
        assert_eq!(
            journal
                .iter()
                .filter(|e| e.starts_with("terminate"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn the_windows_of_other_programs_are_never_asked() {
        let pc = FakePc::with(
            &[10, 99],
            vec![
                window(1, 10, true, OnClose::Exits),
                window(2, 99, true, OnClose::Exits),
            ],
        );
        run(&pc, &[target("a", &[proc(10, 1)])]).await;
        assert_eq!(pc.journal(), ["close:10:visible"]);
        assert!(pc.alive(99, 990));
    }

    #[tokio::test]
    async fn progress_counts_the_finished_programs() {
        let pc = FakePc::with(&[10, 20], vec![window(1, 10, true, OnClose::Exits)]);
        let seen = Mutex::new(Vec::new());
        let targets = [target("a", &[proc(10, 1)]), target("b", &[proc(20, 1)])];
        close_all(&pc, &targets, &quick(), &|done| {
            seen.lock().unwrap().push(done)
        })
        .await;
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.last(), Some(&2));
        assert!(seen.windows(2).all(|pair| pair[0] <= pair[1]), "{seen:?}");
    }

    #[test]
    fn children_are_ended_before_the_process_that_started_them() {
        let order: Vec<u32> = children_first(&[proc(1, 0), proc(2, 1), proc(3, 2), proc(4, 1)])
            .iter()
            .map(|p| p.pid)
            .collect();
        assert_eq!(order[0], 3);
        assert_eq!(*order.last().unwrap(), 1);
        // A parent outside the program does not count, and a loop does not hang.
        assert_eq!(children_first(&[proc(7, 99)]).len(), 1);
        assert_eq!(children_first(&[proc(5, 6), proc(6, 5)]).len(), 2);
    }

    /// The one test that ends real processes. It ends ONLY a child that this test started itself
    /// (a hidden `cmd` and the `ping` it runs), found by the process ids that it was given.
    #[tokio::test]
    async fn a_real_child_that_the_test_started_is_ended_and_nothing_else() {
        use super::super::pc::SystemPc;
        use crate::platform::process::{creation_time, hidden_command, list_processes};

        let mut child = {
            let mut command = hidden_command("cmd.exe");
            command.args(["/c", "ping -n 60 127.0.0.1 >nul"]);
            command.spawn().unwrap()
        };
        let parent = child.id().unwrap();
        // The ping that cmd starts: look for it for a moment.
        let mut grandchild = None;
        for _ in 0..40 {
            grandchild = list_processes()
                .into_iter()
                .find(|p| p.parent_pid == parent);
            if grandchild.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let mut procs = vec![ProcId {
            pid: parent,
            parent_pid: std::process::id(),
            created: creation_time(parent).unwrap(),
        }];
        if let Some(grandchild) = &grandchild {
            procs.push(ProcId {
                pid: grandchild.pid,
                parent_pid: parent,
                created: creation_time(grandchild.pid).unwrap(),
            });
        }
        let targets = [Target {
            id: "test-child".into(),
            name: "test child".into(),
            procs: procs.clone(),
        }];
        let pc = SystemPc;
        let timings = CloseTimings {
            polite_wait: Duration::from_millis(200),
            hidden_wait: Duration::from_millis(200),
            poll: Duration::from_millis(20),
        };
        let outcomes = close_all(&pc, &targets, &timings, &|_| {}).await;
        assert_eq!(outcomes[0].result, Result::Ended);
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("the child ended")
            .unwrap();
        for process in &procs {
            for _ in 0..40 {
                if !pc.alive(process.pid, process.created) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert!(
                !pc.alive(process.pid, process.created),
                "{} ended",
                process.pid
            );
        }
        assert!(pc.alive(
            std::process::id(),
            creation_time(std::process::id()).unwrap()
        ));
    }
}
