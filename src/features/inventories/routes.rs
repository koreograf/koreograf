use std::collections::{BTreeMap, BTreeSet};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::backend::Backend;
use crate::db::{self, Environment, Inventory, InventorySource};
use crate::error::{ApiError, ApiJson, AppResult};
use crate::features::identity::{AuthUser, Role};
use crate::features::inventories::{self as inventory, EffectiveVar, InventoryData, InventoryHost};
use crate::features::projects::{project_dir, read_project_file, visible_project};
use crate::http::{
    blank, check_static_ini, gate, no_content, require_description, require_name, team_header,
};
use crate::runner;
use crate::Shared;

use super::model::{NetboxResult, NetboxSource, DEFAULT_MAX_HOSTS, MAX_HOSTS_LIMIT};
use super::service::{
    classify_inventory, fetch, parse_filters, parse_hosts, preview, source_json_with_collisions,
    test_connection, valid_endpoint, valid_token,
};

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/inventories/import", post(import_inventory))
        .route("/inventories/netbox", post(create_netbox))
        .route("/inventories/netbox/test", post(test_netbox))
        .route("/inventories/netbox/preview", post(preview_netbox))
        .route("/inventories/{id}/sync", post(sync_inventory))
        .route("/inventories/{id}/sources", post(add_source))
        .route(
            "/inventories/{id}/sources/{source_id}",
            get(get_source).put(update_source).delete(delete_source),
        )
        .route(
            "/inventories/{id}/sources/{source_id}/sync",
            post(sync_source),
        )
        .route("/inventories", get(list_inventories).post(create_inventory))
        .route("/inventories/{id}/hosts", get(list_inventory_hosts))
        .route("/inventories/{id}/hosts/{name}", get(get_inventory_host))
        .route("/inventories/{id}/groups", get(list_inventory_groups))
        .route("/inventories/{id}/groups/{name}", get(get_inventory_group))
        .route(
            "/inventories/{id}",
            get(get_inventory)
                .put(update_inventory)
                .delete(delete_inventory),
        )
        .route(
            "/environments",
            get(list_environments).post(create_environment),
        )
        .route(
            "/environments/{id}",
            get(get_environment)
                .put(update_environment)
                .delete(delete_environment),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceInput {
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    api_endpoint: String,
    #[serde(default)]
    credential_id: String,
    #[serde(default)]
    token: String,
    #[serde(default = "yes")]
    validate_certs: bool,
    #[serde(default, rename = "ca_file")]
    _ca_file: String,
    #[serde(default)]
    group_by: Vec<String>,
    #[serde(default)]
    group_names_raw: bool,
    #[serde(default)]
    load_config_context: bool,
    #[serde(default = "default_max_hosts")]
    max_hosts: usize,
    #[serde(default)]
    query_filters: Vec<String>,
    #[serde(default = "yes")]
    virtual_machines: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportBody {
    project_id: String,
    path: String,
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NetboxBody {
    name: String,
    #[serde(default)]
    api_endpoint: String,
    #[serde(default)]
    credential_id: String,
    #[serde(default)]
    token: String,
    #[serde(default = "yes")]
    validate_certs: bool,
    #[serde(default, rename = "ca_file")]
    _ca_file: String,
    #[serde(default)]
    group_by: Vec<String>,
    #[serde(default)]
    group_names_raw: bool,
    #[serde(default)]
    load_config_context: bool,
    #[serde(default = "default_max_hosts")]
    max_hosts: usize,
    #[serde(default)]
    query_filters: Vec<String>,
    #[serde(default = "yes")]
    virtual_machines: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NetboxTestBody {
    #[serde(default)]
    api_endpoint: String,
    #[serde(default)]
    credential_id: String,
    #[serde(default)]
    token: String,
    #[serde(default = "yes")]
    validate_certs: bool,
    #[serde(default, rename = "ca_file")]
    _ca_file: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NetboxPreviewBody {
    #[serde(default)]
    api_endpoint: String,
    #[serde(default)]
    credential_id: String,
    #[serde(default)]
    token: String,
    #[serde(default = "yes")]
    validate_certs: bool,
    #[serde(default, rename = "ca_file")]
    _ca_file: String,
    #[serde(default)]
    group_by: Vec<String>,
    #[serde(default)]
    group_names_raw: bool,
    #[serde(default)]
    load_config_context: bool,
    #[serde(default = "default_max_hosts")]
    max_hosts: usize,
    #[serde(default)]
    query_filters: Vec<String>,
    #[serde(default = "yes")]
    virtual_machines: bool,
}

fn yes() -> bool {
    true
}

fn default_max_hosts() -> usize {
    DEFAULT_MAX_HOSTS
}

#[derive(Deserialize)]
struct StoredNetbox {
    #[serde(default = "yes")]
    validate_certs: bool,
    #[serde(default)]
    group_by: Vec<String>,
    #[serde(default)]
    group_names_raw: bool,
    #[serde(default)]
    load_config_context: bool,
    #[serde(default = "default_max_hosts")]
    max_hosts: usize,
    #[serde(default)]
    query: Vec<(String, String)>,
    #[serde(default = "yes")]
    virtual_machines: bool,
}

#[derive(Serialize)]
pub(super) struct NetboxTestResponse {
    version: String,
}

#[derive(Serialize)]
pub(super) struct NetboxPreviewResponse {
    host_count: usize,
    groups: Vec<NetboxPreviewGroupResponse>,
    sample_host: Option<NetboxPreviewHostResponse>,
    collisions: Vec<String>,
    dropped_vars: Vec<String>,
}

#[derive(Serialize)]
struct NetboxPreviewGroupResponse {
    name: String,
    host_count: usize,
}

#[derive(Serialize)]
struct NetboxPreviewHostResponse {
    name: String,
    vars: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub(super) struct SourceDetailResponse {
    id: String,
    inventory_id: String,
    name: String,
    kind: String,
    source: Value,
    credential_id: Option<String>,
    host_count: usize,
    body: String,
}

pub async fn import_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<ImportBody>,
) -> AppResult<(StatusCode, Json<InventoryDetailResponse>)> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let path = body.path.trim();
    runner::check_relative(path).map_err(ApiError::from)?;
    if path.is_empty() {
        return Err(ApiError::bad("path_is_missing"));
    }
    let project = visible_project(&state, &body.project_id, &team)?;
    let root = project_dir(&state, &project)?;
    let text = read_project_file(&root, path)?;
    let found = classify_inventory(path, &text)
        .ok_or_else(|| ApiError::bad("the_file_is_not_an_inventory"))?;
    if found.format == "plugin" {
        return Err(ApiError::bad("plugin_files_are_not_copied_as_text_add_2"));
    }
    let name = match blank(body.name) {
        Some(name) => require_name(&name)?,
        None => require_name(&file_stem(path))?,
    };
    let item = Inventory {
        id: Uuid::new_v4().to_string(),
        team_id: team,
        name,
        body: text,
        created_at: db::now(),
        source_kind: "project".into(),
        source_json: project_source_json(&project.id, path),
        credential_id: None,
        admin_saved: user.is_admin,
    };
    let source_row = db::InventorySource {
        id: Uuid::new_v4().to_string(),
        inventory_id: item.id.clone(),
        name: file_stem(path),
        kind: "project".into(),
        source_json: item.source_json.clone(),
        credential_id: None,
        body: item.body.clone(),
        position: 0,
        created_at: item.created_at.clone(),
    };
    let db = state.store();
    db.insert_inventory(&item, std::slice::from_ref(&source_row), false)?;
    Ok((StatusCode::CREATED, Json(inventory_detail(db, &item)?)))
}

pub async fn create_netbox(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<NetboxBody>,
) -> AppResult<(StatusCode, Json<InventoryDetailResponse>)> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    let access = netbox_access(
        &state,
        &team,
        &body.credential_id,
        &body.token,
        &body.api_endpoint,
    )?;
    ensure_certificate_override(&state, &user, &team, true, body.validate_certs, None)?;
    let source = netbox_source_fields(NetboxFields {
        api_endpoint: &access.api_endpoint,
        validate_certs: body.validate_certs,
        group_by: &body.group_by,
        group_names_raw: body.group_names_raw,
        load_config_context: body.load_config_context,
        max_hosts: body.max_hosts,
        query_filters: &body.query_filters,
        virtual_machines: body.virtual_machines,
    })?;
    let fetched = fetch_netbox(&state, &source, &access.token).await?;
    let inventory_id = Uuid::new_v4().to_string();
    let item = Inventory {
        id: inventory_id,
        team_id: team.clone(),
        name: name.clone(),
        body: fetched.body,
        created_at: db::now(),
        source_kind: "netbox".into(),
        source_json: source_json_with_collisions(&source, &fetched.collisions).to_string(),
        credential_id: Some(access.credential_id.clone()),
        admin_saved: user.is_admin,
    };
    let source_row = db::InventorySource {
        id: Uuid::new_v4().to_string(),
        inventory_id: item.id.clone(),
        name: "netbox".into(),
        kind: "netbox".into(),
        source_json: item.source_json.clone(),
        credential_id: Some(access.credential_id),
        body: item.body.clone(),
        position: 0,
        created_at: item.created_at.clone(),
    };
    let db = state.store();
    db.insert_inventory(&item, std::slice::from_ref(&source_row), true)?;
    let item = db.inventory(&item.id)?.ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(inventory_detail(db, &item)?)))
}

pub async fn test_netbox(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<NetboxTestBody>,
) -> AppResult<Json<NetboxTestResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let access = netbox_access(
        &state,
        &team,
        &body.credential_id,
        &body.token,
        &body.api_endpoint,
    )?;
    ensure_certificate_override(&state, &user, &team, true, body.validate_certs, None)?;
    let source = connection_source(&access.api_endpoint, &access.token, body.validate_certs)?;
    let proxy = crate::features::operations::load(&state)?.proxy;
    let version = test_connection(&source, &access.token, &proxy)
        .await
        .map_err(ApiError::bad)?;
    Ok(Json(NetboxTestResponse { version }))
}

