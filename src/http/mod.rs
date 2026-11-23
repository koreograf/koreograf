use std::net::{IpAddr, SocketAddr};
use std::path::{Component, Path, PathBuf};

use axum::extract::{Path as UrlPath, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::Json;
use axum::Router;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tower_http::compression::CompressionLayer;
use tower_http::set_header::response::SetMultipleResponseHeadersLayer;
use tower_http::trace::TraceLayer;

/// Style attributes stay inline. `style-src 'self'` alone would block them.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; img-src 'self' data:; style-src 'self'; style-src-attr 'unsafe-inline'; script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

use crate::db::{self, Job, JobEvent};
use crate::error::{ApiError, AppResult};
use crate::features::identity as auth;
use crate::features::identity::{AuthUser, Role};
use crate::runner;
use crate::Shared;

pub(crate) fn warn_public_urls(cfg: &crate::config::Config) {
    for item in &cfg.public_url {
        if parse_origin(item).is_none() {
            tracing::warn!(
                value = %item,
                "KOREOGRAF_PUBLIC_URL entry is not an absolute URL with a scheme, host, and port; it is ignored"
            );
        }
    }
}

fn bare_host(host: &str) -> String {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

fn parse_origin(value: &str) -> Option<(String, String, u16)> {
    let url = url::Url::parse(value).ok()?;
    let host = url.host_str()?;
    let port = url.port_or_known_default()?;
    Some((url.scheme().to_ascii_lowercase(), bare_host(host), port))
}

fn authority_origin(value: &str, scheme: Option<&str>) -> Option<(String, String, u16)> {
    let authority: axum::http::uri::Authority = value.parse().ok()?;
    let scheme = scheme
        .and_then(|item| item.split(',').next())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .unwrap_or("http")
        .to_ascii_lowercase();
    let port = authority
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });
    Some((scheme, bare_host(authority.host()), port))
}

fn same_origin(expected: &(String, String, u16), actual: &(String, String, u16)) -> bool {
    expected == actual
}

fn maintenance_allowed(method: &Method, path: &str) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
        || matches!(
            path,
            "/api/login" | "/api/logout" | "/api/v1/login" | "/api/v1/logout"
        )
}

async fn maintenance_guard(State(state): State<Shared>, req: Request, next: Next) -> Response {
    if !state.in_maintenance() || maintenance_allowed(req.method(), req.uri().path()) {
        return next.run(req).await;
    }
    ApiError::unavailable("maintenance_mode_is_active").into_response()
}

fn allowed_origin(state: &Shared, headers: &HeaderMap, source: &HeaderValue) -> bool {
    let Some(actual) = source.to_str().ok().and_then(parse_origin) else {
        return false;
    };
    let allowed = state
        .cfg
        .public_url
        .iter()
        .filter_map(|item| parse_origin(item))
        .collect::<Vec<_>>();
    if !allowed.is_empty() {
        return allowed.iter().any(|item| same_origin(item, &actual));
    }
    let host = if state.cfg.trusted_proxy {
        headers
            .get("x-forwarded-host")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
    } else {
        None
    }
    .or_else(|| {
        headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
    });
    let scheme = if state.cfg.trusted_proxy {
        headers
            .get("x-forwarded-proto")
            .and_then(|value| value.to_str().ok())
    } else {
        None
    };
    host.and_then(|item| authority_origin(item, scheme))
        .is_some_and(|expected| same_origin(&expected, &actual))
}

/// A live session cookie identifies a browser client. The bearer token is
/// resolved first on every route, so it wins over the cookie here too.
fn session_authenticated(state: &Shared, headers: &HeaderMap) -> bool {
    let Some(token) = auth::read_cookie(headers, "koreograf_session") else {
        return false;
    };
    matches!(
        state.store().user_by_session(&token),
        Ok(Some(user)) if !user.disabled
    )
}

async fn origin_guard(State(state): State<Shared>, req: Request, next: Next) -> Response {
    if !matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) {
        return next.run(req).await;
    }
    let source = req
        .headers()
        .get(header::ORIGIN)
        .or_else(|| req.headers().get(header::REFERER));
    if let Some(source) = source {
        // A header that is present must always match, even without a session.
        if allowed_origin(&state, req.headers(), source) {
            return next.run(req).await;
        }
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "cross_origin_request_is_not_allowed"})),
        )
            .into_response();
    }
    // Only a browser client must prove its origin when both headers are absent.
    if auth::bearer_token(req.headers()).is_some() || !session_authenticated(&state, req.headers())
    {
        return next.run(req).await;
    }
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({"error": "cross_origin_request_is_not_allowed"})),
    )
        .into_response()
}

