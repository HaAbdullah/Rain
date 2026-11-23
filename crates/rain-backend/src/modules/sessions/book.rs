//! The session book: the sessions that Rain knows, and how hook events and the registry change
//! them. Section 03 6.3 and 6.4.
//!
//! Everything in this file is plain data and pure functions. There is no I/O, no clock call and
//! no locking here: the `Service` owns the book behind one mutex, passes the time in, and
//! publishes after every change. That makes the state machine testable with a table of cases.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rain_types::time::iso_from_system_time;
use rain_types::{
    ClaudeSession, PermissionRequest, SessionEvent, SessionEventKind, SessionHost, Status,
};
use serde_json::Value;
use tokio::sync::oneshot;

use super::hook_wire::{HookEnv, text};
use super::registry::RegistryInfo;
use super::summary::{cut, first_line, input_hash, relative_path, summary};
use super::transcript::{TailFacts, model_label};

/// A session that Rain remembers after it ended: this long, and at most this many.
pub const ENDED_KEEP: Duration = Duration::from_secs(24 * 60 * 60);
pub const ENDED_KEEP_MAX: usize = 50;
/// `GET /sessions` lists at most this many ended sessions, and this many in all.
pub const ENDED_LISTED: usize = 20;
pub const LIST_MAX: usize = 40;
/// The most recent events a session keeps.
pub const RECENT_MAX: usize = 8;
/// The registry says "idle" this much later than the last hook event while hooks say "working":
/// the user pressed Esc (no hook fires for that). Unproven; a huge value switches the rule off.
pub const ESC_OVERRIDE: Duration = Duration::from_secs(3);
/// Two creation times of one process differ by this many 100 ns ticks at most (10 ms). They are
/// never compared with `==`: the registry and Windows differ by about 300 ns.
pub const START_TOLERANCE_TICKS: u64 = 100_000;
/// A request that the registry no longer calls waiting, and that cannot be answered from Rain, is
/// cleared when the registry changed this long after it arrived.
const REGISTRY_CLEARS_AFTER: Duration = Duration::from_secs(2);

/// What the hooks last told us about the work of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    Working,
    Idle,
    Failed,
    /// No hook event seen yet.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ended {
    pub at: SystemTime,
    pub ok: bool,
}

/// What `answerPermission` hands to the hook that waits: the JSON text it prints.
pub type Decision = String;

/// One open permission request. It lives in memory only.
pub struct Pending {
    pub id: String,
    pub tool: String,
    /// Equal for equal `tool_input`, whatever the order of its keys (`summary::input_hash`).
    pub input_hash: u64,
    pub summary: String,
    pub detail: String,
    pub requested_at: SystemTime,
    /// True while the hook program still waits for Rain.
    pub answerable: bool,
    /// The way back to the waiting handler. `None` when nothing waits (the request is known only
    /// because Claude Code has it open).
    pub reply: Option<oneshot::Sender<Decision>>,
    /// What "Always allow" is made from.
    pub input: Value,
    pub suggestions: Option<Value>,
}

/// Where the title of a session can come from, best first (section 03 6.4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleSources {
    /// The name in the registry, and whether the user chose it.
    pub registry_name: Option<String>,
    pub registry_name_by_user: bool,
    pub hook: Option<String>,
    pub transcript: Option<String>,
    pub first_prompt: Option<String>,
    /// A title that was stored when the session ended.
    pub stored: Option<String>,
}

impl TitleSources {
    pub fn resolve(&self, cwd: &str) -> String {
        let user_name = self
            .registry_name
            .as_ref()
            .filter(|_| self.registry_name_by_user);
        let derived_name = self.registry_name.as_ref();
        [
            user_name,
            self.stored.as_ref(),
            self.hook.as_ref(),
            self.transcript.as_ref(),
            derived_name,
            self.first_prompt.as_ref(),
        ]
        .into_iter()
        .flatten()
        .find(|title| !title.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| folder_name(cwd))
    }
}

/// The last part of a path: the name of the working folder.
pub fn folder_name(cwd: &str) -> String {
    let name = cwd
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("");
    if name.is_empty() {
        "Claude".to_owned()
    } else {
        name.to_owned()
    }
}

/// What a hook event asks the caller to do after it was applied.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// Read the tail of the transcript (first sight, after `Stop`).
    pub read_transcript: bool,
    /// The record is to be dropped silently (`/clear`, `--resume`).
    pub drop_record: bool,
    /// The session ended: `true` for a normal end.
    pub finish: Option<bool>,
}

pub struct SessionRec {
    pub id: String,
    pub pid: Option<u32>,
    /// The creation time of the process (FILETIME). From the registry, else what Windows said
    /// when the session was first seen.
    pub proc_start: Option<u64>,
    pub env_entrypoint: Option<String>,
    pub cwd: String,
    pub branch: Option<String>,
    /// The model as Claude wrote it, or a label that was stored; `model_label` shortens it.
    pub model: Option<String>,
    pub title: TitleSources,
    pub started_at: SystemTime,
    pub last_hook_at: SystemTime,
    pub turn: Turn,
    pub registry: Option<RegistryInfo>,
    pub pending: Vec<Pending>,
    pub recent: VecDeque<SessionEvent>,
    pub tokens: u64,
    pub last_message: String,
    pub transcript_path: Option<PathBuf>,
    pub stopped_by_rain: bool,
    pub ended: Option<Ended>,
    /// The exit watch of the process. Dropping it cancels the watch.
    pub watch: Option<Box<dyn Send>>,
}

