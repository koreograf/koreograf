#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/web_source.rs"]
mod web_source;

use std::path::Path;
use std::process::Command;

const VIEW_PROBE: &str = r##"
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
const messages = {
  route_error_title: "This page could not be loaded.",
  route_error_explanation: "Try again, or open the overview.",
  retry: "Try again",
  back_to_overview: "Back to overview",
  not_found: "Not found.",
};
function t(key) { return messages[key] || key; }
function esc(value) {
  return String(value ?? "").replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  }[ch]));
}
function h(key) { return esc(t(key)); }
let shellCalls = [];
function shell(title, body) { shellCalls.push({ title, body }); }
let retryListener = null;
const document = {
  querySelector: (selector) => selector === "#route-retry"
    ? { addEventListener: (name, listener) => { if (name === "click") retryListener = listener; } }
    : null,
};
let loginMessage = null;
function renderLogin(message) { loginMessage = message; }
let routed = 0;
function route() { routed += 1; }
const renderRouteError = new Function(
  "t", "esc", "h", "shell", "document", "renderLogin", "route", "me",
  `${extractFunction("renderRouteError")}\nreturn renderRouteError;`,
)(t, esc, h, shell, document, renderLogin, route, {});
renderRouteError(new Error("not_found"));
check(shellCalls.length === 1, "the failed route did not render a shell");
const view = shellCalls[0];
check(view.title === "This page could not be loaded.", "the error page heading is missing");
check(view.body.includes("Not found."), "the API error message is missing");
check(view.body.includes('id="route-retry"'), "the error page has no retry action");
check(view.body.includes('href="#/"'), "the error page has no overview link");
check(view.body.includes("Back to overview"), "the overview link is not labelled");
check(typeof retryListener === "function", "the retry action is not bound");
retryListener();
check(routed === 1, "the retry action does not rerun the route");
const errorView = extractFunction("renderRouteError");
check(!errorView.includes("topmostCard"), "the error page still targets the previous card");
check(!errorView.includes("insertAdjacentHTML"), "the error page still injects into previous content");
shellCalls = [];
messages["<b>unsafe</b>"] = "<b>unsafe</b>";
renderRouteError(new Error("<b>unsafe</b>"));
check(shellCalls.length === 1, "the unsafe error did not render");
check(!shellCalls[0].body.includes("<b>unsafe</b>"), "the error page leaks raw HTML");
check(shellCalls[0].body.includes("&lt;b&gt;unsafe&lt;/b&gt;"), "the error text is not escaped");
const routeStart = js.indexOf("async function route()");
check(routeStart >= 0, "route is missing");
let routeBody = "";
let depth = 0;
for (let index = js.indexOf("{", routeStart); index < js.length; index += 1) {
  routeBody += js[index];
  if (js[index] === "{") depth += 1;
  else if (js[index] === "}") {
    depth -= 1;
    if (depth === 0) break;
  }
}
check(routeBody.includes("renderRouteError(error)"), "route failures do not render the error page");
check(!routeBody.includes("handle(error)"), "route failures still use the banner handler");
"##;

const BANNER_PROBE: &str = r##"
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
const handle = extractFunction("handle");
check(handle.includes("topmostCard()"), "the dialog handler no longer targets the open overlay");
check(handle.includes("banner bad"), "the dialog handler no longer shows a banner");
"##;

const CATALOG_PROBE: &str = r##"
const en = __EN__;
const de = __DE__;
function check(ok, message) {
  if (!ok) { console.error(message); process.exit(1); }
}
const keys = ["back_to_overview", "retry", "route_error_explanation", "route_error_title"];
for (const key of keys) {
  check(typeof en[key] === "string" && en[key].length > 0, `en.json misses ${key}`);
  check(typeof de[key] === "string" && de[key].length > 0, `de.json misses ${key}`);
}
const enKeys = Object.keys(en);
const deKeys = Object.keys(de);
check(JSON.stringify(enKeys) === JSON.stringify(deKeys), "the catalogs list different keys");
for (const list of [enKeys, deKeys]) {
  for (let index = 1; index < list.length; index += 1) {
    check(list[index - 1] < list[index], `catalog order breaks at ${list[index]}`);
  }
}
"##;

#[test]
fn route_failure_renders_an_error_page() {
    let js = web_source::source();
    let probe = VIEW_PROBE.replace("__JS__", &serde_json::to_string(&js).expect("js json"));
    run_node_probe("route-error-view", &probe);
}

#[test]
fn overlay_field_errors_keep_the_banner() {
    let js = web_source::source();
    let probe = BANNER_PROBE.replace("__JS__", &serde_json::to_string(&js).expect("js json"));
    run_node_probe("overlay-error-banner", &probe);
}

#[test]
fn route_error_copy_is_in_both_catalogs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let en = std::fs::read_to_string(root.join("web/i18n/en.json")).expect("en");
    let de = std::fs::read_to_string(root.join("web/i18n/de.json")).expect("de");
    let probe = CATALOG_PROBE.replace("__EN__", &en).replace("__DE__", &de);
    run_node_probe("route-error-catalog", &probe);
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
