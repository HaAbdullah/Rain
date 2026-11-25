//! Game mode for the whole PC: closing the other programs. Section 06 6.11.
//!
//! - `keep`: the built-in list of programs that are never closed (one table);
//! - `select`: which programs may be closed, as pure functions;
//! - `closing`: the order of the closing, behind the `Pc` trait;
//! - `pc`: the trait and the real PC (`platform::process`, `platform::windows`);
//! - `file`: `gamemode.toml`, the master switch and the programs the user keeps;
//! - `icons`: the media kind `proc-icon`.
//!
//! This file holds the part the service uses: the last scan, the preview for the screen, the
//! number for the checklist row and the closing itself. The scan runs only when the Game mode
//! screen asks for the list, when game mode starts, and when the closing begins.

mod closing;
mod file;
mod icons;
mod keep;
mod pc;
mod select;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rain_types::{IconRef, OtherApp, OtherApps, OtherAppsPatch};
use tokio::time::Instant;

pub use closing::{CloseTimings, Outcome, Result as CloseResult, Target};
pub use file::OthersFile;
pub use icons::ProcIcons;
pub use pc::{NoPc, Pc, Scan, SystemPc};
pub use select::{GameKeep, Rules, Selection};

use super::neighbours::Neighbours;
use crate::core::blocking;
use crate::core::error::ApiResult;
use crate::modules::jobs::file_edit;
use crate::store::hub::ConfigHub;
use crate::store::toml_file::TomlFile;

/// A scan that is younger than this answers the screen again without a new scan.
const SCAN_REUSE: Duration = Duration::from_secs(5);

/// The result of one scan, with what the screen needs to draw it.
struct Cached {
    at: Instant,
    selection: Selection,
    /// `<lower-case program path>` to the name that the launcher knows.
    names: HashMap<String, String>,
}

pub struct Others {
    file: TomlFile<OthersFile>,
    pc: Arc<dyn Pc>,
    icons: Arc<ProcIcons>,
    timings: CloseTimings,
    cache: Mutex<Option<Arc<Cached>>>,
    /// The game that the run in progress starts: a later look for the screen keeps it too.
    game: Mutex<Option<String>>,
}

impl Others {
    /// Writes `gamemode.toml` when it is missing and reads it.
    pub async fn open(
        hub: &ConfigHub,
        pc: Arc<dyn Pc>,
        icons: Arc<ProcIcons>,
        timings: CloseTimings,
    ) -> ApiResult<Self> {
        if hub.data_dir().is_dir() {
            file_edit::ensure_default_file(hub, file::FILE_NAME, file::DEFAULT_FILE).await;
        }
        Ok(Self {
            file: file::open(hub).await?,
            pc,
            icons,
            timings,
            cache: Mutex::new(None),
            game: Mutex::new(None),
        })
    }

    /// Fires when `gamemode.toml` changes (a hand edit, or Rain's own write).
    pub fn changes(&self) -> tokio::sync::watch::Receiver<Arc<OthersFile>> {
        self.file.subscribe()
    }

    pub fn enabled(&self) -> bool {
        self.file.get().close_other_apps
    }

