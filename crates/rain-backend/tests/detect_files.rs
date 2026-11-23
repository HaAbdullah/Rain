//! The files that detection reads, as made-up fixtures. Section 06 6.1.2 to 6.1.4, 10.3.

use std::path::{Path, PathBuf};

use rain_backend::modules::settings::{epic_folder, parse_daily_notes_json, parse_obsidian_json};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("detect")
        .join(name);
    std::fs::read_to_string(path).expect("a detect fixture")
}

#[test]
fn one_vault_gives_its_key_and_a_normalised_path() {
    let vaults = parse_obsidian_json(&fixture("obsidian-one.json"));
    assert_eq!(vaults.len(), 1);
    assert_eq!(vaults[0].id, "4f2a9c1b7d03e856");
    assert_eq!(vaults[0].path, Path::new(r"C:\Fake Notes\Main"));
}

#[test]
fn two_vaults_give_two_entries_and_the_caller_decides_that_there_is_no_default() {
    let vaults = parse_obsidian_json(&fixture("obsidian-two.json"));
    assert_eq!(vaults.len(), 2);
}

#[test]
fn a_file_that_is_not_json_gives_no_vault() {
    assert!(parse_obsidian_json(&fixture("obsidian-bad.json")).is_empty());
}

#[test]
fn daily_notes_with_only_a_folder_is_what_the_owners_pc_has() {
    let notes = parse_daily_notes_json(&fixture("daily-notes-folder-only.json"));
    assert_eq!(notes.folder.as_deref(), Some("Dailys"));
    assert_eq!(notes.format, None);
}

#[test]
fn daily_notes_with_both_keys_ignores_the_template() {
    let notes = parse_daily_notes_json(&fixture("daily-notes-both.json"));
    assert_eq!(notes.folder.as_deref(), Some("Journal/Days"));
    assert_eq!(notes.format.as_deref(), Some("YYYY-MM-DD"));
}

#[test]
fn the_epic_folder_is_the_launcher_data_folder_only_when_it_holds_the_manifests() {
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(epic_folder(temp.path()), None, "nothing installed");

    let launcher = temp.path().join("Epic").join("EpicGamesLauncher");
    std::fs::create_dir_all(launcher.join("Data")).unwrap();
    assert_eq!(
        epic_folder(temp.path()),
        None,
        "the launcher without manifests"
    );

    std::fs::create_dir_all(launcher.join("Data").join("Manifests")).unwrap();
    assert_eq!(epic_folder(temp.path()), Some(launcher));
}
