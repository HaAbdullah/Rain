//! The migration runner: how every module gets its tables. Foundation section 10.5.
//!
//! Each module has a list of migrations (`pub const MIGRATIONS: &[Migration]` in its `mod.rs`).
//! Ids start at 1 for each module and are consecutive. The table `core_migrations` records what
//! ran. At start, for every module in `MODULES` order, each migration above the recorded
//! maximum runs in its own transaction. A shipped migration is never edited: a change is a new
//! migration.
//!
//! The runner also enforces the naming rule: a module creates only tables called
//! `<module>_<thing>` and indexes called `idx_<module>_<...>`.

use std::collections::BTreeSet;

use rusqlite::{Connection, Transaction};
use tracing::info;

use super::db::{Db, DbError};

/// One step of a module's schema. `sql` comes from a file next to the module:
/// `include_str!("migrations/0001_create_history.sql")`.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub id: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

/// What stops a module from getting its schema.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// The database was written by a newer Rain: it records a migration this Rain does not know.
    /// An older Rain on a newer database refuses to start the module (reason `dbTooNew`).
    #[error(
        "the database holds migration {recorded} of this module, but this Rain knows only up to {known}"
    )]
    TooNew { recorded: u32, known: u32 },
    /// The list in the code is wrong (a gap, a repeat). A programming error.
    #[error("the migrations are not numbered 1, 2, 3 ...: {0}")]
    BadList(String),
    /// A migration created a table or index that does not belong to its module.
    #[error("migration {id} ({name}) created {object}, which is not named for its module")]
    NamingRule {
        id: u32,
        name: &'static str,
        object: String,
    },
    #[error("migration {id} ({name}) failed: {source}")]
    Failed {
        id: u32,
        name: &'static str,
        source: rusqlite::Error,
    },
    #[error("could not read or write the list of migrations: {0}")]
    Sql(#[from] rusqlite::Error),
}

impl MigrationError {
    /// The tag that the module's routes answer with.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::TooNew { .. } => "dbTooNew",
            _ => "moduleFailed",
        }
    }
}

/// Runs the pending migrations of every module on the database thread. Returns the modules that
/// could not get their schema, with the cause; the other modules are not affected.
pub async fn run_all(
    db: &Db,
    modules: Vec<(&'static str, &'static [Migration])>,
    applied_at: String,
) -> Result<Vec<(&'static str, MigrationError)>, DbError> {
    db.call(move |connection| {
        ensure_table(connection)?;
        let mut failed = Vec::new();
        for (module, migrations) in modules {
            match apply(connection, module, migrations, &applied_at) {
                Ok(ran) if ran > 0 => info!(module, ran, "database migrations applied"),
                Ok(_) => {}
                Err(e) => failed.push((module, e)),
            }
        }
        Ok(failed)
    })
    .await
}

fn ensure_table(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS core_migrations (
            module     TEXT    NOT NULL,
            id         INTEGER NOT NULL,
            name       TEXT    NOT NULL,
            applied_at TEXT    NOT NULL,
            PRIMARY KEY (module, id)
        )",
    )
}

/// Runs the migrations of one module that are above the recorded maximum. Returns how many ran.
/// `core_migrations` must exist.
pub fn apply(
    connection: &mut Connection,
    module: &str,
    migrations: &[Migration],
    applied_at: &str,
) -> Result<usize, MigrationError> {
    check_list(migrations)?;
    let known = migrations.last().map_or(0, |m| m.id);
    let recorded: u32 = connection.query_row(
        "SELECT COALESCE(MAX(id), 0) FROM core_migrations WHERE module = ?1",
        [module],
        |row| row.get(0),
    )?;
    if recorded > known {
        return Err(MigrationError::TooNew { recorded, known });
    }
    let pending = migrations.iter().filter(|m| m.id > recorded);
    let mut ran = 0;
    for migration in pending {
        // One transaction for each step: it is applied completely, with its record, or not at all.
        let transaction = connection.transaction()?;
        run_one(&transaction, module, migration, applied_at)?;
        transaction.commit()?;
        ran += 1;
    }
    Ok(ran)
}

fn check_list(migrations: &[Migration]) -> Result<(), MigrationError> {
    for (index, migration) in migrations.iter().enumerate() {
        let expected = index as u32 + 1;
        if migration.id != expected {
            return Err(MigrationError::BadList(format!(
                "{} has id {}, expected {expected}",
                migration.name, migration.id
            )));
        }
        if let Some(word) = transaction_control_word(migration.sql) {
            return Err(MigrationError::BadList(format!(
                "{} uses {word}, but the runner already wraps each migration in a transaction",
                migration.name
            )));
        }
    }
    Ok(())
}

