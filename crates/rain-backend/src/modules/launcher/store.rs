//! The files and tables of the launcher. Section 01 4 and 5.
//!
//! Three TOML files in the data folder (`pins.toml`, `pools.toml`, `app-ids.toml`) and two
//! SQLite tables (`launcher_app`, `launcher_launch`). This file knows their shapes, their rules
//! and how a change is written into them. It does no I/O on its own.
//!
//! The policy for a file with a mistake (section 01 4): `pins.toml` and `pools.toml` are strict,
//! because a half-loaded pool list would silently hide a pool, so one bad item refuses the whole
//! file and the last good value stays in use. `app-ids.toml` is lenient: a bad entry is skipped
//! and the rest is used, because a refused id map would give every app a new id and break the
//! pins and pools that point to them.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params};
use serde::Deserialize;
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value, value};

use super::apps::{Issued, Row};
use super::limits::{
    ARGS_MAX, DELAY_MAX_MS, DELAY_SUM_MAX_MS, HISTORY_PER_TARGET, ID_MAX, PINS_MAX, POOL_MAX,
    POOL_MEMBERS_MAX, POOL_NAME_MAX,
};
use crate::core::error::{ApiError, ApiResult};
use crate::store::migrations::Migration;
use crate::store::toml_file::{set_value, string_value};

pub const PINS_FILE: &str = "pins.toml";
pub const POOLS_FILE: &str = "pools.toml";
pub const APP_IDS_FILE: &str = "app-ids.toml";

pub const MIGRATIONS: &[Migration] = &[Migration {
    id: 1,
    name: "create_launcher",
    sql: include_str!("migrations/0001_create_launcher.sql"),
}];

/// Ids use only these characters, so they are safe in a path, a header and a file name.
pub fn is_plain_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= ID_MAX
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '-'))
}

// ---- pins.toml ----

