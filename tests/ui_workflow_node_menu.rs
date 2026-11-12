#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

fn web_file(name: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(root.join("web").join(name)).expect(name)
}

fn function<'a>(script: &'a str, name: &str) -> &'a str {
    let start = script
        .find(&format!("function {name}("))
        .unwrap_or_else(|| panic!("{name} is missing"));
    let rest = &script[start + 1..];
    let end = rest.find("\n  function ").unwrap_or(rest.len());
    &script[start..start + 1 + end]
}

#[test]
fn workflow_nodes_use_canvas_and_node_context_menus() {
    let script = web_file("app.js");
    let add = function(&script, "addNodeControls");
    assert!(
        !add.contains("add-template"),
        "the inspector still has a template select"
    );
    assert!(
        add.contains("id=\"add-node\""),
        "the inspector has no keyboard-reachable Add node button"
    );
    let bind_add = function(&script, "bindAddNode");
    assert!(
        bind_add.contains("openAddNodeDialog(null)"),
        "the Add node button does not open the overlay"
    );

    let canvas = function(&script, "openCanvasMenu");
    assert!(
        canvas.contains("data-add-node"),
        "the canvas menu has no Add node item"
    );

    let menu = function(&script, "openNodeMenu");
    for item in ["data-edit-node", "data-delete-node", "data-connect-node"] {
        assert!(menu.contains(item), "the node menu has no {item} item");
    }

    let add_dialog = function(&script, "openAddNodeDialog");
    assert!(
        add_dialog.contains("openDialog({"),
        "Add node does not use the shared overlay"
    );
    assert!(
        add_dialog.contains("id=\"add-template\""),
        "Add node has no template picker"
    );
    assert!(
        add_dialog.contains("name_optional"),
        "Add node has no optional name field"
    );

    let editor = function(&script, "openNodeEditor");
    assert!(
        editor.contains("openDialog({"),
        "Edit node does not use the shared overlay"
    );
    assert!(
        editor.contains("readOverrides(draft, form)"),
        "Edit node skips the override rules"
    );
    let fields = function(&script, "nodeEditorFields");
    assert!(
        fields.contains("nodeParams("),
        "Edit node has no parameter fields"
    );
    assert!(
        fields.contains("id=\"node-join\""),
        "Edit node has no convergence field"
    );

    let inspector = function(&script, "renderInspector");
    assert!(
        !inspector.contains("id=\"add-template\""),
        "the inspector still renders Add node"
    );
    assert!(
        inspector.contains("id=\"edge-when\""),
        "the inspector lost edge editing"
    );
    assert!(
        inspector.contains("id=\"node-connect\""),
        "the inspector lost Connect"
    );
}

#[test]
fn workflow_node_menu_copy_is_in_both_catalogs() {
    for language in ["de", "en"] {
        let text = web_file(&format!("i18n/{language}.json"));
        let value: serde_json::Value = serde_json::from_str(&text).expect("catalog json");
        let object = value.as_object().expect("catalog object");
        for key in [
            "edit_node",
            "name_optional",
            "right_click_empty_canvas_to_add_a_node",
        ] {
            let entry = object
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            assert!(
                !entry.trim().is_empty(),
                "{key} is missing from {language}.json"
            );
        }
    }
}

#[test]
fn workflow_docs_describe_the_node_menus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let docs =
        std::fs::read_to_string(root.join("docs/workflows-webhooks.md")).expect("workflow docs");
    assert!(
        docs.contains("Right-click empty canvas to add a node"),
        "the workflow docs do not describe adding a node"
    );
    assert!(
        docs.contains("edit, delete, or connect"),
        "the workflow docs do not describe the node menu"
    );
    assert!(
        !docs.contains("Add node stays in the side panel"),
        "the workflow docs still send Add node to the inspector"
    );
}