pub async fn preview_netbox(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<NetboxPreviewBody>,
) -> AppResult<Json<NetboxPreviewResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let access = netbox_access(
        &state,
        &team,
        &body.credential_id,
        &body.token,
        &body.api_endpoint,
    )?;
    ensure_certificate_override(&state, &user, &team, true, body.validate_certs, None)?;
    let source = netbox_source_fields(NetboxFields {
        api_endpoint: &access.api_endpoint,
        validate_certs: body.validate_certs,
        group_by: &body.group_by,
        group_names_raw: body.group_names_raw,
        load_config_context: body.load_config_context,
        max_hosts: body.max_hosts,
        query_filters: &body.query_filters,
        virtual_machines: body.virtual_machines,
    })?;
    let fetched = fetch_netbox(&state, &source, &access.token).await?;
    let report = preview(&fetched.body);
    Ok(Json(NetboxPreviewResponse {
        host_count: report.host_count,
        groups: report
            .groups
            .into_iter()
            .map(|(name, host_count)| NetboxPreviewGroupResponse { name, host_count })
            .collect(),
        sample_host: report
            .sample_host
            .map(|(name, vars)| NetboxPreviewHostResponse { name, vars }),
        collisions: fetched.collisions,
        dropped_vars: fetched.dropped_vars,
    }))
}

pub async fn sync_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<InventoryDetailResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let sources = {
        let db = state.store();
        let item = db.inventory(&id)?.ok_or(ApiError::NotFound)?;
        if item.team_id != team {
            return Err(ApiError::NotFound);
        }
        db.list_inventory_sources(&id)?
    };
    let mut fresh = Vec::new();
    for source in &sources {
        match source.kind.as_str() {
            "project" => fresh.push((
                source.id.clone(),
                NetboxResult {
                    body: refresh_project(&state, &team, &source.source_json)?,
                    collisions: Vec::new(),
                    dropped_vars: Vec::new(),
                },
            )),
            "netbox" => fresh.push((
                source.id.clone(),
                refresh_netbox(&state, &source.credential_id, &team, &source.source_json).await?,
            )),
            _ => {}
        }
    }
    if fresh.is_empty() {
        return Err(ApiError::bad("this_inventory_has_no_external_source"));
    }
    let collisions: Vec<String> = fresh
        .iter()
        .flat_map(|(_, fetched)| fetched.collisions.clone())
        .collect();
    let dropped_vars: Vec<String> = fresh
        .iter()
        .flat_map(|(_, fetched)| fetched.dropped_vars.clone())
        .collect();
    let bodies: Vec<(String, String)> = fresh
        .iter()
        .map(|(source_id, fetched)| (source_id.clone(), fetched.body.clone()))
        .collect();
    let db = state.store();
    db.sync_inventory_sources(&id, &bodies, user.is_admin)?;
    let item = db.inventory(&id)?.ok_or(ApiError::NotFound)?;
    let mut response = inventory_detail(db, &item)?;
    response.collisions = Some(collisions);
    response.dropped_vars = Some(dropped_vars);
    Ok(Json(response))
}

fn project_source_json(project_id: &str, path: &str) -> String {
    #[derive(Serialize)]
    struct ProjectSource {
        project_id: String,
        path: String,
    }
    serde_json::to_string(&ProjectSource {
        project_id: project_id.to_string(),
        path: path.to_string(),
    })
    .unwrap_or_default()
}

fn project_source_paths(source_json: &str) -> AppResult<(String, String)> {
    let source = db::json_object(source_json);
    let project_id = source
        .get("project_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad("the_project_source_is_incomplete"))?;
    let path = source
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad("the_project_source_is_incomplete"))?;
    Ok((project_id.to_string(), path.to_string()))
}

fn refresh_project(state: &Shared, team: &str, source_json: &str) -> AppResult<String> {
    let (project_id, path) = project_source_paths(source_json)?;
    let project = visible_project(state, &project_id, team)?;
    let root = project_dir(state, &project)?;
    read_project_file(&root, &path)
}

fn stored_netbox(source_json: &str) -> AppResult<StoredNetbox> {
    serde_json::from_str(source_json).map_err(|_| ApiError::bad("the_netbox_source_is_incomplete"))
}

fn netbox_credential(
    state: &Shared,
    credential_id: &str,
    team_id: &str,
) -> AppResult<(String, String)> {
    let secret = credential_secret(state, credential_id, team_id)?;
    let token = token_from_value(&secret)?;
    let url = secret
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if url.is_empty() {
        return Err(ApiError::bad("netbox_needs_an_address"));
    }
    if !valid_endpoint(&url) {
        return Err(ApiError::bad("the_netbox_address_must_be_http_or_https"));
    }
    Ok((token, url))
}

async fn refresh_netbox(
    state: &Shared,
    credential_id: &Option<String>,
    team_id: &str,
    source_json: &str,
) -> AppResult<NetboxResult> {
    let stored = stored_netbox(source_json)?;
    let credential_id = credential_id
        .as_deref()
        .ok_or_else(|| ApiError::bad("netbox_credential_is_required"))?;
    let (token, url) = netbox_credential(state, credential_id, team_id)?;
    let source = NetboxSource {
        api_endpoint: url,
        validate_certs: stored.validate_certs,
        group_by: stored.group_by,
        group_names_raw: stored.group_names_raw,
        load_config_context: stored.load_config_context,
        max_hosts: stored.max_hosts,
        query: stored.query,
        virtual_machines: stored.virtual_machines,
    };
    fetch_netbox(state, &source, &token).await
}

