//! The logic of the library: the list, the text of one file, and which program opens a file.
//! Everything here is blocking and read only; the routes call it through `blocking::run`.
//!
//! There is no cache. Each call scans the few known folders again, so the answer is always what
//! the disk holds now, and nothing is read until a route is called. A file can be opened only
//! when the scan lists it: the id of a file is a hash of its path, and the text route looks the id
//! up in a fresh scan, so a route can never be made to read some other path.

use std::cmp::Ordering;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use rain_types::{ClaudeLibrary, LibraryFact, LibraryItem, LibraryKind, LibraryText};

use super::frontmatter::{self, first_line, shorten};
use super::scan::{self, Candidate, KnownProject};
use super::setup::{self, read_head};
use crate::modules::sessions::claude_paths::ClaudePaths;

/// How much of a file the text route returns.
pub const TEXT_MAX_BYTES: usize = 512 * 1024;
/// How much of a file the list reads for the front matter.
const HEAD_BYTES: usize = 16 * 1024;
const DESCRIPTION_MAX_CHARS: usize = 300;
const NAME_MAX_CHARS: usize = 100;

/// What a scan works from.
pub struct Inputs {
    pub paths: ClaudePaths,
    pub projects: Vec<KnownProject>,
    /// The name of the user's profile folder, to shorten the names of project folders.
    pub home_folder_name: String,
}

/// The id of a file: a hash of its path, so it is the same on every scan.
pub fn id_of(path: &Path) -> String {
    let text = path.to_string_lossy().to_lowercase().replace('\\', "/");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("cl-{hash:016x}")
}

pub fn index(inputs: &Inputs) -> ClaudeLibrary {
    if !inputs.paths.exists() && inputs.projects.is_empty() {
        return ClaudeLibrary {
            found: false,
            items: Vec::new(),
            setup: Default::default(),
            truncated: false,
        };
    }
    let plugins = setup::plugin_roots(&inputs.paths);
    let collected = scan::collect(
        &inputs.paths,
        &plugins,
        &inputs.projects,
        &inputs.home_folder_name,
    );
    let mut items: Vec<LibraryItem> = collected.candidates.iter().map(describe).collect();
    items.sort_by(order);
    ClaudeLibrary {
        found: inputs.paths.exists(),
        items,
        setup: setup::read_setup(&inputs.paths, &plugins, &inputs.projects),
        truncated: collected.truncated,
    }
}

/// You first, then projects, then plugins; then by scope, kind and name.
fn order(a: &LibraryItem, b: &LibraryItem) -> Ordering {
    fn rank(scope: &str) -> u8 {
        if scope == "You" {
            0
        } else if scope.starts_with("Project ") {
            1
        } else {
            2
        }
    }
    fn kind_rank(kind: LibraryKind) -> u8 {
        match kind {
            LibraryKind::Instruction => 0,
            LibraryKind::Skill => 1,
            LibraryKind::Agent => 2,
            LibraryKind::Command => 3,
            LibraryKind::Memory => 4,
        }
    }
    rank(&a.scope)
        .cmp(&rank(&b.scope))
        .then_with(|| a.scope.to_lowercase().cmp(&b.scope.to_lowercase()))
        .then_with(|| kind_rank(a.kind).cmp(&kind_rank(b.kind)))
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.path.cmp(&b.path))
}

fn describe(candidate: &Candidate) -> LibraryItem {
    let meta = fs::metadata(&candidate.path).ok();
    let parsed = frontmatter::parse(&read_head(&candidate.path, HEAD_BYTES));
    let name = parsed
        .fact("name")
        .map(|name| shorten(name, NAME_MAX_CHARS))
        .unwrap_or_else(|| candidate.fallback_name.clone());
    let description = match parsed.fact("description") {
        Some(text) => shorten(text, DESCRIPTION_MAX_CHARS),
        None if candidate.kind == LibraryKind::Memory && candidate.fallback_name == "MEMORY" => {
            "The index of this project's memory".to_owned()
        }
        None => first_line(&parsed.body),
    };
    let memory_type = if candidate.kind == LibraryKind::Memory {
        parsed
            .fact("metadata.type")
            .or_else(|| parsed.fact("type"))
            .map(|kind| shorten(kind, 30))
    } else {
        None
    };
    LibraryItem {
        id: id_of(&candidate.path),
        kind: candidate.kind,
        name,
        description,
        scope: candidate.scope.clone(),
        path: candidate.path.to_string_lossy().into_owned(),
        modified_at: meta
            .as_ref()
            .and_then(|meta| meta.modified().ok())
            .map(rain_types::time::iso_from_system_time),
        size: meta.map_or(0, |meta| meta.len()),
        memory_type,
    }
}

/// The path of a listed file. `None` when the scan does not list the id.
pub fn path_of(inputs: &Inputs, id: &str) -> Option<PathBuf> {
    let plugins = setup::plugin_roots(&inputs.paths);
    scan::collect(
        &inputs.paths,
        &plugins,
        &inputs.projects,
        &inputs.home_folder_name,
    )
    .candidates
    .into_iter()
    .find(|candidate| id_of(&candidate.path) == id)
    .map(|candidate| candidate.path)
}

