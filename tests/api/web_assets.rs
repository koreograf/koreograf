use std::os::unix::fs::symlink;
use std::path::PathBuf;

use axum::http::{header, HeaderMap, StatusCode};

use crate::common::{config, request, temp_dir, Options, TestApp};

fn header_value(headers: &HeaderMap, name: header::HeaderName) -> &str {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .expect("header")
}

fn assert_security_headers(headers: &HeaderMap, path: &str) {
    assert_eq!(
        header_value(headers, header::X_CONTENT_TYPE_OPTIONS),
        "nosniff",
        "{path}"
    );
    assert_eq!(
        header_value(headers, header::X_FRAME_OPTIONS),
        "DENY",
        "{path}"
    );
    assert!(
        header_value(headers, header::CONTENT_SECURITY_POLICY).contains("default-src 'self'"),
        "{path}"
    );
}

fn temp_web_app(label: &str) -> (axum::Router, PathBuf) {
    let base = temp_dir(label);
    let web = base.join("web");
    let data = base.join("data");
    std::fs::create_dir_all(&web).expect("web directory");
    std::fs::create_dir_all(&data).expect("data directory");
    let mut cfg = config(&data, &Options::default());
    cfg.web_dir = web;
    let state = koreograf::boot(cfg).expect("boot");
    (koreograf::router(state), base)
}

#[tokio::test]
async fn web_assets_send_types_etags_and_security_headers() {
    let harness = TestApp::boot("assets", Options::default());
    for (path, content_type) in [
        ("/js/features/jobs.js", "text/javascript; charset=utf-8"),
        ("/app.css", "text/css; charset=utf-8"),
        ("/i18n/de.json", "application/json; charset=utf-8"),
        ("/i18n/en.json", "application/json; charset=utf-8"),
        ("/index.html", "text/html; charset=utf-8"),
    ] {
        let (status, headers, body) = request(&harness.app, "GET", path, &[], None).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(!body.is_empty(), "{path}");
        assert_eq!(
            header_value(&headers, header::CONTENT_TYPE),
            content_type,
            "{path}"
        );
        assert_eq!(
            header_value(&headers, header::CACHE_CONTROL),
            "no-cache",
            "{path}"
        );
        let etag = header_value(&headers, header::ETAG);
        assert!(
            etag.starts_with('"') && etag.ends_with('"'),
            "{path}: {etag}"
        );
        assert_security_headers(&headers, path);

        let (status, cached, body) =
            request(&harness.app, "GET", path, &[("if-none-match", etag)], None).await;
        assert_eq!(status, StatusCode::NOT_MODIFIED, "{path}");
        assert!(body.is_empty(), "{path}");
        assert_eq!(header_value(&cached, header::ETAG), etag, "{path}");
        assert_eq!(
            header_value(&cached, header::CACHE_CONTROL),
            "no-cache",
            "{path}"
        );
        assert_security_headers(&cached, path);

        let (status, _, body) = request(
            &harness.app,
            "GET",
            path,
            &[("if-none-match", "\"stale\"")],
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(!body.is_empty(), "{path}");
    }
}

#[tokio::test]
async fn feature_js_is_compressed_with_gzip_and_brotli() {
    let harness = TestApp::boot("assets-compressed", Options::default());
    let (status, headers, body) = request(
        &harness.app,
        "GET",
        "/js/features/jobs.js",
        &[("accept-encoding", "gzip")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_value(&headers, header::CONTENT_ENCODING), "gzip");
    assert!(
        body.len() <= 90 * 1024,
        "gzip feature module is {} bytes",
        body.len()
    );
    assert!(
        header_value(&headers, header::VARY).contains("accept-encoding"),
        "missing vary header"
    );
    assert_security_headers(&headers, "/js/features/jobs.js gzip");

    let (status, headers, body) = request(
        &harness.app,
        "GET",
        "/js/features/jobs.js",
        &[("accept-encoding", "br")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_value(&headers, header::CONTENT_ENCODING), "br");
    assert!(
        body.len() <= 90 * 1024,
        "brotli feature module is {} bytes",
        body.len()
    );
    assert_security_headers(&headers, "/js/features/jobs.js brotli");
}

#[tokio::test]
async fn nested_files_and_every_allowed_extension_are_served() {
    let (app, base) = temp_web_app("assets-types");
    let web = base.join("web");
    let files = [
        ("nested/script.js", "text/javascript; charset=utf-8"),
        ("nested/style.css", "text/css; charset=utf-8"),
        ("nested/data.json", "application/json; charset=utf-8"),
        ("nested/icon.svg", "image/svg+xml"),
        ("nested/page.html", "text/html; charset=utf-8"),
    ];
    for (name, _) in files {
        let path = web.join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
        std::fs::write(&path, b"asset body").expect("asset");
    }
    std::fs::write(web.join("index.html"), b"<html></html>").expect("index");
    std::fs::write(web.join("notes.txt"), b"not served").expect("text");

    for (name, content_type) in files {
        let path = format!("/{name}");
        let (status, headers, body) = request(&app, "GET", &path, &[], None).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(body, b"asset body", "{path}");
        assert_eq!(
            header_value(&headers, header::CONTENT_TYPE),
            content_type,
            "{path}"
        );
    }

    let (status, headers, body) = request(&app, "GET", "/", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"<html></html>");
    assert_eq!(
        header_value(&headers, header::CONTENT_TYPE),
        "text/html; charset=utf-8"
    );

    for path in ["/notes.txt", "/nested/missing.js", "/nested"] {
        let (status, _, _) = request(&app, "GET", path, &[], None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }

    drop(app);
    let _ = std::fs::remove_dir_all(base);
}

#[tokio::test]
async fn traversal_and_symlinks_outside_the_web_directory_are_rejected() {
    let (app, base) = temp_web_app("assets-escape");
    let web = base.join("web");
    std::fs::write(web.join("ok.js"), b"ok").expect("asset");
    std::fs::write(base.join("secret.js"), b"secret").expect("secret");
    symlink(base.join("secret.js"), web.join("escape.js")).expect("escape symlink");
    symlink(web.join("ok.js"), web.join("inside.js")).expect("inside symlink");

    for path in [
        "/../secret.js",
        "/%2e%2e/secret.js",
        "/..%2fsecret.js",
        "/%2Fetc%2Fpasswd.js",
        "/escape.js",
        "/missing.js",
    ] {
        let (status, _, _) = request(&app, "GET", path, &[], None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }

    let (status, _, body) = request(&app, "GET", "/inside.js", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"ok");

    drop(app);
    let _ = std::fs::remove_dir_all(base);
}
