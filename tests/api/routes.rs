use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::ServiceExt;

use crate::common::{call_bearer, call_cookie, request, Options, TestApp};

const API_PREFIXES: [&str; 2] = ["/api", "/api/v1"];

async fn write_team(
    app: &axum::Router,
    prefix: &str,
    cookie: &str,
    name: &str,
    origin: Option<&str>,
) -> StatusCode {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("{prefix}/teams"))
        .header("cookie", cookie)
        .header("content-type", "application/json")
        .header("host", "example.test");
    if let Some(origin) = origin {
        builder = builder.header("origin", origin);
    }
    let body = json!({ "name": name }).to_string();
    app.clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test(flavor = "multi_thread")]
async fn each_module_registers_its_routes() {
    let harness = TestApp::boot("http-module-routes", Options::default());
    let (status, _, health) = request(&harness.app, "GET", "/healthz", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let engine = if harness.is_postgres() {
        "postgresql"
    } else {
        "sqlite"
    };
    assert_eq!(String::from_utf8_lossy(&health), format!("ok ({engine})"));
    for prefix in API_PREFIXES {
        for (method, path) in [
            ("GET", "/maintenance"),
            ("GET", "/overview"),
            ("GET", "/cache"),
            ("DELETE", "/cache"),
            ("GET", "/assistant"),
            ("GET", "/schedules"),
            ("GET", "/me"),
            ("GET", "/settings"),
            ("GET", "/projects/missing/contents"),
            ("GET", "/jobs"),
        ] {
            let uri = format!("{prefix}{path}");
            let (status, _, _) = request(&harness.app, method, &uri, &[], None).await;
            assert_ne!(status, StatusCode::NOT_FOUND, "{method} {uri}");
            assert_ne!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {uri}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn both_api_prefixes_share_login_reads_writes_bearer_tokens_and_404() {
    let harness = TestApp::boot("http-prefixes", Options::default());
    let app = harness.app.clone();

    for prefix in API_PREFIXES {
        let (status, me, cookie) = call_cookie(
            &app,
            "POST",
            &format!("{prefix}/login"),
            None,
            None,
            Some(json!({"username": "admin", "password": "test-admin-pass"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{prefix}: {me}");
        assert_eq!(me["username"], "admin", "{prefix}: {me}");
        let cookie = cookie.unwrap_or_else(|| panic!("{prefix}: session cookie"));
        let team = me["teams"][0]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{prefix}: team id"))
            .to_string();

        let (status, read, _) = call_cookie(
            &app,
            "GET",
            &format!("{prefix}/me"),
            Some(&cookie),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{prefix}: {read}");
        assert_eq!(read["username"], "admin", "{prefix}: {read}");

        assert_eq!(
            write_team(&app, prefix, &cookie, &format!("{prefix} no origin"), None).await,
            StatusCode::FORBIDDEN,
            "{prefix}: cookie write without Origin"
        );
        assert_eq!(
            write_team(
                &app,
                prefix,
                &cookie,
                &format!("{prefix} same origin"),
                Some("http://example.test")
            )
            .await,
            StatusCode::CREATED,
            "{prefix}: write with same Origin"
        );
        assert_eq!(
            write_team(
                &app,
                prefix,
                &cookie,
                &format!("{prefix} foreign origin"),
                Some("http://other.test")
            )
            .await,
            StatusCode::FORBIDDEN,
            "{prefix}: write with foreign Origin"
        );

        let (status, created, _) = call_cookie(
            &app,
            "POST",
            &format!("{prefix}/me/tokens"),
            Some(&cookie),
            None,
            Some(json!({"name": format!("token {prefix}")})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{prefix}: {created}");
        let token = created["secret"]
            .as_str()
            .unwrap_or_else(|| panic!("{prefix}: token secret"));
        let (status, bearer) = call_bearer(&app, "GET", &format!("{prefix}/me"), token, None).await;
        assert_eq!(status, StatusCode::OK, "{prefix}: {bearer}");
        assert_eq!(bearer["username"], "admin", "{prefix}: {bearer}");

        let (status, missing, _) = call_cookie(
            &app,
            "GET",
            &format!("{prefix}/projects/missing"),
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{prefix}: {missing}");
        assert_eq!(
            missing,
            json!({"error": "not_found"}),
            "{prefix}: {missing}"
        );
    }
}
