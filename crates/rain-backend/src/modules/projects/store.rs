//! The tables of this module: which folders Rain has seen, when the user opened a project, and
//! the cache of git facts. Section 02 5.1.
//!
//! Every function takes the connection and runs on the database thread (`state.db.call`).

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params};

use super::dates::{self, STRIP_DAYS};
use super::git::HeadSig;
use super::ids::project_id;

/// A project folder that discovery found.
#[derive(Debug, Clone)]
pub struct Found {
    /// `norm_path` of the folder.
    pub path: String,
    pub key: String,
    /// The folder name.
    pub name: String,
    pub root_key: String,
    pub branch: String,
    pub sig: HeadSig,
}

/// A project that was seen before and is not found now.
#[derive(Debug, Clone)]
pub struct MissingRow {
    pub id: String,
    pub key: String,
    pub path: String,
    pub name: String,
}

/// The cached git facts of one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRow {
    pub key: String,
    /// `HeadSig::encode`.
    pub head_sig: String,
    pub branch: String,
    pub dirty: Option<u32>,
    /// When `dirty` was computed (ISO).
    pub dirty_at: Option<String>,
    pub commits: Option<Vec<u32>>,
    /// The local date (`YYYY-MM-DD`) that `commits` is right for.
    pub commits_day: Option<String>,
    /// When a computation was last tried (ISO), also when git failed.
    pub computed_at: String,
}

/// Everything one list needs from the database.
#[derive(Debug, Default)]
pub struct Snapshot {
    /// The found projects with their ids, in the order they were given.
    pub found: Vec<(Found, String)>,
    pub missing: Vec<MissingRow>,
    /// `path_key` to the time of the last open.
    pub opened: HashMap<String, String>,
    pub git: HashMap<String, GitRow>,
}

struct Known {
    id: String,
    key: String,
    path: String,
    name: String,
    root_key: String,
    last_seen_at: String,
}

