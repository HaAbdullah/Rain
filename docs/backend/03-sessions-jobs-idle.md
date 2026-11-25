# Rain backend, section 03: Claude sessions, scheduled jobs, idle jobs

| | |
|---|---|
| Area | Claude sessions (stage 8), scheduled jobs and idle jobs (stage 6) |
| Modules | `sessions`, `jobs`, `idle` (under `crates/rain-backend/src/modules/`), plus the new crate `crates/rain-hook` |
| Rust types | `crates/rain-types/src/sessions.rs` and `jobs.rs` |
| Author | feature designer for section 03 |
| Date | 2026-11-22 |
| Status | draft 1. Written on top of `00-foundation.md` draft 2 and `decisions.md`. Where this file needs a different shared rule it says so in section 12. |

Reading guide. An engineer who has read `decisions.md`, `00-foundation.md` and this file has everything needed. Words are used as in the foundation (shell, backend, hook, pipe, module, topic, frame, sandbox, data folder, local folder, stage). Two more words are used with one meaning only:

| Word | Meaning |
|---|---|
| registry | the small files that Claude Code keeps itself, one per running session: `<claude folder>\sessions\<pid>.json` |
| book | the in-memory list of known sessions inside the `sessions` module (`SessionBook`, section 6.3) |

"Proven" and "UNPROVEN" mean what they mean in the foundation (a spike ran it on this PC, or nobody saw it work). Every UNPROVEN item has a defensive rule next to it and a line in section 11 (acceptance) or in the gate of section 6.10.

---------------------------------------------------------------------------------------------------

## 1. Scope

### 1.1 What this section covers

| Part | `RainApi` methods and topics | Stage |
|---|---|---|
| Scheduled jobs | `listJobs`, `setJobEnabled`, `runJob`, `getRunLog`, topic `jobs` | 6 |
| Idle jobs | `listIdleJobs`, `setIdleJobEnabled`, `runIdleJob`, `getIdleJobLog`, topic `idleJobs` | 6 |
| Sessions, read only (8a) | `listSessions`, `getSessionTranscript`, `focusSession`, `stopSession`, topic `sessions` | 8a |
| Permission answers and monitoring (8b) | `answerPermission` (changed), the route `POST /internal/hook`, the program `rain-hook`, the hook installer behind the Settings switch "Monitor Claude sessions" (the switch, its state and its repair belong to section 06: setting `monitorClaude`, state `monitorClaudeState`; this module offers `install_hooks()`, `remove_hooks()` and `hooks_state()`) | 8b |
| Not in this section | `getMachineState` and the `userActive` rule (section 05, stage 3); the Settings type and `PATCH /settings` (section 06); search items for sessions, jobs and idle jobs (section 01, which calls the `Service` methods of 1.3); game mode itself (section 06) | |

Stage 8 is two steps. 8a builds the registry reader, the transcript reader, focus and stop. Nothing in 8a touches any Claude settings file. 8b builds `rain-hook`, the hook route and the installer, and is released to the real PC only through the gate of section 6.10. The app builds and runs after each step: after 8a the session list is real, no session ever has a `request`, and `answerPermission` answers `409 requestExpired`.

### 1.2 What it does not do

- Rain never starts, resumes or prompts an interactive Claude session (ui-spec section 10). Rain starts Claude only for a scheduled Claude job (headless, `claude -p`).
- No UI to create or edit jobs or idle jobs. They are TOML files that the user edits. Rain writes back only `enabled` (decision C7) and an `id` where one is missing (4.1).
- No stop route for a job run and no pause route for an idle job. The UI has none. Stopping runs is internal: game mode and shutdown call the `Service` methods of 1.3.
- No network request of any kind (foundation 13.2).
- No key or credential of Claude is ever read. The `.key` files next to the registry files are never opened. `messagingSocketPath`, `peerProtocol`, `pidDomain` and the account fields of transcripts are never read, stored or logged.

### 1.3 Public `Service` methods other modules may call (foundation 11.2)

| Module | Method | Used by | Meaning |
|---|---|---|---|
| `sessions` | `list() -> Vec<ClaudeSession>` | 01 (search), 06 | the same snapshot as `GET /sessions`, from memory, no I/O |
| `sessions` | `open_user_sessions() -> u32` | 06 (game mode item 1) | the live sessions that are not runs of Rain's own jobs (a job run is never in the session book, 6.2), counted only; game mode never stops one |
| `sessions` | `install_hooks() -> ApiResult<()>` and `remove_hooks() -> ApiResult<()>` | 06 (`PATCH /settings`, `monitorClaude`) | install or remove the hooks (6.9; idempotent; a repair is `install_hooks()` again). Called BEFORE the setting is written; an error aborts the write. The result does not depend on the stored switch value: `install_hooks()` verifies with `status(true)`, `remove_hooks()` with `status(false)` (6.9.6) |
| `sessions` | `hooks_state() -> MonitorClaudeState` | 06 | cheap, for Settings: `ok` with an event received is `on`, `ok` without an event yet is `onInactive`, `off` is `off`, and `broken`, `blocked` and `unavailable` are `needsRepair` (the message is in the `ApiError` of the failed call, or in the Health source `claude-hooks`) |
| `jobs` | `list() -> Vec<ScheduledJob>` | 01 (search) | snapshot from memory |
| `jobs` | `wind_down() -> &dyn WindDownPort` (the trait of `core/winddown.rs`, section 06 6.7.1: `counts()`, `running_watch()`, `pause(bool)`, `stop_all(budget)`) | 06 | `stop_all(budget)` terminates every Rain-owned job run (`TerminateJobObject`, then checks the job is empty inside the budget), marks each run outcome `stopped` (status `idle`, reason `gameMode`), sets `handled_until` back as in 6.11.5 and returns one `Stopped` per run. `pause(true)` is the existing rule "starts nothing and advances nothing in game mode" (6.11.3); `pause(false)` clears it and a job that came due runs once. Call order (section 06 6.7.2): `pause(false)` is called AFTER `game_mode_active` was cleared; it nudges the scheduler (T2), re-creates the idle sampler (T3) and then runs `plan_catch_up` (6.11.5); called before the flag is cleared, the scheduler and the start path would still refuse the catch-up run. `running_watch()` is a `watch` of the number of runs in progress |
| `jobs` | `read_log(run_id) -> ApiResult<Vec<LogLine>>`, the log store's `delete(run_id)` (removes the log rows of one run; the only way idle logs are deleted), `start_child(...)` | `idle` | the shared log store and the shared child runner (6.11.9). `idle` never touches `jobs_*` tables itself |
| `idle` | `list() -> Vec<IdleJob>`; `wind_down() -> &dyn WindDownPort` | 01, 06 | the same trait with the same meaning: `pause(true)` stops the sampler from starting or resuming a run, `stop_all(budget)` ends the running run |

What this module needs from other modules (each is repeated in section 12):

