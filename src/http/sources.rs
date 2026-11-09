use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{AuthUser, Role};
use crate::backend::Backend;
use crate::db::{self, Inventory, Project};
use crate::error::{ApiError, ApiJson, AppResult};
use crate::netbox::{self, NetboxSource};
use crate::runner;
use crate::scan;
use crate::Shared;

use super::catalog::inventory_json;
use super::{blank, check_static_ini, created, gate, require_name, team_header};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceInput {
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
    netbox::DEFAULT_MAX_HOSTS
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

pub async fn project_contents(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let project = visible_project(&state, &id, &team)?;
    let root = project_dir(&state, &project)?;
    let contents = scan::scan_project(&root).map_err(ApiError::bad)?;
    Ok(Json(json!(contents)))
}

pub async fn import_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<ImportBody>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let path = body.path.trim();
    runner::check_relative(path).map_err(ApiError::bad)?;
    if path.is_empty() {
        return Err(ApiError::bad("path_is_missing"));
    }
    let project = visible_project(&state, &body.project_id, &team)?;
    let root = project_dir(&state, &project)?;
    let text = read_project_file(&root, path)?;
    let found = scan::classify_inventory(path, &text)
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
        source_json: json!({"project_id": project.id, "path": path}).to_string(),
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
    Ok(created(inventory_json(db, &item)?))
}

pub async fn create_netbox(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<NetboxBody>,
) -> AppResult<axum::response::Response> {
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
        source_json: netbox::source_json_with_collisions(&source, &fetched.collisions).to_string(),
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
    Ok(created(inventory_json(db, &item)?))
}

pub async fn test_netbox(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<NetboxTestBody>,
) -> AppResult<Json<Value>> {
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
    let proxy = crate::settings::load(&state)?.proxy;
    let version = netbox::test_connection(&source, &access.token, &proxy)
        .await
        .map_err(ApiError::bad)?;
    Ok(Json(json!({ "version": version })))
}

pub async fn preview_netbox(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<NetboxPreviewBody>,
) -> AppResult<Json<Value>> {
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
    let report = netbox::preview(&fetched.body);
    let sample = report.sample_host.map(|(name, vars)| {
        json!({
            "name": name,
            "vars": vars,
        })
    });
    Ok(Json(json!({
        "host_count": report.host_count,
        "groups": report.groups.iter().map(|(name, hosts)| json!({
            "name": name,
            "host_count": hosts,
        })).collect::<Vec<_>>(),
        "sample_host": sample,
        "collisions": fetched.collisions,
        "dropped_vars": fetched.dropped_vars,
    })))
}