impl SessionRec {
    pub fn new(id: &str, now: SystemTime) -> Self {
        Self {
            id: id.to_owned(),
            pid: None,
            proc_start: None,
            env_entrypoint: None,
            cwd: String::new(),
            branch: None,
            model: None,
            title: TitleSources::default(),
            started_at: now,
            last_hook_at: now,
            turn: Turn::Unknown,
            registry: None,
            pending: Vec::new(),
            recent: VecDeque::new(),
            tokens: 0,
            last_message: String::new(),
            transcript_path: None,
            stopped_by_rain: false,
            ended: None,
            watch: None,
        }
    }

    // ---- hook events ----

    /// Applies one hook event of mode `event` (section 03 6.3.2).
    pub fn apply_event(&mut self, event: &Value, env: &HookEnv, now: SystemTime) -> Applied {
        let mut applied = Applied::default();
        self.last_hook_at = now;
        if let Some(cwd) = text(event, "cwd") {
            cwd.clone_into(&mut self.cwd);
        }
        if let Some(path) = text(event, "transcript_path") {
            self.transcript_path = Some(PathBuf::from(path));
        }
        if self.pid.is_none() {
            self.pid = env.process_id;
        }
        if self.env_entrypoint.is_none() {
            self.env_entrypoint.clone_from(&env.entrypoint);
        }
        let name = text(event, "hook_event_name").unwrap_or("");
        let tool = text(event, "tool_name").unwrap_or("");
        let input = event.get("tool_input").cloned().unwrap_or(Value::Null);

        match name {
            "SessionStart" => {
                self.turn = Turn::Idle;
                self.set_model(text(event, "model"));
                self.set_hook_title(text(event, "session_title"));
                applied.read_transcript = true;
            }
            "UserPromptSubmit" => {
                self.turn = Turn::Working;
                self.set_hook_title(text(event, "session_title"));
                if let Some(prompt) = text(event, "prompt") {
                    self.push_recent(now, SessionEventKind::Prompt, prompt);
                }
                // The user moved on: a request that Rain cannot answer is over.
                self.pending.retain(|request| request.answerable);
            }
            "PreToolUse" => {
                self.turn = Turn::Working;
                // An edit is written by `PostToolUse`, when it is known to have happened.
                if !is_edit_tool(tool) {
                    let line = format!("{tool}: {}", summary(tool, &input, &self.cwd));
                    self.push_recent(now, SessionEventKind::Tool, &line);
                }
            }
            "PostToolUse" | "PostToolUseFailure" => {
                self.turn = Turn::Working;
                self.remove_matching(tool, &input);
                if name == "PostToolUse" && is_edit_tool(tool) {
                    let path = text(&input, "file_path")
                        .or_else(|| text(&input, "notebook_path"))
                        .unwrap_or("");
                    let shown = relative_path(path, &self.cwd);
                    self.push_recent(now, SessionEventKind::Edit, &shown);
                }
                let interrupted = event.get("is_interrupt").and_then(Value::as_bool) == Some(true);
                if name == "PostToolUseFailure"
                    && !interrupted
                    && let Some(error) = text(event, "error")
                {
                    self.push_recent(now, SessionEventKind::Error, first_line(error));
                }
            }
            "PermissionDenied" => {
                self.remove_matching(tool, &input);
                self.push_recent(
                    now,
                    SessionEventKind::Permission,
                    &format!("Denied: {tool}"),
                );
            }
            "Stop" => {
                // Work that goes on in the background keeps the session working.
                let background = event
                    .get("background_tasks")
                    .and_then(Value::as_array)
                    .is_some_and(|tasks| {
                        tasks
                            .iter()
                            .any(|task| text(task, "status") == Some("running"))
                    });
                if !background {
                    self.turn = Turn::Idle;
                }
                if let Some(message) = text(event, "last_assistant_message") {
                    self.last_message = cut(message.trim(), 600);
                    self.push_recent(now, SessionEventKind::Message, message);
                }
                self.pending.clear();
                applied.read_transcript = true;
            }
            "StopFailure" => {
                self.turn = Turn::Failed;
                let kind = text(event, "error").unwrap_or("The turn failed");
                self.push_recent(now, SessionEventKind::Error, kind);
                self.pending.clear();
            }
            "PostModelSwitch" => self.set_model(text(event, "to_model")),
            "CwdChanged" => {}
            "SessionEnd" => {
                self.pending.clear();
                match text(event, "reason") {
                    // The process lives on and a new session id follows.
                    Some("clear" | "resume") => applied.drop_record = true,
                    _ => applied.finish = Some(true),
                }
            }
            // `PermissionRequest` is handled by the permission flow. Anything else only counts
            // as activity.
            _ => {}
        }
        applied
    }

    fn set_model(&mut self, model: Option<&str>) {
        if let Some(model) = model {
            self.model = Some(model.to_owned());
        }
    }

    fn set_hook_title(&mut self, title: Option<&str>) {
        if let Some(title) = title {
            self.title.hook = Some(cut(first_line(title), 80));
        }
    }

    fn push_recent(&mut self, at: SystemTime, kind: SessionEventKind, text: &str) {
        self.recent.push_front(SessionEvent {
            at: iso_from_system_time(at),
            kind,
            text: cut(first_line(text), 200),
        });
        self.recent.truncate(RECENT_MAX);
    }

