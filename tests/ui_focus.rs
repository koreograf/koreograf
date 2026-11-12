#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;

const FOCUS_PROBE: &str = r##"
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
function element(props = {}) {
  return {
    focused: false,
    id: "",
    classList: { contains: () => false },
    getAttribute: () => null,
    focus() { this.focused = true; },
    ...props,
  };
}
const trigger = element({ getAttribute: (name) => (name === "href" ? "#/templates?edit=t1" : null) });
const replacement = element({ getAttribute: (name) => (name === "href" ? "#/templates?edit=t1" : null) });
const body = element();
const main = element({ id: "main" });
const document = {
  activeElement: body,
  body,
  documentElement: element(),
  contains: (node) => node === trigger,
  getElementById: (id) => (id === "main" ? main : null),
  querySelectorAll: () => [replacement],
};
const focusOpener = new Function("document", "focusTrigger", `${extractFunction("focusOpener")}\nreturn focusOpener;`)(document, trigger);
const focusReturn = new Function("document", `${extractFunction("focusReturn")}\nreturn focusReturn;`)(document);
check(focusOpener() === trigger, "focusOpener ignores the last trigger when BODY has focus");
const modal = extractFunction("makeModal");
check(modal.includes("const opener = focusOpener()"), "makeModal does not remember the opener");
check(modal.includes("focusReturn(opener)"), "makeModal does not restore the opener");
trigger.focused = false;
focusReturn(trigger);
check(trigger.focused, "focusReturn does not focus a connected opener");
document.contains = () => false;
focusReturn(trigger);
check(replacement.focused, "focusReturn does not focus the matching control after a rerender");
document.activeElement = main;
check(focusOpener() === main, "focusOpener ignores a focused control");
document.activeElement = element({ classList: { contains: (name) => name === "skip-link" } });
check(focusOpener() === trigger, "focusOpener keeps the skip link as the opener");
"##;

const LOGIN_PROBE: &str = r##"
const js = __JS__;
const css = __CSS__;
function check(ok, message) {
  if (!ok) { console.error(message); process.exit(1); }
}
const start = js.indexOf("function renderLogin(");
check(start >= 0, "renderLogin is missing");
const end = js.indexOf("\nfunction ", start + 1);
const body = js.slice(start, end);
const order = ['name="username"', 'name="password"', 'type="submit"', "langSwitch()"];
let previous = -1;
for (const token of order) {
  const index = body.indexOf(token);
  check(index > previous, `login DOM order is wrong at ${token}`);
  previous = index;
}
const rule = /\.login-card\s*>\s*\.lang-switch\s*\{([^}]*)\}/.exec(css);
check(rule, "the login language switch has no placement rule");
check(/position:\s*absolute/.test(rule[1]), "the login language switch is not positioned");
check(/top:\s*28px/.test(rule[1]) && /right:\s*28px/.test(rule[1]), "the login language switch is not top right");
check(/\.login-card\s*\{[^}]*position:\s*relative/.test(css), "the login card is not the placement container");
"##;

const TARGET_PROBE: &str = r##"
const css = __CSS__;
function check(ok, message) {
  if (!ok) { console.error(message); process.exit(1); }
}
function block(selector) {
  return new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*\\{([^}]*)\\}`).exec(css);
}
const button = block(".btn");
check(button, "the button rule is missing");
check(/min-width:\s*24px/.test(button[1]) && /min-height:\s*24px/.test(button[1]), "buttons may shrink below 24px");
const compact = block(".btn.compact");
check(compact, "the compact button rule is missing");
check(/min-width:\s*24px/.test(compact[1]) && /min-height:\s*24px/.test(compact[1]), "compact buttons may shrink below 24px");
const link = block(".table a");
check(link, "the table link rule is missing");
check(/min-width:\s*24px/.test(link[1]) && /min-height:\s*24px/.test(link[1]), "table links may shrink below 24px");
const coarse = /@media \(pointer: coarse\)\s*\{([\s\S]*?)\n\}/.exec(css);
check(coarse, "the coarse pointer rule is missing");
check(/min-height:\s*44px/.test(coarse[1]) && /min-width:\s*44px/.test(coarse[1]), "coarse pointer targets stay below 44px");
for (const selector of [".btn", ".table a", ".lang-button", ".dialog-close"]) {
  check(coarse[1].includes(selector), `coarse pointer targets miss ${selector}`);
}
"##;

#[test]
fn overlays_restore_focus_to_the_trigger() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let js = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let probe = FOCUS_PROBE.replace("__JS__", &serde_json::to_string(&js).expect("js json"));
    run_node_probe("overlay-focus", &probe);
}

#[test]
fn login_tab_order_follows_the_form() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let js = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    let probe = LOGIN_PROBE
        .replace("__JS__", &serde_json::to_string(&js).expect("js json"))
        .replace("__CSS__", &serde_json::to_string(&css).expect("css json"));
    run_node_probe("login-tab-order", &probe);
}

#[test]
fn action_targets_meet_the_minimum_size() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    let probe = TARGET_PROBE.replace("__CSS__", &serde_json::to_string(&css).expect("css json"));
    run_node_probe("action-targets", &probe);
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
