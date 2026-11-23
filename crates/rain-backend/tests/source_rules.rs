//! Rules about the source text of the backend (foundation sections 11.1, 13.3 and 13.4).
//!
//! The scan is a plain text scan. It is crude on purpose: it is cheap, and a false alarm is
//! fixed by moving the code. Most rules have nothing to find yet (there is no vault code and no
//! Claude hooks installer in stage 1a); they are here so that the code that comes later cannot
//! break them unnoticed. Every rule is also tried on a small example that breaks it, so that a
//! scan that finds nothing is known to be able to find something.

use std::fs;
use std::path::{Path, PathBuf};

/// A source file: its path below `src/` with `/` as the separator, and its text.
struct Source {
    path: String,
    text: String,
}

fn read_sources() -> Vec<Source> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<Source>) {
        for entry in fs::read_dir(dir).expect("a readable source folder") {
            let path = entry.expect("a folder entry").path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let relative = path.strip_prefix(root).expect("a path below src");
                out.push(Source {
                    path: relative.to_string_lossy().replace('\\', "/"),
                    text: fs::read_to_string(&path).expect("a UTF-8 source file"),
                });
            }
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = Vec::new();
    walk(&root, &root, &mut sources);
    sources
}

/// The code of a file without its `//` comments (a line comment can mention anything).
/// String literals are kept: a path in a string is exactly what some rules look for.
fn code_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.split("//").next().unwrap_or("")))
}

fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// True when `word` appears in `line` as a whole word.
fn has_word(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(start, _)| {
        let before = line[..start].chars().next_back();
        let after = line[start + word.len()..].chars().next();
        !before.is_some_and(is_identifier_char) && !after.is_some_and(is_identifier_char)
    })
}

// ---- rule 1: unsafe code lives in platform/ only (section 11.1) ----

