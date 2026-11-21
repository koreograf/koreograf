#![allow(dead_code, clippy::expect_used, clippy::unwrap_used)]

mod helpers;

#[allow(unused_imports)]
pub use helpers::*;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use http_body_util::BodyExt;
use koreograf::{Config, CookieSecure, LdapConfig, LocalJobs, RunnerMode, Shared};
use serde_json::{json, Value};
use tower::ServiceExt;

pub const ADMIN_PASSWORD: &str = "test-admin-pass";

#[derive(Clone)]
pub struct Options {
    pub container_image: bool,
    pub mcp: bool,
    pub trusted_proxy: bool,
    pub ldap: Option<LdapConfig>,
    pub project_roots: Vec<PathBuf>,
    pub admin_user: String,
    /// `KOREOGRAF_MAX_CONCURRENT_JOBS`. Four unless a test needs a queue.
    pub max_concurrent_jobs: usize,
    /// `KOREOGRAF_MAX_CONCURRENT_JOBS_PER_TEAM`. The documented default is 2.
    pub max_concurrent_jobs_per_team: usize,
    /// `KOREOGRAF_JOB_LOG_RETENTION_DAYS`.
    pub job_log_retention_days: u64,
    /// `KOREOGRAF_WORKFLOW_RUN_RETENTION_DAYS`.
    pub workflow_run_retention_days: u64,
    /// `KOREOGRAF_HOOK_DELIVERY_RETENTION_DAYS`.
    pub hook_delivery_retention_days: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            container_image: false,
            mcp: true,
            trusted_proxy: false,
            ldap: None,
            project_roots: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")],
            admin_user: "admin".into(),
            max_concurrent_jobs: 4,
            max_concurrent_jobs_per_team: 2,
            job_log_retention_days: 30,
            workflow_run_retention_days: 90,
            hook_delivery_retention_days: 30,
        }
    }
}

pub fn config(data_dir: &Path, options: &Options) -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        data_dir: data_dir.to_path_buf(),
        data_host: None,
        web_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
        examples_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
        admin_user: options.admin_user.clone(),
        admin_password: Some(ADMIN_PASSWORD.into()),
        runner: RunnerMode::Local,
        container_image: options.container_image,
        job_timeout_secs: 60,
        job_output_lines: 8_000,
        job_log_retention_days: options.job_log_retention_days,
        workflow_run_retention_days: options.workflow_run_retention_days,
        hook_delivery_retention_days: options.hook_delivery_retention_days,
        max_concurrent_jobs: options.max_concurrent_jobs,
        max_concurrent_jobs_per_team: options.max_concurrent_jobs_per_team,
        local_jobs: LocalJobs::All,
        project_roots: options.project_roots.clone(),
        ldap: options.ldap.clone(),
        hsts: false,
        cookie_secure: CookieSecure::Auto,
        trusted_proxy: options.trusted_proxy,
        mcp: options.mcp,
        public_url: Vec::new(),
    }
}

pub fn temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "koreograf-{label}-{nanos}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("temp directory");
    dir
}

pub struct TestApp {
    pub app: axum::Router,
    pub state: Shared,
    pub dir: PathBuf,
    pub options: Options,
    pub database_url: Option<String>,
    pub database_name: Option<String>,
    pub postgres_base: Option<String>,
}

impl TestApp {
    pub fn boot(label: &str, options: Options) -> Self {
        if postgres_backend_selected() {
            let base = postgres_test_url()
                .expect("KOREOGRAF_TEST_BACKEND=postgres needs KOREOGRAF_TEST_POSTGRES_URL");
            let name = postgres_database_name(label);
            let handle = tokio::runtime::Handle::current();
            return tokio::task::block_in_place(|| {
                handle.block_on(Self::boot_postgres_at(label, options, base, name))
            });
        }
        Self::boot_sqlite(label, options)
    }

    pub fn boot_sqlite(label: &str, options: Options) -> Self {
        let dir = temp_dir(label);
        let state = koreograf::boot(config(&dir, &options)).expect("sqlite boot");
        Self {
            app: koreograf::router(state.clone()),
            state,
            dir,
            options,
            database_url: None,
            database_name: None,
            postgres_base: None,
        }
    }

    pub async fn boot_postgres(label: &str, options: Options) -> Option<Self> {
        let base = postgres_test_url()?;
        let name = postgres_database_name(label);
        Some(Self::boot_postgres_at(label, options, base, name).await)
    }

