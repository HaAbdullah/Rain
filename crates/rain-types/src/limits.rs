//! Every limit, timeout and size that more than one program must agree on.
//! The values and the sections that explain them are in docs/backend/00-foundation.md, Appendix A.

use std::time::Duration;

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;

// ---- API ----

/// Every route starts with this path (section 4.1).
pub const API_PREFIX: &str = "/api/v1";
/// Values in the API stay below 2^53 - 1, the largest integer a JavaScript number holds exactly.
pub const JSON_INT_MAX: u64 = (1 << 53) - 1;

// ---- the pipe (section 3) ----

/// Pipe instances waiting for a client at all times.
pub const PIPE_PENDING_INSTANCES: usize = 8;
/// Open connections at most.
pub const PIPE_MAX_CONNECTIONS: usize = 64;
/// A client that finds all instances busy retries after this, for at most the deadline.
pub const PIPE_BUSY_RETRY: Duration = Duration::from_millis(5);
pub const PIPE_BUSY_DEADLINE: Duration = Duration::from_secs(3);
/// Time a client has to send its request headers. hyper also applies it to an idle keep-alive
/// connection (it waits for the next request head), so an idle connection is closed after this.
pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest request body (some routes raise it to 4 MiB).
pub const BODY_LIMIT: usize = MIB;
/// A handler aims to finish in 10 s; the server enforces this hard limit.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Longest URL the shell accepts from the page, in bytes.
pub const URL_MAX: usize = 2048;

// ---- the clients: the shell and the tests (section 6) ----

pub const CLIENT_POOL_IDLE_PER_HOST: usize = 8;
/// Shorter than `HEADER_READ_TIMEOUT`: the client drops an idle connection before the server does,
/// so a request is not sent on a connection that is closing.
pub const CLIENT_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(8);
/// Calls from the page that may be in flight at once.
pub const CLIENT_MAX_INFLIGHT: usize = 32;
/// Timeout of a call from the page, and the most it may ask for.
pub const BRIDGE_TIMEOUT: Duration = Duration::from_secs(20);
pub const BRIDGE_TIMEOUT_MAX: Duration = Duration::from_secs(60);
/// Largest response the shell passes on.
pub const RESPONSE_MAX: usize = 16 * MIB;

// ---- the stream (section 5) ----

/// A comment line is sent this often on an idle stream, per connected client.
pub const SSE_KEEPALIVE: Duration = Duration::from_secs(25);
/// A client that read nothing for this long treats the connection as dead.
pub const STREAM_DEAD_AFTER: Duration = Duration::from_secs(60);
/// Waits between reconnect attempts. The last value repeats until a connection works.
pub const STREAM_BACKOFF: [Duration; 4] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
];
/// Frames the broadcast channel holds for a slow client before it is told to resync.
pub const BUS_CAPACITY: usize = 64;
/// Largest payload of one frame.
pub const FRAME_MAX: usize = 256 * KIB;
/// A topic that can change faster than this is throttled by its publisher (frames per second).
pub const TOPIC_RATE_MAX: u32 = 4;

// ---- lifecycle (section 2) ----

/// How long the shell waits for a new backend to answer, and how often it asks.
pub const BACKEND_READY_WAIT: Duration = Duration::from_secs(5);
pub const BACKEND_READY_POLL: Duration = Duration::from_millis(50);
/// The shell starts the backend at most this many times in this window.
pub const SPAWN_GUARD_MAX: u32 = 3;
pub const SPAWN_GUARD_WINDOW: Duration = Duration::from_secs(60);
/// How long the shell waits for an old backend to leave before it starts a new one.
pub const UPGRADE_WAIT: Duration = Duration::from_secs(5);
/// A module that does not stop within this is abandoned.
pub const MODULE_STOP_BUDGET: Duration = Duration::from_secs(5);
/// The whole shutdown; after this the process exits anyway.
pub const SHUTDOWN_BUDGET: Duration = Duration::from_secs(8);
/// After a game mode stop the shell waits this long past the closing pipe, then exits.
pub const GAME_MODE_EXIT_GRACE: Duration = Duration::from_secs(3);

// ---- storage and logs (section 10) ----

/// A TOML file is re-read this long after the last change event.
pub const TOML_DEBOUNCE: Duration = Duration::from_millis(300);
/// A TOML write is retried this often (the file may be open elsewhere), this far apart.
pub const TOML_WRITE_RETRIES: u32 = 8;
pub const TOML_WRITE_RETRY_DELAY: Duration = Duration::from_millis(25);
/// A file that is empty although it held text is looked at again after this long (an editor may be
/// half-way through saving it).
pub const TOML_TRUNCATION_SETTLE: Duration = Duration::from_millis(100);
pub const SQLITE_CACHE_KIB: u32 = 2048;
pub const LOG_FILES_KEPT: usize = 7;
pub const LOG_SIZE_BUDGET: u64 = 20 * MIB as u64;
/// Log lines waiting for the writer thread. More are dropped, on purpose.
pub const LOG_QUEUE_LINES: usize = 8192;