    fn cached(&self) -> Option<Arc<Cached>> {
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Looks at the PC and keeps the answer, for `count` and the next `preview`. `game_id` is the
    /// game that game mode starts, to keep it and its folder.
    pub async fn look(&self, neighbours: &dyn Neighbours, game_id: Option<&str>) {
        *self.game.lock().unwrap_or_else(PoisonError::into_inner) = game_id.map(str::to_owned);
        self.scan(neighbours, game_id).await;
    }

    async fn scan(&self, neighbours: &dyn Neighbours, game_id: Option<&str>) -> Arc<Cached> {
        let rules = self.rules(neighbours, game_id);
        let names = neighbours.app_names();
        let pc = Arc::clone(&self.pc);
        let selection = blocking::run(move || select::select(&pc.scan(), &rules))
            .await
            .unwrap_or_default();
        self.icons
            .remember(selection.candidates.iter().filter_map(|c| c.image.clone()));
        let cached = Arc::new(Cached {
            at: Instant::now(),
            selection,
            names,
        });
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&cached));
        cached
    }

    fn rules(&self, neighbours: &dyn Neighbours, game_id: Option<&str>) -> Rules {
        let game = game_id
            .and_then(|id| neighbours.game(id))
            .map(|game| GameKeep {
                exe: game.exe_path,
                install: game.install_path,
            });
        Rules {
            windir: std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_owned()),
            self_pid: std::process::id(),
            rain_dir: std::env::current_exe()
                .ok()
                .map(|exe| select::dir_of_image(&exe.to_string_lossy())),
            session_pids: neighbours.live_claude_pids(),
            game,
            keep: self.file.get().keep.clone(),
        }
    }

    /// The list for the screen. A scan that is a few seconds old is used again.
    pub async fn preview(&self, neighbours: &dyn Neighbours) -> OtherApps {
        let cached = match self.cached() {
            Some(cached) if cached.at.elapsed() < SCAN_REUSE => cached,
            _ => {
                let game = self
                    .game
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                self.scan(neighbours, game.as_deref()).await
            }
        };
        self.preview_of(&cached)
    }

    fn preview_of(&self, cached: &Cached) -> OtherApps {
        let file = self.file.get();
        let apps: Vec<OtherApp> = cached
            .selection
            .candidates
            .iter()
            .map(|candidate| {
                let name = display_name(candidate, &cached.names);
                OtherApp {
                    id: candidate.id.clone(),
                    icon: match &candidate.image {
                        Some(image) => IconRef::image(
                            &format!("media/proc-icon/{}", icons::icon_id(image)),
                            &name,
                        ),
                        None => IconRef::letter(&name),
                    },
                    name,
                    processes: u32::try_from(candidate.procs.len()).unwrap_or(u32::MAX),
                    memory_bytes: candidate.memory_bytes,
                    has_window: candidate.has_window,
                    close: !file::keeps(&file, &candidate.id),
                }
            })
            .collect();
        let freed_bytes = if file.close_other_apps {
            apps.iter()
                .filter(|app| app.close)
                .map(|app| app.memory_bytes)
                .sum()
        } else {
            0
        };
        OtherApps {
            enabled: file.close_other_apps,
            apps,
            always_kept: cached.selection.always_kept,
            freed_bytes,
        }
    }

    /// `PATCH /game-mode/other-apps`: writes the file, answers with the list as it is now.
    pub async fn patch(
        &self,
        neighbours: &dyn Neighbours,
        patch: OtherAppsPatch,
    ) -> ApiResult<OtherApps> {
        if let Some(enabled) = patch.enabled {
            file::set_enabled(&self.file, enabled).await?;
        }
        if let Some(choice) = patch.app {
            file::set_closing(&self.file, &choice.id, choice.close).await?;
        }
        Ok(self.preview(neighbours).await)
    }

    /// How many programs the last scan found that will be closed. 0 while the switch is off or
    /// nothing was scanned. In memory only: the driver calls this often.
    pub fn count(&self) -> u32 {
        let file = self.file.get();
        if !file.close_other_apps {
            return 0;
        }
        let Some(cached) = self.cached() else {
            return 0;
        };
        let closing = cached
            .selection
            .candidates
            .iter()
            .filter(|candidate| !file::keeps(&file, &candidate.id))
            .count();
        u32::try_from(closing).unwrap_or(u32::MAX)
    }

    /// Forgets the last scan: the next start looks again.
    pub fn forget(&self) {
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = None;
        *self.game.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Looks at the PC again and closes what the user did not keep. `progress` is told how many
    /// programs are finished. Empty when the switch is off.
    pub async fn close(
        &self,
        neighbours: &dyn Neighbours,
        game_id: Option<&str>,
        progress: &(dyn Fn(u32) + Sync),
    ) -> Vec<Outcome> {
        if !self.enabled() {
            return Vec::new();
        }
        let cached = self.scan(neighbours, game_id).await;
        let file = self.file.get();
        let targets: Vec<Target> = cached
            .selection
            .candidates
            .iter()
            .filter(|candidate| !file::keeps(&file, &candidate.id))
            .map(|candidate| Target {
                id: candidate.id.clone(),
                name: display_name(candidate, &cached.names),
                procs: candidate.procs.clone(),
            })
            .collect();
        closing::close_all(&*self.pc, &targets, &self.timings, progress).await
    }
}

/// The name the launcher gives the program, else the file name without `.exe`, with a capital.
fn display_name(candidate: &select::Candidate, names: &HashMap<String, String>) -> String {
    if let Some(name) = candidate
        .image
        .as_deref()
        .and_then(|image| names.get(&select::norm(image)))
    {
        return name.clone();
    }
    let stem = keep::stem(&candidate.id);
    let mut letters = stem.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => candidate.id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use select::Candidate;

    fn candidate(id: &str, image: Option<&str>) -> Candidate {
        Candidate {
            id: id.to_owned(),
            image: image.map(str::to_owned),
            procs: Vec::new(),
            memory_bytes: 0,
            has_window: false,
            kept_by_user: false,
        }
    }

    /// Looks at the real PC and prints what game mode would close. It only reads: no window is
    /// asked and no process is touched. Run it by hand:
    /// `cargo test -p rain-backend --lib print_the_preview -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the real PC; run by hand"]
    fn print_the_preview_of_this_pc() {
        let scan = SystemPc.scan();
        let rules = Rules {
            windir: std::env::var("WINDIR").unwrap_or_default(),
            self_pid: std::process::id(),
            rain_dir: std::env::current_exe()
                .ok()
                .map(|exe| select::dir_of_image(&exe.to_string_lossy())),
            ..Rules::default()
        };
        let selection = select::select(&scan, &rules);
        println!("always kept: {}", selection.always_kept);
        for candidate in &selection.candidates {
            println!(
                "{} | {} processes | {} MB | window: {}",
                candidate.id,
                candidate.procs.len(),
                candidate.memory_bytes / 1_000_000,
                candidate.has_window
            );
        }
        let total: u64 = selection.candidates.iter().map(|c| c.memory_bytes).sum();
        println!("total: {} MB", total / 1_000_000);
    }

    #[test]
    fn the_name_comes_from_the_launcher_else_from_the_file_name() {
        let mut names = HashMap::new();
        names.insert(r"c:\apps\discord.exe".to_owned(), "Discord".to_owned());
        let known = candidate("discord.exe", Some(r"C:\Apps\Discord.exe"));
        assert_eq!(display_name(&known, &names), "Discord");
        let unknown = candidate("notes.exe", Some(r"C:\Apps\Notes.exe"));
        assert_eq!(display_name(&unknown, &names), "Notes");
        assert_eq!(display_name(&candidate("x.exe", None), &names), "X");
    }
}
