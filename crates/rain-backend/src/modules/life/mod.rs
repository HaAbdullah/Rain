//! Life: the Obsidian vault, habits and daily notes. Feature section 04.
//!
//! A habit is a checkbox property at the top (YAML frontmatter) of a day's daily note, for
//! example `gym: true`. Rain reads the notes, shows the habits, and writes exactly ONE thing to
//! the vault: when the user ticks a habit, that one property in that day's note (and creates a
//! bare note when the day has none). Everything that writes is in `vault_write.rs`.
//!
//! The files of this module:
//! - `dates`, `moment`, `daily`: calendar arithmetic, the Moment date format and the path of
//!   the note of a date (all pure);
//! - `frontmatter`: pure text functions that read and set one property (no disk access);
//! - `vault_write`: the ONLY code that writes inside the vault;
//! - `index`: which notes exist and which true/false properties they have (read only);
//! - `stats`: what the Life screen shows, computed from the index (pure);
//! - `store`: `habits.toml` and the two SQLite tables;
//! - `service`, `routes`: the logic, and the HTTP routes that only call it.

mod daily;
mod dates;
mod frontmatter;
mod index;
mod moment;
mod routes;
mod service;
mod stats;
mod store;
mod vault_write;

use std::time::Duration;

pub use service::Service;

use super::ModuleEntry;
use crate::store::migrations::Migration;

/// Names this module on the errors it raises about habits.
const SOURCE: &str = "habits";
/// Names the vault on the errors it raises about the vault and in `/health`.
const VAULT_SOURCE: &str = "vault";

// ---- constants (section 6.12) ----

/// The history window: this many days ending today (today included).
pub const LIFE_INDEX_DAYS: i64 = 800;
/// The length of the history strip of a list row: days, weeks and months.
pub const LIFE_LIST_DAYS: i64 = 70;
pub const LIFE_LIST_WEEKS: i64 = 12;
pub const LIFE_LIST_MONTHS: i64 = 12;
/// How many days can be ticked: today and the 30 days before it.
pub const TICK_WINDOW_DAYS: i64 = 31;
/// How many of the last days, weeks and months the completion rate looks at.
pub const RATE_DAYS: usize = 90;
pub const RATE_WEEKS: usize = 12;
pub const RATE_MONTHS: usize = 12;
/// A read looks at the files again when the last look is older than this.
pub const RESTAT_MIN_INTERVAL: Duration = Duration::from_secs(2);
/// How much of the top of a note is read to find its properties.
pub const HEAD_READ_MAX: usize = 64 * 1024;
/// The longest range `GET /habit-days` answers, in days.
pub const HABIT_DAYS_RANGE_MAX: i64 = 1200;
pub const HABITS_MAX: usize = 50;
pub const SUGGESTIONS_MAX: usize = 20;
/// Rows kept in the audit table `life_write`.
pub const WRITE_LOG_KEEP: i64 = 500;
/// How often a tick starts again when the note changed while Rain edited it.
pub const STALE_ROUNDS: u32 = 3;
/// A held note is tried this many times, this far apart, before the tick answers `noteBusy`.
pub const NOTE_RENAME_RETRIES: u32 = 8;
pub const NOTE_RENAME_DELAY: Duration = Duration::from_millis(25);
/// How long the daily notes folder must be still before its change is reported.
pub const WATCH_QUIET: Duration = Duration::from_millis(300);

/// The tables of this module: the cache of the notes and the audit trail of ticks.
pub const MIGRATIONS: &[Migration] = &[Migration {
    id: 1,
    name: "create_life_tables",
    sql: include_str!("migrations/0001_create_life_tables.sql"),
}];

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "life",
    migrations: MIGRATIONS,
    router: routes::router,
    init: |state| {
        let state = state.clone();
        Box::pin(async move {
            let service = Service::new(&state)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e.body.message))?;
            state.slots.life.set(service);
            Ok(())
        })
    },
    start: |state| {
        state.life().start(&state);
        super::ready()
    },
    stop: |state| {
        Box::pin(async move {
            state.life().stop().await;
        })
    },
};
