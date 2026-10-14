use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{AuthUser, Role};
use crate::db::{self, Inventory, Project};
use crate::error::{ApiError, AppResult};
use crate::netbox::{self, NetboxSource};
use crate::runner;
use crate::scan;
use crate::Shared;

use super::catalog::inventory_json;
use super::{blank, created, gate, require_name, team_header};

#[derive(Deserialize)]
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
    group_by: Vec<String>,
    #[serde(default)]
    query_filters: Vec<String>,
    #[serde(default = "yes")]
    virtual_machines: bool,
}

#[derive(Deserialize)]
pub(super) struct ImportBody {
    project_id: String,
    path: String,
    name: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct NetboxBody {
    name: String,
    api_endpoint: String,
    token: String,
    #[serde(default = "yes")]
    validate_certs: bool,
    #[serde(default)]
    group_by: Vec<String>,
    #[serde(default)]
    query_filters: Vec<String>,
    #[serde(default = "yes")]
    virtual_machines: bool,
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
    Json(body): Json<ImportBody>,
) -> AppResult<axum::response::Response> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let path = body.path.trim();
    runner::check_relative(path).map_err(ApiError::bad)?;
    if path.is_empty() {
        return Err(ApiError::bad("Pfad fehlt."));
    }
    let project = visible_project(&state, &body.project_id, &team)?;
    let root = project_dir(&state, &project)?;
    let text = read_project_file(&root, path)?;
    let found = scan::classify_inventory(path, &text)
        .ok_or_else(|| ApiError::bad("Die Datei ist kein Inventar."))?;
    if found.format == "plugin" {
        return Err(ApiError::bad(
            "Plugin-Dateien werden nicht als Text übernommen. NetBox trägst du als Quelle ein, andere Plugins führt dieser Prototyp nicht aus.",
        ));
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
    Json(body): Json<NetboxBody>,
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
        source_json: json!({
            "api_endpoint": source.api_endpoint,
            "validate_certs": source.validate_certs,
            "group_by": source.group_by,
            "query": source.query,
            "virtual_machines": source.virtual_machines,
        })
        .to_string(),
        credential_id: Some(credential_id.clone()),
    };
    let source_row = db::InventorySource {
        id: Uuid::new_v4().to_string(),
        inventory_id: item.id.clone(),
        name: "NetBox".into(),
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
        },
    )?;
    db::insert_inventory(&tx, &item)?;
    db::insert_inventory_source(&tx, &source_row)?;
    tx.commit()?;
    Ok(created(inventory_json(&db, &item)?))
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
        return Err(ApiError::bad("Dieses Inventar hat keine externe Quelle."));
    }
    let db = state.db()?;
    for (source_id, body) in &fresh {
        db::set_inventory_source_body(&db, source_id, body)?;
    }
    db::refresh_inventory_summary(&db, &id)?;
    let item = db::inventory(&db, &id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_json(&db, &item)?))
}

fn refresh_project(state: &Shared, team: &str, source_json: &str) -> AppResult<String> {
    let source = db::json_object(source_json);
    let project_id = source
        .get("project_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad("Projektquelle ist unvollständig."))?;
    let path = source
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad("Projektquelle ist unvollständig."))?;
    let project = visible_project(state, project_id, team)?;
    let root = project_dir(state, &project)?;
    read_project_file(&root, path)
}

async fn refresh_netbox(
    state: &Shared,
    credential_id: &Option<String>,
    team_id: &str,
    source_json: &str,
) -> AppResult<String> {
    let stored: StoredNetbox = serde_json::from_str(source_json)
        .map_err(|_| ApiError::bad("NetBox-Quelle ist unvollständig."))?;
    let credential_id = credential_id
        .as_deref()
        .ok_or_else(|| ApiError::bad("NetBox-Token fehlt."))?;
    let token = {
        let db = state.db()?;
        let credential = db::credential(&db, credential_id)?
            .ok_or_else(|| ApiError::bad("NetBox-Token fehlt."))?;
        if credential.team_id != team_id || credential.kind != "netbox" {
            return Err(ApiError::bad("NetBox-Token gehört zu einem anderen Team."));
        }
        let plain = state.decrypt(&credential.secret_blob)?;
        token_from_secret(&plain)?
    };
    let source = NetboxSource {
        api_endpoint: stored.api_endpoint,
        validate_certs: stored.validate_certs,
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
    let value: Value =
        serde_json::from_str(plain).map_err(|_| ApiError::bad("NetBox-Zugang ist beschädigt."))?;
    let token = value.get("token").and_then(Value::as_str).unwrap_or("");
    if !netbox::valid_token(token) {
        return Err(ApiError::bad("NetBox-Token ist ungültig."));
    }
    Ok(token.to_string())
}

fn netbox_source(body: &NetboxBody) -> AppResult<NetboxSource> {
    let endpoint = body.api_endpoint.trim().trim_end_matches('/').to_string();
    if !netbox::valid_endpoint(&endpoint) {
        return Err(ApiError::bad("NetBox-Adresse muss http oder https sein."));
    }
    if !netbox::valid_token(body.token.trim()) {
        return Err(ApiError::bad("NetBox-Token ist ungültig."));
    }
    let group_by = body
        .group_by
        .iter()
        .flat_map(|item| item.split(','))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect();
    Ok(NetboxSource {
        api_endpoint: endpoint,
        validate_certs: body.validate_certs,
        group_by,
        query: netbox::parse_filters(&body.query_filters).map_err(ApiError::bad)?,
        virtual_machines: body.virtual_machines,
    })
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
        "path" => Ok(PathBuf::from(&project.source)),
        "git" => {
            let dest = state.cfg.data_dir.join("checkouts").join(&project.id);
            if dest.join(".git").is_dir() {
                Ok(dest)
            } else {
                Err(ApiError::bad("Git-Projekt zuerst synchronisieren."))
            }
        }
        other => Err(ApiError::bad(format!("Unbekannte Projektquelle {other}."))),
    }
}