/// The first line of `sql` that starts a statement the runner must own: ending the transaction
/// early would break "all or nothing", and a PRAGMA does not work inside a transaction. A
/// `BEGIN` and `END` are not listed because a trigger body needs them (a stray one fails or is caught after the run).
fn transaction_control_word(sql: &str) -> Option<&'static str> {
    const WORDS: [&str; 5] = ["COMMIT", "ROLLBACK", "SAVEPOINT", "RELEASE", "PRAGMA"];
    sql.lines().find_map(|line| {
        let first = line
            .trim_start()
            .split(|c: char| !c.is_ascii_alphabetic())
            .next()?;
        WORDS
            .into_iter()
            .find(|word| first.eq_ignore_ascii_case(word))
    })
}

fn run_one(
    transaction: &Transaction<'_>,
    module: &str,
    migration: &Migration,
    applied_at: &str,
) -> Result<(), MigrationError> {
    let before = schema_objects(transaction)?;
    transaction
        .execute_batch(migration.sql)
        .map_err(|source| MigrationError::Failed {
            id: migration.id,
            name: migration.name,
            source,
        })?;
    if transaction.is_autocommit() {
        // Something the line check cannot see (a COMMIT after a `;` on one line) ended it.
        return Err(MigrationError::BadList(format!(
            "{} ended the transaction of the runner",
            migration.name
        )));
    }
    let table_prefix = format!("{module}_");
    let index_prefix = format!("idx_{module}_");
    for (kind, name) in schema_objects(transaction)?.difference(&before) {
        let prefix = if kind == "index" {
            &index_prefix
        } else {
            &table_prefix
        };
        if !name.starts_with(prefix.as_str()) {
            return Err(MigrationError::NamingRule {
                id: migration.id,
                name: migration.name,
                object: format!("{kind} {name}"),
            });
        }
    }
    transaction.execute(
        "INSERT INTO core_migrations (module, id, name, applied_at) VALUES (?1, ?2, ?3, ?4)",
        (module, migration.id, migration.name, applied_at),
    )?;
    Ok(())
}

