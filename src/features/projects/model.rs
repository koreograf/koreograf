use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::db::Project;
use crate::error::{ApiError, AppResult};
use crate::runner;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectBody {
    pub(super) name: String,
    pub(super) source_type: String,
    pub(super) source: String,
    #[serde(default)]
    pub(super) revision: String,
    pub(super) organization_id: Option<String>,
    #[serde(default)]
    pub(super) team_ids: Vec<String>,
    pub(super) credential_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectUpdateBody {
    pub(super) name: Option<String>,
    pub(super) source_type: Option<String>,
    pub(super) source: Option<String>,
    pub(super) revision: Option<String>,
    pub(super) organization_id: Option<String>,
    #[serde(default)]
    pub(super) team_ids: Option<Vec<String>>,
    pub(super) credential_id: Option<String>,
}

#[derive(Serialize)]
pub(super) struct ProjectResponse {
    pub(super) id: String,
    pub(super) team_id: String,
    pub(super) organization_id: String,
    pub(super) organization_name: String,
    pub(super) name: String,
    pub(super) source_type: String,
    pub(super) source: String,
    pub(super) revision: String,
    pub(super) credential_id: Option<String>,
    pub(super) created_at: String,
    pub(super) team_ids: Vec<String>,
    pub(super) last_sync_at: String,
    pub(super) last_sync_commit: String,
    pub(super) last_sync_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sync: Option<SyncResponse>,
}

impl ProjectResponse {
    pub(super) fn new(project: &Project, team: &str) -> Self {
        Self {
            id: project.id.clone(),
            team_id: project.team_id.clone(),
            organization_id: project.organization_id.clone(),
            organization_name: project.organization_name.clone(),
            name: project.name.clone(),
            source_type: project.source_type.clone(),
            source: runner::mask_git_url(&project.source),
            revision: project.revision.clone(),
            credential_id: if project.team_id == team {
                project.credential_id.clone()
            } else {
                None
            },
            created_at: project.created_at.clone(),
            team_ids: project.team_ids.clone(),
            last_sync_at: project.last_sync_at.clone(),
            last_sync_commit: project.last_sync_commit.clone(),
            last_sync_status: project.last_sync_status.clone(),
            sync: None,
        }
    }
}

#[derive(Serialize)]
pub(super) struct ProjectListResponse(pub(super) Vec<ProjectResponse>);

#[derive(Serialize)]
pub(super) struct SyncResponse {
    pub(super) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) vars: Option<BTreeMap<String, String>>,
}

impl SyncResponse {
    pub(super) fn from_result(result: AppResult<String>) -> Self {
        match result {
            Ok(path) => Self {
                ok: true,
                path: Some(path),
                error: None,
                vars: None,
            },
            Err(ApiError::BadRequest(message)) => Self {
                ok: false,
                path: None,
                error: Some(message.key),
                vars: Some(message.vars),
            },
            Err(err) => Self {
                ok: false,
                path: None,
                error: Some(err.to_string()),
                vars: None,
            },
        }
    }
}

#[derive(Serialize)]
pub(super) struct SyncProjectResponse {
    pub(super) path: String,
}
