//! The definitions in `jobs.toml`, checked entry by entry. Section 03 4.1.
//!
//! The file struct keeps each `[[job]]` as a plain TOML table, so that a wrong key in one entry
//! cannot make the whole file unreadable. `load` turns the tables into `JobDef`s and says, for
//! each entry that it skipped, why.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use croner::Cron;
use rain_types::JobKind;
use serde::Deserialize;
use toml::Table;

use super::entry::{CommandLine, Entry, KeyResult, command_line, derive_ids, valid_id};
use super::schedule;

/// `jobs.toml` as the typed reader sees it. Every key of the entries is read later, one by one.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct JobsFile {
    pub job: Vec<Table>,
}

/// The file name, for messages and for the data folder.
pub const FILE_NAME: &str = "jobs.toml";
/// The name of the table array in the file.
pub const TABLE: &str = "job";

const NAME_MAX: usize = 80;
const LABEL_MAX: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    DontAsk,
    AcceptEdits,
    Default,
    Plan,
    Auto,
    BypassPermissions,
}

impl PermissionMode {
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "dontAsk" => Self::DontAsk,
            "acceptEdits" => Self::AcceptEdits,
            "default" => Self::Default,
            "plan" => Self::Plan,
            "auto" => Self::Auto,
            "bypassPermissions" => Self::BypassPermissions,
            _ => return None,
        })
    }

    /// The word `claude --permission-mode` takes.
    pub fn as_flag(self) -> &'static str {
        match self {
            Self::DontAsk => "dontAsk",
            Self::AcceptEdits => "acceptEdits",
            Self::Default => "default",
            Self::Plan => "plan",
            Self::Auto => "auto",
            Self::BypassPermissions => "bypassPermissions",
        }
    }
}

/// The keys that only a Claude job has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeJob {
    pub skill: Option<String>,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub permission_mode: PermissionMode,
    pub dangerously_skip_permissions: bool,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub max_turns: Option<u32>,
    pub program_path: String,
    pub extra_args: Vec<String>,
}

/// What a job runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Command(CommandLine),
    Claude(ClaudeJob),
}

/// One valid entry of `jobs.toml`.
#[derive(Debug, Clone)]
pub struct JobDef {
    pub id: String,
    pub name: String,
    pub kind: JobKind,
    /// The five-field text as written.
    pub schedule: String,
    pub cron: Cron,
    pub enabled: bool,
    /// The `label` of the file; `None` means the label is written from the schedule.
    pub label: Option<String>,
    pub working_dir: Option<PathBuf>,
    pub timeout_minutes: u32,
    pub catch_up: bool,
    pub env: BTreeMap<String, String>,
    pub action: Action,
}

impl PartialEq for JobDef {
    /// The parsed `Cron` has no cheap equality that matters here: the text says the same.
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.kind == other.kind
            && self.schedule == other.schedule
            && self.enabled == other.enabled
            && self.label == other.label
            && self.working_dir == other.working_dir
            && self.timeout_minutes == other.timeout_minutes
            && self.catch_up == other.catch_up
            && self.env == other.env
            && self.action == other.action
    }
}

/// An entry that was skipped. `index` counts the `[[job]]` tables from 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub index: usize,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct Loaded {
    pub defs: Vec<JobDef>,
    pub skipped: Vec<Skipped>,
}

/// Checks every entry. A bad one is skipped and named in `skipped`; the others are unaffected.
pub fn load(file: &JobsFile) -> Loaded {
    let raw: Vec<(Option<String>, Option<String>)> = file
        .job
        .iter()
        .map(|table| (text_key(table, "id"), text_key(table, "name")))
        .collect();
    let ids = derive_ids("job-", &raw);

    let mut loaded = Loaded::default();
    let mut seen: HashSet<&str> = HashSet::new();
    for (index, (table, id)) in file.job.iter().zip(&ids).enumerate() {
        let result = if !seen.insert(id.as_str()) {
            Err(format!("another job already has the id \"{id}\""))
        } else {
            parse_entry(&Entry(table), id)
        };
        match result {
            Ok(def) => loaded.defs.push(def),
            Err(message) => loaded.skipped.push(Skipped {
                index,
                message: describe(table, index, &message),
            }),
        }
    }
    loaded
}

/// Names the entry in the message, so that a person can find it in the file.
fn describe(table: &Table, index: usize, message: &str) -> String {
    match text_key(table, "name").filter(|name| !name.trim().is_empty()) {
        Some(name) => format!("Job \"{name}\": {message}"),
        None => format!("Job {}: {message}", index + 1),
    }
}

fn text_key(table: &Table, key: &str) -> Option<String> {
    table.get(key).and_then(|v| v.as_str()).map(str::to_owned)
}