    async fn boot_postgres_at(label: &str, options: Options, base: String, name: String) -> Self {
        let database_url = {
            let _guard = postgres_guard().await;
            create_database(&base, &name).await
        };
        let database =
            koreograf::parse_database_config(&database_url, None, None).expect("postgres url");
        let dir = temp_dir(label);
        let state = koreograf::boot_with(config(&dir, &options), database).expect("postgres boot");
        Self {
            app: koreograf::router(state.clone()),
            state,
            dir,
            options,
            database_url: Some(database_url),
            database_name: Some(name),
            postgres_base: Some(base),
        }
    }

    pub fn is_postgres(&self) -> bool {
        self.database_url.is_some()
    }

    /// Boots a second time against the same data directory and database.
    pub fn reboot(&mut self) {
        self.try_reboot().expect("reboot");
    }

    pub fn try_reboot(&mut self) -> anyhow::Result<()> {
        let cfg = config(&self.dir, &self.options);
        let state = match &self.database_url {
            Some(url) => {
                let database = koreograf::parse_database_config(url, None, None)
                    .map_err(anyhow::Error::msg)?;
                koreograf::boot_with(cfg, database)?
            }
            None => koreograf::boot(cfg)?,
        };
        self.app = koreograf::router(state.clone());
        self.state = state;
        Ok(())
    }

    /// Runs raw SQL on whichever backend this app uses.
    pub async fn execute_sql(&self, sql: &str) {
        match &self.database_url {
            Some(url) => {
                postgres_client(url)
                    .await
                    .batch_execute(sql)
                    .await
                    .expect("sql");
            }
            None => {
                let conn = self.state.db().expect("sqlite");
                conn.execute_batch(sql).expect("sql");
            }
        }
    }

    pub async fn sql_i64(&self, sql: &str) -> i64 {
        match &self.database_url {
            Some(url) => postgres_client(url)
                .await
                .query_one(sql, &[])
                .await
                .expect("query")
                .get(0),
            None => {
                let conn = self.state.db().expect("sqlite");
                conn.query_row(sql, [], |row| row.get(0)).expect("query")
            }
        }
    }

    pub async fn sql_text(&self, sql: &str) -> Vec<String> {
        match &self.database_url {
            Some(url) => postgres_client(url)
                .await
                .query(sql, &[])
                .await
                .expect("query")
                .iter()
                .map(|row| row.get(0))
                .collect(),
            None => {
                let conn = self.state.db().expect("sqlite");
                let mut statement = conn.prepare(sql).expect("prepare");
                let rows = statement.query_map([], |row| row.get(0)).expect("query");
                rows.map(|row| row.expect("row")).collect()
            }
        }
    }

    pub async fn login(&self, username: &str, password: &str) -> String {
        login(&self.app, username, password).await
    }

    pub async fn sign_in(&self) -> (String, String) {
        sign_in(&self.app).await
    }

    pub async fn call(
        &self,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        team: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        call(&self.app, method, uri, cookie, team, body).await
    }

    pub async fn call_full(
        &self,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        team: Option<&str>,
        body: Option<Value>,
    ) -> Reply {
        call_full(&self.app, method, uri, cookie, team, body).await
    }

    pub async fn call_bearer(
        &self,
        method: &str,
        uri: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        call_bearer(&self.app, method, uri, token, body).await
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        if let (Some(base), Some(name)) = (self.postgres_base.clone(), self.database_name.clone()) {
            drop_database_blocking(&base, &name);
        }
    }
}

pub fn postgres_backend_selected() -> bool {
    matches!(
        std::env::var("KOREOGRAF_TEST_BACKEND").as_deref(),
        Ok("postgres")
    )
}

fn drop_database_blocking(base: &str, name: &str) {
    let base = base.to_string();
    let name = name.to_string();
    let _ = std::thread::spawn(move || {
        if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            runtime.block_on(drop_database(&base, &name));
        }
    })
    .join();
}

pub struct Reply {
    pub status: StatusCode,
    pub body: Value,
    pub headers: HeaderMap,
    pub cookie: Option<String>,
}

pub async fn request(
    app: &axum::Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder
        .body(Body::from(body.unwrap_or_default().to_vec()))
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}

pub async fn call_full(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    team: Option<&str>,
    body: Option<Value>,
) -> Reply {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        // The origin guard needs same-origin headers on every cookie write.
        builder = builder
            .header("cookie", cookie)
            .header("host", "example.test")
            .header("origin", "http://example.test");
    }
    if let Some(team) = team {
        builder = builder.header("x-team-id", team);
    }
    let request = if let Some(body) = body {
        builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request")
    } else {
        builder.body(Body::empty()).expect("request")
    };
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let cookie = headers
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::to_string);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).to_string()))
    };
    Reply {
        status,
        body,
        headers,
        cookie,
    }
}

