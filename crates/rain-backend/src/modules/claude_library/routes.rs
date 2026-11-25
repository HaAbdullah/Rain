//! The routes of this module:
//!
//! | Method | Path | Does |
//! |---|---|---|
//! | GET | `/claude/library` | the files of the Claude setup and its short facts |
//! | GET | `/claude/library/item?id=` | the text of one listed file (at most 512 KB) |
//! | POST | `/claude/library/open` | opens one listed file in the editor of the projects |

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use rain_types::{ClaudeLibrary, LibraryOpenBody, LibraryText};
use serde::Deserialize;

use super::scan::KnownProject;
use super::service::{self, Inputs, TextError};
use crate::core::blocking;
use crate::core::error::{ApiError, ApiJson, ApiQuery, ApiResult};
use crate::core::launch_contract::OpenRequest;
use crate::core::state::AppState;
use crate::modules::sessions::claude_paths::ClaudePaths;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/claude/library", get(list))
        .route("/claude/library/item", get(item))
        .route("/claude/library/open", post(open))
}

#[derive(Deserialize)]
struct ItemQuery {
    id: String,
}

/// What a scan needs: the Claude folder, the projects Rain knows, the profile folder name.
async fn inputs(state: &AppState) -> ApiResult<Inputs> {
    let paths = ClaudePaths::resolve(&state.config);
    let state = state.clone();
    blocking::run(move || {
        // The cheap list of the projects module: a folder scan, no git.
        let projects = state
            .projects()
            .list_cached()
            .iter()
            .filter(|project| !project.missing)
            .map(|project| KnownProject {
                name: project.name.clone(),
                path: project.path.clone().into(),
            })
            .collect();
        Inputs {
            paths,
            projects,
            home_folder_name: home_folder_name(),
        }
    })
    .await
}

fn home_folder_name() -> String {
    std::env::var_os("USERPROFILE")
        .and_then(|home| {
            std::path::Path::new(&home)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

async fn list(State(state): State<AppState>) -> ApiResult<Json<ClaudeLibrary>> {
    let inputs = inputs(&state).await?;
    blocking::run(move || service::index(&inputs))
        .await
        .map(Json)
}

fn text_problem(problem: TextError) -> ApiError {
    match problem {
        TextError::NotListed => {
            ApiError::not_found("That file is not in the Claude library").with_reason("notListed")
        }
        TextError::Unreadable => ApiError::not_found("That file could not be read. It may be gone")
            .with_reason("fileGone"),
    }
}

async fn item(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<ItemQuery>,
) -> ApiResult<Json<LibraryText>> {
    let inputs = inputs(&state).await?;
    blocking::run(move || service::text(&inputs, &query.id))
        .await?
        .map(Json)
        .map_err(text_problem)
}

async fn open(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<LibraryOpenBody>,
) -> ApiResult<StatusCode> {
    let inputs = inputs(&state).await?;
    let id = body.id;
    let path = blocking::run(move || service::path_of(&inputs, &id))
        .await?
        .ok_or_else(|| text_problem(TextError::NotListed))?;
    let Some(editor) = state.projects().editor_program() else {
        return Err(
            ApiError::failed("Rain did not find an editor to open the file with")
                .with_reason("editorMissing"),
        );
    };
    let cwd = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    state
        .opener()
        .open(OpenRequest::Spawn {
            exe: editor,
            args_raw: service::editor_args(&path),
            cwd,
        })
        .await
        .map_err(|error| ApiError::failed(error.ui_text).with_reason("editorFailed"))?;
    Ok(StatusCode::NO_CONTENT)
}
