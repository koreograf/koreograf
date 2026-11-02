use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{AuthUser, Role};
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
    token: String,
    #[serde(default = "yes")]
    validate_certs: bool,
    #[serde(default)]
    ca_file: String,
    #[serde(default)]
    group_by: Vec<String>,
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
    api_endpoint: String,
    token: String,
    #[serde(default = "yes")]
    validate_certs: bool,
    #[serde(default)]
    ca_file: String,
    #[serde(default)]
    group_by: Vec<String>,
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
    token: String,
    #[serde(default = "yes")]
    validate_certs: bool,
    #[serde(default)]
    ca_file: String,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct StoredNetbox {
    api_endpoint: String,
    #[serde(default = "yes")]
    validate_certs: bool,
    #[serde(default)]
    ca_file: String,
    #[serde(default)]
    group_by: Vec<String>,
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
    let mut db = state.db()?;
    let tx = db.transaction()?;
    db::insert_inventory(&tx, &item)?;
    db::insert_inventory_source(&tx, &source_row)?;
    tx.commit()?;
    Ok(created(inventory_json(&db, &item)?))
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
    let source = netbox_source(&body)?;
    let ini = fetch_netbox(&state, &source, body.token.trim()).await?;
    let secret = json!({"token": body.token.trim()}).to_string();
    let blob = state.encrypt(&secret)?;
    let credential_id = Uuid::new_v4().to_string();
    let inventory_id = Uuid::new_v4().to_string();
    let item = Inventory {
        id: inventory_id,
        team_id: team.clone(),
        name: name.clone(),
        body: ini,
        created_at: db::now(),
        source_kind: "netbox".into(),
        source_json: netbox::source_json(&source).to_string(),
        credential_id: Some(credential_id.clone()),
        admin_saved: user.is_admin,
    };
    let source_row = db::InventorySource {
        id: Uuid::new_v4().to_string(),
        inventory_id: item.id.clone(),
        name: "netbox".into(),
        kind: "netbox".into(),
        source_json: item.source_json.clone(),
        credential_id: Some(credential_id.clone()),
        body: item.body.clone(),
        position: 0,
        created_at: item.created_at.clone(),
    };
    let mut db = state.db()?;
    let tx = db.transaction()?;
    db::insert_credential(
        &tx,
        &db::Credential {
            id: credential_id,
            team_id: team,
            name: format!("{name} Token"),
            kind: "netbox".into(),
            secret_blob: blob,
            created_at: db::now(),
            auto: true,
        },
    )?;
    db::insert_inventory(&tx, &item)?;
    db::insert_inventory_source(&tx, &source_row)?;
    tx.commit()?;
    Ok(created(inventory_json(&db, &item)?))
}

