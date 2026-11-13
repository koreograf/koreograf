use std::convert::Infallible;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use serde::{Deserialize, Serialize};

use crate::db::{self, User};
use crate::error::ApiError;
use crate::Shared;

use super::service::{api_token_user, bearer_token, read_cookie};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer = 1,
    Operator = 2,
    Owner = 3,
}

impl Role {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "viewer" => Some(Self::Viewer),
            "operator" => Some(Self::Operator),
            "owner" => Some(Self::Owner),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Operator => "operator",
            Self::Owner => "owner",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuthUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub auth_source: String,
    pub email: Option<String>,
    pub is_admin: bool,
}

impl From<User> for AuthUser {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            auth_source: user.auth_source,
            email: user.email,
            username: user.username,
            display_name: user.display_name,
            is_admin: user.is_admin,
        }
    }
}

impl FromRequestParts<Shared> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Shared,
    ) -> Result<Self, Self::Rejection> {
        if let Some(token) = bearer_token(&parts.headers) {
            if token.is_empty() {
                return Err(ApiError::Unauthorized);
            }
            return api_token_user(state, &token)?.ok_or(ApiError::Unauthorized);
        }
        let token =
            read_cookie(&parts.headers, "koreograf_session").ok_or(ApiError::Unauthorized)?;
        let user = state
            .store()
            .user_by_session(&token)?
            .ok_or(ApiError::Unauthorized)?;
        if user.disabled {
            state.store().delete_session(&token)?;
            return Err(ApiError::Unauthorized);
        }
        Ok(user.into())
    }
}

#[derive(Debug)]
pub struct LdapIdentity {
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub groups: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginBody {
    pub(super) username: String,
    pub(super) password: String,
}

pub struct ClientPeer(pub(super) Option<SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for ClientPeer {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| info.0),
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserBody {
    pub(super) username: String,
    pub(super) password: String,
    pub(super) display_name: String,
    #[serde(default)]
    pub(super) is_admin: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MeUpdate {
    pub(super) display_name: String,
    pub(super) email: Option<String>,
    #[serde(default)]
    pub(super) password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TokenBody {
    pub(super) name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UserUpdate {
    pub(super) display_name: String,
    pub(super) is_admin: bool,
    pub(super) disabled: bool,
    #[serde(default)]
    pub(super) password: String,
}

#[derive(Serialize)]
pub(super) struct TeamResponse {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) ldap_group: Option<String>,
    pub(super) organization_id: String,
    pub(super) organization_name: String,
    pub(super) role: String,
}

#[derive(Serialize)]
pub(super) struct ProfileResponse {
    pub(super) id: String,
    pub(super) username: String,
    pub(super) display_name: String,
    pub(super) auth_source: String,
    pub(super) email: Option<String>,
    pub(super) is_admin: bool,
    pub(super) teams: Vec<TeamResponse>,
    pub(super) memberships: Vec<db::UserMembership>,
}

#[derive(Serialize)]
pub(super) struct AccountResponse {
    pub(super) id: String,
    pub(super) username: String,
    pub(super) display_name: String,
    pub(super) auth_source: String,
    pub(super) email: Option<String>,
    pub(super) is_admin: bool,
    pub(super) disabled: bool,
    pub(super) created_at: String,
    pub(super) team_count: i64,
}

#[derive(Serialize)]
pub(super) struct AccountDetailResponse {
    #[serde(flatten)]
    pub(super) account: AccountResponse,
    pub(super) memberships: Vec<db::UserMembership>,
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct AccountListResponse(pub(super) Vec<AccountResponse>);

#[derive(Serialize)]
pub(super) struct TokenResponse {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) prefix: String,
    pub(super) created_at: String,
    pub(super) last_used_at: Option<String>,
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct TokenListResponse(pub(super) Vec<TokenResponse>);

#[derive(Serialize)]
pub(super) struct CreatedTokenResponse {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) prefix: String,
    pub(super) secret: String,
    pub(super) created_at: String,
    pub(super) last_used_at: Option<String>,
}

impl ProfileResponse {
    pub(super) fn new(
        user: &AuthUser,
        teams: &[(db::Team, String)],
        memberships: &[db::UserMembership],
    ) -> Self {
        Self {
            id: user.id.clone(),
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            auth_source: user.auth_source.clone(),
            email: user.email.clone(),
            is_admin: user.is_admin,
            teams: teams
                .iter()
                .map(|(team, role)| TeamResponse {
                    id: team.id.clone(),
                    name: team.name.clone(),
                    description: team.description.clone(),
                    ldap_group: team.ldap_group.clone(),
                    organization_id: team.organization_id.clone(),
                    organization_name: team.organization_name.clone(),
                    role: role.clone(),
                })
                .collect(),
            memberships: memberships.to_vec(),
        }
    }
}

impl AccountResponse {
    pub(super) fn new(user: &User, team_count: i64) -> Self {
        Self {
            id: user.id.clone(),
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            auth_source: user.auth_source.clone(),
            email: user.email.clone(),
            is_admin: user.is_admin,
            disabled: user.disabled,
            created_at: user.created_at.clone(),
            team_count,
        }
    }
}

impl AccountDetailResponse {
    pub(super) fn new(user: &User, memberships: &[db::UserMembership]) -> Self {
        let team_count = i64::try_from(memberships.len()).unwrap_or(i64::MAX);
        Self {
            account: AccountResponse::new(user, team_count),
            memberships: memberships.to_vec(),
        }
    }
}

impl TokenResponse {
    pub(super) fn new(token: &db::ApiToken) -> Self {
        Self {
            id: token.id.clone(),
            name: token.name.clone(),
            prefix: token.prefix.clone(),
            created_at: token.created_at.clone(),
            last_used_at: token.last_used_at.clone(),
        }
    }
}
