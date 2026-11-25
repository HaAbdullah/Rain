//! The scheduled tasks of the Claude desktop app, read only. Feature section 03.
//!
//! The app keeps its task list in `scheduled-tasks.json` and one small file per session in its
//! `claude-code-sessions` folder (see `ClaudePaths::app_session_folders`). A session that a task
//! started has the task's id and Claude's own one-line summary of how it ended. Rain never starts
//! these tasks and never writes there: they need the app's own browser.
//!
//! The format is the app's and may change. Every field is optional. A task without an id or
//! without an `enabled` value is left out (Rain could not describe it truthfully), a session file
//! that does not parse is skipped, and a missing folder gives an empty list. Session files are
//! parsed again only when their size or time changed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use rain_types::time::iso_from_system_time;
use rain_types::{JobKind, JobRun, JobSource, ScheduledJob, Status};
use serde::Deserialize;
use serde_json::Value;

use super::schedule;
use crate::modules::sessions::claude_paths::ClaudePaths;

/// The prefix of the ids of these entries, so that they never clash with an id of `jobs.toml`.
pub const ID_PREFIX: &str = "claude-app:";
/// The most runs an entry shows, as for Rain's own jobs.
const RUNS_MAX: usize = 20;
/// Session files not changed for this long are not read: they are no recent run of any task.
const SESSION_MAX_AGE: Duration = Duration::from_secs(60 * 24 * 3600);
/// A session file larger than this is not read: it is no small status file of the app.
const SESSION_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// A session without a summary that was active this recently is taken as still running.
const RUNNING_WINDOW: Duration = Duration::from_secs(30 * 60);
/// The longest outcome line shown.
const OUTCOME_MAX: usize = 160;

/// Reads the app's files. Holds only a cache of parsed session files.
pub struct Reader {
    paths: ClaudePaths,
    cache: Mutex<HashMap<PathBuf, Cached>>,
}

struct Cached {
    modified: SystemTime,
    len: u64,
    brief: Option<Brief>,
}