async fn fetch_netbox(
    state: &Shared,
    source: &NetboxSource,
    token: &str,
) -> AppResult<NetboxResult> {
    let proxy = crate::features::operations::load(state)?.proxy;
    fetch(source, token, &proxy).await.map_err(ApiError::bad)
}

struct NetboxFields<'a> {
    api_endpoint: &'a str,
    validate_certs: bool,
    group_by: &'a [String],
    group_names_raw: bool,
    load_config_context: bool,
    max_hosts: usize,
    query_filters: &'a [String],
    virtual_machines: bool,
}

fn netbox_config(
    fields: &NetboxFields<'_>,
    query: Vec<(String, String)>,
) -> AppResult<NetboxSource> {
    let endpoint = fields.api_endpoint.trim().trim_end_matches('/').to_string();
    if !valid_endpoint(&endpoint) {
        return Err(ApiError::bad("the_netbox_address_must_be_http_or_https"));
    }
    if fields.max_hosts == 0 || fields.max_hosts > MAX_HOSTS_LIMIT {
        return Err(ApiError::bad_vars(
            "netbox_host_limit_is_out_of_range",
            [("max", MAX_HOSTS_LIMIT.to_string())],
        ));
    }
    Ok(NetboxSource {
        api_endpoint: endpoint,
        validate_certs: fields.validate_certs,
        group_by: fields
            .group_by
            .iter()
            .flat_map(|item| item.split(','))
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        group_names_raw: fields.group_names_raw,
        load_config_context: fields.load_config_context,
        max_hosts: fields.max_hosts,
        query,
        virtual_machines: fields.virtual_machines,
    })
}

fn connection_source(
    api_endpoint: &str,
    token: &str,
    validate_certs: bool,
) -> AppResult<NetboxSource> {
    if !valid_token(token.trim()) {
        return Err(ApiError::bad("the_netbox_token_is_invalid"));
    }
    netbox_config(
        &NetboxFields {
            api_endpoint,
            validate_certs,
            group_by: &[],
            group_names_raw: false,
            load_config_context: false,
            max_hosts: DEFAULT_MAX_HOSTS,
            query_filters: &[],
            virtual_machines: false,
        },
        Vec::new(),
    )
}

fn netbox_source_fields(fields: NetboxFields<'_>) -> AppResult<NetboxSource> {
    let query = parse_filters(fields.query_filters).map_err(ApiError::bad)?;
    netbox_config(&fields, query)
}

struct NetboxAccess {
    credential_id: String,
    token: String,
    api_endpoint: String,
}

fn netbox_access(
    state: &Shared,
    team: &str,
    credential_id: &str,
    token: &str,
    api_endpoint: &str,
) -> AppResult<NetboxAccess> {
    let credential_id = credential_id.trim();
    if credential_id.is_empty() {
        if !token.trim().is_empty() {
            return Err(ApiError::bad("netbox_token_needs_a_credential"));
        }
        return Err(ApiError::bad("netbox_credential_is_required"));
    }
    let (token, url) = netbox_credential(state, credential_id, team)?;
    let endpoint = api_endpoint.trim();
    if !endpoint.is_empty() && !endpoint_matches(endpoint, &url) {
        return Err(ApiError::bad("netbox_endpoint_must_match_credential"));
    }
    Ok(NetboxAccess {
        credential_id: credential_id.to_string(),
        token,
        api_endpoint: url,
    })
}

fn endpoint_matches(left: &str, right: &str) -> bool {
    endpoint_key(left) == endpoint_key(right)
}

fn endpoint_key(endpoint: &str) -> String {
    let text = endpoint.trim().trim_end_matches('/');
    let Some((scheme, rest)) = text.split_once("://") else {
        return text.to_ascii_lowercase();
    };
    let scheme = scheme.to_ascii_lowercase();
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let mut authority = authority.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" => ":80",
        "https" => ":443",
        _ => "",
    };
    if !default_port.is_empty() && authority.ends_with(default_port) {
        authority.truncate(authority.len() - default_port.len());
    }
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        format!("{scheme}://{authority}")
    } else {
        format!("{scheme}://{authority}/{path}")
    }
}

fn ensure_certificate_override(
    state: &Shared,
    user: &AuthUser,
    team: &str,
    credential_used: bool,
    validate_certs: bool,
    stored: Option<&StoredNetbox>,
) -> AppResult<()> {
    if !credential_used {
        return Ok(());
    }
    let stored_validate_certs = stored.is_none_or(|item| item.validate_certs);
    if validate_certs == stored_validate_certs {
        return Ok(());
    }
    gate(state, user, team, Role::Owner)
}

fn credential_secret(state: &Shared, id: &str, team: &str) -> AppResult<Value> {
    let (blob, kind, credential_team, disabled) = {
        let db = state.store();
        let item = db.credential(id)?.ok_or(ApiError::NotFound)?;
        (item.secret_blob, item.kind, item.team_id, item.disabled)
    };
    if credential_team != team {
        return Err(ApiError::NotFound);
    }
    if kind != "netbox" {
        return Err(ApiError::bad("netbox_credential_has_the_wrong_kind"));
    }
    if disabled {
        return Err(ApiError::bad("netbox_credential_is_disabled"));
    }
    let plain = state.decrypt(&blob)?;
    serde_json::from_str(&plain).map_err(|_| ApiError::bad("the_netbox_credential_is_damaged"))
}

fn token_from_value(value: &Value) -> AppResult<String> {
    let token = value.get("token").and_then(Value::as_str).unwrap_or("");
    if !valid_token(token) {
        return Err(ApiError::bad("the_netbox_token_is_invalid"));
    }
    Ok(token.to_string())
}

const MAX_SOURCES: usize = 8;

struct ResolvedSource {
    name: String,
    kind: String,
    source_json: String,
    body: String,
    credential_id: Option<String>,
}

pub(crate) async fn create_with_sources(
    state: Shared,
    team: String,
    name: String,
    inputs: Vec<SourceInput>,
    admin_saved: bool,
    user: &AuthUser,
) -> AppResult<(StatusCode, Json<InventoryDetailResponse>)> {
    if inputs.len() > MAX_SOURCES {
        return Err(ApiError::bad("at_most_8_sources"));
    }
    let mut resolved = Vec::with_capacity(inputs.len());
    for input in &inputs {
        resolved.push(resolve_source(&state, &team, input, user).await?);
    }
    let inventory_id = Uuid::new_v4().to_string();
    let created_at = db::now();
    let item = Inventory {
        id: inventory_id.clone(),
        team_id: team.clone(),
        name,
        body: String::new(),
        created_at: created_at.clone(),
        source_kind: "ini".into(),
        source_json: "{}".into(),
        credential_id: None,
        admin_saved,
    };
    let sources: Vec<db::InventorySource> = resolved
        .iter()
        .enumerate()
        .map(|(position, source)| db::InventorySource {
            id: Uuid::new_v4().to_string(),
            inventory_id: inventory_id.clone(),
            name: source.name.clone(),
            kind: source.kind.clone(),
            source_json: source.source_json.clone(),
            credential_id: source.credential_id.clone(),
            body: source.body.clone(),
            position: position as i64,
            created_at: created_at.clone(),
        })
        .collect();
    let db = state.store();
    db.insert_inventory(&item, &sources, true)?;
    let item = db.inventory(&inventory_id)?.ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(inventory_detail(db, &item)?)))
}