pub async fn test_netbox(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    ApiJson(body): ApiJson<NetboxTestBody>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let source = connection_source(
        &body.api_endpoint,
        &body.token,
        body.validate_certs,
        &body.ca_file,
    )?;
    let proxy = crate::settings::load(&state)?.proxy;
    let version = netbox::test_connection(&source, body.token.trim(), &proxy)
        .await
        .map_err(ApiError::bad)?;
    Ok(Json(json!({ "version": version })))
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
        let db = state.db()?;
        let item = db::inventory(&db, &id)?.ok_or(ApiError::NotFound)?;
        if item.team_id != team {
            return Err(ApiError::NotFound);
        }
        db::list_inventory_sources(&db, &id)?
    };
    let mut fresh = Vec::new();
    for source in &sources {
        match source.kind.as_str() {
            "project" => fresh.push((
                source.id.clone(),
                refresh_project(&state, &team, &source.source_json)?,
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
    let db = state.db()?;
    for (source_id, body) in &fresh {
        db::set_inventory_source_body(&db, source_id, body)?;
    }
    db::refresh_inventory_summary(&db, &id)?;
    db::set_inventory_admin_saved(&db, &id, user.is_admin)?;
    let item = db::inventory(&db, &id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_json(&db, &item)?))
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

fn stored_token(
    state: &Shared,
    credential_id: &Option<String>,
    team_id: &str,
) -> AppResult<String> {
    let credential_id = credential_id
        .as_deref()
        .ok_or_else(|| ApiError::bad("the_netbox_token_is_missing"))?;
    let db = state.db()?;
    let credential = db::credential(&db, credential_id)?
        .ok_or_else(|| ApiError::bad("the_netbox_token_is_missing"))?;
    if credential.team_id != team_id || credential.kind != "netbox" {
        return Err(ApiError::bad("the_netbox_token_belongs_to_another_team"));
    }
    let plain = state.decrypt(&credential.secret_blob)?;
    token_from_secret(&plain)
}

async fn refresh_netbox(
    state: &Shared,
    credential_id: &Option<String>,
    team_id: &str,
    source_json: &str,
) -> AppResult<String> {
    let stored = stored_netbox(source_json)?;
    let token = stored_token(state, credential_id, team_id)?;
    let source = NetboxSource {
        api_endpoint: stored.api_endpoint,
        validate_certs: stored.validate_certs,
        ca_file: stored.ca_file,
        group_by: stored.group_by,
        query: stored.query,
        virtual_machines: stored.virtual_machines,
    };
    fetch_netbox(state, &source, &token).await
}

async fn fetch_netbox(state: &Shared, source: &NetboxSource, token: &str) -> AppResult<String> {
    let proxy = crate::settings::load(state)?.proxy;
    netbox::fetch_ini(source, token, &proxy)
        .await
        .map_err(ApiError::bad)
}

fn token_from_secret(plain: &str) -> AppResult<String> {
    let value: Value = serde_json::from_str(plain)
        .map_err(|_| ApiError::bad("the_netbox_credential_is_damaged"))?;
    let token = value.get("token").and_then(Value::as_str).unwrap_or("");
    if !netbox::valid_token(token) {
        return Err(ApiError::bad("the_netbox_token_is_invalid"));
    }
    Ok(token.to_string())
}

fn netbox_config(
    api_endpoint: &str,
    validate_certs: bool,
    ca_file: &str,
    group_by: &[String],
    query: Vec<(String, String)>,
    virtual_machines: bool,
) -> AppResult<NetboxSource> {
    let endpoint = api_endpoint.trim().trim_end_matches('/').to_string();
    if !netbox::valid_endpoint(&endpoint) {
        return Err(ApiError::bad("the_netbox_address_must_be_http_or_https"));
    }
    let ca_file = ca_file.trim().to_string();
    crate::tls::absolute_ca_path(&ca_file).map_err(ApiError::bad)?;
    Ok(NetboxSource {
        api_endpoint: endpoint,
        validate_certs,
        ca_file,
        group_by: group_by
            .iter()
            .flat_map(|item| item.split(','))
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        query,
        virtual_machines,
    })
}

fn connection_source(
    api_endpoint: &str,
    token: &str,
    validate_certs: bool,
    ca_file: &str,
) -> AppResult<NetboxSource> {
    if !netbox::valid_token(token.trim()) {
        return Err(ApiError::bad("the_netbox_token_is_invalid"));
    }
    netbox_config(
        api_endpoint,
        validate_certs,
        ca_file,
        &[],
        Vec::new(),
        false,
    )
}

fn netbox_source(body: &NetboxBody) -> AppResult<NetboxSource> {
    if !netbox::valid_token(body.token.trim()) {
        return Err(ApiError::bad("the_netbox_token_is_invalid"));
    }
    let query = netbox::parse_filters(&body.query_filters).map_err(ApiError::bad)?;
    netbox_config(
        &body.api_endpoint,
        body.validate_certs,
        &body.ca_file,
        &body.group_by,
        query,
        body.virtual_machines,
    )
}

fn visible_project(state: &Shared, id: &str, team: &str) -> AppResult<Project> {
    let db = state.db()?;
    if !db::project_visible(&db, id, team)? {
        return Err(ApiError::NotFound);
    }
    db::project(&db, id)?.ok_or(ApiError::NotFound)
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
    credential_name: Option<String>,
    credential_blob: Option<String>,
}

pub(super) async fn create_with_sources(
    state: Shared,
    team: String,
    name: String,
    inputs: Vec<SourceInput>,
    admin_saved: bool,
) -> AppResult<axum::response::Response> {
    if inputs.len() > MAX_SOURCES {
        return Err(ApiError::bad("at_most_8_sources"));
    }
    let mut resolved = Vec::with_capacity(inputs.len());
    for input in &inputs {
        resolved.push(resolve_source(&state, &team, input).await?);
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
    let mut db = state.db()?;
    let tx = db.transaction()?;
    db::insert_inventory(&tx, &item)?;
    for (position, source) in resolved.iter().enumerate() {
        store_source(
            &tx,
            &team,
            &inventory_id,
            position as i64,
            source,
            &created_at,
        )?;
    }
    db::refresh_inventory_summary(&tx, &inventory_id)?;
    tx.commit()?;
    let item = db::inventory(&db, &inventory_id)?.ok_or(ApiError::NotFound)?;
    Ok(created(inventory_json(&db, &item)?))
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
        let db = state.db()?;
        let item = db::inventory(&db, &id)?.ok_or(ApiError::NotFound)?;
        if item.team_id != team {
            return Err(ApiError::NotFound);
        }
        let count = db::list_inventory_sources(&db, &id)?.len();
        if count >= MAX_SOURCES {
            return Err(ApiError::bad("at_most_8_sources"));
        }
        count as i64
    };
    let resolved = resolve_source(&state, &team, &body).await?;
    let mut db = state.db()?;
    let tx = db.transaction()?;
    store_source(&tx, &team, &id, position, &resolved, &db::now())?;
    db::refresh_inventory_summary(&tx, &id)?;
    db::set_inventory_admin_saved(&tx, &id, user.is_admin)?;
    tx.commit()?;
    let item = db::inventory(&db, &id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_json(&db, &item)?))
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
        let db = state.db()?;
        let item = db::inventory(&db, &inventory_id)?.ok_or(ApiError::NotFound)?;
        if item.team_id != team {
            return Err(ApiError::NotFound);
        }
        let source = db::inventory_source(&db, &source_id)?.ok_or(ApiError::NotFound)?;
        if source.inventory_id != inventory_id {
            return Err(ApiError::NotFound);
        }
        let before = inventory_json(&db, &item)?["host_count"]
            .as_u64()
            .unwrap_or(0);
        (source, before)
    };
    reject_foreign_fields(&source.kind, &body)?;
    let resolved = match source.kind.as_str() {
        "ini" => resolve_ini_update(&source, &body)?,
        "project" => resolve_project_update(&state, &team, &source, &body)?,
        "netbox" => resolve_netbox_update(&state, &team, &source, &body).await?,
        _ => return Err(ApiError::bad("a_source_is_manual_project_or_netbox")),
    };
    let mut db = state.db()?;
    let tx = db.transaction()?;
    db::update_inventory_source(
        &tx,
        &source_id,
        &resolved.name,
        &resolved.source_json,
        &resolved.body,
    )?;
    if let Some(credential_name) = &resolved.credential_name {
        let credential_id = source
            .credential_id
            .as_deref()
            .ok_or_else(|| ApiError::bad("the_netbox_token_is_missing"))?;
        let mut credential = db::credential(&tx, credential_id)?
            .ok_or_else(|| ApiError::bad("the_netbox_token_is_missing"))?;
        if credential.team_id != team || credential.kind != "netbox" {
            return Err(ApiError::bad("the_netbox_token_belongs_to_another_team"));
        }
        credential.name = credential_name.clone();
        if let Some(blob) = &resolved.credential_blob {
            credential.secret_blob = blob.clone();
        }
        db::update_credential(&tx, &credential)?;
    }
    db::refresh_inventory_summary(&tx, &inventory_id)?;
    db::set_inventory_admin_saved(&tx, &inventory_id, user.is_admin)?;
    tx.commit()?;
    let item = db::inventory(&db, &inventory_id)?.ok_or(ApiError::NotFound)?;
    let mut value = inventory_json(&db, &item)?;
    if source.kind == "project" || source.kind == "netbox" {
        let after = value["host_count"].as_u64().unwrap_or(0);
        value["sync"] = json!({"ok": true, "before": before, "after": after});
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
    let db = state.db()?;
    let source = owned_source(&db, &inventory_id, &source_id, &team)?;
    db::delete_inventory_source(&db, &source_id, &inventory_id)?;
    if source.kind == "netbox" {
        if let Some(credential_id) = source.credential_id {
            let _ = db::delete_credential(&db, &credential_id, &team);
        }
    }
    db::refresh_inventory_summary(&db, &inventory_id)?;
    db::set_inventory_admin_saved(&db, &inventory_id, user.is_admin)?;
    let item = db::inventory(&db, &inventory_id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_json(&db, &item)?))
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
        let db = state.db()?;
        owned_source(&db, &inventory_id, &source_id, &team)?
    };
    let body = match source.kind.as_str() {
        "project" => refresh_project(&state, &team, &source.source_json)?,
        "netbox" => {
            refresh_netbox(&state, &source.credential_id, &team, &source.source_json).await?
        }
        _ => return Err(ApiError::bad("this_inventory_has_no_external_source")),
    };
    let db = state.db()?;
    db::set_inventory_source_body(&db, &source_id, &body)?;
    db::refresh_inventory_summary(&db, &inventory_id)?;
    db::set_inventory_admin_saved(&db, &inventory_id, user.is_admin)?;
    let item = db::inventory(&db, &inventory_id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_json(&db, &item)?))
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
    token: Option<String>,
    #[serde(default)]
    validate_certs: Option<bool>,
    #[serde(default)]
    ca_file: Option<String>,
    #[serde(default)]
    group_by: Option<Vec<String>>,
    #[serde(default)]
    query_filters: Option<Vec<String>>,
    #[serde(default)]
    virtual_machines: Option<bool>,
}

