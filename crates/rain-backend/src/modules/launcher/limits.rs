//! The numbers of this module. None is shared with another module or crate.

use std::time::Duration;

/// A read finds the app list older than this: it answers from memory and starts a rescan. The
/// age is checked at read time; there is no timer.
pub const APPS_SCAN_MAX_AGE: Duration = Duration::from_secs(10 * 60);
/// How still a Start Menu folder must be before the config hub reports a change.
pub const START_MENU_QUIET: Duration = Duration::from_secs(2);
/// The longest `GET /apps` waits for the very first scan.
pub const FIRST_SCAN_WAIT: Duration = Duration::from_secs(10);
/// How long a snapshot of the running programs is reused.
pub const PROCESS_TTL: Duration = Duration::from_secs(2);
/// An app that Rain started counts as running for this long when no process is seen yet.
pub const START_GRACE: Duration = Duration::from_secs(8);
/// One start call of a launch step.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the check "does the program file exist" may take before the start goes on anyway.
pub const EXISTS_TIMEOUT: Duration = Duration::from_secs(3);
/// A finished run stays available to `GET /launches/{id}` for this long.
pub const RUN_KEEP: Duration = Duration::from_secs(10 * 60);
/// At most this many runs are kept; the oldest finished one goes first.
pub const RUN_MAX: usize = 64;

pub const SEARCH_MAX: usize = 50;
pub const RECENT_MAX: usize = 50;
pub const RECENT_DEFAULT: usize = 12;
pub const QUERY_MAX: usize = 200;

/// The side of an icon in pixels.
pub const ICON_PX: u32 = 64;
/// Bump to make every icon reference change.
pub const ICON_RENDER_VERSION: u32 = 1;

pub const POOL_MAX: usize = 100;
pub const POOL_MEMBERS_MAX: usize = 40;
pub const POOL_NAME_MAX: usize = 80;
pub const ARGS_MAX: usize = 2000;
pub const DELAY_MAX_MS: u32 = 60_000;
pub const DELAY_SUM_MAX_MS: u32 = 600_000;

pub const PINS_MAX: usize = 200;
/// Longest id of anything in the three files (foundation 4.3).
pub const ID_MAX: usize = 128;
/// Rows of history kept for each target, and for how many days.
pub const HISTORY_PER_TARGET: i64 = 20;
pub const HISTORY_DAYS: u64 = 365;
