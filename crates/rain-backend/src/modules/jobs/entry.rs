//! Reading one `[[job]]` or `[[idle_job]]` table of the user's file. Section 03 4.1.1 and 4.2.1.
//!
//! The files are edited by hand, so every key may be missing or have the wrong type. The typed
//! file struct would reject the whole file for one wrong key. Instead each entry stays a plain
//! TOML table, and these helpers read one key at a time and say in plain words what is wrong, so
//! that one bad entry is skipped and the others still work.
//!
//! Idle jobs share this code (the keys `id`, `name`, `enabled`, `command`, `args`, `shell`,
//! `working_dir`, `env` and `timeout_minutes` are the same).

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use toml::{Table, Value};

/// How long an id may be, and which characters it may hold (foundation 4.3).
const ID_MAX: usize = 128;
/// How much of a name an id derived from it keeps.
const DERIVED_ID_MAX: usize = 60;

/// The text of a key, or what is wrong with it. Errors are sentences for the Health report.
pub type KeyResult<T> = Result<T, String>;

/// One table of the file.
pub struct Entry<'a>(pub &'a Table);

impl Entry<'_> {
    pub fn string(&self, key: &str) -> KeyResult<Option<String>> {
        match self.0.get(key) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(format!("`{key}` must be text")),
        }
    }

    pub fn bool(&self, key: &str) -> KeyResult<Option<bool>> {
        match self.0.get(key) {
            None => Ok(None),
            Some(Value::Boolean(value)) => Ok(Some(*value)),
            Some(_) => Err(format!("`{key}` must be true or false")),
        }
    }

    /// An integer between `min` and `max` (both allowed).
    pub fn int(&self, key: &str, min: i64, max: i64) -> KeyResult<Option<i64>> {
        match self.0.get(key) {
            None => Ok(None),
            Some(Value::Integer(value)) if (min..=max).contains(value) => Ok(Some(*value)),
            Some(_) => Err(format!(
                "`{key}` must be a whole number from {min} to {max}"
            )),
        }
    }

    pub fn strings(&self, key: &str) -> KeyResult<Vec<String>> {
        match self.0.get(key) {
            None => Ok(Vec::new()),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| match item {
                    Value::String(text) => Ok(text.clone()),
                    _ => Err(format!("`{key}` must be a list of text")),
                })
                .collect(),
            Some(_) => Err(format!("`{key}` must be a list of text")),
        }
    }

    /// The `env` table: extra environment variables for the run.
    pub fn env(&self) -> KeyResult<BTreeMap<String, String>> {
        match self.0.get("env") {
            None => Ok(BTreeMap::new()),
            Some(Value::Table(table)) => table
                .iter()
                .map(|(name, value)| match value {
                    Value::String(text) if valid_env_name(name) => Ok((name.clone(), text.clone())),
                    _ => Err("`env` must hold only text values with plain names".to_owned()),
                })
                .collect(),
            Some(_) => Err("`env` must be a table of text".to_owned()),
        }
    }

    /// `working_dir`: an absolute path, or `None` for the default folder.
    pub fn working_dir(&self) -> KeyResult<Option<PathBuf>> {
        let Some(text) = self.string("working_dir")? else {
            return Ok(None);
        };
        let path = PathBuf::from(&text);
        if text.trim().is_empty() || !path.is_absolute() {
            return Err("`working_dir` must be a full path such as D:\\folder".to_owned());
        }
        Ok(Some(path))
    }
}

fn valid_env_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('=') && !name.contains('\0')
}

// ---- the command of an entry ----

/// How a command line is run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// The program is started directly, with its arguments one by one.
    None,
    Cmd,
    Powershell,
}

/// The program of a `cron` job or an idle job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    pub command: String,
    pub args: Vec<String>,
    pub shell: Shell,
}

pub fn command_line(entry: &Entry) -> KeyResult<CommandLine> {
    let command = entry
        .string("command")?
        .filter(|text| !text.trim().is_empty())
        .ok_or("`command` is missing")?;
    let shell = match entry.string("shell")?.as_deref() {
        None | Some("none") => Shell::None,
        Some("cmd") => Shell::Cmd,
        Some("powershell") => Shell::Powershell,
        Some(_) => return Err("`shell` must be \"none\", \"cmd\" or \"powershell\"".to_owned()),
    };
    let args = entry.strings("args")?;
    if shell != Shell::None && !args.is_empty() {
        return Err("`args` can only be used with shell = \"none\"".to_owned());
    }
    Ok(CommandLine {
        command,
        args,
        shell,
    })
}

// ---- ids ----

/// An id as foundation 4.3 allows it: `A-Za-z0-9._~-`, 1 to 128 characters.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ID_MAX
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '-'))
}

/// `prefix` plus the lower-case name with every run of other characters turned into one `-`.
fn derived_base(prefix: &str, name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let mut id = format!("{prefix}{slug}");
    id.truncate(DERIVED_ID_MAX);
    id.trim_end_matches('-').to_owned()
}

