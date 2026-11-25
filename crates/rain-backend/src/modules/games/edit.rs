//! What the user changes: adding a game by hand, correcting a game, hiding it, removing it.
//! Section 02, 6.12. Every change is one edit of `games.toml` (comments and order survive), then
//! the library is built again and published.

use std::path::{Path, PathBuf};

use rain_types::{CustomGameDraft, Game, GamePatch};

use super::config::{self, Custom, Field, TITLE_MAX, edit_correction, is_absolute, is_exe};
use super::covers::{self, CoverProblem};
use super::library::game_of;
use super::model::Found;
use super::names::{is_inside, norm_path, path_key, slug, unique_id};
use super::service::Service;
use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult};

fn invalid(reason: &'static str, message: impl Into<String>) -> ApiError {
    ApiError::invalid(message)
        .with_source("games")
        .with_reason(reason)
}

fn unknown_game() -> ApiError {
    ApiError::not_found("Game not found")
        .with_source("games")
        .with_reason("unknownId")
}

/// A failed write of `games.toml`: the reasons of `TomlFile` (`configBroken`, `configNewer`,
/// `configChanged`) pass through; anything else is "could not save".
fn save_error(error: ApiError) -> ApiError {
    if error.status == axum::http::StatusCode::INTERNAL_SERVER_ERROR {
        return ApiError::failed("Could not save games.toml")
            .with_source("games")
            .with_reason("saveFailed");
    }
    error.with_source("games")
}

fn cover_error(problem: CoverProblem, path: &str) -> ApiError {
    match problem {
        CoverProblem::NotFound => {
            invalid("coverNotFound", format!("Cover image not found: {path}"))
        }
        CoverProblem::NotImage => invalid(
            "coverNotImage",
            "The cover must be a PNG, JPG, WEBP or GIF image",
        ),
        CoverProblem::TooLarge => invalid("coverTooLarge", "The cover image is larger than 8 MB"),
    }
}

/// A program file the user named, checked in the order of section 6.12. `folder` is the game
/// folder of a folder game: there the path may also be relative to it.
fn check_exe(text: &str, folder: Option<&Path>) -> ApiResult<PathBuf> {
    let text = text.trim();
    if text.is_empty() {
        return Err(invalid("noExe", "A game needs a program file"));
    }
    let path = match folder {
        Some(folder) if !is_absolute(text) => folder.join(text),
        _ if is_absolute(text) => PathBuf::from(norm_path(text)),
        _ => {
            return Err(invalid(
                "exeNotAbsolute",
                "Use the full path of the program, for example D:\\Games\\Example Game\\game.exe",
            ));
        }
    };
    if !path.is_file() {
        return Err(invalid(
            "exeNotFound",
            format!("Program file not found: {}", path.display()),
        ));
    }
    if !is_exe(&path.to_string_lossy()) {
        return Err(invalid("notAnExe", "The program file must be an .exe"));
    }
    Ok(path)
}

fn check_title(text: &str) -> ApiResult<String> {
    let title = text.trim();
    if title.is_empty() {
        return Err(invalid("noName", "A game needs a name"));
    }
    if title.chars().count() > TITLE_MAX {
        return Err(invalid("noName", "The name is too long"));
    }
    Ok(title.to_owned())
}

/// The program is inside a folder game: it is that game already.
fn listed_folder_game<'a>(entries: &'a [Found], exe: &Path) -> Option<&'a Found> {
    entries.iter().find(|entry| {
        entry
            .install
            .as_ref()
            .is_some_and(|folder| is_inside(&exe.to_string_lossy(), &folder.to_string_lossy()))
    })
}

fn already_listed(found: &Found) -> ApiError {
    let hidden = if found.hidden { " (hidden)" } else { "" };
    ApiError::conflict(
        "alreadyListed",
        format!("{} is already in the library{hidden}", found.title),
    )
    .with_source("games")
}

