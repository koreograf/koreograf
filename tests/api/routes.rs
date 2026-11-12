use axum::http::StatusCode;

use crate::common::{request, Options, TestApp};

#[tokio::test]
async fn each_module_registers_its_routes() {
    let harness = TestApp::boot("http-module-routes", Options::default());
    for (method, path) in [
        ("GET", "/api/maintenance"),
        ("GET", "/api/overview"),
        ("GET", "/api/assistant"),
        ("GET", "/api/schedules"),
        ("GET", "/api/me"),
        ("GET", "/api/settings"),
        ("GET", "/api/projects/missing/contents"),
        ("GET", "/api/jobs"),
    ] {
        let (status, _, _) = request(&harness.app, method, path, &[], None).await;
        assert_ne!(status, StatusCode::NOT_FOUND, "{method} {path}");
        assert_ne!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
    }
}