/// What the id of each entry is, in file order. `entries` holds the `id` and the `name` of every
/// entry as written (either may be missing). An entry with an `id` keeps it (a second entry
/// with the same id gets the same text again, and the caller skips it as a duplicate). An entry
/// without one gets `prefix` plus its name, and `-2`, `-3` when that is taken by an earlier entry.
///
/// Pure and used twice: when the file is read, and when `enabled` is written back, so that both
/// find the same entry for the same id.
pub fn derive_ids(prefix: &str, entries: &[(Option<String>, Option<String>)]) -> Vec<String> {
    let mut taken: HashSet<String> = HashSet::new();
    let mut ids = Vec::with_capacity(entries.len());
    for (index, (id, name)) in entries.iter().enumerate() {
        let id = match id {
            Some(id) => id.clone(),
            None => {
                let mut base = derived_base(prefix, name.as_deref().unwrap_or_default());
                if base.trim_end_matches('-') == prefix.trim_end_matches('-') {
                    base = format!("{}entry-{}", prefix, index + 1);
                }
                let mut candidate = base.clone();
                let mut number = 2;
                while taken.contains(&candidate) {
                    candidate = format!("{base}-{number}");
                    number += 1;
                }
                candidate
            }
        };
        taken.insert(id.clone());
        ids.push(id);
    }
    ids
}

/// The line number of each `[[table]]` header in `text`, in file order. Used to point an issue at
/// the entry that has it.
pub fn entry_lines(text: &str, table: &str) -> Vec<u32> {
    let header = format!("[[{table}]]");
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            let line = line.trim();
            line.strip_prefix(header.as_str())
                .is_some_and(|rest| rest.trim().is_empty() || rest.trim_start().starts_with('#'))
        })
        .map(|(index, _)| index as u32 + 1)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Table {
        text.parse().expect("a TOML table")
    }

    #[test]
    fn keys_of_the_wrong_type_are_named_in_plain_words() {
        let t = table("enabled = \"yes\"\nname = 3\nn = 99\nargs = [1]\nenv = { A = 1 }");
        let entry = Entry(&t);
        assert_eq!(
            entry.bool("enabled").unwrap_err(),
            "`enabled` must be true or false"
        );
        assert_eq!(entry.string("name").unwrap_err(), "`name` must be text");
        assert!(entry.int("n", 1, 10).unwrap_err().contains("from 1 to 10"));
        assert!(entry.strings("args").is_err());
        assert!(entry.env().is_err());
        assert_eq!(entry.string("missing").unwrap(), None);
    }

    #[test]
    fn a_working_dir_must_be_a_full_path() {
        assert!(
            Entry(&table("working_dir = \"relative\\\\x\""))
                .working_dir()
                .is_err()
        );
        assert!(
            Entry(&table("working_dir = \"C:\\\\x\""))
                .working_dir()
                .is_ok()
        );
        assert_eq!(Entry(&table("a = 1")).working_dir().unwrap(), None);
    }

    #[test]
    fn a_command_line_follows_the_rules_of_the_shell_key() {
        let ok = table("command = \"robocopy\"\nargs = [\"a\", \"b\"]");
        let line = command_line(&Entry(&ok)).unwrap();
        assert_eq!(line.shell, Shell::None);
        assert_eq!(line.args, ["a", "b"]);

        let cmd = table("command = \"echo hi\"\nshell = \"cmd\"");
        assert_eq!(command_line(&Entry(&cmd)).unwrap().shell, Shell::Cmd);

        assert!(command_line(&Entry(&table("a = 1"))).is_err());
        assert!(command_line(&Entry(&table("command = \"\""))).is_err());
        assert!(command_line(&Entry(&table("command = \"x\"\nshell = \"bash\""))).is_err());
        let mixed = table("command = \"x\"\nshell = \"cmd\"\nargs = [\"a\"]");
        assert!(command_line(&Entry(&mixed)).unwrap_err().contains("args"));
    }

    #[test]
    fn ids_allow_only_the_foundation_characters() {
        assert!(valid_id("job-1.a_b~c"));
        assert!(!valid_id(""));
        assert!(!valid_id("has space"));
        assert!(!valid_id("slash/x"));
        assert!(!valid_id(&"a".repeat(129)));
    }

    #[test]
    fn a_derived_id_is_the_slug_of_the_name_and_clashes_get_a_number() {
        let entries = vec![
            (None, Some("Back up notes!".to_owned())),
            (None, Some("back  UP notes".to_owned())),
            (Some("custom".to_owned()), Some("Whatever".to_owned())),
            (None, Some("Back up notes".to_owned())),
            (None, None),
        ];
        assert_eq!(
            derive_ids("job-", &entries),
            [
                "job-back-up-notes",
                "job-back-up-notes-2",
                "custom",
                "job-back-up-notes-3",
                "job-entry-5"
            ]
        );
    }

    #[test]
    fn derived_ids_do_not_change_when_a_later_entry_is_added() {
        let first = vec![(None, Some("A".to_owned())), (None, Some("B".to_owned()))];
        let mut longer = first.clone();
        longer.push((None, Some("A".to_owned())));
        assert_eq!(
            derive_ids("idle-", &first)[..],
            derive_ids("idle-", &longer)[..2]
        );
    }

    #[test]
    fn a_long_name_gives_a_short_id() {
        let ids = derive_ids("job-", &[(None, Some("x".repeat(200)))]);
        assert!(ids[0].len() <= 60);
    }

    #[test]
    fn the_lines_of_the_entries_are_found_with_their_comments() {
        let text = "schema_version = 1\n\n[[job]]\nid = \"a\"\n\n[[job]] # second\nid = \"b\"\n  [[job]]\n[[other]]\n";
        assert_eq!(entry_lines(text, "job"), [3, 6, 8]);
    }
}