pub async fn sync_inventory(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
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
                netbox::NetboxResult {
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
    let mut value = inventory_json(db, &item)?;
    value["collisions"] = json!(collisions);
    value["dropped_vars"] = json!(dropped_vars);
    Ok(Json(value))
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
    if !netbox::valid_endpoint(&url) {
        return Err(ApiError::bad("the_netbox_address_must_be_http_or_https"));
    }
    Ok((token, url))
}

async fn refresh_netbox(
    state: &Shared,
    credential_id: &Option<String>,
    team_id: &str,
    source_json: &str,
) -> AppResult<netbox::NetboxResult> {
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
) -> AppResult<netbox::NetboxResult> {
    let proxy = crate::settings::load(state)?.proxy;
    netbox::fetch(source, token, &proxy)
        .await
        .map_err(ApiError::bad)
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
    if !netbox::valid_endpoint(&endpoint) {
        return Err(ApiError::bad("the_netbox_address_must_be_http_or_https"));
    }
    if fields.max_hosts == 0 || fields.max_hosts > netbox::MAX_HOSTS_LIMIT {
        return Err(ApiError::bad_vars(
            "netbox_host_limit_is_out_of_range",
            [("max", netbox::MAX_HOSTS_LIMIT.to_string())],
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
    if !netbox::valid_token(token.trim()) {
        return Err(ApiError::bad("the_netbox_token_is_invalid"));
    }
    netbox_config(
        &NetboxFields {
            api_endpoint,
            validate_certs,
            group_by: &[],
            group_names_raw: false,
            load_config_context: false,
            max_hosts: netbox::DEFAULT_MAX_HOSTS,
            query_filters: &[],
            virtual_machines: false,
        },
        Vec::new(),
    )
}

fn netbox_source_fields(fields: NetboxFields<'_>) -> AppResult<NetboxSource> {
    let query = netbox::parse_filters(fields.query_filters).map_err(ApiError::bad)?;
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
        let item = db
            .credential(id)?
            .ok_or_else(|| ApiError::bad("netbox_credential_is_required"))?;
        (item.secret_blob, item.kind, item.team_id, item.disabled)
    };
    if credential_team != team {
        return Err(ApiError::bad("the_credential_belongs_to_another_team"));
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
    if !netbox::valid_token(token) {
        return Err(ApiError::bad("the_netbox_token_is_invalid"));
    }
    Ok(token.to_string())
}

fn visible_project(state: &Shared, id: &str, team: &str) -> AppResult<Project> {
    let db = state.store();
    if !db.project_visible(id, team)? {
        return Err(ApiError::NotFound);
    }
    db.project(id)?.ok_or(ApiError::NotFound)
}

fn project_dir(state: &Shared, project: &Project) -> AppResult<PathBuf> {
    match project.source_type.as_str() {
        "path" => runner::stored_directory(
            std::path::Path::new(&project.source),
            &state.cfg.data_dir,
            &state.cfg.project_roots,
        )
        .map_err(ApiError::bad),
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

fn read_project_file(root: &std::path::Path, rel: &str) -> AppResult<String> {
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

const MAX_SOURCES: usize = 8;

struct ResolvedSource {
    name: String,
    kind: String,
    source_json: String,
    body: String,
    credential_id: Option<String>,
}

pub(super) async fn create_with_sources(
    state: Shared,
    team: String,
    name: String,
    inputs: Vec<SourceInput>,
    admin_saved: bool,
    user: &AuthUser,
) -> AppResult<axum::response::Response> {
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
    Ok(created(inventory_json(db, &item)?))
}

pub async fn add_source(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<SourceInput>,
) -> AppResult<Json<Value>> {
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
    Ok(Json(inventory_json(db, &item)?))
}

pub async fn get_source(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((inventory_id, source_id)): Path<(String, String)>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Viewer)?;
    let db = state.store();
    let source = owned_source(db, &inventory_id, &source_id, &team)?;
    Ok(Json(json!({
        "id": source.id,
        "inventory_id": source.inventory_id,
        "name": source.name,
        "kind": source.kind,
        "source": db::json_object(&source.source_json),
        "credential_id": source.credential_id,
        "host_count": crate::inventory::parse_hosts(&source.body).len(),
        "body": source.body,
    })))
}

pub async fn update_source(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((inventory_id, source_id)): Path<(String, String)>,
    ApiJson(body): ApiJson<SourceUpdate>,
) -> AppResult<Json<Value>> {
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
        let before = inventory_json(db, &item)?["host_count"]
            .as_u64()
            .unwrap_or(0);
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
    let mut value = inventory_json(db, &item)?;
    if source.kind == "project" || source.kind == "netbox" {
        let after = value["host_count"].as_u64().unwrap_or(0);
        value["sync"] = json!({"ok": true, "before": before, "after": after});
        value["collisions"] = json!(resolved.collisions);
        value["dropped_vars"] = json!(resolved.dropped_vars);
    }
    Ok(Json(value))
}

pub async fn delete_source(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((inventory_id, source_id)): Path<(String, String)>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.store();
    owned_source(db, &inventory_id, &source_id, &team)?;
    db.delete_inventory_source(&source_id, &inventory_id, user.is_admin)?;
    let item = db.inventory(&inventory_id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_json(db, &item)?))
}

pub async fn sync_source(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((inventory_id, source_id)): Path<(String, String)>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let source = {
        let db = state.store();
        owned_source(db, &inventory_id, &source_id, &team)?
    };
    let fetched = match source.kind.as_str() {
        "project" => netbox::NetboxResult {
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
    let mut value = inventory_json(db, &item)?;
    value["collisions"] = json!(fetched.collisions);
    value["dropped_vars"] = json!(fetched.dropped_vars);
    Ok(Json(value))
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
    runner::check_relative(path).map_err(ApiError::bad)?;
    if path.is_empty() {
        return Err(ApiError::bad("path_is_missing"));
    }
    let project = visible_project(state, project_id, team)?;
    let root = project_dir(state, &project)?;
    let text = read_project_file(&root, path)?;
    let found = scan::classify_inventory(path, &text)
        .ok_or_else(|| ApiError::bad("the_file_is_not_an_inventory"))?;
    if found.format == "plugin" {
        return Err(ApiError::bad("plugin_files_are_not_copied_as_text_add"));
    }
    let name = updated_name(body.name.clone(), &source.name, &file_stem(path))?;
    Ok(ResolvedUpdate {
        name,
        source_json: json!({"project_id": project.id, "path": path}).to_string(),
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
        Some(filters) => netbox::parse_filters(filters).map_err(ApiError::bad)?,
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
        source_json: netbox::source_json_with_collisions(&parsed, &fetched.collisions).to_string(),
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
            runner::check_relative(path).map_err(ApiError::bad)?;
            if path.is_empty() {
                return Err(ApiError::bad("path_is_missing"));
            }
            let project = visible_project(state, input.project_id.trim(), team)?;
            let root = project_dir(state, &project)?;
            let text = read_project_file(&root, path)?;
            let found = scan::classify_inventory(path, &text)
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
                source_json: json!({"project_id": project.id, "path": path}).to_string(),
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
                source_json: netbox::source_json_with_collisions(&parsed, &fetched.collisions)
                    .to_string(),
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