struct ResolvedUpdate {
    name: String,
    source_json: String,
    body: String,
    credential_name: Option<String>,
    credential_blob: Option<String>,
}

fn reject_foreign_fields(kind: &str, body: &SourceUpdate) -> AppResult<()> {
    let fields: &[(&str, bool)] = match kind {
        "ini" => &[
            ("project_id", body.project_id.is_some()),
            ("path", body.path.is_some()),
            ("api_endpoint", body.api_endpoint.is_some()),
            ("token", body.token.is_some()),
            ("validate_certs", body.validate_certs.is_some()),
            ("ca_file", body.ca_file.is_some()),
            ("group_by", body.group_by.is_some()),
            ("query_filters", body.query_filters.is_some()),
            ("virtual_machines", body.virtual_machines.is_some()),
        ],
        "project" => &[
            ("body", body.body.is_some()),
            ("api_endpoint", body.api_endpoint.is_some()),
            ("token", body.token.is_some()),
            ("validate_certs", body.validate_certs.is_some()),
            ("ca_file", body.ca_file.is_some()),
            ("group_by", body.group_by.is_some()),
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
        credential_name: None,
        credential_blob: None,
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
        credential_name: None,
        credential_blob: None,
    })
}

async fn resolve_netbox_update(
    state: &Shared,
    team: &str,
    source: &db::InventorySource,
    body: &SourceUpdate,
) -> AppResult<ResolvedUpdate> {
    let stored = stored_netbox(&source.source_json)?;
    let new_token = body
        .token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty());
    let query = match &body.query_filters {
        Some(filters) => netbox::parse_filters(filters).map_err(ApiError::bad)?,
        None => stored.query.clone(),
    };
    let parsed = netbox_config(
        body.api_endpoint.as_deref().unwrap_or(&stored.api_endpoint),
        body.validate_certs.unwrap_or(stored.validate_certs),
        body.ca_file.as_deref().unwrap_or(&stored.ca_file),
        body.group_by.as_deref().unwrap_or(&stored.group_by),
        query,
        body.virtual_machines.unwrap_or(stored.virtual_machines),
    )?;
    let changes_destination = parsed.api_endpoint != stored.api_endpoint
        || parsed.ca_file != stored.ca_file
        || (stored.validate_certs && !parsed.validate_certs);
    if changes_destination && new_token.is_none() {
        return Err(ApiError::bad("netbox_token_required_for_endpoint_change"));
    }
    let token = match new_token {
        Some(token) => token.to_string(),
        None => stored_token(state, &source.credential_id, team)?,
    };
    let name = updated_name(body.name.clone(), &source.name, "netbox")?;
    let ini = fetch_netbox(state, &parsed, &token).await?;
    let credential_blob = match new_token {
        Some(token) => Some(state.encrypt(&json!({"token": token}).to_string())?),
        None => None,
    };
    Ok(ResolvedUpdate {
        credential_name: Some(format!("{name} Token")),
        credential_blob,
        name,
        source_json: netbox::source_json(&parsed).to_string(),
        body: ini,
    })
}

