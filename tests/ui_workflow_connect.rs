use std::path::Path;
use std::process::Command;

const CONNECT_PROBE: &str = r##"
const js = __JS__;
const css = __CSS__;
function check(ok, message) {
  if (!ok) { console.error(message); process.exit(1); }
}
function extractFunction(name) {
  const start = js.indexOf(`function ${name}(`);
  check(start >= 0, `${name} is missing`);
  const open = js.indexOf("{", start);
  let depth = 0;
  for (let index = open; index < js.length; index += 1) {
    if (js[index] === "{") depth += 1;
    else if (js[index] === "}") {
      depth -= 1;
      if (depth === 0) return js.slice(start, index + 1);
    }
  }
  check(false, `${name} has no closing brace`);
}
const nodes = extractFunction("renderNodes");
check(!nodes.includes("data-port"), "the node card still has the old port buttons");
check(!nodes.includes('class="ports"'), "the node card still renders the port row");
check(nodes.includes("data-node-menu"), "the node has no connect menu button");
check(nodes.includes('addEventListener("contextmenu"'), "the node has no context menu");
check(nodes.includes("openNodeMenu(node"), "the context menu does not offer a connection");
check(nodes.includes("connectDrag"), "the node cannot drag a connection");
check(nodes.includes("finishConnect(node.id)"), "a clicked target is not connected");

const menu = extractFunction("openNodeMenu");
check(menu.includes('setAttribute("role", "menu")'), "the node menu has no menu role");
check(menu.includes("data-connect-node"), "the node menu has no Connect item");
check(menu.includes("startConnect(node)"), "the menu item does not start a connection");

const start = extractFunction("startConnect");
check(start.includes("connecting = { from: node.id }"), "the source node is not armed");
check(start.includes("connect_click_or_drag_target"), "the connection mode has no instruction");

const finish = extractFunction("finishConnect");
check(finish.includes("fromNode.id === toNode.id"), "self-links are not refused");
check(finish.includes("await chooseEdgeType(fromNode, toNode)"), "the edge type is not asked after the target");
check(finish.includes("edgeExists(fromNode.id, toNode.id, when)"), "duplicate edges are not rejected");
check(finish.includes("if (!when)"), "a cancelled prompt still adds an edge");
check(finish.includes("graph.edges.push({ from: fromNode.id, to: toNode.id, when })"), "the chosen edge is not added");

const type = extractFunction("chooseEdgeType");
check(type.includes("openDialog({"), "the edge type does not use the shared overlay");
check(type.includes("data-edge-when"), "the overlay has no edge type choices");
check(!type.includes("window.prompt"), "the edge type uses window.prompt");

const inspector = extractFunction("renderInspector");
check(inspector.includes('id="node-connect"'), "the inspector has no Connect action");
check(inspector.includes('id="edge-when"'), "an existing edge cannot change its type");
check(inspector.includes("the_same_edge_is_present_twice"), "changing an edge can create a duplicate");
check(inspector.includes('id="edge-delete"'), "an existing edge cannot be deleted");

check(css.includes(".node-menu-button"), "the node menu button has no style");
check(css.includes(".node-menu-pop"), "the node menu has no style");
check(css.includes(".edges path.connecting"), "the drag preview has no style");
"##;

#[test]
fn nodes_connect_from_a_menu_and_ask_for_the_edge_type() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let js = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    let probe = CONNECT_PROBE
        .replace("__JS__", &serde_json::to_string(&js).expect("js json"))
        .replace("__CSS__", &serde_json::to_string(&css).expect("css json"));
    let path = std::env::temp_dir().join(format!(
        "koreograf-workflow-connect-{}.js",
        std::process::id()
    ));
    std::fs::write(&path, probe).expect("probe");
    let output = Command::new("node").arg(&path).output().expect("node");
    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "probe failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_connect_copy_is_in_both_catalogs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for language in ["de", "en"] {
        let text = std::fs::read_to_string(root.join(format!("web/i18n/{language}.json")))
            .expect("catalog");
        let value: serde_json::Value = serde_json::from_str(&text).expect("catalog json");
        let object = value.as_object().expect("catalog object");
        for key in [
            "choose_edge_type",
            "connect_cancelled",
            "connect_click_or_drag_target",
            "connect_help",
            "connect_nodes",
            "node_actions",
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
fn the_docs_describe_the_connect_flow() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let docs =
        std::fs::read_to_string(root.join("docs/workflows-webhooks.md")).expect("workflow docs");
    assert!(
        docs.contains("Connect"),
        "the workflow docs never mention Connect"
    );
    assert!(
        docs.contains("drag from the source"),
        "the workflow docs do not describe dragging"
    );
    assert!(
        docs.contains("Self-links are refused"),
        "the workflow docs omit the self-link rule"
    );
}
