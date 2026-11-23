//! Git facts: the branch from the files, the changed-file count and the commits of 14 days from
//! the `git` program. Section 02 6.3.
//!
//! Why the `git` program and not a Rust git library: a library adds a lot of build time and size
//! for three small questions, and the program answers them the way the user's own tools do.
//! Every call goes through `GitRunner`, so a test gives canned answers and starts no process.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::BoxFuture;

use crate::platform::process::hidden_command;

/// The longest a single git call may take. A longer one is killed.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(20);
/// The most output of one git call that is read. More is a failure.
pub const GIT_OUTPUT_MAX: usize = 4 * 1024 * 1024;

/// One call: `<program> -C <repo> <args...>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCall {
    pub program: PathBuf,
    pub repo: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    /// Only used to recognise one known message; never logged.
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// The program could not be started.
    Spawn(String),
    /// The call took longer than `GIT_TIMEOUT`.
    Timeout,
    /// The call printed more than `GIT_OUTPUT_MAX`.
    TooMuchOutput,
}

/// How git is asked. Production starts the program; tests answer from a table.
pub trait GitRunner: Send + Sync {
    fn run(&self, call: GitCall) -> BoxFuture<'_, Result<GitOutput, GitError>>;
}

/// The real `git` program, started without a window.
pub struct SystemGit;

impl GitRunner for SystemGit {
    fn run(&self, call: GitCall) -> BoxFuture<'_, Result<GitOutput, GitError>> {
        Box::pin(async move {
            let mut command = hidden_command(&call.program);
            command
                .arg("-C")
                .arg(&call.repo)
                .args(&call.args)
                // Never wait for a password, never page the output, never translate it, and
                // never write an index lock: a status must not disturb the user's own git.
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_PAGER", "cat")
                .env("LC_ALL", "C")
                .env("GIT_OPTIONAL_LOCKS", "0")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                // A call that is dropped (timeout, shutdown) must not leave a process behind.
                .kill_on_drop(true);
            let child = command
                .spawn()
                .map_err(|e| GitError::Spawn(e.kind().to_string()))?;
            let output = tokio::time::timeout(GIT_TIMEOUT, child.wait_with_output())
                .await
                .map_err(|_| GitError::Timeout)?
                .map_err(|e| GitError::Spawn(e.kind().to_string()))?;
            if output.stdout.len() > GIT_OUTPUT_MAX {
                return Err(GitError::TooMuchOutput);
            }
            Ok(GitOutput {
                exit_code: output.status.code(),
                stdout: output.stdout,
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            })
        })
    }
}

// ---- the questions ----

/// The arguments (after `-C <repo>`) of the changed-file question.
pub fn status_args() -> Vec<String> {
    [
        "--no-optional-locks",
        "-c",
        "core.quotepath=off",
        "status",
        "--porcelain=v1",
        "-z",
    ]
    .map(String::from)
    .to_vec()
}

/// The arguments of the commit-times question. Author time, because it does not change when a
/// commit is rebased or cherry-picked (committer time does, which would make old work look new).
/// `--branches` counts work on every local branch once and leaves remote noise out. The `--since`
/// only keeps the output small; the exact 14 day window is cut in Rust.
pub fn log_args() -> Vec<String> {
    [
        "-c",
        "core.quotepath=off",
        "log",
        "--branches",
        "--no-merges",
        "--since=16 days ago",
        "--max-count=10000",
        "--format=%at",
    ]
    .map(String::from)
    .to_vec()
}

/// The number of changed files in `git status --porcelain=v1 -z`: one per record. A rename or
/// copy has a second record with the old name, which is not counted again.
pub fn parse_status_z(output: &[u8]) -> u32 {
    let mut count = 0u32;
    let mut records = output
        .split(|&byte| byte == 0)
        .filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        // A record is two status letters, a space, and the path.
        if record.len() < 4 {
            continue;
        }
        count += 1;
        if matches!(record[0], b'R' | b'C') || matches!(record[1], b'R' | b'C') {
            records.next();
        }
    }
    count
}

/// The author times (Unix seconds) of `git log --format=%at`, one per line. Lines that are not a
/// number are ignored.
pub fn parse_log(output: &[u8]) -> Vec<i64> {
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|line| line.trim().parse::<i64>().ok())
        .collect()
}

/// A repository with no commit yet: git exits with 128, and the situation is "no commits", not
/// a failure.
pub fn is_no_commits_yet(output: &GitOutput) -> bool {
    output.exit_code == Some(128) && output.stderr.contains("does not have any commits yet")
}

// ---- the files of a repository ----