/// What Rain keeps of a session file that belongs to a task.
#[derive(Debug, Clone)]
struct Brief {
    task_id: String,
    session_id: String,
    started_ms: u64,
    last_edge_ms: Option<u64>,
    last_active_ms: Option<u64>,
    category: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct TasksFile {
    scheduled_tasks: Vec<Task>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Task {
    id: Option<String>,
    display_name: Option<String>,
    cron_expression: Option<String>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SessionFile {
    scheduled_task_id: Option<String>,
    session_id: Option<String>,
    // Milliseconds since 1970. Kept as plain values: a different type must not lose the file.
    created_at: Option<Value>,
    last_turn_edge_at: Option<Value>,
    last_activity_at: Option<Value>,
    post_turn_summary: Option<Summary>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Summary {
    status_category: Option<String>,
    status_detail: Option<String>,
}

fn millis(value: &Option<Value>) -> Option<u64> {
    let value = value.as_ref()?;
    value
        .as_u64()
        .or_else(|| value.as_f64().filter(|n| *n >= 0.0).map(|n| n as u64))
}

fn time_of(ms: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_millis(ms)
}

impl Reader {
    pub fn new(paths: ClaudePaths) -> Self {
        Self {
            paths,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The entries for the Jobs tab. Does file I/O: call it off the async threads.
    pub fn read(&self, now: SystemTime) -> Vec<ScheduledJob> {
        let folders = self.paths.app_session_folders();
        let tasks = read_tasks(&folders);
        if tasks.is_empty() {
            return Vec::new();
        }
        let briefs = self.read_sessions(&folders, now);
        tasks
            .into_iter()
            .map(|task| entry(task, &briefs, now))
            .collect()
    }

    fn read_sessions(&self, folders: &[PathBuf], now: SystemTime) -> Vec<Brief> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let mut seen = HashSet::new();
        let mut briefs = Vec::new();
        for file in session_files(folders) {
            let Ok(meta) = std::fs::metadata(&file) else {
                continue;
            };
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if now
                .duration_since(modified)
                .is_ok_and(|age| age > SESSION_MAX_AGE)
            {
                continue;
            }
            let fresh = cache
                .get(&file)
                .is_some_and(|c| c.modified == modified && c.len == meta.len());
            if !fresh {
                cache.insert(
                    file.clone(),
                    Cached {
                        modified,
                        len: meta.len(),
                        brief: if meta.len() > SESSION_MAX_BYTES {
                            None
                        } else {
                            parse_session(&file)
                        },
                    },
                );
            }
            if let Some(brief) = cache.get(&file).and_then(|c| c.brief.clone()) {
                briefs.push(brief);
            }
            seen.insert(file);
        }
        cache.retain(|path, _| seen.contains(path));
        briefs
    }
}

/// `scheduled-tasks.json` of every folder, the first copy of a task winning.
fn read_tasks(folders: &[PathBuf]) -> Vec<Task> {
    let mut tasks: Vec<Task> = Vec::new();
    for folder in folders {
        for file in files_below(folder, |name| name == "scheduled-tasks.json") {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let Ok(parsed) = serde_json::from_str::<TasksFile>(&text) else {
                continue;
            };
            for task in parsed.scheduled_tasks {
                let known = task.id.as_deref().is_some_and(|id| !id.is_empty());
                let duplicate = tasks.iter().any(|other| other.id == task.id);
                if known && task.enabled.is_some() && !duplicate {
                    tasks.push(task);
                }
            }
        }
    }
    tasks
}

fn session_files(folders: &[PathBuf]) -> Vec<PathBuf> {
    folders
        .iter()
        .flat_map(|folder| {
            files_below(folder, |name| {
                name.starts_with("local_") && name.ends_with(".json")
            })
        })
        .collect()
}

/// Files two folders deep (`<folder>/<account>/<organisation>/<file>`) whose name passes `keep`.
/// The two levels in between are the app's account and organisation ids; their names do not matter.
fn files_below(folder: &Path, keep: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for first in dirs_in(folder) {
        for second in dirs_in(&first) {
            let Ok(entries) = std::fs::read_dir(&second) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                if entry.path().is_file() && name.to_str().is_some_and(&keep) {
                    found.push(entry.path());
                }
            }
        }
    }
    found
}

fn dirs_in(folder: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_session(file: &Path) -> Option<Brief> {
    let text = std::fs::read_to_string(file).ok()?;
    let parsed: SessionFile = serde_json::from_str(&text).ok()?;
    let task_id = parsed.scheduled_task_id.filter(|id| !id.is_empty())?;
    let started_ms = millis(&parsed.created_at)?;
    let summary = parsed.post_turn_summary.unwrap_or_default();
    Some(Brief {
        task_id,
        session_id: parsed.session_id.filter(|id| !id.is_empty())?,
        started_ms,
        last_edge_ms: millis(&parsed.last_turn_edge_at),
        last_active_ms: millis(&parsed.last_activity_at),
        category: summary.status_category,
        detail: summary.status_detail,
    })
}

/// One list entry from a task and the sessions that belong to it.
fn entry(task: Task, briefs: &[Brief], now: SystemTime) -> ScheduledJob {
    let id = task.id.unwrap_or_default();
    let enabled = task.enabled.unwrap_or(false);
    let cron_text = task.cron_expression.unwrap_or_default();
    let parsed = schedule::parse(&cron_text).ok();

    let mut mine: Vec<&Brief> = briefs.iter().filter(|b| b.task_id == id).collect();
    mine.sort_by_key(|brief| std::cmp::Reverse(brief.started_ms));
    let recent_runs: Vec<JobRun> = mine
        .into_iter()
        .take(RUNS_MAX)
        .map(|brief| run_of(&id, brief, now))
        .collect();

    ScheduledJob {
        source: JobSource::ClaudeApp,
        name: task
            .display_name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| id.clone()),
        id: format!("{ID_PREFIX}{id}"),
        kind: JobKind::Claude,
        schedule_label: if parsed.is_some() {
            schedule::label(&cron_text)
        } else {
            String::new()
        },
        cron: cron_text,
        enabled,
        running: recent_runs
            .first()
            .is_some_and(|run| run.status == Status::Running),
        next_run_at: parsed
            .filter(|_| enabled)
            .and_then(|cron| schedule::next_after(&cron, now))
            .map(iso_from_system_time),
        recent_runs,
    }
}

fn run_of(task_id: &str, brief: &Brief, now: SystemTime) -> JobRun {
    let ended = brief.category.is_some();
    let status = match brief.category.as_deref().map(str::to_ascii_lowercase) {
        Some(c) if c == "completed" => Status::Success,
        // "blocked" means the run stopped and needs the owner: it is a problem, not a success.
        Some(c) if matches!(c.as_str(), "blocked" | "failed" | "error") => Status::Failed,
        Some(c) if matches!(c.as_str(), "working" | "running") => Status::Running,
        Some(_) => Status::Idle,
        None => {
            let recent = brief
                .last_active_ms
                .or(Some(brief.started_ms))
                .map(time_of)
                .is_some_and(|at| {
                    now.duration_since(at)
                        .is_ok_and(|age| age <= RUNNING_WINDOW)
                });
            if recent {
                Status::Running
            } else {
                Status::Idle
            }
        }
    };
    let duration_ms = if ended && status != Status::Running {
        brief
            .last_edge_ms
            .and_then(|end| end.checked_sub(brief.started_ms))
    } else {
        None
    };
    JobRun {
        id: brief.session_id.clone(),
        job_id: format!("{ID_PREFIX}{task_id}"),
        status,
        started_at: iso_from_system_time(time_of(brief.started_ms)),
        duration_ms,
        exit_code: None,
        outcome: brief.detail.as_deref().and_then(one_line),
    }
}

/// The text on one line, cut to a sensible length. `None` for an empty text.
fn one_line(text: &str) -> Option<String> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return None;
    }
    if flat.chars().count() <= OUTCOME_MAX {
        return Some(flat);
    }
    let cut: String = flat.chars().take(OUTCOME_MAX - 1).collect();
    Some(format!("{}...", cut.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outcome_is_one_short_line() {
        assert_eq!(one_line("  a\n b  "), Some("a b".to_owned()));
        assert_eq!(one_line(" \n "), None);
        let long = "word ".repeat(100);
        let cut = one_line(&long).unwrap();
        assert!(cut.chars().count() <= OUTCOME_MAX + 2);
        assert!(cut.ends_with("..."));
    }

    #[test]
    fn times_may_be_numbers_of_either_kind() {
        assert_eq!(millis(&Some(Value::from(5u64))), Some(5));
        assert_eq!(millis(&Some(Value::from(5.0f64))), Some(5));
        assert_eq!(millis(&Some(Value::from("5"))), None);
        assert_eq!(millis(&None), None);
    }
}