// ---- Claude hooks (decision C2) ----

/// How long the forwarder waits for Rain's answer to a permission request.
pub const HOOK_WAIT: Duration = Duration::from_secs(20);
/// The time written into the installed hook entry; longer than `HOOK_WAIT`.
pub const HOOK_HANDLER_TIMEOUT: Duration = Duration::from_secs(35);

// ---- work inside the backend (section 11) ----

/// A finished disk scan stays in memory this long after its last read.
pub const SCAN_ARENA_TTL: Duration = Duration::from_secs(10 * 60);
/// The disk tree in one response: at most this many nodes, this many children per folder.
pub const SCAN_TREE_BUDGET: usize = 2000;
pub const SCAN_PER_FOLDER: usize = 12;
pub const SCAN_WALK_THREADS: usize = 8;
/// A folder deeper than the tree budget shows this many children; the folder the user opened
/// shows more. Files kept per folder, and the deepest folder level that a scan walks.
pub const SCAN_OPEN_FOLDER_CHILDREN: usize = 200;
pub const SCAN_KEEP_FILES_PER_DIR: usize = 64;
pub const SCAN_MAX_DEPTH: usize = 256;
/// A scan reports its counters this often while they change; the table keeps this many scan
/// records; the walk threads run at below-normal priority while the flag is true.
pub const SCAN_PROGRESS_TICK: Duration = Duration::from_millis(250);
pub const SCAN_RECORDS_KEPT: usize = 8;
pub const SCAN_BELOW_NORMAL: bool = true;
/// `POST /paths/check`: time one path may take, paths probed at once, paths in one request, and
/// the longest path text.
pub const PATH_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
pub const PATH_PROBE_PARALLEL: usize = 4;
pub const PATHS_CHECK_MAX: usize = 32;
pub const PATH_MAX_CHARS: usize = 1024;
/// A user counts as active when the last input is younger than this (milliseconds).
pub const USER_ACTIVE_BELOW_MS: u64 = 60_000;
/// How long the backend holds a permission request open for the hook (the forwarder waits
/// `HOOK_WAIT`, which is longer, so the backend answers first).
pub const HOOK_SERVER_WAIT: Duration = Duration::from_secs(19);
/// A call on the COM thread is abandoned after this.
pub const COM_CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// `git` processes running at once.
pub const GIT_MAX_PARALLEL: usize = 4;
// ---- game mode and settings (section 06) ----

/// How long game mode waits for running work to end when the user chooses "Wait".
pub const GAME_MODE_WAIT_LIMIT: Duration = Duration::from_secs(180);
/// A review that nobody answers for this long is cancelled.
pub const REVIEW_ABANDON: Duration = Duration::from_secs(10 * 60);
/// Time one wind-down item may take, time the game launch may take, and the shortest time an
/// item stays visible on screen.
pub const GAME_MODE_ITEM_BUDGET: Duration = Duration::from_secs(5);
pub const GAME_MODE_LAUNCH_BUDGET: Duration = Duration::from_secs(10);
pub const GAME_MODE_ITEM_MIN_VISIBLE: Duration = Duration::from_millis(400);
/// Game mode records kept in the database.
pub const GAMEMODE_RECORDS_KEPT: usize = 20;
/// After a data folder change the page gets this long to refetch before the backend restarts.
pub const RESTART_DELAY: Duration = Duration::from_millis(500);
/// Detected values (vault, Steam, Epic) are looked up again when older than this.
pub const DETECT_TTL: Duration = Duration::from_secs(30);

/// Async runtime: worker threads, blocking threads at most, and how long an idle one stays.
pub const RUNTIME_WORKERS: usize = 2;
pub const BLOCKING_MAX: usize = 8;
pub const BLOCKING_KEEPALIVE: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_int_max_is_the_largest_exact_javascript_integer() {
        assert_eq!(JSON_INT_MAX, 9_007_199_254_740_991);
    }

    #[test]
    fn a_full_bus_cannot_hold_more_than_16_mib() {
        assert_eq!(BUS_CAPACITY * FRAME_MAX, 16 * MIB);
    }

    #[test]
    fn the_hook_handler_outlives_the_wait_for_rain() {
        assert!(HOOK_HANDLER_TIMEOUT > HOOK_WAIT);
    }

    #[test]
    fn the_stream_is_not_called_dead_before_two_keep_alives() {
        assert!(STREAM_DEAD_AFTER >= SSE_KEEPALIVE * 2);
    }

    #[test]
    fn the_request_limit_is_longer_than_the_handler_aim_and_shorter_than_the_bridge_timeout() {
        assert!(REQUEST_TIMEOUT > Duration::from_secs(10));
        assert!(REQUEST_TIMEOUT < BRIDGE_TIMEOUT);
    }

    #[test]
    fn the_backoff_only_grows() {
        assert!(STREAM_BACKOFF.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