/// Why a text could not be given.
#[derive(Debug, PartialEq, Eq)]
pub enum TextError {
    /// The scan does not list this id.
    NotListed,
    /// The file is listed but cannot be read now.
    Unreadable,
}

pub fn text(inputs: &Inputs, id: &str) -> Result<LibraryText, TextError> {
    let path = path_of(inputs, id).ok_or(TextError::NotListed)?;
    let (bytes, truncated) = read_capped(&path).map_err(|_| TextError::Unreadable)?;
    let parsed = frontmatter::parse(&String::from_utf8_lossy(&bytes));
    Ok(LibraryText {
        id: id.to_owned(),
        facts: parsed
            .facts
            .into_iter()
            .map(|(key, value)| LibraryFact { key, value })
            .collect(),
        body: parsed.body,
        truncated,
    })
}

/// At most `TEXT_MAX_BYTES` of the file, and whether there was more.
fn read_capped(path: &Path) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(TEXT_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > TEXT_MAX_BYTES;
    bytes.truncate(TEXT_MAX_BYTES);
    Ok((bytes, truncated))
}

/// The argument text for the editor: the path in quotes (a Windows path holds no quote).
pub fn editor_args(path: &Path) -> String {
    format!("\"{}\"", path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn inputs(root: &Path) -> Inputs {
        Inputs {
            paths: ClaudePaths::with_root(root.to_path_buf()),
            projects: Vec::new(),
            home_folder_name: "me".into(),
        }
    }

    #[test]
    fn the_id_is_stable_and_differs_by_path() {
        let a = id_of(Path::new(r"C:\x\a.md"));
        assert_eq!(
            a,
            id_of(Path::new("c:/x/A.md")),
            "case and slashes do not count"
        );
        assert_ne!(a, id_of(Path::new(r"C:\x\b.md")));
        assert!(a.starts_with("cl-"));
    }

    #[test]
    fn the_list_reads_name_description_and_memory_type() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("claude");
        write(
            &root.join("skills/alpha/SKILL.md"),
            "---\nname: alpha-skill\ndescription: Does the alpha thing\n---\nbody",
        );
        write(
            &root.join("agents/plain.md"),
            "# Plain agent\nno front matter",
        );
        write(
            &root.join("projects/C--work-shop/memory/pref.md"),
            "---\nname: pref\ndescription: a preference\nmetadata:\n  type: feedback\n---\nx",
        );
        write(
            &root.join("projects/C--work-shop/memory/MEMORY.md"),
            "- [pref](pref.md)",
        );
        let library = index(&inputs(&root));
        assert!(library.found && !library.truncated);
        let find = |name: &str| library.items.iter().find(|i| i.name == name).unwrap();
        assert_eq!(find("alpha-skill").description, "Does the alpha thing");
        assert_eq!(find("alpha-skill").scope, "You");
        assert_eq!(find("plain").description, "Plain agent");
        assert_eq!(find("pref").memory_type.as_deref(), Some("feedback"));
        assert_eq!(find("pref").scope, "Project work-shop");
        assert_eq!(
            find("MEMORY").description,
            "The index of this project's memory"
        );
        assert!(find("pref").modified_at.is_some());
        // You first, then projects.
        assert_eq!(library.items[0].scope, "You");
    }

    #[test]
    fn the_text_of_a_listed_file_comes_back_as_facts_and_body() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("claude");
        write(
            &root.join("agents/a.md"),
            "---\nname: a\nmodel: opus\n---\n# Hi",
        );
        let inputs = inputs(&root);
        let item = index(&inputs).items.remove(0);
        let text = text(&inputs, &item.id).unwrap();
        assert_eq!(text.body, "# Hi");
        assert_eq!(text.facts.len(), 2);
        assert!(!text.truncated);
    }

    #[test]
    fn an_id_that_the_scan_does_not_list_reads_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("claude");
        write(&root.join("agents/a.md"), "x");
        // A file next to the library that is not one of its files.
        write(&root.join("credentials-like.json"), "SECRET");
        let inputs = inputs(&root);
        let other = id_of(&root.join("credentials-like.json"));
        assert_eq!(text(&inputs, &other), Err(TextError::NotListed));
        assert_eq!(text(&inputs, "../../etc/passwd"), Err(TextError::NotListed));
    }

    #[test]
    fn a_long_file_is_cut_at_the_limit() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("claude");
        write(
            &root.join("agents/big.md"),
            &"x".repeat(TEXT_MAX_BYTES + 100),
        );
        let inputs = inputs(&root);
        let item = index(&inputs).items.remove(0);
        let text = text(&inputs, &item.id).unwrap();
        assert!(text.truncated);
        assert_eq!(text.body.len(), TEXT_MAX_BYTES);
    }

    #[test]
    fn a_missing_claude_folder_is_reported_as_not_found() {
        let library = index(&inputs(Path::new("no-such-claude-folder-for-tests")));
        assert!(!library.found);
        assert!(library.items.is_empty());
    }

    #[test]
    fn the_editor_gets_the_path_in_quotes() {
        assert_eq!(editor_args(Path::new(r"C:\a b\c.md")), "\"C:\\a b\\c.md\"");
    }
}
