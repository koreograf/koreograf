#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/web_source.rs"]
mod web_source;

use std::process::Command;

const SOURCE_GUARD_PROBE: &str = r##"
const js = __JS__;
function check(ok, message) {
  if (!ok) { console.error(message); process.exit(1); }
}
function functionBody(name) {
  const found = js.indexOf(`function ${name}(`);
  check(found >= 0, `${name} is missing`);
  let open = -1;
  let parens = 0;
  for (let index = found; index < js.length; index += 1) {
    if (js[index] === "(") parens += 1;
    else if (js[index] === ")") parens -= 1;
    else if (js[index] === "{" && parens === 0) {
      open = index;
      break;
    }
  }
  check(open >= 0, `${name} has no body`);
  let depth = 0;
  for (let index = open; index < js.length; index += 1) {
    if (js[index] === "{") depth += 1;
    else if (js[index] === "}") {
      depth -= 1;
      if (depth === 0) return js.slice(found, index + 1);
    }
  }
  check(false, `${name} has no closing brace`);
}
function listener(mount, type) {
  const marker = `card.addEventListener("${type}"`;
  const from = mount.indexOf(marker);
  check(from >= 0, `the ${type} listener is missing`);
  const line = mount.slice(from, mount.indexOf("\n", from)).trim();
  const listeners = {};
  const card = { dataset: {}, addEventListener: (name, handler) => { listeners[name] = handler; } };
  new Function("card", line)(card);
  check(typeof listeners[type] === "function", `the ${type} listener is not bound`);
  listeners[type]();
  check(card.dataset.dirty === "1", `${type} does not mark the source card dirty`);
}
const mount = functionBody("mountSourceDraft");
const netbox = mount.indexOf('if (kind === "netbox")');
check(netbox >= 0, "the netbox branch is missing");
for (const type of ["input", "change"]) {
  const marker = `card.addEventListener("${type}"`;
  const from = mount.indexOf(marker);
  check(from >= 0, `the ${type} listener is missing`);
  check(from < netbox, `the ${type} listener only runs for netbox cards`);
  check(mount.split(marker).length === 2, `the ${type} listener is bound more than once`);
  listener(mount, type);
}
check(
  mount.includes('card.querySelector("[data-remove-draft]").addEventListener("click", () => card.remove());'),
  "cancelling a source card does not remove it",
);
const guardStart = js.indexOf('appState.setLeaveGuard(() => document.querySelector("#source-edit [data-dirty]")');
check(guardStart >= 0, "the source edit leave guard is missing");
const guardEnd = js.indexOf(");", guardStart);
const guardExpr = js.slice(guardStart + "appState.setLeaveGuard(".length, guardEnd);
function guard(dirty, draft) {
  const document = {
    querySelector: (selector) => selector.includes("[data-dirty]") ? (dirty ? {} : null) : (draft ? {} : null),
  };
  return new Function("document", `return (${guardExpr})();`)(document);
}
check(guard(false, false) === false, "the leave guard fires without a source edit");
check(guard(true, false) === true, "a dirty source card does not trigger the leave guard");
check(guard(false, true) === true, "a source draft does not trigger the leave guard");
const route = js.indexOf("async function route()");
check(route >= 0, "route is missing");
const confirm = js.indexOf("if (appState.leaveGuard && appState.shownHash !== null", route);
const reset = js.indexOf("appState.setLeaveGuard(null);", route);
check(confirm >= 0 && reset > confirm, "route does not clear the leave guard after the confirm");
"##;

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

#[test]
fn source_cards_share_the_dirty_guard() {
    let js = web_source::source();
    let probe = SOURCE_GUARD_PROBE.replace("__JS__", &serde_json::to_string(&js).expect("js json"));
    run_node_probe("source-dirty-guard", &probe);
}
