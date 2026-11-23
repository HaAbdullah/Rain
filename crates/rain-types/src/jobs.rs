//! Scheduled jobs, idle jobs and the log lines of their runs. Section 03 3.2.
//!
//! The definitions of jobs live in the user's TOML files (`jobs.toml`, `idle-jobs.toml`); these
//! types are what the API shows of them. `LogLine` is also the line type of a Claude session's
//! transcript.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::common::Status;

/// What a scheduled job runs: a command line, or a headless Claude run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    Cron,
    Claude,
}

/// One run of a scheduled job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct JobRun {
    // "run-20261121-093000-ab12": the UTC start time and four hex characters.
    pub id: String,
    pub job_id: String,
    // `idle` means "stopped on purpose" (game mode, or Rain stopping): it is not a failure.
    #[ts(inline)]
    pub status: Status,
    pub started_at: String,
    // Absent only while the run is going.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    // The real exit code of the program. Absent while running and when no code exists (a time
    // limit, a deliberate stop, a program that could not start): the log says why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub ts: String,
    #[ts(inline)]
    pub level: LogLevel,
    pub text: String,
}

/// A scheduled job as `GET /jobs` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ScheduledJob {
    pub id: String,
    pub name: String,
    #[ts(inline)]
    pub kind: JobKind,
    // The five-field text of jobs.toml, shown as it is.
    pub cron: String,
    // "Every day at 06:00": the `label` of the file, or one written from the schedule.
    pub schedule_label: String,
    pub enabled: bool,
    // True exactly when `recent_runs[0].status` is `running`.
    pub running: bool,
    // Absent when the job is disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
    // Newest first, at most 20.
    pub recent_runs: Vec<JobRun>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum IdleState {
    Queued,
    Running,
    Paused,
    Success,
    Failed,
}

/// What an idle job loads. At most one running idle job per resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum IdleResource {
    Cpu,
    Gpu,
}

/// An idle job as `GET /idle-jobs` lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct IdleJob {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[ts(inline)]
    pub state: IdleState,
    // 0.0 to 1.0.
    pub progress: f64,
    #[ts(inline)]
    pub resource: IdleResource,
    pub idle_after_minutes: u32,
    // Only while running, and only when the job reports it or `expected_minutes` gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_minutes: Option<u32>,
    // The end of the latest run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
}

/// The body of `PATCH /jobs/{id}` and `PATCH /idle-jobs/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnabledBody {
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::round_trip;
    use serde_json::json;

    fn run(status: Status) -> JobRun {
        JobRun {
            id: "run-20261121-093000-ab12".into(),
            job_id: "backup-notes".into(),
            status,
            started_at: "2026-11-21T09:30:00.000Z".into(),
            duration_ms: None,
            exit_code: None,
        }
    }

    #[test]
    fn a_running_run_has_no_duration_and_no_exit_code() {
        assert_eq!(
            round_trip(&run(Status::Running)),
            json!({
                "id": "run-20261121-093000-ab12",
                "jobId": "backup-notes",
                "status": "running",
                "startedAt": "2026-11-21T09:30:00.000Z"
            })
        );
    }

    #[test]
    fn a_finished_run_carries_its_duration_and_exit_code() {
        let mut finished = run(Status::Failed);
        finished.duration_ms = Some(1200);
        finished.exit_code = Some(3);
        let json = round_trip(&finished);
        assert_eq!(json["durationMs"], 1200);
        assert_eq!(json["exitCode"], 3);
    }

    #[test]
    fn a_scheduled_job_leaves_out_a_missing_next_run() {
        let job = ScheduledJob {
            id: "j".into(),
            name: "Job".into(),
            kind: JobKind::Claude,
            cron: "0 6 * * *".into(),
            schedule_label: "Every day at 06:00".into(),
            enabled: false,
            running: false,
            next_run_at: None,
            recent_runs: vec![],
        };
        let json = round_trip(&job);
        assert!(json.get("nextRunAt").is_none());
        assert_eq!(json["kind"], "claude");
        assert_eq!(json["scheduleLabel"], "Every day at 06:00");
    }

    #[test]
    fn an_idle_job_uses_the_names_of_the_screen() {
        let job = IdleJob {
            id: "idle-a".into(),
            name: "A".into(),
            enabled: true,
            state: IdleState::Paused,
            progress: 0.5,
            resource: IdleResource::Gpu,
            idle_after_minutes: 10,
            eta_minutes: None,
            last_run_at: None,
        };
        assert_eq!(
            round_trip(&job),
            json!({
                "id": "idle-a", "name": "A", "enabled": true, "state": "paused",
                "progress": 0.5, "resource": "gpu", "idleAfterMinutes": 10
            })
        );
    }

    #[test]
    fn the_enabled_body_refuses_other_keys() {
        assert!(serde_json::from_value::<EnabledBody>(json!({ "enabled": true })).is_ok());
        assert!(
            serde_json::from_value::<EnabledBody>(json!({ "enabled": true, "name": "x" })).is_err()
        );
        assert!(serde_json::from_value::<EnabledBody>(json!({})).is_err());
    }

    #[test]
    fn log_levels_use_the_frontend_names() {
        for (level, name) in [
            (LogLevel::Info, "info"),
            (LogLevel::Warn, "warn"),
            (LogLevel::Error, "error"),
        ] {
            assert_eq!(serde_json::to_value(level).unwrap(), json!(name));
        }
    }
}
