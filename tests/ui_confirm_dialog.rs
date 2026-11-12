#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

fn function<'a>(script: &'a str, name: &str) -> &'a str {
    let start = script
        .find(&format!("function {name}("))
        .unwrap_or_else(|| panic!("{name} is missing"));
    let rest = &script[start + 1..];
    let end = rest
        .find("\n  function ")
        .or_else(|| rest.find("\nfunction "))
        .unwrap_or(rest.len());
    &script[start..start + 1 + end]
}

#[test]
fn web_ui_has_no_native_confirms() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    assert!(
        !script.contains("window.confirm"),
        "app.js still calls window.confirm"
    );
    assert!(
        !script.contains("confirm("),
        "app.js still calls the native confirm"
    );
}

#[test]
fn the_confirm_helper_uses_the_shared_overlay() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let helper = function(&script, "confirmDialog");
    assert!(
        helper.contains("new Promise"),
        "confirmDialog does not return a Promise"
    );
    assert!(
        helper.contains("openDialog({"),
        "confirmDialog has its own overlay"
    );
    assert!(
        helper.contains("onClose: () => finish(false)"),
        "closing does not resolve false"
    );
    assert!(
        helper.contains("finish(true)"),
        "confirming does not resolve true"
    );
    assert!(
        helper.contains("button.focus()"),
        "the confirm button does not receive focus"
    );
    assert!(
        helper.contains("danger"),
        "confirmDialog has no danger button"
    );

    let modal = function(&script, "makeModal");
    assert!(
        modal.contains("await close()"),
        "makeModal cannot wait for an async close"
    );
    assert!(
        script.contains("confirmDialog({ title: t(\"discard_unsaved_changes\")"),
        "closing a dirty dialog does not use the in-app confirm"
    );
}

#[test]
fn the_default_confirm_title_is_in_both_catalogs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for language in ["de", "en"] {
        let text = std::fs::read_to_string(root.join(format!("web/i18n/{language}.json")))
            .expect("catalog");
        let value: serde_json::Value = serde_json::from_str(&text).expect("catalog json");
        let entry = value
            .as_object()
            .and_then(|object| object.get("confirm"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(
            !entry.trim().is_empty(),
            "confirm is missing from {language}.json"
        );
    }
}
