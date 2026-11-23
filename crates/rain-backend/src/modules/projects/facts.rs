//! When a git fact is stale, and how it is computed. Section 02 6.3.
//!
//! Nothing here runs on a timer. A fact is checked when a client asks for the list: the
//! signature of the repository's files (`git::HeadSig`) says whether something happened, and an
//! age limit covers what the files cannot show (edits of files that are not staged).

use std::path::Path;

use super::dates::{self, STRIP_DAYS};
use super::git::{
    GitCall, GitRunner, HeadSig, is_no_commits_yet, log_args, parse_log, parse_status_z,
    status_args,
};
use super::store::GitRow;

/// How old the changed-file count may be before it is asked again, in milliseconds.
pub const DIRTY_TTL_MS: i64 = 60_000;

/// What to do about the commit strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commits {
    /// It is right.
    Keep,
    /// It is right except that days passed: shift it left, no git call.
    Shift(i64),
    /// Ask git.
    Recompute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Needed {
    pub dirty: bool,
    pub commits: Commits,
}

impl Needed {
    pub fn nothing(self) -> bool {
        !self.dirty && self.commits == Commits::Keep
    }
}

/// Which facts of a project must be computed again.
///
/// - `row`: the cached facts, if any. Without a row everything is needed.
/// - `sig`: the signature of the repository's files now.
/// - `now_ms`: the time now. `today`: the local day number of now.
/// - `force`: a refresh was asked for: the age gates do not apply.
///
/// A fact that is absent in the row (git failed last time) is tried again only after
/// `DIRTY_TTL_MS`, so that a repository that git refuses does not cost a process on every list.
pub fn needs(row: Option<&GitRow>, sig: &HeadSig, now_ms: i64, today: i64, force: bool) -> Needed {
    let Some(row) = row else {
        return Needed {
            dirty: true,
            commits: Commits::Recompute,
        };
    };
    let old = HeadSig::decode(&row.head_sig);
    let attempt_age = age_ms(now_ms, &row.computed_at);
    let may_retry = force || attempt_age.is_none_or(|age| age >= DIRTY_TTL_MS);

    let dirty_age = row.dirty_at.as_deref().and_then(|at| age_ms(now_ms, at));
    let dirty_is_old = dirty_age.is_none_or(|age| age >= DIRTY_TTL_MS);
    let dirty = old != *sig || (dirty_is_old && may_retry);

    let commits = if !old.same_history(sig) {
        Commits::Recompute
    } else if row.commits.is_none() {
        if may_retry {
            Commits::Recompute
        } else {
            Commits::Keep
        }
    } else {
        match row.commits_day.as_deref().and_then(dates::parse_date) {
            Some(day) if day == today => Commits::Keep,
            Some(day) if day < today => Commits::Shift(today - day),
            _ => Commits::Recompute,
        }
    };
    Needed { dirty, commits }
}

fn age_ms(now_ms: i64, at: &str) -> Option<i64> {
    dates::iso_to_unix_ms(at).map(|then| now_ms - then)
}

/// Everything a computation needs besides git itself.
pub struct Moment<'a> {
    pub now_iso: &'a str,
    /// The local date of now, `YYYY-MM-DD`.
    pub today_date: &'a str,
    /// The local day number of now.
    pub today: i64,
    /// The UTC offset in seconds at a Unix time.
    pub offset_at: &'a (dyn Fn(i64) -> i64 + Sync),
}

/// The repository that is computed, with what is known about it.
#[derive(Clone, Copy)]
pub struct Target<'a> {
    pub repo: &'a Path,
    pub key: &'a str,
    pub old: Option<&'a GitRow>,
    pub sig: &'a HeadSig,
    pub branch: &'a str,
}