fn api_routes() -> Router<Shared> {
    Router::new()
        .route("/version", get(version))
        .merge(crate::features::activity::routes())
        .merge(crate::features::assistant::routes())
        .merge(crate::features::credentials::routes())
        .merge(crate::features::identity::routes())
        .merge(crate::features::inventories::routes())
        .merge(crate::features::jobs::routes())
        .merge(crate::features::projects::routes())
        .merge(crate::features::schedules::routes())
        .merge(crate::features::service_accounts::routes())
        .merge(crate::features::workflows::routes())
        .merge(crate::features::operations::routes())
        .fallback(not_found)
}

fn api_prefixes(routes: Router<Shared>) -> Router<Shared> {
    Router::new()
        .nest("/api/v1", routes.clone())
        .nest("/api", routes)
}

fn is_safe_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Safe routes with a protected side effect or key material.
fn is_protected_read(path: &str) -> bool {
    path.ends_with("/admin/backup") || path.ends_with("/runner/preflight")
}

/// A read-only personal token may inspect the API but not change it. This runs
/// before the route so a rejected request never touches `last_used_at`.
async fn read_only_guard(State(state): State<Shared>, request: Request, next: Next) -> Response {
    if is_safe_method(request.method()) && !is_protected_read(request.uri().path()) {
        return next.run(request).await;
    }
    let Some(token) = auth::bearer_token(request.headers()) else {
        return next.run(request).await;
    };
    match auth::api_token_read_only(&state, &token) {
        Ok(true) => ApiError::ReadOnlyToken.into_response(),
        Ok(false) => next.run(request).await,
        Err(error) => error.into_response(),
    }
}

fn guarded_api_routes(state: Shared) -> Router<Shared> {
    api_routes()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            origin_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(state, read_only_guard))
}

pub(crate) fn api_router(state: Shared) -> Router {
    api_prefixes(guarded_api_routes(state.clone())).with_state(state)
}

fn mcp_router(state: Shared) -> Router<Shared> {
    let service_state = state.clone();
    let service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        move || {
            Ok(crate::features::assistant::Server::http(
                service_state.clone(),
            ))
        },
        std::sync::Arc::new(
            rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(),
        ),
        rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default()
            .disable_allowed_hosts(),
    );
    Router::new()
        .route_service("/mcp", service)
        .route_layer(axum::middleware::from_fn_with_state(state, mcp_auth))
}

async fn mcp_auth(
    State(state): State<Shared>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let Some(token) = auth::bearer_token(&headers) else {
        return ApiError::Unauthorized.into_response();
    };
    // A live read token may open the MCP session and call read tools. The API
    // guard behind `Server::http` still refuses every write with ReadOnlyToken.
    // Do not touch `last_used_at` here for a read token, so a refused write
    // leaves the row unchanged; an accepted API call touches it.
    match auth::api_token_read_only(&state, &token) {
        Ok(true) => return next.run(request).await,
        Ok(false) => {}
        Err(error) => return error.into_response(),
    }
    match auth::api_token_user(&state, &token) {
        Ok(Some(_)) => next.run(request).await,
        Ok(None) => ApiError::Unauthorized.into_response(),
        Err(error) => error.into_response(),
    }
}

pub fn router(state: Shared) -> Router {
    let mut app = Router::new()
        .route("/healthz", get(health))
        .route("/metrics", get(metrics))
        .route("/favicon.svg", get(favicon))
        .route("/i18n/de.json", get(catalog_de))
        .route("/i18n/en.json", get(catalog_en))
        .route("/", get(index))
        .route("/{*path}", any(asset))
        .route(
            "/hooks/{id}/{token}",
            post(crate::features::workflows::ingest),
        )
        .merge(api_prefixes(guarded_api_routes(state.clone())));
    if state.cfg.mcp {
        app = app.merge(mcp_router(state.clone()));
    }
    app = app.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        maintenance_guard,
    ));
    app.layer(TraceLayer::new_for_http())
        .layer(response_headers(state.cfg.hsts))
        .layer(CompressionLayer::new())
        .with_state(state)
}

fn response_headers(hsts: bool) -> SetMultipleResponseHeadersLayer<Response> {
    let hsts = hsts.then(|| HeaderValue::from_static("max-age=31536000"));
    SetMultipleResponseHeadersLayer::if_not_present(vec![
        (
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        )
            .into(),
        (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")).into(),
        (
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        )
            .into(),
        (
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        )
            .into(),
        (
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
        )
            .into(),
        (header::STRICT_TRANSPORT_SECURITY, hsts).into(),
    ])
}

async fn version() -> Json<Value> {
    Json(crate::version_payload())
}

async fn health(State(state): State<Shared>) -> Response {
    match state.store().ping() {
        Ok(()) => (StatusCode::OK, format!("ok ({})", state.engine())).into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("database unavailable ({})", state.engine()),
        )
            .into_response(),
    }
}

