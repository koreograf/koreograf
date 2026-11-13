#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/web_source.rs"]
mod web_source;

use std::process::Command;

const PROBE: &str = r##"
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
const source = extractFunction("assistantNavItem");
const make = new Function("t", `${source}\nreturn assistantNavItem;`);
const nav = make((key) => key === "assistant" ? "Assistant" : key);
check(JSON.stringify(nav(false)) === "[]", "the assistant entry shows while disabled");
const items = nav(true);
check(Array.isArray(items) && items.length === 1, "the assistant entry is missing while enabled");
check(items[0][0] === "#/assistant", "the assistant entry does not point at the chat page");
check(items[0][1] === "Assistant", "the assistant entry has no label");
check(js.includes("...assistantNavItem(appState.assistantEnabled)"), "the navigation does not use the enabled flag");
check(js.includes('api("GET", "/api/v1/assistant")'), "the UI does not read the assistant status");
const page = extractFunction("pageAssistant");
check(page.includes("if (!appState.assistantEnabled)"), "the chat page does not check the enabled flag");
check(page.includes('api("POST", "/api/v1/assistant/chat"'), "the chat page does not call the assistant route");
const turn = extractFunction("assistantTurnHtml");
check(turn.includes("esc(turn.content)"), "the chat page does not escape answers");
check(js.includes("data-assistant-confirm"), "a write has no confirmation card");
check(js.includes("data-assistant-cancel"), "a confirmation card cannot be canceled");
check(js.includes('/api/v1/assistant/confirm'), "the confirmation card does not call the confirm route");
check(js.includes('href="#/assistant?explain='), "a failed job has no explain entry");
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
fn the_assistant_entry_only_shows_when_the_server_enables_it() {
    let js = web_source::source();
    let probe = PROBE.replace("__JS__", &serde_json::to_string(&js).expect("js json"));
    run_node_probe("assistant-ui", &probe);
}