    /// Adds a recent event from outside (an answer, a stop).
    pub fn note(&mut self, at: SystemTime, kind: SessionEventKind, text: &str) {
        self.push_recent(at, kind, text);
    }

    /// Removes the open request that a finished tool call belongs to: equal tool name and an
    /// equal hash of the input (section 03 6.5.6, rule 2).
    fn remove_matching(&mut self, tool: &str, input: &Value) {
        let hash = input_hash(input);
        if let Some(at) = self
            .pending
            .iter()
            .position(|request| request.tool == tool && request.input_hash == hash)
        {
            self.pending.remove(at);
        }
    }

    // ---- the registry ----

    pub fn apply_registry(&mut self, info: RegistryInfo) {
        self.pid = Some(info.pid);
        if info.proc_start.is_some() {
            self.proc_start = info.proc_start;
        }
        if let Some(cwd) = &info.cwd
            && self.cwd.is_empty()
        {
            self.cwd.clone_from(cwd);
        }
        if let Some(ms) = info.started_at_ms {
            self.started_at = UNIX_EPOCH + Duration::from_millis(ms);
        }
        self.title.registry_name.clone_from(&info.name);
        self.title.registry_name_by_user = info.name_source.as_deref() == Some("user");

        // Rule 4 of 6.5.6: the user answered in the terminal and no hook followed. A request that
        // only Claude Code can answer is cleared when the registry no longer says "waiting".
        let still_waiting = info.status.as_deref() == Some("waiting");
        let registry_time = info
            .status_updated_at_ms
            .map(|ms| UNIX_EPOCH + Duration::from_millis(ms));
        if !still_waiting && let Some(changed) = registry_time {
            self.pending.retain(|request| {
                request.answerable || changed < request.requested_at + REGISTRY_CLEARS_AFTER
            });
        }
        self.registry = Some(info);
    }

    /// Facts read from the tail of the transcript. Hook events are newer than the transcript for
    /// what they carry, so they are not overwritten.
    pub fn apply_tail(&mut self, facts: TailFacts) {
        if let Some(model) = facts.model {
            // A model that a hook told is as good; the transcript only fills a gap.
            if self.model.is_none() {
                self.model = Some(model);
            }
        }
        if let Some(tokens) = facts.tokens {
            self.tokens = tokens;
        }
        if facts.branch.is_some() {
            self.branch = facts.branch;
        }
        if facts.title.is_some() {
            self.title.transcript = facts.title;
        }
        if facts.first_prompt.is_some() {
            self.title.first_prompt = facts.first_prompt;
        }
        if self.last_message.is_empty()
            && let Some(message) = facts.last_message
        {
            self.last_message = message;
        }
        // After a restart of Rain the recent events are the end of the transcript.
        if self.recent.is_empty() {
            self.recent = facts.recent.into_iter().collect();
        }
    }

    // ---- what is shown ----

    pub fn host(&self) -> SessionHost {
        let entrypoint = self
            .registry
            .as_ref()
            .and_then(|info| info.entrypoint.as_deref())
            .or(self.env_entrypoint.as_deref());
        match entrypoint {
            Some("cli") => SessionHost::Terminal,
            Some("claude-desktop") => SessionHost::Desktop,
            _ => SessionHost::Other,
        }
    }

    /// The later of the last hook event and the registry's own time.
    pub fn last_activity(&self) -> SystemTime {
        let registry = self
            .registry
            .as_ref()
            .and_then(|info| info.updated_at_ms)
            .map(|ms| UNIX_EPOCH + Duration::from_millis(ms));
        registry.map_or(self.last_hook_at, |at| at.max(self.last_hook_at))
    }

    /// The status shown and whether the process still runs (section 03 6.3.3). The first
    /// matching row wins.
    pub fn status(&self) -> (Status, bool) {
        if let Some(ended) = self.ended {
            return (
                if ended.ok {
                    Status::Success
                } else {
                    Status::Failed
                },
                false,
            );
        }
        if !self.pending.is_empty() {
            return (Status::Waiting, true);
        }
        let registry_status = self
            .registry
            .as_ref()
            .and_then(|info| info.status.as_deref());
        match self.turn {
            Turn::Failed => (Status::Failed, true),
            Turn::Working if self.esc_pressed() => (Status::Idle, true),
            Turn::Working => (Status::Running, true),
            Turn::Idle => (Status::Idle, true),
            // The registry is only a hint while no hook has spoken. "waiting" in the registry
            // means Claude waits for the next prompt, which is idle.
            Turn::Unknown if registry_status == Some("busy") => (Status::Running, true),
            Turn::Unknown => (Status::Idle, true),
        }
    }

    /// The registry says "idle" clearly later than the last hook event: the user pressed Esc,
    /// which fires no hook.
    fn esc_pressed(&self) -> bool {
        let Some(info) = &self.registry else {
            return false;
        };
        if info.status.as_deref() != Some("idle") {
            return false;
        }
        info.status_updated_at_ms.is_some_and(|ms| {
            UNIX_EPOCH + Duration::from_millis(ms) >= self.last_hook_at + ESC_OVERRIDE
        })
    }

    /// The first request that can be answered, else the first.
    pub fn shown_request(&self) -> Option<&Pending> {
        self.pending
            .iter()
            .find(|request| request.answerable)
            .or_else(|| self.pending.first())
    }