fn read_project_file(root: &std::path::Path, rel: &str) -> AppResult<String> {
    let root = root
        .canonicalize()
        .map_err(|_| ApiError::bad("Projektpfad fehlt."))?;
    let full = root.join(rel);
    let full = full
        .canonicalize()
        .map_err(|_| ApiError::bad("Datei fehlt im Projekt."))?;
    if !full.starts_with(&root) {
        return Err(ApiError::bad("Pfad verlässt das Projekt."));
    }
    let meta = std::fs::metadata(&full).map_err(|err| ApiError::bad(err.to_string()))?;
    if !meta.is_file() {
        return Err(ApiError::bad("Pfad ist keine Datei."));
    }
    if meta.len() > 1_000_000 {
        return Err(ApiError::bad("Inventardatei ist größer als 1 MB."));
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
) -> AppResult<axum::response::Response> {
    if inputs.len() > MAX_SOURCES {
        return Err(ApiError::bad("Höchstens 8 Quellen."));
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
    Json(body): Json<SourceInput>,
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
            return Err(ApiError::bad("Höchstens 8 Quellen."));
        }
        count as i64
    };
    let resolved = resolve_source(&state, &team, &body).await?;
    let mut db = state.db()?;
    let tx = db.transaction()?;
    store_source(&tx, &team, &id, position, &resolved, &db::now())?;
    db::refresh_inventory_summary(&tx, &id)?;
    tx.commit()?;
    let item = db::inventory(&db, &id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_json(&db, &item)?))
}

pub async fn update_source(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    Path((inventory_id, source_id)): Path<(String, String)>,
    Json(body): Json<SourcePatch>,
) -> AppResult<Json<Value>> {
    let team = team_header(&headers)?;
    gate(&state, &user, &team, Role::Operator)?;
    let db = state.db()?;
    owned_source(&db, &inventory_id, &source_id, &team)?;
    let source = db::inventory_source(&db, &source_id)?.ok_or(ApiError::NotFound)?;
    if !body.body.trim().is_empty() && source.kind != "ini" {
        return Err(ApiError::bad(
            "Diese Quelle kommt aus einem Projekt oder aus NetBox. Aktualisieren holt sie neu.",
        ));
    }
    if !body.name.trim().is_empty() {
        db::rename_inventory_source(&db, &source_id, &require_name(&body.name)?)?;
    }
    if !body.body.trim().is_empty() {
        db::set_inventory_source_body(&db, &source_id, &body.body)?;
        db::refresh_inventory_summary(&db, &inventory_id)?;
    }
    let item = db::inventory(&db, &inventory_id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_json(&db, &item)?))
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
        _ => return Err(ApiError::bad("Dieses Inventar hat keine externe Quelle.")),
    };
    let db = state.db()?;
    db::set_inventory_source_body(&db, &source_id, &body)?;
    db::refresh_inventory_summary(&db, &inventory_id)?;
    let item = db::inventory(&db, &inventory_id)?.ok_or(ApiError::NotFound)?;
    Ok(Json(inventory_json(&db, &item)?))
}

#[derive(Deserialize)]
pub(super) struct SourcePatch {
    #[serde(default)]
    name: String,
    #[serde(default)]
    body: String,
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
                return Err(ApiError::bad("Die manuelle Quelle ist leer."));
            }
            Ok(ResolvedSource {
                name: source_label(&input.name, "Manuell")?,
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
                return Err(ApiError::bad("Pfad fehlt."));
            }
            let project = visible_project(state, input.project_id.trim(), team)?;
            let root = project_dir(state, &project)?;
            let text = read_project_file(&root, path)?;
            let found = scan::classify_inventory(path, &text)
                .ok_or_else(|| ApiError::bad("Die Datei ist kein Inventar."))?;
            if found.format == "plugin" {
                return Err(ApiError::bad(
                    "Plugin-Dateien werden nicht als Text übernommen. NetBox trägst du als eigene Quelle ein.",
                ));
            }
            let name = if input.name.trim().is_empty() {
                let stem = file_stem(path);
                if stem.trim().is_empty() {
                    "Projekt".into()
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
                group_by: input.group_by.clone(),
                query_filters: input.query_filters.clone(),
                virtual_machines: input.virtual_machines,
            })?;
            let ini = fetch_netbox(state, &parsed, input.token.trim()).await?;
            let name = source_label(&input.name, "NetBox")?;
            let secret = json!({"token": input.token.trim()}).to_string();
            let blob = state.encrypt(&secret)?;
            Ok(ResolvedSource {
                credential_name: Some(format!("{name} Token")),
                credential_blob: Some(blob),
                name,
                kind: "netbox".into(),
                source_json: json!({
                    "api_endpoint": parsed.api_endpoint,
                    "validate_certs": parsed.validate_certs,
                    "group_by": parsed.group_by,
                    "query": parsed.query,
                    "virtual_machines": parsed.virtual_machines,
                })
                .to_string(),
                body: ini,
            })
        }
        _ => Err(ApiError::bad("Quelle ist manuell, Projekt oder NetBox.")),
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