pub async fn add_source(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<SourceInput>,
) -> AppResult<Json<InventoryDetailResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let position = {
        let db = state.store();
        let item = db.inventory(&id)?.ok_or(ApiError::NotFound)?;
        if item.team_id != team {
            return Err(ApiError::NotFound);
        }
        let count = db.list_inventory_sources(&id)?.len();
        if count >= MAX_SOURCES {
            return Err(ApiError::bad("at_most_8_sources"));
        }
        count as i64
    };
    let resolved = resolve_source(&state, &team, &body, &user).await?;
    let source = db::InventorySource {
        id: Uuid::new_v4().to_string(),
        inventory_id: id.clone(),
        name: resolved.name,
        kind: resolved.kind,
        source_json: resolved.source_json,
        credential_id: resolved.credential_id,
        body: resolved.body,
        position,
        created_at: db::now(),
    };
    let db = state.store();
    db.add_inventory_source(&id, &source, user.is_admin)?;
    let item = db.inventory(&id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_detail(db, &item)?))
}

pub async fn get_source(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((inventory_id, source_id)): Path<(String, String)>,
) -> AppResult<Json<SourceDetailResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let source = owned_source(db, &inventory_id, &source_id, &team)?;
    Ok(Json(SourceDetailResponse {
        id: source.id,
        inventory_id: source.inventory_id,
        name: source.name,
        kind: source.kind,
        source: db::json_object(&source.source_json),
        credential_id: source.credential_id,
        host_count: parse_hosts(&source.body).len(),
        body: source.body,
    }))
}

pub async fn update_source(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((inventory_id, source_id)): Path<(String, String)>,
    ApiJson(body): ApiJson<SourceUpdate>,
) -> AppResult<Json<InventoryDetailResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let (source, before) = {
        let db = state.store();
        let item = db.inventory(&inventory_id)?.ok_or(ApiError::NotFound)?;
        if item.team_id != team {
            return Err(ApiError::NotFound);
        }
        let source = db.inventory_source(&source_id)?.ok_or(ApiError::NotFound)?;
        if source.inventory_id != inventory_id {
            return Err(ApiError::NotFound);
        }
        let before = inventory_detail(db, &item)?.host_count();
        (source, before)
    };
    reject_foreign_fields(&source.kind, &body)?;
    let resolved = match source.kind.as_str() {
        "ini" => resolve_ini_update(&source, &body)?,
        "project" => resolve_project_update(&state, &team, &source, &body)?,
        "netbox" => resolve_netbox_update(&state, &team, &source, &body, &user).await?,
        _ => return Err(ApiError::bad("a_source_is_manual_project_or_netbox")),
    };
    let mut updated = source.clone();
    updated.name = resolved.name;
    updated.source_json = resolved.source_json;
    updated.credential_id = resolved
        .credential_id
        .clone()
        .or_else(|| source.credential_id.clone());
    updated.body = resolved.body;
    let db = state.store();
    db.update_inventory_source(&updated, user.is_admin)?;
    let item = db.inventory(&inventory_id)?.ok_or(ApiError::NotFound)?;
    let mut response = inventory_detail(db, &item)?;
    if source.kind == "project" || source.kind == "netbox" {
        let after = response.host_count();
        response.sync = Some(InventorySyncResponse {
            ok: true,
            before,
            after,
        });
        response.collisions = Some(resolved.collisions);
        response.dropped_vars = Some(resolved.dropped_vars);
    }
    Ok(Json(response))
}

pub async fn delete_source(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((inventory_id, source_id)): Path<(String, String)>,
) -> AppResult<Json<InventoryDetailResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.store();
    owned_source(db, &inventory_id, &source_id, &team)?;
    db.delete_inventory_source(&source_id, &inventory_id, user.is_admin)?;
    let item = db.inventory(&inventory_id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_detail(db, &item)?))
}

pub async fn sync_source(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((inventory_id, source_id)): Path<(String, String)>,
) -> AppResult<Json<InventoryDetailResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let source = {
        let db = state.store();
        owned_source(db, &inventory_id, &source_id, &team)?
    };
    let fetched = match source.kind.as_str() {
        "project" => NetboxResult {
            body: refresh_project(&state, &team, &source.source_json)?,
            collisions: Vec::new(),
            dropped_vars: Vec::new(),
        },
        "netbox" => {
            refresh_netbox(&state, &source.credential_id, &team, &source.source_json).await?
        }
        _ => return Err(ApiError::bad("this_inventory_has_no_external_source")),
    };
    let db = state.store();
    db.sync_inventory_sources(
        &inventory_id,
        &[(source_id.clone(), fetched.body)],
        user.is_admin,
    )?;
    let item = db.inventory(&inventory_id)?.ok_or(ApiError::NotFound)?;
    let mut response = inventory_detail(db, &item)?;
    response.collisions = Some(fetched.collisions);
    response.dropped_vars = Some(fetched.dropped_vars);
    Ok(Json(response))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceUpdate {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    api_endpoint: Option<String>,
    #[serde(default)]
    credential_id: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    validate_certs: Option<bool>,
    #[serde(default, rename = "ca_file")]
    _ca_file: Option<String>,
    #[serde(default)]
    group_by: Option<Vec<String>>,
    #[serde(default)]
    group_names_raw: Option<bool>,
    #[serde(default)]
    load_config_context: Option<bool>,
    #[serde(default)]
    max_hosts: Option<usize>,
    #[serde(default)]
    query_filters: Option<Vec<String>>,
    #[serde(default)]
    virtual_machines: Option<bool>,
}

struct ResolvedUpdate {
    name: String,
    source_json: String,
    body: String,
    credential_id: Option<String>,
    collisions: Vec<String>,
    dropped_vars: Vec<String>,
}

fn reject_foreign_fields(kind: &str, body: &SourceUpdate) -> AppResult<()> {
    let fields: &[(&str, bool)] = match kind {
        "ini" => &[
            ("project_id", body.project_id.is_some()),
            ("path", body.path.is_some()),
            ("api_endpoint", body.api_endpoint.is_some()),
            ("credential_id", body.credential_id.is_some()),
            ("token", body.token.is_some()),
            ("validate_certs", body.validate_certs.is_some()),
            ("ca_file", body._ca_file.is_some()),
            ("group_by", body.group_by.is_some()),
            ("group_names_raw", body.group_names_raw.is_some()),
            ("load_config_context", body.load_config_context.is_some()),
            ("max_hosts", body.max_hosts.is_some()),
            ("query_filters", body.query_filters.is_some()),
            ("virtual_machines", body.virtual_machines.is_some()),
        ],
        "project" => &[
            ("body", body.body.is_some()),
            ("api_endpoint", body.api_endpoint.is_some()),
            ("credential_id", body.credential_id.is_some()),
            ("token", body.token.is_some()),
            ("validate_certs", body.validate_certs.is_some()),
            ("ca_file", body._ca_file.is_some()),
            ("group_by", body.group_by.is_some()),
            ("group_names_raw", body.group_names_raw.is_some()),
            ("load_config_context", body.load_config_context.is_some()),
            ("max_hosts", body.max_hosts.is_some()),
            ("query_filters", body.query_filters.is_some()),
            ("virtual_machines", body.virtual_machines.is_some()),
        ],
        "netbox" => &[
            ("body", body.body.is_some()),
            ("project_id", body.project_id.is_some()),
            ("path", body.path.is_some()),
        ],
        _ => return Err(ApiError::bad("a_source_is_manual_project_or_netbox")),
    };
    if let Some(&(field, _)) = fields.iter().find(|(_, present)| *present) {
        return Err(ApiError::bad_vars(
            "json_field_is_not_supported",
            [("field", field)],
        ));
    }
    Ok(())
}

