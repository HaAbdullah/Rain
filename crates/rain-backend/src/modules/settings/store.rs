//! `settings.toml`: its schema, its rules, and how a change is written into it.
//! Section 06 4.1. Both the reading and the writing go through `TomlFile` (foundation 10.4).

use rain_types::{CountdownSeconds, ResetAfterSeconds};
use serde::Deserialize;
use toml_edit::{DocumentMut, Value, value};

use super::rules;
use crate::core::error::ApiResult;
use crate::store::hub::ConfigHub;
use crate::store::toml_file::{TomlFile, set_value};

const FILE_NAME: &str = "settings.toml";

/// The comment above the first key of a new file. It is written once, as the decor of
/// `schema_version`, so it is never removed by an edit.
const HEADER: &str = "\
# Rain settings.
# Rain writes only the settings you change in the app. You may edit this file by hand; Rain reloads it.
# A key that is missing here uses the default shown in the comment.
#
#   hide_on_blur = true                   hide the window when it loses focus
#   reset_after_seconds = 60              back to Home after being hidden this long: 30, 60 or 300
#   vault_path = ''                       Obsidian vault folder; empty = the only vault Obsidian knows
#   daily_note_format = ''                empty = the vault's own setting, else YYYY-MM-DD
#   daily_note_folder = ''                folder inside the vault; empty = the vault's own setting
#   project_roots = []                    folders that contain your repositories
#   game_folders = []                     folders that contain installed games, one folder per game
#   steam_enabled = true
#   steam_path = ''                       empty = detected
#   epic_enabled = true
#   epic_path = ''                        empty = detected
#   riot_enabled = true
#   riot_path = ''                        empty = detected
#   game_mode_countdown_seconds = 10      5, 10, 20 or 30
#   monitor_claude = false                the switch \"Monitor Claude sessions\"
";

/// The file as Rain reads it. Every key is optional, and a key Rain does not know is ignored
/// (and survives an edit), because people edit this file by hand.
///
/// A key that is absent differs from one that is present only where the default is not the
/// empty value: those are `Option`.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct SettingsFile {
    pub hide_on_blur: Option<bool>,
    pub reset_after_seconds: Option<ResetAfterSeconds>,
    pub vault_path: String,
    pub daily_note_format: String,
    pub daily_note_folder: String,
    pub project_roots: Vec<String>,
    pub game_folders: Vec<String>,
    pub steam_enabled: Option<bool>,
    pub steam_path: String,
    pub epic_enabled: Option<bool>,
    pub epic_path: String,
    pub riot_enabled: Option<bool>,
    pub riot_path: String,
    pub game_mode_countdown_seconds: Option<CountdownSeconds>,
    /// The intent of the switch "Monitor Claude sessions"; the state is computed by section 03.
    pub monitor_claude: Option<bool>,
}

/// The numbers of the two unions are checked by their types, while the file is parsed. This
/// checks the rest, with the same rules as a `PATCH` (section 06, table V).
fn validate(file: &SettingsFile) -> Result<(), String> {
    let named = |key: &str, result: Result<(), rules::Reject>| {
        result.map_err(|reject| format!("{key}: {}", reject.message()))
    };
    named("vault_path", rules::path_text(&file.vault_path))?;
    named("steam_path", rules::path_text(&file.steam_path))?;
    named("epic_path", rules::path_text(&file.epic_path))?;
    named("riot_path", rules::path_text(&file.riot_path))?;
    named(
        "daily_note_format",
        rules::note_format(&file.daily_note_format),
    )?;
    named(
        "daily_note_folder",
        rules::note_folder(&file.daily_note_folder).map(|_| ()),
    )?;
    named("project_roots", rules::folder_list(&file.project_roots))?;
    named("game_folders", rules::folder_list(&file.game_folders))
}

pub async fn open(hub: &ConfigHub) -> ApiResult<TomlFile<SettingsFile>> {
    TomlFile::open(hub, FILE_NAME, validate).await
}

/// One key to set, or to remove (`None`: the key goes back to its default).
#[derive(Debug)]
pub struct KeyEdit {
    pub key: &'static str,
    pub value: Option<Value>,
}

/// Writes the edits into the document. Nothing else in the file changes. A new file also gets
/// its header and `schema_version` first.
pub fn apply_edits(document: &mut DocumentMut, edits: Vec<KeyEdit>) -> ApiResult<()> {
    let sets_a_key = edits.iter().any(|edit| edit.value.is_some());
    let blank = document.to_string().trim().is_empty();
    let only_comments = document.is_empty() && !blank;
    if sets_a_key && blank {
        write_header(document);
    }
    for edit in edits {
        match edit.value {
            Some(new) => set_value(document.as_table_mut(), edit.key, new),
            None => {
                document.remove(edit.key);
            }
        }
    }
    if only_comments {
        keep_comments_first(document);
    }
    Ok(())
}

/// `toml_edit` keeps the text after the last key as the end of the document. In a file that held
/// only comments that is all of it, so a key added now would land above the user's comments.
/// This makes the comments the decor of the first key instead.
fn keep_comments_first(document: &mut DocumentMut) {
    let comments = document.trailing().as_str().unwrap_or_default().to_owned();
    let Some(first) = document.iter().next().map(|(key, _)| key.to_owned()) else {
        return;
    };
    document.set_trailing("");
    if let Some(mut key) = document.key_mut(&first) {
        key.leaf_decor_mut().set_prefix(comments);
    }
}