pub async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    team: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let reply = call_full(app, method, uri, cookie, team, body).await;
    (reply.status, reply.body)
}

pub async fn call_cookie(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    team: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, Option<String>) {
    let reply = call_full(app, method, uri, cookie, team, body).await;
    (reply.status, reply.body, reply.cookie)
}

pub async fn call_bearer(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let authorization = format!("Bearer {token}");
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", authorization);
    let request = if let Some(body) = body {
        builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request")
    } else {
        builder.body(Body::empty()).expect("request")
    };
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).to_string()))
    };
    (status, body)
}

pub async fn call_bearer_team(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    team: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let bearer = format!("Bearer {token}");
    let has_body = body.is_some();
    let mut extra = vec![("authorization", bearer.as_str())];
    if has_body {
        extra.push(("content-type", "application/json"));
    }
    if let Some(team) = team {
        extra.push(("x-team-id", team));
    }
    let body = body.map(|value| value.to_string().into_bytes());
    let (status, bytes, _) = call_raw(app, method, uri, None, &extra, body).await;
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).to_string()))
    };
    (status, body)
}

pub async fn call_raw_headers(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    extra: &[(&str, &str)],
    body: Option<Vec<u8>>,
) -> (StatusCode, Vec<u8>, HeaderMap) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
        if !extra
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("host"))
        {
            builder = builder.header("host", "example.test");
        }
        if !extra.iter().any(|(key, _)| {
            key.eq_ignore_ascii_case("origin") || key.eq_ignore_ascii_case("referer")
        }) {
            builder = builder.header("origin", "http://example.test");
        }
    }
    for (key, value) in extra {
        builder = builder.header(*key, *value);
    }
    let request = match body {
        Some(body) => builder.body(Body::from(body)).expect("request"),
        None => builder.body(Body::empty()).expect("request"),
    };
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (status, bytes.to_vec(), headers)
}

pub async fn call_raw(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Vec<u8>>,
) -> (StatusCode, Vec<u8>, Option<String>) {
    let mut request_headers = Vec::new();
    if let Some(cookie) = cookie {
        request_headers.push(("cookie", cookie));
    }
    request_headers.extend_from_slice(headers);
    let (status, headers, bytes) =
        request(app, method, uri, &request_headers, body.as_deref()).await;
    let cookie = headers
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::to_string);
    (status, bytes, cookie)
}

pub async fn login(app: &axum::Router, username: &str, password: &str) -> String {
    let reply = login_reply(app, None, username, password).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.cookie.expect("session cookie")
}

pub async fn login_reply(
    app: &axum::Router,
    forwarded_for: Option<&str>,
    username: &str,
    password: &str,
) -> Reply {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/login")
        .header("content-type", "application/json");
    if let Some(source) = forwarded_for {
        builder = builder.header("x-forwarded-for", source);
    }
    let request = builder
        .body(Body::from(
            json!({ "username": username, "password": password }).to_string(),
        ))
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let cookie = headers
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::to_string);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    Reply {
        status,
        body,
        headers,
        cookie,
    }
}

pub async fn sign_in(app: &axum::Router) -> (String, String) {
    let reply = login_reply(app, None, "admin", ADMIN_PASSWORD).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let cookie = reply.cookie.expect("session cookie");
    let team = reply.body["teams"][0]["id"]
        .as_str()
        .expect("team id")
        .to_string();
    (cookie, team)
}

pub fn postgres_test_url() -> Option<String> {
    let Ok(url) = std::env::var("KOREOGRAF_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is not set");
        return None;
    };
    if url.trim().is_empty() {
        eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is empty");
        return None;
    }
    Some(url)
}

pub fn postgres_database_name(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!(
        "koreograf_{}_{}_{}",
        label.replace('-', "_"),
        nanos,
        std::process::id()
    )
}

pub fn database_url(base: &str, name: &str) -> String {
    let mut url = url::Url::parse(base).expect("postgres url");
    url.set_path(&format!("/{name}"));
    url.to_string()
}

pub async fn postgres_client(url: &str) -> tokio_postgres::Client {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("postgres connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

pub async fn create_database(base: &str, name: &str) -> String {
    let client = postgres_client(base).await;
    client
        .batch_execute(&format!("CREATE DATABASE \"{name}\""))
        .await
        .expect("create database");
    database_url(base, name)
}

pub async fn drop_database(base: &str, name: &str) {
    let client = postgres_client(base).await;
    let _ = client
        .batch_execute(&format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"))
        .await;
}

static POSTGRES_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

pub async fn postgres_guard() -> tokio::sync::MutexGuard<'static, ()> {
    POSTGRES_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}
