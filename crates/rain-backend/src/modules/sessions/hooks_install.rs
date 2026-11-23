//! The hook installer: puts Rain's hooks into the user-level Claude settings file, and takes them
//! out again. Section 03 6.9.
//!
//! THIS IS THE ONLY FILE THAT MAY NAME OR CHANGE THE CLAUDE SETTINGS FILE (foundation 13.3 rule 2;
//! `tests/source_rules.rs` fails the build if the file name appears anywhere else). Everything
//! here is a plain function of paths and values, so the tests run it on throwaway files.
//!
//! What it guarantees (each point has a test at the bottom):
//!
//! - Only entries that carry the marker argument `--rain-managed=<id>` (or run `rain-hook.exe`)
//!   are ever added, replaced or removed. Rain never modifies, reorders or removes anything else.
//! - Every other key, its order, indentation, line ending, BOM and trailing newline survive. Only
//!   exotic numbers (an exponent, a very long fraction) and `\uXXXX` escapes are rewritten in
//!   their normal form: same meaning, other bytes (section 03 6.9.4).
//! - A file that does not parse, or has an unexpected shape, is refused and left untouched.
//! - A backup of the original is made (the newest 10 are kept), the write is atomic (a temporary
//!   file in the same folder, then a rename), and a change made by someone else between the read
//!   and the rename (Claude Code rewrites this file itself at times) is merged, not overwritten.
//! - After writing, the file is read back and checked.
//!
//! A sandbox run (a debug build, a test, any `RAIN_*` override) never writes a file outside the
//! system temporary folder and the data and local folders of the run: `allowed`.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use rain_types::limits::HOOK_HANDLER_TIMEOUT;
use serde::Serialize;
use serde_json::ser::PrettyFormatter;
use serde_json::{Map, Value, json};

use super::claude_paths::ClaudePaths;
use crate::core::error::ApiError;

/// Every handler that Rain writes has an argument that starts with this. It is the only thing
/// that Rain trusts to find its own entries (besides the name of the program).
pub const MARK_PREFIX: &str = "--rain-managed=";
/// The program of the hook. A handler that runs it is Rain's even without the marker.
pub const FORWARDER_FILE: &str = "rain-hook.exe";
/// The number of events that Rain installs a handler for.
pub const EVENTS_EXPECTED: usize = 12;
/// Newest backups kept.
const BACKUPS_KEPT: usize = 10;
/// The whole read-merge-write is repeated this often when somebody else writes meanwhile.
const MAX_ATTEMPTS: usize = 4;
/// A rename that fails because another program holds the file open is tried again this often.
const RENAME_RETRIES: u32 = 4;
const RENAME_DELAY: Duration = Duration::from_millis(25);

/// Events that are only observed. They run in the background (`async`), so Claude never waits.
const OBSERVE_ASYNC: [&str; 10] = [
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
];
/// Synchronous on purpose: its budget is 1.5 s, the program needs about 20 ms, and whether a
/// background hook survives the exit of an interactive session was never seen.
const OBSERVE_SYNC: &str = "SessionEnd";
const PERMISSION_EVENT: &str = "PermissionRequest";

/// The file name of the settings. Only this file names it.
pub fn settings_file(paths: &ClaudePaths) -> PathBuf {
    paths.root().join("settings.json")
}

// ---- what is installed ----

/// What the handlers are made from.
#[derive(Debug, Clone)]
pub struct Spec {
    /// The absolute path of the hook program.
    pub forwarder: String,
    /// Six hex characters, made at the first install and reused.
    pub install_id: String,
    /// The pipe of the backend that installs the hooks. `None` only in unit tests of the file format.
    pub pipe: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Installed {
        events: usize,
        backup: Option<PathBuf>,
    },
    Unchanged,
    Removed {
        handlers: usize,
        backup: Option<PathBuf>,
    },
}

/// Why the file was not changed. The text is shown to the user as the notice of the Settings
/// switch, so each one is a plain sentence (section 03 6.9.6).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InstallError {
    #[error("Your Claude settings file is not valid JSON. Rain did not change it.")]
    InvalidJson,
    #[error(
        "Your Claude settings file has an unexpected shape in its hooks section. Rain did not change it."
    )]
    BadShape,
    #[error(
        "Claude hooks are switched off in your Claude settings (disableAllHooks). Turn them on first."
    )]
    HooksDisabled,
    #[error("Could not write the Claude settings file.")]
    CannotWrite,
}

impl InstallError {
    pub fn into_api(self) -> ApiError {
        match self {
            Self::HooksDisabled => ApiError::conflict("hooksDisabled", self.to_string()),
            _ => ApiError::failed(self.to_string()).with_reason("installFailed"),
        }
    }
}

