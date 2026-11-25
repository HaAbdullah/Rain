//! Every type that crosses the Rain API, written once in Rust.
//!
//! Rust is the source of truth: `cargo test -p rain-types write_single_file` generates the
//! TypeScript copy `src/bindings/rain-types.ts` (see `bindings.rs`).
//! The rules every type follows are in docs/backend/00-foundation.md section 9.2.

mod bindings;
mod common;
mod disk;
pub mod env;
mod events;
mod gamemode;
mod games;
mod jobs;
mod launcher;
mod layout;
mod life;
pub mod limits;
mod projects;
mod sessions;
mod settings;
mod system;
pub mod time;

pub use bindings::single_file;
pub use common::{ApiError, ErrorCode, IconKind, IconRef, Status};
pub use disk::{
    DiskNode, DiskScan, Drive, FileCategory, MachineState, NodeKind, PathCheck, PathKind,
    PathsCheckBody, RevealBody, ScanStatus, StartScanBody,
};
pub use events::{
    Frame, GameMode, Games, Habits, HealthTopic, Hello, IdleJobs, Jobs, Launch, Lifecycle,
    LifecycleKind, Resync, ResyncReason, Scan, Sessions, SettingsTopic, StopReason, TOPIC_NAMES,
    Topic,
};
// projects (section 02)
pub use events::Projects;
pub use gamemode::{
    GameModeChoice, GameModeChoiceBody, GameModePhase, GameModeRecap, GameModeState, RecapItem,
    RecapOutcome, StartGameModeBody, WindDownEffect, WindDownItem, WindDownKind, WindDownStatus,
};
pub use games::{CustomGameDraft, Game, GameLibrary, GamePatch, GameSource, GameSourceStatus};
pub use jobs::{
    EnabledBody, IdleJob, IdleResource, IdleState, JobKind, JobRun, LogLevel, LogLine, ScheduledJob,
};
pub use launcher::{
    AppEntry, AppKind, LaunchMode, LaunchRun, LaunchStep, LaunchTarget, LaunchType, PinBody,
    PinnableType, Pool, PoolDraft, PoolMember, RunStatus, SearchItem, SearchType,
};
pub use layout::{
    LAYOUT_ITEMS_MAX, LAYOUT_MONITOR_MAX, LayoutItem, LayoutSave, LayoutScope, LayoutStepOption,
    LayoutWhat, LayoutZone, ProjectLayout,
};
pub use life::{
    Cadence, CalendarEvent, Habit, HabitCell, HabitDay, HabitDraft, HabitStats, HabitSummary,
    HabitTickBody, OpenNoteResult, PropertySuggestion, VaultStatus,
};
pub use projects::{EnvKind, EnvStep, Project};
pub use sessions::{
    ClaudeSession, FocusKind, FocusResult, PermissionAnswer, PermissionAnswerBody,
    PermissionAnswerResult, PermissionRequest, SessionEvent, SessionEventKind, SessionHost,
};
pub use settings::{
    CountdownSeconds, MonitorClaudeState, ResetAfterSeconds, SUMMON_HOTKEY, Settings, SettingsPatch,
};
pub use system::{
    ConfigIssue, Health, Ping, ShellFacts, ShellStatus, ShutdownBody, SourceState, SourceStatus,
};

/// Helpers shared by the unit tests of every module of this crate.
#[cfg(test)]
mod test_support {
    use serde::{Serialize, de::DeserializeOwned};
    use serde_json::Value;
    use std::fmt::Debug;

    /// Fails if the JSON contains a `null` anywhere: an absent optional field must be left out,
    /// never sent as `null` (foundation section 4.2).
    pub fn assert_no_nulls(value: &Value) {
        match value {
            Value::Null => panic!("found a null in the JSON"),
            Value::Array(items) => items.iter().for_each(assert_no_nulls),
            Value::Object(map) => map.values().for_each(assert_no_nulls),
            _ => {}
        }
    }

    /// Serialises `value`, checks there is no `null`, reads it back, and returns the JSON.
    pub fn round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T) -> Value {
        let json = serde_json::to_value(value).unwrap();
        assert_no_nulls(&json);
        let back: T = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(&back, value);
        json
    }
}
