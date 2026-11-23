//! What game mode needs from the other modules, in one place. Section 06 6.7.1.
//!
//! Game mode calls the other modules only through this trait. The production version forwards
//! to their `Service`s. A test puts a fake in its place (`FakeWindDown` for the two ports of jobs
//! and idle jobs, a counter for the sessions, a list of games) and so never touches a real
//! process.
//!
//! Note what is NOT here: any way to stop a Claude session. Game mode lists them and leaves them
//! alone (decisions.md section 4).

use std::sync::Arc;

use futures_util::future::BoxFuture;

use crate::core::winddown::WindDownPort;
use crate::modules::{disk, games, idle, jobs, sessions};

/// A game as game mode needs it: to name it and to know that it can start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameInfo {
    pub title: String,
    pub installed: bool,
}

pub trait Neighbours: Send + Sync {
    /// Open Claude sessions that are not runs of Rain's own jobs. Only counted.
    fn open_user_sessions(&self) -> u32;
    /// The port of the scheduled jobs.
    fn jobs(&self) -> &dyn WindDownPort;
    /// The port of the idle jobs.
    fn idle(&self) -> &dyn WindDownPort;
    /// Ends a running disk scan at once.
    fn cancel_scan(&self);
    /// `None` for an id the library does not know.
    fn game(&self, id: &str) -> Option<GameInfo>;
    /// Starts the game outside Rain's process tree. `Ok(in_job)`: true when the backend itself
    /// runs inside a job and the game may close with Rain. `Err`: the plain message for the user.
    fn launch_game(&self, id: &str) -> BoxFuture<'_, Result<bool, String>>;
}

/// The real thing: the `Service`s of the other modules.
pub struct Production {
    sessions: Arc<sessions::Service>,
    jobs: Arc<jobs::Service>,
    idle: Arc<idle::Service>,
    disk: Arc<disk::Service>,
    games: Arc<games::Service>,
}

impl Production {
    pub fn new(
        sessions: Arc<sessions::Service>,
        jobs: Arc<jobs::Service>,
        idle: Arc<idle::Service>,
        disk: Arc<disk::Service>,
        games: Arc<games::Service>,
    ) -> Self {
        Self {
            sessions,
            jobs,
            idle,
            disk,
            games,
        }
    }
}

impl Neighbours for Production {
    fn open_user_sessions(&self) -> u32 {
        self.sessions.open_user_sessions()
    }

    fn jobs(&self) -> &dyn WindDownPort {
        self.jobs.wind_down()
    }

    fn idle(&self) -> &dyn WindDownPort {
        self.idle.wind_down()
    }

    fn cancel_scan(&self) {
        self.disk.cancel_running();
    }

    fn game(&self, id: &str) -> Option<GameInfo> {
        self.games.get(id).map(|game| GameInfo {
            title: game.title,
            installed: game.installed,
        })
    }

    fn launch_game(&self, id: &str) -> BoxFuture<'_, Result<bool, String>> {
        let id = id.to_owned();
        Box::pin(async move {
            self.games
                .launch_after_game_mode(&id)
                .await
                .map(|launched| launched.in_job)
                .map_err(|e| e.body.message)
        })
    }
}