/// The exact handlers that Rain merges into the file: (event, handler).
pub fn rain_handlers(spec: &Spec) -> Vec<(&'static str, Value)> {
    let mark = format!("{MARK_PREFIX}{}", spec.install_id);
    let args = |mode: &str| -> Value {
        let mut args = vec![json!(mode)];
        if let Some(pipe) = &spec.pipe {
            args.push(json!(format!("--pipe={pipe}")));
        }
        args.push(json!(mark));
        Value::Array(args)
    };
    let mut handlers = Vec::new();
    for event in OBSERVE_ASYNC {
        handlers.push((
            event,
            json!({ "type": "command", "command": spec.forwarder, "args": args("event"), "async": true }),
        ));
    }
    handlers.push((
        OBSERVE_SYNC,
        json!({ "type": "command", "command": spec.forwarder, "args": args("event") }),
    ));
    handlers.push((
        PERMISSION_EVENT,
        json!({ "type": "command", "command": spec.forwarder, "args": args("permission"),
                "timeout": HOOK_HANDLER_TIMEOUT.as_secs() }),
    ));
    handlers
}

/// True for a handler that Rain wrote: a command with an argument that starts with the marker,
/// or one that runs the program of the hook.
pub fn is_rain_handler(handler: &Value) -> bool {
    if handler.get("type").and_then(Value::as_str) != Some("command") {
        return false;
    }
    let marked = handler
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| {
            args.iter().any(|arg| {
                arg.as_str()
                    .is_some_and(|text| text.starts_with(MARK_PREFIX))
            })
        });
    let runs_forwarder = handler
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| base_name(command).eq_ignore_ascii_case(FORWARDER_FILE));
    marked || runs_forwarder
}

fn base_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// The install id of an entry that is already in the file, so that a repair keeps it.
pub fn existing_install_id(root: &Value) -> Option<String> {
    let hooks = root.get("hooks")?.as_object()?;
    hooks
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter(|handler| is_rain_handler(handler))
        .filter_map(|handler| handler.get("args").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .find_map(|arg| arg.strip_prefix(MARK_PREFIX))
        .map(str::to_owned)
}

/// Removes Rain's handlers (any install id). Empties that the removal made are pruned: a matcher
/// group that Rain emptied, an event array that became empty, an empty `hooks` object. A user
/// handler inside a Rain group survives with its group. Returns how many handlers were removed.
pub fn remove_rain_hooks(root: &mut Value) -> usize {
    let Some(object) = root.as_object_mut() else {
        return 0;
    };
    let Some(hooks) = object.get_mut("hooks").and_then(Value::as_object_mut) else {
        return 0;
    };
    let mut removed = 0;
    let mut emptied_events = Vec::new();
    for (event, groups) in hooks.iter_mut() {
        let Some(groups) = groups.as_array_mut() else {
            continue;
        };
        let mut changed = false;
        groups.retain_mut(|group| {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let before = handlers.len();
            handlers.retain(|handler| !is_rain_handler(handler));
            let gone = before - handlers.len();
            removed += gone;
            changed |= gone > 0;
            // A group is dropped only when Rain's removal emptied it.
            !(gone > 0 && handlers.is_empty())
        });
        if changed && groups.is_empty() {
            emptied_events.push(event.clone());
        }
    }
    for event in emptied_events {
        hooks.shift_remove(&event);
    }
    if removed > 0 && hooks.is_empty() {
        object.shift_remove("hooks");
    }
    removed
}

/// Adds Rain's handlers, each as a matcher group of its own at the end of the event's array
/// (nothing of the user's is touched or reordered). Rain's existing handlers are replaced first,
/// so a moved program path is repaired and never duplicated.
pub fn add_rain_hooks(root: &mut Value, spec: &Spec) -> Result<(), InstallError> {
    remove_rain_hooks(root);
    let object = root.as_object_mut().ok_or(InstallError::BadShape)?;
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or(InstallError::BadShape)?;
    for (event, handler) in rain_handlers(spec) {
        let groups = hooks
            .entry(event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or(InstallError::BadShape)?;
        groups.push(json!({ "matcher": "", "hooks": [handler] }));
    }
    Ok(())
}

// ---- reading and writing the text ----

/// How the file was written, so that the new text looks the same.
struct Format {
    bom: bool,
    crlf: bool,
    indent: String,
    trailing_newline: bool,
}

impl Format {
    fn of(text: &str) -> Self {
        let body = text.strip_prefix('\u{feff}').unwrap_or(text);
        let indent = body
            .lines()
            .find(|line| line.starts_with(' ') || line.starts_with('\t'))
            .map(|line| {
                line.chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect::<String>()
            })
            .unwrap_or_else(|| "  ".to_owned());
        Self {
            bom: text.starts_with('\u{feff}'),
            crlf: body.contains("\r\n"),
            indent,
            trailing_newline: body.ends_with('\n'),
        }
    }

    fn fresh() -> Self {
        Self {
            bom: false,
            crlf: false,
            indent: "  ".to_owned(),
            trailing_newline: true,
        }
    }

    fn render(&self, value: &Value) -> Result<String, InstallError> {
        let mut buffer = Vec::new();
        let formatter = PrettyFormatter::with_indent(self.indent.as_bytes());
        let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, formatter);
        value
            .serialize(&mut serializer)
            .map_err(|_| InstallError::CannotWrite)?;
        let mut text = String::from_utf8(buffer).map_err(|_| InstallError::CannotWrite)?;
        if self.crlf {
            text = text.replace('\n', "\r\n");
        }
        if self.trailing_newline {
            text.push_str(if self.crlf { "\r\n" } else { "\n" });
        }
        if self.bom {
            text.insert(0, '\u{feff}');
        }
        Ok(text)
    }
}

/// Parses the file. A root that is not an object, a `hooks` that is not an object, or an event
/// value that is not an array is refused: Rain would have to guess what to keep.
fn parse(bytes: &[u8]) -> Result<(Value, String), InstallError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| InstallError::InvalidJson)?
        .to_owned();
    let body = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let value: Value = serde_json::from_str(body).map_err(|_| InstallError::InvalidJson)?;
    let object = value.as_object().ok_or(InstallError::BadShape)?;
    if let Some(hooks) = object.get("hooks") {
        let hooks = hooks.as_object().ok_or(InstallError::BadShape)?;
        if hooks.values().any(|groups| !groups.is_array()) {
            return Err(InstallError::BadShape);
        }
    }
    Ok((value, text))
}