async fn metrics(State(state): State<Shared>, req: Request) -> Response {
    // The peer comes from the TCP connection. Forwarded headers must not grant
    // the loopback exception, so a proxy in front of Koreograf still needs the
    // token for a remote scrape.
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|info| info.0);
    if !metrics_allowed(&state, peer, req.headers()) {
        return ApiError::Unauthorized.into_response();
    }
    (
        [
            (
                header::CONTENT_TYPE,
                "text/plain; version=0.0.4; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        crate::features::operations::scrape(&state),
    )
        .into_response()
}

fn is_loopback_peer(address: SocketAddr) -> bool {
    match address.ip() {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| mapped.is_loopback())
        }
    }
}

fn metrics_allowed(state: &Shared, peer: Option<SocketAddr>, headers: &HeaderMap) -> bool {
    let Some(expected) = crate::config::metrics_token() else {
        // Without a token, only an in-process or sidecar scrape on loopback
        // works. An unknown peer fails closed.
        return peer.is_some_and(is_loopback_peer);
    };
    if auth::bearer_token(headers)
        .is_some_and(|token| crate::crypto::tokens_equal(&token, &expected))
    {
        return true;
    }
    let Some(token) = auth::read_cookie(headers, "koreograf_session") else {
        return false;
    };
    match state.store().user_by_session(&token) {
        Ok(Some(user)) => user.is_admin && !user.disabled,
        _ => false,
    }
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

async fn index(State(state): State<Shared>, headers: HeaderMap) -> Response {
    asset_response(&state.cfg.web_dir, "index.html", &headers).await
}

async fn catalog_de(State(state): State<Shared>, headers: HeaderMap) -> Response {
    catalog_response(&state, "de", &headers)
}

async fn catalog_en(State(state): State<Shared>, headers: HeaderMap) -> Response {
    catalog_response(&state, "en", &headers)
}

fn catalog_response(state: &Shared, language: &str, request_headers: &HeaderMap) -> Response {
    let Some(bytes) = state.catalogs.get(language) else {
        return asset_not_found();
    };
    bytes_response(bytes, "application/json; charset=utf-8", request_headers)
}

async fn asset(
    State(state): State<Shared>,
    method: Method,
    UrlPath(path): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    if !matches!(method, Method::GET | Method::HEAD) {
        return asset_not_found();
    }
    asset_response(&state.cfg.web_dir, &path, &headers).await
}

fn asset_not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not found.").into_response()
}

async fn asset_response(web_dir: &Path, requested: &str, request_headers: &HeaderMap) -> Response {
    let Some(path) = asset_path(web_dir, requested).await else {
        return asset_not_found();
    };
    let Some(content_type) = asset_content_type(&path) else {
        return asset_not_found();
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => return asset_not_found(),
    };
    bytes_response(&bytes, content_type, request_headers)
}

fn bytes_response(
    bytes: &[u8],
    content_type: &'static str,
    request_headers: &HeaderMap,
) -> Response {
    let etag = format!("\"{}\"", hex::encode(Sha256::digest(bytes)));
    let Ok(etag_header) = HeaderValue::from_str(&etag) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let headers = [
        (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
        (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        (header::ETAG, etag_header),
    ];
    if if_none_match(request_headers, &etag) {
        return (StatusCode::NOT_MODIFIED, headers).into_response();
    }
    (headers, bytes.to_vec()).into_response()
}

async fn asset_path(web_dir: &Path, requested: &str) -> Option<PathBuf> {
    let relative = Path::new(requested);
    if requested.is_empty() || requested.starts_with('/') {
        return None;
    }
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    asset_content_type(relative)?;
    let root = tokio::fs::canonicalize(web_dir).await.ok()?;
    let path = tokio::fs::canonicalize(root.join(relative)).await.ok()?;
    if !path.starts_with(&root) || !path.is_file() {
        return None;
    }
    asset_content_type(&path)?;
    Some(path)
}

fn asset_content_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "js" => Some("text/javascript; charset=utf-8"),
        "css" => Some("text/css; charset=utf-8"),
        "json" => Some("application/json; charset=utf-8"),
        "svg" => Some("image/svg+xml"),
        "html" => Some("text/html; charset=utf-8"),
        _ => None,
    }
}

fn if_none_match(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|candidate| {
            let candidate = candidate.trim();
            candidate == "*" || candidate == etag || candidate.strip_prefix("W/") == Some(etag)
        })
}