/// Computes the needed facts of one repository and returns its new row. A fact whose git call
/// fails keeps its old value (absent when there was none); the failure is logged by the caller
/// through the returned flag.
pub async fn compute(
    runner: &dyn GitRunner,
    git: &Path,
    target: &Target<'_>,
    needed: Needed,
    moment: &Moment<'_>,
) -> (GitRow, bool) {
    let Target {
        repo,
        key,
        old,
        sig,
        branch,
    } = *target;
    let mut row = old.cloned().unwrap_or_else(|| GitRow {
        key: key.to_owned(),
        head_sig: String::new(),
        branch: String::new(),
        dirty: None,
        dirty_at: None,
        commits: None,
        commits_day: None,
        computed_at: moment.now_iso.to_owned(),
    });
    let mut failed = false;

    if needed.dirty {
        let call = GitCall {
            program: git.to_path_buf(),
            repo: repo.to_path_buf(),
            args: status_args(),
        };
        match runner.run(call).await {
            Ok(output) if output.exit_code == Some(0) => {
                row.dirty = Some(parse_status_z(&output.stdout));
                row.dirty_at = Some(moment.now_iso.to_owned());
            }
            _ => failed = true,
        }
    }

    match needed.commits {
        Commits::Keep => {}
        Commits::Shift(days) => {
            if let Some(counts) = &row.commits {
                row.commits = Some(dates::shift(counts, days));
                row.commits_day = Some(moment.today_date.to_owned());
            }
        }
        Commits::Recompute => {
            let call = GitCall {
                program: git.to_path_buf(),
                repo: repo.to_path_buf(),
                args: log_args(),
            };
            match runner.run(call).await {
                Ok(output) if output.exit_code == Some(0) => {
                    let times = parse_log(&output.stdout);
                    row.commits = Some(dates::bucket(&times, moment.today, moment.offset_at));
                    row.commits_day = Some(moment.today_date.to_owned());
                }
                // A repository without a commit has nothing to count.
                Ok(output) if is_no_commits_yet(&output) => {
                    row.commits = Some(vec![0; STRIP_DAYS]);
                    row.commits_day = Some(moment.today_date.to_owned());
                }
                _ => failed = true,
            }
        }
    }

    row.head_sig = sig.encode();
    row.branch = branch.to_owned();
    row.computed_at = moment.now_iso.to_owned();
    (row, failed)
}

#[cfg(test)]
pub mod fake {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use futures_util::future::BoxFuture;

    use super::super::git::{GitCall, GitError, GitOutput, GitRunner};

    /// Canned answers by sub-command (`status` or `log`), and a record of the calls.
    #[derive(Default)]
    pub struct FakeGit {
        pub answers: Mutex<HashMap<&'static str, Result<GitOutput, GitError>>>,
        pub calls: Mutex<Vec<(String, &'static str)>>,
    }