fn compact(value: &Value) -> String {
    // Order-sensitive, because of `preserve_order`.
    serde_json::to_string(value).unwrap_or_default()
}

fn disables_hooks(value: &Value) -> bool {
    value.get("disableAllHooks") == Some(&Value::Bool(true))
}

// ---- backup and the atomic write ----

/// A name that sorts by time: `20261121T093000123Z` from `2026-11-21T09:30:00.123Z`.
fn backup_stamp(iso: &str) -> String {
    iso.replace([':', '-', '.'], "")
}

fn make_backup(
    original: &[u8],
    dir: &Path,
    stamp: &str,
    keep: usize,
) -> Result<PathBuf, InstallError> {
    fs::create_dir_all(dir).map_err(|_| InstallError::CannotWrite)?;
    let target = dir.join(format!("settings.json.{stamp}"));
    fs::write(&target, original).map_err(|_| InstallError::CannotWrite)?;
    let mut all: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|_| InstallError::CannotWrite)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("settings.json."))
        })
        .collect();
    all.sort();
    while all.len() > keep {
        let _ = fs::remove_file(all.remove(0));
    }
    Ok(target)
}

/// Replaces `target` with `temp`. A rename that fails because another program holds the target
/// open (Claude Code reading it) is tried again a few times.
fn replace_file(temp: &Path, target: &Path) -> std::io::Result<()> {
    let mut last = None;
    for attempt in 0..RENAME_RETRIES {
        match fs::rename(temp, target) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        if attempt + 1 < RENAME_RETRIES {
            // This runs on a blocking thread (`core::blocking::run`), so sleeping blocks no async
            // worker, and the wait is the whole point.
            #[allow(clippy::disallowed_methods)]
            std::thread::sleep(RENAME_DELAY);
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no attempt was made")))
}

/// What a run changed: how many handlers, the backup it made, and the bytes it started from.
type Changed = (usize, Option<PathBuf>, Option<Vec<u8>>);

/// The change that one run makes.
type Edit<'a> = &'a dyn Fn(&mut Value) -> Result<usize, InstallError>;

/// The read, edit, write loop that install and uninstall share. `before_rename` lets a test play
/// another program that writes the file at the worst moment.
fn modify(
    path: &Path,
    backup_dir: &Path,
    stamp: &str,
    check_disabled: bool,
    edit: Edit<'_>,
    before_rename: &mut dyn FnMut(usize),
) -> Result<Changed, InstallError> {
    for attempt in 0..MAX_ATTEMPTS {
        let original = match fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(InstallError::CannotWrite),
        };
        let (mut value, format) = match &original {
            Some(bytes) => {
                let (value, text) = parse(bytes)?;
                (value, Format::of(&text))
            }
            None => (Value::Object(Map::new()), Format::fresh()),
        };
        if check_disabled && disables_hooks(&value) {
            return Err(InstallError::HooksDisabled);
        }
        let before = compact(&value);
        let changed = edit(&mut value)?;
        if changed == 0 || compact(&value) == before {
            return Ok((0, None, original));
        }
        let text = format.render(&value)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|_| InstallError::CannotWrite)?;
        }
        let temp = path.with_extension("json.rain-tmp");
        fs::write(&temp, text.as_bytes()).map_err(|_| InstallError::CannotWrite)?;
        before_rename(attempt);
        // Did somebody else change the file while Rain worked? Then start again from their
        // version.
        if fs::read(path).ok() != original {
            let _ = fs::remove_file(&temp);
            continue;
        }
        let backup = match &original {
            Some(bytes) => Some(
                make_backup(bytes, backup_dir, stamp, BACKUPS_KEPT).inspect_err(|_| {
                    let _ = fs::remove_file(&temp);
                })?,
            ),
            None => None,
        };
        replace_file(&temp, path).map_err(|_| {
            let _ = fs::remove_file(&temp);
            InstallError::CannotWrite
        })?;
        return Ok((changed, backup, original));
    }
    Err(InstallError::CannotWrite)
}