pub fn team_header(headers: &HeaderMap) -> AppResult<String> {
    headers
        .get("x-team-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ApiError::bad("x_team_id_is_missing"))
}

pub fn gate(state: &Shared, user: &AuthUser, team_id: &str, min: Role) -> AppResult<()> {
    let store = state.store();
    if store.team_by_id(team_id)?.is_none() {
        return Err(ApiError::NotFound);
    }
    auth::authorize(store, user, team_id, min)
}

pub fn require_name(name: &str) -> AppResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(ApiError::bad("name_is_missing_or_too_long"));
    }
    if name.chars().any(char::is_control) || has_bidi_control(name) {
        return Err(ApiError::bad("the_name_has_invalid_characters"));
    }
    Ok(name.to_string())
}

fn has_bidi_control(value: &str) -> bool {
    value.chars().any(|ch| {
        matches!(
            ch,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
    })
}

/// A static INI body with a broken line is rejected before it is stored.
pub fn check_static_ini(body: &str) -> AppResult<()> {
    if let Some(line) = crate::features::inventories::find_ini_error(body) {
        return Err(ApiError::bad_vars(
            "inventory_ini_is_invalid",
            [("line", line.to_string())],
        ));
    }
    Ok(())
}

pub fn require_description(description: &str) -> AppResult<String> {
    let description = description.trim();
    if description.chars().count() > 1000 {
        return Err(ApiError::bad("description_is_too_long"));
    }
    if has_bidi_control(description) {
        return Err(ApiError::bad("the_value_contains_invalid_characters"));
    }
    Ok(description.to_string())
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

pub fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
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

pub fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use axum::http::Method;

    use super::{is_loopback_peer, maintenance_allowed, require_description, require_name};
    use std::net::SocketAddr;

    #[test]
    fn only_loopback_peers_pass_the_default_metrics_check() {
        for allowed in [
            "127.0.0.1:49152",
            "127.10.20.30:49152",
            "[::1]:49152",
            "[::ffff:127.0.0.1]:49152",
        ] {
            let address: SocketAddr = allowed.parse().expect("peer");
            assert!(is_loopback_peer(address), "{allowed}");
        }
        for denied in [
            "10.0.0.1:49152",
            "192.0.2.10:49152",
            "203.0.113.7:49152",
            "[2001:db8::1]:49152",
            "[::ffff:10.0.0.1]:49152",
        ] {
            let address: SocketAddr = denied.parse().expect("peer");
            assert!(!is_loopback_peer(address), "{denied}");
        }
    }

    #[test]
    fn maintenance_keeps_both_sign_in_paths_open() {
        for prefix in ["/api", "/api/v1"] {
            assert!(maintenance_allowed(
                &Method::POST,
                &format!("{prefix}/login")
            ));
            assert!(maintenance_allowed(
                &Method::POST,
                &format!("{prefix}/logout")
            ));
            assert!(!maintenance_allowed(&Method::POST, &format!("{prefix}/me")));
        }
        assert!(maintenance_allowed(&Method::GET, "/api/v1/jobs"));
    }

    #[test]
    fn names_and_descriptions_count_characters() {
        let mark = "é";
        assert_eq!(mark.len(), 2);
        assert!(require_name(&mark.repeat(120)).is_ok());
        assert!(require_name(&mark.repeat(121))
            .unwrap_err()
            .to_string()
            .contains("name_is_missing_or_too_long"));
        assert_eq!(require_description("  ").unwrap(), "");
        let ok = require_description(&mark.repeat(1000)).unwrap();
        assert_eq!(ok.chars().count(), 1000);
        assert!(ok.len() > 1000);
        assert!(require_description(&mark.repeat(1001))
            .unwrap_err()
            .to_string()
            .contains("description_is_too_long"));
    }

    #[test]
    fn names_reject_control_characters() {
        assert!(require_name("plain name").is_ok());
        assert!(require_name("a\0name")
            .unwrap_err()
            .to_string()
            .contains("the_name_has_invalid_characters"));
        assert!(require_name("a\u{7}name")
            .unwrap_err()
            .to_string()
            .contains("the_name_has_invalid_characters"));
        for mark in [
            '\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}',
            '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ] {
            let name = format!("team{mark}name");
            assert!(require_name(&name)
                .unwrap_err()
                .to_string()
                .contains("the_name_has_invalid_characters"));
        }
        assert!(require_name("")
            .unwrap_err()
            .to_string()
            .contains("name_is_missing_or_too_long"));
    }

    #[test]
    fn descriptions_reject_bidi_controls() {
        assert_eq!(
            require_description("plain description").unwrap(),
            "plain description"
        );
        assert!(require_description("a\u{202e}b")
            .unwrap_err()
            .to_string()
            .contains("the_value_contains_invalid_characters"));
    }
}
