mod admin;
mod catalog;
mod session;
mod sources;
mod work;

use std::path::Path;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::auth::{self, AuthUser, Role};
use crate::db::{self, Job, JobEvent};
use crate::error::{ApiError, AppResult};
use crate::runner;
use crate::Shared;

pub fn router(state: Shared) -> Router {
    let api = Router::new()
        .route("/login", post(session::login))
        .route("/logout", post(session::logout))
        .route("/me", get(session::me).put(session::update_me))
        .route(
            "/users",
            get(session::list_users).post(session::create_user),
        )
        .route(
            "/users/{id}",
            get(session::get_user)
                .put(session::update_user)
                .delete(session::delete_user),
        )
        .route("/overview", get(catalog::overview))
        .route(
            "/teams",
            get(catalog::list_teams).post(catalog::create_team),
        )
        .route("/teams/{id}", axum::routing::put(catalog::update_team))
        .route(
            "/teams/{id}/members",
            get(catalog::list_members).post(catalog::add_member),
        )
        .route(
            "/teams/{id}/members/{user_id}",
            axum::routing::delete(catalog::remove_member),
        )
        .route(
            "/projects",
            get(catalog::list_projects).post(catalog::create_project),
        )
        .route(
            "/projects/{id}",
            get(catalog::get_project)
                .put(catalog::update_project)
                .delete(catalog::delete_project),
        )
        .route("/projects/{id}/sync", post(catalog::sync_project))
        .route("/projects/{id}/contents", get(sources::project_contents))
        .route(
            "/inventories",
            get(catalog::list_inventories).post(catalog::create_inventory),
        )
        .route("/inventories/import", post(sources::import_inventory))
        .route("/inventories/netbox", post(sources::create_netbox))
        .route("/inventories/{id}/sync", post(sources::sync_inventory))
        .route("/inventories/{id}/sources", post(sources::add_source))
        .route(
            "/inventories/{id}/sources/{source_id}",
            put(sources::update_source).delete(sources::delete_source),
        )
        .route(
            "/inventories/{id}/sources/{source_id}/sync",
            post(sources::sync_source),
        )
        .route(
            "/inventories/{id}",
            get(catalog::get_inventory)
                .put(catalog::update_inventory)
                .delete(catalog::delete_inventory),
        )
        .route(
            "/organizations",
            get(catalog::list_organizations).post(catalog::create_organization),
        )
        .route(
            "/organizations/{id}",
            get(catalog::get_organization).put(catalog::update_organization),
        )
        .route(
            "/credentials",
            get(catalog::list_credentials).post(catalog::create_credential),
        )
        .route(
            "/credentials/{id}",
            get(catalog::get_credential)
                .put(catalog::update_credential)
                .delete(catalog::delete_credential),
        )
        .route(
            "/environments",
            get(catalog::list_environments).post(catalog::create_environment),
        )
        .route(
            "/environments/{id}",
            get(catalog::get_environment)
                .put(catalog::update_environment)
                .delete(catalog::delete_environment),
        )
        .route("/cache", get(catalog::cache))
        .route("/plugins", get(catalog::plugins))
        .route(
            "/templates",
            get(work::list_templates).post(work::create_template),
        )
        .route(
            "/templates/{id}",
            axum::routing::put(work::update_template).delete(work::delete_template),
        )
        .route("/templates/{id}/launch", post(work::launch_template))
        .route("/jobs", get(work::list_jobs))
        .route("/jobs/{id}", get(work::get_job))
        .route("/jobs/{id}/cancel", post(work::cancel_job))
        .route("/jobs/{id}/relaunch", post(work::relaunch_job))
        .route("/jobs/{id}/output", get(work::job_output))
        .route("/jobs/{id}/events", get(work::job_events))
        .route(
            "/workflows",
            get(work::list_workflows).post(work::create_workflow),
        )
        .route(
            "/workflows/{id}",
            get(work::get_workflow)
                .put(work::update_workflow)
                .delete(work::delete_workflow),
        )
        .route("/workflows/{id}/launch", post(work::launch_workflow))
        .route("/workflow-jobs/{id}", get(work::get_workflow_job))
        .route("/hooks", get(work::list_hooks).post(work::create_hook))
        .route("/hooks/{id}", axum::routing::delete(work::delete_hook))
        .route("/hooks/{id}/test", post(work::test_hook))
        .route("/hooks/{id}/deliveries", get(work::list_deliveries))
        .merge(
            Router::new()
                .route("/admin/backup", get(admin::download).post(admin::restore))
                .layer(DefaultBodyLimit::max(256 * 1024 * 1024)),
        )
        .fallback(not_found);

    Router::new()
        .route("/healthz", get(health))
        .route("/metrics", get(metrics))
        .route("/favicon.svg", get(favicon))
        .route("/", get(index))
        .route("/app.js", get(script))
        .route("/app.css", get(style))
        .route("/hooks/{id}/{token}", post(work::ingest))
        .nest("/api", api)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

async fn metrics(State(state): State<Shared>) -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "text/plain; version=0.0.4; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        crate::metrics::scrape(&state),
    )
}