fn updated_name(provided: Option<String>, current: &str, fallback: &str) -> AppResult<String> {
    match provided {
        Some(raw) => source_label(&raw, fallback),
        None => Ok(current.to_string()),
    }
}

fn resolve_ini_update(
    source: &db::InventorySource,
    body: &SourceUpdate,
) -> AppResult<ResolvedUpdate> {
    let name = updated_name(body.name.clone(), &source.name, "manual")?;
    let text = match body.body.as_ref() {
        Some(text) => {
            if text.trim().is_empty() {
                return Err(ApiError::bad("the_manual_source_is_empty"));
            }
            check_static_ini(text)?;
            text.clone()
        }
        None => source.body.clone(),
    };
    Ok(ResolvedUpdate {
        name,
        source_json: source.source_json.clone(),
        body: text,
        credential_id: None,
        collisions: Vec::new(),
        dropped_vars: Vec::new(),
    })
}

fn resolve_project_update(
    state: &Shared,
    team: &str,
    source: &db::InventorySource,
    body: &SourceUpdate,
) -> AppResult<ResolvedUpdate> {
    let (current_project_id, current_path) = project_source_paths(&source.source_json)?;
    let project_id = body
        .project_id
        .as_deref()
        .unwrap_or(&current_project_id)
        .trim();
    let path = body.path.as_deref().unwrap_or(&current_path).trim();
    runner::check_relative(path).map_err(ApiError::from)?;
    if path.is_empty() {
        return Err(ApiError::bad("path_is_missing"));
    }
    let project = visible_project(state, project_id, team)?;
    let root = project_dir(state, &project)?;
    let text = read_project_file(&root, path)?;
    let found = classify_inventory(path, &text)
        .ok_or_else(|| ApiError::bad("the_file_is_not_an_inventory"))?;
    if found.format == "plugin" {
        return Err(ApiError::bad("plugin_files_are_not_copied_as_text_add"));
    }
    let name = updated_name(body.name.clone(), &source.name, &file_stem(path))?;
    Ok(ResolvedUpdate {
        name,
        source_json: project_source_json(&project.id, path),
        body: text,
        credential_id: None,
        collisions: Vec::new(),
        dropped_vars: Vec::new(),
    })
}

async fn resolve_netbox_update(
    state: &Shared,
    team: &str,
    source: &db::InventorySource,
    body: &SourceUpdate,
    user: &AuthUser,
) -> AppResult<ResolvedUpdate> {
    let stored = stored_netbox(&source.source_json)?;
    let credential_id = body
        .credential_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .or(source.credential_id.as_deref())
        .unwrap_or("");
    let access = netbox_access(
        state,
        team,
        credential_id,
        body.token.as_deref().unwrap_or(""),
        body.api_endpoint.as_deref().unwrap_or(""),
    )?;
    let query = match &body.query_filters {
        Some(filters) => parse_filters(filters).map_err(ApiError::bad)?,
        None => stored.query.clone(),
    };
    let validate_certs = body.validate_certs.unwrap_or(stored.validate_certs);
    let parsed = netbox_config(
        &NetboxFields {
            api_endpoint: &access.api_endpoint,
            validate_certs,
            group_by: body.group_by.as_deref().unwrap_or(&stored.group_by),
            group_names_raw: body.group_names_raw.unwrap_or(stored.group_names_raw),
            load_config_context: body
                .load_config_context
                .unwrap_or(stored.load_config_context),
            max_hosts: body.max_hosts.unwrap_or(stored.max_hosts),
            query_filters: &[],
            virtual_machines: body.virtual_machines.unwrap_or(stored.virtual_machines),
        },
        query,
    )?;
    ensure_certificate_override(state, user, team, true, validate_certs, Some(&stored))?;
    let name = updated_name(body.name.clone(), &source.name, "netbox")?;
    let fetched = fetch_netbox(state, &parsed, &access.token).await?;
    Ok(ResolvedUpdate {
        credential_id: Some(access.credential_id),
        name,
        source_json: source_json_with_collisions(&parsed, &fetched.collisions).to_string(),
        body: fetched.body,
        collisions: fetched.collisions,
        dropped_vars: fetched.dropped_vars,
    })
}

fn owned_source(
    db: &dyn Backend,
    inventory_id: &str,
    source_id: &str,
    team: &str,
) -> AppResult<db::InventorySource> {
    let item = db.inventory(inventory_id)?.ok_or(ApiError::NotFound)?;
    if item.team_id != team {
        return Err(ApiError::NotFound);
    }
    let source = db.inventory_source(source_id)?.ok_or(ApiError::NotFound)?;
    if source.inventory_id != inventory_id {
        return Err(ApiError::NotFound);
    }
    Ok(source)
}

async fn resolve_source(
    state: &Shared,
    team: &str,
    input: &SourceInput,
    user: &AuthUser,
) -> AppResult<ResolvedSource> {
    match input.kind.as_str() {
        "ini" => {
            if input.body.trim().is_empty() {
                return Err(ApiError::bad("the_manual_source_is_empty"));
            }
            check_static_ini(&input.body)?;
            Ok(ResolvedSource {
                name: source_label(&input.name, "manual")?,
                kind: "ini".into(),
                source_json: "{}".into(),
                body: input.body.clone(),
                credential_id: None,
            })
        }
        "project" => {
            let path = input.path.trim();
            runner::check_relative(path).map_err(ApiError::from)?;
            if path.is_empty() {
                return Err(ApiError::bad("path_is_missing"));
            }
            let project = visible_project(state, input.project_id.trim(), team)?;
            let root = project_dir(state, &project)?;
            let text = read_project_file(&root, path)?;
            let found = classify_inventory(path, &text)
                .ok_or_else(|| ApiError::bad("the_file_is_not_an_inventory"))?;
            if found.format == "plugin" {
                return Err(ApiError::bad("plugin_files_are_not_copied_as_text_add"));
            }
            let name = if input.name.trim().is_empty() {
                let stem = file_stem(path);
                if stem.trim().is_empty() {
                    "project".into()
                } else {
                    stem
                }
            } else {
                require_name(&input.name)?
            };
            Ok(ResolvedSource {
                name,
                kind: "project".into(),
                source_json: project_source_json(&project.id, path),
                body: text,
                credential_id: None,
            })
        }
        "netbox" => {
            let name = source_label(&input.name, "netbox")?;
            let access = netbox_access(
                state,
                team,
                &input.credential_id,
                &input.token,
                &input.api_endpoint,
            )?;
            ensure_certificate_override(state, user, team, true, input.validate_certs, None)?;
            let parsed = netbox_source_fields(NetboxFields {
                api_endpoint: &access.api_endpoint,
                validate_certs: input.validate_certs,
                group_by: &input.group_by,
                group_names_raw: input.group_names_raw,
                load_config_context: input.load_config_context,
                max_hosts: input.max_hosts,
                query_filters: &input.query_filters,
                virtual_machines: input.virtual_machines,
            })?;
            let fetched = fetch_netbox(state, &parsed, &access.token).await?;
            Ok(ResolvedSource {
                credential_id: Some(access.credential_id),
                name,
                kind: "netbox".into(),
                source_json: source_json_with_collisions(&parsed, &fetched.collisions).to_string(),
                body: fetched.body,
            })
        }
        _ => Err(ApiError::bad("a_source_is_manual_project_or_netbox")),
    }
}

fn source_label(raw: &str, fallback: &str) -> AppResult<String> {
    if raw.trim().is_empty() {
        Ok(fallback.to_string())
    } else {
        require_name(raw)
    }
}