/// The tables and indexes of the database, as (kind, name). SQLite's own and the automatic
/// indexes of a primary key are left out.
fn schema_objects(connection: &Connection) -> rusqlite::Result<BTreeSet<(String, String)>> {
    let mut statement = connection.prepare(
        "SELECT type, name FROM sqlite_master
         WHERE type IN ('table', 'index') AND name NOT LIKE 'sqlite_%'",
    )?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREATE: Migration = Migration {
        id: 1,
        name: "create_history",
        sql: "CREATE TABLE launcher_launch (id INTEGER PRIMARY KEY, at TEXT NOT NULL);
              CREATE INDEX idx_launcher_launch_at ON launcher_launch (at);",
    };
    const ADD_COLUMN: Migration = Migration {
        id: 2,
        name: "add_target",
        sql: "ALTER TABLE launcher_launch ADD COLUMN target TEXT",
    };

    fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        ensure_table(&connection).unwrap();
        connection
    }

    fn recorded(connection: &Connection, module: &str) -> Vec<(u32, String)> {
        let mut statement = connection
            .prepare("SELECT id, name FROM core_migrations WHERE module = ?1 ORDER BY id")
            .unwrap();
        statement
            .query_map([module], |row| {
                Ok((row.get(0).unwrap(), row.get(1).unwrap()))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn every_migration_runs_once_in_order_and_is_recorded() {
        let mut connection = database();
        let ran = apply(&mut connection, "launcher", &[CREATE, ADD_COLUMN], "t").unwrap();

        assert_eq!(ran, 2);
        assert_eq!(
            recorded(&connection, "launcher"),
            [
                (1, "create_history".to_owned()),
                (2, "add_target".to_owned())
            ]
        );
        connection
            .execute(
                "INSERT INTO launcher_launch (at, target) VALUES ('t', 'x')",
                [],
            )
            .unwrap();
    }

    #[test]
    fn a_second_run_does_nothing() {
        let mut connection = database();
        apply(&mut connection, "launcher", &[CREATE], "t").unwrap();
        assert_eq!(
            apply(&mut connection, "launcher", &[CREATE], "t").unwrap(),
            0
        );
        assert_eq!(
            apply(&mut connection, "launcher", &[CREATE, ADD_COLUMN], "t").unwrap(),
            1
        );
    }

    #[test]
    fn modules_have_their_own_numbering() {
        let mut connection = database();
        apply(&mut connection, "launcher", &[CREATE], "t").unwrap();
        let other = Migration {
            id: 1,
            name: "create_run",
            sql: "CREATE TABLE jobs_run (id INTEGER PRIMARY KEY)",
        };
        assert_eq!(apply(&mut connection, "jobs", &[other], "t").unwrap(), 1);
        assert_eq!(recorded(&connection, "jobs").len(), 1);
    }

    #[test]
    fn a_database_from_a_newer_rain_refuses_the_module() {
        let mut connection = database();
        apply(&mut connection, "launcher", &[CREATE, ADD_COLUMN], "t").unwrap();
        let error = apply(&mut connection, "launcher", &[CREATE], "t").unwrap_err();
        assert!(matches!(
            error,
            MigrationError::TooNew {
                recorded: 2,
                known: 1
            }
        ));
        assert_eq!(error.reason(), "dbTooNew");
    }

    #[test]
    fn a_table_that_is_not_named_for_its_module_is_refused_and_rolled_back() {
        let mut connection = database();
        let stray = Migration {
            id: 1,
            name: "create_stray",
            sql: "CREATE TABLE history (id INTEGER)",
        };
        let error = apply(&mut connection, "launcher", &[stray], "t").unwrap_err();
        assert!(matches!(error, MigrationError::NamingRule { .. }));
        assert!(recorded(&connection, "launcher").is_empty());
        assert!(
            connection.prepare("SELECT * FROM history").is_err(),
            "the transaction was rolled back"
        );
    }

    #[test]
    fn an_index_must_carry_the_idx_prefix_of_its_module() {
        let mut connection = database();
        let stray = Migration {
            id: 1,
            name: "create_table",
            sql: "CREATE TABLE launcher_launch (at TEXT); CREATE INDEX launch_at ON launcher_launch (at);",
        };
        let error = apply(&mut connection, "launcher", &[stray], "t").unwrap_err();
        assert!(matches!(error, MigrationError::NamingRule { .. }));
    }

    #[test]
    fn a_failing_migration_leaves_nothing_behind() {
        let mut connection = database();
        let broken = Migration {
            id: 1,
            name: "half",
            sql: "CREATE TABLE launcher_half (id INTEGER); INSERT INTO no_such_table VALUES (1);",
        };
        let error = apply(&mut connection, "launcher", &[broken], "t").unwrap_err();
        assert!(matches!(error, MigrationError::Failed { id: 1, .. }));
        assert!(connection.prepare("SELECT * FROM launcher_half").is_err());
    }

    #[test]
    fn ids_must_start_at_one_and_have_no_gaps() {
        let mut connection = database();
        let second_only = Migration { id: 2, ..CREATE };
        assert!(matches!(
            apply(&mut connection, "launcher", &[second_only], "t"),
            Err(MigrationError::BadList(_))
        ));
    }

    #[test]
    fn a_migration_may_not_control_the_transaction_or_use_pragma() {
        let mut connection = database();
        for sql in [
            "CREATE TABLE launcher_a (id INTEGER);
COMMIT;",
            "create table launcher_a (id integer);
  rollback;",
            "PRAGMA foreign_keys = ON;",
            "SAVEPOINT x;",
        ] {
            let migration = Migration { sql, ..CREATE };
            assert!(
                matches!(
                    apply(&mut connection, "launcher", &[migration], "t"),
                    Err(MigrationError::BadList(_))
                ),
                "{sql}"
            );
        }
        assert!(connection.prepare("SELECT * FROM launcher_a").is_err());
    }

    #[test]
    fn a_commit_hidden_on_one_line_is_caught_after_the_run() {
        let mut connection = database();
        let migration = Migration {
            sql: "CREATE TABLE launcher_a (id INTEGER); COMMIT;",
            ..CREATE
        };
        assert!(matches!(
            apply(&mut connection, "launcher", &[migration], "t"),
            Err(MigrationError::BadList(_))
        ));
    }

    #[test]
    fn a_trigger_with_begin_and_end_is_allowed() {
        let mut connection = database();
        let migration = Migration {
            sql: "CREATE TABLE launcher_a (id INTEGER, n INTEGER);
CREATE TRIGGER launcher_a_n AFTER INSERT ON launcher_a
BEGIN
  UPDATE launcher_a SET n = 1 WHERE id = NEW.id;
END;",
            ..CREATE
        };
        assert_eq!(
            apply(&mut connection, "launcher", &[migration], "t").unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn run_all_reports_the_failed_module_and_still_migrates_the_others() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::new(temp.path().join("rain.db"));
        db.open(1).await.unwrap();
        let failed = run_all(
            &db,
            vec![("launcher", &[STRAY][..]), ("jobs", &[CREATE_JOBS][..])],
            "t".to_owned(),
        )
        .await
        .unwrap();

        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, "launcher");
        let ran: u32 = db
            .call(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM core_migrations WHERE module = 'jobs'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(ran, 1);
    }

    const STRAY: Migration = Migration {
        id: 1,
        name: "create_stray",
        sql: "CREATE TABLE history (id INTEGER)",
    };

    const CREATE_JOBS: Migration = Migration {
        id: 1,
        name: "create_run",
        sql: "CREATE TABLE jobs_run (id INTEGER PRIMARY KEY)",
    };
}
