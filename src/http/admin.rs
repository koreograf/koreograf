use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::backup;
use crate::db;
use crate::dbmigrate::{self, CheckResult, MigrationStatus, PendingTarget, Runtime};
use crate::error::{ApiError, ApiJson, AppResult};
use crate::features::identity as auth;
use crate::features::identity::AuthUser;
use crate::Shared;

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/maintenance", get(maintenance_status))
        .route("/admin/backup", get(download).post(restore))
        .route("/admin/database/status", get(database_status))
        .route("/admin/database/check", post(database_check))
        .route("/admin/database/migrate", post(database_migrate))
        .layer(DefaultBodyLimit::max(256 * 1024 * 1024))
}

#[derive(Deserialize, Default)]
pub struct DownloadQuery {
    #[serde(default)]
    tofu_state: bool,
}

#[derive(Serialize)]
pub(super) struct MaintenanceResponse {
    active: bool,
    since: Option<String>,
}

impl MaintenanceResponse {
    fn new(since: Option<String>) -> Self {
        Self {
            active: since.is_some(),
            since,
        }
    }
}

#[derive(Serialize)]
pub(super) struct RestoreResponse {
    restored: bool,
    engine: &'static str,
    previous: &'static str,
}

#[derive(Serialize)]
pub(super) struct DatabaseStatusResponse {
    engine: &'static str,
    supported: bool,
    maintenance: MaintenanceResponse,
    migration: MigrationStatus,
}

#[derive(Serialize)]
pub(super) struct DatabaseCheckResponse {
    all_passed: bool,
    id: Option<String>,
    checks: Vec<CheckResult>,
}

#[derive(Serialize)]
pub(super) struct DatabaseMigrateResponse {
    started: bool,
}

pub async fn download(
    State(state): State<Shared>,
    Query(query): Query<DownloadQuery>,
    user: AuthUser,
) -> AppResult<Response> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    let bytes = backup::export(&state, query.tofu_state)?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/zip"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"koreograf-backup.zip\""),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        bytes,
    )
        .into_response())
}

