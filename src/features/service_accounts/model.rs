use serde::{Deserialize, Serialize};

use crate::db::{self, ServiceAccount};
use crate::features::identity::NewToken;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ServiceAccountBody {
    pub(super) username: String,
    pub(super) display_name: String,
    #[serde(default)]
    pub(super) teams: Vec<ServiceAccountTeamBody>,
    /// Accepted only so a clear refusal can name the rule.
    #[serde(default)]
    pub(super) is_admin: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ServiceAccountTeamBody {
    pub(super) team_id: String,
    pub(super) role: String,
}

#[derive(Serialize)]
pub(super) struct ServiceAccountResponse {
    pub(super) id: String,
    pub(super) username: String,
    pub(super) display_name: String,
    pub(super) auth_source: String,
    pub(super) disabled: bool,
    pub(super) created_at: String,
    pub(super) created_by: Option<String>,
    pub(super) created_by_username: Option<String>,
    pub(super) team_count: i64,
}

#[derive(Serialize)]
pub(super) struct ServiceAccountDetailResponse {
    #[serde(flatten)]
    pub(super) account: ServiceAccountResponse,
    pub(super) memberships: Vec<db::UserMembership>,
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct ServiceAccountListResponse(pub(super) Vec<ServiceAccountResponse>);

#[derive(Serialize)]
pub(super) struct TokenResponse {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) prefix: String,
    pub(super) scope: String,
    pub(super) created_at: String,
    pub(super) last_used_at: Option<String>,
    pub(super) expires_at: Option<String>,
    pub(super) expired: bool,
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct TokenListResponse(pub(super) Vec<TokenResponse>);

#[derive(Serialize)]
pub(super) struct CreatedTokenResponse {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) prefix: String,
    pub(super) scope: String,
    pub(super) secret: String,
    pub(super) created_at: String,
    pub(super) last_used_at: Option<String>,
    pub(super) expires_at: Option<String>,
}

impl ServiceAccountResponse {
    pub(super) fn new(account: &ServiceAccount) -> Self {
        Self {
            id: account.id.clone(),
            username: account.username.clone(),
            display_name: account.display_name.clone(),
            auth_source: "service".to_string(),
            disabled: account.disabled,
            created_at: account.created_at.clone(),
            created_by: account.created_by.clone(),
            created_by_username: account.created_by_username.clone(),
            team_count: account.team_count,
        }
    }
}

impl ServiceAccountDetailResponse {
    pub(super) fn new(account: &ServiceAccount, memberships: Vec<db::UserMembership>) -> Self {
        Self {
            account: ServiceAccountResponse::new(account),
            memberships,
        }
    }
}

impl TokenResponse {
    pub(super) fn new(token: &db::ApiToken) -> Self {
        Self {
            id: token.id.clone(),
            name: token.name.clone(),
            prefix: token.prefix.clone(),
            scope: token.scope.clone(),
            created_at: token.created_at.clone(),
            last_used_at: token.last_used_at.clone(),
            expires_at: token.expires_at.clone(),
            expired: db::has_expired(token.expires_at.as_deref()),
        }
    }
}

impl CreatedTokenResponse {
    pub(super) fn new(token: NewToken) -> Self {
        Self {
            id: token.id,
            name: token.name,
            prefix: token.prefix,
            scope: token.scope,
            secret: token.secret,
            created_at: token.created_at,
            last_used_at: None,
            expires_at: token.expires_at,
        }
    }
}