fn write_header(document: &mut DocumentMut) {
    document.insert("schema_version", value(1));
    if let Some(mut key) = document.key_mut("schema_version") {
        key.leaf_decor_mut().set_prefix(HEADER);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> SettingsFile {
        toml::from_str(text).unwrap()
    }

    fn set(key: &'static str, new: Value) -> KeyEdit {
        KeyEdit {
            key,
            value: Some(new),
        }
    }

    fn remove(key: &'static str) -> KeyEdit {
        KeyEdit { key, value: None }
    }

    #[test]
    fn an_empty_file_means_every_default() {
        let file = parse("");
        assert_eq!(file.hide_on_blur, None);
        assert_eq!(file.reset_after_seconds, None);
        assert!(file.vault_path.is_empty() && file.project_roots.is_empty());
    }

    #[test]
    fn a_number_outside_a_union_is_an_error_that_says_where() {
        let text = "hide_on_blur = true\nreset_after_seconds = 45\n";
        let error = toml::from_str::<SettingsFile>(text).unwrap_err();
        let span = error.span().expect("the error says where");
        assert_eq!(&text[span], "45");
    }

    #[test]
    fn unknown_keys_are_ignored_and_the_known_ones_of_the_later_stages_are_read() {
        let file = parse(
            "future_key = 1\nmonitor_claude = true\ngame_folders = ['D:\\Games']\nhide_on_blur = false\n",
        );
        assert_eq!(file.hide_on_blur, Some(false));
        assert_eq!(file.monitor_claude, Some(true));
        assert_eq!(file.game_folders, ["D:\\Games"]);
    }

    #[test]
    fn the_rules_of_a_patch_apply_to_a_hand_edit_too() {
        let ok = parse("project_roots = ['D:\\Code']\n");
        assert!(validate(&ok).is_ok());

        let long = SettingsFile {
            steam_path: "x".repeat(2000),
            ..SettingsFile::default()
        };
        assert_eq!(
            validate(&long).unwrap_err(),
            "steam_path: That text is too long."
        );

        let bad_format = SettingsFile {
            daily_note_format: "a:b".into(),
            ..SettingsFile::default()
        };
        assert!(
            validate(&bad_format)
                .unwrap_err()
                .starts_with("daily_note_format")
        );

        let many = SettingsFile {
            project_roots: vec!["D:\\x".into(); 33],
            ..SettingsFile::default()
        };
        assert!(validate(&many).unwrap_err().starts_with("project_roots"));

        let many_games = SettingsFile {
            game_folders: vec!["D:\\x".into(); 33],
            ..SettingsFile::default()
        };
        assert!(
            validate(&many_games)
                .unwrap_err()
                .starts_with("game_folders")
        );

        let absolute_folder = SettingsFile {
            daily_note_folder: "C:\\Notes".into(),
            ..SettingsFile::default()
        };
        assert!(
            validate(&absolute_folder)
                .unwrap_err()
                .starts_with("daily_note_folder")
        );
    }

    #[test]
    fn a_new_file_gets_its_header_then_schema_version_then_the_keys() {
        let mut document = DocumentMut::new();
        apply_edits(
            &mut document,
            vec![set("reset_after_seconds", Value::from(300))],
        )
        .unwrap();

        let text = document.to_string();
        assert!(text.starts_with("# Rain settings.\n"), "{text}");
        assert!(
            text.ends_with(
                "\"Monitor Claude sessions\"\nschema_version = 1\nreset_after_seconds = 300\n"
            ),
            "{text}"
        );
        assert_eq!(
            parse(&text).reset_after_seconds,
            Some(ResetAfterSeconds::Seconds300)
        );
    }

    #[test]
    fn removing_a_key_of_a_missing_file_changes_nothing() {
        let mut document = DocumentMut::new();
        apply_edits(&mut document, vec![remove("vault_path")]).unwrap();
        assert_eq!(
            document.to_string(),
            "",
            "no header for a change that writes nothing"
        );
    }

    #[test]
    fn an_existing_file_keeps_everything_that_is_not_edited() {
        let before = "# mine\nschema_version = 1\nhide_on_blur = false  # because\n\n# my own note\nfuture_key = 1\nvault_path = 'D:\\Vault'\n";
        let mut document: DocumentMut = before.parse().unwrap();
        apply_edits(
            &mut document,
            vec![
                set("hide_on_blur", Value::from(true)),
                remove("vault_path"),
                set("steam_enabled", Value::from(false)),
            ],
        )
        .unwrap();

        assert_eq!(
            document.to_string(),
            "# mine\nschema_version = 1\nhide_on_blur = true  # because\n\n# my own note\nfuture_key = 1\nsteam_enabled = false\n"
        );
    }

    #[test]
    fn a_comment_directly_above_a_removed_key_goes_with_it() {
        let mut document: DocumentMut =
            "a_key = 1\n\n# about the vault\nvault_path = 'x'\nz_key = 2\n"
                .parse()
                .unwrap();
        apply_edits(&mut document, vec![remove("vault_path")]).unwrap();
        assert_eq!(document.to_string(), "a_key = 1\nz_key = 2\n");
    }

    #[test]
    fn a_file_that_only_has_comments_is_not_given_a_second_header() {
        let mut document: DocumentMut = "# my own notes\n".parse().unwrap();
        apply_edits(&mut document, vec![set("hide_on_blur", Value::from(false))]).unwrap();
        assert_eq!(
            document.to_string(),
            "# my own notes\nhide_on_blur = false\n"
        );
    }
}
