use std::collections::HashSet;

use uuid::Uuid;

use crate::db::ServiceAccount;
use crate::error::{ApiError, AppResult};
use crate::features::activity;
use crate::features::identity::{capped_role, new_username, AuthUser, Role};
use crate::Shared;

use super::model::ServiceAccountBody;

pub(super) fn reject_service_principal(user: &AuthUser) -> AppResult<()> {
    if user.auth_source == "service" {
        Err(ApiError::Forbidden)
    } else {
        Ok(())
    }
}

pub(super) fn display_name(value: &str) -> AppResult<&str> {
    let display = value.trim();
    if display.is_empty() {
        Err(ApiError::bad("display_name_is_missing"))
    } else {
        Ok(display)
    }
}

pub(super) fn create(
    state: &Shared,
    creator: &AuthUser,
    body: ServiceAccountBody,
) -> AppResult<ServiceAccount> {
    reject_service_principal(creator)?;
    if body.is_admin {
        return Err(ApiError::bad("a_service_account_is_never_an_administrator"));
    }
    if body.teams.is_empty() {
        return Err(ApiError::bad("a_service_account_needs_at_least_one_team"));
    }
    let display = display_name(&body.display_name)?;
    let username = new_username(state.store(), &body.username)?;
    let mut seen = HashSet::new();
    let mut grants = Vec::new();
    for requested in &body.teams {
        if !seen.insert(requested.team_id.as_str()) {
            return Err(ApiError::bad("the_team_is_listed_twice"));
        }
        if state.store().team_by_id(&requested.team_id)?.is_none() {
            return Err(ApiError::bad("the_team_is_missing"));
        }
        let role = Role::parse(&requested.role)
            .ok_or_else(|| ApiError::bad("role_is_viewer_operator_or_owner"))?;
        capped_role(state.store(), creator, &requested.team_id, role)?;
        grants.push((requested.team_id.clone(), role));
    }
    let id = Uuid::new_v4().to_string();
    state
        .store()
        .insert_service_account(&id, &username, display, &creator.id)?;
    for (team_id, role) in &grants {
        state
            .store()
            .upsert_membership(team_id, &id, role.as_str(), "manual")?;
    }
    for (team_id, _) in &grants {
        activity::note(
            state,
            team_id,
            &creator.username,
            "create",
            "service_account",
            &id,
            &username,
            "success",
        );
    }
    state
        .store()
        .service_account_by_id(&id)?
        .ok_or(ApiError::NotFound)
}

/// A team owner of a team the account belongs to, or an instance
/// administrator, may read the account and manage its tokens.
pub(super) fn visible_account(
    state: &Shared,
    user: &AuthUser,
    id: &str,
) -> AppResult<ServiceAccount> {
    reject_service_principal(user)?;
    let account = state
        .store()
        .service_account_by_id(id)?
        .ok_or(ApiError::NotFound)?;
    if user.is_admin {
        return Ok(account);
    }
    for membership in state.store().list_user_memberships(id)? {
        if state
            .store()
            .role_of(&membership.team_id, &user.id)?
            .as_deref()
            == Some("owner")
        {
            return Ok(account);
        }
    }
    Err(ApiError::Forbidden)
}

pub(super) fn list(state: &Shared, user: &AuthUser) -> AppResult<Vec<ServiceAccount>> {
    reject_service_principal(user)?;
    state.store().list_service_accounts(&user.id, user.is_admin)
}

pub(super) fn delete(state: &Shared, user: &AuthUser, id: &str) -> AppResult<()> {
    let account = state
        .store()
        .service_account_by_id(id)?
        .ok_or(ApiError::NotFound)?;
    let memberships = state.store().list_user_memberships(id)?;
    if !state.store().delete_user(id)? {
        return Err(ApiError::NotFound);
    }
    for membership in memberships {
        activity::note(
            state,
            &membership.team_id,
            &user.username,
            "delete",
            "service_account",
            &account.id,
            &account.username,
            "success",
        );
    }
    Ok(())
}

/// The token create/revoke entry goes to every team the account belongs to
/// and the human can manage.
pub(super) fn note_token(
    state: &Shared,
    user: &AuthUser,
    account: &ServiceAccount,
    action: &str,
    token_id: &str,
    token_name: &str,
) -> AppResult<()> {
    for membership in state.store().list_user_memberships(&account.id)? {
        if user.is_admin
            || state
                .store()
                .role_of(&membership.team_id, &user.id)?
                .as_deref()
                == Some("owner")
        {
            activity::note(
                state,
                &membership.team_id,
                &user.username,
                action,
                "service_account_token",
                token_id,
                token_name,
                "success",
            );
        }
    }
    Ok(())
}