async fn not_found() -> ApiError {
    ApiError::NotFound
}

async fn favicon() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="6" fill="#1c1917"/><path d="M6 17h5l2-6 3 12 2-6h8" fill="none" stroke="#fdba74" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##,
    )
}

async fn index(State(state): State<Shared>) -> Response {
    file(
        &state.cfg.web_dir.join("index.html"),
        "text/html; charset=utf-8",
    )
    .await
}

async fn script(State(state): State<Shared>) -> Response {
    file(
        &state.cfg.web_dir.join("app.js"),
        "text/javascript; charset=utf-8",
    )
    .await
}

async fn style(State(state): State<Shared>) -> Response {
    file(
        &state.cfg.web_dir.join("app.css"),
        "text/css; charset=utf-8",
    )
    .await
}

async fn file(path: &Path, content_type: &'static str) -> Response {
    match tokio::fs::read(path).await {
        Ok(bytes) => (
            [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))],
            bytes,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Datei fehlt").into_response(),
    }
}

pub fn team_header(headers: &HeaderMap) -> AppResult<String> {
    headers
        .get("x-team-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ApiError::bad("X-Team-Id fehlt."))
}

pub fn gate(state: &Shared, user: &AuthUser, team_id: &str, min: Role) -> AppResult<()> {
    let db = state.db()?;
    if db::team_by_id(&db, team_id)?.is_none() {
        return Err(ApiError::NotFound);
    }
    auth::authorize(&db, user, team_id, min)
}

pub fn require_name(name: &str) -> AppResult<String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err(ApiError::bad("Name fehlt oder ist zu lang."));
    }
    Ok(name.to_string())
}

pub fn blank(value: Option<String>) -> Option<String> {
    value.and_then(|item| {
        let item = item.trim().to_string();
        if item.is_empty() {
            None
        } else {
            Some(item)
        }
    })
}

pub fn job_json(job: &Job, events: Option<&[JobEvent]>) -> serde_json::Value {
    let mut value = serde_json::json!({
        "id": job.id,
        "team_id": job.team_id,
        "template_id": job.template_id,
        "workflow_job_id": job.workflow_job_id,
        "workflow_node_id": job.workflow_node_id,
        "name": job.name,
        "tool": job.tool,
        "status": job.status,
        "extra_vars": db::json_object(&job.extra_vars),
        "launched_by": job.launched_by,
        "trigger": job.trigger,
        "started_at": job.started_at,
        "finished_at": job.finished_at,
        "created_at": job.created_at,
        "parameters": db::json_object(&job.parameters),
        "failed_hosts": runner::failed_host_names(&job.failed_hosts),
    });
    if let Some(events) = events {
        value["events"] = serde_json::json!(events
            .iter()
            .map(|event| serde_json::json!({
                "id": event.id,
                "ts": event.ts,
                "stream": event.stream,
                "line": event.line,
            }))
            .collect::<Vec<_>>());
    }
    value
}

pub fn created(body: serde_json::Value) -> Response {
    (StatusCode::CREATED, axum::Json(body)).into_response()
}

pub fn accepted(body: serde_json::Value) -> Response {
    (StatusCode::ACCEPTED, axum::Json(body)).into_response()
}

pub fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}