fn owned_source(
    db: &rusqlite::Connection,
    inventory_id: &str,
    source_id: &str,
    team: &str,
) -> AppResult<db::InventorySource> {
    let item = db::inventory(db, inventory_id)?.ok_or(ApiError::NotFound)?;
    if item.team_id != team {
        return Err(ApiError::NotFound);
    }
    let source = db::inventory_source(db, source_id)?.ok_or(ApiError::NotFound)?;
    if source.inventory_id != inventory_id {
        return Err(ApiError::NotFound);
    }
    Ok(source)
}

fn store_source(
    db: &rusqlite::Connection,
    team: &str,
    inventory_id: &str,
    position: i64,
    source: &ResolvedSource,
    created_at: &str,
) -> AppResult<()> {
    let credential_id = if let Some(blob) = &source.credential_blob {
        let id = Uuid::new_v4().to_string();
        let credential_name = source
            .credential_name
            .clone()
            .unwrap_or_else(|| format!("{} Token", source.name));
        db::insert_credential(
            db,
            &db::Credential {
                id: id.clone(),
                team_id: team.to_string(),
                name: credential_name,
                kind: "netbox".into(),
                secret_blob: blob.clone(),
                created_at: created_at.to_string(),
                auto: true,
            },
        )?;
        Some(id)
    } else {
        None
    };
    db::insert_inventory_source(
        db,
        &db::InventorySource {
            id: Uuid::new_v4().to_string(),
            inventory_id: inventory_id.to_string(),
            name: source.name.clone(),
            kind: source.kind.clone(),
            source_json: source.source_json.clone(),
            credential_id,
            body: source.body.clone(),
            position,
            created_at: created_at.to_string(),
        },
    )
}

async fn resolve_source(
    state: &Shared,
    team: &str,
    input: &SourceInput,
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
                credential_name: None,
                credential_blob: None,
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
                credential_name: None,
                credential_blob: None,
            })
        }
        "netbox" => {
            let parsed = netbox_source(&NetboxBody {
                name: input.name.clone(),
                api_endpoint: input.api_endpoint.clone(),
                token: input.token.clone(),
                validate_certs: input.validate_certs,
                ca_file: input.ca_file.clone(),
                group_by: input.group_by.clone(),
                query_filters: input.query_filters.clone(),
                virtual_machines: input.virtual_machines,
            })?;
            let ini = fetch_netbox(state, &parsed, input.token.trim()).await?;
            let name = source_label(&input.name, "netbox")?;
            let secret = json!({"token": input.token.trim()}).to_string();
            let blob = state.encrypt(&secret)?;
            Ok(ResolvedSource {
                credential_name: Some(format!("{name} Token")),
                credential_blob: Some(blob),
                name,
                kind: "netbox".into(),
                source_json: netbox::source_json(&parsed).to_string(),
                body: ini,
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