impl Service {
    /// `POST /games`.
    pub async fn save_custom(&self, draft: CustomGameDraft) -> ApiResult<Game> {
        let title = check_title(&draft.title)?;
        let exe = check_exe(&draft.exe_path, None)?;
        let library = self.library(true).await?;
        if let Some(found) = listed_folder_game(&library.entries, &exe) {
            return Err(already_listed(found));
        }
        let exe_key = path_key(&exe.to_string_lossy());
        if let Some(found) = library.entries.iter().find(|entry| {
            entry.key.starts_with("custom:")
                && entry
                    .local
                    .as_ref()
                    .and_then(|local| local.exe.as_ref())
                    .is_some_and(|other| path_key(&other.to_string_lossy()) == exe_key)
        }) {
            return Err(already_listed(found));
        }
        // A file that would refuse the write refuses before anything is copied.
        self.file
            .writable()
            .map_err(|error| error.with_source("games"))?;
        let picked = match draft
            .cover_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            Some(path) => Some(check_cover(path).await?),
            None => None,
        };

        let id = unique_id(&format!("game-{}", slug(&title)), |candidate| {
            library.entries.iter().any(|entry| entry.id == candidate)
        });
        let app = self.app()?;
        let data_dir = app.hub.data_dir().to_path_buf();
        let cover = match picked {
            Some((bytes, extension)) => Some(import_cover(&data_dir, &id, bytes, extension).await?),
            None => None,
        };
        let custom = Custom {
            id: id.clone(),
            title,
            exe: exe.to_string_lossy().into_owned(),
            args: None,
            cover: cover.clone(),
            hidden: false,
        };
        if let Err(error) = self
            .file
            .edit(move |document| config::add_custom(document, &custom))
            .await
        {
            if let Some(cover) = cover {
                covers::remove_imported(&data_dir, &cover);
            }
            return Err(save_error(error));
        }
        self.after_change(&id).await
    }

    /// `PATCH /games/{id}`.
    pub async fn patch(&self, id: &str, patch: GamePatch) -> ApiResult<Game> {
        if patch == GamePatch::default() {
            return Err(invalid("emptyPatch", "Nothing to change"));
        }
        let library = self.library(false).await?;
        let found = library.find(id).cloned().ok_or_else(unknown_game)?;
        let local = found.local.clone();
        let editable =
            patch.title.is_some() || patch.exe_path.is_some() || patch.cover_path.is_some();
        if editable && local.is_none() {
            return Err(invalid(
                "notEditable",
                "Steam, Epic and Riot games can only be hidden",
            ));
        }
        self.file
            .writable()
            .map_err(|error| error.with_source("games"))?;

        let removable = local.as_ref().is_some_and(|local| local.removable);
        let folder = found.install.clone();
        let app = self.app()?;
        let data_dir = app.hub.data_dir().to_path_buf();
        let mut fields: Vec<Field> = Vec::new();
        let mut imported: Option<String> = None;
        let old_cover = self.cover_value(&found);

        if let Some(hidden) = patch.hidden {
            fields.push(Field::Hidden(hidden));
        }
        if let Some(text) = &patch.title {
            if text.trim().is_empty() {
                // A folder game goes back to its folder name; a hand-added game needs a name.
                if removable {
                    return Err(invalid("noName", "A game needs a name"));
                }
                fields.push(Field::Title(None));
            } else {
                fields.push(Field::Title(Some(check_title(text)?)));
            }
        }
        if let Some(text) = &patch.exe_path {
            if text.trim().is_empty() {
                if removable {
                    return Err(invalid("noExe", "A game needs a program file"));
                }
                fields.push(Field::Exe(None));
            } else {
                let exe = check_exe(text, folder.as_deref())?;
                if removable && let Some(other) = listed_folder_game(&library.entries, &exe) {
                    return Err(already_listed(other));
                }
                // A folder game keeps the path relative to its folder, as a person would write it.
                let stored = match folder
                    .as_deref()
                    .and_then(|folder| exe.strip_prefix(folder).ok())
                {
                    Some(relative) if !removable => relative.to_string_lossy().into_owned(),
                    _ => exe.to_string_lossy().into_owned(),
                };
                fields.push(Field::Exe(Some(stored)));
            }
        }
        if let Some(text) = patch.cover_path.as_deref().map(str::trim) {
            if text.is_empty() {
                fields.push(Field::Cover(None));
            } else {
                let (bytes, extension) = check_cover(text).await?;
                let value = import_cover(&data_dir, id, bytes, extension).await?;
                imported = Some(value.clone());
                fields.push(Field::Cover(Some(value)));
            }
        }

        let (key, id_text) = (found.key.clone(), found.id.clone());
        let written = self
            .file
            .edit(move |document| {
                if removable {
                    config::edit_custom(document, &id_text, &fields)
                } else {
                    edit_correction(document, &key, &fields)
                }
            })
            .await;
        if let Err(error) = written {
            if let Some(value) = imported {
                covers::remove_imported(&data_dir, &value);
            }
            return Err(save_error(error));
        }
        // The old cover file goes when nothing names it any more.
        if let Some(old) = old_cover
            && (patch.cover_path.is_some())
        {
            self.remove_cover_if_unused(&old, &data_dir);
        }
        if patch.hidden == Some(true) {
            self.unpin(id).await;
        }
        self.after_change(id).await
    }

    /// `DELETE /games/{id}`. An unknown id is fine (the game is gone, which is what was asked).
    pub async fn delete(&self, id: &str) -> ApiResult<()> {
        let library = self.library(false).await?;
        let Some(found) = library.find(id).cloned() else {
            return Ok(());
        };
        if !found.local.as_ref().is_some_and(|local| local.removable) {
            return Err(ApiError::conflict(
                "notRemovable",
                "Only games you added yourself can be removed. Hide it instead",
            )
            .with_source("games"));
        }
        let old_cover = self.cover_value(&found);
        let id_text = found.id.clone();
        self.file
            .edit(move |document| {
                config::remove_custom(document, &id_text);
                Ok(())
            })
            .await
            .map_err(save_error)?;
        let data_dir = self.app()?.hub.data_dir().to_path_buf();
        if let Some(old) = old_cover {
            self.remove_cover_if_unused(&old, &data_dir);
        }
        self.unpin(id).await;
        self.invalidate();
        self.publish().await;
        Ok(())
    }

    /// The `cover` value that `games.toml` holds for a game: only an imported file is returned,
    /// because only those are Rain's to remove.
    fn cover_value(&self, found: &Found) -> Option<String> {
        let doc = self.file.get();
        let parsed = config::parse(&doc);
        let value = if found.key.starts_with("custom:") {
            parsed
                .customs
                .iter()
                .find(|custom| custom.id == found.id)
                .and_then(|custom| custom.cover.clone())
        } else {
            config::canonical_key(&found.key)
                .and_then(|key| parsed.corrections.get(&key).and_then(|c| c.cover.clone()))
        };
        value.filter(|value| value.starts_with("media/"))
    }

    fn remove_cover_if_unused(&self, value: &str, data_dir: &Path) {
        let parsed = config::parse(&self.file.get());
        let in_use = parsed
            .customs
            .iter()
            .filter_map(|custom| custom.cover.clone())
            .chain(parsed.corrections.values().filter_map(|c| c.cover.clone()));
        if !covers::still_used(value, in_use) {
            covers::remove_imported(data_dir, value);
        }
    }

    /// After a write: scan again, tell the screens, and answer with the changed game.
    async fn after_change(&self, id: &str) -> ApiResult<Game> {
        self.invalidate();
        let library = self.library(true).await?;
        let found = library.find(id).ok_or_else(unknown_game)?;
        let live = self.live(self.running_ids(&library).await?);
        let game = game_of(found, &live);
        self.publish().await;
        Ok(game)
    }
}

async fn check_cover(path: &str) -> ApiResult<(Vec<u8>, &'static str)> {
    let owned = path.to_owned();
    blocking::run(move || covers::read_and_check(Path::new(&owned)))
        .await?
        .map_err(|problem| cover_error(problem, path))
}

async fn import_cover(
    data_dir: &Path,
    id: &str,
    bytes: Vec<u8>,
    extension: &'static str,
) -> ApiResult<String> {
    let (data_dir, id) = (data_dir.to_path_buf(), id.to_owned());
    blocking::run(move || covers::import(&data_dir, &id, &bytes, extension))
        .await?
        .map_err(|_| {
            ApiError::failed("Could not save the cover image")
                .with_source("games")
                .with_reason("saveFailed")
        })
}
