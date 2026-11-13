use std::path::PathBuf;

use crate::backend::Backend;
use crate::db::Project;
use crate::error::{ApiError, AppResult};
use crate::features::identity::AuthUser;
use crate::runner;
use crate::Shared;

pub(super) fn validate_project_source(
    state: &Shared,
    user: &AuthUser,
    source_type: &str,
    source: &str,
    revision: &str,
) -> AppResult<(String, String)> {
    let source = source.trim();
    let revision = revision.trim();
    if !runner::valid_revision(revision) {
        return Err(ApiError::bad(
            "revision_may_contain_only_letters_digits_dot_slash",
        ));
    }
    match source_type {
        "path" => {
            runner::authorize_project_source(
                "path",
                source,
                &state.cfg.data_dir,
                &state.cfg.project_roots,
                user.is_admin,
            )
            .map_err(|err| project_path_error(err, user.is_admin))?;
        }
        "git" => {
            if !runner::valid_git(source) {
                return Err(ApiError::bad("the_git_address_must_be_http_s_ssh"));
            }
            if runner::git_address_has_credentials(source) {
                return Err(ApiError::bad_vars(
                    "the_git_address_contains_credentials",
                    [("field", "credential_id")],
                ));
            }
            if source.starts_with("file://") {
                runner::authorize_project_source(
                    "git",
                    source,
                    &state.cfg.data_dir,
                    &state.cfg.project_roots,
                    user.is_admin,
                )
                .map_err(|err| project_path_error(err, user.is_admin))?;
            }
        }
        _ => return Err(ApiError::bad("source_is_path_or_git")),
    }
    Ok((source.to_string(), revision.to_string()))
}

pub(super) fn project_path_error(err: runner::ProjectPathError, is_admin: bool) -> ApiError {
    match err {
        runner::ProjectPathError::NotDirectory if is_admin => {
            ApiError::bad("the_project_path_is_not_a_directory")
        }
        runner::ProjectPathError::NotDirectory | runner::ProjectPathError::Denied => {
            ApiError::bad("the_project_path_is_not_allowed")
        }
        runner::ProjectPathError::AdminsOnly => {
            ApiError::bad("path_projects_are_limited_to_administrators")
        }
    }
}

pub(super) fn validate_project_credential(
    db: &dyn Backend,
    team: &str,
    credential_id: Option<&str>,
) -> AppResult<Option<String>> {
    let Some(id) = credential_id else {
        return Ok(None);
    };
    let credential = db.credential(id)?.ok_or(ApiError::NotFound)?;
    if credential.team_id != team {
        return Err(ApiError::NotFound);
    }
    if credential.kind != "scm" {
        return Err(ApiError::bad(
            "the_credential_is_not_a_source_control_credential",
        ));
    }
    if credential.disabled {
        return Err(ApiError::bad("the_source_control_credential_is_disabled"));
    }
    Ok(Some(credential.id))
}

pub(super) fn check_team_ids(
    db: &dyn Backend,
    organization_id: &str,
    team_ids: &[String],
) -> AppResult<()> {
    for team_id in team_ids {
        let team = db
            .team_by_id(team_id)?
            .ok_or_else(|| ApiError::bad("team_is_missing"))?;
        if team.organization_id != organization_id {
            return Err(ApiError::bad(
                "assigned_teams_must_belong_to_the_project_organization",
            ));
        }
    }
    Ok(())
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