fn load_known(conn: &Connection) -> rusqlite::Result<Vec<Known>> {
    let mut statement = conn
        .prepare("SELECT id, path_key, path, name, root_key, last_seen_at FROM projects_known")?;
    let rows = statement.query_map([], |row| {
        Ok(Known {
            id: row.get(0)?,
            key: row.get(1)?,
            path: row.get(2)?,
            name: row.get(3)?,
            root_key: row.get(4)?,
            last_seen_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

/// The ids that were given so far, by `path_key`. The service keeps them in memory so that the
/// parts that cannot wait for the database (search, a launch plan) give the same ids.
pub fn load_known_ids(conn: &Connection) -> rusqlite::Result<HashMap<String, String>> {
    Ok(load_known(conn)?
        .into_iter()
        .map(|known| (known.key, known.id))
        .collect())
}

/// Records what discovery found and answers with everything a list needs.
///
/// - A found project that is new gets an id (`proj-<slug>`, with the hash of its path added when
///   the plain id belongs to another folder). A known one keeps its id.
/// - A known project that is not found stays listed as missing while its root is still a root
///   and it was seen within `forget_days` days; otherwise it is forgotten.
/// - `last_seen_at` is written at most once a day, so a plain list writes nothing.
pub fn sync(
    conn: &mut Connection,
    found: &[Found],
    root_keys: &HashSet<String>,
    now_iso: &str,
    forget_days: u32,
) -> rusqlite::Result<Snapshot> {
    let tx = conn.transaction()?;
    let known = load_known(&tx)?;
    let today = now_iso.get(..10).and_then(dates::parse_date).unwrap_or(0);

    let mut by_key: HashMap<&str, &Known> = known.iter().map(|k| (k.key.as_str(), k)).collect();
    let mut taken: HashSet<String> = known.iter().map(|k| k.id.clone()).collect();

    // Forget what no root explains any more.
    for old in known.iter().filter(|k| !root_keys.contains(&k.root_key)) {
        forget(&tx, &old.key)?;
        taken.remove(&old.id);
        by_key.remove(old.key.as_str());
    }

    // New folders are given ids in the order of their keys, so the same folders always get the
    // same ids, whatever order they were found in.
    let mut order: Vec<&Found> = found.iter().collect();
    order.sort_by(|a, b| a.key.cmp(&b.key));
    let mut ids: HashMap<&str, String> = HashMap::new();
    for item in order {
        if let Some(old) = by_key.get(item.key.as_str()) {
            ids.insert(item.key.as_str(), old.id.clone());
            let seen_today = old.last_seen_at.get(..10) == now_iso.get(..10);
            if !seen_today || old.path != item.path || old.name != item.name {
                tx.execute(
                    "UPDATE projects_known SET path = ?1, name = ?2, root_key = ?3, last_seen_at = ?4 \
                     WHERE path_key = ?5",
                    params![item.path, item.name, item.root_key, now_iso, item.key],
                )?;
            }
            continue;
        }
        let id = project_id(&item.name, &item.key, &|candidate| {
            taken.contains(candidate)
        });
        tx.execute(
            "INSERT INTO projects_known (path_key, id, path, name, root_key, first_seen_at, last_seen_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![item.key, id, item.path, item.name, item.root_key, now_iso],
        )?;
        taken.insert(id.clone());
        ids.insert(item.key.as_str(), id);
    }

    let found_keys: HashSet<&str> = found.iter().map(|f| f.key.as_str()).collect();
    let mut missing = Vec::new();
    for old in known
        .iter()
        .filter(|k| root_keys.contains(&k.root_key) && !found_keys.contains(k.key.as_str()))
    {
        let seen = old
            .last_seen_at
            .get(..10)
            .and_then(dates::parse_date)
            .unwrap_or(0);
        if forget_days > 0 && today - seen <= i64::from(forget_days) {
            missing.push(MissingRow {
                id: old.id.clone(),
                key: old.key.clone(),
                path: old.path.clone(),
                name: old.name.clone(),
            });
        } else {
            forget(&tx, &old.key)?;
        }
    }

    // Rows that no known project explains.
    tx.execute(
        "DELETE FROM projects_git_cache WHERE path_key NOT IN (SELECT path_key FROM projects_known)",
        [],
    )?;
    tx.execute(
        "DELETE FROM projects_opened WHERE path_key NOT IN (SELECT path_key FROM projects_known)",
        [],
    )?;

    let snapshot = Snapshot {
        found: found
            .iter()
            .filter_map(|f| ids.get(f.key.as_str()).map(|id| (f.clone(), id.clone())))
            .collect(),
        missing,
        opened: load_opened(&tx)?,
        git: load_git(&tx)?,
    };
    tx.commit()?;
    Ok(snapshot)
}

fn forget(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    for table in ["projects_known", "projects_opened", "projects_git_cache"] {
        conn.execute(&format!("DELETE FROM {table} WHERE path_key = ?1"), [key])?;
    }
    Ok(())
}

fn load_opened(conn: &Connection) -> rusqlite::Result<HashMap<String, String>> {
    let mut statement = conn.prepare("SELECT path_key, opened_at FROM projects_opened")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

fn load_git(conn: &Connection) -> rusqlite::Result<HashMap<String, GitRow>> {
    let mut statement = conn.prepare(
        "SELECT path_key, head_sig, branch, dirty, dirty_at, commits, commits_day, computed_at \
         FROM projects_git_cache",
    )?;
    let rows = statement.query_map([], |row| {
        let commits: Option<String> = row.get(5)?;
        let dirty: Option<i64> = row.get(3)?;
        Ok(GitRow {
            key: row.get(0)?,
            head_sig: row.get(1)?,
            branch: row.get(2)?,
            dirty: dirty.and_then(|n| u32::try_from(n).ok()),
            dirty_at: row.get(4)?,
            // A damaged value is the same as an unknown one.
            commits: commits
                .and_then(|text| serde_json::from_str::<Vec<u32>>(&text).ok())
                .filter(|counts| counts.len() == STRIP_DAYS),
            commits_day: row.get(6)?,
            computed_at: row.get(7)?,
        })
    })?;
    rows.map(|row| row.map(|git| (git.key.clone(), git)))
        .collect()
}

/// Writes or replaces the cached facts of some projects.
pub fn save_git_rows(conn: &mut Connection, rows: &[GitRow]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    for row in rows {
        let commits = row
            .commits
            .as_ref()
            .and_then(|counts| serde_json::to_string(counts).ok());
        tx.execute(
            "INSERT INTO projects_git_cache \
               (path_key, head_sig, branch, dirty, dirty_at, commits, commits_day, computed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(path_key) DO UPDATE SET head_sig = ?2, branch = ?3, dirty = ?4, \
               dirty_at = ?5, commits = ?6, commits_day = ?7, computed_at = ?8",
            params![
                row.key,
                row.head_sig,
                row.branch,
                row.dirty,
                row.dirty_at,
                commits,
                row.commits_day,
                row.computed_at
            ],
        )?;
    }
    tx.commit()
}

/// Makes every cached fact stale (`POST /projects/refresh`): the values stay, so they can still
/// be shown, but their age checks fail.
pub fn clear_stale(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE projects_git_cache SET dirty_at = NULL, commits_day = NULL",
        [],
    )?;
    Ok(())
}

/// The user opened this project from Rain.
pub fn record_opened(conn: &Connection, key: &str, now_iso: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO projects_opened (path_key, opened_at, opens) VALUES (?1, ?2, 1) \
         ON CONFLICT(path_key) DO UPDATE SET opened_at = ?2, opens = opens + 1",
        params![key, now_iso],
    )?;
    Ok(())
}

/// The time of the last open of one project.
#[cfg(test)]
pub fn opened_at(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT opened_at FROM projects_opened WHERE path_key = ?1",
        [key],
        |row| row.get(0),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::projects::MIGRATIONS;

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        for migration in MIGRATIONS {
            conn.execute_batch(migration.sql).unwrap();
        }
        conn
    }

    fn found(path: &str, root: &str) -> Found {
        let name = path.rsplit('\\').next().unwrap().to_owned();
        Found {
            path: path.to_owned(),
            key: path.to_lowercase(),
            name,
            root_key: root.to_lowercase(),
            branch: "main".to_owned(),
            sig: HeadSig::default(),
        }
    }

    fn roots(list: &[&str]) -> HashSet<String> {
        list.iter().map(|r| r.to_lowercase()).collect()
    }

    const NOW: &str = "2026-11-10T09:00:00.000Z";

    #[test]
    fn new_folders_get_ids_and_a_second_folder_with_the_same_name_gets_the_hash() {
        let mut conn = database();
        let list = [
            found("A:\\Work\\alpha", "A:\\Work"),
            found("B:\\Work\\alpha", "B:\\Work"),
        ];
        let snapshot = sync(&mut conn, &list, &roots(&["A:\\Work", "B:\\Work"]), NOW, 30).unwrap();
        let ids: HashMap<_, _> = snapshot
            .found
            .iter()
            .map(|(f, id)| (f.key.clone(), id.clone()))
            .collect();
        assert_eq!(ids["a:\\work\\alpha"], "proj-alpha");
        assert_eq!(
            ids["b:\\work\\alpha"],
            format!(
                "proj-alpha-{}",
                crate::modules::projects::ids::fnv8("b:\\work\\alpha")
            )
        );
        // The same again: the ids do not move.
        let again = sync(&mut conn, &list, &roots(&["A:\\Work", "B:\\Work"]), NOW, 30).unwrap();
        assert_eq!(again.found.len(), 2);
        let again_ids: HashMap<_, _> = again
            .found
            .iter()
            .map(|(f, id)| (f.key.clone(), id.clone()))
            .collect();
        assert_eq!(ids, again_ids);
    }

    #[test]
    fn a_vanished_project_is_missing_for_the_forget_time_and_then_gone() {
        let mut conn = database();
        let list = [found("A:\\Work\\alpha", "A:\\Work")];
        sync(&mut conn, &list, &roots(&["A:\\Work"]), NOW, 30).unwrap();

        let later = "2026-11-20T09:00:00.000Z";
        let snapshot = sync(&mut conn, &[], &roots(&["A:\\Work"]), later, 30).unwrap();
        assert_eq!(snapshot.missing.len(), 1);
        assert_eq!(snapshot.missing[0].id, "proj-alpha");

        let much_later = "2026-12-12T09:00:00.000Z"; // 32 days after the last sighting
        let snapshot = sync(&mut conn, &[], &roots(&["A:\\Work"]), much_later, 30).unwrap();
        assert!(snapshot.missing.is_empty());
        assert!(load_known_ids(&conn).unwrap().is_empty());
    }

    #[test]
    fn forget_time_zero_never_lists_missing_projects() {
        let mut conn = database();
        sync(
            &mut conn,
            &[found("A:\\Work\\alpha", "A:\\Work")],
            &roots(&["A:\\Work"]),
            NOW,
            0,
        )
        .unwrap();
        let snapshot = sync(&mut conn, &[], &roots(&["A:\\Work"]), NOW, 0).unwrap();
        assert!(snapshot.missing.is_empty());
    }

    #[test]
    fn a_project_of_a_folder_that_is_no_root_any_more_is_forgotten_with_its_rows() {
        let mut conn = database();
        let list = [found("A:\\Work\\alpha", "A:\\Work")];
        sync(&mut conn, &list, &roots(&["A:\\Work"]), NOW, 30).unwrap();
        record_opened(&conn, "a:\\work\\alpha", NOW).unwrap();
        let snapshot = sync(&mut conn, &[], &roots(&["Z:\\Else"]), NOW, 30).unwrap();
        assert!(snapshot.missing.is_empty());
        assert!(snapshot.opened.is_empty());
        assert_eq!(opened_at(&conn, "a:\\work\\alpha").unwrap(), None);
    }

    #[test]
    fn opens_are_counted_and_the_last_time_wins() {
        let mut conn = database();
        sync(
            &mut conn,
            &[found("A:\\Work\\alpha", "A:\\Work")],
            &roots(&["A:\\Work"]),
            NOW,
            30,
        )
        .unwrap();
        record_opened(&conn, "a:\\work\\alpha", "2026-11-10T10:00:00.000Z").unwrap();
        record_opened(&conn, "a:\\work\\alpha", "2026-11-10T11:00:00.000Z").unwrap();
        assert_eq!(
            opened_at(&conn, "a:\\work\\alpha").unwrap().as_deref(),
            Some("2026-11-10T11:00:00.000Z")
        );
        let opens: i64 = conn
            .query_row("SELECT opens FROM projects_opened", [], |r| r.get(0))
            .unwrap();
        assert_eq!(opens, 2);
    }

    #[test]
    fn git_rows_round_trip_and_stale_marks_clear_the_ages() {
        let mut conn = database();
        sync(
            &mut conn,
            &[found("A:\\Work\\alpha", "A:\\Work")],
            &roots(&["A:\\Work"]),
            NOW,
            30,
        )
        .unwrap();
        let row = GitRow {
            key: "a:\\work\\alpha".into(),
            head_sig: "h".into(),
            branch: "main".into(),
            dirty: Some(2),
            dirty_at: Some(NOW.into()),
            commits: Some(vec![1; 14]),
            commits_day: Some("2026-11-10".into()),
            computed_at: NOW.into(),
        };
        save_git_rows(&mut conn, std::slice::from_ref(&row)).unwrap();
        let snapshot = sync(
            &mut conn,
            &[found("A:\\Work\\alpha", "A:\\Work")],
            &roots(&["A:\\Work"]),
            NOW,
            30,
        )
        .unwrap();
        assert_eq!(snapshot.git["a:\\work\\alpha"], row);

        clear_stale(&conn).unwrap();
        let stale = load_git(&conn).unwrap();
        let cleared = &stale["a:\\work\\alpha"];
        assert_eq!((cleared.dirty, cleared.dirty_at.clone()), (Some(2), None));
        assert_eq!(cleared.commits_day, None);
        assert_eq!(cleared.commits, Some(vec![1; 14]));
    }

    #[test]
    fn git_rows_of_unknown_folders_are_deleted() {
        let mut conn = database();
        let orphan = GitRow {
            key: "q:\\nowhere".into(),
            head_sig: String::new(),
            branch: String::new(),
            dirty: None,
            dirty_at: None,
            commits: None,
            commits_day: None,
            computed_at: NOW.into(),
        };
        save_git_rows(&mut conn, &[orphan]).unwrap();
        let snapshot = sync(&mut conn, &[], &roots(&["A:\\Work"]), NOW, 30).unwrap();
        assert!(snapshot.git.is_empty());
    }
}