pub async fn restore(
    State(state): State<Shared>,
    headers: HeaderMap,
    user: AuthUser,
    body: Bytes,
) -> AppResult<Response> {
    if !user.is_admin {
        return Err(ApiError::Forbidden);
    }
    if headers
        .get("x-restore-confirm")
        .and_then(|value| value.to_str().ok())
        != Some("ERSETZEN")
    {
        return Err(ApiError::bad("confirm_the_replacement_with_ersetzen"));
    }
    backup::restore(&state, &body)?;
    let mut response = (
        StatusCode::OK,
        Json(RestoreResponse {
            restored: true,
            engine: state.engine(),
            previous: "restore-previous.zip",
        }),
    )
        .into_response();
    let cookie = HeaderValue::from_str(&auth::clear_cookie(auth::request_cookie_secure(
        &state.cfg, &headers,
    )))
    .map_err(|err| ApiError::Internal(err.to_string()))?;
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DatabaseTargetBody {
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    database: String,
    user: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    sslmode: String,
    ca_file: Option<String>,
}

fn default_port() -> u16 {
    5432
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DatabaseStartBody {
    id: String,
}

fn admin(user: &AuthUser) -> AppResult<()> {
    if user.is_admin {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn migration_lock(state: &Shared) -> AppResult<std::sync::MutexGuard<'_, Runtime>> {
    state
        .migration
        .lock()
        .map_err(|_| ApiError::Internal("migration state is damaged".into()))
}

pub async fn maintenance_status(State(state): State<Shared>) -> Json<MaintenanceResponse> {
    Json(MaintenanceResponse::new(state.maintenance_since()))
}

pub async fn database_status(
    State(state): State<Shared>,
    user: AuthUser,
) -> AppResult<Json<DatabaseStatusResponse>> {
    admin(&user)?;
    let runtime = migration_lock(&state)?;
    Ok(Json(DatabaseStatusResponse {
        engine: state.engine(),
        supported: state.engine() == "sqlite",
        maintenance: MaintenanceResponse::new(runtime.maintenance_since.clone()),
        migration: runtime.status.clone(),
    }))
}

pub async fn database_check(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<DatabaseTargetBody>,
) -> AppResult<Json<DatabaseCheckResponse>> {
    admin(&user)?;
    if state.engine() != "sqlite" {
        return Err(ApiError::bad("database_already_on_postgresql"));
    }
    let target = dbmigrate::parse_form_target(
        &body.host,
        body.port,
        &body.database,
        &body.user,
        &body.password,
        &body.sslmode,
        body.ca_file.as_deref(),
    )?;
    {
        let mut runtime = migration_lock(&state)?;
        if runtime.status.running || runtime.maintenance_since.is_some() {
            return Err(ApiError::conflict_vars(
                "database_migration_already_running",
                [("field", "id")],
            ));
        }
        runtime.pending = None;
    }
    let source = state.cfg.data_dir.join("koreograf.db");
    let checks = dbmigrate::run_checks(&target, &source).await;
    let passed = dbmigrate::all_passed(&checks);
    let id = if passed {
        let id = Uuid::new_v4().to_string();
        let mut runtime = migration_lock(&state)?;
        runtime.pending = Some(PendingTarget {
            id: id.clone(),
            config: target,
        });
        Some(id)
    } else {
        None
    };
    Ok(Json(DatabaseCheckResponse {
        all_passed: passed,
        id,
        checks,
    }))
}

pub async fn database_migrate(
    State(state): State<Shared>,
    user: AuthUser,
    ApiJson(body): ApiJson<DatabaseStartBody>,
) -> AppResult<(StatusCode, Json<DatabaseMigrateResponse>)> {
    admin(&user)?;
    if state.engine() != "sqlite" {
        return Err(ApiError::bad("database_already_on_postgresql"));
    }
    let target = {
        let mut runtime = migration_lock(&state)?;
        if runtime.status.running || runtime.maintenance_since.is_some() {
            return Err(ApiError::conflict_vars(
                "database_migration_already_running",
                [("field", "id")],
            ));
        }
        let Some(pending) = runtime.pending.as_ref() else {
            return Err(ApiError::bad("run_the_checks_again"));
        };
        if pending.id != body.id {
            return Err(ApiError::bad("run_the_checks_again"));
        }
        {
            let procs = state
                .procs
                .lock()
                .map_err(|_| ApiError::Internal("job state is damaged".into()))?;
            if !procs.is_empty() {
                return Err(ApiError::conflict_vars(
                    "finish_or_cancel_running_jobs",
                    [("field", "id")],
                ));
            }
        }
        let target = match runtime.pending.take() {
            Some(pending) => pending.config,
            None => return Err(ApiError::bad("run_the_checks_again")),
        };
        let started_at = db::now();
        let audit = dbmigrate::audit_entry(&user.username, &target);
        runtime.status = MigrationStatus {
            running: true,
            started_at: Some(started_at.clone()),
            target_host: target.target(),
            url: dbmigrate::target_url(&target),
            tables: MigrationStatus::empty_tables(),
            audit: Some(audit.clone()),
            ..MigrationStatus::default()
        };
        runtime.maintenance_since = Some(started_at);
        target
    };
    tracing::info!(
        audit = %dbmigrate::audit_line(&dbmigrate::audit_entry(&user.username, &target)),
        "database migration started"
    );
    let task_state = state.clone();
    let username = user.username.clone();
    tokio::spawn(async move {
        run_migration(task_state, target, username).await;
    });
    Ok((
        StatusCode::ACCEPTED,
        Json(DatabaseMigrateResponse { started: true }),
    ))
}

async fn run_migration(state: Shared, target: crate::PostgresConfig, user: String) {
    let snapshot = state
        .cfg
        .data_dir
        .join(format!("migration-{}.db", Uuid::new_v4()));
    let result = async {
        state.store().export_sqlite(&snapshot)?;
        dbmigrate::check_source(&snapshot)
            .map_err(|_| ApiError::bad("database_migration_failed"))?;
        let client = crate::backend::connect_client(&target)
            .await
            .map_err(|_| ApiError::bad("database_migration_failed"))?;
        dbmigrate::precheck(&client)
            .await
            .map_err(|_| ApiError::bad("database_migration_failed"))?;
        let progress_state = state.clone();
        dbmigrate::copy_source(&client, &snapshot, move |table, rows| {
            if let Ok(mut runtime) = progress_state.migration.lock() {
                if let Some(item) = runtime
                    .status
                    .tables
                    .iter_mut()
                    .find(|item| item.table == table)
                {
                    item.rows = rows;
                    item.done = true;
                }
                runtime.status.rows += rows;
            }
        })
        .await
        .map_err(|_| ApiError::bad("database_migration_failed"))
    }
    .await;
    remove_snapshot(&snapshot);
    let host = target.target();
    match result {
        Ok(rows) => {
            if let Ok(mut runtime) = state.migration.lock() {
                runtime.status.running = false;
                runtime.status.done = true;
                runtime.status.failed = false;
                runtime.status.rows = rows;
                runtime.status.finished_at = Some(db::now());
                runtime.status.error = None;
            }
            tracing::info!(
                user = %user,
                target = %host,
                rows,
                "database migration finished; restart with the new database URL"
            );
        }
        Err(_) => {
            if let Ok(mut runtime) = state.migration.lock() {
                runtime.status.running = false;
                runtime.status.done = false;
                runtime.status.failed = true;
                runtime.status.finished_at = Some(db::now());
                runtime.status.error = Some("database_migration_failed");
                runtime.maintenance_since = None;
            }
            tracing::error!(
                user = %user,
                target = %host,
                "database migration failed; Koreograf stays on SQLite"
            );
        }
    }
}

fn remove_snapshot(snapshot: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = snapshot.as_os_str().to_os_string();
        name.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(name));
    }
}
