//! The definitions in `idle-jobs.toml`, checked entry by entry. Section 03 4.2.
//!
//! Same approach as `jobs/model.rs`: every `[[idle_job]]` stays a plain TOML table in the typed
//! file, and a bad entry is skipped and named while the others work.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use rain_types::IdleResource;
use serde::Deserialize;
use toml::Table;

use crate::modules::jobs::entry::{
    CommandLine, Entry, KeyResult, command_line, derive_ids, valid_id,
};

pub const FILE_NAME: &str = "idle-jobs.toml";
/// The name of the table array in the file.
pub const TABLE: &str = "idle_job";
/// The prefix of an id that is derived from a name.
pub const ID_PREFIX: &str = "idle-";

const NAME_MAX: usize = 80;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct IdleFile {
    pub idle_job: Vec<Table>,
}

/// When a finished job becomes `queued` again (section 03 6.12.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rerun {
    Daily,
    Weekly,
    EveryIdle,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleDef {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub resource: IdleResource,
    pub idle_after_minutes: u32,
    pub command: CommandLine,
    pub working_dir: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub timeout_minutes: u32,
    pub rerun: Rerun,
    pub min_gap_minutes: u32,
    pub expected_minutes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub index: usize,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct Loaded {
    pub defs: Vec<IdleDef>,
    pub skipped: Vec<Skipped>,
}

pub fn load(file: &IdleFile) -> Loaded {
    let raw: Vec<(Option<String>, Option<String>)> = file
        .idle_job
        .iter()
        .map(|table| (text_key(table, "id"), text_key(table, "name")))
        .collect();
    let ids = derive_ids(ID_PREFIX, &raw);

    let mut loaded = Loaded::default();
    let mut seen: HashSet<&str> = HashSet::new();
    for (index, (table, id)) in file.idle_job.iter().zip(&ids).enumerate() {
        let result = if !seen.insert(id.as_str()) {
            Err(format!("another idle job already has the id \"{id}\""))
        } else {
            parse_entry(&Entry(table), id)
        };
        match result {
            Ok(def) => loaded.defs.push(def),
            Err(message) => {
                let who = match text_key(table, "name").filter(|n| !n.trim().is_empty()) {
                    Some(name) => format!("Idle job \"{name}\""),
                    None => format!("Idle job {}", index + 1),
                };
                loaded.skipped.push(Skipped {
                    index,
                    message: format!("{who}: {message}"),
                });
            }
        }
    }
    loaded
}

fn text_key(table: &Table, key: &str) -> Option<String> {
    table.get(key).and_then(|v| v.as_str()).map(str::to_owned)
}

fn parse_entry(entry: &Entry, id: &str) -> KeyResult<IdleDef> {
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
    let resource = match entry.string("resource")?.as_deref() {
        None | Some("cpu") => IdleResource::Cpu,
        Some("gpu") => IdleResource::Gpu,
        Some(_) => return Err("`resource` must be \"cpu\" or \"gpu\"".to_owned()),
    };
    let rerun = match entry.string("rerun")?.as_deref() {
        None | Some("daily") => Rerun::Daily,
        Some("weekly") => Rerun::Weekly,
        Some("every_idle") => Rerun::EveryIdle,
        Some("manual") => Rerun::Manual,
        Some(_) => {
            return Err(
                "`rerun` must be \"daily\", \"weekly\", \"every_idle\" or \"manual\"".to_owned(),
            );
        }
    };
    Ok(IdleDef {
        id: id.to_owned(),
        name,
        enabled: entry.bool("enabled")?.unwrap_or(false),
        resource,
        idle_after_minutes: entry
            .int("idle_after_minutes", 1, 1440)?
            .map_or(10, |n| n as u32),
        command: command_line(entry)?,
        working_dir: entry.working_dir()?,
        env: entry.env()?,
        timeout_minutes: entry
            .int("timeout_minutes", 1, 1440)?
            .map_or(240, |n| n as u32),
        rerun,
        min_gap_minutes: entry
            .int("min_gap_minutes", 0, 10_080)?
            .map_or(60, |n| n as u32),
        expected_minutes: entry.int("expected_minutes", 1, 10_080)?.map(|n| n as u32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(text: &str) -> IdleFile {
        toml::from_str(text).expect("a readable file")
    }

    #[test]
    fn a_good_entry_gets_the_documented_defaults() {
        let loaded = load(&file(
            "[[idle_job]]\nname = \"Rebuild thumbs\"\ncommand = \"thumbs.exe\"\n",
        ));
        assert!(loaded.skipped.is_empty());
        let job = &loaded.defs[0];
        assert_eq!(job.id, "idle-rebuild-thumbs");
        assert!(!job.enabled);
        assert_eq!(job.resource, IdleResource::Cpu);
        assert_eq!(job.idle_after_minutes, 10);
        assert_eq!(job.timeout_minutes, 240);
        assert_eq!(job.rerun, Rerun::Daily);
        assert_eq!(job.min_gap_minutes, 60);
        assert_eq!(job.expected_minutes, None);
    }

    #[test]
    fn every_documented_key_is_read() {
        let loaded = load(&file(
            "[[idle_job]]\nid = \"a\"\nname = \"A\"\nenabled = true\nresource = \"gpu\"\n\
             idle_after_minutes = 3\ncommand = \"x\"\nargs = [\"1\"]\nrerun = \"every_idle\"\n\
             min_gap_minutes = 5\nexpected_minutes = 30\ntimeout_minutes = 9\n",
        ));
        let job = &loaded.defs[0];
        assert!(job.enabled);
        assert_eq!(job.resource, IdleResource::Gpu);
        assert_eq!(job.idle_after_minutes, 3);
        assert_eq!(job.rerun, Rerun::EveryIdle);
        assert_eq!(job.expected_minutes, Some(30));
        assert_eq!(job.command.args, ["1"]);
    }

    #[test]
    fn bad_entries_are_skipped_with_a_reason_and_the_rest_work() {
        let loaded = load(&file(
            "[[idle_job]]\nname = \"No command\"\n\
             [[idle_job]]\nname = \"Bad resource\"\ncommand = \"x\"\nresource = \"tpu\"\n\
             [[idle_job]]\nname = \"Bad rerun\"\ncommand = \"x\"\nrerun = \"often\"\n\
             [[idle_job]]\nname = \"Bad shell\"\ncommand = \"x\"\nshell = \"bash\"\n\
             [[idle_job]]\nname = \"Mixed\"\ncommand = \"x\"\nshell = \"cmd\"\nargs = [\"a\"]\n\
             [[idle_job]]\nname = \"Too long\"\ncommand = \"x\"\nidle_after_minutes = 5000\n\
             [[idle_job]]\ncommand = \"x\"\n\
             [[idle_job]]\nname = \"Good\"\ncommand = \"x\"\n",
        ));
        assert_eq!(loaded.defs.len(), 1);
        assert_eq!(loaded.defs[0].name, "Good");
        let messages: Vec<_> = loaded.skipped.iter().map(|s| s.message.as_str()).collect();
        assert!(messages[0].contains("`command` is missing"));
        assert!(messages[1].contains("`resource` must be"));
        assert!(messages[2].contains("`rerun` must be"));
        assert!(messages[3].contains("`shell` must be"));
        assert!(messages[4].contains("`args` can only be used"));
        assert!(messages[5].contains("`idle_after_minutes`"));
        assert!(messages[6].contains("no `name`"));
    }
}