fn unsafe_outside_platform(sources: &[Source]) -> Vec<String> {
    sources
        .iter()
        .filter(|source| !source.path.starts_with("platform/"))
        .flat_map(|source| {
            code_lines(&source.text)
                .filter(|(_, line)| has_word(line, "unsafe"))
                .map(|(number, _)| {
                    format!("{}:{number} uses `unsafe` outside platform/", source.path)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

// ---- rule 2: the user-level Claude settings file (section 13.3, rule 2) ----

/// The only file that may name the settings file, and so write it.
const CLAUDE_SETTINGS_FILE: &str = "modules/sessions/hooks_install.rs";
/// This file may name the `.claude` folder (it only reads session files there), but not the
/// settings file.
const CLAUDE_PATHS_FILE: &str = "modules/sessions/claude_paths.rs";

fn claude_settings_outside_installer(sources: &[Source]) -> Vec<String> {
    sources
        .iter()
        .flat_map(|source| {
            let may_name_folder =
                source.path == CLAUDE_SETTINGS_FILE || source.path == CLAUDE_PATHS_FILE;
            let may_name_settings = source.path == CLAUDE_SETTINGS_FILE;
            code_lines(&source.text)
                .filter(|(_, line)| {
                    (!may_name_folder && line.contains(".claude"))
                        || (!may_name_settings && line.contains("settings.json"))
                })
                .map(|(number, _)| {
                    format!("{}:{number} names the Claude settings file", source.path)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

// ---- rule 3: the vault write rule (section 13.3, rule 1) ----

const VAULT_WRITE_FILE: &str = "modules/life/vault_write.rs";
const WRITE_CALLS: [&str; 6] = [
    "fs::write",
    "File::create",
    "OpenOptions",
    "fs::rename",
    "fs::remove_file",
    "fs::create_dir",
];

fn vault_writes_outside_the_one_function(sources: &[Source]) -> Vec<String> {
    let mut found = Vec::new();
    for source in sources.iter().filter(|s| s.path != VAULT_WRITE_FILE) {
        let in_life = source.path.starts_with("modules/life/");
        let mentions_vault = source.text.to_lowercase().contains("vault");
        for (number, line) in code_lines(&source.text) {
            // A write call in the life module, or in any file that handles a vault path.
            let writes = WRITE_CALLS.iter().any(|call| line.contains(call));
            if writes && (in_life || mentions_vault) {
                found.push(format!(
                    "{}:{number} writes a file near vault code",
                    source.path
                ));
            }
        }
    }
    found
}

fn habit_function_used_outside_its_tick(sources: &[Source]) -> Vec<String> {
    // Foundation 13.3: the daily tick (`life/service.rs`, function `tick`) is the only caller.
    let allowed = [VAULT_WRITE_FILE, "modules/life/service.rs"];
    sources
        .iter()
        .filter(|source| !allowed.contains(&source.path.as_str()))
        .flat_map(|source| {
            code_lines(&source.text)
                .filter(|(_, line)| line.contains("set_habit_property"))
                .map(|(number, _)| format!("{}:{number} uses set_habit_property", source.path))
                .collect::<Vec<_>>()
        })
        .collect()
}

// The rules of the life design (section 04 6.1) that go beyond the three above. Together they keep
// the vault write in one file that is easy to read and to audit.

/// Calls that only `vault_write.rs` may make inside `modules/life/`.
const LIFE_WRITE_CALLS: [&str; 10] = [
    "fs::write",
    "File::create",
    "OpenOptions",
    "fs::rename",
    "fs::remove_file",
    "fs::remove_dir",
    "fs::create_dir",
    "fs::copy",
    "set_permissions",
    "set_modified",
];

fn life_writes_outside_the_write_file(sources: &[Source]) -> Vec<String> {
    sources
        .iter()
        .filter(|s| s.path.starts_with("modules/life/") && s.path != VAULT_WRITE_FILE)
        .flat_map(|source| {
            code_lines(&source.text)
                .filter(|(_, line)| LIFE_WRITE_CALLS.iter().any(|call| line.contains(call)))
                .map(|(number, _)| format!("{}:{number} writes near the vault", source.path))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The text functions of `frontmatter.rs` are pure: no disk access, so they cannot write.
fn frontmatter_touches_the_disk(sources: &[Source]) -> Vec<String> {
    sources
        .iter()
        .filter(|s| s.path == "modules/life/frontmatter.rs")
        .flat_map(|source| {
            code_lines(&source.text)
                .filter(|(_, line)| {
                    line.contains("fs::")
                        || line.contains("std::fs")
                        || line.contains("std::io")
                        || has_word(line, "File")
                })
                .map(|(number, _)| format!("{}:{number} touches the disk", source.path))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The write file has its own temporary file code: it does not use the shared store or the
/// blocking helper (the route calls it from inside `blocking::run`).
fn write_file_uses_shared_code(sources: &[Source]) -> Vec<String> {
    sources
        .iter()
        .filter(|s| s.path == VAULT_WRITE_FILE)
        .flat_map(|source| {
            code_lines(&source.text)
                .filter(|(_, line)| {
                    line.contains("crate::store") || line.contains("core::blocking")
                })
                .map(|(number, _)| format!("{}:{number} uses shared code", source.path))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Obsidian links that may create a note or read the vault are never used. The one link Rain
/// uses, `obsidian://open`, is built in `daily.rs` only, after the note was seen to exist.
const OBSIDIAN_LINK_FILE: &str = "modules/life/daily.rs";
const OBSIDIAN_FORBIDDEN: [&str; 4] = [
    "obsidian://new",
    "obsidian://daily",
    "obsidian://search",
    "obsidian://hook-get-address",
];

fn obsidian_links_that_may_not_exist(sources: &[Source]) -> Vec<String> {
    sources
        .iter()
        .flat_map(|source| {
            // Not `code_lines`: it cuts a line at `//`, and a link contains one. A line that is
            // a comment as a whole is skipped instead.
            source
                .text
                .lines()
                .enumerate()
                .filter(|(_, line)| !line.trim_start().starts_with("//"))
                .filter(|(_, line)| {
                    OBSIDIAN_FORBIDDEN.iter().any(|link| line.contains(link))
                        || (source.path != OBSIDIAN_LINK_FILE && line.contains("obsidian://open"))
                })
                .map(|(index, _)| format!("{}:{} uses an Obsidian link", source.path, index + 1))
                .collect::<Vec<_>>()
        })
        .collect()
}

// ---- rule 4: what is never logged (section 13.4) ----

const SECRET_FIELDS: [&str; 6] = ["text", "content", "detail", "prompt", "body", "secret"];
const LOG_MACROS: [&str; 5] = ["trace!", "debug!", "info!", "warn!", "error!"];

/// Removes the contents of string literals, so that a word inside a message is not taken for a
/// field name. Handles `\"` inside a string; raw strings are not used in log calls.
fn without_string_contents(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut in_string, mut escaped) = (false, false);
    for c in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
                out.push('"');
            }
        } else {
            if c == '"' {
                in_string = true;
            }
            out.push(c);
        }
    }
    out
}

/// The text between the parentheses of the log macro call that starts at `open` (the `(`).
fn call_arguments(text: &str, open: usize) -> &str {
    let mut depth = 0usize;
    for (offset, c) in text[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &text[open + 1..open + offset];
                }
            }
            _ => {}
        }
    }
    &text[open + 1..]
}

/// The forbidden field names that a log call passes: `text = ...`, `%text`, `?text`, or a bare
/// `text,`. A field access such as `request.body` or `?body.reason` is not a field name.
fn forbidden_fields(arguments: &str) -> Vec<&'static str> {
    let flat = without_string_contents(arguments);
    SECRET_FIELDS
        .iter()
        .copied()
        .filter(|field| {
            flat.match_indices(field).any(|(start, _)| {
                let (head, tail) = (&flat[..start], &flat[start + field.len()..]);
                let whole_word = !head.chars().next_back().is_some_and(is_identifier_char)
                    && !tail.chars().next().is_some_and(is_identifier_char);
                if !whole_word || head.ends_with('.') || tail.starts_with('.') {
                    return false;
                }
                let before = head.trim_end().chars().next_back(); // None at the first argument
                let after = tail.trim_start();
                let named = after.starts_with('=') && !after.starts_with("==");
                let formatted = matches!(before, Some('%' | '?'));
                let bare = matches!(before, None | Some(','))
                    && (after.is_empty() || after.starts_with(','));
                named || formatted || bare
            })
        })
        .collect()
}

fn secrets_in_log_calls(sources: &[Source]) -> Vec<String> {
    let mut found = Vec::new();
    for source in sources {
        for (start, _) in source
            .text
            .match_indices('!')
            .filter(|(at, _)| LOG_MACROS.iter().any(|m| source.text[..=*at].ends_with(m)))
        {
            // `start` is the `!`; the `(` follows it.
            let rest = &source.text[start + 1..];
            if !rest.starts_with('(') {
                continue;
            }
            let arguments = call_arguments(&source.text, start + 1);
            let line = source.text[..start].matches('\n').count() + 1;
            for field in forbidden_fields(arguments) {
                found.push(format!(
                    "{}:{line} logs a field named `{field}`",
                    source.path
                ));
            }
        }
    }
    found
}

// ---- rule 5: game mode ends nothing by itself (section 06, tests) ----

/// Game mode ends work only through `WindDownPort`. It never stops a session, ends a process or
/// kills anything on its own, so these words may not appear in its files.
const GAME_MODE_FORBIDDEN: [&str; 4] = ["stop_session", "TerminateProcess", "taskkill", ".kill("];

fn game_mode_ending_things_itself(sources: &[Source]) -> Vec<String> {
    sources
        .iter()
        .filter(|source| source.path.starts_with("modules/gamemode/"))
        .flat_map(|source| {
            code_lines(&source.text)
                .flat_map(|(number, line)| {
                    GAME_MODE_FORBIDDEN
                        .iter()
                        .filter(|word| line.contains(*word))
                        .map(move |word| (number, *word))
                })
                .map(|(number, word)| format!("{}:{number} game mode uses `{word}`", source.path))
                .collect::<Vec<_>>()
        })
        .collect()
}

// ---- the scan of the real source ----

#[test]
fn the_backend_source_follows_the_rules() {
    let sources = read_sources();
    assert!(sources.len() > 20, "the scan found the source files");

    let mut found = Vec::new();
    found.extend(unsafe_outside_platform(&sources));
    found.extend(claude_settings_outside_installer(&sources));
    found.extend(vault_writes_outside_the_one_function(&sources));
    found.extend(habit_function_used_outside_its_tick(&sources));
    found.extend(life_writes_outside_the_write_file(&sources));
    found.extend(frontmatter_touches_the_disk(&sources));
    found.extend(write_file_uses_shared_code(&sources));
    found.extend(obsidian_links_that_may_not_exist(&sources));
    found.extend(secrets_in_log_calls(&sources));
    found.extend(game_mode_ending_things_itself(&sources));
    assert!(
        found.is_empty(),
        "source rules broken:\n{}",
        found.join("\n")
    );
}

/// Foundation 1.6: `arbitrary_precision` would break untagged number enums through feature
/// unification, so no crate of the workspace may switch it on.
#[test]
fn serde_json_does_not_have_the_arbitrary_precision_feature() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // clippy.toml forbids `Command::new` in the backend; this test is not the backend.
    #[allow(clippy::disallowed_methods)]
    let output = std::process::Command::new(env!("CARGO"))
        .args(["tree", "--workspace", "-e", "features", "-i", "serde_json"])
        .current_dir(root)
        .output()
        .expect("cargo tree could be started");
    assert!(output.status.success(), "cargo tree failed");
    let tree = String::from_utf8_lossy(&output.stdout);
    assert!(
        tree.contains("serde_json feature \"default\""),
        "the tree lists the features of serde_json"
    );
    assert!(
        !tree.contains("arbitrary_precision"),
        "a crate switched on serde_json/arbitrary_precision"
    );
}

#[test]
fn the_crate_forbids_unsafe_code_outside_platform() {
    let lib = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
        .expect("lib.rs");
    assert!(lib.contains("#![deny(unsafe_code)]"));
    let platform =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform/mod.rs"))
            .expect("platform/mod.rs");
    assert!(platform.contains("#![allow(unsafe_code)]"));
}

// ---- each rule must be able to fail ----

fn source(path: &str, text: &str) -> Vec<Source> {
    vec![Source {
        path: path.into(),
        text: text.into(),
    }]
}

#[test]
fn unsafe_is_found_outside_platform_and_allowed_inside() {
    let code = "fn f() { unsafe { g() } }";
    assert_eq!(unsafe_outside_platform(&source("core/x.rs", code)).len(), 1);
    assert!(unsafe_outside_platform(&source("platform/x.rs", code)).is_empty());
    // A comment, a lint name and a longer word are not `unsafe` code.
    let harmless = "// unsafe is explained here\n#![deny(unsafe_code)]\nlet unsafely = 1;";
    assert!(unsafe_outside_platform(&source("core/x.rs", harmless)).is_empty());
}

#[test]
fn the_claude_settings_file_is_found_outside_the_installer() {
    let code = r#"let p = home.join(".claude").join("settings.json");"#;
    assert_eq!(
        claude_settings_outside_installer(&source("core/x.rs", code)).len(),
        1
    );
    assert!(claude_settings_outside_installer(&source(CLAUDE_SETTINGS_FILE, code)).is_empty());
}

#[test]
fn the_claude_folder_may_be_named_in_the_paths_file_but_not_the_settings_file() {
    let folder = r#"let p = home.join(".claude").join("projects");"#;
    assert!(claude_settings_outside_installer(&source(CLAUDE_PATHS_FILE, folder)).is_empty());
    let settings = r#"let p = dir.join("settings.json");"#;
    assert_eq!(
        claude_settings_outside_installer(&source(CLAUDE_PATHS_FILE, settings)).len(),
        1
    );
}

#[test]
fn a_file_write_near_vault_code_is_found() {
    let code = "let vault = settings.vault_path();\nstd::fs::write(vault.join(\"a.md\"), text)?;";
    assert_eq!(
        vault_writes_outside_the_one_function(&source("modules/games/x.rs", code)).len(),
        1
    );
    // The same call in the one allowed file, and a write in a file that knows no vault, pass.
    assert!(vault_writes_outside_the_one_function(&source(VAULT_WRITE_FILE, code)).is_empty());
    let unrelated = "std::fs::write(cache.join(\"icon.png\"), bytes)?;";
    assert!(
        vault_writes_outside_the_one_function(&source("modules/games/x.rs", unrelated)).is_empty()
    );
    // Anything that writes inside the life module, outside the one file, is found.
    assert_eq!(
        vault_writes_outside_the_one_function(&source("modules/life/service.rs", unrelated)).len(),
        1
    );
}

#[test]
fn the_habit_function_may_be_named_only_in_the_tick_and_its_own_file() {
    let code = "vault_write::set_habit_property(path, key, true)";
    for path in ["modules/jobs/x.rs", "modules/life/routes.rs"] {
        assert_eq!(
            habit_function_used_outside_its_tick(&source(path, code)).len(),
            1,
            "{path}"
        );
    }
    assert!(
        habit_function_used_outside_its_tick(&source("modules/life/service.rs", code)).is_empty()
    );
    assert!(habit_function_used_outside_its_tick(&source(VAULT_WRITE_FILE, code)).is_empty());
}

#[test]
fn the_life_module_may_not_write_anywhere_but_its_write_file() {
    let code = "std::fs::copy(a, b)?;";
    assert_eq!(
        life_writes_outside_the_write_file(&source("modules/life/store.rs", code)).len(),
        1
    );
    assert!(life_writes_outside_the_write_file(&source(VAULT_WRITE_FILE, code)).is_empty());
    assert!(life_writes_outside_the_write_file(&source("modules/games/x.rs", code)).is_empty());
    // A comment that names a call is fine.
    let comment = "// never fs::write here";
    assert!(life_writes_outside_the_write_file(&source("modules/life/x.rs", comment)).is_empty());
}

#[test]
fn the_frontmatter_file_may_not_touch_the_disk() {
    for code in [
        "use std::fs;",
        "let f = std::fs::File::open(p);",
        "use std::io::Read;",
        "let x: File;",
    ] {
        assert_eq!(
            frontmatter_touches_the_disk(&source("modules/life/frontmatter.rs", code)).len(),
            1,
            "{code}"
        );
    }
    assert!(
        frontmatter_touches_the_disk(&source("modules/life/frontmatter.rs", "let profile = 1;"))
            .is_empty()
    );
    assert!(
        frontmatter_touches_the_disk(&source("modules/life/index.rs", "use std::fs;")).is_empty()
    );
}

#[test]
fn the_write_file_may_not_use_the_shared_store_or_the_blocking_helper() {
    for code in [
        "use crate::store::toml_file;",
        "crate::core::blocking::run(f)",
    ] {
        assert_eq!(
            write_file_uses_shared_code(&source(VAULT_WRITE_FILE, code)).len(),
            1,
            "{code}"
        );
    }
    assert!(
        write_file_uses_shared_code(&source("modules/life/service.rs", "use crate::store::x;"))
            .is_empty()
    );
}

#[test]
fn only_the_open_link_of_the_daily_file_is_allowed() {
    let open = r#"format!("obsidian://open?vault={id}")"#;
    assert!(obsidian_links_that_may_not_exist(&source(OBSIDIAN_LINK_FILE, open)).is_empty());
    assert_eq!(
        obsidian_links_that_may_not_exist(&source("modules/life/service.rs", open)).len(),
        1
    );
    for link in [
        "obsidian://new?vault=a",
        "obsidian://daily",
        "obsidian://search?query=x",
    ] {
        let code = format!("let l = \"{link}\";");
        assert_eq!(
            obsidian_links_that_may_not_exist(&source(OBSIDIAN_LINK_FILE, &code)).len(),
            1,
            "{link}"
        );
    }
}

#[test]
fn a_log_call_with_a_secret_field_is_found() {
    let bad = [
        r#"info!(text = %note, "read a note");"#,
        r#"warn!(content, "saved");"#,
        r#"error!(error = %e, detail = ?request, "failed");"#,
        r#"debug!(%prompt, "sent");"#,
        r#"info!(path = %p, body = len, "x");"#,
    ];
    for code in bad {
        assert!(
            !secrets_in_log_calls(&source("core/x.rs", code)).is_empty(),
            "not found in: {code}"
        );
    }
}

#[test]
fn ordinary_log_calls_pass() {
    let good = [
        r#"info!(module = module.name, "module started");"#,
        r#"info!("the text of the note is not logged");"#,
        r#"warn!(open = connections.len(), "too many connections");"#,
        r#"debug!(%method, path, status = 200, "request");"#,
        r#"error!(topic = T::NAME, error = %e, "the payload cannot be written");"#,
        r#"info!(size = request.body.len(), "got a request");"#,
        r#"info!(reason = ?body.reason, "stop requested");"#,
    ];
    for code in good {
        assert!(
            secrets_in_log_calls(&source("core/x.rs", code)).is_empty(),
            "a false alarm in: {code}"
        );
    }
}

#[test]
fn game_mode_that_ends_something_itself_is_found() {
    let code = "fn f(pid: u32) { terminate(pid); sessions.stop_session(id); }";
    assert_eq!(
        game_mode_ending_things_itself(&source("modules/gamemode/service.rs", code)).len(),
        1
    );
    assert!(game_mode_ending_things_itself(&source("modules/jobs/service.rs", code)).is_empty());
    let clean = "fn f(port: &dyn WindDownPort) { port.stop_all(budget); }";
    assert!(
        game_mode_ending_things_itself(&source("modules/gamemode/service.rs", clean)).is_empty()
    );
}