fn file_stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| name.to_string())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryBody {
    name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    sources: Vec<crate::features::inventories::SourceInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryUpdateBody {
    name: String,
    #[serde(default)]
    body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentBody {
    pub(super) name: String,
    pub(super) image: String,
    #[serde(default)]
    pub(super) tool: String,
    #[serde(default = "default_pull")]
    pub(super) pull_policy: String,
    #[serde(default)]
    pub(super) network: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default)]
    pub(super) scope: Option<String>,
}

fn default_pull() -> String {
    "if_not_present".into()
}
#[derive(Serialize)]
pub(super) struct InventorySourceResponse {
    id: String,
    name: String,
    kind: String,
    source: Value,
    credential_id: Option<String>,
    host_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
}

impl InventorySourceResponse {
    fn summary(source: &InventorySource) -> Self {
        Self {
            id: source.id.clone(),
            name: source.name.clone(),
            kind: source.kind.clone(),
            source: db::json_object(&source.source_json),
            credential_id: source.credential_id.clone(),
            host_count: inventory::parse_hosts(&source.body).len(),
            body: None,
        }
    }

    fn detail(source: &InventorySource) -> Self {
        Self {
            body: Some(source.body.clone()),
            ..Self::summary(source)
        }
    }
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct InventoryListResponse(Vec<InventorySummaryResponse>);

#[derive(Serialize)]
pub(super) struct InventorySummaryResponse {
    id: String,
    team_id: String,
    name: String,
    created_at: String,
    source_kind: String,
    source: Value,
    credential_id: Option<String>,
    source_count: usize,
    sources: Vec<InventorySourceResponse>,
    host_count: usize,
    group_count: usize,
}

#[derive(Serialize)]
pub(crate) struct InventoryDetailResponse {
    id: String,
    team_id: String,
    name: String,
    body: String,
    created_at: String,
    source_kind: String,
    source: Value,
    credential_id: Option<String>,
    source_count: usize,
    sources: Vec<InventorySourceResponse>,
    host_count: usize,
    group_count: usize,
    hosts: Vec<InventoryHost>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sync: Option<InventorySyncResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) collisions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) dropped_vars: Option<Vec<String>>,
}

#[derive(Serialize)]
pub(crate) struct InventorySyncResponse {
    pub ok: bool,
    pub before: usize,
    pub after: usize,
}

impl InventoryDetailResponse {
    pub(crate) fn host_count(&self) -> usize {
        self.host_count
    }
}

#[derive(Serialize)]
pub(super) struct InventoryHostsResponse {
    id: String,
    total: usize,
    page: usize,
    page_size: usize,
    groups: Vec<String>,
    hosts: Vec<InventoryHost>,
}

#[derive(Serialize)]
pub(super) struct InventoryHostSourceResponse {
    id: Option<String>,
    name: String,
    kind: String,
}

#[derive(Serialize)]
pub(super) struct InventoryHostResponse {
    id: String,
    name: String,
    groups: Vec<String>,
    source: InventoryHostSourceResponse,
    vars: Vec<EffectiveVar>,
    netbox_id: Option<String>,
    netbox_url: Option<String>,
}

#[derive(Serialize)]
pub(super) struct InventoryGroupsResponse {
    id: String,
    total: usize,
    groups: Vec<InventoryGroupEntryResponse>,
}

#[derive(Serialize)]
struct InventoryGroupEntryResponse {
    name: String,
    host_count: usize,
    children: Vec<String>,
    vars: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub(super) struct InventoryGroupResponse {
    id: String,
    name: String,
    children: Vec<String>,
    parents: Vec<String>,
    vars: BTreeMap<String, String>,
    host_count: usize,
    total: usize,
    page: usize,
    page_size: usize,
    hosts: Vec<InventoryHost>,
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct EnvironmentListResponse(Vec<Environment>);

pub async fn list_inventories(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<InventoryListResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let items = db.list_inventories(&team)?;
    let mut rows = Vec::with_capacity(items.len());
    for item in &items {
        rows.push(inventory_summary(db, item)?);
    }
    Ok(Json(InventoryListResponse(rows)))
}

pub async fn get_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<InventorySummaryResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let item = visible_inventory(db, &id, &team)?;
    Ok(Json(inventory_summary(db, &item)?))
}

const HOST_PAGE_SIZE: usize = 50;
const HOST_PAGE_SIZE_MAX: usize = 200;

#[derive(Deserialize)]
pub struct HostQuery {
    #[serde(default)]
    page: Option<usize>,
    #[serde(default)]
    page_size: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    search: String,
    #[serde(default)]
    group: String,
    #[serde(default)]
    sort: String,
}

pub async fn list_inventory_hosts(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<HostQuery>,
) -> AppResult<Json<InventoryHostsResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let item = visible_inventory(db, &id, &team)?;
    let sources = db.list_inventory_sources(&item.id)?;
    let data = inventory_data(&item, &sources);
    let groups = group_names(&data);
    let needle = query.search.trim().to_lowercase();
    let group = query.group.trim().to_lowercase();
    let shown: Vec<_> = data
        .hosts
        .into_iter()
        .filter(|host| matches_host(host, &needle, &group))
        .collect();
    let page = page_hosts(shown, &query);
    Ok(Json(InventoryHostsResponse {
        id: item.id,
        total: page.total,
        page: page.page,
        page_size: page.page_size,
        groups,
        hosts: page.hosts,
    }))
}

struct HostPage {
    total: usize,
    page: usize,
    page_size: usize,
    hosts: Vec<InventoryHost>,
}

fn page_hosts(mut hosts: Vec<InventoryHost>, query: &HostQuery) -> HostPage {
    sort_hosts(&mut hosts, &query.sort);
    let total = hosts.len();
    let page_size = query
        .page_size
        .or(query.limit)
        .unwrap_or(HOST_PAGE_SIZE)
        .clamp(1, HOST_PAGE_SIZE_MAX);
    let offset = query.offset.unwrap_or_else(|| {
        query
            .page
            .unwrap_or(1)
            .saturating_sub(1)
            .saturating_mul(page_size)
    });
    let page = offset / page_size + 1;
    let hosts = hosts.into_iter().skip(offset).take(page_size).collect();
    HostPage {
        total,
        page,
        page_size,
        hosts,
    }
}

pub async fn get_inventory_host(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((id, name)): Path<(String, String)>,
) -> AppResult<Json<InventoryHostResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let item = visible_inventory(db, &id, &team)?;
    let sources = db.list_inventory_sources(&item.id)?;
    let data = inventory_data(&item, &sources);
    let Some(host) = data.hosts.iter().find(|host| host.name == name) else {
        return Err(ApiError::NotFound);
    };
    let vars = inventory::effective_vars(&data, &name);
    let netbox_id = vars
        .iter()
        .find(|item| item.name == "netbox_id")
        .map(|item| item.value.clone());
    let source = host_source(&sources, &name);
    let source_response = match source {
        Some(source) => InventoryHostSourceResponse {
            id: Some(source.id.clone()),
            name: source.name.clone(),
            kind: source.kind.clone(),
        },
        None => InventoryHostSourceResponse {
            id: None,
            name: item.name.clone(),
            kind: item.source_kind.clone(),
        },
    };
    Ok(Json(InventoryHostResponse {
        id: item.id,
        name: host.name.clone(),
        groups: inventory::host_groups(&data, &name),
        source: source_response,
        vars,
        netbox_url: netbox_url(&state, db, source, netbox_id.as_deref()),
        netbox_id,
    }))
}