    pub fn to_api(&self, project_of: &dyn Fn(&str) -> Option<String>) -> ClaudeSession {
        let (status, alive) = self.status();
        ClaudeSession {
            id: self.id.clone(),
            title: self.title.resolve(&self.cwd),
            cwd: self.cwd.clone(),
            project_id: project_of(&self.cwd),
            branch: self.branch.clone(),
            model: model_label(self.model.as_deref().unwrap_or("")),
            status,
            started_at: iso_from_system_time(self.started_at),
            last_activity_at: iso_from_system_time(self.last_activity()),
            tokens: self.tokens,
            last_message: self.last_message.clone(),
            request: self.shown_request().map(|request| PermissionRequest {
                id: request.id.clone(),
                tool: request.tool.clone(),
                summary: request.summary.clone(),
                detail: request.detail.clone(),
                requested_at: iso_from_system_time(request.requested_at),
                answerable: request.answerable,
            }),
            recent: self.recent.iter().cloned().collect(),
            host: self.host(),
            alive,
        }
    }
}

/// The topic frame may be this large at most, with room to spare (`FRAME_MAX` is 256 KiB).
const FRAME_BUDGET: usize = 200 * 1024;

/// Makes a list fit one frame of the stream. Forty sessions that each hold a request with a full
/// 4096 character detail, a long last message and eight long events would not (section 03 2.4 aimed
/// at under 200 KiB). When the list is too large, the least important texts are shortened first:
/// the recent events, then the last messages. A request's `detail` is never shortened here: it is
/// the exact command that the user is asked to approve. `GET /sessions` always has everything.
pub fn fit_frame(mut list: Vec<ClaudeSession>) -> Vec<ClaudeSession> {
    let size = |list: &[ClaudeSession]| serde_json::to_string(list).map_or(0, |text| text.len());
    if size(&list) <= FRAME_BUDGET {
        return list;
    }
    for session in &mut list {
        session.recent.truncate(2);
    }
    if size(&list) <= FRAME_BUDGET {
        return list;
    }
    for session in &mut list {
        session.recent.clear();
        session.last_message = cut(&session.last_message, 200);
    }
    list
}

/// `Edit`, `Write`, `MultiEdit` and `NotebookEdit`.
pub fn is_edit_tool(tool: &str) -> bool {
    matches!(tool, "Edit" | "Write" | "MultiEdit" | "NotebookEdit")
}

/// Two creation times name the same process when they differ by less than 10 ms.
pub fn same_start(a: u64, b: u64) -> bool {
    a.abs_diff(b) < START_TOLERANCE_TICKS
}

/// All sessions that Rain knows. Plain data: the `Service` keeps it behind a mutex.
#[derive(Default)]
pub struct Book {
    pub recs: HashMap<String, SessionRec>,
    /// Claude session ids that belong to one of Rain's own job runs (a hook said so). The
    /// registry scan skips them, so a job run never shows up as a session of the user.
    pub job_sessions: HashSet<String>,
}

/// More ids than this are not remembered: the oldest knowledge is cleared at once, which only
/// risks that a run seen again through the registry alone would appear for a moment.
const JOB_SESSIONS_MAX: usize = 1000;

impl Book {
    /// Marks a session id as one of a job run: it is removed from the book and never added again.
    /// Returns true when a record was removed.
    pub fn mark_job_session(&mut self, session_id: &str) -> bool {
        if self.job_sessions.len() >= JOB_SESSIONS_MAX {
            self.job_sessions.clear();
        }
        self.job_sessions.insert(session_id.to_owned());
        self.recs.remove(session_id).is_some()
    }
}

impl Book {
    /// The list of `GET /sessions` and of the topic: live sessions first (the busiest at the top
    /// of the list that the UI sorts again), then the newest ended ones, 40 at most.
    pub fn list(
        &self,
        now: SystemTime,
        project_of: &dyn Fn(&str) -> Option<String>,
    ) -> Vec<ClaudeSession> {
        let mut live: Vec<&SessionRec> = self
            .recs
            .values()
            .filter(|rec| rec.ended.is_none())
            .collect();
        live.sort_by(|a, b| {
            b.last_activity()
                .cmp(&a.last_activity())
                .then_with(|| a.id.cmp(&b.id))
        });
        let mut ended: Vec<&SessionRec> = self
            .recs
            .values()
            .filter(|rec| {
                rec.ended
                    .is_some_and(|e| now.duration_since(e.at).unwrap_or_default() < ENDED_KEEP)
            })
            .collect();
        ended.sort_by(|a, b| {
            let (ea, eb) = (a.ended.map(|e| e.at), b.ended.map(|e| e.at));
            eb.cmp(&ea).then_with(|| a.id.cmp(&b.id))
        });
        ended.truncate(ENDED_LISTED);
        live.into_iter()
            .chain(ended)
            .take(LIST_MAX)
            .map(|rec| rec.to_api(project_of))
            .collect()
    }

    /// Sessions that run, for `open_user_sessions`.
    pub fn live_count(&self) -> usize {
        self.recs.values().filter(|rec| rec.ended.is_none()).count()
    }