    pub fn ok(stdout: &str) -> Result<GitOutput, GitError> {
        Ok(GitOutput {
            exit_code: Some(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: String::new(),
        })
    }

    impl FakeGit {
        pub fn answer(&self, sub: &'static str, result: Result<GitOutput, GitError>) {
            self.answers.lock().unwrap().insert(sub, result);
        }

        pub fn count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl GitRunner for FakeGit {
        fn run(&self, call: GitCall) -> BoxFuture<'_, Result<GitOutput, GitError>> {
            let sub = if call.args.iter().any(|a| a == "status") {
                "status"
            } else {
                "log"
            };
            self.calls
                .lock()
                .unwrap()
                .push((call.repo.to_string_lossy().into_owned(), sub));
            let answer = self
                .answers
                .lock()
                .unwrap()
                .get(sub)
                .cloned()
                .unwrap_or_else(|| ok(""));
            Box::pin(std::future::ready(answer))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeGit, ok};
    use super::*;
    use crate::modules::projects::git::GitOutput;
    use std::path::PathBuf;

    const NOW: &str = "2026-11-10T12:00:00.000Z";
    const NOW_MS: i64 = 1_794_312_000_000; // 2026-11-10T12:00:00Z

    fn today() -> i64 {
        dates::parse_date("2026-11-10").unwrap()
    }

    fn sig(log: &str) -> HeadSig {
        HeadSig {
            head: "ref: refs/heads/main".into(),
            logs: log.into(),
            index: "i".into(),
        }
    }

    fn row(sig: &HeadSig, dirty_at: Option<&str>, day: Option<&str>) -> GitRow {
        GitRow {
            key: "k".into(),
            head_sig: sig.encode(),
            branch: "main".into(),
            dirty: Some(1),
            dirty_at: dirty_at.map(str::to_owned),
            commits: Some(vec![0; 14]),
            commits_day: day.map(str::to_owned),
            computed_at: dirty_at.unwrap_or(NOW).to_owned(),
        }
    }

    #[test]
    fn the_test_time_is_what_it_says() {
        assert_eq!(dates::iso_to_unix_ms(NOW), Some(NOW_MS));
    }

    #[test]
    fn a_new_project_needs_everything() {
        let needed = needs(None, &sig("a"), NOW_MS, today(), false);
        assert_eq!(
            needed,
            Needed {
                dirty: true,
                commits: Commits::Recompute
            }
        );
    }

    #[test]
    fn fresh_facts_with_the_same_files_need_nothing() {
        let s = sig("a");
        let r = row(&s, Some("2026-11-10T11:59:30.000Z"), Some("2026-11-10"));
        assert!(needs(Some(&r), &s, NOW_MS, today(), false).nothing());
    }

    #[test]
    fn the_changed_file_count_gets_old_after_a_minute() {
        let s = sig("a");
        let r = row(&s, Some("2026-11-10T11:58:00.000Z"), Some("2026-11-10"));
        let needed = needs(Some(&r), &s, NOW_MS, today(), false);
        assert!(needed.dirty);
        assert_eq!(needed.commits, Commits::Keep);
    }

    #[test]
    fn a_touched_index_asks_for_the_count_only_and_a_touched_log_for_both() {
        let s = sig("a");
        let r = row(&s, Some("2026-11-10T11:59:50.000Z"), Some("2026-11-10"));
        let other_index = HeadSig {
            index: "j".into(),
            ..s.clone()
        };
        let needed = needs(Some(&r), &other_index, NOW_MS, today(), false);
        assert_eq!((needed.dirty, needed.commits), (true, Commits::Keep));
        let needed = needs(Some(&r), &sig("b"), NOW_MS, today(), false);
        assert_eq!((needed.dirty, needed.commits), (true, Commits::Recompute));
    }

    #[test]
    fn a_new_day_with_the_same_history_only_shifts_the_strip() {
        let s = sig("a");
        let r = row(&s, Some("2026-11-10T11:59:50.000Z"), Some("2026-11-08"));
        let needed = needs(Some(&r), &s, NOW_MS, today(), false);
        assert_eq!(needed.commits, Commits::Shift(2));
        assert!(!needed.dirty);
    }

    #[test]
    fn a_fact_that_failed_is_tried_again_only_after_the_age_limit() {
        let s = sig("a");
        let mut r = row(&s, None, None);
        r.dirty = None;
        r.commits = None;
        r.computed_at = "2026-11-10T11:59:40.000Z".into();
        let soon = needs(Some(&r), &s, NOW_MS, today(), false);
        assert!(soon.nothing(), "{soon:?}");
        r.computed_at = "2026-11-10T11:58:00.000Z".into();
        let later = needs(Some(&r), &s, NOW_MS, today(), false);
        assert_eq!((later.dirty, later.commits), (true, Commits::Recompute));
        // A refresh ignores the gate.
        r.computed_at = "2026-11-10T11:59:59.000Z".into();
        let forced = needs(Some(&r), &s, NOW_MS, today(), true);
        assert_eq!((forced.dirty, forced.commits), (true, Commits::Recompute));
    }

    fn moment() -> Moment<'static> {
        Moment {
            now_iso: NOW,
            today_date: "2026-11-10",
            today: 0,
            offset_at: &|_| 0,
        }
    }

    async fn run(
        git: &FakeGit,
        key: &str,
        old: Option<&GitRow>,
        sig: &HeadSig,
        needed: Needed,
        moment: &Moment<'_>,
    ) -> (GitRow, bool) {
        let target = Target {
            repo: &PathBuf::from("D:\\w\\a"),
            key,
            old,
            sig,
            branch: "main",
        };
        compute(git, &PathBuf::from("git.exe"), &target, needed, moment).await
    }

    fn moment_today() -> (i64, i64) {
        (today(), NOW_MS / 1000)
    }

    #[tokio::test]
    async fn compute_reads_status_and_log() {
        let git = FakeGit::default();
        git.answer("status", ok(" M a.txt\0?? b/\0"));
        let (day, now_s) = moment_today();
        let times = format!("{}\n{}\n{}\n", now_s, now_s - 86_400, now_s - 20 * 86_400);
        git.answer("log", ok(&times));
        let m = Moment {
            today: day,
            ..moment()
        };
        let needed = Needed {
            dirty: true,
            commits: Commits::Recompute,
        };
        let (row, failed) = run(&git, "k", None, &sig("a"), needed, &m).await;
        assert!(!failed);
        assert_eq!(row.dirty, Some(2));
        assert_eq!(row.dirty_at.as_deref(), Some(NOW));
        let commits = row.commits.unwrap();
        assert_eq!((commits[13], commits[12]), (1, 1));
        assert_eq!(commits.iter().sum::<u32>(), 2);
        assert_eq!(row.commits_day.as_deref(), Some("2026-11-10"));
        assert_eq!(git.count(), 2);
    }

    #[tokio::test]
    async fn a_repository_without_commits_has_a_flat_strip_and_is_not_a_failure() {
        let git = FakeGit::default();
        git.answer(
            "log",
            Ok(GitOutput {
                exit_code: Some(128),
                stdout: Vec::new(),
                stderr: "fatal: your current branch 'main' does not have any commits yet".into(),
            }),
        );
        let needed = Needed {
            dirty: false,
            commits: Commits::Recompute,
        };
        let (row, failed) = run(&git, "k", None, &sig("a"), needed, &moment()).await;
        assert!(!failed);
        assert_eq!(row.commits, Some(vec![0; 14]));
        assert_eq!(row.dirty, None);
    }

    #[tokio::test]
    async fn a_failed_call_keeps_the_old_value_and_reports_it() {
        let git = FakeGit::default();
        git.answer(
            "status",
            Ok(GitOutput {
                exit_code: Some(128),
                stdout: Vec::new(),
                stderr: "fatal: detected dubious ownership".into(),
            }),
        );
        let s = sig("a");
        let old = row(&s, Some("2026-11-10T10:00:00.000Z"), Some("2026-11-10"));
        let needed = Needed {
            dirty: true,
            commits: Commits::Keep,
        };
        let (row, failed) = run(&git, "k", Some(&old), &s, needed, &moment()).await;
        assert!(failed);
        assert_eq!(row.dirty, Some(1));
        assert_eq!(row.dirty_at.as_deref(), Some("2026-11-10T10:00:00.000Z"));
        assert_eq!(row.computed_at, NOW);
    }

    #[tokio::test]
    async fn a_shift_makes_no_git_call() {
        let git = FakeGit::default();
        let s = sig("a");
        let mut old = row(&s, Some(NOW), Some("2026-11-08"));
        old.commits = Some((1..=14).collect());
        let needed = Needed {
            dirty: false,
            commits: Commits::Shift(2),
        };
        let (row, failed) = run(&git, "k", Some(&old), &s, needed, &moment()).await;
        assert!(!failed);
        assert_eq!(git.count(), 0);
        let counts = row.commits.unwrap();
        assert_eq!(&counts[..3], &[3, 4, 5]);
        assert_eq!(&counts[12..], &[0, 0]);
        assert_eq!(row.commits_day.as_deref(), Some("2026-11-10"));
    }
}
