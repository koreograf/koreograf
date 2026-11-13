use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};

use crate::auth::{AuthUser, Role};
use crate::db::Project;
use crate::error::{ApiError, AppResult};
use crate::features::inventories::{scan_project, ProjectContents};
use crate::runner;
use crate::Shared;

use super::{gate, team_header};

pub(super) fn routes() -> Router<Shared> {
    Router::new().route("/projects/{id}/contents", get(project_contents))
}

pub async fn project_contents(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ProjectContents>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let project = visible_project(&state, &id, &team)?;
    let root = project_dir(&state, &project)?;
    let contents = scan_project(&root).map_err(ApiError::bad)?;
    Ok(Json(contents))
}

pub(crate) fn visible_project(state: &Shared, id: &str, team: &str) -> AppResult<Project> {
    let db = state.store();
    if !db.project_visible(id, team)? {
        return Err(ApiError::NotFound);
    }
    db.project(id)?.ok_or(ApiError::NotFound)
}

pub(crate) fn project_dir(state: &Shared, project: &Project) -> AppResult<PathBuf> {
    match project.source_type.as_str() {
        "path" => runner::stored_directory(
            std::path::Path::new(&project.source),
            &state.cfg.data_dir,
            &state.cfg.project_roots,
        )
        .map_err(ApiError::from),
        "git" => {
            let dest = state.cfg.data_dir.join("checkouts").join(&project.id);
            if dest.join(".git").is_dir() {
                Ok(dest)
            } else {
                Err(ApiError::bad("sync_the_git_project_first"))
            }
        }
        other => Err(ApiError::bad_vars(
            "project.unknown_source",
            [("source", other)],
        )),
    }
}

pub(crate) fn read_project_file(root: &std::path::Path, rel: &str) -> AppResult<String> {
    let root = root
        .canonicalize()
        .map_err(|_| ApiError::bad("project_path_is_missing"))?;
    let full = root.join(rel);
    let full = full
        .canonicalize()
        .map_err(|_| ApiError::bad("file_is_missing_from_the_project"))?;
    if !full.starts_with(&root) {
        return Err(ApiError::bad("path_leaves_the_project"));
    }
    let meta = std::fs::metadata(&full).map_err(|err| ApiError::bad(err.to_string()))?;
    if !meta.is_file() {
        return Err(ApiError::bad("path_is_not_a_file"));
    }
    if meta.len() > 1_000_000 {
        return Err(ApiError::bad("the_inventory_file_is_larger_than_1_mb"));
    }
    std::fs::read_to_string(full).map_err(|err| ApiError::bad(err.to_string()))
}