/// The folder that holds `HEAD` and the logs. `<repo>\.git` is a folder in a normal repository
/// and a file with a line `gitdir: <path>` in a worktree or a submodule (the path may be
/// relative to the repository).
pub fn git_dir(repo: &Path) -> Option<PathBuf> {
    let dot_git = repo.join(".git");
    let meta = fs::metadata(&dot_git).ok()?;
    if meta.is_dir() {
        return Some(dot_git);
    }
    let text = fs::read_to_string(&dot_git).ok()?;
    let target = text
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))?
        .trim();
    let target = Path::new(target);
    let dir = if target.is_absolute() {
        target.to_path_buf()
    } else {
        repo.join(target)
    };
    dir.is_dir().then_some(dir)
}

/// The branch name from the text of `HEAD`: `ref: refs/heads/<name>` gives the name, another ref
/// gives what follows `refs/`, a detached HEAD (a 40 or 64 digit hash) gives its first 7 digits.
pub fn branch_from_head(text: &str) -> String {
    let text = text.trim();
    if let Some(reference) = text.strip_prefix("ref:") {
        let reference = reference.trim();
        return reference
            .strip_prefix("refs/heads/")
            .or_else(|| reference.strip_prefix("refs/"))
            .unwrap_or(reference)
            .to_owned();
    }
    let is_hash =
        (text.len() == 40 || text.len() == 64) && text.chars().all(|c| c.is_ascii_hexdigit());
    if is_hash {
        text[..7].to_owned()
    } else {
        String::new()
    }
}

/// The branch of a repository, read from its `HEAD` file. Empty when it cannot be read.
pub fn read_branch(repo: &Path) -> String {
    git_dir(repo)
        .and_then(|dir| fs::read_to_string(dir.join("HEAD")).ok())
        .map(|text| branch_from_head(&text))
        .unwrap_or_default()
}

/// What changes when someone commits, checks out, resets, merges or stages, read from files only.
/// It does not see edits of files that are not staged, which is why the changed-file count also
/// has an age limit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HeadSig {
    /// The text of `HEAD`.
    pub head: String,
    /// Time and size of `logs\HEAD`; when there is none, of the ref that HEAD names and of
    /// `packed-refs`.
    pub logs: String,
    /// Time and size of `index`.
    pub index: String,
}

const SEPARATOR: char = '\u{1f}';

impl HeadSig {
    pub fn encode(&self) -> String {
        format!(
            "{}{SEPARATOR}{}{SEPARATOR}{}",
            self.head, self.logs, self.index
        )
    }

    pub fn decode(text: &str) -> Self {
        let mut parts = text.split(SEPARATOR).map(str::to_owned);
        Self {
            head: parts.next().unwrap_or_default(),
            logs: parts.next().unwrap_or_default(),
            index: parts.next().unwrap_or_default(),
        }
    }

    /// The part that decides whether the commit strip is stale.
    pub fn same_history(&self, other: &Self) -> bool {
        self.head == other.head && self.logs == other.logs
    }
}

fn stamp(path: &Path) -> String {
    let Ok(meta) = fs::metadata(path) else {
        return String::new();
    };
    let millis = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_millis());
    format!("{millis}-{}", meta.len())
}

/// The signature of a repository. Files only, so it costs microseconds.
pub fn head_signature(repo: &Path) -> HeadSig {
    let Some(dir) = git_dir(repo) else {
        return HeadSig::default();
    };
    let head = fs::read_to_string(dir.join("HEAD")).unwrap_or_default();
    let head = head.trim().to_owned();
    let mut logs = stamp(&dir.join("logs").join("HEAD"));
    if logs.is_empty() {
        // A repository without a reflog (a tool that never writes one): the ref file and the
        // packed refs change on every commit instead.
        let reference = head
            .strip_prefix("ref:")
            .map(|name| dir.join(name.trim().replace('/', "\\")))
            .map(|path| stamp(&path))
            .unwrap_or_default();
        logs = format!("{reference}|{}", stamp(&dir.join("packed-refs")));
    }
    HeadSig {
        head,
        logs,
        index: stamp(&dir.join("index")),
    }
}