/// Installs Rain's 12 handlers. `stamp` names the backup (see `backup_stamp`).
pub fn install(
    path: &Path,
    backup_dir: &Path,
    spec: &Spec,
    stamp: &str,
) -> Result<Outcome, InstallError> {
    install_with_hook(path, backup_dir, spec, stamp, &mut |_| {})
}

fn install_with_hook(
    path: &Path,
    backup_dir: &Path,
    spec: &Spec,
    stamp: &str,
    hook: &mut dyn FnMut(usize),
) -> Result<Outcome, InstallError> {
    let edit = |value: &mut Value| add_rain_hooks(value, spec).map(|()| EVENTS_EXPECTED);
    let (changed, backup, before) = modify(path, backup_dir, stamp, true, &edit, hook)?;
    if changed == 0 {
        return Ok(Outcome::Unchanged);
    }
    // Verify by reading back: without Rain's entries the file must equal what it was before.
    let after = fs::read(path).map_err(|_| InstallError::CannotWrite)?;
    let (mut written, _) = parse(&after)?;
    let (rain_entries, _) = count_rain_entries(&written);
    remove_rain_hooks(&mut written);
    let mut expected = match &before {
        Some(bytes) => parse(bytes)?.0,
        None => Value::Object(Map::new()),
    };
    // An older install of Rain in the file was replaced, so compare without it.
    remove_rain_hooks(&mut expected);
    if rain_entries < EVENTS_EXPECTED || compact(&written) != compact(&expected) {
        return Err(InstallError::CannotWrite);
    }
    Ok(Outcome::Installed {
        events: changed,
        backup,
    })
}

/// Removes every handler of Rain. A file that does not exist, or has none, is `Unchanged`.
pub fn uninstall(path: &Path, backup_dir: &Path, stamp: &str) -> Result<Outcome, InstallError> {
    let edit = |value: &mut Value| Ok(remove_rain_hooks(value));
    let (removed, backup, _) = modify(path, backup_dir, stamp, false, &edit, &mut |_| {})?;
    if removed == 0 {
        return Ok(Outcome::Unchanged);
    }
    let after = fs::read(path).map_err(|_| InstallError::CannotWrite)?;
    let (written, _) = parse(&after)?;
    if count_rain_entries(&written).0 > 0 {
        return Err(InstallError::CannotWrite);
    }
    Ok(Outcome::Removed {
        handlers: removed,
        backup,
    })
}

// ---- looking at the file ----

/// How many of the 12 expected events have a Rain handler, and how many Rain handlers there are
/// at all.
pub fn count_rain_entries(root: &Value) -> (usize, usize) {
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return (0, 0);
    };
    let mut events = 0;
    let mut handlers = 0;
    for (event, groups) in hooks {
        let found = groups
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|group| group.get("hooks").and_then(Value::as_array))
            .flatten()
            .filter(|handler| is_rain_handler(handler))
            .count();
        handlers += found;
        let expected = OBSERVE_ASYNC.contains(&event.as_str())
            || event == OBSERVE_SYNC
            || event == PERMISSION_EVENT;
        if found > 0 && expected {
            events += 1;
        }
    }
    (events, handlers)
}

/// What the checks found (section 03 3.1: internal, not an API type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookState {
    Off,
    Ok,
    Broken,
    Blocked,
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct HooksStatus {
    /// What the switch is meant to be (the argument `expected_on`).
    pub enabled: bool,
    pub state: HookState,
    /// A plain sentence, set when the state is not `Ok` or `Off`.
    pub message: Option<String>,
    pub events_installed: u32,
    pub events_expected: u32,
    pub forwarder_ok: bool,
    /// The last hook event that this backend received since it started.
    pub last_event_at: Option<String>,
}

/// Looks at the settings file and says how the hooks stand, given what the switch should be.
/// `forwarder_ok` is whether the copied program is there and up to date; `claude_found` is
/// whether the Claude folder exists. Reads one small file and changes nothing.
pub fn status(
    path: &Path,
    expected_on: bool,
    forwarder_ok: bool,
    claude_found: bool,
) -> HooksStatus {
    let result = |state, message: Option<&str>, installed: usize| HooksStatus {
        enabled: expected_on,
        state,
        message: message.map(str::to_owned),
        events_installed: installed as u32,
        events_expected: EVENTS_EXPECTED as u32,
        forwarder_ok,
        last_event_at: None,
    };
    if !claude_found {
        return if expected_on {
            result(
                HookState::Unavailable,
                Some("Claude Code was not found on this PC."),
                0,
            )
        } else {
            result(HookState::Off, None, 0)
        };
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => {
            return if expected_on {
                result(
                    HookState::Unavailable,
                    Some("Could not read the Claude settings file."),
                    0,
                )
            } else {
                result(HookState::Off, None, 0)
            };
        }
    };
    let parsed = if bytes.is_empty() {
        Ok((Value::Object(Map::new()), String::new()))
    } else {
        parse(&bytes)
    };
    let (value, _) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            return if expected_on {
                result(HookState::Blocked, Some(&error.to_string()), 0)
            } else {
                result(HookState::Off, None, 0)
            };
        }
    };
    let (events, handlers) = count_rain_entries(&value);
    if !expected_on {
        return result(HookState::Off, None, events);
    }
    if disables_hooks(&value) {
        return result(
            HookState::Blocked,
            Some(&InstallError::HooksDisabled.to_string()),
            events,
        );
    }
    if events < EVENTS_EXPECTED || handlers < EVENTS_EXPECTED {
        return result(
            HookState::Broken,
            Some("Some of Rain's Claude hooks are missing. Repair them in Settings."),
            events,
        );
    }
    if !forwarder_ok {
        return result(
            HookState::Broken,
            Some("The Rain hook program is missing or out of date. Repair it in Settings."),
            events,
        );
    }
    result(HookState::Ok, None, events)
}