const PINS_HEADER: &str = "\
# pins.toml: what is pinned to Home, in order. Rain edits this file when you pin or unpin;
# you may reorder the entries or add one by hand.
";

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct PinsFile {
    pub pin: Vec<PinRow>,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PinRow {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
}

pub fn validate_pins(file: &PinsFile) -> Result<(), String> {
    if file.pin.len() > PINS_MAX {
        return Err(format!("there can be at most {PINS_MAX} pins"));
    }
    for pin in &file.pin {
        if !matches!(pin.kind.as_str(), "app" | "pool" | "project" | "game") {
            return Err(format!(
                "a pin has the type '{}'; use app, pool, project or game",
                pin.kind
            ));
        }
        if !is_plain_id(&pin.id) {
            return Err(
                "a pin has an id that is empty, too long or has characters outside A-Z a-z 0-9 . _ ~ -"
                    .to_owned(),
            );
        }
    }
    Ok(())
}

impl PinsFile {
    /// The pins in file order. A pair that occurs twice counts once (the first).
    pub fn in_order(&self) -> Vec<(&str, &str)> {
        let mut seen = HashSet::new();
        self.pin
            .iter()
            .map(|pin| (pin.kind.as_str(), pin.id.as_str()))
            .filter(|pair| seen.insert(*pair))
            .collect()
    }

    pub fn contains(&self, kind: &str, id: &str) -> bool {
        self.pin.iter().any(|pin| pin.kind == kind && pin.id == id)
    }
}

/// Appends `[[pin]]`. Does nothing when the pin is there.
pub fn add_pin(document: &mut DocumentMut, kind: &str, id: &str) -> ApiResult<()> {
    start_file(document, PINS_HEADER);
    let pins = array_of_tables(document, "pin", PINS_FILE)?;
    if pins
        .iter()
        .any(|t| text_of(t, "type") == kind && text_of(t, "id") == id)
    {
        return Ok(());
    }
    let mut table = Table::new();
    table.insert("type", value(kind));
    table.insert("id", value(id));
    pins.push(table);
    Ok(())
}

/// Removes the first `[[pin]]` of this pair, with the comment lines directly above it. A pin
/// that is not there is not an error.
pub fn remove_pin(document: &mut DocumentMut, kind: &str, id: &str) -> ApiResult<()> {
    if document.get("pin").is_none() {
        return Ok(());
    }
    let pins = array_of_tables(document, "pin", PINS_FILE)?;
    let at = pins
        .iter()
        .position(|t| text_of(t, "type") == kind && text_of(t, "id") == id);
    if let Some(at) = at {
        pins.remove(at);
    }
    if pins.is_empty() {
        document.remove("pin");
    }
    Ok(())
}

// ---- pools.toml ----

const POOLS_HEADER: &str = "\
# pools.toml: groups of apps that start together. Rain rewrites a pool when you save it in the pool editor.
# The editor cannot set `args` or `delay_ms` yet; set them here, then reopen Rain's Apps screen.
";

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct PoolsFile {
    pub pool: Vec<PoolRow>,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PoolRow {
    pub id: String,
    pub name: String,
    pub members: Vec<MemberRow>,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MemberRow {
    pub app: String,
    pub args: Option<String>,
    pub delay_ms: Option<u32>,
}

/// The rules of a pool, as one message for a person or the file's issue line. The same rules
/// check a draft from the dialog (`pools.rs` maps the same cases to error reasons).
pub fn validate_pools(file: &PoolsFile) -> Result<(), String> {
    if file.pool.len() > POOL_MAX {
        return Err(format!("there can be at most {POOL_MAX} pools"));
    }
    let mut ids = HashSet::new();
    for pool in &file.pool {
        let label = format!("pool '{}'", pool.id);
        let Some(rest) = pool.id.strip_prefix("pool-") else {
            return Err(format!("{label}: the id must start with pool-"));
        };
        if rest.is_empty() || rest.len() > 100 || !is_plain_id(&pool.id) {
            return Err(format!(
                "{label}: the id is too long or has characters outside A-Z a-z 0-9 . _ ~ -"
            ));
        }
        if !ids.insert(pool.id.as_str()) {
            return Err(format!("{label}: the id is used twice"));
        }
        let name_length = pool.name.trim().chars().count();
        if name_length == 0 || name_length > POOL_NAME_MAX {
            return Err(format!(
                "{label}: the name needs 1 to {POOL_NAME_MAX} characters"
            ));
        }
        if pool.members.is_empty() || pool.members.len() > POOL_MEMBERS_MAX {
            return Err(format!("{label}: it needs 1 to {POOL_MEMBERS_MAX} members"));
        }
        let mut delays = 0u32;
        for member in &pool.members {
            if !is_plain_id(&member.app) {
                return Err(format!("{label}: a member has a bad app id"));
            }
            if let Some(args) = &member.args
                && (args.chars().count() > ARGS_MAX || args.contains(['\n', '\r']))
            {
                return Err(format!(
                    "{label}: the args of a member are too long or have a line break"
                ));
            }
            let delay = member.delay_ms.unwrap_or(0);
            if delay > DELAY_MAX_MS {
                return Err(format!(
                    "{label}: delay_ms of a member is more than {DELAY_MAX_MS}"
                ));
            }
            delays = delays.saturating_add(delay);
        }
        if delays > DELAY_SUM_MAX_MS {
            return Err(format!(
                "{label}: the delays add up to more than {DELAY_SUM_MAX_MS} ms"
            ));
        }
    }
    Ok(())
}

/// The members as the multi-line array of the sample file in the header comment.
fn members_value(members: &[MemberRow]) -> Value {
    let mut array = Array::new();
    for member in members {
        let mut inline = InlineTable::new();
        inline.insert("app", string_value(&member.app));
        if let Some(args) = &member.args {
            inline.insert("args", string_value(args));
        }
        if let Some(delay) = member.delay_ms.filter(|delay| *delay > 0) {
            inline.insert("delay_ms", Value::from(i64::from(delay)));
        }
        let mut item = Value::InlineTable(inline);
        item.decor_mut().set_prefix("\n  ");
        array.push_formatted(item);
    }
    array.set_trailing("\n");
    array.set_trailing_comma(true);
    Value::Array(array)
}

/// Appends a `[[pool]]` table.
pub fn create_pool(
    document: &mut DocumentMut,
    id: &str,
    name: &str,
    members: &[MemberRow],
) -> ApiResult<()> {
    start_file(document, POOLS_HEADER);
    let pools = array_of_tables(document, "pool", POOLS_FILE)?;
    let mut table = Table::new();
    table.insert("id", value(id));
    table.insert("name", Item::Value(string_value(name)));
    table.insert("members", Item::Value(members_value(members)));
    pools.push(table);
    Ok(())
}

/// Replaces `name` and `members` of the pool. Every other key and comment of the table stays;
/// the comments inside the old `members` array do not (a limit, documented in section 01 4.2).
pub fn update_pool(
    document: &mut DocumentMut,
    id: &str,
    name: &str,
    members: &[MemberRow],
) -> ApiResult<()> {
    if document.get("pool").is_none() {
        return Err(unknown_pool());
    }
    let pools = array_of_tables(document, "pool", POOLS_FILE)?;
    let table = pools
        .iter_mut()
        .find(|t| text_of(t, "id") == id)
        .ok_or_else(unknown_pool)?;
    set_value(table, "name", string_value(name));
    set_value(table, "members", members_value(members));
    Ok(())
}

/// Removes the pool's table with the comments directly above it. An unknown id is not an error.
pub fn delete_pool(document: &mut DocumentMut, id: &str) -> ApiResult<()> {
    if document.get("pool").is_none() {
        return Ok(());
    }
    let pools = array_of_tables(document, "pool", POOLS_FILE)?;
    let at = pools.iter().position(|t| text_of(t, "id") == id);
    if let Some(at) = at {
        pools.remove(at);
    }
    if pools.is_empty() {
        document.remove("pool");
    }
    Ok(())
}

fn unknown_pool() -> ApiError {
    ApiError::not_found("Pool not found")
        .with_source("launcher")
        .with_reason("unknownPool")
}

// ---- app-ids.toml ----

const APP_IDS_HEADER: &str = "\
# app-ids.toml: the stable id of every app Rain has listed. Pins and pools refer to these ids,
# so do not change an `id` unless you also change pins.toml and pools.toml.
# Optional per app:  hidden = true   keywords = [\"editor\", \"ide\"]
";

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct AppIdsFile {
    pub app: Vec<AppIdRow>,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppIdRow {
    pub id: String,
    pub key: String,
    pub name: Option<String>,
    pub hidden: bool,
    pub keywords: Vec<String>,
}

/// What the id map knows about one app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdEntry {
    pub id: String,
    pub hidden: bool,
    pub keywords: Vec<String>,
}

/// `app-ids.toml` as a lookup by key. Built leniently: an entry with a bad id or key, or a
/// duplicate, is skipped and the rest is used.
#[derive(Debug, Default, Clone)]
pub struct IdMap {
    by_key: HashMap<String, IdEntry>,
    taken: HashSet<String>,
}

impl IdMap {
    pub fn entry(&self, key: &str) -> Option<&IdEntry> {
        self.by_key.get(key)
    }

    /// Every id that is in use, also the ids of apps that are gone: a new app never gets one.
    pub fn taken_ids(&self) -> HashSet<String> {
        self.taken.clone()
    }
}

pub fn id_map(file: &AppIdsFile) -> IdMap {
    let mut map = IdMap::default();
    for row in &file.app {
        let key = row.key.to_lowercase();
        let id_ok = row
            .id
            .strip_prefix("app-")
            .is_some_and(|rest| !rest.is_empty() && rest.len() <= 100)
            && is_plain_id(&row.id);
        if !id_ok || key.is_empty() || map.by_key.contains_key(&key) || map.taken.contains(&row.id)
        {
            continue;
        }
        let mut keywords: Vec<String> = Vec::new();
        for word in row.keywords.iter().map(|word| word.trim().to_lowercase()) {
            if !word.is_empty() && word.chars().count() <= 40 && !keywords.contains(&word) {
                keywords.push(word);
            }
        }
        keywords.truncate(12);
        map.taken.insert(row.id.clone());
        map.by_key.insert(
            key,
            IdEntry {
                id: row.id.clone(),
                hidden: row.hidden,
                keywords,
            },
        );
    }
    map
}

/// Appends one `[[app]]` per new key, in one write.
pub fn append_app_ids(document: &mut DocumentMut, issued: &[Issued]) -> ApiResult<()> {
    start_file(document, APP_IDS_HEADER);
    let apps = array_of_tables(document, "app", APP_IDS_FILE)?;
    for new in issued {
        let mut table = Table::new();
        table.insert("id", value(new.id.as_str()));
        table.insert("key", Item::Value(string_value(&new.key)));
        table.insert("name", Item::Value(string_value(&new.name)));
        apps.push(table);
    }
    Ok(())
}

// ---- helpers for the edits ----

/// A file that has no keys at all (new, empty, or only comments) gets `schema_version = 1` as its
/// first line. The comment above it is the header, or the user's own comments when the file
/// already had some (they are kept at the top).
fn start_file(document: &mut DocumentMut, header: &str) {
    if !document.as_table().is_empty() {
        return;
    }
    let comments = document.trailing().as_str().unwrap_or_default().to_owned();
    let lead = if comments.trim().is_empty() {
        header.to_owned()
    } else {
        comments
    };
    document.set_trailing("");
    document.insert("schema_version", value(1));
    if let Some(mut key) = document.key_mut("schema_version") {
        key.leaf_decor_mut().set_prefix(lead);
    }
}

/// The `[[name]]` array of the document, created when it is not there. A file that wrote the
/// list in another form (`pin = [ {..} ]`) is read by Rain but cannot be edited by it.
fn array_of_tables<'a>(
    document: &'a mut DocumentMut,
    name: &str,
    file: &str,
) -> ApiResult<&'a mut ArrayOfTables> {
    document
        .entry(name)
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or_else(|| {
            ApiError::conflict(
                "configBroken",
                format!(
                    "{file} writes its list in a form that Rain cannot edit. Use [[{name}]] tables."
                ),
            )
        })
}

fn text_of(table: &Table, key: &str) -> String {
    table
        .get(key)
        .and_then(Item::as_str)
        .unwrap_or_default()
        .to_owned()
}

// ---- SQLite ----

/// The raw rows of the last scan, in scan order.
pub fn load_rows(connection: &Connection) -> rusqlite::Result<Vec<Row>> {
    let mut statement = connection.prepare(
        "SELECT key, aumid, name, target, args, target_mtime FROM launcher_app ORDER BY rowid",
    )?;
    statement
        .query_map([], |row| {
            Ok(Row {
                key: row.get(0)?,
                aumid: row.get(1)?,
                name: row.get(2)?,
                target: row.get(3)?,
                args: row.get(4)?,
                target_mtime: row.get(5)?,
            })
        })?
        .collect()
}

/// Replaces the cache as a whole: delete all, insert all, in one transaction.
pub fn replace_rows(
    connection: &mut Connection,
    rows: &[Row],
    scanned_at: &str,
) -> rusqlite::Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute("DELETE FROM launcher_app", [])?;
    {
        let mut insert = transaction.prepare(
            "INSERT INTO launcher_app (key, aumid, name, target, args, target_mtime, scanned_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for row in rows {
            insert.execute(params![
                row.key,
                row.aumid,
                row.name,
                row.target,
                row.args,
                row.target_mtime,
                scanned_at
            ])?;
        }
    }
    transaction.commit()
}