/// A `SystemTime` as Unix seconds.
pub fn unix_seconds(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |since| {
        i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_counts_one_per_record() {
        assert_eq!(parse_status_z(b""), 0);
        assert_eq!(parse_status_z(b" M a.txt\0 M b.txt\0?? c.txt\0"), 3);
        // An untracked folder is one entry.
        assert_eq!(parse_status_z(b"?? build/\0"), 1);
    }

    #[test]
    fn a_rename_or_a_copy_counts_once() {
        assert_eq!(parse_status_z(b"R  new.txt\0old.txt\0 M xyz\0"), 2);
        assert_eq!(parse_status_z(b"C  copy.txt\0orig.txt\0"), 1);
        assert_eq!(parse_status_z(b" R aaa\0bbb\0"), 1);
    }

    #[test]
    fn status_survives_garbage() {
        assert_eq!(parse_status_z(&[0xff, 0xfe, 0, 0, 0x80]), 0);
        assert_eq!(parse_status_z(b"x\0\0\0"), 0);
    }

    #[test]
    fn log_reads_one_time_per_line() {
        assert_eq!(
            parse_log(b"1700000000\n1700000100\n"),
            vec![1_700_000_000, 1_700_000_100]
        );
        assert_eq!(parse_log(b""), Vec::<i64>::new());
        assert_eq!(parse_log(b"junk\n42\r\n"), vec![42]);
    }

    #[test]
    fn a_repository_without_commits_is_recognised() {
        let output = GitOutput {
            exit_code: Some(128),
            stdout: Vec::new(),
            stderr: "fatal: your current branch 'main' does not have any commits yet".into(),
        };
        assert!(is_no_commits_yet(&output));
        let other = GitOutput {
            stderr: "fatal: detected dubious ownership".into(),
            ..output
        };
        assert!(!is_no_commits_yet(&other));
    }

    #[test]
    fn branch_names_come_from_the_text_of_head() {
        assert_eq!(branch_from_head("ref: refs/heads/main\n"), "main");
        assert_eq!(branch_from_head("ref: refs/heads/feature/x"), "feature/x");
        assert_eq!(
            branch_from_head("ref: refs/remotes/origin/dev"),
            "remotes/origin/dev"
        );
        assert_eq!(branch_from_head(&"a1".repeat(20)), "a1a1a1a");
        assert_eq!(branch_from_head(&"b2".repeat(32)), "b2b2b2b");
        assert_eq!(branch_from_head("garbage"), "");
        assert_eq!(branch_from_head(""), "");
    }

    fn repo_with_head(text: &str) -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".git")).unwrap();
        fs::write(temp.path().join(".git").join("HEAD"), text).unwrap();
        temp
    }

    #[test]
    fn the_branch_is_read_from_a_normal_repository() {
        let repo = repo_with_head("ref: refs/heads/main\n");
        assert_eq!(read_branch(repo.path()), "main");
    }

    #[test]
    fn the_branch_follows_a_gitdir_file_relative_and_absolute() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("store").join("wt");
        fs::create_dir_all(&store).unwrap();
        fs::write(store.join("HEAD"), "ref: refs/heads/topic\n").unwrap();

        let relative = temp.path().join("relative");
        fs::create_dir_all(&relative).unwrap();
        fs::write(relative.join(".git"), "gitdir: ../store/wt\n").unwrap();
        assert_eq!(read_branch(&relative), "topic");

        let absolute = temp.path().join("absolute");
        fs::create_dir_all(&absolute).unwrap();
        fs::write(
            absolute.join(".git"),
            format!("gitdir: {}\n", store.display()),
        )
        .unwrap();
        assert_eq!(read_branch(&absolute), "topic");
    }

    #[test]
    fn a_missing_head_gives_an_empty_branch() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".git")).unwrap();
        assert_eq!(read_branch(temp.path()), "");
        assert_eq!(read_branch(&temp.path().join("nowhere")), "");
    }

    #[test]
    fn the_signature_changes_when_the_reflog_is_touched() {
        let repo = repo_with_head("ref: refs/heads/main\n");
        let logs = repo.path().join(".git").join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("HEAD"), "one").unwrap();
        let before = head_signature(repo.path());
        fs::write(logs.join("HEAD"), "one and two").unwrap();
        let after = head_signature(repo.path());
        assert_ne!(before.logs, after.logs);
        assert!(!before.same_history(&after));
        assert_eq!(before.head, after.head);
    }

    #[test]
    fn without_a_reflog_the_ref_file_stands_in() {
        let repo = repo_with_head("ref: refs/heads/main\n");
        let refs = repo.path().join(".git").join("refs").join("heads");
        fs::create_dir_all(&refs).unwrap();
        fs::write(refs.join("main"), "1234").unwrap();
        let before = head_signature(repo.path());
        fs::write(refs.join("main"), "1234567890").unwrap();
        assert_ne!(before.logs, head_signature(repo.path()).logs);
    }

    #[test]
    fn a_signature_survives_the_database() {
        let sig = HeadSig {
            head: "ref: refs/heads/main".into(),
            logs: "1-2".into(),
            index: "3-4".into(),
        };
        assert_eq!(HeadSig::decode(&sig.encode()), sig);
        assert_eq!(HeadSig::decode(""), HeadSig::default());
    }
}