fn parse_entry(entry: &Entry, id: &str) -> KeyResult<JobDef> {
    if !valid_id(id) {
        return Err("`id` may use only letters, digits and . _ ~ - (up to 128)".to_owned());
    }
    let name = entry
        .string("name")?
        .filter(|name| !name.trim().is_empty())
        .ok_or("the job has no `name`")?;
    if name.chars().count() > NAME_MAX {
        return Err(format!("`name` is longer than {NAME_MAX} characters"));
    }
    let kind = match entry.string("kind")?.as_deref() {
        Some("cron") => JobKind::Cron,
        Some("claude") => JobKind::Claude,
        Some(_) => return Err("`kind` must be \"cron\" or \"claude\"".to_owned()),
        None => return Err("the job has no `kind`".to_owned()),
    };
    let schedule_text = entry
        .string("schedule")?
        .ok_or("the job has no `schedule`")?;
    let cron = schedule::parse(&schedule_text)?;
    let label = entry.string("label")?;
    if label
        .as_ref()
        .is_some_and(|l| l.chars().count() > LABEL_MAX)
    {
        return Err(format!("`label` is longer than {LABEL_MAX} characters"));
    }
    let default_timeout = if kind == JobKind::Claude { 60 } else { 30 };
    let timeout_minutes = entry
        .int("timeout_minutes", 1, 1440)?
        .map_or(default_timeout, |n| n as u32);
    let action = match kind {
        JobKind::Cron => Action::Command(command_line(entry)?),
        JobKind::Claude => Action::Claude(claude_job(entry)?),
    };
    Ok(JobDef {
        id: id.to_owned(),
        name,
        kind,
        schedule: schedule_text,
        cron,
        enabled: entry.bool("enabled")?.unwrap_or(false),
        label,
        working_dir: entry.working_dir()?,
        timeout_minutes,
        catch_up: entry.bool("catch_up")?.unwrap_or(true),
        env: entry.env()?,
        action,
    })
}