pub async fn list_inventory_groups(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<InventoryGroupsResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let item = visible_inventory(db, &id, &team)?;
    let sources = db.list_inventory_sources(&item.id)?;
    let data = inventory_data(&item, &sources);
    let groups: Vec<InventoryGroupEntryResponse> = data
        .groups
        .iter()
        .map(|group| InventoryGroupEntryResponse {
            name: group.name.clone(),
            host_count: inventory::group_hosts(&data, &group.name).len(),
            children: group.children.clone(),
            vars: group.vars.clone(),
        })
        .collect();
    let total = groups.len();
    Ok(Json(InventoryGroupsResponse {
        id: item.id,
        total,
        groups,
    }))
}

pub async fn get_inventory_group(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((id, name)): Path<(String, String)>,
    Query(query): Query<HostQuery>,
) -> AppResult<Json<InventoryGroupResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let item = visible_inventory(db, &id, &team)?;
    let sources = db.list_inventory_sources(&item.id)?;
    let data = inventory_data(&item, &sources);
    let Some(group) = data.groups.iter().find(|group| group.name == name).cloned() else {
        return Err(ApiError::NotFound);
    };
    let hosts: Vec<InventoryHost> = inventory::group_hosts(&data, &name)
        .into_iter()
        .cloned()
        .collect();
    let needle = query.search.trim().to_lowercase();
    let shown: Vec<_> = hosts
        .into_iter()
        .filter(|host| matches_host(host, &needle, ""))
        .collect();
    let page = page_hosts(shown, &query);
    let parents: Vec<String> = data
        .groups
        .iter()
        .filter(|item| item.children.iter().any(|child| child == &name))
        .map(|item| item.name.clone())
        .collect();
    Ok(Json(InventoryGroupResponse {
        id: item.id,
        name: group.name,
        children: group.children,
        parents,
        vars: group.vars,
        host_count: page.total,
        total: page.total,
        page: page.page,
        page_size: page.page_size,
        hosts: page.hosts,
    }))
}

fn visible_inventory(db: &dyn Backend, id: &str, team: &str) -> AppResult<Inventory> {
    let item = db.inventory(id)?.ok_or(ApiError::NotFound)?;
    if item.team_id != team {
        return Err(ApiError::NotFound);
    }
    Ok(item)
}

fn inventory_data(item: &Inventory, sources: &[InventorySource]) -> InventoryData {
    if sources.is_empty() {
        inventory::parse_inventory(&item.body)
    } else {
        let bodies: Vec<&str> = sources.iter().map(|source| source.body.as_str()).collect();
        inventory::merge_inventory(&bodies)
    }
}

fn group_names(data: &InventoryData) -> Vec<String> {
    data.groups
        .iter()
        .map(|group| group.name.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn group_count(data: &InventoryData) -> usize {
    data.groups
        .iter()
        .filter(|group| group.name != "all" && group.name != "ungrouped")
        .filter(|group| !inventory::group_hosts(data, &group.name).is_empty())
        .count()
}

fn host_source<'a>(sources: &'a [InventorySource], name: &str) -> Option<&'a InventorySource> {
    sources.iter().rev().find(|source| {
        inventory::parse_hosts(&source.body)
            .iter()
            .any(|host| host.name == name)
    })
}

fn netbox_url(
    state: &Shared,
    db: &dyn Backend,
    source: Option<&InventorySource>,
    netbox_id: Option<&str>,
) -> Option<String> {
    let source = source.filter(|item| item.kind == "netbox")?;
    let id = netbox_id?;
    if !id.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let credential_id = source.credential_id.as_deref()?;
    let credential = db.credential(credential_id).ok()??;
    let plain = state.decrypt(&credential.secret_blob).ok()?;
    let value: Value = serde_json::from_str(&plain).ok()?;
    let endpoint = value
        .get("url")
        .and_then(Value::as_str)?
        .trim_end_matches('/');
    if endpoint.is_empty() {
        return None;
    }
    Some(format!("{endpoint}/dcim/devices/{id}/"))
}

fn matches_host(
    host: &crate::features::inventories::InventoryHost,
    needle: &str,
    group: &str,
) -> bool {
    if !group.is_empty() && !host.groups.iter().any(|item| item.to_lowercase() == group) {
        return false;
    }
    needle.is_empty()
        || host.name.to_lowercase().contains(needle)
        || host
            .groups
            .iter()
            .any(|item| item.to_lowercase().contains(needle))
        || host.vars.iter().any(|(key, value)| {
            key.to_lowercase().contains(needle) || value.to_lowercase().contains(needle)
        })
}

fn sort_hosts(hosts: &mut [crate::features::inventories::InventoryHost], sort: &str) {
    let (field, reverse) = match sort.trim().strip_prefix('-') {
        Some(field) => (field, true),
        None => (sort.trim(), false),
    };
    hosts.sort_by(|left, right| {
        let key = |host: &crate::features::inventories::InventoryHost| match field {
            "address" => host
                .vars
                .get("ansible_host")
                .cloned()
                .unwrap_or_default()
                .to_lowercase(),
            "groups" => host.groups.join(",").to_lowercase(),
            _ => host.name.to_lowercase(),
        };
        let order = key(left)
            .cmp(&key(right))
            .then_with(|| left.name.cmp(&right.name));
        if reverse {
            order.reverse()
        } else {
            order
        }
    });
}

pub async fn update_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<InventoryUpdateBody>,
) -> AppResult<Json<InventoryDetailResponse>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    let db = state.store();
    let existing = db.inventory(&id)?.ok_or(ApiError::NotFound)?;
    if existing.team_id != team {
        return Err(ApiError::NotFound);
    }
    let sources = db.list_inventory_sources(&id)?;
    if !body.body.trim().is_empty() {
        if sources.len() > 1 {
            return Err(ApiError::bad(
                "this_inventory_has_several_sources_edit_the_text",
            ));
        }
        if let Some(source) = sources.first() {
            if source.kind != "ini" {
                return Err(ApiError::bad("this_source_comes_from_a_project_or_from"));
            }
        }
    }
    check_static_ini(&body.body)?;
    db.update_inventory(&id, &team, &name, &existing.body)?;
    if !body.body.trim().is_empty() {
        if let Some(source) = sources.first() {
            db.set_inventory_source_body(&source.id, &body.body)?;
            db.refresh_inventory_summary(&id)?;
        } else {
            db.set_inventory_body(&id, &team, &body.body)?;
        }
    }
    db.set_inventory_admin_saved(&id, user.is_admin)?;
    let item = db.inventory(&id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_detail(db, &item)?))
}

pub async fn create_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<InventoryBody>,
) -> AppResult<(StatusCode, Json<InventoryDetailResponse>)> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let name = require_name(&body.name)?;
    if !body.sources.is_empty() {
        return crate::features::inventories::create_with_sources(
            state,
            team,
            name,
            body.sources,
            user.is_admin,
            &user,
        )
        .await;
    }
    check_static_ini(&body.body)?;
    let item = Inventory {
        id: Uuid::new_v4().to_string(),
        team_id: team,
        name,
        body: body.body.clone(),
        created_at: db::now(),
        source_kind: "ini".into(),
        source_json: "{}".into(),
        credential_id: None,
        admin_saved: user.is_admin,
    };
    let source = if body.body.trim().is_empty() {
        None
    } else {
        Some(db::InventorySource {
            id: Uuid::new_v4().to_string(),
            inventory_id: item.id.clone(),
            name: "manual".into(),
            kind: "ini".into(),
            source_json: "{}".into(),
            credential_id: None,
            body: body.body,
            position: 0,
            created_at: item.created_at.clone(),
        })
    };
    let sources: Vec<db::InventorySource> = source.into_iter().collect();
    let db = state.store();
    db.insert_inventory(&item, &sources, false)?;
    Ok((StatusCode::CREATED, Json(inventory_detail(db, &item)?)))
}