    /// Forgets ended sessions that are older than 24 hours, and the oldest ones past 50.
    pub fn prune_ended(&mut self, now: SystemTime) {
        self.recs.retain(|_, rec| {
            rec.ended
                .is_none_or(|e| now.duration_since(e.at).unwrap_or_default() < ENDED_KEEP)
        });
        let mut ended: Vec<(SystemTime, String)> = self
            .recs
            .values()
            .filter_map(|rec| rec.ended.map(|e| (e.at, rec.id.clone())))
            .collect();
        if ended.len() > ENDED_KEEP_MAX {
            ended.sort();
            for (_, id) in ended.drain(..ended.len() - ENDED_KEEP_MAX) {
                self.recs.remove(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_790_000_000 + seconds)
    }

    fn rec() -> SessionRec {
        SessionRec::new("s1", t(0))
    }

    fn event(name: &str, extra: Value) -> Value {
        let mut event =
            json!({ "hook_event_name": name, "session_id": "s1", "cwd": r"C:\work\demo" });
        if let (Some(base), Some(more)) = (event.as_object_mut(), extra.as_object()) {
            base.extend(more.clone());
        }
        event
    }

    fn pending(tool: &str, input: Value, answerable: bool) -> Pending {
        Pending {
            id: "req-00000001".into(),
            tool: tool.into(),
            input_hash: input_hash(&input),
            summary: "s".into(),
            detail: "d".into(),
            requested_at: t(0),
            answerable,
            reply: None,
            input,
            suggestions: None,
        }
    }

    fn registry(status: &str, status_at_s: u64) -> RegistryInfo {
        RegistryInfo {
            pid: 1,
            session_id: "s1".into(),
            status: Some(status.into()),
            status_updated_at_ms: Some((1_790_000_000 + status_at_s) * 1000),
            ..RegistryInfo::default()
        }
    }

    #[test]
    fn a_job_session_is_taken_out_of_the_book_and_remembered() {
        let mut book = Book::default();
        book.recs.insert("s1".into(), rec());
        assert!(book.mark_job_session("s1"), "a record was removed");
        assert!(book.recs.is_empty());
        assert!(book.job_sessions.contains("s1"));
        assert!(!book.mark_job_session("s1"), "nothing left to remove");
    }

    #[test]
    fn the_status_table_row_by_row() {
        // 1 and 2: ended
        let mut r = rec();
        r.ended = Some(Ended { at: t(1), ok: true });
        assert_eq!(r.status(), (Status::Success, false));
        r.ended = Some(Ended {
            at: t(1),
            ok: false,
        });
        assert_eq!(r.status(), (Status::Failed, false));
        // 3: an open request beats everything but the end
        let mut r = rec();
        r.turn = Turn::Failed;
        r.pending.push(pending("Bash", json!({}), false));
        assert_eq!(r.status(), (Status::Waiting, true));
        // 4: failed turn
        let mut r = rec();
        r.turn = Turn::Failed;
        assert_eq!(r.status(), (Status::Failed, true));
        // 5: working
        let mut r = rec();
        r.turn = Turn::Working;
        assert_eq!(r.status(), (Status::Running, true));
        // 6: idle
        r.turn = Turn::Idle;
        assert_eq!(r.status(), (Status::Idle, true));
        // 7: nothing from hooks yet, the registry says busy
        let mut r = rec();
        r.apply_registry(registry("busy", 0));
        assert_eq!(r.status(), (Status::Running, true));
        // 8: any other registry text, or none, is idle
        for text in ["idle", "waiting", "somethingNew"] {
            let mut r = rec();
            r.apply_registry(registry(text, 0));
            assert_eq!(r.status(), (Status::Idle, true), "{text}");
        }
        assert_eq!(rec().status(), (Status::Idle, true));
    }

    #[test]
    fn the_registry_value_waiting_is_never_the_waiting_status() {
        let mut r = rec();
        r.turn = Turn::Working;
        r.apply_registry(registry("waiting", 0));
        assert_eq!(r.status().0, Status::Running);
    }

    #[test]
    fn the_esc_rule_needs_a_registry_idle_clearly_after_the_last_hook() {
        let mut r = rec();
        r.apply_event(
            &event("UserPromptSubmit", json!({})),
            &HookEnv::default(),
            t(10),
        );
        r.apply_registry(registry("idle", 11));
        assert_eq!(r.status().0, Status::Running, "only 1 s later");
        r.apply_registry(registry("idle", 13));
        assert_eq!(r.status().0, Status::Idle, "3 s later");
        r.apply_registry(registry("busy", 20));
        assert_eq!(r.status().0, Status::Running);
    }

    #[test]
    fn session_start_creates_the_facts() {
        let mut r = rec();
        let applied = r.apply_event(
            &event("SessionStart", json!({ "model": "claude-sonnet-4-5-20250929", "session_title": "Parser work", "transcript_path": r"C:\t\s1.jsonl" })),
            &HookEnv { process_id: Some(77), entrypoint: Some("cli".into()), ..HookEnv::default() },
            t(1),
        );
        assert!(applied.read_transcript);
        assert_eq!((r.turn, r.pid), (Turn::Idle, Some(77)));
        let api = r.to_api(&|_| None);
        assert_eq!(api.model, "sonnet 4.5");
        assert_eq!(api.title, "Parser work");
        assert_eq!(api.host, SessionHost::Terminal);
        assert_eq!(api.cwd, r"C:\work\demo");
    }

    #[test]
    fn prompts_tools_and_edits_become_recent_events_newest_first() {
        let mut r = rec();
        let env = HookEnv::default();
        r.apply_event(
            &event("UserPromptSubmit", json!({ "prompt": "Fix it\nplease" })),
            &env,
            t(1),
        );
        r.apply_event(
            &event(
                "PreToolUse",
                json!({ "tool_name": "Bash", "tool_input": { "command": "cargo test" } }),
            ),
            &env,
            t(2),
        );
        r.apply_event(
            &event(
                "PreToolUse",
                json!({ "tool_name": "Edit", "tool_input": { "file_path": r"C:\work\demo\a.rs" } }),
            ),
            &env,
            t(3),
        );
        r.apply_event(
            &event(
                "PostToolUse",
                json!({ "tool_name": "Edit", "tool_input": { "file_path": r"C:\work\demo\a.rs" } }),
            ),
            &env,
            t(4),
        );
        let texts: Vec<_> = r.recent.iter().map(|e| (e.kind, e.text.as_str())).collect();
        assert_eq!(
            texts,
            [
                (SessionEventKind::Edit, "a.rs"),
                (SessionEventKind::Tool, "Bash: cargo test"),
                (SessionEventKind::Prompt, "Fix it"),
            ]
        );
    }

    #[test]
    fn recent_keeps_eight_and_failures_say_why_unless_interrupted() {
        let mut r = rec();
        let env = HookEnv::default();
        for n in 0..12 {
            r.apply_event(
                &event("UserPromptSubmit", json!({ "prompt": format!("p{n}") })),
                &env,
                t(n),
            );
        }
        assert_eq!(r.recent.len(), RECENT_MAX);
        assert_eq!(r.recent[0].text, "p11");

        let failed = json!({ "tool_name": "Bash", "tool_input": {}, "error": "Exit code 1\nmore" });
        r.apply_event(&event("PostToolUseFailure", failed.clone()), &env, t(20));
        assert_eq!(
            (r.recent[0].kind, r.recent[0].text.as_str()),
            (SessionEventKind::Error, "Exit code 1")
        );
        let mut interrupted = failed;
        interrupted["is_interrupt"] = json!(true);
        r.apply_event(&event("PostToolUseFailure", interrupted), &env, t(21));
        assert_eq!(r.recent[0].kind, SessionEventKind::Error);
        assert_eq!(
            r.recent[0].at,
            iso_from_system_time(t(20)),
            "no new error event for an interrupt"
        );
    }

    #[test]
    fn stop_makes_it_idle_sets_the_message_and_clears_requests() {
        let mut r = rec();
        let env = HookEnv::default();
        r.apply_event(&event("UserPromptSubmit", json!({})), &env, t(1));
        r.pending.push(pending("Bash", json!({}), true));
        let applied = r.apply_event(
            &event(
                "Stop",
                json!({ "last_assistant_message": "All done.\nBye" }),
            ),
            &env,
            t(2),
        );
        assert!(applied.read_transcript);
        assert_eq!(r.status(), (Status::Idle, true));
        assert_eq!(r.last_message, "All done.\nBye");
        assert!(r.pending.is_empty());
    }

    #[test]
    fn a_running_background_task_keeps_the_session_working_after_stop() {
        let mut r = rec();
        let env = HookEnv::default();
        r.apply_event(&event("UserPromptSubmit", json!({})), &env, t(1));
        r.apply_event(
            &event(
                "Stop",
                json!({ "background_tasks": [{ "status": "running" }] }),
            ),
            &env,
            t(2),
        );
        assert_eq!(r.turn, Turn::Working);
        r.apply_event(
            &event(
                "Stop",
                json!({ "background_tasks": [{ "status": "done" }] }),
            ),
            &env,
            t(3),
        );
        assert_eq!(r.turn, Turn::Idle);
    }

    #[test]
    fn stop_failure_fails_the_turn_and_the_next_prompt_recovers() {
        let mut r = rec();
        let env = HookEnv::default();
        r.apply_event(
            &event("StopFailure", json!({ "error": "rate_limit" })),
            &env,
            t(1),
        );
        assert_eq!(r.status(), (Status::Failed, true));
        r.apply_event(&event("UserPromptSubmit", json!({})), &env, t(2));
        assert_eq!(r.status().0, Status::Running);
    }

    #[test]
    fn session_end_drops_for_clear_and_resume_and_finishes_for_the_rest() {
        let env = HookEnv::default();
        for reason in ["clear", "resume"] {
            let applied = rec().apply_event(
                &event("SessionEnd", json!({ "reason": reason })),
                &env,
                t(1),
            );
            assert!(applied.drop_record && applied.finish.is_none(), "{reason}");
        }
        for reason in [
            Some("logout"),
            Some("prompt_input_exit"),
            Some("other"),
            None,
        ] {
            let extra = reason.map_or(json!({}), |r| json!({ "reason": r }));
            let applied = rec().apply_event(&event("SessionEnd", extra), &env, t(1));
            assert_eq!(applied.finish, Some(true), "{reason:?}");
            assert!(!applied.drop_record);
        }
    }

    #[test]
    fn unknown_events_only_count_as_activity() {
        let mut r = rec();
        r.turn = Turn::Working;
        let applied = r.apply_event(&event("SomethingNew", json!({})), &HookEnv::default(), t(9));
        assert_eq!(applied, Applied::default());
        assert_eq!(r.turn, Turn::Working);
        assert_eq!(r.last_hook_at, t(9));
        // An event without any field at all is also fine.
        let applied = r.apply_event(&json!({}), &HookEnv::default(), t(10));
        assert_eq!(applied, Applied::default());
    }

    #[test]
    fn a_finished_tool_clears_its_request_whatever_the_key_order_and_only_that_one() {
        let mut r = rec();
        let env = HookEnv::default();
        r.pending.push(pending(
            "Bash",
            json!({ "command": "a", "timeout": 5 }),
            true,
        ));
        r.pending
            .push(pending("Bash", json!({ "command": "b" }), true));
        let other = json!({ "tool_name": "Bash", "tool_input": { "command": "z" } });
        r.apply_event(&event("PostToolUse", other), &env, t(1));
        assert_eq!(r.pending.len(), 2, "another input clears nothing");
        let same = json!({ "tool_name": "Bash", "tool_input": { "timeout": 5, "command": "a" } });
        r.apply_event(&event("PostToolUse", same), &env, t(2));
        assert_eq!(r.pending.len(), 1);
        assert_eq!(r.pending[0].input["command"], "b");
        let denied = json!({ "tool_name": "Bash", "tool_input": { "command": "b" } });
        r.apply_event(&event("PermissionDenied", denied), &env, t(3));
        assert!(r.pending.is_empty());
        assert_eq!(r.recent[0].text, "Denied: Bash");
    }

    #[test]
    fn a_new_prompt_clears_only_requests_that_rain_cannot_answer() {
        let mut r = rec();
        r.pending
            .push(pending("Bash", json!({ "command": "a" }), true));
        r.pending.push(pending("AskUserQuestion", json!({}), false));
        r.apply_event(
            &event("UserPromptSubmit", json!({})),
            &HookEnv::default(),
            t(1),
        );
        assert_eq!(r.pending.len(), 1);
        assert!(r.pending[0].answerable);
    }

    #[test]
    fn the_registry_clears_a_request_that_only_claude_can_answer() {
        let mut r = rec();
        r.pending.push(pending("Bash", json!({}), false));
        r.pending.push(pending("Edit", json!({ "a": 1 }), true));
        r.apply_registry(registry("waiting", 5));
        assert_eq!(r.pending.len(), 2, "still waiting");
        r.apply_registry(registry("busy", 1));
        assert_eq!(
            r.pending.len(),
            2,
            "changed less than 2 s after the request"
        );
        r.apply_registry(registry("busy", 3));
        assert_eq!(r.pending.len(), 1);
        assert!(
            r.pending[0].answerable,
            "an answerable request is never cleared by the registry"
        );
    }

    #[test]
    fn the_shown_request_is_the_first_answerable_else_the_first() {
        let mut r = rec();
        r.pending.push(pending("AskUserQuestion", json!({}), false));
        let mut second = pending("Bash", json!({ "c": 1 }), true);
        second.id = "req-00000002".into();
        r.pending.push(second);
        assert_eq!(r.to_api(&|_| None).request.unwrap().id, "req-00000002");
        r.pending.remove(1);
        assert_eq!(r.to_api(&|_| None).request.unwrap().tool, "AskUserQuestion");
    }

    #[test]
    fn the_title_comes_from_the_best_source_and_ends_at_the_folder_name() {
        let mut titles = TitleSources::default();
        assert_eq!(titles.resolve(r"C:\work\demo"), "demo");
        titles.first_prompt = Some("first prompt".into());
        assert_eq!(titles.resolve(""), "first prompt");
        titles.registry_name = Some("derived".into());
        assert_eq!(titles.resolve(""), "derived");
        titles.transcript = Some("from transcript".into());
        assert_eq!(titles.resolve(""), "from transcript");
        titles.hook = Some("from hook".into());
        assert_eq!(titles.resolve(""), "from hook");
        titles.registry_name_by_user = true;
        assert_eq!(
            titles.resolve(""),
            "derived",
            "a name that the user chose wins"
        );
        assert_eq!(folder_name(r"C:\a\b\"), "b");
        assert_eq!(folder_name(""), "Claude");
    }

    #[test]
    fn the_host_comes_from_the_registry_before_the_hook() {
        let mut r = rec();
        assert_eq!(r.host(), SessionHost::Other);
        r.env_entrypoint = Some("claude-desktop".into());
        assert_eq!(r.host(), SessionHost::Desktop);
        r.apply_registry(RegistryInfo {
            entrypoint: Some("cli".into()),
            ..registry("idle", 0)
        });
        assert_eq!(r.host(), SessionHost::Terminal);
        r.apply_registry(RegistryInfo {
            entrypoint: Some("sdk-ts".into()),
            ..registry("idle", 0)
        });
        assert_eq!(r.host(), SessionHost::Other);
    }

    #[test]
    fn two_creation_times_300_ns_apart_are_the_same_process_and_one_second_apart_are_not() {
        let start = 134_398_601_336_233_121;
        assert!(same_start(start, start + 3));
        assert!(same_start(start, start - 99_999));
        assert!(!same_start(start, start + 10_000_000));
    }

    #[test]
    fn the_list_has_live_sessions_first_then_the_newest_ended_ones_and_forgets_old_ones() {
        let mut book = Book::default();
        let now = t(100_000);
        for n in 0..3 {
            let mut live = SessionRec::new(&format!("live{n}"), t(1000 + n));
            live.last_hook_at = t(1000 + n);
            book.recs.insert(live.id.clone(), live);
        }
        for n in 0..25u64 {
            let mut done = SessionRec::new(&format!("done{n:02}"), t(0));
            done.ended = Some(Ended {
                at: now - Duration::from_secs(60 * (n + 1)),
                ok: true,
            });
            book.recs.insert(done.id.clone(), done);
        }
        let mut old = SessionRec::new("old", t(0));
        old.ended = Some(Ended {
            at: now - Duration::from_secs(25 * 3600),
            ok: true,
        });
        book.recs.insert("old".into(), old);

        let list = book.list(now, &|_| None);
        assert_eq!(list.len(), 3 + ENDED_LISTED);
        assert_eq!(
            list[0].id, "live2",
            "the most recently active live session first"
        );
        assert_eq!(list[3].id, "done00", "ended sessions newest first");
        assert!(list.iter().all(|s| s.id != "old"));
        assert_eq!(book.live_count(), 3);

        book.prune_ended(now);
        assert!(!book.recs.contains_key("old"));
        assert_eq!(book.recs.len(), 3 + 25);
    }

    #[test]
    fn the_ended_rows_past_fifty_are_dropped_oldest_first() {
        let mut book = Book::default();
        let now = t(100_000);
        for n in 0..60u64 {
            let mut done = SessionRec::new(&format!("d{n:02}"), t(0));
            done.ended = Some(Ended {
                at: now - Duration::from_secs(n + 1),
                ok: true,
            });
            book.recs.insert(done.id.clone(), done);
        }
        book.prune_ended(now);
        assert_eq!(book.recs.len(), ENDED_KEEP_MAX);
        assert!(book.recs.contains_key("d00") && !book.recs.contains_key("d59"));
    }

    #[test]
    fn forty_sessions_with_full_requests_stay_under_the_frame_limit() {
        let mut book = Book::default();
        for n in 0..LIST_MAX {
            let mut rec =
                SessionRec::new(&format!("{n:08}-0000-0000-0000-000000000000"), t(n as u64));
            rec.cwd = r"C:worka-rather-long-folder-namewithseverallevels".into();
            rec.last_message = "m".repeat(600);
            for k in 0..RECENT_MAX {
                rec.note(t(k as u64), SessionEventKind::Tool, &"e".repeat(200));
            }
            let mut request = pending("Bash", json!({ "n": n }), true);
            request.summary = "s".repeat(120);
            request.detail = "d".repeat(4096);
            rec.pending.push(request);
            book.recs.insert(rec.id.clone(), rec);
        }
        let list = book.list(t(100), &|_| None);
        assert_eq!(list.len(), LIST_MAX);
        // Worst case on purpose: every text at its limit. As it is, it would not fit one frame.
        let full = serde_json::to_string(&list).unwrap().len();
        assert!(
            full > rain_types::limits::FRAME_MAX,
            "the worst case is really large: {full}"
        );
        let fitted = fit_frame(list);
        let bytes = serde_json::to_string(&fitted).unwrap().len();
        assert!(bytes < FRAME_BUDGET, "{bytes} bytes");
        assert!(
            fitted
                .iter()
                .all(|s| s.request.as_ref().is_some_and(|r| r.detail.len() == 4096)),
            "no request detail is cut"
        );
        // A list that fits is left alone.
        let small = vec![rec().to_api(&|_| None)];
        assert_eq!(fit_frame(small.clone()), small);
    }

    /// A deterministic pseudo-random walk over events, registry changes and deaths. After every
    /// step the invariants of the state machine must hold (section 03 10.3, `status_invariants`).
    #[test]
    fn status_invariants_hold_over_random_sequences() {
        let names = [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "PostToolUseFailure",
            "PermissionDenied",
            "Stop",
            "StopFailure",
            "PostModelSwitch",
            "CwdChanged",
            "Odd",
        ];
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move |bound: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % bound
        };
        for _ in 0..200 {
            let mut r = rec();
            let mut was_ended = false;
            for step in 0..60u64 {
                let at = t(step);
                match next(5) {
                    0..=2 => {
                        let name = names[next(names.len() as u64) as usize];
                        let tool = json!({ "tool_name": "Bash", "tool_input": { "command": format!("c{}", next(3)) } });
                        r.apply_event(&event(name, tool), &HookEnv::default(), at);
                    }
                    3 => {
                        let id = next(3);
                        let mut request =
                            pending("Bash", json!({ "command": format!("c{id}") }), next(2) == 0);
                        request.id = format!("req-{id}");
                        r.pending.push(request);
                    }
                    _ => {
                        let status = ["busy", "idle", "waiting"][next(3) as usize];
                        r.apply_registry(registry(status, step));
                    }
                }
                if next(40) == 0 && r.ended.is_none() {
                    r.ended = Some(Ended {
                        at,
                        ok: next(2) == 0,
                    });
                    r.pending.clear();
                }
                let (status, alive) = r.status();
                assert_eq!(
                    status == Status::Waiting,
                    !r.pending.is_empty() && r.ended.is_none()
                );
                if r.ended.is_some() {
                    assert!(!alive);
                    assert!(matches!(status, Status::Success | Status::Failed));
                    was_ended = true;
                }
                if was_ended {
                    assert!(r.ended.is_some(), "an ended record never lives again");
                }
                if let Some(shown) = r.shown_request() {
                    let first_answerable = r.pending.iter().find(|p| p.answerable);
                    let expected = first_answerable.unwrap_or(&r.pending[0]);
                    assert_eq!(shown.id, expected.id);
                }
            }
        }
    }
}