fn claude_job(entry: &Entry) -> KeyResult<ClaudeJob> {
    let skill = entry.string("skill")?;
    if let Some(skill) = &skill {
        let allowed = (1..=100).contains(&skill.len())
            && skill
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'));
        if !allowed {
            return Err("`skill` may use only letters, digits and . _ : - (up to 100)".to_owned());
        }
    }
    let prompt = entry.string("prompt")?.filter(|p| !p.trim().is_empty());
    if skill.is_none() && prompt.is_none() {
        return Err("a Claude job needs a `skill`, a `prompt`, or both".to_owned());
    }
    let permission_mode = match entry.string("permission_mode")? {
        None => PermissionMode::DontAsk,
        Some(text) => PermissionMode::parse(&text).ok_or(
            "`permission_mode` must be \"dontAsk\", \"acceptEdits\", \"default\", \"plan\", \"auto\" or \"bypassPermissions\"",
        )?,
    };
    let dangerously_skip_permissions = entry.bool("dangerously_skip_permissions")?.unwrap_or(false);
    if permission_mode == PermissionMode::BypassPermissions && !dangerously_skip_permissions {
        return Err(
            "permission_mode = \"bypassPermissions\" needs dangerously_skip_permissions = true"
                .to_owned(),
        );
    }
    Ok(ClaudeJob {
        skill,
        prompt,
        model: entry.string("model")?.filter(|m| !m.trim().is_empty()),
        permission_mode,
        dangerously_skip_permissions,
        allowed_tools: entry.strings("allowed_tools")?,
        disallowed_tools: entry.strings("disallowed_tools")?,
        max_turns: entry.int("max_turns", 1, 1000)?.map(|n| n as u32),
        program_path: entry
            .string("claude_program")?
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| "claude".to_owned()),
        extra_args: entry.strings("extra_args")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(text: &str) -> JobsFile {
        toml::from_str(text).expect("a readable file")
    }

    #[test]
    fn a_good_command_job_gets_the_documented_defaults() {
        let loaded = load(&file(
            "[[job]]\nname = \"Back up notes\"\nkind = \"cron\"\nschedule = \"30 2 * * *\"\ncommand = \"robocopy\"\n",
        ));
        assert!(loaded.skipped.is_empty());
        let job = &loaded.defs[0];
        assert_eq!(job.id, "job-back-up-notes");
        assert!(!job.enabled, "a job without `enabled` never runs by itself");
        assert!(job.catch_up);
        assert_eq!(job.timeout_minutes, 30);
        assert_eq!(job.working_dir, None);
    }

    #[test]
    fn a_claude_job_has_a_longer_default_timeout_and_dont_ask_mode() {
        let loaded = load(&file(
            "[[job]]\nname = \"C\"\nkind = \"claude\"\nschedule = \"0 6 * * *\"\nskill = \"my-skill\"\n",
        ));
        let Action::Claude(claude) = &loaded.defs[0].action else {
            panic!("a Claude action");
        };
        assert_eq!(loaded.defs[0].timeout_minutes, 60);
        assert_eq!(claude.permission_mode, PermissionMode::DontAsk);
        assert_eq!(claude.program_path, "claude");
    }

    #[test]
    fn validate_entry() {
        let bad = [
            (
                "[[job]]\nkind = \"cron\"\nschedule = \"* * * * *\"\ncommand = \"x\"",
                "no `name`",
            ),
            (
                "[[job]]\nname = \"A\"\nschedule = \"* * * * *\"\ncommand = \"x\"",
                "no `kind`",
            ),
            (
                "[[job]]\nname = \"A\"\nkind = \"timer\"\nschedule = \"* * * * *\"",
                "`kind` must be",
            ),
            (
                "[[job]]\nname = \"A\"\nkind = \"cron\"\ncommand = \"x\"",
                "no `schedule`",
            ),
            (
                "[[job]]\nname = \"A\"\nkind = \"cron\"\nschedule = \"every day\"\ncommand = \"x\"",
                "Use five fields",
            ),
            (
                "[[job]]\nname = \"A\"\nkind = \"cron\"\nschedule = \"* * * * *\"",
                "`command` is missing",
            ),
            (
                "[[job]]\nname = \"A\"\nkind = \"cron\"\nschedule = \"* * * * *\"\ncommand = \"x\"\ntimeout_minutes = 0",
                "`timeout_minutes`",
            ),
            (
                "[[job]]\nname = \"A\"\nkind = \"cron\"\nschedule = \"* * * * *\"\ncommand = \"x\"\nenabled = \"yes\"",
                "`enabled`",
            ),
            (
                "[[job]]\nname = \"A\"\nkind = \"claude\"\nschedule = \"* * * * *\"",
                "needs a `skill`",
            ),
            (
                "[[job]]\nname = \"A\"\nkind = \"claude\"\nschedule = \"* * * * *\"\nprompt = \"x\"\npermission_mode = \"bypassPermissions\"",
                "dangerously_skip_permissions",
            ),
            (
                "[[job]]\nname = \"A\"\nkind = \"claude\"\nschedule = \"* * * * *\"\nskill = \"bad skill!\"",
                "`skill` may use",
            ),
            (
                "[[job]]\nid = \"bad id\"\nname = \"A\"\nkind = \"cron\"\nschedule = \"* * * * *\"\ncommand = \"x\"",
                "`id` may use",
            ),
        ];
        for (text, expected) in bad {
            let loaded = load(&file(text));
            assert!(loaded.defs.is_empty(), "{text}");
            assert!(
                loaded.skipped[0].message.contains(expected),
                "{text}: {}",
                loaded.skipped[0].message
            );
        }
    }

    #[test]
    fn one_bad_entry_does_not_hide_the_others_and_duplicate_ids_are_skipped() {
        let loaded = load(&file(
            "[[job]]\nname = \"One\"\nkind = \"cron\"\nschedule = \"* * * * *\"\ncommand = \"x\"\n\
             [[job]]\nname = \"Broken\"\nkind = \"cron\"\nschedule = \"nope\"\ncommand = \"x\"\n\
             [[job]]\nid = \"job-one\"\nname = \"Same id\"\nkind = \"cron\"\nschedule = \"* * * * *\"\ncommand = \"x\"\n\
             [[job]]\nname = \"Two\"\nkind = \"cron\"\nschedule = \"* * * * *\"\ncommand = \"x\"\n",
        ));
        let names: Vec<_> = loaded.defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["One", "Two"]);
        let skipped: Vec<_> = loaded.skipped.iter().map(|s| s.index).collect();
        assert_eq!(skipped, [1, 2]);
        assert!(loaded.skipped[1].message.contains("already has the id"));
    }

    #[test]
    fn entries_with_the_same_name_get_numbered_ids() {
        let loaded = load(&file(
            "[[job]]\nname = \"Same\"\nkind = \"cron\"\nschedule = \"* * * * *\"\ncommand = \"x\"\n\
             [[job]]\nname = \"Same\"\nkind = \"cron\"\nschedule = \"* * * * *\"\ncommand = \"x\"\n",
        ));
        let ids: Vec<_> = loaded.defs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["job-same", "job-same-2"]);
    }

    #[test]
    fn a_wrong_type_in_one_entry_does_not_break_reading_the_file() {
        // `schedule = 5` would fail a typed struct for the whole file; here it skips one entry.
        let loaded = load(&file(
            "[[job]]\nname = \"A\"\nkind = \"cron\"\nschedule = 5\ncommand = \"x\"\n\
             [[job]]\nname = \"B\"\nkind = \"cron\"\nschedule = \"* * * * *\"\ncommand = \"x\"\n",
        ));
        assert_eq!(loaded.defs.len(), 1);
        assert!(
            loaded.skipped[0]
                .message
                .ends_with("`schedule` must be text")
        );
    }
}