// ---- the sandbox rule ----

/// A path made absolute and with every folder that exists resolved, also when the file itself
/// does not exist yet: the nearest existing parent is resolved and the rest is appended.
fn lenient_canonical(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        if let Ok(resolved) = fs::canonicalize(&current) {
            return rest.iter().rev().fold(resolved, |acc, part| acc.join(part));
        }
        match (current.file_name().map(ToOwned::to_owned), current.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                current = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// True when `path` lies inside `folder` (both resolved, case ignored).
fn is_inside(path: &Path, folder: &Path) -> bool {
    let key = |path: &Path| -> Vec<String> {
        path.components()
            .filter(|c| !matches!(c, Component::CurDir))
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect()
    };
    let (path, folder) = (
        key(&lenient_canonical(path)),
        key(&lenient_canonical(folder)),
    );
    !folder.is_empty() && path.len() >= folder.len() && path[..folder.len()] == folder[..]
}

/// Whether the installer may write `target`. A real run may. A sandbox run may only write inside
/// the system temporary folder or the data or local folder of this run, so that it can never
/// change the real settings file of the user (foundation 2.7 and 13.3).
pub fn allowed(target: &Path, sandbox: bool, safe_folders: &[&Path]) -> bool {
    !sandbox || safe_folders.iter().any(|folder| is_inside(target, folder))
}

/// The backups of the settings file, in the local folder.
pub fn backup_dir(local_dir: &Path) -> PathBuf {
    local_dir.join("backups").join("claude-settings")
}

/// The place where the hook program is copied to, in the local folder (foundation appendix B).
pub fn forwarder_path(local_dir: &Path) -> PathBuf {
    local_dir.join("bin").join(FORWARDER_FILE)
}

/// A new install id: six hex characters from the time and the process id.
pub fn new_install_id(now_ms: u128) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    (now_ms, std::process::id()).hash(&mut hasher);
    format!("{:06x}", hasher.finish() & 0x00ff_ffff)
}

/// The text for a backup file name from a time.
pub fn stamp_of(iso: &str) -> String {
    backup_stamp(iso)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FWD: &str = r"C:\Users\someone\AppData\Local\Rain\bin\rain-hook.exe";
    const STAMP: &str = "20261121T093000000Z";

    fn spec() -> Spec {
        Spec {
            forwarder: FWD.into(),
            install_id: "abc123".into(),
            pipe: None,
        }
    }

    /// A settings file with other keys, numbers, user hooks in the same events as Rain's, and an
    /// untouched neighbour. All names are made up.
    const FIXTURE: &str = r#"{
  "theme": "dark",
  "env": {
    "RATIO": 1.0,
    "BIG": 12345678901234567890
  },
  "permissions": {
    "allow": [
      "Bash(git status)"
    ]
  },
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "C:\\tools\\audit.exe",
            "args": [
              "--log"
            ]
          }
        ]
      }
    ],
    "PermissionRequest": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "C:\\tools\\their-permission.exe",
            "timeout": 10
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "C:\\tools\\stop.exe"
          }
        ]
      }
    ]
  },
  "modelSettings": {
    "default": "sonnet"
  }
}
"#;

    struct Sandbox {
        dir: tempfile::TempDir,
    }

    impl Sandbox {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
            }
        }
        fn file(&self) -> PathBuf {
            self.dir.path().join("settings.json")
        }
        fn backups(&self) -> PathBuf {
            self.dir.path().join("backups")
        }
        fn install(&self, spec: &Spec, stamp: &str) -> Result<Outcome, InstallError> {
            install(&self.file(), &self.backups(), spec, stamp)
        }
        fn uninstall(&self) -> Result<Outcome, InstallError> {
            uninstall(&self.file(), &self.backups(), STAMP)
        }
        fn text(&self) -> String {
            fs::read_to_string(self.file()).unwrap()
        }
    }

    #[test]
    fn install_then_uninstall_gives_back_the_same_bytes() {
        let sb = Sandbox::new();
        fs::write(sb.file(), FIXTURE).unwrap();
        assert!(matches!(
            sb.install(&spec(), STAMP).unwrap(),
            Outcome::Installed { events: 12, .. }
        ));
        let installed = sb.text();
        assert!(installed.contains("--rain-managed=abc123"));
        assert!(
            installed.contains("their-permission.exe") && installed.contains("audit.exe"),
            "user hooks stay"
        );
        assert!(
            installed.contains("\"RATIO\": 1.0") && installed.contains("12345678901234567890"),
            "number text is kept"
        );
        assert!(matches!(
            sb.uninstall().unwrap(),
            Outcome::Removed { handlers: 12, .. }
        ));
        assert_eq!(sb.text(), FIXTURE, "byte for byte after a full round trip");
    }

    #[test]
    fn user_entries_are_untouched_and_rain_comes_last() {
        let sb = Sandbox::new();
        fs::write(sb.file(), FIXTURE).unwrap();
        sb.install(&spec(), STAMP).unwrap();
        let (value, _) = parse(&fs::read(sb.file()).unwrap()).unwrap();
        let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            ["theme", "env", "permissions", "hooks", "modelSettings"],
            "top-level order kept"
        );
        let requests = value["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0]["hooks"][0]["command"],
            "C:\\tools\\their-permission.exe"
        );
        assert_eq!(requests[1]["hooks"][0]["timeout"], 35);
        assert_eq!(requests[1]["hooks"][0]["args"][0], "permission");
        assert_eq!(value["hooks"]["SessionStart"][0]["hooks"][0]["async"], true);
        assert!(
            value["hooks"]["SessionEnd"][0]["hooks"][0]
                .get("async")
                .is_none()
        );
    }

    #[test]
    fn installing_twice_changes_nothing_and_a_new_path_replaces_not_duplicates() {
        let sb = Sandbox::new();
        fs::write(sb.file(), FIXTURE).unwrap();
        sb.install(&spec(), STAMP).unwrap();
        let once = fs::read(sb.file()).unwrap();
        assert_eq!(sb.install(&spec(), STAMP).unwrap(), Outcome::Unchanged);
        assert_eq!(fs::read(sb.file()).unwrap(), once);
        let moved = Spec {
            forwarder: r"D:\moved\rain-hook.exe".into(),
            ..spec()
        };
        sb.install(&moved, "20261121T093001000Z").unwrap();
        let text = sb.text();
        assert_eq!(
            text.matches("--rain-managed=").count(),
            12,
            "still exactly one entry per event"
        );
        assert!(
            text.contains(r"D:\\moved\\rain-hook.exe") && !text.contains(r"AppData\\Local\\Rain"),
            "old path is gone"
        );
    }

    #[test]
    fn file_without_hooks_key_returns_to_no_hooks_key() {
        let sb = Sandbox::new();
        let original = "{\n  \"theme\": \"dark\"\n}\n";
        fs::write(sb.file(), original).unwrap();
        sb.install(&spec(), STAMP).unwrap();
        assert!(sb.text().contains("\"hooks\""));
        sb.uninstall().unwrap();
        assert_eq!(sb.text(), original);
    }

    #[test]
    fn absent_file_is_created_and_removal_leaves_an_empty_object() {
        let sb = Sandbox::new();
        assert!(matches!(
            sb.install(&spec(), STAMP).unwrap(),
            Outcome::Installed { backup: None, .. }
        ));
        assert!(sb.file().exists());
        sb.uninstall().unwrap();
        assert_eq!(sb.text(), "{}\n");
    }

    #[test]
    fn broken_or_blocked_files_are_refused_and_not_modified() {
        let sb = Sandbox::new();
        let cases: [(&str, InstallError); 5] = [
            ("{ \"theme\": ", InstallError::InvalidJson),
            ("[1,2]", InstallError::BadShape),
            ("{\"hooks\": 5}", InstallError::BadShape),
            ("{\"hooks\": {\"Stop\": {}}}", InstallError::BadShape),
            ("{\"disableAllHooks\": true}", InstallError::HooksDisabled),
        ];
        for (bad, expected) in cases {
            fs::write(sb.file(), bad).unwrap();
            assert_eq!(sb.install(&spec(), STAMP).unwrap_err(), expected, "{bad}");
            assert_eq!(sb.text(), bad, "file unchanged for {bad}");
        }
        assert!(
            !sb.dir.path().join("settings.json.rain-tmp").exists(),
            "no temp file left behind"
        );
        assert!(
            !sb.backups().exists(),
            "no backup for a file that was not changed"
        );
    }

    #[test]
    fn the_refusals_have_the_messages_and_reasons_of_the_section() {
        let invalid = InstallError::InvalidJson.into_api();
        assert_eq!(invalid.body.reason.as_deref(), Some("installFailed"));
        assert_eq!(
            invalid.body.message,
            "Your Claude settings file is not valid JSON. Rain did not change it."
        );
        let disabled = InstallError::HooksDisabled.into_api();
        assert_eq!(disabled.body.reason.as_deref(), Some("hooksDisabled"));
        assert_eq!(disabled.status, axum::http::StatusCode::CONFLICT);
        assert_eq!(
            InstallError::CannotWrite.into_api().body.message,
            "Could not write the Claude settings file."
        );
    }

    #[test]
    fn disable_all_hooks_does_not_stop_a_removal() {
        let sb = Sandbox::new();
        let text = r#"{"disableAllHooks":true,"hooks":{"Stop":[{"matcher":"","hooks":[{"type":"command","command":"x.exe","args":["event","--rain-managed=zz"]}]}]}}"#;
        fs::write(sb.file(), text).unwrap();
        assert!(matches!(
            sb.uninstall().unwrap(),
            Outcome::Removed { handlers: 1, .. }
        ));
    }

    #[test]
    fn crlf_tabs_and_bom_are_kept() {
        let sb = Sandbox::new();
        let original = "\u{feff}{\r\n\t\"theme\": \"dark\",\r\n\t\"note\": \"caf\\u00e9\"\r\n}\r\n";
        fs::write(sb.file(), original).unwrap();
        sb.install(&spec(), STAMP).unwrap();
        let text = sb.text();
        assert!(
            text.starts_with('\u{feff}')
                && text.contains("\r\n\t\"theme\"")
                && !text.replace("\r\n", "").contains('\n')
        );
        sb.uninstall().unwrap();
        // "\u00e9" is written back as the character itself: same meaning, not the same bytes
        // (a documented limit).
        assert_eq!(sb.text(), original.replace("caf\\u00e9", "caf\u{e9}"));
    }

    #[test]
    fn backups_are_made_and_pruned() {
        let sb = Sandbox::new();
        fs::write(sb.file(), FIXTURE).unwrap();
        for n in 0..13 {
            let moved = Spec {
                forwarder: format!(r"C:\v{n}\rain-hook.exe"),
                ..spec()
            };
            sb.install(&moved, &format!("20261121T0930{n:02}000Z"))
                .unwrap();
        }
        let mut names: Vec<_> = fs::read_dir(sb.backups())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names.len(),
            BACKUPS_KEPT,
            "only the newest backups are kept"
        );
        // The newest backup is the file as it was just before the last install: it holds v11.
        let newest = fs::read_to_string(sb.backups().join(names.last().unwrap())).unwrap();
        assert!(newest.contains(r"C:\\v11\\rain-hook.exe"));
        assert!(names[0].as_str() > "settings.json.20261121T093002000Z");
    }

    #[test]
    fn a_change_by_someone_else_before_the_rename_is_merged_not_overwritten() {
        let sb = Sandbox::new();
        fs::write(sb.file(), FIXTURE).unwrap();
        let path = sb.file();
        let mut hook = |attempt: usize| {
            if attempt == 0 {
                // Claude Code itself rewrites the file, for example when the user picks "always
                // allow".
                let text = fs::read_to_string(&path)
                    .unwrap()
                    .replace("\"theme\": \"dark\"", "\"theme\": \"light\"");
                fs::write(&path, text).unwrap();
            }
        };
        install_with_hook(&sb.file(), &sb.backups(), &spec(), STAMP, &mut hook).unwrap();
        let text = sb.text();
        assert!(
            text.contains("\"theme\": \"light\""),
            "the other program's change survived"
        );
        assert!(text.contains("--rain-managed=abc123"));
    }

    #[test]
    fn uninstall_removes_only_marked_entries_even_with_another_install_id() {
        let sb = Sandbox::new();
        fs::write(sb.file(), FIXTURE).unwrap();
        sb.install(
            &Spec {
                install_id: "old-id".into(),
                ..spec()
            },
            STAMP,
        )
        .unwrap();
        sb.uninstall().unwrap();
        assert_eq!(sb.text(), FIXTURE);
    }

    #[test]
    fn a_user_handler_in_the_same_group_as_a_rain_handler_survives() {
        let sb = Sandbox::new();
        let text = r#"{"hooks":{"Stop":[{"matcher":"","hooks":[{"type":"command","command":"mine.exe"},{"type":"command","command":"x.exe","args":["event","--rain-managed=zz"],"async":true}]}]}}"#;
        fs::write(sb.file(), text).unwrap();
        sb.uninstall().unwrap();
        let (value, _) = parse(&fs::read(sb.file()).unwrap()).unwrap();
        assert_eq!(
            value["hooks"]["Stop"][0]["hooks"].as_array().unwrap().len(),
            1
        );
        assert_eq!(value["hooks"]["Stop"][0]["hooks"][0]["command"], "mine.exe");
    }

    #[test]
    fn a_handler_that_runs_the_hook_program_counts_as_rain_s_without_the_marker() {
        let handler = json!({ "type": "command", "command": r"D:\anywhere\RAIN-HOOK.EXE", "args": ["event"] });
        assert!(is_rain_handler(&handler));
        assert!(!is_rain_handler(
            &json!({ "type": "command", "command": "mine.exe", "args": ["--rain"] })
        ));
        assert!(!is_rain_handler(
            &json!({ "type": "http", "args": ["--rain-managed=x"] })
        ));
    }

    #[test]
    fn the_fragment_has_12_events_with_the_right_modes() {
        let handlers = rain_handlers(&spec());
        assert_eq!(handlers.len(), EVENTS_EXPECTED);
        let mut names: Vec<_> = handlers.iter().map(|(event, _)| *event).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            12,
            "one handler for each event, none for Notification"
        );
        assert!(!names.contains(&"Notification"));
        for (event, handler) in &handlers {
            let is_async = handler.get("async") == Some(&json!(true));
            assert_eq!(is_async, OBSERVE_ASYNC.contains(event), "{event}");
        }
    }

    #[test]
    fn the_permission_handler_has_a_timeout_of_35_seconds_and_is_synchronous() {
        let handlers = rain_handlers(&spec());
        let (_, permission) = handlers
            .iter()
            .find(|(event, _)| *event == PERMISSION_EVENT)
            .unwrap();
        assert_eq!(permission["timeout"], 35);
        assert!(permission.get("async").is_none());
        assert_eq!(permission["args"][0], "permission");
        let (_, end) = handlers
            .iter()
            .find(|(event, _)| *event == OBSERVE_SYNC)
            .unwrap();
        assert!(end.get("async").is_none(), "SessionEnd stays synchronous");
    }

    #[test]
    fn the_pipe_argument_is_written_only_for_a_sandbox_backend() {
        let with_pipe = Spec {
            pipe: Some("rain-test-1".into()),
            ..spec()
        };
        let handlers = rain_handlers(&with_pipe);
        assert_eq!(
            handlers[0].1["args"],
            json!(["event", "--pipe=rain-test-1", "--rain-managed=abc123"])
        );
        assert_eq!(
            rain_handlers(&spec())[0].1["args"],
            json!(["event", "--rain-managed=abc123"])
        );
    }

    #[test]
    fn an_existing_install_id_is_found_for_a_repair() {
        let sb = Sandbox::new();
        fs::write(sb.file(), FIXTURE).unwrap();
        assert_eq!(
            existing_install_id(&parse(FIXTURE.as_bytes()).unwrap().0),
            None
        );
        sb.install(&spec(), STAMP).unwrap();
        let (value, _) = parse(sb.text().as_bytes()).unwrap();
        assert_eq!(existing_install_id(&value).as_deref(), Some("abc123"));
        assert_eq!(new_install_id(1_790_000_000_000).len(), 6);
    }

    #[test]
    fn the_sandbox_rule_allows_only_throwaway_places() {
        let temp = tempfile::tempdir().unwrap();
        let inside = temp.path().join("claude").join("settings.json");
        let safe = [temp.path()];
        assert!(
            allowed(&inside, true, &safe),
            "a file that does not exist yet, inside a safe folder"
        );
        let elsewhere = tempfile::tempdir().unwrap();
        assert!(!allowed(
            &elsewhere.path().join("settings.json"),
            true,
            &safe
        ));
        assert!(
            allowed(&elsewhere.path().join("settings.json"), false, &safe),
            "a real run may"
        );
        // `..` cannot leave the safe folder unnoticed.
        let sneaky = temp
            .path()
            .join("claude")
            .join("..")
            .join("..")
            .join("x")
            .join("settings.json");
        assert!(!allowed(&sneaky, true, &safe));
        // Case does not matter on Windows.
        let upper = PathBuf::from(temp.path().to_string_lossy().to_uppercase()).join("a.json");
        assert!(allowed(&upper, true, &safe));
    }

    #[test]
    fn status_follows_the_table_of_the_section() {
        let sb = Sandbox::new();
        let found = |on, forwarder_ok| status(&sb.file(), on, forwarder_ok, true);
        // No file: off is fine, on is broken.
        assert_eq!(found(false, true).state, HookState::Off);
        assert_eq!(found(true, true).state, HookState::Broken);
        // Installed.
        fs::write(sb.file(), FIXTURE).unwrap();
        sb.install(&spec(), STAMP).unwrap();
        let ok = found(true, true);
        assert_eq!((ok.state, ok.events_installed), (HookState::Ok, 12));
        assert_eq!(
            found(true, false).state,
            HookState::Broken,
            "the program is out of date"
        );
        // Switch off with entries present: off, nothing changed.
        let off = found(false, true);
        assert_eq!((off.state, off.events_installed), (HookState::Off, 12));
        // One entry deleted by hand.
        let mut value = parse(sb.text().as_bytes()).unwrap().0;
        value["hooks"].as_object_mut().unwrap().shift_remove("Stop");
        fs::write(sb.file(), serde_json::to_string_pretty(&value).unwrap()).unwrap();
        let broken = found(true, true);
        assert_eq!(
            (broken.state, broken.events_installed),
            (HookState::Broken, 11)
        );
        assert!(broken.message.unwrap().contains("missing"));
        // Hooks switched off by the user, broken JSON, no Claude folder.
        fs::write(sb.file(), r#"{"disableAllHooks": true}"#).unwrap();
        assert_eq!(found(true, true).state, HookState::Blocked);
        fs::write(sb.file(), "{ nope").unwrap();
        assert_eq!(found(true, true).state, HookState::Blocked);
        assert_eq!(found(false, true).state, HookState::Off);
        assert_eq!(
            status(&sb.file(), true, true, false).state,
            HookState::Unavailable
        );
        assert_eq!(status(&sb.file(), false, true, false).state, HookState::Off);
    }

    #[test]
    fn a_stamp_sorts_by_time() {
        assert_eq!(stamp_of("2026-11-21T09:30:00.123Z"), "20261121T093000123Z");
    }
}