/// The newest use of every target: `(type, id, at)`.
pub fn load_history(connection: &Connection) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut statement = connection.prepare(
        "SELECT target_type, target_id, MAX(at) FROM launcher_launch GROUP BY target_type, target_id",
    )?;
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect()
}

/// One row per used target of a finished run, then the rows of each target beyond the newest
/// `HISTORY_PER_TARGET` are deleted.
pub fn insert_history(
    connection: &mut Connection,
    used: &[(&str, &str)],
    at: &str,
    run_id: &str,
) -> rusqlite::Result<()> {
    let transaction = connection.transaction()?;
    for (kind, id) in used {
        transaction.execute(
            "INSERT INTO launcher_launch (target_type, target_id, at, run_id) VALUES (?1, ?2, ?3, ?4)",
            params![kind, id, at, run_id],
        )?;
        transaction.execute(
            "DELETE FROM launcher_launch WHERE target_type = ?1 AND target_id = ?2 AND id NOT IN (
                 SELECT id FROM launcher_launch WHERE target_type = ?1 AND target_id = ?2
                 ORDER BY at DESC, id DESC LIMIT ?3)",
            params![kind, id, HISTORY_PER_TARGET],
        )?;
    }
    transaction.commit()
}

/// A deleted pool takes its history with it.
pub fn delete_history(connection: &Connection, kind: &str, id: &str) -> rusqlite::Result<()> {
    connection.execute(
        "DELETE FROM launcher_launch WHERE target_type = ?1 AND target_id = ?2",
        params![kind, id],
    )?;
    Ok(())
}