| From | Method | Used for |
|---|---|---|
| `projects` (02) | `project_id_for_path(path: &str) -> Option<String>` (longest folder prefix, case-insensitive, `\` and `/` equal) | `ClaudeSession.projectId` |
| `machine` (05) | `idle_ms() -> Option<u64>` (one `GetLastInputInfo` call, section 11.6 of the foundation); `None` means the user counts as present (no run starts, a running run is frozen) | idle jobs |
| `games` (02) | `any_game_running() -> bool` (async; REQUIRED; section 02 6.15 provides it and computes it at call time with a 2 s cache) | idle jobs treat a running game as "user active", because a game pad does not move the `GetLastInputInfo` clock (UNPROVEN, contract report 4.4) |
| `settings` (06) | `Service::resolved()` and `watch()`; the field `Resolved.monitor_claude` | the hooks verification on start (6.9.7) |
| core | `state.game_mode_active`, `state.activity`, `state.health`, `Clock`, `Db`, `ConfigHub` | as in the foundation |

### 1.4 Decisions of `decisions.md` that this section implements

| Decision | Where |
|---|---|
| C1 status: `waiting` only from an open permission request; the registry is for discovery and liveness; hooks first; degrade when the registry format changes | 6.3 |
| C2 permission hold about 20 s, fail open in every branch, request id in `answerPermission`, an expired request returns an error the UI can show | 6.2, 6.5 |
| C7 `enabled` of jobs written back with `toml_edit` | 4.1 |
| C12 `focusSession` returns `window`, `app` or `none`; `none` is not an error | 6.6 |
| Hooks behind the switch "Monitor Claude sessions"; Rain not running means Claude behaves as before; never the real settings file while developing | 6.2, 6.9, 6.10 |
| "Always allow" is session only, nothing written to any settings file | 6.5 |
| Game mode never touches the user's sessions; it stops Rain-started jobs | 1.3, 6.11, 6.12 |
| A job missed while the PC was off or in game mode runs once at the next start | 6.11.5 |
| First job: a Claude job that runs the user's skill at 06:00, switched off, put in the data folder by a seeding step, never in code | 4.1.4 |
| Idle jobs start empty, documented format, commented example | 4.2 |

---------------------------------------------------------------------------------------------------

## 2. Routes

All paths are after `/api/v1` (foundation 4.1). Errors use the one body of foundation 4.5. "Idem." is the idempotency column that foundation 4.6 requires.

### 2.1 Sessions (module `sessions`)

| Method | Path | Request | Response | OK | Errors (code, reason) | Idem. | Topic effects |
|---|---|---|---|---|---|---|---|
| GET | `/sessions` | none | `ClaudeSession[]` (live sessions plus ended ones of the last 24 h, newest first, at most 40) | 200 | `unavailable` `moduleFailed` | yes | none |
| POST | `/sessions/{id}/permission` | `PermissionAnswerBody` | `PermissionAnswerResult` | 200 | `notFound` `unknownSession`; `conflict` `requestExpired` (no open request with this id, the forwarder already gave up, the session is gone); `conflict` `monitoringOff` (hooks are not installed, so nothing can be answered); `unavailable` `shuttingDown` | no (a second answer for the same request gets `requestExpired`) | `sessions` |
| POST | `/sessions/{id}/focus` | none | `FocusResult` | 200 | `notFound` `unknownSession` | yes (focusing twice is harmless) | none |
| POST | `/sessions/{id}/stop` | none | none | 204 | `notFound` `unknownSession`; `conflict` `notStoppable` (message tells why: desktop host, already ended, process not verified, no process id); `failed` `stopFailed` | yes (an ended session answers `204` too) | `sessions` |
| GET | `/sessions/{id}/transcript` | none | `LogLine[]` (oldest first, at most 1000 lines) | 200 | `notFound` `unknownSession`; `notFound` `noTranscript` (the file is gone or never found) | yes | none |
| POST | `/internal/hook` | `HookEnvelope` (6.2.2), body limit 4 MiB | for `mode: permission` a body that `rain-hook` prints verbatim, or `204` | 200 or 204 | `invalid` `badEnvelope` | no | `sessions` |

Notes:
- `answerPermission` in the contract changes: it carries the request id, and it returns what was applied (S.3, S.4 in 9.2). The UI always sends the id of `session.request` that it shows.
- `/internal/hook` is under `/internal/`, so the shell and the Vite proxy never forward it (foundation 3.2). Only `rain-hook` calls it, on the pipe. Its body limit is raised to 4 MiB because a `Write` tool call may carry a large file (foundation 3.4 allows a route to say so). The envelope has a version `v`; the route stays compatible for the life of API v1 (foundation 4.1).
- `POST /sessions/{id}/stop` is not `DELETE` because the registry has `POST` (foundation 14.4); keep it.

### 2.2 Scheduled jobs (module `jobs`)

| Method | Path | Request | Response | OK | Errors | Idem. | Topic effects |
|---|---|---|---|---|---|---|---|
| GET | `/jobs` | none | `ScheduledJob[]` (file order of `jobs.toml`) | 200 | `unavailable` `moduleFailed` | yes | none |
| PATCH | `/jobs/{id}` | `EnabledBody` `{ "enabled": boolean }` | `ScheduledJob` | 200 | `notFound` `unknownJob`; `invalid` `badBody`; `conflict` `configChanged` (the entry moved or vanished while writing); `failed` `saveFailed` (message "Could not save jobs.toml"); `conflict` `configNewer` (written by a newer Rain, read only); `conflict` `configBroken` (the file does not parse: it is never overwritten) | yes (absolute value) | `jobs` |
| POST | `/jobs/{id}/runs` | none | `JobRun` | 201 (a new run); 200 (a run of this job is already in progress: that run is returned, nothing new starts) | `notFound` `unknownJob`; `conflict` `gameMode`; `unavailable` `shuttingDown` | no (second press returns the run in progress) | `jobs` |
| GET | `/runs/{id}/log` | none | `LogLine[]` (oldest first; at most 2000 lines; when lines were cut the first line is a `warn` that says how many) | 200 | `notFound` `unknownRun` | yes | none |

### 2.3 Idle jobs (module `idle`)

| Method | Path | Request | Response | OK | Errors | Idem. | Topic effects |
|---|---|---|---|---|---|---|---|
| GET | `/idle-jobs` | none | `IdleJob[]` (file order) | 200 | `unavailable` `moduleFailed` | yes | none |
| PATCH | `/idle-jobs/{id}` | `EnabledBody` | `IdleJob` | 200 | as `PATCH /jobs/{id}` with `unknownIdleJob` | yes | `idleJobs` |
| POST | `/idle-jobs/{id}/run` | none | `IdleJob` | 200 (also when the job already runs: the current job is returned) | `notFound` `unknownIdleJob`; `conflict` `gameMode`; `conflict` `resourceBusy` (another idle job holds the same resource `cpu` or `gpu`; message names it); `unavailable` `shuttingDown` | yes (a second call returns the running job) | `idleJobs` |
| GET | `/idle-jobs/{id}/log` | none | `LogLine[]` of the latest run (empty array when the job never ran) | 200 | `notFound` `unknownIdleJob` | yes | none |

### 2.4 Live topics

| Topic | Payload | Kind | Max rate | Who publishes and when |
|---|---|---|---|---|
| `sessions` | `ClaudeSession[]` | whole list | 4 per second (`Throttle`, newest wins) | `sessions` module: after every hook event that changes a session, after a registry change, when a watched process exits, after an answer or a stop, when a request expires. Not published while no client is connected (bus rule) |
| `jobs` | `ScheduledJob[]` | whole list | 4 per second | `jobs` module: a run starts or ends, `enabled` changes, `jobs.toml` reloads, `nextRunAt` moves after a scheduled run, a catch-up decision |
| `idleJobs` | `IdleJob[]` | whole list | 4 per second, and a progress change smaller than 1 percent point is not published | `idle` module: state changes, progress, eta, enable, `idle-jobs.toml` reload |

Topic structs (all in `crates/rain-types/src/events.rs`, naming rule of foundation 5.3): `Sessions` (stage 8), `Jobs` and `IdleJobs` (stage 6), each with the payload of its row.

Payload size rule (foundation 5.3, `FRAME_MAX` 256 KiB): `sessions` is the only topic that can grow. The list is capped at 40 sessions, `lastMessage` at 600 characters, an event text at 200, `request.detail` at 4096 characters and `request.summary` at 120. The test `sessions_payload_under_frame_max` builds 40 sessions each with a full `detail` and asserts the JSON is under 200 KiB.

---------------------------------------------------------------------------------------------------

## 3. Types

All types follow foundation 9.2. They live in `crates/rain-types/src/sessions.rs` and `jobs.rs`; each type is added to the `decls!` list of `bindings.rs`. Names equal the TypeScript names that exist today. `Status` is the type of `common.rs` (`idle | running | waiting | success | failed`).

### 3.1 `sessions.rs`

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use crate::common::Status;

/// What the user answers to a permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PermissionAnswer { Once, Always, Deny }

/// One open permission request of a session. Lives in memory only (never in a database).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PermissionRequest {
    pub id: String,            // "req-" + 8 hex characters, created by Rain, opaque to the UI
    pub tool: String,          // tool_name as Claude Code sends it: "Bash", "Edit", "mcp__server__tool"
    pub summary: String,       // one line, at most 120 characters (6.5.2)
    pub detail: String,        // the exact command or a diff-like text, at most 4096 characters (6.5.2)
    pub requested_at: String,  // ISODateTime, the time Rain received the event
    /// true while the hook forwarder still waits for an answer from Rain.
    /// false once it gave up (about 20 s) or when the tool needs a typed answer (AskUserQuestion, ExitPlanMode):
    /// the request is still open in Claude Code, but only the terminal or the app can answer it.
    pub answerable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SessionEventKind { Prompt, Tool, Edit, Message, Permission, Error }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub at: String,
    #[ts(inline)]
    pub kind: SessionEventKind,
    pub text: String,          // at most 200 characters
}

/// Where the session runs. Decides what focus and stop can do (6.6, 6.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SessionHost { Terminal, Desktop, Other }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ClaudeSession {
    pub id: String,                       // the Claude session id (a UUID string)
    pub title: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub model: String,                    // short label such as "sonnet 4.5"; "unknown" when nothing says
    #[ts(inline)]
    pub status: Status,
    pub started_at: String,
    pub last_activity_at: String,
    pub tokens: u64,                      // CONTEXT size of the last request (6.4), not a running total
    pub last_message: String,             // at most 600 characters
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<PermissionRequest>,
    pub recent: Vec<SessionEvent>,        // newest first, at most 8
    #[ts(inline)]
    pub host: SessionHost,                // NEW
    pub alive: bool,                      // NEW: the process still runs. false for ended sessions (status success or failed)
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionAnswerBody {
    pub request_id: String,
    #[ts(inline)]
    pub answer: PermissionAnswer,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PermissionAnswerResult {
    #[ts(inline)]
    pub applied: PermissionAnswer,        // what really happened: "always" becomes "once" when no session rule could be made
    /// For applied = always: the rule that now holds for this session, e.g. "Bash(npm install)".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FocusKind { Window, App, None }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusResult {
    #[ts(inline)]
    pub focused: FocusKind,
}

/// INTERNAL (not in `rain-types`, no `TS`): what `status()` of the installer finds (6.9.7). Section 06 owns the API type
/// `MonitorClaudeState` (`off | on | onInactive | needsRepair`); `hooks_state()` maps this value to it (6.9.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookState { Off, Ok, Broken, Blocked, Unavailable }

#[derive(Debug, Clone)]
pub struct HooksStatus {
    pub enabled: bool,                    // the `expected_on` argument of `status(expected_on)` (6.9.7): `Resolved.monitor_claude` at start and in `hooks_state()`, `true` in `install_hooks()`, `false` in `remove_hooks()`
    pub state: HookState,
    pub message: Option<String>,          // plain sentence, set when state is not ok or off
    pub events_installed: u32,
    pub events_expected: u32,             // 12
    pub forwarder_ok: bool,
    pub last_event_at: Option<String>,    // the last hook event this backend received since it started
}
```

Resulting TypeScript (generated; shown for the new and changed types only):

```ts
export type PermissionAnswer = "once" | "always" | "deny";
export type PermissionRequest = { id: string, tool: string, summary: string, detail: string, requestedAt: string, answerable: boolean };
export type SessionHost = "terminal" | "desktop" | "other";
export type ClaudeSession = { id: string, title: string, cwd: string, projectId?: string, branch?: string, model: string,
  status: "idle" | "running" | "waiting" | "success" | "failed", startedAt: string, lastActivityAt: string, tokens: number,
  lastMessage: string, request?: PermissionRequest, recent: Array<SessionEvent>, host: "terminal" | "desktop" | "other", alive: boolean };
export type PermissionAnswerBody = { requestId: string, answer: "once" | "always" | "deny" };
export type PermissionAnswerResult = { applied: "once" | "always" | "deny", ruleText?: string };
export type FocusResult = { focused: "window" | "app" | "none" };
```

What changes against `src/data/types.ts` lines 47 to 56, and why:

| Item | Change | Why |
|---|---|---|
| `PermissionRequest.answerable` | added, required | a request can stay open in Claude Code after Rain's forwarder gave up; the UI must stop offering buttons that cannot work (6.5.4) |
| `ClaudeSession.host`, `.alive` | added, required | focus and stop depend on the host; the UI offers Stop only for a live terminal session and names the focus button after the host (6.6, 6.7) |
| `ClaudeSession.tokens` | same field, new meaning | the context size of the last request, which a hook-less read of the transcript can give cheaply and which matches the "48k" example; the UI label changes from "Tokens" to "Context" (9.1) |
| `ClaudeSession.status` meanings | `success` = the session has ended normally; `idle` = alive and waiting for the next prompt; `failed` = API error or the process vanished while working | contract report P-S1 and P-S5; table in 6.3.3 |
| `PermissionAnswerBody`, `PermissionAnswerResult`, `FocusResult` | new | S.3, S.4 |
| everything else of the session types | unchanged | |

`HookEnvelope` and the hook payload are NOT in `rain-types`: the forwarder has no dependencies and cannot use the crate. The backend parses them with `serde_json::Value` (every field optional, 6.2.2).

### 3.2 `jobs.rs`

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use crate::common::Status;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum JobKind { Cron, Claude }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct JobRun {
    pub id: String,                       // "run-20261121-093000-ab12" (UTC time plus 4 hex)
    pub job_id: String,
    #[ts(inline)]
    pub status: Status,                   // running | success | failed | idle (idle = stopped on purpose, 6.11.7)
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,         // absent only while running
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,           // the real process exit code (u32 widened); absent while running and when no code exists (timeout, stopped, could not start)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel { Info, Warn, Error }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub ts: String,
    #[ts(inline)]
    pub level: LogLevel,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ScheduledJob {
    pub id: String,
    pub name: String,
    #[ts(inline)]
    pub kind: JobKind,
    pub cron: String,                     // the five-field text of jobs.toml, shown as is
    pub schedule_label: String,           // "Every day at 06:00" (label override of the TOML, or generated, 6.11.2)
    pub enabled: bool,
    pub running: bool,                    // true exactly when recent_runs[0].status == running
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,      // absent when disabled
    pub recent_runs: Vec<JobRun>,         // newest first, at most 20
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum IdleState { Queued, Running, Paused, Success, Failed }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum IdleResource { Cpu, Gpu }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct IdleJob {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[ts(inline)]
    pub state: IdleState,
    pub progress: f64,                    // 0.0 to 1.0
    #[ts(inline)]
    pub resource: IdleResource,
    pub idle_after_minutes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_minutes: Option<u32>,         // only while running and when the job reports or estimates it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,      // the end of the latest run
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnabledBody { pub enabled: bool }
```

Resulting TypeScript (equal to the hand-written types except the points of the table below):

```ts
export type JobRun = { id: string, jobId: string, status: "idle" | "running" | "waiting" | "success" | "failed", startedAt: string, durationMs?: number, exitCode?: number };
export type LogLine = { ts: string, level: "info" | "warn" | "error", text: string };
export type ScheduledJob = { id: string, name: string, kind: "cron" | "claude", cron: string, scheduleLabel: string, enabled: boolean,
  running: boolean, nextRunAt?: string, recentRuns: Array<JobRun> };
export type IdleJob = { id: string, name: string, enabled: boolean, state: "queued" | "running" | "paused" | "success" | "failed",
  progress: number, resource: "cpu" | "gpu", idleAfterMinutes: number, etaMinutes?: number, lastRunAt?: string };
export type EnabledBody = { enabled: boolean };
```

What changes against `src/data/types.ts` lines 59 to 64: nothing in shape. Meaning changes only:

| Item | Change | Why |
|---|---|---|
| `JobRun.status` | `idle` now means "stopped on purpose" (by game mode, by shutdown). The UI already draws any value other than success, failed and running in the neutral colour (`runColor.ts`) and "Needs you" counts only `failed`. No type change. | a deliberate stop must not raise a red alarm on Home (6.11.7) |
| `JobRun.exitCode` | absent when no real exit code exists (timeout, stopped, could not start); the log explains | the UI shows nothing for an absent code; an invented code would lie |
| `ScheduledJob.scheduleLabel` | generated by the hand-written label function; the wording differs from the mock ("Every day at 06:00", not "Daily 06:00") | the spike's label function (spike crates, `cron_label.rs`); the column is 168 px and truncates |
| `IdleJob.state` after disable | `paused` (progress kept for display); the UI shows "Disabled" anyway | 6.12.3 |

`MachineState` is generated by section 05 in stage 3, before this section's stages. `RainApi.getMachineState` is not changed here.

---------------------------------------------------------------------------------------------------
## 4. Data files

Two files in the data folder belong to this section (rows of foundation 10.2): `jobs.toml` and `idle-jobs.toml`. Both follow the common rules of foundation 10.3 and 10.4: first key `schema_version`, a missing key means version 1, readers use `#[serde(default)]` and do not deny unknown keys, writers use `toml_edit`, the file is read with the `TomlFile<T>` type, a parse error keeps the last good value and adds a `ConfigIssue`.

Session data has no TOML file. The one external file that this section writes is the user-level Claude settings file, and only through the installer of 6.9 (not a data folder file).

### 4.1 `jobs.toml` (scheduled jobs)

#### 4.1.1 Schema

One table array `[[job]]`. File order is list order.

Keys of every job:

| Key | Type | Default | Rule |
|---|---|---|---|
| `id` | string | derived (below) | identity of the job and of its history. `A-Za-z0-9._~-`, 1 to 128 characters (foundation 4.3). Duplicate ids: the second entry is skipped with a `ConfigIssue` |
| `name` | string | none (required) | 1 to 80 characters, shown in the list. A job without a name is skipped with a `ConfigIssue` |
| `kind` | `"cron"` or `"claude"` | none (required) | any other value: entry skipped with a `ConfigIssue` |
| `schedule` | string | none (required) | exactly five cron fields: minute hour day-of-month month day-of-week, local time. `*`, lists, ranges, steps and `0` or `7` for Sunday are accepted. Six or seven fields, `@daily` style macros and text that `croner` rejects: entry skipped, the `ConfigIssue` message is "Use five fields: minute hour day month weekday" |
| `enabled` | bool | `false` | a job that has no `enabled` key never runs by itself |
| `label` | string | generated | replaces the generated schedule label, at most 40 characters |
| `working_dir` | string | `<local>\cache\jobs\<id>` (Rain creates it; the user may delete the whole cache folder, so Rain recreates this default folder at every run start) | an absolute path. A folder the user set must exist when a run starts, else the run fails with a clear log line. Relative paths are refused |
| `timeout_minutes` | integer 1 to 1440 | 30 for `cron`, 60 for `claude` | when reached the run is terminated (6.11.7) |
| `catch_up` | bool | `true` | whether a run missed while Rain or the PC was off runs once at the next start (6.11.5) |
| `env` | inline table of strings | empty | extra environment variables for the run. Never put a secret here (foundation 10.3 rule 7) |

Keys of `kind = "cron"` (a command):

| Key | Type | Default | Rule |
|---|---|---|---|
| `command` | string | none (required) | a program path or a name found on `PATH` (exec form, no shell) when `shell` is `"none"`; a whole script line otherwise |
| `args` | array of strings | empty | passed one by one with correct Windows quoting. Must be empty when `shell` is not `"none"` |
| `shell` | `"none"`, `"cmd"`, `"powershell"` | `"none"` | `cmd` runs `cmd.exe /d /s /c "<command>"`; `powershell` runs `powershell.exe -NoProfile -NonInteractive -Command <command>` (Windows PowerShell 5.1) |

Keys of `kind = "claude"` (a headless Claude run):

| Key | Type | Default | Rule |
|---|---|---|---|
| `skill` | string | none | the name of an existing Claude skill. `A-Za-z0-9._:-`, 1 to 100 characters |
| `prompt` | string | none | the prompt text. At least one of `skill` and `prompt` is required, else the entry is skipped. With both, the skill instruction comes first (6.11.8) |
| `model` | string | none (Claude's own default) | passed as `--model` |
| `permission_mode` | `"dontAsk"`, `"acceptEdits"`, `"default"`, `"plan"`, `"auto"`, `"bypassPermissions"` | `"dontAsk"` | an unattended run cannot answer a prompt, so `dontAsk` (deny everything not listed in `allowed_tools`) is the default. `"bypassPermissions"` is accepted only together with `dangerously_skip_permissions = true`; without it the entry is skipped with a `ConfigIssue` |
| `dangerously_skip_permissions` | bool | `false` | see above |
| `allowed_tools` | array of strings | empty | joined with a comma into one `--allowedTools` argument. Examples: `"Read"`, `"Bash(git status *)"`, `"Skill"` |
| `disallowed_tools` | array of strings | empty | one `--disallowedTools` argument |
| `max_turns` | integer 1 to 1000 | none | `--max-turns` |
| `claude_program` | string | `"claude"` | a path or a name. A bare name is searched on `PATH` for `claude.exe`; if that fails Rain tries `%USERPROFILE%\.local\bin\claude.exe` (the default install place; built from the environment at run time, never written in code as a user path) |
| `extra_args` | array of strings | empty | appended last to the command line, for a flag this section does not know |

What a hand edit may do: add, change, remove, reorder jobs; add comments anywhere; change a schedule (the scheduler recomputes at once); remove a job (its history stays in the database for 30 days, 6.11.10). A running run is not interrupted by an edit.

What Rain writes back, and when:
- `enabled`, when the user toggles it (`PATCH /jobs/{id}`). The value keeps its trailing comment (the proven `set_bool_keep_decor`, foundation 10.4). If the key is absent it is added at the end of that entry.
- `id`, when the entry has none and Rain writes that entry anyway (a toggle): the derived id is inserted as the first key of the entry, so a later rename of `name` cannot orphan the history. Rain never writes ids into entries it does not otherwise touch.
- The whole file once, at first start when it does not exist (4.1.3).

Derived id (entry without `id`): `job-` plus the lower-case `name` with every run of characters outside `a-z0-9` replaced by one `-`, trimmed, cut to 60 characters; on a clash `-2`, `-3`.

`schema_version`: 1. The migration function exists (`fn migrate(doc, from)`) and does nothing yet.

#### 4.1.2 Validation and what happens on errors

Per entry, not per file. A bad entry is skipped, a `ConfigIssue { file: "jobs.toml", line, message }` names it, and all other entries work ("one bad job must not make `listJobs` fail", contract report P-J1). An entry issue is reported through `health.set_file_issues` as information. It does not block writes: `setJobEnabled` and `setIdleJobEnabled` work for every good entry and keep the text of the bad one unchanged. `configBroken` is returned only when the whole file failed to parse or validate. A file that does not parse at all keeps the last good list (foundation 10.4); at first load it means an empty list plus the issue. `GET /jobs` itself fails only when the module failed to start (`unavailable`, `moduleFailed`).

#### 4.1.3 Default when the file is missing, and the first write

At the first start in which `jobs.toml` does not exist, the backend writes exactly this text (atomic write, foundation 10.4). It has no job, so the Jobs tab is a true empty state.

```toml
schema_version = 1

# Scheduled jobs. One [[job]] table per job. Rain shows them in the Jobs tab,
# runs them by schedule, and writes back only `enabled` (and `id` when it is missing).
# Edit this file by hand; Rain reloads it at once. Times are local time.
#
# A command job:
#
# [[job]]
# id = "backup-notes"                  # optional, stable name of the job
# name = "Back up notes"
# kind = "cron"
# schedule = "30 2 * * *"              # minute hour day month weekday (five fields)
# enabled = false
# command = "robocopy"
# args = ["D:\\notes", "E:\\backup\\notes", "/MIR"]
# working_dir = "D:\\"                 # optional
# timeout_minutes = 30                 # optional
# catch_up = true                      # run once at the next start when a run was missed
#
# A Claude job (headless `claude -p`, nobody can answer a permission prompt):
#
# [[job]]
# name = "Morning skill run"
# kind = "claude"
# schedule = "0 6 * * *"
# enabled = false
# skill = "my-skill"                   # a skill that exists in your Claude setup
# prompt = "Optional extra instructions."
# permission_mode = "dontAsk"          # deny everything that is not listed below
# allowed_tools = ["Skill", "Read", "Write", "Edit", "Glob", "Grep"]
# timeout_minutes = 60
```

#### 4.1.4 Seeding the owner's first job (decisions section 4)

The owner's job (a Claude job that runs the owner's skill at 06:00, switched off) is personal data. It is never in code, in a fixture or in a document of the repository. The seeding step is a documented one-time step of stage 6, done by the lead on the owner's PC with the owner's data folder (the path comes from settings, not from this document):

1. Stop the backend. Open `jobs.toml` in the owner's data folder (the backend wrote it at its first start, 4.1.3).
2. Append this entry, with `<skill-name>` replaced by the name that `decisions.md` section 4 gives, and save:

```toml
[[job]]
id = "job-morning-skill-run"
name = "Morning skill run"
kind = "claude"
schedule = "0 6 * * *"
enabled = false
skill = "<skill-name>"
permission_mode = "dontAsk"
# First run: read the log of "Run now", find the lines "Denied <tool>", and add the tools the skill really needs.
allowed_tools = ["Skill", "Read", "Write", "Edit", "Glob", "Grep", "Bash"]
timeout_minutes = 60
```

3. Start the backend. The Jobs tab shows the job, switched off. The acceptance check A6.1 of section 11 runs it once by hand.

Why a manual step and not a code path: any automatic seed would need the owner's skill name in code or a seed file in the repository. A file the owner edits is the same mechanism that every other user uses, and it is the smallest design that keeps the repository free of personal data. The text of the example above is deliberately generic (`my-skill`) in the file Rain writes. The `allowed_tools` of the step 2 entry is a starting point that the first log run refines; `Bash` is included because the owner's skill compiles and checks files, and the owner can narrow it to `Bash(<pattern>)` rules after reading the first log.

### 4.2 `idle-jobs.toml` (idle jobs)

#### 4.2.1 Schema

One table array `[[idle_job]]`. Common keys as in 4.1.1 where the name is the same.

| Key | Type | Default | Rule |
|---|---|---|---|
| `id`, `name` | as 4.1.1 | derived id prefix is `idle-` | |
| `enabled` | bool | `false` | |
| `resource` | `"cpu"` or `"gpu"` | `"cpu"` | what the job loads. At most one running idle job per resource; the other waits |
| `idle_after_minutes` | integer 1 to 1440 | 10 | the PC must have had no input for this long before the job starts or resumes |
| `command`, `args`, `shell` | as 4.1.1 `cron` | none (required `command`) | |
| `working_dir`, `env`, `timeout_minutes` | as 4.1.1 | working dir `<local>\cache\idle\<id>` (owned by the `idle` module, never the folder of `jobs`; the user may delete the whole cache folder, so Rain recreates the default folder at every run start; foundation 10.6); timeout 240 minutes (range 1 to 1440) | a run that reaches the timeout is terminated and counts as failed |
| `rerun` | `"daily"`, `"weekly"`, `"every_idle"`, `"manual"` | `"daily"` | when a finished job becomes `queued` again (6.12.4) |
| `min_gap_minutes` | integer 0 to 10080 | 60 | only for `every_idle`: the least time between the end of a run and the next start |
| `expected_minutes` | integer 1 to 10080 | none | how long a full run takes. Used for `etaMinutes` and an estimated `progress` when the job reports none |

Progress reporting convention (the answer to contract report P-I3): a worker writes a line to its standard output that matches (case-insensitive, whole line)

```
rain-progress: <number>[%]
rain-eta: <minutes>
```

`rain-progress: 0.42` and `rain-progress: 42%` both mean 0.42. Values are clamped to 0 to 1; a decrease is ignored. These lines are also written to the log. A job that reports nothing shows progress 0 while it runs and 1 when it succeeds; with `expected_minutes` it shows `min(0.95, elapsed / expected)` and an eta.

What Rain writes back: `enabled` and a missing `id`, exactly as 4.1.1. At first start Rain writes the file below (so the owner can see the format). It has no job: idle jobs start empty (decisions section 5).

```toml
schema_version = 1

# Idle jobs: programs that run while you are away from the PC.
# A job starts after `idle_after_minutes` without keyboard or mouse input, is frozen when you come back,
# and continues after the same time of quiet. Only one job per resource (cpu or gpu) runs at a time.
# Rain reloads this file at once. Rain writes back only `enabled` (and `id` when it is missing).
#
# [[idle_job]]
# id = "idle-thumbnail-cache"            # optional, stable name of the job
# name = "Rebuild thumbnail cache"
# enabled = false
# resource = "cpu"                       # "cpu" or "gpu"
# idle_after_minutes = 10
# command = "D:\\tools\\thumbs.exe"
# args = ["--all"]
# rerun = "daily"                        # "daily", "weekly", "every_idle" or "manual"
# expected_minutes = 30                  # optional: gives an estimate when the program reports no progress
#
# The program may print lines like these to report progress:
#   rain-progress: 42%
#   rain-eta: 12
```

#### 4.2.2 Validation

As 4.1.2, per entry. `command` empty, `resource` or `rerun` outside their sets, a `shell` other than the three values, `args` together with a `shell` other than `none`: entry skipped with a `ConfigIssue`.

---------------------------------------------------------------------------------------------------

## 5. Tables

SQLite follows foundation 10.5. Each module owns the tables with its prefix. Timestamps are RFC 3339 text in the API format, durations are integer milliseconds. No table here is a rebuildable cache: they are machine history. Losing `rain.db` loses history and the catch-up bookkeeping, never a definition (the definitions are TOML).

Migrations: `modules/jobs/migrations/0001_jobs.sql`, `modules/idle/migrations/0001_idle.sql`, `modules/sessions/migrations/0001_sessions.sql`. Each module exports `pub const MIGRATIONS` with one entry per file (foundation 10.5).

### 5.1 `jobs` module

```sql
-- 0001_jobs.sql
CREATE TABLE jobs_run (
  id                 TEXT PRIMARY KEY,                    -- run-20261121-093000-ab12
  job_id             TEXT NOT NULL,                       -- the id in jobs.toml
  trigger_kind       TEXT NOT NULL CHECK (trigger_kind IN ('schedule', 'manual', 'catchup')),
  status             TEXT NOT NULL CHECK (status IN ('running', 'success', 'failed', 'idle')),
  started_at         TEXT NOT NULL,
  ended_at           TEXT,
  duration_ms        INTEGER,
  exit_code          INTEGER,                             -- real exit code only
  claude_session_id  TEXT,                                -- set when the run was a Claude job and its SessionStart was seen
  log_lines          INTEGER NOT NULL DEFAULT 0,          -- lines stored in jobs_run_log
  log_omitted        INTEGER NOT NULL DEFAULT 0           -- lines dropped by the cap (6.11.9)
);
CREATE INDEX idx_jobs_run_job_started ON jobs_run (job_id, started_at DESC);
CREATE INDEX idx_jobs_run_status ON jobs_run (status);

-- Log lines of scheduled runs AND of idle runs (idle run ids start with "irun-"); only the jobs module writes here.
CREATE TABLE jobs_run_log (
  run_id  TEXT    NOT NULL,
  seq     INTEGER NOT NULL,
  ts      TEXT    NOT NULL,
  level   TEXT    NOT NULL CHECK (level IN ('info', 'warn', 'error')),
  text    TEXT    NOT NULL,                               -- at most 2000 characters
  PRIMARY KEY (run_id, seq)
) WITHOUT ROWID;

-- The scheduler's memory: every occurrence up to this instant has been handled (run, skipped or deliberately not run).
CREATE TABLE jobs_state (
  job_id         TEXT PRIMARY KEY,
  handled_until  TEXT NOT NULL,
  updated_at     TEXT NOT NULL
);
```

Retention (run at the end of every run, in the same transaction as the final update):

| Rule | Value |
|---|---|
| runs kept per job | the newest 20 (`JOB_RUNS_KEPT`); older runs and their log lines are deleted |
| runs of jobs that are no longer in `jobs.toml` | deleted when older than 30 days (checked at start) |
| log lines stored per run | at most 4000 from the start plus the last 500 (6.11.9) |
| total log rows | at most 200 000 (`JOBS_LOG_ROWS_MAX`), counting the rows of scheduled runs AND of idle runs; above it the log lines of the oldest finished runs are deleted first (the run rows stay, `log_lines` becomes 0). Rows whose run id starts with `irun-` are removed only by the log store's `delete` (called by `idle` for its old runs) and by this cap, the oldest `irun-` runs first, because the `jobs_run` retention knows only `jobs_run` rows |
| a `running` row at start | closed as `failed` with a log line "Rain stopped while this run was in progress" (6.11.6) |

### 5.2 `idle` module

```sql
-- 0001_idle.sql
CREATE TABLE idle_state (
  job_id           TEXT PRIMARY KEY,
  last_started_at  TEXT,
  last_ended_at    TEXT,
  last_result      TEXT CHECK (last_result IN ('success', 'failed'))   -- NULL until the first run ends
);

CREATE TABLE idle_run (
  id          TEXT PRIMARY KEY,                           -- irun-20261121-031500-9f3c
  job_id      TEXT NOT NULL,
  started_at  TEXT NOT NULL,
  ended_at    TEXT,
  status      TEXT NOT NULL CHECK (status IN ('running', 'success', 'failed', 'stopped')),
  manual      INTEGER NOT NULL DEFAULT 0,                 -- started by "Run now"
  exit_code   INTEGER
);
CREATE INDEX idx_idle_run_job_started ON idle_run (job_id, started_at DESC);
```

Retention: the newest 5 runs per job and their lines in `jobs_run_log` (deleted through the log store's `delete(run_id)`, one call per run that falls out of the newest 5). Progress and the paused state are not stored: after a restart a job is `queued` again (its process died with Rain, 6.12.6).

### 5.3 `sessions` module

```sql
-- 0001_sessions.sql
-- Sessions that ended, so the "Done" and "Failed" groups and their transcripts survive a backend restart.
CREATE TABLE sessions_ended (
  id                TEXT PRIMARY KEY,                     -- Claude session id
  title             TEXT NOT NULL,
  cwd               TEXT NOT NULL,
  project_id        TEXT,
  branch            TEXT,
  model             TEXT NOT NULL,
  host              TEXT NOT NULL,
  status            TEXT NOT NULL CHECK (status IN ('success', 'failed')),
  started_at        TEXT NOT NULL,
  ended_at          TEXT NOT NULL,
  last_activity_at  TEXT NOT NULL,
  tokens            INTEGER NOT NULL DEFAULT 0,
  last_message      TEXT NOT NULL DEFAULT '',
  transcript_path   TEXT,                                 -- the path Claude Code gave; read only, never opened for writing
  recent_json       TEXT NOT NULL DEFAULT '[]'            -- the last 8 SessionEvents
);
CREATE INDEX idx_sessions_ended_ended_at ON sessions_ended (ended_at DESC);
```

Retention: rows older than 24 hours are deleted at start and whenever a session ends; at most 50 rows (oldest first). `GET /sessions` lists at most the 20 newest ended sessions. Open permission requests are never stored.

---------------------------------------------------------------------------------------------------
## 6. Algorithms

Spike source files are named as `S\<spike>\...` with `S` = the spike root of `foundation-review.md`. Everything marked "copy" is throwaway spike code: copy the pattern and the comments, then rewrite it to the style rules of the foundation (no `unwrap` in product paths, `thiserror` inside modules, no `Command::new` outside `platform/`).

### 6.1 Where session facts come from (hooks first, defensive extras second)

| Source | Role | Status of the knowledge |
|---|---|---|
| Hook events from `rain-hook` (6.2) | the first and main source: identity, status, request, recent events, last message | proven headless (spike claude-hooks); interactive terminal and desktop UNPROVEN, gate 6.10 |
| The registry `<claude folder>\sessions\<pid>.json` | discovery of sessions that Rain did not see start (before Rain ran, before hooks were installed, a never-trusted folder where hooks stay silent); liveness (pid plus start time); the name of the session; the host (`entrypoint`) | undocumented, may change; every field optional; unknown values mean "unknown" and never an error (decision C1) |
| The transcript `<claude folder>\projects\<folder>\<session id>.jsonl` | the readable log (`getSessionTranscript`); model, tokens, branch, title and last message at first sight and after each `Stop` | internal format, parsed line by line with `serde_json::Value`, every field optional |
| The process table (Toolhelp plus `GetProcessTimes`) | liveness; the parent chain for focus; the children for stop | proven (spike process Q6, claude-hooks Q4) |

Degrading when the registry format changes: the module logs once at `warn` ("registry files not understood") and sets `Health.sources` `claude-registry` to `failed` with that message. Hook events still create and update sessions; liveness falls back to the pid that the hook reports (`CLAUDE_PID`, 6.2.2) or, with none, to "ended when `SessionEnd` arrives". With both sources gone the list is empty and the UI shows its empty state, not an error.

Where the Claude folder is: `<claude folder>` = `Config.claude_dir` if set (tests, and `RAIN_CLAUDE_DIR` for manual runs, see 12.2), else the environment variable `CLAUDE_CONFIG_DIR` if set, else `%USERPROFILE%\.claude`. All of it is in one file, `modules/sessions/claude_paths.rs` (12.1 asks the architect to allow the folder name only there). `claude_paths.rs` only builds paths. Nothing outside `hooks_install.rs` names the settings file.

Reading the registry (`registry.rs`): list the folder, keep only names that match `^[0-9]+\.json$` exactly (so `<pid>.<hex>.key` files are never opened), read each file (cap 64 KiB), parse with `serde_json::Value`. Fields used: `pid`, `sessionId`, `cwd`, `startedAt` (epoch ms), `procStart` (decimal FILETIME text), `kind`, `entrypoint`, `name`, `nameSource`, `status`, `statusUpdatedAt`, `updatedAt`, `waitingFor`, `version`. Anything else is ignored and never stored. A file that does not parse is skipped (it may be half written; the next change event reads it again).

Triggers (all event driven, no timer): at module start; on any change under the registry folder through the shared watcher (`state.config.watch_dir`, non-recursive, 300 ms debounce, 12.5 asks for the call); and on every read of `GET /sessions` for the liveness check only (a few `OpenProcess` calls, about 20 microseconds each).

### 6.2 The hook protocol and the program `rain-hook`

#### 6.2.1 `rain-hook` (crate `crates/rain-hook`, built in stage 8b)

Zero dependencies, `#![forbid(unsafe_code)]`, release profile `opt-level = "z"` (foundation 1.4). The spike forwarder is the model for cost and fail-open behaviour (`S\claude-hooks\fwd\src\main.rs`, 162 KB, about 20 ms per call, of which `main` 4.4 to 6.6 ms). Differences from the spike: it talks to the pipe instead of writing a log, it does no process-table walk (the parent chain cost 4.7 ms and is not needed: the backend gets the pid from the registry or from `CLAUDE_PID`), and it does not use `unsafe`.

Command line (exec form, installed by 6.9):

```
rain-hook.exe event      [--pipe=<name>] --rain-managed=<id>
rain-hook.exe permission [--pipe=<name>] --rain-managed=<id>
rain-hook.exe selftest
```

`--rain-managed=<id>` is only the marker by which the installer finds its own entries; the program ignores it. `--pipe=<name>` is written by the installer only when the backend's pipe name differs from the default real name (sandbox, `RAIN_PIPE`); the default is the function `pipe_name()`, a copy of `rain_types::env::pipe_name` (foundation 1.2, compared by a test).

Constants (module `limits` of the hook; the shared ones are copies of `rain_types::limits`):

| Constant | Value | Meaning |
|---|---|---|
| `HOOK_WAIT` | 20 s | how long `permission` waits for Rain's answer (foundation appendix A) |
| `EVENT_WAIT` | 300 ms | how long `event` waits for the `204` |
| `STDIN_MAX` | 3 MiB | larger input: nothing is forwarded |
| `BUSY_RETRY` | 5 ms, at most 10 tries | `ERROR_PIPE_BUSY` (231) |
| `LOG_MAX` | 256 KiB | `hook.log`, truncated at the start when over (foundation 10.7) |

Flow (every branch ends with exit code 0; stdout stays empty except in the one case marked PRINT):

```
1. read all of stdin (stop at STDIN_MAX: over -> exit 0, nothing forwarded; for `permission` this means the native prompt)
2. check that the text starts with '{' and ends with '}' (after trimming); else exit 0
3. build the envelope text by string concatenation (6.2.2); env values come from RAIN_RUN_ID, RAIN_JOB_ID,
   CLAUDE_CODE_ENTRYPOINT and CLAUDE_PID, each JSON-escaped by a 15-line function; the stdin text is embedded unchanged
4. open the pipe with OpenOptions read+write on \\.\pipe\<name>
     os error 2 (not found)  -> Rain is not running: exit 0 at once. This is the normal case, nothing is logged
     os error 231 (busy)     -> sleep 5 ms, retry up to 10 times, then exit 0 and log "pipe busy"
     os error 5 (denied)     -> exit 0, log "pipe denied"
5. write one HTTP/1.1 request:  POST /api/v1/internal/hook   Host: rain   Content-Type: application/json
                                Content-Length: <n>   Connection: close   then the envelope
6. start a reader thread that reads the whole answer (limit 256 KiB) and sends it over a channel;
   the main thread waits with recv_timeout(EVENT_WAIT or HOOK_WAIT)
7. on timeout, EOF without data, parse failure, status other than 200 or 204: exit 0 (log the reason unless it is a timeout of `event`)
8. mode permission and status 200 and the body starts with '{': PRINT the body to stdout unchanged, flush, exit 0
9. everything else: exit 0
```

Why the answer body is printed unchanged: the backend builds the exact JSON that Claude Code expects (6.5.5). The forwarder then needs no JSON parser, which keeps it at zero dependencies and makes a mistake in it unable to produce a wrong decision. Why a thread and not a read timeout: a blocking read on a pipe opened with `OpenOptions` cannot time out; a thread that is abandoned at `process::exit` is the cheapest fix (UNPROVEN in a spike, foundation appendix C row "named-pipe write and reply"; checked in stage 8b by the hook tests of section 10).

Fail-open table (the foundation's checklist, A2.5 of the research, made concrete). In every row Claude Code sees exit 0 and empty stdout, which equals "no hook": the normal prompt appears and no "hook error" notice is shown.

| Failure | What the forwarder does |
|---|---|
| Rain not running, pipe missing | exit 0 at once (about 20 ms total, the process creation cost) |
| Rain in game mode or shutting down | the pipe is gone or the backend answers `204` empty: same |
| pipe busy for more than 50 ms | exit 0 |
| backend slow: no answer within the wait | exit 0 (for `permission` after 20 s) |
| invalid or oversized input | exit 0, nothing forwarded |
| backend answers an error status or garbage | exit 0, log one line |
| the forwarder panics | `panic = "abort"` is NOT used (the foundation profile keeps `unwind`); `main` wraps the body in `catch_unwind` and exits 0 |

Cost budget (foundation 1.3): wall time under 50 ms and `main` under 10 ms when Rain is not running; measured by the test `hook_budget` in stage 8b on the release build.

`selftest`: prints nothing, exits 0. The installer runs it with a 2 second limit to prove that the copied program starts (6.9.5).

#### 6.2.2 The envelope

The body of `POST /internal/hook`:

```json
{
  "v": 1,
  "mode": "event",
  "env": { "runId": "run-20261121-060000-ab12", "jobId": "job-morning-skill-run", "entrypoint": "cli", "claudePid": 4242 },
  "event": { "session_id": "...", "transcript_path": "...", "cwd": "...", "hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": { "command": "npm install" } }
}
```

Backend type (in `sessions/hook_wire.rs`, not in `rain-types`):

```rust
#[derive(Deserialize)]                       // NOT deny_unknown_fields: an older forwarder and a newer backend must interoperate
struct HookEnvelope {
    v: u32,                                  // 1; any other value -> 400 badEnvelope
    mode: HookMode,                          // "event" | "permission"
    #[serde(default)] env: HookEnv,          // every field optional
    event: serde_json::Value,                // the stdin of Claude Code, untouched
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct HookEnv { run_id: Option<String>, job_id: Option<String>, entrypoint: Option<String>, claude_pid: Option<u32> }
```

Fields of `event` that the backend reads (all optional; a missing one means "unknown" and never an error): `session_id`, `transcript_path`, `cwd`, `hook_event_name`, `permission_mode`, `source`, `model`, `session_title`, `prompt`, `tool_name`, `tool_input`, `error`, `is_interrupt`, `last_assistant_message`, `background_tasks`, `reason`, `to_model`, `permission_suggestions`, `agent_id`. The key table is `S\claude-hooks\out\keys-all.json`. Events whose `hook_event_name` Rain does not know are ignored (answered `204`). Hook payloads are never logged as a whole, and never their text (foundation 13.4); the logs may carry the event name and the session id.

The test `envelope_matches_forwarder` includes the hook's `envelope.rs` with `#[path]` and parses what it builds with the backend type, on a list of inputs (quotes, backslashes, newlines, unicode, an empty `env`).

#### 6.2.3 What the route does

```
POST /internal/hook
  parse the envelope                          -> invalid, reason badEnvelope on any failure
  env.runId present (a headless run that Rain started)
        -> if mode is permission: answer 204 at once (never shown to the user, never held)
        -> else: jobs.attach_session(runId, session_id) when the event is SessionStart; answer 204. The session is NOT added to the book
  mode event      -> book.apply(event); answer 204 (the work is done inside the handler; it takes microseconds)
  mode permission -> 6.5 (the handler may wait up to HOOK_SERVER_WAIT = 19 s, then answers 204)
  the switch is off or the module is shutting down -> answer 204
```

### 6.3 The session book: events to state

#### 6.3.1 Record

```rust
struct SessionRec {
    id: String,                        // Claude session id
    pid: Option<u32>, proc_start: Option<u64>,       // FILETIME, from the registry; or from CLAUDE_PID at first sight
    host: SessionHost,                 // from the registry entrypoint or env.entrypoint
    cwd: String, project_id: Option<String>, branch: Option<String>, model: String,
    title: TitleSources,               // registry name, hook title, transcript title, first prompt, folder name
    started_at: SystemTime, last_activity_at: SystemTime,
    turn: Turn,                        // Working | Idle | Failed | Unknown  (what hooks last told us)
    registry: Option<RegistryInfo>,    // last read of the registry file
    pending: Vec<Pending>,             // open permission requests, oldest first
    recent: VecDeque<SessionEvent>,    // newest first, at most 8
    tokens: u64, last_message: String, transcript_path: Option<PathBuf>,
    stopped_by_rain: bool, ended: Option<Ended>,   // Ended { at, ok: bool }
}
```

The book is `HashMap<String, SessionRec>` behind one `std::sync::Mutex` (never held across an await). Every change ends with `publish()`, which builds the list (6.3.4) and calls the throttled `bus.publish::<Sessions>`.

#### 6.3.2 What each hook event does

All events first update: `last_activity_at = now`; `cwd` (every payload carries it); `transcript_path` when present; the pid from `env.claudePid` when the record has none. An event for an unknown `session_id` creates the record (a session that started before Rain, or a never-trusted folder that reported only now). `SessionRec.turn` is what the hooks say; the status shown is derived in 6.3.3.

| Event | Effect |
|---|---|
| `SessionStart` | create or refresh the record; `turn = Idle`; `model` from `model`; `title` from `session_title`; queue a transcript tail read (6.4). With `source` `resume` or `compact` the record is the same session |
| `UserPromptSubmit` | `turn = Working`; recent event `prompt` with the first line of `prompt`, cut to 200; `title` from `session_title`; clear every `pending` entry that is not answerable (the user moved on) |
| `PreToolUse` | `turn = Working`; recent event `tool` "<tool>: <summary>" (6.5.2 builds the summary), except for `Edit`, `Write`, `MultiEdit`, `NotebookEdit` whose event is written by `PostToolUse` |
| `PermissionRequest` | not here: mode `permission` (6.5) |
| `PostToolUse` | `turn = Working`; remove the matching pending request (6.5.6); for the edit tools a recent event `edit` with the file path (relative to `cwd` when under it) |
| `PostToolUseFailure` | as `PostToolUse`; when `is_interrupt` is not true a recent event `error` with the first line of `error` |
| `PermissionDenied` | remove the matching pending request; recent event `permission` "Denied: <tool>" |
| `Stop` | `turn = Idle` unless `background_tasks` holds an entry whose `status` is `"running"` (then stay `Working`; the field names are from the docs, UNVERIFIED); `last_message` from `last_assistant_message` (600 characters); recent event `message` (first line, 200); clear every pending request; queue a transcript tail read |
| `StopFailure` | `turn = Failed`; recent event `error` with the `error` kind; clear every pending request |
| `PostModelSwitch` | `model` from `to_model` |
| `CwdChanged` | `cwd`; recompute `project_id` |
| `SessionEnd` | `reason` `clear` or `resume`: the process lives on and a new `SessionStart` with a new id follows, so the record is dropped silently (not kept, not stored). Any other reason (`logout`, `prompt_input_exit`, `other`, unknown): the session ended normally, `finish(rec, ok = true)` (6.3.5) |
| anything else (`Notification`, `SubagentStart`, ...) | not installed, ignored. If one arrives it only updates `last_activity_at` |

A `Notification` hook is deliberately not installed: it carries nothing the status needs (decision C1: `waiting` comes only from an open permission request), and every installed hook costs about 20 ms of process creation per event.

#### 6.3.3 The status shown (the state machine)

`status(rec, now)` is a pure function, unit tested with a table of cases (10.1). First match wins.

| # | Condition | `status` | `alive` |
|---|---|---|---|
| 1 | `ended = Some(ok = true)` | `success` | false |
| 2 | `ended = Some(ok = false)` | `failed` | false |
| 3 | `pending` is not empty | `waiting` | true |
| 4 | `turn = Failed` | `failed` | true (the process still runs; it leaves this state at the next prompt) |
| 5 | `turn = Working`, and not (the Esc rule below) | `running` | true |
| 6 | `turn = Idle` | `idle` | true |
| 7 | `turn = Unknown` (no hook event seen yet) and the registry says `status = "busy"` | `running` | true |
| 8 | `turn = Unknown` otherwise (registry `idle`, `waiting`, any other text, or no registry) | `idle` | true |

Notes:
- `waiting` needs `pending` non-empty, nothing else. The registry value `waiting` (with `waitingFor` "input needed") means "Claude finished and waits for you to type" and maps to `idle` (row 8). This is decision C1 and the observation of the contract report.
- `success` means the session ended normally: the group "Done" lists sessions that are over. A finished turn of a live session is `idle` ("waiting for your next prompt"), not "Done". Reason: the interactive CLI cannot tell "the task is finished" from "waiting for the next prompt", and a session that is merely paused must not leave the user's sight.
- The Esc rule (UNPROVEN: the foundation notes that no hook fires when the user presses Esc): when `turn = Working` and the registry says `status = "idle"` with `statusUpdatedAt` newer than the last hook event by at least `ESC_OVERRIDE` = 3 s, the row shows `idle`. If the registry value turns out to mean something else the effect is cosmetic; the constant can be set to a very large value to switch the rule off.
- Stale (a `running` row with no news for 60 s) is drawn by the UI from `lastActivityAt`; the backend never sends it. `last_activity_at` is the later of the last hook event and the registry `updatedAt`.

Liveness and death (a process that dies leaves no `Stop` and no `SessionEnd`; the registry file stays until the next `claude` start, proven):

| Signal | Source | Cost |
|---|---|---|
| a registered exit wait fires | `platform::process::ExitWatcher` (6.3.6) | zero while idle |
| the check at read time | every `GET /sessions`, every registry event: `alive(rec)` = process with `pid` exists and `|creation FILETIME - procStart| < 100000` ticks (10 ms; the two differ by about 300 ns, never compare with `==`; spike claude-hooks Q4). With no `procStart` (hook-only record) the creation time recorded at first sight is used | 20 microseconds per session |
| the registry file vanished and the pid is gone | registry event | free |

When a record turns not alive: `finish(rec, ok)` with `ok = false` if its status was `running` or `waiting` (or `turn = Failed`) and `stopped_by_rain` is false; else `ok = true`. A session killed from outside while working therefore becomes `failed`; one the user closed while it was idle becomes `success`; one that Rain stopped becomes `success` (a deliberate end).

Pid conflict (the `/clear` case without a `SessionEnd`, hooks off): when a registry file's `pid` is the pid of an existing record with a different `sessionId`, the old record is dropped silently.

#### 6.3.4 The list

`list()` = live records (not ended, not owned by a run) in any order plus the newest 20 ended sessions of the last 24 hours, cut to 40 in total (live first). Each record becomes a `ClaudeSession` through `to_api(rec, now)` (field sources in 6.4). Records whose `env.runId` was set (a Rain-run headless Claude job) are not in the book at all (6.2.3).

#### 6.3.5 `finish`

`finish(rec, ok)`: set `ended`, drop pending requests (their forwarders are gone or will be told), insert the row into `sessions_ended`, delete expired rows (retention in 5.3), publish. The ended record stays in memory until the retention passes.

#### 6.3.6 `ExitWatcher` (platform)

`platform::process::ExitWatcher::register(pid, expected_start, callback) -> Option<ExitWatcher>`: `OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, pid)`, check the creation time against `expected_start` (tolerance as above), `RegisterWaitForSingleObject(&mut wait, handle, callback, ctx, INFINITE, WT_EXECUTEONLYONCE)`. The callback runs on a system thread-pool thread, does nothing but send the pid over a tokio `mpsc` channel, and the sessions task then runs the check above. `Drop` calls `UnregisterWait` and closes the handle after the callback is known to be finished (a flag). Section 02 uses the same type through `platform::process::open_for_wait(pid, creation_time) -> Option<ExitWatcher>` (foundation 11.6: `register` with a callback that completes a tokio `oneshot`, and the future `ExitWatcher::exited()`), so there is one mechanism for "wait for a process to end" and no thread is parked. This is event driven and costs nothing while the process lives (foundation 11.5, row "process ended"). UNPROVEN in a spike. If `register` fails (access denied for a protected process, API error), nothing breaks: the read-time check and the registry event still find a dead session; only the instant update is lost. A watcher exists for every live record that has a verified pid, at most 40.

### 6.4 Each `ClaudeSession` field and its source

| Field | Source, in order | Notes |
|---|---|---|
| `id` | the Claude `session_id` (hooks) or `sessionId` (registry) | stable across `--resume` (proven) |
| `title` | registry `name` when `nameSource` is `user`; `session_title` of a hook; the last `custom-title` then `ai-title` line of the transcript; the registry `name` when derived; the first user prompt cut to 60 characters; the folder name of `cwd` | the first non-empty wins. The title of a session can change; the id does not |
| `cwd` | the newest event's `cwd`; else the registry `cwd` | |
| `projectId` | `projects.project_id_for_path(cwd)` | absent when no project matches |
| `branch` | the `gitBranch` field of the newest transcript line that has one | no `git` process is started for this (foundation 11.6 limits git calls to the projects module); absent when unknown |
| `model` | `model` of `SessionStart`, `to_model` of `PostModelSwitch`, else the `message.model` of the newest assistant transcript line that is not `<synthetic>`; shortened by `model_label` | `model_label(raw)`: lower-case; if it contains `opus`, `sonnet` or `haiku` the result is that word plus the version digits joined with a dot ("sonnet 4.5" from an id with `4-5`); else the raw text; empty gives "unknown" |
| `status` | 6.3.3 | |
| `startedAt` | registry `startedAt` (epoch ms); else the time of the first event Rain saw; else the first transcript line | |
| `lastActivityAt` | 6.3.3, and the time the transcript file was last written (the only sign of work while the hooks are off) | |
| `tokens` | the context size of the last request: `input_tokens + cache_read_input_tokens + cache_creation_input_tokens + output_tokens` of the newest assistant line, counted once per `message.id` (the transcript writes one line per content block; spike claude-hooks Q5) | read from the tail of the transcript at first sight, after each `Stop`, and whenever the transcript file grew (6.4 below), not on every hook event. The number can lag by a few seconds during a long turn |
| `lastMessage` | `last_assistant_message` of `Stop`; else the newest assistant `text` block of the transcript tail. A session that no hook has spoken for follows the transcript: every read replaces it | 600 characters |
| `request` | the first answerable entry of `pending`, else the first entry | 6.5 |
| `recent` | hook events (6.3.2); after a Rain restart, and for a session that no hook has spoken for, from the transcript tail (the last 8 of: user prompts, assistant text, tool_use) | newest first |
| `host` | registry `entrypoint` `cli` gives `terminal`, `claude-desktop` gives `desktop`, anything else or none `other`; the hook `env.entrypoint` when there is no registry file | |
| `alive` | 6.3.3 | |

The transcript tail read (`transcript.rs`, run through `core::blocking::run`): open read-only, read the last 256 KiB, drop the first partial line, parse each remaining line with `serde_json::Value`, ignore every `type` that is not `user`, `assistant`, `custom-title`, `ai-title` (the other types and `api-request-blob` and `session-owner` lines are discarded unread, research A4). The file may be 13 MB: only the tail is read. A line that does not parse (the last line of a live file) is skipped. When `transcript_path` is missing it is computed from `cwd` (every non-alphanumeric character replaced by `-`, under `<claude folder>\projects`); if that file does not exist the fields stay as they are. Nothing of the text is ever logged.

When the tail is read again (a session with the switch "Monitor Claude sessions" off gets no hook, so the file is its only source): at first sight, after `Stop`, and whenever the transcript grew. The growth is noticed by one `notify` watch (`hub.watch_dir`, non-recursive, quiet period 2 s) on each folder that holds the transcript of a live session; sessions in one folder share the watch, and a watch is dropped when no live session uses its folder any more. After a change the folder is read, then the watch task waits 2 s before it looks again, so a file that Claude writes all the time costs at most one tail read per session about every 2 s (the debouncer alone would hand out an event every 500 ms). A tail is read only when the size or the modified time of the file differs from the last read; a change of another file in the folder costs one `metadata` call per session. `GET /sessions` does the same check for every live session before it answers, so the first paint after a summon is fresh even when an event was missed. A transcript that does not exist yet, or whose folder does not exist yet, is tried again at the next registry change, hook event or `GET /sessions`; nothing polls for it. A session that no hook has spoken for (`hooked` is false) takes model, last message and recent events from every read; a session with hooks keeps what its hooks said and takes only tokens, title, branch and the file time from the transcript. The topic `sessions` is published only when something shown changed, so a hook `Stop` and the watch that see the same write publish once.

### 6.5 The permission flow, end to end

#### 6.5.1 Sequence

```
Claude Code                rain-hook                 backend (sessions)                   UI
 tool needs approval  -> PermissionRequest hook starts rain-hook permission
                          read stdin, open pipe, POST /internal/hook (mode permission)
                                                      1. book: add Pending{id req-xxxxxxxx, tool, summary, detail,
                                                         key = (tool_name, hash of tool_input), tx} ; status = waiting
                                                      2. publish topic sessions  ------------------------------->  row turns "Waiting on you"
                                                      3. wait on tx for up to HOOK_SERVER_WAIT = 19 s
                                                                                                           user presses Ctrl+Y within 20 s:
                                                      <---------------------- POST /sessions/{id}/permission {requestId, answer: once}
                                                      4. find the request, answerable, tx still open -> send the Decision;
                                                         remove it from pending; event "Allowed ..."; publish
                          <---- 200 body = the JSON of 6.5.5 ----
                          PRINT body, exit 0
 command runs
 PostToolUse hook   ->  event: matching pending already removed; nothing more
```

Timing rules:
- The forwarder waits `HOOK_WAIT` = 20 s (decision C2). The backend holds the request for `HOOK_SERVER_WAIT` = 19 s, one second less, so the backend always answers first and the forwarder never has to give up on its own. The installed handler timeout is 35 s (`HOOK_HANDLER_TIMEOUT`, foundation appendix A), so Claude Code never kills the forwarder.
- After 19 s without an answer the handler sets `answerable = false` on the request, publishes, and answers `204` with an empty body. The forwarder exits 0 with empty stdout, and Claude Code shows its own prompt (if it was already showing one, it simply stays). The request remains in `pending`: it is still open in Claude Code (decision C1 says `waiting` comes from an open request).
- If the handler future is dropped early (the forwarder was killed, or Claude Code cancelled the hook because the user answered in the terminal first), a guard object in the handler runs on drop: it sets `answerable = false` and publishes. The row then waits for a clearing event (6.5.6).

#### 6.5.2 Summary and detail

`tool` is `tool_name`. Rules for `summary` (one line, at most 120 characters, trailing "..." when cut) and `detail` (at most 4096 characters; when cut, the last line is `... N more characters`):

| Tool | `summary` | `detail` |
|---|---|---|
| `Bash` | the first line of `tool_input.command`, trimmed | the whole command; when `description` exists it is NOT included (the UI must show exactly what runs) |
| `Edit` | `file_path`, shown relative to `cwd` when under it | `--- <path>\n+++ <path>\n@@ edit @@\n` then every line of `old_string` prefixed `-` and every line of `new_string` prefixed `+` |
| `MultiEdit` | `file_path` plus " (N edits)" | the same, one `@@ edit k @@` block per edit |
| `Write` | `file_path` | `--- <path>\n+++ <path>\n@@ new content @@\n` then the lines of `content` prefixed `+` |
| `NotebookEdit` | `notebook_path` | the `new_source` text |
| `Read`, `Glob`, `Grep`, `WebFetch`, `WebSearch` | the first string among `file_path`, `path`, `pattern`, `url`, `query` | `tool_input` pretty printed |
| an MCP tool (`mcp__server__tool`) | the tool name | `tool_input` pretty printed |
| any other | the first string value of `tool_input`, else the tool name | `tool_input` pretty printed |

The UI already tints a diff by the regular expression `/^(---|\+\+\+|@@)/m` (`RequestCard.vue` line 19), so the diff-like text above needs no UI change. Pretty printing uses a fixed key order (sorted) so the same input always gives the same text.

#### 6.5.3 Tools that need a typed answer

`AskUserQuestion` and `ExitPlanMode` cannot be answered by a bare allow (they need `updatedInput`, research A2.1). For these the hook route answers `204` at once (the forwarder exits and Claude Code shows its own dialog) and the book adds a request with `answerable = false`, `summary` = the tool name, `detail` empty. The row says "waiting" and the card says to answer in the terminal.

#### 6.5.4 What the UI may do

The UI shows Allow once, Always allow and Deny only for a request with `answerable = true`. With `answerable = false` the card shows one line, "Answer this in the terminal" (for `host = desktop`: "Answer this in the Claude app"). Focus (Enter) still works.

#### 6.5.5 The answer: Claude Code's JSON

The backend builds the text that the forwarder prints:

```json
// once
{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}
// deny
{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denied from Rain","interrupt":false}}}
// always (session only; the rule is built by 6.5.7)
{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow",
  "updatedPermissions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"npm install"}],"behavior":"allow","destination":"session"}]}}}
```

Proven in the spike (Q2): the documented allow JSON runs the command; deny refuses and the message reaches Claude; allow with `updatedPermissions` `addRules` and destination `session` ran, the next matching command fired no request, and nothing was written to any settings file. `onFailure` is never set (it would turn a failure into a denial, research A1.5).

#### 6.5.6 Clearing a request

A pending request is removed when (any of):
1. the user answers through Rain (6.5.1, step 4);
2. `PostToolUse`, `PostToolUseFailure` or `PermissionDenied` arrives for the same session with an equal `tool_name` and an equal hash of `tool_input`. There is no `tool_use_id` on `PermissionRequest`, so this is the only match; the hash is over the input JSON with keys sorted recursively, so the key order of the two events does not matter;
3. `Stop`, `StopFailure`, `SessionEnd`, or `UserPromptSubmit` arrives (the answerable ones are already gone or being answered; for the others the user has moved on);
4. the registry says `status` is no longer `waiting`, its `statusUpdatedAt` is later than `requested_at` plus 2 s, and the request is not answerable (a defensive rule for the case where the user denied in the terminal and no hook followed; UNPROVEN);
5. the process of the session is gone (6.3.3).

An answerable request is never removed by rule 4: the open connection of its forwarder is the truth.

#### 6.5.7 "Always allow" (decision: session only)

`Always` never writes any file. The rule sent to Claude Code has `destination: "session"`, which Claude Code keeps in memory for that session (proven, Q2). Choosing the rule, in this order (the first that applies):

1. The first entry of `permission_suggestions` whose `type` is `addRules` and whose `behavior` is `allow`: sent as it is except that `destination` is forced to `"session"`. `rule_text` = `<toolName>(<ruleContent>)`, or `<toolName>` without content.
2. Tool `Bash`: `{type: addRules, rules: [{toolName: "Bash", ruleContent: <the exact command>}], behavior: allow, destination: session}`. The rule is the exact command text, so it never allows more than the user saw. A command that contains `*` is not offered as an always rule (the star is a wildcard in rules): the answer degrades to `once`.
3. An edit tool (`Edit`, `Write`, `MultiEdit`, `NotebookEdit`): ONLY when `permission_suggestions` contains an entry of type `setMode`: it is copied with `destination` forced to `session`, and `rule_text` = "Accept edits for this session". Rain does not build a `setMode` entry itself: only `addRules` with destination `session` was proven (Q2), so an invented `setMode` shape could make `applied: always` a false claim. Without such a suggestion the answer degrades to `once` with the message of rule 5. Gate 1 item G1.12 checks the shape; until it passes, `applied` is `once` for edit tools even when a suggestion exists.
4. An MCP tool: `addRules` with the tool name and no `ruleContent` (research: a content on an MCP rule matches nothing). `rule_text` = the tool name.
5. Anything else: no rule can be made. The answer is sent as `once` and `PermissionAnswerResult.applied` is `once`; the UI says "Allowed once. No session rule was possible."

Suggestions of the types `addDirectories`, `setMode` (other than the copy of rule 3) and `replaceRules` are not used. Rain never uses a destination other than `session`, and never `bypassPermissions`.

#### 6.5.8 Several requests at once

A session can have more than one open request (parallel tool calls, subagents). `pending` is ordered by arrival. `ClaudeSession.request` is the first answerable one, else the first. After an answer the next request shows at once and the status stays `waiting`. The UI sends the request id it shows; an answer for any other id is `conflict` `requestExpired`, so a request that changed between render and key press is never approved blind (contract report P-S2b).

#### 6.5.9 Everything that can go wrong with an answer

| Case | Result |
|---|---|
| answer arrives after the forwarder gave up (`answerable = false`) | `409 conflict`, reason `requestExpired`, message "That request is no longer open. Answer it in the terminal." |
| the request was cleared by an event, or the id is unknown | same |
| the monitoring switch is off (no hooks installed) | `409`, reason `monitoringOff`, message "Monitoring is off. Turn on Monitor Claude sessions in Settings." |
| two quick answers for one request | the first wins, the second gets `requestExpired` |
| the sender dropped (`tx.send` fails) | `requestExpired` |
| the backend stops while a forwarder waits | the pipe closes; the forwarder exits 0; Claude Code shows its own prompt |

### 6.6 `focusSession`

Result: `window` (a specific window of the session's host was brought to the front and verified), `app` (the host application was brought to the front, but not the session's tab or conversation), `none` (nothing could be done; not an error, decision C12). The UI hides Rain after `window` or `app`, and shows a notice after `none` (9.1).

Steps (module `sessions/focus.rs`, Win32 calls in `platform/windows_focus.rs`):

```
1. find the session; unknown -> 404. Ended session or no verified pid -> none.
2. chain = the session's pid and its ancestors from one Toolhelp snapshot (at most 12 levels), stopping before explorer.exe,
   services.exe, the Rain shell and the Rain backend itself.
3. candidates = visible top-level windows (EnumWindows, IsWindowVisible, no tool windows) whose owning pid is in chain.
   For a terminal session the chain reaches the terminal program (Windows Terminal, conhost, an editor with a terminal).
   For a desktop session it reaches the Claude desktop application.
4. none -> result none.
5. choose: the candidate whose title contains the session's folder name or title; else the first candidate in z-order.
6. bring it to the front: ShowWindow(SW_RESTORE) when minimised; SetForegroundWindow; then check GetForegroundWindow() == chosen within 150 ms.
   If not: AttachThreadInput trick, check again; if not: a synthetic Alt key press (keybd_event) then SetForegroundWindow, check again.
   If still not in front: FlashWindowEx on the chosen window and return none.
7. result: window when the host is terminal and (the title matched or there was exactly one candidate);
           app otherwise (desktop host, or a window picked without a title match).
```

Why the extra steps in 6: `SetForegroundWindow` is honoured only for the foreground process or a process that received the last input. The backend is neither (the window belongs to the shell process). This is UNPROVEN in both directions; the sequence tries the three known ways and reports the truth (`none`) when none works. If the acceptance check F1 of section 11 fails on the owner's PC, open point 12.9 describes the fallback (the shell calls `AllowSetForegroundWindow` for the backend before it forwards a focus request).

What focus cannot do: select a Windows Terminal tab, or a conversation inside the desktop app (no deep link exists, research A5). That is why the result has `app`.

### 6.7 `stopSession`

Allowed only for a live session of host `terminal` (and for `other` hosts with a verified pid). The desktop host is refused: how the desktop application reacts when its child process is killed was never seen, and the app may close with it. Rain never kills the user's work by guess.

```
1. find the session; unknown -> 404. Already ended -> 204 (idempotent).
2. host desktop -> 409 notStoppable, message "This session runs inside the Claude desktop app. Close it there."
3. no pid, or the check of 6.3.3 fails (pid reused or gone) -> 409 notStoppable, "Rain cannot verify this session's process. Close it in its terminal."
4. the image name of the process (QueryFullProcessImageNameW, base name) must be claude.exe or node.exe; else 409 notStoppable (the pid does not belong to Claude).
5. one Toolhelp snapshot: collect the descendants of the pid whose creation time is not earlier than the root's (guards against reused pids).
6. TerminateProcess(root), then TerminateProcess of each descendant. Wait up to STOP_WAIT = 2 s for the root's handle.
7. stopped_by_rain = true; add the recent event "Stopped from Rain"; the exit watcher (or the next check) finishes the record as success.
8. 204. On a Win32 failure: 500 failed, reason stopFailed, message "Could not stop the session".
```

There is no graceful external stop for an interactive session (research A5: no signal reaches Claude on Windows). The transcript stays and the session can be resumed later with `claude --resume`. The terminal tab keeps its shell prompt.

### 6.8 `getSessionTranscript`

Source: the transcript file of the session (`transcript_path` of the record, or computed, 6.4). Result: `LogLine[]`, oldest first, at most 1000 lines.

Reading: read the last 2 MiB; parse lines; if fewer than 200 log lines came out and the file is larger, read 8 MiB more, up to 16 MiB in all. (A transcript can be 13 MB; the cost is one read of at most 16 MiB and a parse, well under the 10 s aim, on a blocking thread.) A first line `warn` says "N earlier lines are not shown" when the cap cut anything.

Mapping, line by line in file order (research A4):

| Transcript line | Output |
|---|---|
| `user` whose `message.content` is a string, or has `text` blocks, and whose `origin.kind` is `human` or absent | `info` "You: <text>" |
| `assistant`, `text` block | `info` "Claude: <text>" |
| `assistant`, `tool_use` block | `info` "<name>: <summary of input>" (the summary rules of 6.5.2) |
| `user`, `tool_result` block with `is_error: true` | `error` "  <first line of the result>" |
| `assistant` with `isApiErrorMessage` or `apiError` | `error` with the first line of the text |
| `system` with `subtype` `compact_boundary` | `warn` "Conversation was compacted" |
| thinking blocks, `isMeta`, `isCompactSummary`, `isVisibleInTranscriptOnly`, `isSidechain: true`, every bookkeeping type | skipped |
| a non-error `tool_result` | skipped (it holds file contents; the transcript here is a readable summary, not a copy) |

`ts` = the line's `timestamp`. Every text is cut to 1000 characters. The last line says how the session stands: `waiting` gives `warn` "Waiting for your answer: <tool> <summary>"; `failed` gives `error` "Session stopped after an error"; `success` gives `info` "Session ended"; `idle` gives `info` "Waiting for your next prompt"; `running` gives `info` "Working". Unknown line types are ignored, and a file that cannot be read gives `notFound` `noTranscript`.

---------------------------------------------------------------------------------------------------
### 6.9 The hook installer (`sessions/hooks_install.rs`)

The only code that may name or touch the user-level Claude settings file (foundation 13.3 rule 2). It is the tested merge routine of spike claude-hooks Q7 (`S\claude-hooks\merge\src\main.rs`, 595 lines, 11 tests) moved into the module and adapted as described here. Copy the file and its tests first, then adapt.

#### 6.9.1 What it touches

| File | Path | Access |
|---|---|---|
| the user-level Claude settings | `<claude folder>\settings.json` (6.1) | read always (status); written only by `install`, `uninstall`, `repair` and only when allowed by 6.9.8 |
| the forwarder | `<local>\bin\rain-hook.exe` (foundation appendix B), copied from `rain-hook.exe` next to the backend exe | written by the installer |
| backups of the settings file | `<local>\backups\claude-settings\settings.json.<UTC stamp>` (12.4 asks for this folder), the newest 10 kept | written by the installer |

No other Claude file is ever written. `<claude folder>\backups` is Claude Code's own and is not used.

#### 6.9.2 The JSON that is merged

For each of the 12 events Rain adds ONE matcher group at the end of that event's array (the array is created if absent). `<fwd>` is the absolute forwarder path with doubled backslashes; `<id>` is the install id (6 hex characters, made at the first install, reused afterwards); `[--pipe]` appears only when the backend's pipe is not the default real name (6.2.1).

```json
{
  "hooks": {
    "SessionStart": [
      { "matcher": "", "hooks": [
        { "type": "command", "command": "<fwd>", "args": ["event", "--rain-managed=<id>"], "async": true } ] } ],
    "PermissionRequest": [
      { "matcher": "", "hooks": [
        { "type": "command", "command": "<fwd>", "args": ["permission", "--rain-managed=<id>"], "timeout": 35 } ] } ],
    "SessionEnd": [
      { "matcher": "", "hooks": [
        { "type": "command", "command": "<fwd>", "args": ["event", "--rain-managed=<id>"] } ] } ]
  }
}
```

The full set:

| Event | Mode | `async` | Why it is installed |
|---|---|---|---|
| `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PermissionDenied`, `Stop`, `StopFailure`, `PostModelSwitch`, `CwdChanged` | `event` | true | observation; Claude never waits for it (proven: async works for every one of these events, spike Q3) |
| `SessionEnd` | `event` | no | its budget is 1.5 s and the forwarder needs about 20 ms; whether an async hook survives the exit of an interactive session is UNPROVEN, so it stays synchronous (as in the spike) |
| `PermissionRequest` | `permission` | no | has to return a decision; `timeout` 35 s (`HOOK_HANDLER_TIMEOUT`) |

`Notification` is not installed (6.3.2). `matcher` is `""` (all). `onFailure` is never set. `if` is never used (a hook with `if` on a non-tool event never runs). Exec form (`args` present) is used so that a missing Git Bash cannot block a tool call (research A1.5): the program must be a real `.exe`, which `rain-hook.exe` is.

Cost: every tool call fires `PreToolUse` and `PostToolUse`, so about 2 forwarder starts of about 20 ms each per tool call, in the background. This is accepted (measured by gate item G1.9). If the cost shows in practice, the one lever is to drop `PostToolUse` (then edit events and request clearing degrade to `Stop`).

#### 6.9.3 Marking and finding Rain's entries

The marker is the extra argument `--rain-managed=<id>` in `args`, not a custom JSON field (an unknown field in a handler was never tested). An entry is Rain's when its `args` contain a string that starts with `--rain-managed=` or its `command` base name is `rain-hook.exe`. Removal deletes exactly those handlers, then the matcher groups that became empty, then empty event arrays, then an empty `hooks` object. A user handler inside a Rain group survives (tested). Rain never modifies, reorders or removes an entry it did not create.

#### 6.9.4 The routine (all steps are in the spike code, with tests)

```
install(settings_path, backup_dir, spec):
  1. read bytes (a missing file means "{}"), remember length, mtime and the bytes
  2. parse as strict JSON (serde_json Value, feature preserve_order so key order survives).
     Refuse (change nothing) on: invalid JSON; a root that is not an object; `hooks` that is not an object; an event value that is not an array;
     `disableAllHooks` equal to true.
  3. remember the format: indent (spaces or tabs), line ending (LF or CRLF), BOM, trailing newline; render with the same
  4. remove every existing Rain handler (any install id), then add the 12 handlers of 6.9.2 (so a changed forwarder path replaces, never duplicates)
  5. if the result equals the input after removal and addition -> "unchanged", write nothing
  6. copy the original bytes to backup_dir (first write only per run), prune to the newest 10
  7. write `settings.json.rain-tmp` in the same folder, flush, then replace the target (MoveFileExW replace); retry 4 times on a sharing violation
  8. just before the rename re-read the target; if length or mtime changed since step 1 (Claude Code or the user wrote it), start again from step 1 (MAX_ATTEMPTS = 4)
  9. read the file back and check: removing Rain's entries from it gives the original structure (deep compare) and all 12 entries are present
```

`uninstall` is steps 1, 2 (without the `disableAllHooks` refusal), the removal of 4, and 6 to 9. `status` is steps 1 and 2 and a count. All three are plain functions that take paths and return plain data (`Outcome`, `Status`); the module wraps them in `core::blocking::run`.

Limits that stay (documented in the spike): `serde_json` is used with the feature `preserve_order` but WITHOUT `arbitrary_precision` (12.3 explains why); a number written with an exponent or a very long fraction is rewritten in its normal form (same meaning, other bytes), and a `\uXXXX` escape is written as the character. A normal Claude settings file has neither. Everything else is byte-identical after install and uninstall on a copy (test `install_then_uninstall_gives_back_the_same_bytes`).

#### 6.9.5 The forwarder file

Before the merge, `ensure_forwarder()` copies `rain-hook.exe` (next to the backend exe) to `<local>\bin\rain-hook.exe`: skip when both files are byte-equal; else write `<local>\bin\.rain-hook.exe.tmp`, then rename over the target (a forwarder that runs at that instant makes the rename fail with a sharing error; retry 8 times, 25 ms apart; if it still fails the old file stays and the status says "forwarder out of date"). Then run `rain-hook.exe selftest` with a 2 second limit; failure is `failed` reason `hookMissing`. The path must be stable and outside any folder that Claude Code or Rain updates (research A7 step 11); a path that contains spaces is fine in exec form (UNPROVEN, gate item G1.11).

#### 6.9.6 The switch

`install_hooks()` (switch on) and `remove_hooks()` (switch off), called by section 06 before it writes `monitor_claude`:

```
on:   1. allowed(target)? else conflict, reason sandbox        2. the claude folder must exist else unavailable, "Claude Code was not found on this PC"
      3. ensure_forwarder()                                     4. install()                         5. status(true) must be ok else failed, reason installFailed
      return Ok(())
off:  1. allowed(target)? else conflict sandbox                 2. uninstall()                        3. status(false) must find no Rain entry else failed, reason installFailed        4. return Ok(())
```

`status(expected_on: bool)` is told what the switch is meant to be; it does not read `settings.toml`. This is why the two calls work while the key still holds its old value (section 06 writes the key only after they succeed): `install_hooks()` calls `status(true)`, `remove_hooks()` calls `status(false)`. Only module start and `hooks_state()` pass `Resolved.monitor_claude`.

An error aborts the settings write (the setting keeps its old value). The message is shown by the Settings screen as the notice text, so it is a plain sentence:

| Refusal | `code`, `reason` | Message |
|---|---|---|
| invalid JSON | `failed` `installFailed` | "Your Claude settings file is not valid JSON. Rain did not change it." |
| unexpected shape | `failed` `installFailed` | "Your Claude settings file has an unexpected shape in its hooks section. Rain did not change it." |
| `disableAllHooks` | `conflict` `hooksDisabled` | "Claude hooks are switched off in your Claude settings (disableAllHooks). Turn them on first." |
| cannot write | `failed` `installFailed` | "Could not write the Claude settings file." |
| no Claude folder | `unavailable` `noClaude` | "Claude Code was not found on this PC." |
| forwarder missing or selftest failed | `failed` `hookMissing` | "The Rain hook program was not found or did not start." |
| sandbox | `conflict` `sandbox` | "A development build never changes your real Claude settings." |

#### 6.9.7 Verification on start, and repair

At backend start (module `start`, after the registry scan) and on every `hooks_state()` call, `status(Resolved.monitor_claude)` runs (reads one small file; no timer, no watcher on `settings.json`). Start only reads and verifies; it never writes the Claude settings file (decisions.md section 4: the switch is the only installer):

| Switch | Found | Action |
|---|---|---|
| on | all 12 entries, forwarder present and equal | state `ok` |
| on | entries missing or changed, or forwarder missing or older | state `needsRepair` (`HookState::Broken`), `Health.sources` `claude-hooks` = `failed` with the message; nothing is changed at start. The Settings Repair action (`monitorClaude: true`, which calls `install_hooks()`) repairs it |
| on | `disableAllHooks`, bad JSON or shape | state `blocked`, source `failed` with the message of 6.9.6; nothing is changed |
| off | Rain entries present (the owner edited `settings.toml` by hand) | state `off`; nothing is changed; section 06 reports the mismatch as a `ConfigIssue` (06 6.2) |
| off | none | state `off`, source `off` |

A repair is `install_hooks()` called again, which section 06 does when the user sends `monitorClaude: true` while the state is `needsRepair` (the "Repair" action of the Settings row is section 06's). Claude Code rewrites its settings file itself at times (for example when the user picks an always-allow with a persistent destination); a repair is the answer when that drops Rain's entries.

#### 6.9.8 Sandbox rule (foundation 2.7, 13.3)

`allowed(target)` is true when `!sandbox`, or when the canonical target path lies inside the system temp folder or inside the data folder or the local folder of this run. A sandbox run (debug build, test, any `RAIN_*` override) therefore never writes the real file: `<claude folder>` resolves to the real folder unless `Config.claude_dir` (tests) or `RAIN_CLAUDE_DIR` (manual runs, 12.2) points into a temp folder. Reading the real registry and the real transcripts in a sandbox run is allowed (read only; it is how stage 8a is developed against the owner's real sessions). The scan `source_rules.rs` additionally fails the build if `settings.json` appears outside this file.

#### 6.9.9 Running sessions, and what stays behind

- Claude Code is documented to pick up edits of its settings files by a file watcher. Whether a session that is already running (terminal or desktop) loads Rain's new hooks live was never seen (UNPROVEN, gate item G3.4). Until a session fires its first event Rain knows it only from the registry (6.1).
- A session in a folder that was never trusted may hold back all hooks until trust is accepted (docs; the spike ran headless only). Such a session shows from the registry alone, with status `idle` or `running` and never `waiting`.
- After `uninstall` the forwarder stays in `<local>\bin` so that a session that loaded the hooks earlier and keeps running them finds a program (no "hook error" notice). It is replaced only by `ensure_forwarder`. With the switch off the route answers `204` and ignores the event.
- Game mode ends the backend; every hook then exits 0 at once (6.2.1).

### 6.10 The gate: what must be checked by hand before the switch is turned on on the real PC

Decision (decisions.md section 4): the switch is turned on on this PC only after the feature is built and tested, and the user-level Claude settings file is never touched while developing. The automated tests of section 10 prove the installer and the forwarder on throwaway files. Interactive behaviour of Claude Code was never seen (foundation appendix C). The gate below proves it in three steps, from the safest to the most real. Do the steps in order; stop at the first failed item and read its "if different" note.

The key to Gate 1 is that no real settings file is changed: Claude Code accepts an extra settings file on its command line (`claude --settings <file>`, documented), and hooks given that way run in interactive sessions. A sandbox backend (debug build, `RAIN_CLAUDE_DIR` set to an empty temp folder) writes the throwaway file with the same installer that the real run uses.

#### Gate 0: before any hand check (automatic)

- `npm run check` is green; the tests `install_then_uninstall_gives_back_the_same_bytes`, `hook_budget`, `envelope_matches_forwarder`, `permission_roundtrip` (section 10) pass.
- `rain-hook.exe event` with no backend running: wall time under 50 ms, exit code 0, empty stdout (measured, release build).

#### Gate 1: a throwaway settings file and a terminal session (CLI only; no real file changed)

Setup: create an empty folder `T` under the temp folder. Start the sandbox backend with `RAIN_CLAUDE_DIR=T`. Call `PATCH /settings` with `monitorClaude` true (through the browser dev page, Settings, the switch). Open `T\settings.json` and see the 12 entries. In a normal terminal, in a new empty scratch folder, run `claude --settings T\settings.json`. Use a model that is cheap. Rain shows the session from hook events (the registry of the real folder is not read in this setup).

| # | Do | Expect | If different |
|---|---|---|---|
| G1.1 | start the session, accept the folder trust dialog if shown, type a prompt | Rain lists the session within 2 s; status working then idle; title and folder right | no session at all: hooks do not run from `--settings` or from a fresh folder: record which; Gate 2 and 3 then decide |
| G1.2 | ask Claude to run a command that needs approval (for example `mkdir probe1`) | the Rain row turns to "Waiting on you" with the exact command; look at the terminal at the same moment and WRITE DOWN whether Claude's own dialog is visible while Rain waits | if the terminal shows no dialog while Rain waits: keep `HOOK_WAIT` at 20 s (the user has 20 s to answer in Rain); if that is too long to stare at nothing, lower `HOOK_WAIT` (12.7) |
| G1.3 | press Ctrl+Y in Rain within 20 s | the command runs; the terminal shows no "hook error" line; Rain shows "Allowed" | a "hook error" notice appears: the forwarder printed something or failed: read `hook.log` |
| G1.4 | trigger another request, do nothing for 25 s | the Rain card changes to "Answer this in the terminal"; the terminal now shows its own dialog (if it was hidden before it must appear now) | the terminal never shows its dialog: the hook blocks it: lower `HOOK_WAIT` to 8 s and repeat; if it still blocks, stop and report (blocker for Gate 3) |
| G1.5 | answer that dialog in the terminal | the Rain row clears within 2 s | stays: the clearing rule 6.5.6(2) did not match; record the two events from the debug log (event names only) |
| G1.6 | trigger a request and answer it in the terminal within 10 s | Rain's row clears within 2 s; no stuck forwarder process (check Task Manager: no `rain-hook.exe`) | a forwarder is left behind: Claude Code did not cancel it; it ends by itself at 20 s; record |
| G1.7 | trigger a request, then press "Always allow", then trigger the same command again | the second one runs with no request and no prompt; `T\settings.json` and the real settings file are unchanged (compare file times) | the second one asks again: the session rule did not apply (the rule text is shown in the notice; copy it) |
| G1.12 | ask for an edit approval (for example an `Edit` of a scratch file), press "Always allow" in Rain, then ask for another edit | the next edit shows no request and no prompt (the copied `setMode` suggestion worked); if it asks again, the shape does not work: leave edit tools at `once` | the notice must not have claimed "Accept edits for this session" before this item passes |
| G1.8 | stop the backend (`npm run backend:stop`), then run a command that needs approval | the terminal's own dialog appears at once, no delay, no "hook error" notice | any delay or notice: the fail-open path is wrong: blocker |
| G1.9 | run a prompt with about 20 tool calls | no visible slowdown of Claude; Task Manager shows no lasting CPU use from `rain-hook.exe` | slowdown: drop `PostToolUse` (6.9.2) and repeat |
| G1.10 | press Esc during a long answer | record the Rain status after 5 s (`running` or `idle`) | `running` stays forever: the Esc rule 6.3.3 does not match; leave it (the UI marks it stale after 60 s) |
| G1.11 | run the same from a folder whose path contains spaces, and with the forwarder under a user profile path that contains a space (if the profile has none, copy the forwarder to a folder with a space and edit `T\settings.json` by hand) | events still arrive | none arrive: exec form with a spaced path fails: use the 8.3 short path in `command` |
| G1.12 | exit with `/exit`, then with Ctrl+C twice | Rain shows the session under "Done" (a `SessionEnd` arrived), or "Failed" only when killed | a clean exit shows "Failed": the async or sync `SessionEnd` did not arrive: the liveness check still ends it; record |
| G1.13 | in a folder that was never trusted, start a session and do not accept trust yet | record whether events arrive before trust | none before trust: matches the documentation; fine |
| G1.14 | in the session type `/hooks` | Rain's 12 entries are listed | missing: the file was not merged as expected |

#### Gate 2: the real registry, read only, switch still off (stage 8a)

Start the sandbox backend with no `RAIN_CLAUDE_DIR` (it reads the real `<claude folder>\sessions` and transcripts, read only; the installer is refused in this mode).

| # | Do | Expect |
|---|---|---|
| G2.1 | have one terminal session and one desktop-app session open | both appear, with the right folder, title, host (`terminal`, `desktop`), `idle` or `running` |
| G2.2 | press Enter on the terminal session | its terminal window comes to the front, Rain hides, result `window` (record the way it needed: step 6 of 6.6 that finally worked) |
| G2.3 | press Enter on the desktop session | the desktop app comes to the front, result `app` |
| G2.4 | open the transcript of each | readable lines, no raw JSON, finishes within 1 s |
| G2.5 | close the terminal session by typing `/exit` | the row moves to "Done" within 2 s; closing the terminal window instead shows "Failed" |
| G2.6 | Ctrl+K on the desktop session | "Stop session" is not offered; if forced through the API it is refused with the message of 6.7 |
| G2.7 | Ctrl+K on the terminal session, Stop, confirm | the process ends, the shell prompt returns in that terminal, the row shows "Done" |

#### Gate 3: the real file, switch on (only after Gates 1 and 2 passed)

Do this when no important Claude work is open. The installer keeps a backup of the original file and the removal is tested to give back the same bytes.

| # | Do | Expect |
|---|---|---|
| G3.1 | copy the real settings file by hand to a safe place (a second backup, outside Rain's) | |
| G3.2 | in the real (release) Rain: Settings, turn on "Monitor Claude sessions" | the row shows ok; the file has exactly 12 Rain entries added and nothing else changed (compare with the copy by eye or `fc`) |
| G3.3 | start a NEW terminal session; repeat G1.2 to G1.8 | same results as Gate 1 |
| G3.4 | in a terminal session that was already open before the switch, run a command that needs approval | record whether Rain sees it (live reload of hooks). If not, a restart of that session is needed (the Settings hint says so) |
| G3.5 | in the desktop app start a new Code session and run a command that needs approval | the Rain row shows the request; the desktop dialog is visible meanwhile; answer in Rain: the command runs; then trigger another and answer in the desktop: Rain's row clears. WRITE DOWN which one wins if both are pressed |
| G3.6 | turn the switch off in Settings | the 12 entries are gone; the file equals the copy of G3.1 byte for byte (`fc /b`) |
| G3.7 | turn it on again and leave it on | normal use; keep the first week's `hook.log` in mind: it must stay empty or near empty |

Outcomes that change a constant or a rule (each is a one-line change): `HOOK_WAIT` (G1.2, G1.4), dropping `PostToolUse` (G1.9), refusing the desktop host for permissions (G3.5 shows the desktop never delivers requests to hooks: then the feature is documented as terminal only and the Settings hint says so), the sync or async `SessionEnd` (G1.12). A blocker (G1.4 with a dialog that never appears, G1.8) means the switch stays off and the hook feature is not released; the registry and transcript features of 8a stand on their own.

---------------------------------------------------------------------------------------------------
### 6.11 Scheduled jobs

#### 6.11.1 Loading and the in-memory model

`jobs::store` owns `TomlFile<JobsFile>` for `jobs.toml` (foundation 10.4) and the SQL of section 5. `jobs::Service` keeps:

```rust
struct JobsState {
    defs: Arc<Vec<JobDef>>,                    // validated entries, file order
    runtime: HashMap<String, JobRuntime>,      // by job id
}
struct JobRuntime {
    recent: VecDeque<JobRun>,                  // newest first, at most 20; loaded from jobs_run at start
    live: Option<LiveRun>,                     // the run in progress: its OwnedJob, cancel token, log buffer, the occurrence it came from (`due`)
    handled_until: SystemTime,                 // mirror of jobs_state
}
```

`ScheduledJob` is built from `JobDef` plus `JobRuntime`: `running = live.is_some()` and `recent_runs[0].status == running` are kept in step because both are set in the same critical section (the UI relies on both, contract report 3.2). `list()` costs no I/O. A reload of `jobs.toml` (watcher event, `TomlFile::subscribe`) keeps the runtime of every id that still exists, drops the runtime of removed ids (a live run of a removed job finishes normally; its log stays reachable by run id), and wakes the scheduler.

#### 6.11.2 Cron and the schedule label

- Parser: `croner` 5.0.0 with `chrono` 0.4.45 (foundation 1.6), `Cron::from_str` on the five-field text after the field count was checked. All `chrono` use stays inside `modules/jobs/` (foundation 1.6); the API carries strings.
- Zone: the PC's local zone (`chrono::Local`). Proven behaviour (spike crates Q5, tested with named zones, not this PC's zone): `0 6 * * *` stays at 06:00 local across a DST change; a time in the spring-forward gap runs once at 03:00; a time in the repeated fall-back hour runs once; `*/15` jumps from 01:45 to 03:00.
- The day-of-month and day-of-week combination (when both are restricted classic cron runs on either) is pinned by the unit test `dom_dow_rule` so a library change cannot go unnoticed.
- `nextRunAt` = the first occurrence after now, as an `ISODateTime` in UTC with `Z` and three fraction digits (foundation 4.3), absent when the job is disabled.
- `scheduleLabel`: `label` of the TOML when present, else `cron_label(expr)`, the hand-written function of the spike (`S\crates\src\cron_label.rs`, 150 lines, tested): "Every day at 06:00", "Weekdays at 09:30", "Every 15 minutes", "Every minute", "Every hour", "Every hour at :05", "Every 2 hours", "Mondays at 06:00", "Weekends at 10:00", "Mon, Wed and Fri at 10:00", "Monthly on the 1st at 00:00", "Every day at 09:00 and 17:00"; anything else falls back to the library's `describe()` without its full stop; invalid text is returned unchanged. Copy the file and its tests.

#### 6.11.3 The scheduler (timer T2, foundation 11.5)

One task, one `sleep` at a time, no tick.

```
loop {
    if game_mode_active {                // no `due`, no sleep branch: nothing to wait for but a change
        select! {
            _ = changed.notified()      => continue     // `pause(false)` and the end of game mode call `nudge()`; the loop then recomputes `due` and runs `plan_catch_up`
            _ = resumed.notified()      => { arm_catch_up_delay(); continue }
            _ = shutdown.cancelled()    => break
        }
    }
    let now = clock.now();
    let due = earliest next occurrence over all enabled jobs, counted after each job's handled_until   // pure function, unit tested
    select! {
        _ = sleep(due - now)            => fire every job whose occurrence <= now + 500 ms      // 6.11.4
        _ = changed.notified()          => continue     // jobs.toml reloaded, enable toggled, a run ended, GET /jobs or a stream (re)connect nudged us
        _ = resumed.notified()          => { arm_catch_up_delay(); continue }                   // the PC woke from sleep: ONE `sleep(CATCHUP_DELAY)`, then `plan_catch_up(now)` (6.11.5)
        _ = catch_up_delay              => { plan_catch_up(clock.now()); continue }             // only exists while armed (backend start or resume)
        _ = shutdown.cancelled()        => break
    }
}
```

- The sleep is computed from the wall clock and recomputed on every wake, so a long sleep cannot drift: after any wake the task recomputes from `clock.now()`. A wake earlier than the due time (the monotonic timer fired before the wall clock caught up) simply loops.
- Resume from sleep: `platform::power::on_resume(callback)` registers `PowerRegisterSuspendResumeNotification` with a callback (no window and no message loop needed); the callback only calls `Notify::notify_one`. It is an event source, not a timer, and costs nothing while the PC runs. UNPROVEN in a spike. If the registration fails, the scheduler degrades to a sleep capped at 5 minutes and logs once at `warn`; 12.5 asks the architect to accept this degraded mode.
- Clock changes by the user or by time synchronisation raise no event on Windows. They are healed by the next nudge: every `GET /jobs` and every stream (re)connect calls `scheduler.nudge()`, which makes the task recompute. This is free (no timer).
- While `game_mode_active` is true the task starts nothing and advances nothing (the occurrence stays unhandled until `pause(false)` after a cancelled game mode or the next start, whichever comes first, 6.11.5). In that state the loop does not compute `due` and has no sleep branch: a `due` in the past would make `sleep(due - now)` return at once, the fire would be refused, and the loop would spin at full CPU for the whole review (up to 10 minutes). It waits only on `changed`, `resumed` and `shutdown`. `pause(false)` and the end of game mode call `nudge()`; after the nudge the loop recomputes `due` and runs `plan_catch_up`. Test `scheduler_does_not_spin_in_game_mode`.

#### 6.11.4 Firing, overlap, enabling

For each job whose occurrence `due <= now + 500 ms`:
1. `handled_until = due` (written to `jobs_state`, one row update).
2. If a run of this job is in progress: skip, log at `info` ("job X still running, occurrence skipped"). No run record is made: the contract has no representation for "skipped" and the UI cannot show it. (Decision: skip if still running, like `runJob` of the mock.)
3. Else start a run, trigger `schedule` (6.11.7).

`runJob` (manual): unknown id `notFound`; a run in progress: return that run with status 200 (no second run); `game_mode_active`: `conflict` `gameMode`; else start a run, trigger `manual`, return it with 201. A disabled job may be run (explicit intent). A manual run does not change `handled_until` and does not move `nextRunAt`.

`setJobEnabled(id, true)`: write `enabled = true` (4.1.1); set `handled_until = now` (so enabling a daily 06:00 job at 10:00 does not start it at once); publish. `false`: write; publish; a run in progress continues (as in the mock).

#### 6.11.5 Missed runs (decision of 2026-11-22: run once at the next start)

`jobs_state.handled_until` per job: every occurrence up to that instant has been handled (run, skipped as overlap, or deliberately not run). Rules:

| Moment | Rule |
|---|---|
| first time a job is seen enabled (no `jobs_state` row) | `handled_until = now`; no catch-up for the past before the job existed |
| enable | `handled_until = now` (6.11.4) |
| scheduled fire | `handled_until = due` |
| 60 s after backend start, and 60 s after a resume from sleep (`CATCHUP_DELAY`, so the network is up; one `sleep(CATCHUP_DELAY)` armed at start and on `resumed`) | for every enabled job with `catch_up = true`: if at least one occurrence lies in `(handled_until, now]`, start ONE run with trigger `catchup` (however many occurrences were missed); then `handled_until = now`. A job with `catch_up = false`: only `handled_until = now` |
| a scheduled or catch-up run ended with status `idle` (stopped on purpose by game mode, a stop or a restart) | `handled_until` is set back to just before that run's occurrence, so the next start runs it once more. A manual run does not do this |
| `pause(false)` (game mode cancelled; called after `game_mode_active` was cleared, so the nudge to the scheduler and the idle sampler are not refused) | run `plan_catch_up(now)` at once, without `CATCHUP_DELAY`: one run with trigger `catchup` per enabled job with `catch_up = true` that has an occurrence in `(handled_until, now]`; then `handled_until = now` |
| game mode active | no start, no advance |

A job missed for ten days runs once, not 480 times. A weekly job missed three days ago runs once at the next start. Examples are the test names `catchup_runs_once_after_long_off`, `catchup_skips_job_enabled_after_the_miss`, `catchup_after_game_mode_stop`.

#### 6.11.6 Start-up reconcile

At `start`: open the TOML; load `recent` (20 per job) from `jobs_run`; every row with status `running` (a crash, a kill, or a stop that could not write) is closed as `failed` with `ended_at = now` and one added log line `error` "Rain stopped while this run was in progress" (contract report P-J6); then arm the scheduler, and arm the catch-up delay (`plan_catch_up` runs 60 s later, 6.11.5; only `pause(false)` runs it without the delay). At `stop()` of a normal backend shutdown, runs in progress are terminated (the job handle is closed or terminated; `KILL_ON_JOB_CLOSE` guarantees it even if the backend crashes) and marked `idle` with a `warn` line "Stopped because Rain stopped", inside the 5 s module budget. `upgrade` and `restart` are refused with `409 busy` while any run is in progress (foundation 2.5, section 06 6.3), so only `stop`, `--force` and game mode end runs.

#### 6.11.7 A run

```
start_run(job, trigger, due):
  1. id = run-<UTC yyyymmdd-hhmmss>-<4 hex>; insert jobs_run (status running); push JobRun to recent (front); publish jobs
  2. log "Started by schedule" | "Started by Run now" | "Started after a missed run"
  3. build the SpawnSpec (6.11.8) and call runner::start_child (6.11.9); a start failure finishes the run at once (row below)
  4. wait: select! { child exited | timeout (T4: one sleep per run) | cancel token (stop_all) }
  5. TerminateJobObject (kills any process the run left behind), join the reader threads, flush the log
  6. status, exit code, note by the table below; update jobs_run; push to recent; retention (5.1); handled_until fix (6.11.5); publish
```

| Outcome | `status` | `exitCode` | Last log lines |
|---|---|---|---|
| cron, exit 0 | `success` | 0 | `info` "Finished in <duration>", `info` "Process exited with code 0" |
| cron, exit not 0 | `failed` | the code | `error` "Process exited with code N" |
| Claude job, exit 0, result line `subtype: "success"` and `is_error` false | `success` | 0 | `info` "Finished in <duration>, <n> turns" and the cost when the result has `total_cost_usd` |
| Claude job, exit 0 but the result says error, or there is no result line | `failed` | 0 | `error` "Claude reported an error: <subtype>", the `errors[]` lines; or "Claude ended without a result" |
| Claude job, exit not 0 | `failed` | the code | `error` "Process exited with code N" and the last stderr lines |
| the program cannot be started | `failed` | absent | `error` "Could not start: <what Windows said>" (always at least one line: a failed run is never empty, contract report 3.3) |
| `timeout_minutes` reached | `failed` | absent | `error` "Stopped after the time limit of N minutes" |
| `stop_all` (game mode, shutdown) | `idle` | absent | `warn` "Stopped by game mode" / "Stopped because Rain stopped" |
| found `running` at start | `failed` | absent | `error` "Rain stopped while this run was in progress" |

A run that Rain stops on purpose is `idle` (neutral grey square, not counted by "Needs you"); a run that failed by itself is `failed`. Every `failed` run ends with at least one explanatory `error` line, because the spec's task "read why a job failed" depends on it.

#### 6.11.8 The command of a run

Cron job:

| `shell` | Program and arguments |
|---|---|
| `none` | `command` with `args`, exec form, standard Windows quoting by `Command::args` (resolved through `PATH`/`PATHEXT` by the system; a `.cmd` or `.bat` needs `shell = "cmd"`) |
| `cmd` | `cmd.exe` with `raw_arg` `/d /s /c "<command>"` |
| `powershell` | `powershell.exe -NoProfile -NonInteractive -Command <command>` |

Working directory `working_dir`; stdin null; environment = Rain's own plus `RAIN_RUN_ID`, `RAIN_JOB_ID` and the `env` table; flags `CREATE_NO_WINDOW`; run inside the owned job object (6.11.9).

Claude job (the proven shape, spike claude-hooks Q6 `jobrun/src/main.rs`):

```
<claude program> -p --output-format stream-json --verbose
    [--model <model>] --permission-mode <permission_mode>
    [--allowedTools "<allowed_tools joined by comma>"] [--disallowedTools "<...>"]
    --permission-prompts none [--max-turns <n>] --name "<job name>" [extra_args...]
stdin  = the prompt (UTF-8), then closed          cwd = working_dir
env    = Rain's environment WITHOUT any variable whose name starts with CLAUDE or ANTHROPIC, PLUS RAIN_RUN_ID, RAIN_JOB_ID and the env table
```

The prompt (4.1.1): when `skill` is set, `Run the skill "<skill>" now and finish its whole workflow. Nobody is available to answer questions, so make reasonable choices and say at the end what you did.`; when `prompt` is set too, a blank line and the prompt follow. Headless mode may not allow a user to type a slash command, so the skill is requested in words (the model's own Skill tool runs it); this is UNPROVEN for the owner's skill and is checked by the acceptance step A6.1 (a trivial skill first). The prompt text is never logged by tracing and appears in the run log only as "Skill <name>".

Why the flags: `dontAsk` plus `allowedTools` is the proven unattended combination (denies what is not listed); `--permission-prompts none` tells Claude that nobody can approve, so it stops retrying; `--max-turns` bounds cost. A run takes 4 to 6 seconds even when trivial. A killed run leaves a transcript whose open tool call is closed by Claude on a later `--resume` (proven); Rain does not resume.

Stream to log lines (`jobs/claude_stream.rs`, pure function, unit tested on fixture lines taken from the spike's `out\run*.jsonl` after removing personal paths):

| Stream object | Log line |
|---|---|
| `system` / `init` | `info` "Claude session started (model <model>)"; the `session_id` is stored in `jobs_run.claude_session_id` |
| `assistant`, `text` block | `info` the text, cut to 1000 characters |
| `assistant`, `tool_use` | `info` "<name>: <summary>" (the summary rules of 6.5.2) |
| `user`, `tool_result` with `is_error: true` | `warn` "  <first line>" |
| `user`, other `tool_result` | skipped (file contents) |
| `system` / `api_retry` | `warn` "Retrying the request (attempt N of M)" |
| `system` / `permission_denied` | `warn` "Denied <tool>" (so the user sees what to add to `allowed_tools`) |
| `system` / `hook_started`, `hook_response` and any unknown type | skipped |
| `result` | the closing lines of 6.11.7 |
| a stdout line that is not JSON | `info` the text as it is |
| stderr line | `warn` |

#### 6.11.9 The shared child runner and the log

`jobs::runner` is used by this module and by `idle` (through `jobs::Service`). It is built on `platform::job::OwnedJob` (foundation 11.6: copy `S\process\src\bin\q4_hand.rs` and the helpers `resume_process_threads` and `job_pids` of `S\process\src\lib.rs`): `CreateJobObjectW`, `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, spawn with `CREATE_SUSPENDED | CREATE_NO_WINDOW`, assign, resume the threads, `TerminateJobObject` to stop (proven: a 4-process tree gone in about 1 ms; also when the owner crashes). Never `process-wrap`.

```rust
pub struct SpawnSpec { program: PathBuf, args: Vec<OsString>, raw_args: Option<OsString>, cwd: PathBuf,
                       env_set: Vec<(String, String)>, env_remove_prefixes: Vec<&'static str>, stdin: Option<Vec<u8>> }
impl OwnedJob {
    pub fn spawn(spec: &SpawnSpec) -> io::Result<(OwnedJob, ChildPipes)>;   // ChildPipes { stdout, stderr }
    pub fn pid(&self) -> u32;
    pub fn terminate(&self, code: u32);                                     // TerminateJobObject
    pub fn suspend(&self) -> io::Result<()>;  pub fn resume(&self) -> io::Result<()>;   // 6.12.5
    pub fn exit_watcher(&self, cb) -> Option<ExitWatcher>;                  // RegisterWaitForSingleObject on the main process
    pub fn exit_code(&self) -> Option<u32>;
}
```

Threads and timers per run (all exist only while the run lives, foundation 11.5 row T5): two named `std::thread`s read stdout and stderr and send lines over an unbounded channel; the exit is awaited by `ExitWatcher` (6.3.6, no thread); the log task flushes with a 200 ms `sleep` only while lines are waiting; one `sleep(timeout)` for the time limit.

Line reading is bounded: a reader keeps the first 4096 bytes of each line and discards the rest up to the newline (a program that prints a gigabyte without a newline cannot grow memory); invalid UTF-8 is replaced; a trailing `\r` is removed.

Log store rules (`LogSink`):

| Rule | Value |
|---|---|
| text stored per line | at most 2000 characters; an API response cuts to 1000 |
| lines kept per run | the first 4000, then a ring of the last 500; when lines were dropped, `jobs_run.log_omitted` holds the count and a `warn` line "N lines omitted" sits between head and tail |
| persistence | the head is written to `jobs_run_log` in batches (every 200 ms or 100 lines, one transaction); the tail ring is written when the run ends. A crash leaves the head lines of the run |
| a running run's log | `getRunLog` returns the lines so far from memory plus the stored ones (the same list the dialog will show after the run ends, minus the tail ring) |
| lines returned by the API | at most the last 2000, with a first line `warn` "N earlier lines are not shown" when cut |
| never | log lines are not copied into `tracing` output (foundation 13.4) |

#### 6.11.10 Retention and write-back

Retention is in 5.1. Writing `enabled` is a closure given to `TomlFile::edit`: find the `[[job]]` table whose `id` equals the job id (or whose derived id equals it), set the bool with the value's decor kept, insert `id` as the first key when the entry has none, change nothing else; if the entry cannot be found the call returns `conflict` `configChanged` (foundation 10.4 step 4). After the write the file is reloaded, the topic is published and the scheduler is nudged. Comment lines directly above a `[[job]]` belong to that entry (foundation 10.4: they are removed with it); the generated header of 4.1.3 is a file header and survives.

### 6.12 Idle jobs

#### 6.12.1 Machine facts and the sampler (timer T3)

An idle job needs one fact: how long the user has not touched keyboard or mouse. `machine.idle_ms() -> Option<u64>` (section 05) is one `GetLastInputInfo` call; `None` means Windows refused it and the user counts as present. Idle milliseconds = `(GetTickCount64() as u32).wrapping_sub(dwTime)` (the 32-bit wrap is by design; proven from a windowless process, spike winapi Q4).

The sampler is one task that exists only while at least one idle job is enabled and `game_mode_active` is false (it is created and cancelled by `idle-jobs.toml` reloads, enable changes and game mode; foundation row T3).

```
loop {
    // `idle_ms() -> Option<u64>`: None = Windows refused the call. The user then counts as present:
    // no run starts and a running run is paused (the safe reading, section 05 6.10).
    let Some(idle_ms) = machine.idle_ms() else {
        health.set_source("idle-sampler", Failed, "Idle detection is not available");
        for action in decide(&mut book, Inputs { now, idle_ms: 0, user_present: true }) { perform(action) }
        sleep(next_delay()); continue
    };
    let touched = idle_ms + 500 < now - last_sample;            // input happened since the last sample
    let user_present = touched || idle_ms < 3000 || games.any_game_running().await;   // one call per sample, 02 6.15
    for action in decide(&mut book, Inputs { now, idle_ms, user_present }) { perform(action) }   // pure function plus effects
    sleep( next_delay() )                                        // 5 s while any job is running, else 30 s
}
```

The 30 s rate is the foundation's T3. The 5 s rate while a job runs is allowed by foundation 11.5 (T3): without it a running job would keep running up to 30 s after the user came back. Cost: one Win32 call per sample.

#### 6.12.2 The state machine

`decide` is pure and unit tested on a table of inputs. `idle_after` = `idle_after_minutes` of the job. "Ready" = job enabled, and `idle_ms >= idle_after * 60_000`, and `user_present` is false.

| State | Condition (checked in this order) | Effect, new state |
|---|---|---|
| `queued` | ready, and no job of the same resource is `running` or `paused` (a paused job resumes before a new one starts), and this job is the first queued one of its resource in file order | start the process, `running`, progress 0 |
| `running` (started by idle) | `user_present` | `OwnedJob::suspend`, `paused`, progress kept, eta cleared; log `warn` "Paused: the PC is in use" |
| `running` (started by Run now) | `user_present` | nothing: a manual run ignores idleness |
| `running` | the process exits 0 | `success`, progress 1, `lastRunAt = now` |
| `running` | the process exits not 0, or the time limit is reached | `failed`, `lastRunAt = now` |
| `paused` with a live process | ready | `resume`, `running`; log `info` "Resumed after N min idle" |
| `paused` with a live process | job disabled | terminate; `paused` without a process; log `warn` "Stopped: the job was switched off" |
| `success` or `failed` | the rerun rule (6.12.4) says due, and enabled | `queued` |
| any | game mode `pause(true)` | nothing is stopped. The sampler is cancelled. A `running` run continues. A `paused` run with a live process stays frozen (nothing resumes it while game mode is on). Both count in `counts().running` (06 6.7.1), so the user sees them as interrupts and can choose Wait |
| any | `stop_all(budget)` (06 wind-down step 3) or the backend stops | terminate the process; the run is `stopped`; state `queued` (6.12.6) |

Enable and disable (`PATCH /idle-jobs/{id}`): `false` while `running` or `paused` with a process terminates the process (a run cannot be frozen forever), state `paused`, progress kept for display; a queued job stays queued but never starts while disabled; the UI shows "Disabled" for any disabled job. `true`: a `paused` job without a process becomes `queued` (progress reset to 0 because a new run starts from the beginning); other states stay.

Run now (`POST /idle-jobs/{id}/run`, label "ignore idle"): allowed also when the job is disabled (explicit intent, like a cron job). If the job is `running`: return it. If it is `paused` with a live process: resume it, as a manual run. Else start a new run with `manual = true`. If another job of the same resource is `running` or `paused`: `conflict` `resourceBusy` with the message "<name> is using the <CPU|GPU> right now". During game mode: `conflict` `gameMode`.

#### 6.12.3 Resources

At most one idle job is `running` or `paused` per resource (`cpu` or `gpu`). Two jobs on different resources may run together. The value is only a declaration by the user in the TOML; Rain cannot measure GPU use.

#### 6.12.4 When a finished job becomes `queued` again (the open question of contract report P-I1)

| `rerun` | Due when |
|---|---|
| `daily` (default) | the local calendar date now is later than the local date of `last_started_at` (both through `platform::time::local_date`), or the job never ran |
| `weekly` | `now >= last_started_at + 7 days`, or never ran |
| `every_idle` | `now >= last_ended_at + min_gap_minutes`, or never ran |
| `manual` | never by itself; only Run now. A job that never ran is `queued` and does start once when idle |

A failed run follows the same rule (so a broken job is retried at most once a day by default, not at every idle moment). The check runs at each sampler tick; a state changes to `queued` at the first sample after it became due. No timer exists for this.

#### 6.12.5 Pause and resume of a process tree

A generic program cannot be asked to pause, so Rain freezes it: `OwnedJob::suspend()` lists the job's processes (`QueryInformationJobObject` `JobObjectBasicProcessIdList`, proven) and suspends every thread of each through a Toolhelp thread snapshot and `SuspendThread` (the mirror of the proven `resume_process_threads` helper); a second pass catches processes created between the two calls. `resume()` is the same with `ResumeThread`. UNPROVEN as a whole (the foundation's spike proved resume of a new process only). Defensive rules: a suspend that fails for any process terminates the run instead, records it `stopped`, logs `warn` "Could not pause; the run was stopped" and the job returns to `queued`; the fixture test uses a CPU burner and `ping`, and the acceptance step A7.4 checks it by hand. A frozen program that holds a network connection may time out when it resumes; the user decides by choosing what to run.

#### 6.12.6 Restart and stop

Idle runs die with Rain (`KILL_ON_JOB_CLOSE`). At `stop()` every idle run is terminated and marked `stopped` with the log line "Stopped because Rain stopped". At start: any `idle_run` row still `running` is closed `stopped` with "Rain stopped while this run was in progress"; the job state is rebuilt: `queued` when its run was interrupted or it never ran or its rerun rule is due; else `success` or `failed` from `idle_state.last_result`. Progress is not kept across a restart. Game mode (section 06) calls `idle().wind_down().pause(true)` when it starts: that only cancels the sampler, so no run starts or resumes, and running work is left alone (a `running` run keeps running; a `paused` run with a live process stays frozen; both count in `counts().running`). It calls `stop_all(budget)` only in wind-down step 3 (the `WindDownPort` of 1.3); that is the only outside call that stops idle runs. `pause(false)` after a cancel starts the sampler again, which resumes frozen runs when they are ready.

#### 6.12.7 Progress and eta

While a run lives the stdout reader looks at each line for `rain-progress:` and `rain-eta:` (4.2.1). Without reports: progress 0 while running, 1 on success, or the estimate from `expected_minutes`. `etaMinutes` = the reported eta, else `max(0, expected_minutes - elapsed)` rounded up, else absent. A progress change under 0.01 does not publish. The reported lines are also written to the log.

#### 6.12.8 The log of an idle job

`GET /idle-jobs/{id}/log` returns the log of the newest run of that job from the shared store (`jobs::Service::read_log`). First lines: `info` "Started after N min idle" or "Started by Run now"; `info` "Using the CPU" or "Using the GPU"; then the program's output (stdout `info`, stderr `warn`); pause, resume and stop lines as in 6.12.2; the ending: `info` "Finished" and "Process exited with code 0", or `error` "Process exited with code N", or `error` "Stopped after the time limit of N minutes", or `warn` "Stopped by game mode". A job that never ran returns an empty array.

### 6.13 Numbers

| Name | Value | Where |
|---|---|---|
| `HOOK_WAIT` (forwarder) | 20 s | 6.2.1 |
| `HOOK_SERVER_WAIT` | 19 s | 6.5.1 |
| `HOOK_HANDLER_TIMEOUT` (in the installed JSON) | 35 s | 6.9.2 |
| `EVENT_WAIT` | 300 ms | 6.2.1 |
| `LIVENESS_TOLERANCE` | 100 000 ticks (10 ms) | 6.3.3 |
| `ESC_OVERRIDE` | 3 s | 6.3.3 |
| sessions in the list; ended kept | 40; 20 shown, 24 h, 50 stored | 6.3.4, 5.3 |
| `recent` events per session | 8, text at most 200 characters | 6.3 |
| transcript tail at first sight | 256 KiB | 6.4 |
| transcript for the log | tail 2 MiB, up to 16 MiB, at most 1000 lines | 6.8 |
| `STOP_WAIT` | 2 s | 6.7 |
| focus verify | 150 ms per attempt | 6.6 |
| `JOB_RUNS_KEPT` | 20 per job | 5.1 |
| `CATCHUP_DELAY` | 60 s | 6.11.5 |
| default timeouts | 30 min cron, 60 min claude, 240 min idle | 4.1, 4.2 |
| log lines per run | 4000 head plus 500 tail; API returns the last 2000 | 6.11.9 |
| `JOBS_LOG_ROWS_MAX` | 200 000 | 5.1 |
| idle sampling | 30 s; 5 s while a job runs (request) | 6.12.1 |
| idle runs kept per job | 5 | 5.2 |

---------------------------------------------------------------------------------------------------
## 7. Background work

Every task below is checked against foundation 11.5. "T" numbers are its timer rows. A task with no timer is event driven.

| Task | Trigger | Timer row | Cost | In game mode | How it stops |
|---|---|---|---|---|---|
| Registry watcher (`<claude folder>\sessions`, non-recursive, debounce 300 ms) | file change events | none (T4 debounce for the 300 ms) | one directory handle, zero CPU idle; a rescan reads at most 40 small files | still registered until the module stops (the backend is on its way out) | dropped in `stop()` |
| Exit watch per live session (`RegisterWaitForSingleObject`) | the process ends | none | zero while the process lives; one handle per live session, at most 40 | same | unregistered in `stop()` and when the record ends |
| Hook event handling (`POST /internal/hook`) | a request from `rain-hook` | none | microseconds per event, no I/O; about 2 events per tool call of every Claude session | refused (`204`, nothing recorded) once `game_mode_active` | none |
| Permission hold | one open permission request | T4 (one `sleep(19 s)` created for that request and dropped with it) | one task and one timer per open request | the backend is about to stop; holds end with the pipe | answered, expired or dropped |
| Transcript tail read | first sight of a session, after `Stop`, when the transcript grew (the folder watch below, and `GET /sessions`), on `GET .../transcript` | none | one blocking read of at most 256 KiB (2 to 16 MiB for the log route) | not started | finishes |
| Transcript folder watch (one per folder of a live session, non-recursive, quiet period 2 s) | file change events in that folder | T4 (the 2 s quiet period, then one `sleep(2 s)` after each read, only after a change) | one directory handle per folder, zero CPU idle; a read at most about every 2 s per session while Claude writes | still registered until the module stops | dropped when no live session uses the folder, and in `stop()` |
| Publish throttle for `sessions`, `jobs`, `idleJobs` | a change | T4 (`core::throttle::Throttle`, at most 4 frames per second, newest wins) | one short timer only while a burst is pending | no frames after `done` (foundation 2.6) | dropped when the burst ends |
| Hook status check | backend start; `hooks_state()`; `install_hooks()` | none | one small file read | not started | n/a |
| Scheduler | the next due time, a change, a resume | T2 | one `sleep_until` | starts nothing, advances nothing | cancel token |
| Power resume notification | the PC wakes | none (event source) | zero while running | dropped | unregistered in `stop()` |
| Catch-up delay | backend start, resume from sleep | T4 (one `sleep(60 s)`) | one timer | not started | fires once |
| A job run: time limit | run start | T4 (one `sleep(timeout)`) | one timer per run | cancelled with the run | run end |
| A job run: log flush | lines waiting | T5 (a 200 ms `sleep` only while the run lives and lines are waiting) | one small DB transaction per 200 ms of output | run is stopped | run end |
| A job run: two reader threads (`std::thread`) | run start | T5 rule (exist only while the run lives; the review checklist of 11.5 item) | blocked in `read`, zero CPU | ended by the stop | EOF after the job handle is terminated |
| Idle sampler | at least one idle job enabled | T3 (30 s; 5 s while a job runs: request, 12.5) | one `GetLastInputInfo` call per sample | cancelled | no enabled idle job, game mode, or shutdown |
| Idle run: time limit, log flush, reader threads | as for scheduled runs | T4, T5 | as above | stopped by `stop_all` | run end |

No other timer, `interval`, polling loop or watcher is added by this section. In particular: no watcher on the Claude settings file, no watcher on a transcript file itself (only the folders of live sessions, above), no poll of the registry, no poll for process death.

Idle cost with nothing running: one directory watch handle, up to 40 wait registrations, one `sleep_until` for the next job, no idle sampler without an enabled idle job, no threads of runs. The measurement of foundation stage 2 is repeated after stage 6 and stage 8 (acceptance A6.9, A8.9).

---------------------------------------------------------------------------------------------------

## 8. Errors

Messages are plain English for the user; the UI shows `message` as it is (`errorMessage()`), so none contains a raw OS error alone or any prompt, command or file text.

| Situation | Status | `code` | `reason` | `message` | What the UI shows |
|---|---|---|---|---|---|
| unknown session id | 404 | `notFound` | `unknownSession` | "Session not found" | notice (action error) |
| no open request with this id, or it expired | 409 | `conflict` | `requestExpired` | "That request is no longer open. Answer it in the terminal." | notice; the list refetches |
| answer while monitoring is off | 409 | `conflict` | `monitoringOff` | "Monitoring is off. Turn on Monitor Claude sessions in Settings." | notice |
| stop: desktop host | 409 | `conflict` | `notStoppable` | "This session runs inside the Claude desktop app. Close it there." | notice |
| stop: pid unknown, gone or not verified | 409 | `conflict` | `notStoppable` | "Rain cannot verify this session's process. Close it in its terminal." | notice |
| stop: not a Claude process | 409 | `conflict` | `notStoppable` | "That process is not Claude. Nothing was stopped." | notice |
| stop: Win32 failure | 500 | `failed` | `stopFailed` | "Could not stop the session" | notice |
| transcript missing | 404 | `notFound` | `noTranscript` | "The transcript file was not found" | dialog text "Could not load the log." |
| focus: nothing found | 200 | none | none | (result `none`, not an error) | notice "Could not find the window for this session" |
| settings file cannot be merged | see the table of 6.9.6 | | | | notice from the Settings switch |
| bad hook envelope | 400 | `invalid` | `badEnvelope` | "The hook request was not valid" | none (only `rain-hook` calls it; it ignores the answer) |
| unknown job | 404 | `notFound` | `unknownJob` | "Job not found" | notice |
| unknown run | 404 | `notFound` | `unknownRun` | "Run not found" | dialog "Could not load the log." |
| unknown idle job | 404 | `notFound` | `unknownIdleJob` | "Idle job not found" | notice |
| run now during game mode | 409 | `conflict` | `gameMode` | "Rain is getting ready for game mode. Nothing new starts." | notice |
| idle run now, resource taken | 409 | `conflict` | `resourceBusy` | "<name> is using the CPU right now" (or GPU) | notice |
| backend is shutting down | 503 | `unavailable` | `shuttingDown` | "Rain is stopping" | notice |
| `jobs.toml` or `idle-jobs.toml` cannot be written | 500 | `failed` | `saveFailed` | "Could not save jobs.toml" (or idle-jobs.toml) | notice |
| the entry moved or vanished during the write | 409 | `conflict` | `configChanged` | "jobs.toml changed while saving. Try again." | notice |
| the file is from a newer Rain | 409 | `conflict` | `configNewer` | "jobs.toml was written by a newer Rain and is read only" | notice |
| the file does not parse (write routes `PATCH /jobs/{id}` and `PATCH /idle-jobs/{id}`; a skipped bad entry does not count) | 409 | `conflict` | `configBroken` | "jobs.toml has an error. Fix the file, then try again" (or idle-jobs.toml) | notice; `GET /health` lists the issue |
| the module failed to start | 503 | `unavailable` | `moduleFailed` | "Jobs are unavailable" (or Sessions, Idle jobs) | the existing `ErrorRow` with Retry ("Jobs unavailable") |
| database too new | 503 | `unavailable` | `dbTooNew` | "Run history is unavailable" | `ErrorRow` |

Failures inside a run are not API errors; they are run records with log lines (6.11.7), so the Jobs screen shows them through its failed state and the log dialog.

Degraded sources (foundation 6.9: `Health.sources`, one `SourceStatus { name, state, message? }` each):

| Source name | `off` | `ok` | `failed` |
|---|---|---|---|
| `claude-registry` | no Claude folder or no `sessions` folder yet ("Claude Code has not run on this PC") | files read | files exist but none can be understood ("Claude session files are in a format Rain does not know") |
| `claude-hooks` | switch off | monitoring ok | broken or blocked, message of 6.9.6 or 6.9.7 |
| `scheduler` | no enabled job | armed | module failed; or `ok` with the message "Sleep wake-up detection is not available; checking every 5 minutes" in degraded mode |
| `idle-sampler` | no enabled idle job | sampling | `GetLastInputInfo` failed |

A TOML problem is a `ConfigIssue` (foundation 6.9), not a source state.

---------------------------------------------------------------------------------------------------

## 9. Frontend changes

Paths are relative to the repository root. Line numbers are those of the working tree at commit e4c7f70 (checked while writing); the `Edit` that makes the change must re-check them. The keyboard map, the groups and the layout of ui-spec section 7.5 stay as they are. The design direction is kept: no new screens, no new chrome. The only new visible things are one line of text in the request card, one Settings row pair, a hint under two empty states, and a log that updates while a run is in progress.

### 9.1 Files, in the order of the stages

#### Stage 6 (jobs and idle jobs)

| File | Change |
|---|---|
| `src/data/types.ts` lines 58 to 64 | delete the hand-written `JobRun`, `LogLine`, `ScheduledJob`, `IdleJob` (now generated into the barrel) |
| `src/data/api.ts` lines 56 to 67 | the per-method route comments become the registry rows of 2.2 and 2.3 (comment text only) |
| `src/data/rest/client.ts` | implement the 8 methods: `listJobs: () => call<ScheduledJob[]>('GET', '/jobs')`; `setJobEnabled: (id, enabled) => call<ScheduledJob>('PATCH', \`/jobs/${enc(id)}\`, { enabled })`; `runJob: (id) => call<JobRun>('POST', \`/jobs/${enc(id)}/runs\`)`; `getRunLog: (runId) => call<LogLine[]>('GET', \`/runs/${enc(runId)}/log\`, undefined, { media: false })`; `listIdleJobs`, `setIdleJobEnabled` (`PATCH /idle-jobs/{id}`), `runIdleJob` (`POST /idle-jobs/{id}/run`), `getIdleJobLog` (`GET /idle-jobs/{id}/log`, `media: false`) |
| `src/data/routing.ts` | `methods`: the 8 methods above `'real'`; `topics`: `jobs` and `idleJobs` `'real'` (the only file the router needs, foundation 8.6) |
| `src/composables/useLogDialog.ts` lines 7 to 14 | add to `LogRequest` an optional `follow?: (runId?: ID) => Promise<boolean>` ("true while this log is still being written") |
| `src/lib/follow.ts` (new) | the follow loop below, a pure function with injected timers so vitest can test it |
| `src/components/rain/LogDialog.vue` lines 22 to 51 | after each load, when `request.value.follow` exists, ask it; while it answers `true` and the window is visible, load again after 3 s without clearing the lines; stop on close, on run switch, when the window hides (restart on show) |
| `src/features/workflows/actions.ts` lines 171 to 181 (`openJobLog`) | pass `follow: async (runId) => runId !== undefined && (await api.listJobs()).some((j) => j.id === job.id && j.recentRuns.some((r) => r.id === runId && r.status === 'running'))` |
| same file lines 242 to 251 (`openIdleLog`) | pass `follow: async () => (await api.listIdleJobs()).some((j) => j.id === job.id && j.state === 'running')` |
| `src/features/workflows/jobs/JobsView.vue` lines 79 to 83 | add `hint="Jobs are defined in jobs.toml in your data folder"` to the empty `EmptyState` |
| `src/features/workflows/idle/IdleView.vue` line 74 | add `hint="Idle jobs are defined in idle-jobs.toml in your data folder"` |
| `src/data/mock/...` | the mock adaptation of 9.3 |

The follow loop (`src/lib/follow.ts`):

```ts
export interface Follower { start(): void; stop(): void }
export function createFollower(opts: {
  load: () => Promise<void>          // reloads the lines without clearing them
  isLive: () => Promise<boolean>     // true while the run is still going
  everyMs: number                    // 3000
  setTimer?: typeof setTimeout; clearTimer?: typeof clearTimeout
}): Follower   // start(): load, then while isLive() wait everyMs and load again; stop(): cancels the pending timer; a failed load or isLive stops the loop
```

The loop exists only while the dialog is open and the window is visible, so rule 14 of the UI spec (no intervals while hidden) holds. The run list on the left of the dialog is the snapshot taken at open time (as today); it shows the new state of a run the next time the dialog is opened. This is a known small limit.

#### Stage 8a and 8b (sessions)

| File | Change |
|---|---|
| `src/data/types.ts` lines 46 to 56 | delete the hand-written `PermissionAnswer`, `PermissionRequest`, `SessionEvent`, `ClaudeSession` and their comments (generated now) |
| `src/data/api.ts` lines 50 to 54 | `answerPermission(sessionId: ID, requestId: ID, answer: PermissionAnswer): Promise<PermissionAnswerResult>`; `focusSession(id: ID): Promise<FocusResult>`; new route comments as 2.1; the import list at lines 5 to 10 gains the three new types |
| `src/data/rest/client.ts` | `listSessions: () => call<ClaudeSession[]>('GET', '/sessions')`; `answerPermission: (sessionId, requestId, answer) => call<PermissionAnswerResult>('POST', \`/sessions/${enc(sessionId)}/permission\`, { requestId, answer })`; `focusSession: (id) => call<FocusResult>('POST', \`/sessions/${enc(id)}/focus\`)`; `stopSession: (id) => call<void>('POST', \`/sessions/${enc(id)}/stop\`)`; `getSessionTranscript: (id) => call<LogLine[]>('GET', \`/sessions/${enc(id)}/transcript\`, undefined, { media: false })` |
| `src/data/routing.ts` | the 5 methods `'real'` (`listSessions`, `getSessionTranscript`, `focusSession`, `stopSession`, `answerPermission`), topic `sessions` `'real'` |
| `src/features/workflows/answerText.ts` (new) | `answerNotice(result: PermissionAnswerResult, waitingLeft: number): string`. Rules: `once` gives "Allowed"; `deny` gives "Denied"; `always` with a `ruleText` gives "Always allowed <ruleText> for this session"; `always` that was applied as `once` gives "Allowed once. No session rule was possible."; then ", N waiting" when N > 0 (the existing wording of `actions.ts` line 73). Pure, unit tested |
| `src/features/workflows/actions.ts` lines 37 to 41 | delete `ANSWERED` (replaced by `answerNotice`) |
| same file lines 59 to 74 (`answer`) | `const request = session.request; if (!request) return`; `const result = await ctx.api.answerPermission(session.id, request.id, choice)`; after the list refetch `ctx.notice.success(answerNotice(result, left))` |
| same file lines 76 to 103 and line 162 | `permissionActions(session)` is offered when `session.request?.answerable === true` (was `session.status === 'waiting'`) |
| same file lines 105 to 125 and 149 | `stopSession` is offered when `session.alive && session.host === 'terminal'` (was `isLive`, line 149, which used the status); on success the notice stays "Session stopped" |
| same file lines 146 to 167 | the focus action: label `focusLabel(session.host)` ("Focus terminal", "Focus Claude app", "Focus window"); offered only when `session.alive`; on the result `none` it shows `notice.info('Could not find the window for this session')` and does NOT hide Rain; on `window` or `app` it hides (`hideAfterSuccess`). For an ended session the `transcript` action takes the primary slot (Enter opens it) because Enter has nothing to focus |
| `src/features/workflows/sessions/sessionInfo.ts` | add `focusLabel(host: SessionHost): string` |
| `src/features/workflows/sessions/SessionDetail.vue` line 66 | the fact label "Tokens" becomes "Context" |
| same file line 72 | show the card when `session.request` exists (the status is `waiting` whenever a request exists; the check is simpler and true) |
| same file lines 95 to 99 | the "Focus terminal" button label from `focusLabel(session.host)`; hidden when `!session.alive`; the "Transcript" button stays |
| `src/features/workflows/sessions/RequestCard.vue` line 13 and lines 40 to 52 | the three buttons only when `request.answerable`; otherwise one muted line "Answer this in the terminal" (the text takes the host into account: "in the Claude app" for `desktop`; the host is passed as a prop `host`) |
| `src/actions/commands.ts` lines 125 to 135 | delete the command `simulate-permission` (it only changed mock sessions, which no screen reads from stage 8) |
| `src/data/mock/...` | the mock adaptation of 9.3 |

No change in: `WorkflowsScreen.vue`, `SessionsView.vue` (groups, order and `byUrgency` stay), `SessionRow.vue`, `JobDetail.vue`, `JobRow.vue`, `RunStrip.vue`, `IdleJobDetail.vue`, `IdleRow.vue`, `idleState.ts`, `useNeedsYou.ts`, `HomeScreen.vue`, `ProjectsScreen.vue`, `resources.ts`. They read the same fields.

### 9.2 Contract changes (foundation 15.1 template)

| # | Item (type or method) | From | To | Reason | Frontend files touched | Breaking for the mock? | Stage |
|---|---|---|---|---|---|---|---|
| S.1 | `ClaudeSession` | without `host`, `alive` | adds `host: 'terminal' \| 'desktop' \| 'other'`, `alive: boolean` | focus label, whether Stop is possible, ended sessions | `src/data/types.ts`, `src/features/workflows/actions.ts`, `sessions/SessionDetail.vue`, `sessions/sessionInfo.ts` | yes: the mock seed and `toSession` must set both (`host: 'terminal'`, `alive: status is running, waiting or idle`) | 8 |
| S.2 | `PermissionRequest` | without `answerable` | adds `answerable: boolean` | a request can stay open after Rain's forwarder gave up | `src/data/types.ts`, `actions.ts`, `sessions/RequestCard.vue`, `sessions/SessionDetail.vue` | yes: the mock sets `answerable: true` in `requestPermission` and in the seed requests | 8 |
| S.3 | `answerPermission` | `(sessionId, answer) => ClaudeSession` | `(sessionId, requestId, answer) => PermissionAnswerResult` | decision C2 (request id); the notice names the session rule | `src/data/api.ts`, `src/data/rest/client.ts`, `src/features/workflows/actions.ts`, new `answerText.ts`, `src/data/mock/slices/sessions.ts` | yes: the mock slice takes the extra parameter, checks `request.id`, returns `{ applied: answer }` (the mock slice is deleted at the end of stage 8, so this is needed only if the mock must stay compilable in between; with the router it is never called) | 8 |
| S.4 | `focusSession` | `Promise<void>` | `Promise<FocusResult>` | decision C12 | `api.ts`, `rest/client.ts`, `actions.ts` | yes: the mock returns `{ focused: 'window' }` | 8 |
| S.7 | `ClaudeSession.tokens` | "tokens" | same field, meaning "context size of the last request"; the label becomes "Context" | what the transcript tail gives cheaply and what "48k" meant | `sessions/SessionDetail.vue` | no | 8 |
| S.8 | `JobRun.status` and `JobRun.exitCode` meanings | success, failed, running | adds the meaning `idle` = stopped on purpose; `exitCode` absent when no real code exists | a deliberate stop must not show as a failure | none (the UI already draws other values neutrally) | no | 6 |
| S.9 | `LogRequest` (frontend type, not API) | `{ title, runs?, selectedRunId?, load }` | adds `follow?` | a running run's log updates (contract report P-L2, option B) | `useLogDialog.ts`, `LogDialog.vue`, `actions.ts`, new `lib/follow.ts` | no | 6 |
| S.10 | `ScheduledJob.scheduleLabel` wording | "Daily 06:00" (mock) | "Every day at 06:00" (generated) | the proven label function | none | no (mock labels stay) | 6 |
| S.11 | `RainApi` route comments | per method | registry rows 2.1 to 2.3 | foundation F4 | `src/data/api.ts` | no | 6, 8 |
| S.12 | `TopicPayload.sessions`, `jobs`, `idleJobs` | unchanged shapes | none | | none | no | |
| S.13 | `types.ts` ports | hand-written | generated: `JobRun`, `LogLine`, `ScheduledJob`, `IdleJob` at stage 6; the session types and `PermissionAnswer` at stage 8 | foundation F3 | `src/data/types.ts` | no | 6, 8 |

### 9.3 Mock adaptation (the mock stays compilable until stage 10)

Stage 6:
- `src/data/mock/index.ts` lines 9 and 28: remove `jobsSlice` from the mock api. No stub is written (foundation 8.6): add `listJobs`, `setJobEnabled`, `runJob`, `getRunLog`, `listIdleJobs`, `setIdleJobEnabled`, `runIdleJob`, `getIdleJobLog` and `getMachineState` to `RetiredMockMethod`. `getMachineState` lives in `mock/slices/jobs.ts` (line 108, section 05 9.4); this section retires it here, because it deletes that slice at stage 6. The router never sends these methods to the mock, and `routing.ts` may not name them as `'mock'`.
- Delete `src/data/mock/slices/jobs.ts` after moving `finishRunningJobs` (imported by `slices/gameMode.ts` line 9) into `slices/gameMode.ts`. Because the mock keeps no jobs data any more, `slices/gameMode.ts` lines 74 to 79 count the `job` and `idleJob` items as 0 and `interrupts` false until stage 9 replaces the slice.
- `src/data/mock/store.ts` lines 37 to 40 and 144 to 145: remove `jobs`, `schedules`, `idleJobs`, `failingIdleJobIds`, `findableJobs`. `src/data/mock/seed/workflows.ts` lines 159 to 340: remove the job and idle seed (`Schedule`, `JOBS`, `toJob`, `runs`, `runId`, `nextRun`, `IDLE_JOBS`, ...) and from `WorkflowSeed` (line 342) the `jobs`, `schedules`, `idleJobs`, `failingIdleJobIds` fields; the sessions part of the file stays until stage 8. Remove the `Schedule` import at `store.ts` line 19.
- `src/data/mock/logs.ts` lines 58 to 135: remove `runLog`, `idleJobLog` and their helpers.
- `src/data/mock/search.ts` lines 43 to 51 (`jobItem`, `idleJobItem`) and lines 71 and 72 (the two list entries): remove the job and idle jobs from the mock search; the router sends `search` for the types `job` and `idleJob` to the real side (section 01 does the mapping with `jobs.list()` and `idle.list()`, the subtitle and status rules of the old mock: job subtitle `scheduleLabel`, status `running ? 'running' : recentRuns[0]?.status ?? 'idle'`; idle subtitle "Queued" / "Running" / "Paused" / "Done" / "Failed", status `queued` and `paused` mapped to `idle`).
- Section 01 code that this stage needs (section 03 implements no search trait; section 01 1.3 owns it): in `launcher/search.rs` the mapping of `jobs().list()` and `idle().list()` to `SearchItem`s (table of section 01 6.3), and in `src/data/router.ts` `REAL_SEARCH` gains `job` and `idleJob`. Neither is built in this section; the stage 6 task list must name both.

Stage 8:
- Add the five session methods of `RainApi` (`listSessions`, `getSessionTranscript`, `focusSession`, `stopSession`, `answerPermission`) to `RetiredMockMethod`; no stub (foundation 8.6).
- Delete `src/data/mock/slices/sessions.ts` after moving `finishWorkingSessions` (imported by `slices/gameMode.ts` line 11) into `slices/gameMode.ts`; `src/data/mock/index.ts` lines 14 and 27 and the `startSessionActivity` call lose their references; `src/data/mock/control.ts` line 10 and the functions that call `markWorkingSessionsActive` and `requestPermission` are removed (with the `simulatePermissionRequest` member).
- `src/data/mock/seed/workflows.ts` lines 12 to 157 (sessions) removed, so the whole file is deleted (nothing is left in it); `store.ts` lines 35, 36, 66 to 70 and 87: `sessions`, `quietSessionIds` removed. `slices/gameMode.ts` lines 72 to 73 count `session` items as 0.
- `src/data/mock/logs.ts` lines 137 to 183 (`sessionTranscript` and helpers) removed; the file is deleted if nothing remains. `src/data/mock/search.ts` line 70 removed (session items).
- Section 01 code that this stage needs (section 01 1.3 owns it): in `launcher/search.rs` the mapping of `sessions().list()` to `SearchItem`s (table of section 01 6.3), and in `src/data/router.ts` `REAL_SEARCH` gains `session`; stage 8 then deletes the router special cases for `search`, `listPinned` and `listRecent` (section 01 9.6). The stage 8 task list must name both.


### 9.4 Replacement text for `docs/ui-spec.md` (the file itself is not edited here; the lead applies the jobs parts in stage 6 and the sessions parts in stage 8)

Replace the whole of section 7.5 (a) "Sessions" after the first bullet "ItemList rowSize lg ..." with:

> - Groups in fixed order, never collapsed, left out when empty: Waiting on you (an open permission request exists), Failed (the last turn ended with an API error, or the session's program vanished while it worked), Working, Done (the session has ended normally), Idle (alive and waiting for your next prompt). A finished turn of a live session is Idle, not Done.
> - Row: as before. A waiting row shows the request on line two ("Bash: npm install").
> - Pane `SessionDetail`: title; path; branch, model, started, Context ("36k": the size of the context of the last request); last message; then the request card (below); the last 8 events; buttons "Focus terminal" (or "Focus Claude app" for a desktop session) with Enter, and "Transcript" with Ctrl L. An ended session has no focus button; Enter opens the transcript.
> - **Request card**: tool badge, the exact command or diff, and while the request can still be answered from Rain (about 20 seconds after it arrived): "Allow once" with Ctrl Y, "Always allow" (for this session only; the notice names the rule, for example "Always allowed Bash(npm install) for this session"), "Deny" with Ctrl D. After that the card shows one muted line, "Answer this in the terminal" ("in the Claude app" for a desktop session), and the session stays in Waiting on you until it is answered there.
> - After an answer the selection moves to the next waiting session and a notice reads "Allowed, 1 waiting". Five blocked sessions take five Ctrl+Y presses, each with its command on screen.
> - Enter on a live session brings its window forward and hides Rain; when no window can be found the notice reads "Could not find the window for this session" and Rain stays.
> - Stop session: Ctrl+K, then `ConfirmDialog`. Offered only for a live terminal session. Sessions inside the Claude desktop app cannot be stopped from Rain.
> - Empty: `EmptyState` `bot` "No sessions", hint "Sessions started in a terminal appear here".

Replace in (b) "Jobs" the bullet "Run now: ..." with:

> - Run now starts the job at once, also when it is switched off. The row shows running while the real run goes on, then the new run appears. The log dialog follows a running run: it reloads every 3 seconds while that run is the selected one and the window is visible.
> - Schedule labels are written from the cron text ("Every day at 06:00", "Weekdays at 09:30", "Every 15 minutes"); a job may set its own label in `jobs.toml`.
> - Empty: `EmptyState` `calendar-clock` "No scheduled jobs", hint "Jobs are defined in jobs.toml in your data folder".

Replace in (c) "Idle" the bullet "Empty: ..." with:

> - Empty: `EmptyState` `cpu` "No idle jobs", hint "Idle jobs are defined in idle-jobs.toml in your data folder".
> - A job starts after its idle time, is frozen when you use the PC (state Paused) and continues after the same time of quiet. "Run now (ignore idle)" starts it at once and it is not frozen.

The Settings row "Monitor Claude sessions" (group Sources) and its Repair action are owned by section 06 (9.2); this section adds no row to the table of 7.9. Behaviour that belongs here: a failed switch shows the reason of the `ApiError` as a notice and keeps its old value; a failed check shows the failed glyph with the reason.

Replace the row of table 4.3 "Session | Workflows | Focus terminal | ..." with:

> | Session | Workflows | Focus (terminal, or the Claude app) / Transcript when ended | Open folder in editor | Allow once, Always allow, Deny (only while the request can be answered from Rain), Transcript, Stop session (confirm; live terminal sessions only), Copy folder path |

Section 10 of the spec ("Out of scope"): the bullets "Any backend: ... Claude Code hooks, cron, idle detection ..." are superseded for this area by this section; "Creating or editing jobs and idle jobs; starting or prompting Claude sessions from Rain" stays out of scope.

---------------------------------------------------------------------------------------------------
## 10. Tests

All tests follow foundation 12. None touches the real Claude folder, the real settings file, a real URL or the real registry. Test data is made up: no real path of a person, no session text, no real session id.

### 10.1 Unit tests (pure logic, next to the code)

| Module | Test | Proves |
|---|---|---|
| `sessions` | `status_table` | every row of 6.3.3 on a table of records, including the Esc rule and the registry-only rows |
| `sessions` | `apply_event_*` (one per event of 6.3.2, run on payloads from `tests/fixtures/hook-events/`) | the effect of each event; unknown events only touch `last_activity_at`; every field optional |
| `sessions` | `pending_match_and_clear` | rule 6.5.6: equal `tool_name` and equal input hash, key order of the input does not matter; the other clearing rules |
| `sessions` | `summary_and_detail` | the table of 6.5.2 for Bash, Edit, MultiEdit, Write, NotebookEdit, MCP, an unknown tool; caps and the "N more characters" line; relative path under `cwd` |
| `sessions` | `always_rule_choice` | the five rules of 6.5.7, a suggestion with another destination is forced to `session`, a Bash command with `*` degrades to `once` |
| `sessions` | `decision_json` | the three outputs of 6.5.5 are exactly the documented JSON (compared as `serde_json::Value`) |
| `sessions` | `registry_parse` | names that are not `<pid>.json` are never opened; unknown `status` values; missing fields; half-written file |
| `sessions` | `liveness_rule` | within 100 000 ticks alive, 300 ns apart alive, one second apart not alive, no `procStart` uses the first-sight time |
| `sessions` | `model_label` | "sonnet 4.5" and similar from made-up ids, raw text for unknown, "unknown" for empty |
| `sessions` | `transcript_map` | the table of 6.8 on a fixture; `api-request-blob` and `session-owner` lines are discarded unread (a canary string in them never appears in the output) |
| `sessions` | `title_chain`, `host_from_entrypoint`, `pid_conflict_drops_old_record` | 6.4, 6.3.3 |
| `sessions` | `payload_under_frame_max` | 40 sessions with full details stay under 200 KiB |
| `sessions` | the 11 installer tests copied from the spike (`install_then_uninstall_gives_back_the_same_bytes`, `user_entries_are_untouched_and_rain_comes_last`, `installing_twice_changes_nothing_and_a_new_path_replaces_not_duplicates`, `file_without_hooks_key_returns_to_no_hooks_key`, `absent_file_is_created_and_removal_leaves_an_empty_object`, `broken_or_blocked_files_are_refused_and_not_modified`, `crlf_tabs_and_bom_are_kept`, `backups_are_made_and_pruned`, `a_change_by_someone_else_before_the_rename_is_merged_not_overwritten`, `uninstall_removes_only_marked_entries_even_with_another_install_id`, `a_user_handler_in_the_same_group_as_a_rain_handler_survives`) plus `fragment_has_12_events`, `permission_has_timeout_35_and_is_sync`, `sandbox_target_rule` | 6.9 |
| `jobs` | `cron_label` (the spike's tests) | labels |
| `jobs` | `cron_validate`, `five_fields_only`, `dom_dow_rule`, `sunday_zero_and_seven` | 6.11.2 |
| `jobs` | `next_after_dst` (zones from `chrono-tz`, dev-dependency only) | the DST behaviour of 6.11.2 |
| `jobs` | `due_computation`, `catchup_runs_once_after_long_off`, `catchup_skips_job_enabled_after_the_miss`, `catchup_after_game_mode_stop`, `first_seen_sets_handled_until` | 6.11.5 as pure functions of (definition, `handled_until`, `now`) |
| `jobs` | `validate_entry` | every rule of 4.1.1 and 4.1.2, one bad entry does not hide the others, derived ids and clashes |
| `jobs` | `toml_edit_enabled_keeps_comments` | writing `enabled` keeps the trailing comment and CRLF and inserts `id` first when missing |
| `jobs` | `claude_command_line` | the argument list of 6.11.8 for a skill job, a prompt job, with `allowed_tools`, with `bypassPermissions` (needs the explicit flag) |
| `jobs` | `claude_stream_map` | the table of 6.11.8 on fixture lines |
| `jobs` | `log_sink_caps` | 4000 head plus 500 tail, the omitted marker, 2000-character lines, an endless line is cut at 4096 bytes |
| `jobs` | `status_mapping` | the table of 6.11.7 |
| `idle` | `decide_table` | every row of 6.12.2 as a table of (state, inputs) to (effects, new state); resource exclusivity; paused resumes before a new start |
| `idle` | `rerun_rule` | 6.12.4 across midnight, week, `min_gap_minutes` |
| `idle` | `progress_lines` | `rain-progress: 42%`, `0.42`, garbage, a decrease, `rain-eta` |
| `idle` | `idle_ms_math` | the 32-bit tick wrap |
| `idle` | `decide_machine_none` | `idle_ms() = None`: `user_present` is true, no job starts, a running job is paused; the source `idle-sampler` is `failed` in `Health.sources` |
| `rain-hook` | `envelope_escape`, `stdin_guards`, `pipe_name` (also in `rain-backend` against `rain_types::env::pipe_name` on a list of user names), `response_parse` | 6.2.1, 6.2.2 |

### 10.2 Integration tests (harness of foundation 12.2)

Files: `tests/sessions.rs`, `tests/hooks.rs`, `tests/jobs.rs`, `tests/idle.rs`, `tests/hook_process.rs`. Every route has a success, an `invalid` body, a `notFound` id and, for a mutation, the idempotency of its table row; every topic has a frame test and a `resync`-on-connect test.

Fixtures to add:

| Fixture | Content |
|---|---|
| `tests/fixtures/typical/data/jobs.toml` | five jobs: a `cron` job `cmd /c echo hi` (enabled), a failing `cron` job `cmd /c exit 3`, a `claude` job with `claude_program` pointing at the fake claude (below), a disabled job with comments around it, and a long-running job `[[job]] id = "job-demo-wait"`, `name = "Demo wait"`, `kind = "cron"`, `schedule = "0 0 1 1 *"` (never due in a test), `enabled = true`, `command = "ping"`, `args = ["-n", "90", "127.0.0.1"]` (about 90 s; it runs only when started by hand). Section 06 depends on it: its game mode checks G2, G3 and G8 and the `Busy` fixture state run "Demo wait" from the Jobs screen. Plus one entry with a bad cron (to test the `ConfigIssue`) |
| `tests/fixtures/typical/data/idle-jobs.toml` | two idle jobs on `cpu` and `gpu` (a short CPU burner and `ping`), one disabled |
| `tests/fixtures/empty/data/` | nothing: the first start must write the two commented files of 4.1.3 and 4.2.1 |
| `tests/fixtures/broken/data/jobs.toml` | a syntax error: last good value kept, `ConfigIssue` in `GET /health` |
| `tests/fixtures/claude-home/` | a fake Claude folder: `sessions\<pid>.json` files (pids are patched in by the test), a `.key` file that must never be opened (the test makes it unreadable and asserts no error), a corrupt file, `projects\<encoded cwd>\<id>.jsonl` with every line type of the transcript table and canary strings |
| `tests/fixtures/hook-events/*.json` | made-up payloads in the shapes of `keys-all.json`: `SessionStart`, `UserPromptSubmit`, `PreToolUse-bash`, `PermissionRequest-bash`, `PermissionRequest-edit`, `PermissionRequest-mcp`, `PermissionRequest-askuser`, `PostToolUse-bash`, `PostToolUse-edit`, `PostToolUseFailure`, `Stop`, `StopFailure`, `SessionEnd-clear`, `SessionEnd-other` |
| `crates/rain-backend/examples/fake_claude.rs` (a test helper binary) | reads stdin, prints the stream-json lines of a fixture file given by an environment variable, exits with the code in another variable; used as `claude_program` |

Traits and fakes (foundation 12.2; each is a field of the module `Service`, replaced through the builder):

| Trait | Production | Fake |
|---|---|---|
| `Clock` | system | controllable; with `tokio::time::pause` the scheduler tests advance time without waiting |
| `ProcessProbe` (`creation_filetime(pid)`, `image_name(pid)`, `children(pid)`) | Toolhelp and `GetProcessTimes` | a map; the registry tests also use a real spawned `ping` for one end-to-end check |
| `ExitSource` (`ExitWatcher::register`) | `RegisterWaitForSingleObject` | fires on demand |
| `WindowApi` (`top_level_windows()`, `bring_to_front(hwnd)`, `foreground()`) | Win32 | a list of fake windows; tests the choice and the result mapping of 6.6 |
| `MachineIdle` (`idle_ms() -> Option<u64>`) | `machine::Service` | scripted (includes `None`) |
| `GamesState` (`any_game_running()`) | `games::Service` | scripted |
| `ResumeSignal` | `PowerRegisterSuspendResumeNotification` | a `Notify` the test triggers |

Tests:

| Test | Proves |
|---|---|
| `jobs_list_empty_then_commented_file_written` | empty fixture: `GET /jobs` is `[]`, the file of 4.1.3 exists afterwards, a second start does not rewrite it |
| `job_toggle_writes_enabled_keeps_comments` | `PATCH /jobs/{id}` changes one key; the file differs only on that line; `id` inserted when missing; a hand edit in between gives `configChanged` only when the entry moved |
| `job_runs_and_logs` | `POST /jobs/{id}/runs` returns 201 with status running; the topic `jobs` shows `running`; after the run the run is `success`, `durationMs` set, the log has the start line, the output and the finish lines; the failing job is `failed` with `exitCode` 3 and an `error` line; a second post while running returns 200 with the same run |
| `job_run_no_executable` | a missing program gives a failed run with a "Could not start" line and no exit code |
| `job_timeout` | with time paused, advancing past `timeout_minutes` terminates the tree (a `ping -n 100`) and the run is `failed` with the timeout line |
| `job_stop_all_marks_idle` | `stop_all(budget)` ends the tree, the run is `idle` (outcome `stopped`), the log says so, `handled_until` is set back so the next start catches it up |
| `scheduler_fires_due_job` | fake clock and paused time: an enabled job due in 5 s runs once at its time and not twice; two jobs due together both run; `nextRunAt` moves |
| `scheduler_skips_overlap` | an occurrence that falls during a running run creates no second run |
| `scheduler_catch_up_on_start` | the builder pre-writes `jobs_state`; start: exactly one `catchup` run after the 60 s delay; a job with `catch_up = false` runs none; a job with no row (first seen) runs none |
| `scheduler_resume_signal` | the fake `ResumeSignal` fires after the clock jumped over a due time: one catch-up run |
| `scheduler_game_mode` | with `game_mode_active` nothing starts and `POST /jobs/{id}/runs` is `409 gameMode` |
| `run_log_caps_and_running_log` | a job that prints 6000 lines: head, omitted marker and tail; `GET /runs/{id}/log` while the run is in progress returns the lines so far; retention keeps 20 runs |
| `claude_job_end_to_end` | with the fake claude: the argument list, the prompt on stdin, the environment (no `CLAUDE*` or `ANTHROPIC*` variable, `RAIN_RUN_ID` and `RAIN_JOB_ID` present), the log mapping, the stored session id, success only when the result line says so |
| `jobs_start_closes_running_rows` | a `running` row in the database at start is closed `failed` with the interrupted line |
| `idle_sampler_only_while_enabled` | no sampler task with all idle jobs disabled (observed through a test counter), started on enable, cancelled on disable and on `pause(true)` (a run that is already `running` is not touched), started again on `pause(false)` |
| `idle_runs_pauses_resumes` | scripted idle values: queued, start at the threshold, `paused` on activity (the burner makes no progress while frozen: its output counter does not move), resume after the threshold, `success` |
| `idle_resource_exclusive` | two `cpu` jobs: one runs, the other waits; `Run now` on the waiting one gives `409 resourceBusy` |
| `idle_run_now_ignores_idle`, `idle_disable_stops_process`, `idle_restart_rebuilds_state` | 6.12.2, 6.12.6 |
| `idle_progress_reported` | `rain-progress` lines move the progress; the topic is not published for changes under 1 percent point |
| `sessions_registry_discovery` | the fixture folder gives sessions with the right fields; the `.key` file is never read; a corrupt file is skipped; unknown `status` text gives `idle` |
| `sessions_death_and_ended` | the pid of a spawned `ping` is registered; killing it makes the session `failed` (it was working) or `success` (it was idle) and `alive: false`; a wrong `procStart` is not alive; the row appears in `sessions_ended` and survives a backend restart |
| `sessions_transcript_route` | the log of the fixture session: mapping, cap of 1000 lines, `noTranscript` for a missing file, the canary never appears |
| `sessions_stop_rules` | desktop host refused, ended session `204`, unverified pid refused, a spawned child process tree of a fake "claude" (a copy of `ping.exe` renamed `claude.exe` in a temp folder) is terminated and its child too |
| `sessions_focus_choice` | with `FakeWindows`: window, app, none for the cases of 6.6 |
| `hook_event_flow` | post the fixture events in order: the session appears with the right fields; `Stop` makes it idle and sets the last message; `SessionEnd-clear` drops it silently; `SessionEnd-other` makes it `success` |
| `permission_roundtrip` | post a `permission` envelope in the background; the topic shows `waiting` with the request; `POST /sessions/{id}/permission` `once`; the held request returns 200 with the exact JSON; the book is `running` again; `always` returns the rule and the JSON of 6.5.7; `deny` returns the deny JSON |
| `permission_expiry` | with paused time: after 19 s the held request returns 204, `answerable` is false, and an answer returns `409 requestExpired` with the message of 8 |
| `permission_dropped_forwarder` | the test closes the connection early: the request becomes not answerable |
| `permission_clearing_events` | `PostToolUse` with the same input in another key order clears it; with another input it does not; `Stop` clears; a second request queues behind the first; `request` shows the first answerable |
| `permission_askuser_not_answerable` | `AskUserQuestion` returns 204 at once and the session waits with `answerable: false` |
| `permission_wrong_id_and_monitoring_off` | unknown request id: `requestExpired`; switch off: `monitoringOff` and `204` for every envelope |
| `run_owned_events_ignored` | an envelope with `env.runId` never creates a session and a `permission` one returns 204 at once; the run's `claude_session_id` is set from `SessionStart` |
| `monitor_install_remove_in_temp` | with `Config.claude_dir` in a temp folder: `PATCH /settings` switch on (through the settings module test double) writes the 12 entries and the forwarder copy; off removes them and the file equals the original byte for byte; `disableAllHooks` is refused with the message; the forwarder is replaced when its content differs |
| `monitor_sandbox_refuses_real_folder` | with `claude_dir` outside temp and the run being sandbox: `conflict sandbox`; nothing written |
| `monitor_start_reconcile` | the rows of 6.9.7 at start: needsRepair with nothing changed, blocked, off with Rain entries present (nothing changed); `install_hooks()` ends `ok` while the stored key is still false, `remove_hooks()` ends `off` while it is still true |
| `hook_process_*` (`tests/hook_process.rs`, starts the real `rain-hook.exe` with `RAIN_PIPE`) | prints the allow JSON that the harness answers; empty stdout and exit 0 when no backend runs (and under 500 ms in a debug build); empty stdout on an error status; on a server that never answers (a small fake pipe server in the test) it exits 0 with empty stdout after the wait. The hook honours the variable `RAIN_HOOK_WAIT_MS`, which can only shorten `HOOK_WAIT`, so this test does not take 20 s; the installer never sets it |
| `hook_budget` (release only, ignored in debug) | `rain-hook.exe event` with no backend: wall time median under 50 ms over 30 runs |
| `source_rules_sessions` | the scan of foundation 13.3 finds no `settings.json` outside `hooks_install.rs`, no `.claude` outside `claude_paths.rs`, and no `tracing` macro with a field named `text`, `content`, `detail`, `prompt`, `body` or `secret` in the three modules |
| `process_smoke` (existing) | starts the real backend binary; additionally checks `GET /sessions`, `/jobs`, `/idle-jobs` answer `[]` on an empty fixture and that the two commented files exist |

### 10.3 Property tests

| Test | Proves |
|---|---|
| `status_invariants` | for random sequences of the events of 6.3.2, answers, deaths and registry changes: `waiting` if and only if `pending` is not empty; an ended record is never `alive`; `request`, when present, is the first answerable entry or the first entry; a record never goes from ended back to live |
| `scheduler_does_not_spin_in_game_mode` | with `tokio::time::pause` and `game_mode_active` true while an occurrence is overdue: the number of loop iterations stays at 1 for 10 minutes of virtual time; after `pause(false)` the loop recomputes `due` and `plan_catch_up` runs once |
| `scheduler_never_double_fires` | for random clock steps and job lists: no occurrence starts two runs; `handled_until` never decreases except by the stop rule of 6.11.5 |
| `idle_one_per_resource` | for random idle inputs and enabled sets: never two `running` jobs on one resource; a `paused` job has a live process unless disabled |
| `ids_stable` | derived ids do not change when unrelated entries are added or removed before them |

### 10.4 Frontend tests (vitest, foundation 12.3)

| Test | Proves |
|---|---|
| `rest/client` tests for the 15 methods of this section | method, path, body and `media: false` of each call, with the fake bridge and canned JSON typed by the generated types |
| `answerText.test.ts` | the four notices of 9.1 |
| `follow.test.ts` | `createFollower` with fake timers: reloads while live, stops when not live, stops on `stop()`, stops on a failed load |
| `routing.test.ts` additions | the 15 methods and the 3 topics go to `real` after their stage |

### 10.5 What cannot be tested automatically, and how it is checked

| Item | Check |
|---|---|
| everything interactive about Claude Code: hooks in the terminal and the desktop app, the dialog while a hook waits, cancel on a native answer, Esc, live pickup of a changed settings file, async `SessionEnd`, an untrusted folder | the hand gate of 6.10 |
| `focusSession` on the real desktop | acceptance F1, G2.2, G2.3 |
| `RegisterWaitForSingleObject` exit watch and `PowerRegisterSuspendResumeNotification` | acceptance A8.3 (kill a session by hand and watch the row) and A6.5 (suspend the PC) |
| freezing and resuming a process tree | acceptance A6.8 |
| the owner's skill run headless | acceptance A6.1 |

---------------------------------------------------------------------------------------------------

## 11. Acceptance checks

Run on the real PC after `npm run check` is green. "Empty" is described for each.

### 11.1 Stage 6: jobs and idle jobs

| # | Do | Expect |
|---|---|---|
| A6.0 | start with an empty data folder (or delete `jobs.toml` and `idle-jobs.toml` and restart the backend) | Jobs tab: "No scheduled jobs" with the hint; Idle tab: "No idle jobs" with the hint; both files exist and contain only `schema_version` and the comments of 4.1.3 and 4.2.1 |
| A6.1 | do the seeding step of 4.1.4 on the owner's data folder; restart nothing (the file watcher reloads); open the Jobs tab | one job, switched off, schedule "Every day at 06:00", "Not scheduled" as next run; press Ctrl+R (Run now): the row shows running; open the log (Enter) and watch it follow; the run ends `success` or `failed` with a log that says why (a skill that needs a tool that is not allowed shows `Denied <tool>` lines: add the tools to `allowed_tools`, save the file, run again). The run does not appear in the Sessions tab |
| A6.2 | toggle the job with Ctrl+Enter, then open `jobs.toml` | only the `enabled` line changed; every comment is still there; toggle back: the file equals the original |
| A6.3 | with a throwaway data folder (`RAIN_DATA_DIR`) and a fixture job `cmd /c echo hi` scheduled for the next minute and enabled | it runs once at that minute, the strip shows a new green square, `nextRunAt` moves to the next day or minute |
| A6.4 | enable a job, set its `schedule` to a time 2 minutes ahead, stop the backend, wait 3 minutes, start it | about 60 s after start one run begins (the first log line says it was started after a missed run); no second run |
| A6.5 | have an enabled job due in 10 minutes, put the PC to sleep for 12 minutes, wake it | one run starts about 60 s after wake |
| A6.6 | edit `jobs.toml` by hand: change a name, add a job, add a bad cron to another | the list updates within 1 s; the bad entry is missing and `GET /health` names it in `configIssues` |
| A6.7 | start game mode (stage 9) with a run in progress | the run ends, shows a neutral square (not red), and runs once more at the next start |
| A6.8 | make an idle job that runs a CPU burner, set `idle_after_minutes = 1`, leave the PC alone for 2 minutes | state Running with progress; touch the mouse: Paused within 5 s (CPU use of the burner drops to zero in Task Manager); stay away 1 minute: Running again; it ends Done; the log shows the pause and resume lines |
| A6.9 | with all idle jobs disabled and no job enabled: `rain-backend.exe` idle for 60 s | under 30 MB working set, under 0.1 s CPU gained, no sampler line in the log; enabling one idle job logs "idle sampler started", disabling it logs "stopped" |
| A6.10 | open a job's log while it runs | lines appear without closing the dialog |

### 11.2 Stage 8a: sessions without hooks

| # | Do | Expect |
|---|---|---|
| A8.1 | open Workflows, Sessions with the switch off and a terminal and a desktop session open | both listed (see Gate 2, G2.1); with no session open: "No sessions" |
| A8.2 | G2.2 to G2.4 of the gate | as there |
| A8.3 | kill a session's process from Task Manager | the row becomes "Failed" within 2 s; a session that was idle and is closed with `/exit` becomes "Done" |
| A8.4 | close and reopen the backend | the ended sessions of the last 24 hours are still listed |
| A8.5 | rename or move the Claude folder (a copy of the registry in a temp folder, `RAIN_CLAUDE_DIR`) | the list is empty and `GET /health` shows `claude-registry` `off`, no error row |
| A8.6 | put a registry file with unknown fields and an unknown `status` value | the session shows as idle, nothing breaks |
| A8.7 | G2.5 to G2.7 | as there |
| A8.8 | `rain-backend.exe` idle for 60 s with sessions open | under 30 MB, under 0.1 s CPU gained |
| A8.9 | Task Manager while no Claude session works | no `rain-hook.exe` runs (stage 8a has no hook program at all) |

### 11.3 Stage 8b: hooks and permission answers

The acceptance of 8b is the gate of 6.10 (Gates 0 to 3), plus:

| # | Do | Expect |
|---|---|---|
| A8b.1 | `npm run check` | green, including `permission_roundtrip`, `permission_expiry`, `monitor_install_remove_in_temp`, `hook_process_*` |
| A8b.2 | with the real switch on and Rain stopped, use Claude in a terminal for a few minutes | no delay, no "hook error" notice; `hook.log` is empty or absent |
| A8b.3 | Settings: damage the installed entries by hand (delete one event), reopen Settings | the Claude row shows the failed glyph with "..."; "Repair" fixes it |
| A8b.4 | add `disableAllHooks: true` to a COPY of the settings (sandbox with `RAIN_CLAUDE_DIR`), try the switch | refused with the message of 6.9.6, file unchanged |

---------------------------------------------------------------------------------------------------

## 12. Open points

Only what cannot be decided inside this area. Each: the question, who decides, what the code does until then, what it blocks. Everything else is decided in the text above with its reason.

### 12.1 The scan rule for `.claude` (architect)

DONE in foundation 13.3 rule 2 (`sessions/claude_paths.rs` may name `.claude`; `settings.json` stays forbidden outside `hooks_install.rs`). Nothing is left to decide; the request below is kept for its reason.

Foundation 13.3 rule 2 fails the build if `.claude` or `settings.json` appears in any file other than `hooks_install.rs`. The registry and transcripts live in the `.claude` folder, which this module must read (read only). Request: the scan allows the folder name `.claude` in ONE more file, `sessions/claude_paths.rs` (paths only, no writing), and keeps `settings.json` forbidden everywhere except `hooks_install.rs`. Until decided: the file exists as described and the scan's allow-list is extended by the stage 8a author. Blocks: stage 8a (the scan).

### 12.2 A way to point the module at a throwaway Claude folder (architect)

`Config.claude_dir: Option<PathBuf>` and the environment variable `RAIN_CLAUDE_DIR` (read once by `Config::from_env`, like the others of foundation 1.3). Tests use the `Config` field; the gate of 6.10 uses the variable. Without it the installer cannot be exercised on a throwaway folder in a manual run. The variable is only honoured in sandbox mode, and the installer's `allowed()` rule still demands a temp, data or local folder. Until decided: the `Config` field exists; the variable is read by the stage 8b author. Blocks: Gate 1 (manual).

### 12.3 `serde_json` features of the workspace (architect)

The installer needs `preserve_order` (the user's key order survives). It must NOT get `arbitrary_precision` (which the spike merge crate used): with that feature `#[serde(untagged)]` enums that contain numbers, such as `HabitValue` (foundation 9.2), stop parsing, and Cargo unifies features across the workspace, so one crate asking for it would break `rain-types`. Request: add `preserve_order` to `[workspace.dependencies]` `serde_json`; never add `arbitrary_precision`. The cost is the documented normalisation of exotic numbers (6.9.4). Until decided: `preserve_order` is added by the stage 8b author. Blocks: stage 8b.

### 12.4 A folder for backups of the Claude settings (architect)

DONE in foundation 10.1 (the local folder lists `backups\claude-settings\`). Nothing is left to decide; the text below is kept for its reason.

`<local>\backups\claude-settings\` (6.9.1). The local folder of foundation 10.1 lists `rain.db`, `cache`, `logs`, `bin`; this adds `backups`. A backup must not live in `cache` (which the user may delete). Until decided: the folder is created by the installer. Blocks: nothing.

### 12.5 Timers and event sources (architect)

1. DONE in foundation 11.5 (T3: 30 s, 5 s while an idle job runs; 6.12.1).
2. DONE in foundation 11.5 (T2 degraded mode: a sleep capped at 5 minutes, 6.11.3).
3. The resume notification (`PowerRegisterSuspendResumeNotification`) as an event source, and the `RegisterWaitForSingleObject` exit watches (6.3.6): both are events, not timers, but they are not in the foundation's list of event sources (11.5 table). Until decided: used. Blocks: stage 6 (resume), stage 8.
4. DECIDED in foundation 10.4: `ConfigHub::watch_dir(path, recursive, quiet)` is the shared folder watch; the module calls it for `<claude folder>\sessions` (non-recursive). Blocks: nothing.
5. Per-run `std::thread`s for reading stdout and stderr (6.11.9): the 11.5 checklist asks to list each; they exist only while a run lives. Until decided: used.

### 12.6 Settings and routes (section 06 and architect)

1. The Settings field is `Settings.monitorClaude` (key `monitor_claude`, default false), owned by section 06, which also owns the read-only `monitorClaudeState` and the row in group Sources. Its `PATCH /settings` calls `sessions.Service::install_hooks()` or `remove_hooks()` BEFORE writing the key and aborts the write with the returned `ApiError` when it fails (6.9.6 lists the errors). When `settings.toml` is edited by hand, section 06 (6.2) calls `hooks_state()` after the reload and reports a mismatch as a `ConfigIssue`; it never installs or removes hooks because of a hand edit. Blocks: Settings switch.
2. Registry rows (foundation 14.4): the changed `answerPermission` (body `{ requestId, answer }`, result `PermissionAnswerResult`) and `focusSession` (result `FocusResult`); the internal route `POST /internal/hook` with a 4 MiB body limit. There is no `GET /claude-monitor` and no repair route: the foundation registry does not have them. Done in foundation 14.4.
3. Foundation appendix A: `HOOK_SERVER_WAIT` (19 s) next to `HOOK_WAIT` (20 s). Done (foundation Appendix A and timer T4).

### 12.7 `HOOK_WAIT` (the user, after the gate)

20 s is decision C2. The gate items G1.2 and G1.4 show whether the terminal's own dialog is visible while Rain waits. If it is hidden, 20 s of an apparently frozen terminal may be too long and the owner may prefer 8 to 12 s. Until decided: 20 s. Blocks: nothing (one constant in `rain_types::limits`).

### 12.8 What this module needs from others (other sections)

| To | Need | Until it exists |
|---|---|---|
| 02 projects | `project_id_for_path(path) -> Option<String>` | `projectId` is absent on every session |
| 02 games | `any_game_running()` (required) | none: stage 5 comes before stage 6 |
| 05 machine | `idle_ms() -> Option<u64>` as a `Service` method (`None` = Windows refused the call: the user counts as present). This module uses `idle_ms()` only: the per-job threshold `idleAfterMinutes` is compared here, and `MachineState.userActive` keeps its section 05 rule (idle under 60 s). The idle screen text becomes "PC idle for N min" without "Idle jobs can run" (section 05 open point 4) | the idle module uses `GetLastInputInfo` directly through a `platform` function if `machine` has not been built (stage 3 comes before stage 6, so it exists) |
| 06 game mode | uses `jobs.wind_down()` and `idle.wind_down()` (`WindDownPort`, 1.3) and `sessions.open_user_sessions()`, stopping no session | n/a |
| 01 launcher | search items for sessions, jobs and idle jobs from the `Service::list()` of each, with the mapping of 9.3 (stage 6 for jobs and idle, stage 8 for sessions); ids must be the same as in the lists | the router sends those types to the mock until the stage |

### 12.9 If `SetForegroundWindow` from the backend does not work (architect, after acceptance F1)

6.6 tries three ways and reports `none` when none works. If the hand check shows all three fail on this PC, the fix has to be in the shell, which owns the foreground: `api_request` for `POST /sessions/{id}/focus` would first call `AllowSetForegroundWindow(backend pid)`. That is a special case in the generic bridge, so it needs the architect. Until decided: `none` plus a taskbar flash. Blocks: nothing else.

### 12.10 The seeding step (the lead)

The owner's first job is put into the owner's data folder by hand (4.1.4); the skill name comes from `decisions.md` and is not written in any repository file. Blocks: acceptance A6.1 only.

### 12.11 Decided here, listed so the user can change them

- Stop from Rain is refused for sessions inside the Claude desktop app (6.7).
- A finished turn of a live session shows "Idle", and "Done" is a session that ended (6.3.3).
- A job stopped by game mode or shutdown shows a neutral square and runs once more at the next start (6.11.5, 6.11.7).
- A headless Claude job strips every `CLAUDE*` and `ANTHROPIC*` variable from its environment (foundation 11.6); a user who runs jobs with an API key cannot pass it through `env` (secrets never go in TOML). If that matters, a later round adds a Credential Manager reference in the job.
- Idle jobs freeze (suspend) instead of stopping when the user comes back (6.12.5).
