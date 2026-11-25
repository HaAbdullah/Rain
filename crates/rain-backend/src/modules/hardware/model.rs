//! `hardware.toml`: the owner's own rules for the tiles. Rain only reads this file.
//!
//! A rule can rename a tile, hide it, or say what it opens. It names the tile by its id
//! (`mouse`, `keyboard-2`, `disk`) or by its kind (`mouse` applies to every mouse).

use serde::Deserialize;

pub const FILE_NAME: &str = "hardware.toml";

const MAX_NAME: usize = 60;
const MAX_OPENS: usize = 500;

/// The text that Rain writes when `hardware.toml` does not exist yet. Only comments except for
/// the version, so every default applies.
pub const EXAMPLE_FILE: &str = r#"schema_version = 1

# The Hardware screen finds your disks, keyboard and mouse by itself. This file changes what a
# tile is called, hides a tile, or sets what Enter opens on it. Nothing here is required.
# Rain only reads this file. Edit it with any editor; Rain notices the change.

# A tile is named by its id ("disk", "keyboard", "mouse", "mouse-2") or by its kind.
# "opens" is a program path, a web address, "disk" (the Disk screen) or "none".

# [[tile]]
# id     = "mouse"
# name   = "Desk mouse"
# opens  = "C:\\Program Files\\LGHUB\\lghub.exe"

# [[tile]]
# id     = "keyboard"
# opens  = "https://usevia.app"

# [[tile]]
# id     = "disk"
# hidden = true
"#;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TileRule {
    pub id: String,
    pub name: Option<String>,
    pub hidden: Option<bool>,
    pub opens: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HardwareFile {
    pub tile: Vec<TileRule>,
}

/// The rules that the types cannot state. A file that breaks one keeps its last good copy.
pub fn validate(file: &HardwareFile) -> Result<(), String> {
    for (number, rule) in file.tile.iter().enumerate() {
        let place = number + 1;
        if rule.id.trim().is_empty() {
            return Err(format!("[[tile]] number {place} has no id"));
        }
        if let Some(name) = &rule.name
            && (name.trim().is_empty() || name.chars().count() > MAX_NAME)
        {
            return Err(format!(
                "[[tile]] \"{}\": name must be 1 to {MAX_NAME} characters",
                rule.id
            ));
        }
        if rule
            .opens
            .as_ref()
            .is_some_and(|opens| opens.chars().count() > MAX_OPENS)
        {
            return Err(format!("[[tile]] \"{}\": opens is too long", rule.id));
        }
    }
    Ok(())
}

/// What a rule says a tile opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpensRule {
    Disk,
    Nothing,
    Url(String),
    Program(String),
}

pub fn parse_opens(text: &str) -> OpensRule {
    let text = text.trim();
    let lower = text.to_lowercase();
    if lower == "disk" {
        OpensRule::Disk
    } else if lower.is_empty() || lower == "none" {
        OpensRule::Nothing
    } else if lower.starts_with("https://") || lower.starts_with("http://") {
        OpensRule::Url(text.to_owned())
    } else {
        OpensRule::Program(text.to_owned())
    }
}

impl HardwareFile {
    /// The setting `pick` finds in the rules for a tile; the last rule that sets it wins.
    pub fn setting<T>(
        &self,
        id: &str,
        kind: &str,
        pick: impl Fn(&TileRule) -> Option<T>,
    ) -> Option<T> {
        let mut found = None;
        let kind_rules = self.tile.iter().filter(|rule| {
            rule.id.trim().eq_ignore_ascii_case(kind) && !kind.eq_ignore_ascii_case(id)
        });
        let id_rules = self
            .tile
            .iter()
            .filter(|rule| rule.id.trim().eq_ignore_ascii_case(id));
        for rule in kind_rules.chain(id_rules) {
            if let Some(value) = pick(rule) {
                found = Some(value);
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> HardwareFile {
        toml::from_str(text).expect("valid toml")
    }

    #[test]
    fn the_example_file_is_valid_and_sets_nothing() {
        let file = read(EXAMPLE_FILE);
        assert!(file.tile.is_empty());
        assert!(validate(&file).is_ok());
    }

    #[test]
    fn rules_are_read_from_tile_tables() {
        let file = read(
            "[[tile]]\nid = \"mouse\"\nname = \"Desk mouse\"\nopens = \"none\"\n\n\
             [[tile]]\nid = \"disk\"\nhidden = true\n",
        );
        assert_eq!(file.tile.len(), 2);
        assert_eq!(file.tile[0].name.as_deref(), Some("Desk mouse"));
        assert_eq!(file.tile[1].hidden, Some(true));
        assert!(validate(&file).is_ok());
    }

    #[test]
    fn a_rule_without_an_id_or_with_a_bad_name_is_refused() {
        assert!(validate(&read("[[tile]]\nname = \"x\"\n")).is_err());
        assert!(validate(&read("[[tile]]\nid = \"mouse\"\nname = \"  \"\n")).is_err());
        let long = format!("[[tile]]\nid = \"mouse\"\nname = \"{}\"\n", "x".repeat(61));
        assert!(validate(&read(&long)).is_err());
    }

    #[test]
    fn opens_is_a_program_a_web_address_disk_or_nothing() {
        assert_eq!(parse_opens("disk"), OpensRule::Disk);
        assert_eq!(parse_opens(" Disk "), OpensRule::Disk);
        assert_eq!(parse_opens("none"), OpensRule::Nothing);
        assert_eq!(parse_opens(""), OpensRule::Nothing);
        assert_eq!(
            parse_opens("https://usevia.app"),
            OpensRule::Url("https://usevia.app".to_owned())
        );
        assert_eq!(
            parse_opens(r"C:\Apps\x.exe"),
            OpensRule::Program(r"C:\Apps\x.exe".to_owned())
        );
    }

    #[test]
    fn a_rule_by_id_overrides_a_rule_by_kind() {
        let file = read(
            "[[tile]]\nid = \"mouse\"\nname = \"Any mouse\"\nhidden = true\n\n\
             [[tile]]\nid = \"mouse-2\"\nname = \"Travel mouse\"\n",
        );
        let name = |id: &str| file.setting(id, "mouse", |rule| rule.name.clone());
        assert_eq!(name("mouse").as_deref(), Some("Any mouse"));
        assert_eq!(name("mouse-2").as_deref(), Some("Travel mouse"));
        assert_eq!(
            file.setting("mouse-2", "mouse", |rule| rule.hidden),
            Some(true)
        );
        assert_eq!(
            file.setting("keyboard", "keyboard", |rule| rule.hidden),
            None
        );
    }

    #[test]
    fn ids_match_without_regard_to_case() {
        let file = read("[[tile]]\nid = \"Mouse\"\nhidden = true\n");
        assert_eq!(
            file.setting("mouse", "mouse", |rule| rule.hidden),
            Some(true)
        );
    }
}