/// At backend start: rows older than the cut-off (an RFC 3339 text) go.
pub fn prune_old_history(connection: &Connection, cutoff: &str) -> rusqlite::Result<()> {
    connection.execute("DELETE FROM launcher_launch WHERE at < ?1", params![cutoff])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::toml_file::render;

    fn parse(text: &str) -> DocumentMut {
        text.parse().expect("valid TOML")
    }

    fn member(app: &str) -> MemberRow {
        MemberRow {
            app: app.into(),
            ..MemberRow::default()
        }
    }

    // ---- pins ----

    #[test]
    fn a_pins_file_is_checked_and_read_in_order() {
        let file: PinsFile = toml::from_str(
            "schema_version = 1\n[[pin]]\ntype = 'pool'\nid = 'pool-a'\n[[pin]]\ntype = 'app'\nid = 'app-b'\n[[pin]]\ntype = 'pool'\nid = 'pool-a'\n",
        )
        .expect("a valid pins file");
        assert!(validate_pins(&file).is_ok());
        assert_eq!(file.in_order(), [("pool", "pool-a"), ("app", "app-b")]);
        assert!(file.contains("app", "app-b") && !file.contains("game", "app-b"));
    }

    #[test]
    fn each_rule_of_the_pins_file_has_a_case() {
        let bad_type: PinsFile = toml::from_str("[[pin]]\ntype = 'job'\nid = 'x'\n").unwrap();
        assert!(validate_pins(&bad_type).unwrap_err().contains("job"));
        let bad_id: PinsFile = toml::from_str("[[pin]]\ntype = 'app'\nid = 'a b'\n").unwrap();
        assert!(validate_pins(&bad_id).is_err());
        let empty_id: PinsFile = toml::from_str("[[pin]]\ntype = 'app'\n").unwrap();
        assert!(validate_pins(&empty_id).is_err());
        let many = PinsFile {
            pin: vec![
                PinRow {
                    kind: "app".into(),
                    id: "x".into()
                };
                PINS_MAX + 1
            ],
        };
        assert!(validate_pins(&many).is_err());
    }

    #[test]
    fn pinning_keeps_the_header_and_unpinning_removes_the_comment_above_the_pin() {
        let mut doc = parse("");
        add_pin(&mut doc, "pool", "pool-a").unwrap();
        add_pin(&mut doc, "app", "app-b").unwrap();
        add_pin(&mut doc, "app", "app-b").unwrap();
        let text = render(&doc, false);
        assert!(text.starts_with("# pins.toml: what is pinned to Home"));
        assert!(text.contains("schema_version = 1"));
        assert_eq!(text.matches("[[pin]]").count(), 2);

        let mut doc = parse(
            "# my header\nschema_version = 1\n\n# the notes pin\n[[pin]]\ntype = \"app\"\nid = \"app-notes\"\n\n[[pin]]\ntype = \"app\"\nid = \"app-b\"\n",
        );
        remove_pin(&mut doc, "app", "app-notes").unwrap();
        let text = render(&doc, false);
        assert!(text.starts_with("# my header\nschema_version = 1"));
        assert!(!text.contains("the notes pin") && !text.contains("app-notes"));
        assert!(text.contains("app-b"));
        // Removing something that is not there changes nothing; the last pin takes the key with it.
        let before = doc.to_string();
        remove_pin(&mut doc, "app", "app-gone").unwrap();
        assert_eq!(doc.to_string(), before);
        remove_pin(&mut doc, "app", "app-b").unwrap();
        assert!(!doc.to_string().contains("[[pin]]"));
    }

    #[test]
    fn a_pin_list_in_a_form_rain_cannot_edit_is_refused_not_overwritten() {
        let mut doc = parse("pin = [ { type = 'app', id = 'app-a' } ]\n");
        let error = add_pin(&mut doc, "app", "app-b").unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("configBroken"));
    }

    #[test]
    fn a_file_of_comments_only_keeps_its_comments_on_top() {
        let mut doc = parse("# my own notes\n");
        add_pin(&mut doc, "app", "app-a").unwrap();
        let text = render(&doc, false);
        assert!(text.starts_with("# my own notes\nschema_version = 1"));
    }

    #[test]
    fn crlf_files_stay_crlf() {
        let original = "schema_version = 1\r\n\r\n[[pin]]\r\ntype = \"app\"\r\nid = \"app-a\"\r\n";
        let mut doc = parse(original);
        add_pin(&mut doc, "app", "app-b").unwrap();
        let text = render(&doc, true);
        assert!(!text.replace("\r\n", "").contains('\n'), "{text:?}");
        assert!(text.contains("app-b"));
    }

    // ---- pools ----

    fn parse_pools(text: &str) -> PoolsFile {
        toml::from_str(text).expect("a valid pools file")
    }

    const POOL: &str = "[[pool]]\nid = 'pool-a'\nname = 'A'\nmembers = [{ app = 'app-x' }]\n";

    #[test]
    fn a_valid_pools_file_passes() {
        let file = parse_pools(
            "schema_version = 1\n[[pool]]\nid = 'pool-web-dev'\nname = 'Web dev'\nmembers = [\n  { app = 'app-vs-code' },\n  { app = 'app-chrome', args = '--new-window http://localhost:5173', delay_ms = 2000 },\n]\n",
        );
        assert!(validate_pools(&file).is_ok());
        assert_eq!(file.pool[0].members[1].delay_ms, Some(2000));
    }

    #[test]
    fn each_rule_of_the_pools_file_has_a_case() {
        let cases = [
            ("id = 'x'\nname = 'A'\nmembers = [{ app = 'a' }]", "pool-"),
            ("id = 'pool-'\nname = 'A'\nmembers = [{ app = 'a' }]", "id"),
            (
                "id = 'pool-a b'\nname = 'A'\nmembers = [{ app = 'a' }]",
                "id",
            ),
            (
                "id = 'pool-a'\nname = '  '\nmembers = [{ app = 'a' }]",
                "name",
            ),
            ("id = 'pool-a'\nname = 'A'\nmembers = []", "members"),
            (
                "id = 'pool-a'\nname = 'A'\nmembers = [{ app = 'a b' }]",
                "app id",
            ),
            (
                "id = 'pool-a'\nname = 'A'\nmembers = [{ app = 'a', args = \"x\\ny\" }]",
                "line break",
            ),
            (
                "id = 'pool-a'\nname = 'A'\nmembers = [{ app = 'a', delay_ms = 60001 }]",
                "delay_ms",
            ),
        ];
        for (body, word) in cases {
            let file = parse_pools(&format!("[[pool]]\n{body}\n"));
            let error = validate_pools(&file).expect_err(body);
            assert!(error.contains(word), "{error} should name {word}");
        }
        let twice = parse_pools(&format!("{POOL}{POOL}"));
        assert!(validate_pools(&twice).unwrap_err().contains("twice"));
        let sum = parse_pools(
            "[[pool]]\nid = 'pool-a'\nname = 'A'\nmembers = [{ app = 'a', delay_ms = 60000 },{ app = 'a', delay_ms = 60000 },{ app = 'a', delay_ms = 60000 },{ app = 'a', delay_ms = 60000 },{ app = 'a', delay_ms = 60000 },{ app = 'a', delay_ms = 60000 },{ app = 'a', delay_ms = 60000 },{ app = 'a', delay_ms = 60000 },{ app = 'a', delay_ms = 60000 },{ app = 'a', delay_ms = 60000 },{ app = 'a', delay_ms = 1 }]\n",
        );
        assert!(validate_pools(&sum).unwrap_err().contains("add up"));
        let many = PoolsFile {
            pool: (0..=POOL_MAX)
                .map(|n| PoolRow {
                    id: format!("pool-{n}"),
                    name: "A".into(),
                    members: vec![member("a")],
                })
                .collect(),
        };
        assert!(validate_pools(&many).unwrap_err().contains("at most"));
    }

    #[test]
    fn creating_a_pool_writes_the_header_and_a_readable_members_array() {
        let mut doc = parse("");
        let members = vec![
            member("app-vs-code"),
            MemberRow {
                app: "app-chrome".into(),
                args: Some("--new-window".into()),
                delay_ms: Some(2000),
            },
        ];
        create_pool(&mut doc, "pool-web", "Web", &members).unwrap();
        let text = render(&doc, false);
        assert!(text.starts_with("# pools.toml: groups of apps"));
        let back: PoolsFile = toml::from_str(&text).expect("the new file reads back");
        assert_eq!(back.pool[0].members, members);
        assert!(validate_pools(&back).is_ok());
        assert!(
            text.contains("{ app = \"app-vs-code\" }") || text.contains("{ app = 'app-vs-code' }"),
            "{text}"
        );
    }

    #[test]
    fn updating_a_pool_keeps_its_other_keys_and_comments() {
        let mut doc = parse(
            "schema_version = 1\n\n# my favourite\n[[pool]]\nid = 'pool-a'\nname = 'Old' # why\nnote = 'mine'\nmembers = [{ app = 'app-x' }]\n",
        );
        update_pool(
            &mut doc,
            "pool-a",
            "New",
            &[member("app-y"), member("app-z")],
        )
        .unwrap();
        let text = render(&doc, false);
        assert!(text.contains("# my favourite") && text.contains("note = 'mine'"));
        assert!(text.contains("# why"), "a trailing comment stays: {text}");
        let back: PoolsFile = toml::from_str(&text).unwrap();
        assert_eq!(back.pool[0].name, "New");
        assert_eq!(back.pool[0].members.len(), 2);
        assert_eq!(
            update_pool(&mut doc, "pool-gone", "x", &[member("a")])
                .unwrap_err()
                .body
                .reason
                .as_deref(),
            Some("unknownPool")
        );
    }

    #[test]
    fn deleting_a_pool_takes_the_comment_above_it_and_an_unknown_id_is_fine() {
        let mut doc = parse(&format!(
            "schema_version = 1\n\n# first\n{POOL}\n# second\n[[pool]]\nid = 'pool-b'\nname = 'B'\nmembers = [{{ app = 'a' }}]\n"
        ));
        delete_pool(&mut doc, "pool-a").unwrap();
        let text = doc.to_string();
        assert!(!text.contains("# first") && text.contains("# second"));
        delete_pool(&mut doc, "pool-nope").unwrap();
        delete_pool(&mut doc, "pool-b").unwrap();
        assert!(!doc.to_string().contains("[[pool]]"));
        let mut empty = parse("");
        delete_pool(&mut empty, "pool-a").unwrap();
        assert_eq!(empty.to_string(), "");
    }

    // ---- the id map ----

    #[test]
    fn the_id_map_skips_bad_and_duplicate_entries_and_keeps_the_rest() {
        let file: AppIdsFile = toml::from_str(
            "[[app]]\nid='app-a'\nkey='AUMID:One'\nkeywords=['Editor',' ide ','editor','']\n\
             [[app]]\nid='nope'\nkey='aumid:two'\n\
             [[app]]\nid='app-a'\nkey='aumid:three'\n\
             [[app]]\nid='app-d'\nkey='aumid:one'\n\
             [[app]]\nid='app-e'\nkey=''\n\
             [[app]]\nid='app-f'\nkey='aumid:f'\nhidden=true\n",
        )
        .unwrap();
        let map = id_map(&file);
        let one = map
            .entry("aumid:one")
            .expect("the first entry of the key wins");
        assert_eq!(one.id, "app-a");
        assert_eq!(one.keywords, ["editor", "ide"]);
        assert!(map.entry("aumid:two").is_none() && map.entry("aumid:three").is_none());
        assert!(map.entry("aumid:f").unwrap().hidden);
        let taken = map.taken_ids();
        assert!(taken.contains("app-a") && taken.contains("app-f") && !taken.contains("app-d"));
    }

    #[test]
    fn new_app_ids_are_appended_once_with_the_header() {
        let mut doc = parse("");
        let issued = vec![
            Issued {
                id: "app-notepad".into(),
                key: "aumid:microsoft.windowsnotepad_8wekyb3d8bbwe!app".into(),
                name: "Notepad".into(),
            },
            Issued {
                id: "app-vs-code".into(),
                key: r"aumid:{6d809377}\microsoft vs code\code.exe".into(),
                name: "Visual Studio Code".into(),
            },
        ];
        append_app_ids(&mut doc, &issued).unwrap();
        let text = render(&doc, false);
        assert!(text.starts_with("# app-ids.toml: the stable id"));
        let back: AppIdsFile = toml::from_str(&text).unwrap();
        assert_eq!(back.app.len(), 2);
        assert_eq!(
            back.app[1].key, issued[1].key,
            "a path with backslashes survives"
        );
        // An empty list writes nothing visible: the file is only created when something is new.
        let mut again = parse(&text);
        let before = again.to_string();
        append_app_ids(&mut again, &[]).unwrap();
        assert_eq!(again.to_string(), before);
    }

    // ---- SQLite ----

    fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(include_str!("migrations/0001_create_launcher.sql"))
            .unwrap();
        connection
    }

    #[test]
    fn the_app_cache_is_replaced_as_a_whole_and_keeps_the_scan_order() {
        let mut connection = database();
        let row = |key: &str| Row {
            key: key.into(),
            aumid: "a".into(),
            name: "N".into(),
            target: String::new(),
            args: String::new(),
            target_mtime: (key == "b").then_some(42),
        };
        replace_rows(&mut connection, &[row("z"), row("b"), row("a")], "t").unwrap();
        let keys: Vec<String> = load_rows(&connection)
            .unwrap()
            .into_iter()
            .map(|r| r.key)
            .collect();
        assert_eq!(keys, ["z", "b", "a"]);
        replace_rows(&mut connection, &[row("only")], "t2").unwrap();
        let rows = load_rows(&connection).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target_mtime, None);
    }

    #[test]
    fn history_keeps_twenty_rows_per_target_and_forgets_a_deleted_pool() {
        let mut connection = database();
        for n in 0..25 {
            let at = format!("2026-11-22T10:00:{n:02}.000Z");
            insert_history(&mut connection, &[("app", "app-a")], &at, "run").unwrap();
        }
        insert_history(
            &mut connection,
            &[("pool", "pool-p"), ("app", "app-b")],
            "2026-11-22T11:00:00.000Z",
            "run2",
        )
        .unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM launcher_launch WHERE target_id = 'app-a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, HISTORY_PER_TARGET);
        let mut newest = load_history(&connection).unwrap();
        newest.sort();
        assert_eq!(newest.len(), 3);
        assert_eq!(newest[0].2, "2026-11-22T10:00:24.000Z");

        delete_history(&connection, "pool", "pool-p").unwrap();
        assert_eq!(load_history(&connection).unwrap().len(), 2);
        prune_old_history(&connection, "2026-11-22T10:30:00.000Z").unwrap();
        assert_eq!(load_history(&connection).unwrap().len(), 1);
    }

    #[test]
    fn only_apps_and_pools_can_be_in_the_history() {
        let mut connection = database();
        let error = insert_history(&mut connection, &[("game", "g")], "t", "r");
        assert!(error.is_err(), "the table refuses other target types");
    }
}