fn inventory_summary(c: &dyn Backend, item: &Inventory) -> AppResult<InventorySummaryResponse> {
    let sources = c.list_inventory_sources(&item.id)?;
    let data = inventory_data(item, &sources);
    Ok(InventorySummaryResponse {
        id: item.id.clone(),
        team_id: item.team_id.clone(),
        name: item.name.clone(),
        created_at: item.created_at.clone(),
        source_kind: item.source_kind.clone(),
        source: db::json_object(&item.source_json),
        credential_id: item.credential_id.clone(),
        source_count: sources.len(),
        sources: sources
            .iter()
            .map(InventorySourceResponse::summary)
            .collect(),
        host_count: data.hosts.len(),
        group_count: group_count(&data),
    })
}

pub(crate) fn inventory_detail(
    c: &dyn Backend,
    item: &Inventory,
) -> AppResult<InventoryDetailResponse> {
    let sources = c.list_inventory_sources(&item.id)?;
    let data = inventory_data(item, &sources);
    Ok(InventoryDetailResponse {
        id: item.id.clone(),
        team_id: item.team_id.clone(),
        name: item.name.clone(),
        body: item.body.clone(),
        created_at: item.created_at.clone(),
        source_kind: item.source_kind.clone(),
        source: db::json_object(&item.source_json),
        credential_id: item.credential_id.clone(),
        source_count: sources.len(),
        sources: sources
            .iter()
            .map(InventorySourceResponse::detail)
            .collect(),
        host_count: data.hosts.len(),
        group_count: group_count(&data),
        hosts: data.hosts,
        sync: None,
        collisions: None,
        dropped_vars: None,
    })
}

pub async fn delete_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.store();
    let mut names = db.templates_using_inventory(&id, &team)?;
    names.extend(db.workflows_using_inventory(&id, &team)?);
    if !names.is_empty() {
        return Err(ApiError::conflict_vars(
            "inventory_is_in_use",
            [("names", names.join(", "))],
        ));
    }
    db.delete_inventory(&id, &team)?;
    Ok(no_content())
}

pub async fn list_environments(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
) -> AppResult<Json<EnvironmentListResponse>> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Viewer)?;
    let db = state.store();
    Ok(Json(EnvironmentListResponse(
        db.list_environments(&team_id)?,
    )))
}

fn environment_scope(raw: Option<&str>, fallback: &str) -> AppResult<String> {
    let raw = raw.unwrap_or("").trim();
    let scope = if raw.is_empty() { fallback } else { raw };
    match scope {
        "team" | "organization" | "global" => Ok(scope.to_string()),
        _ => Err(ApiError::bad("scope_is_team_organization_or_global")),
    }
}

fn may_manage_environment(user: &AuthUser, scope: &str) -> bool {
    user.is_admin || matches!(scope, "team" | "organization")
}

fn apply_environment_scope(item: &mut Environment, scope: &str, team: &db::Team) -> AppResult<()> {
    match scope {
        "team" => {
            item.team_id = Some(team.id.clone());
            item.organization_id = None;
        }
        "organization" => {
            if team.organization_id.is_empty() {
                return Err(ApiError::bad("organization_is_missing"));
            }
            item.team_id = None;
            item.organization_id = Some(team.organization_id.clone());
        }
        "global" => {
            item.team_id = None;
            item.organization_id = None;
        }
        _ => {
            return Err(ApiError::bad("scope_is_team_organization_or_global"));
        }
    }
    Ok(())
}

fn visible_environment(db: &dyn Backend, id: &str, team: &db::Team) -> AppResult<Environment> {
    let item = db.environment(id)?.ok_or(ApiError::NotFound)?;
    if !db::environment_visible(&item, team) {
        return Err(ApiError::NotFound);
    }
    Ok(item)
}

pub(super) fn environment_fields(
    user: &AuthUser,
    body: &EnvironmentBody,
    existing: Option<&Environment>,
) -> AppResult<(String, String, String, String, String, String)> {
    let legacy_unchanged = existing.is_some_and(|item| {
        item.tool == "any" && body.tool == "any" && item.image == body.image.trim()
    });
    if !legacy_unchanged && !matches!(body.tool.as_str(), "ansible" | "opentofu" | "shell") {
        return Err(ApiError::bad("choose_a_tool_for_the_environment"));
    }
    if !matches!(
        body.pull_policy.as_str(),
        "if_not_present" | "always" | "never"
    ) {
        return Err(ApiError::bad("pull_policy_is_if_not_present_always_or"));
    }
    let image = body.image.trim();
    if !runner::valid_image(image) {
        return Err(ApiError::bad("the_image_name_is_invalid"));
    }
    let network = body.network.trim();
    if !runner::valid_network(network) {
        return Err(ApiError::bad("the_network_name_is_invalid"));
    }
    if runner::privileged_network(network) && !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    Ok((
        require_name(&body.name)?,
        image.to_string(),
        body.pull_policy.clone(),
        network.to_string(),
        require_description(&body.description)?,
        body.tool.clone(),
    ))
}

pub async fn create_environment(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<EnvironmentBody>,
) -> AppResult<(StatusCode, Json<Environment>)> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Owner)?;
    let (name, image, pull_policy, network, description, tool) =
        environment_fields(&user, &body, None)?;
    let scope = environment_scope(body.scope.as_deref(), "team")?;
    if !may_manage_environment(&user, &scope) {
        return Err(ApiError::Forbidden);
    }
    let db = state.store();
    let team = db.team_by_id(&team_id)?.ok_or(ApiError::NotFound)?;
    let mut item = Environment {
        id: Uuid::new_v4().to_string(),
        team_id: None,
        organization_id: None,
        name,
        tool,
        image,
        pull_policy,
        network,
        description,
        created_at: db::now(),
        scope: String::new(),
        organization_name: String::new(),
        team_name: String::new(),
    };
    apply_environment_scope(&mut item, &scope, &team)?;
    db.insert_environment(&item)?;
    let stored = db.environment(&item.id)?.ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(stored)))
}

pub async fn get_environment(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Environment>> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Viewer)?;
    let db = state.store();
    let team = db.team_by_id(&team_id)?.ok_or(ApiError::NotFound)?;
    let item = visible_environment(db, &id, &team)?;
    Ok(Json(item))
}

pub async fn update_environment(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<EnvironmentBody>,
) -> AppResult<Json<Environment>> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Owner)?;
    let db = state.store();
    let team = db.team_by_id(&team_id)?.ok_or(ApiError::NotFound)?;
    let mut item = visible_environment(db, &id, &team)?;
    let (name, image, pull_policy, network, description, tool) =
        environment_fields(&user, &body, Some(&item))?;
    let scope = environment_scope(body.scope.as_deref(), &item.scope)?;
    if !may_manage_environment(&user, &item.scope) || !may_manage_environment(&user, &scope) {
        return Err(ApiError::Forbidden);
    }
    item.name = name;
    item.image = image;
    item.tool = tool;
    item.pull_policy = pull_policy;
    item.network = network;
    item.description = description;
    apply_environment_scope(&mut item, &scope, &team)?;
    db.update_environment(&item)?;
    let stored = db.environment(&item.id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(stored))
}

pub async fn delete_environment(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    let team_id = team_header(&headers)?;
    gate(&state, &user, &team_id, Role::Owner)?;
    let db = state.store();
    let team = db.team_by_id(&team_id)?.ok_or(ApiError::NotFound)?;
    let item = visible_environment(db, &id, &team)?;
    if !may_manage_environment(&user, &item.scope) {
        return Err(ApiError::Forbidden);
    }
    db.delete_environment(&id)?;
    Ok(no_content())
}
