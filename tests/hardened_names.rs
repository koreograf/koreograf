#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use common::{login_reply, Options, TestApp};
use serde_json::json;

#[tokio::test]
async fn names_and_descriptions_reject_bidi_controls() {
    let app = TestApp::boot(
        "bidi-names",
        Options {
            mcp: false,
            ..Options::default()
        },
    );
    let (cookie, _) = app.sign_in().await;
    let marks = [
        '\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}',
        '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
    ];
    for mark in marks {
        let (status, body) = app
            .call(
                "POST",
                "/api/teams",
                Some(&cookie),
                None,
                Some(json!({ "name": format!("Team{mark}Name"), "description": "" })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{mark:?} {body}");
        assert_eq!(body["error"], "the_name_has_invalid_characters", "{mark:?}");
    }
    let (status, created) = app
        .call(
            "POST",
            "/api/teams",
            Some(&cookie),
            None,
            Some(json!({ "name": "Neutral", "description": "plain" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().expect("team id");
    let (status, body) = app
        .call(
            "PUT",
            &format!("/api/teams/{id}"),
            Some(&cookie),
            None,
            Some(json!({ "name": "Re\u{202e}named", "description": "plain" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "the_name_has_invalid_characters");
    let (status, body) = app
        .call(
            "PUT",
            &format!("/api/teams/{id}"),
            Some(&cookie),
            None,
            Some(json!({ "name": "Neutral", "description": "a\u{202e}b" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "the_value_contains_invalid_characters");
}

#[tokio::test]
async fn names_are_trimmed_and_bounded_by_characters() {
    let app = TestApp::boot(
        "name-limits",
        Options {
            mcp: false,
            ..Options::default()
        },
    );
    let (cookie, _) = app.sign_in().await;
    let (status, created) = app
        .call(
            "POST",
            "/api/teams",
            Some(&cookie),
            None,
            Some(json!({ "name": "  Trimmed  ", "description": "" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["name"], "Trimmed");

    for name in ["   ", &"x".repeat(121), &"\u{00e9}".repeat(121)] {
        let (status, body) = app
            .call(
                "POST",
                "/api/teams",
                Some(&cookie),
                None,
                Some(json!({ "name": name, "description": "" })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name:?} {body}");
        assert_eq!(body["error"], "name_is_missing_or_too_long", "{name:?}");
    }
    let (status, created) = app
        .call(
            "POST",
            "/api/teams",
            Some(&cookie),
            None,
            Some(json!({ "name": "\u{00e9}".repeat(120), "description": "" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["name"].as_str().unwrap().chars().count(), 120);

    let (status, body) = app
        .call(
            "POST",
            "/api/teams",
            Some(&cookie),
            None,
            Some(json!({ "name": "a\u{7}b", "description": "" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "the_name_has_invalid_characters");
}

#[tokio::test]
async fn a_disabled_account_gets_the_generic_sign_in_failure() {
    let app = TestApp::boot(
        "disabled-signin",
        Options {
            mcp: false,
            ..Options::default()
        },
    );
    let (cookie, _) = app.sign_in().await;
    let (status, created) = app
        .call(
            "POST",
            "/api/users",
            Some(&cookie),
            None,
            Some(json!({
                "username": "ada",
                "password": "ada-secret-1",
                "display_name": "Ada",
                "is_admin": false
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().expect("user id");
    let (status, disabled) = app
        .call(
            "PUT",
            &format!("/api/users/{id}"),
            Some(&cookie),
            None,
            Some(json!({ "display_name": "Ada", "is_admin": false, "disabled": true })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{disabled}");
    assert_eq!(disabled["disabled"], true);

    for (username, password) in [
        ("ada", "ada-secret-1"),
        ("ada", "wrong-password"),
        ("nobody", "wrong-password"),
    ] {
        let reply = login_reply(&app.app, None, username, password).await;
        let (status, body) = (reply.status, reply.body);
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{username} {body}");
        assert_eq!(body["error"], "invalid_username_or_password", "{username}");
        assert!(body.get("vars").is_none(), "{username}");
    }
}
