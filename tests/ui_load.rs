#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;

const NAVIGATION_PROBE: &str = r##"
const js = __JS__;
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
check(!js.includes('addEventListener("popstate"'), "popstate still navigates next to hashchange");
const listeners = js.match(/addEventListener\("hashchange"/g) || [];
check(listeners.length === 1, "the router is not bound to exactly one hashchange listener");
const route = extractFunction("route");
const loading = route.indexOf("showLoading(");
const firstAwait = route.indexOf("await ");
check(loading >= 0, "route does not show a loading state");
check(firstAwait > loading, "route waits for data before it shows the loading state");
const show = extractFunction("showLoading");
check(show.includes('aria-busy'), "the loading state is not announced");
check(show.includes("main.innerHTML"), "the loading state does not replace the previous page");
check(!show.includes("await"), "the loading state waits before it replaces the previous page");
"##;

const SKELETON_PROBE: &str = r##"
const css = __CSS__;
function check(ok, message) {
  if (!ok) { console.error(message); process.exit(1); }
}
check(/\.loading-skeleton\s*\{/.test(css), "the loading skeleton has no layout");
check(/\.skeleton-card\s*\{/.test(css), "the skeleton card has no rule");
check(/\.skeleton-line\s*\{[^}]*background:/.test(css), "the skeleton lines have no surface");
"##;

#[test]
fn one_navigation_loads_one_page() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let js = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let probe = NAVIGATION_PROBE.replace("__JS__", &serde_json::to_string(&js).expect("js json"));
    run_node_probe("single-load", &probe);
}

#[test]
fn the_loading_state_replaces_the_previous_page() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    let probe = SKELETON_PROBE.replace("__CSS__", &serde_json::to_string(&css).expect("css json"));
    run_node_probe("loading-skeleton", &probe);
}

fn run_node_probe(name: &str, probe: &str) {
    let path = std::env::temp_dir().join(format!("koreograf-{}-{}.js", name, std::process::id()));
    std::fs::write(&path, probe).expect("probe");
    let output = Command::new("node").arg(&path).output().expect("node");
    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "probe {name} failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
