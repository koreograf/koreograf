#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/web_source.rs"]
mod web_source;

use std::path::Path;
use std::process::Command;

const POLISH_PROBE: &str = r##"
const js = __JS__;
const css = __CSS__;
function check(ok, message) {
  if (!ok) { console.error(message); process.exit(1); }
}
function extractFunction(name) {
  const start = js.indexOf(`function ${name}(`);
  check(start >= 0, `${name} is missing`);
  const lineEnd = js.indexOf("\n", start);
  const open = js.lastIndexOf("{", lineEnd);
  check(open > start, `${name} has no body`);
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
const plain = new Function(`${extractFunction("plainLogLine")}\nreturn plainLogLine;`)();
check(plain("\u001b[31mred\u001b[0m") === "red", "log lines keep sgr colour codes");
check(plain("\u001b[1;32mok\u001b[0m run") === "ok run", "log lines keep compound sgr codes");
check(plain("\u001b[2Kline") === "line", "log lines keep cursor codes");
const append = extractFunction("appendLog");
check(append.includes("line.textContent = plainLogLine(row.line)"), "the job log renders raw escape codes");
check(append.includes("line.koreografRaw = line.textContent"), "the log search keeps escape codes");

const fill = extractFunction("fillVersion");
check(fill.includes("appState.buildCommit.slice(0, 7)"), "the sidebar shows the full commit");
check(fill.includes("link.title = appState.buildCommit"), "the full commit is not in the title");
const versionRule = /\.version-line a\s*\{([^}]*)\}/.exec(css);
check(versionRule, "the version line has no rule");
check(/overflow:\s*hidden/.test(versionRule[1]), "the version line may overflow the rail");
check(/text-overflow:\s*ellipsis/.test(versionRule[1]), "the version line has no ellipsis");

const pill = /\.pill\s*\{([^}]*)\}/.exec(css);
check(pill, "the pill rule is missing");
check(/white-space:\s*nowrap/.test(pill[1]), "pills may wrap inside a word");
const tablePill = /\.table \.pill\s*\{([^}]*)\}/.exec(css);
check(tablePill, "table pills have no minimum width");
check(/min-width:/.test(tablePill[1]), "table pills keep shrinking");
const tool = /\.tool\s*\{([^}]*)\}/.exec(css);
check(tool && /white-space:\s*nowrap/.test(tool[1]), "tool names may wrap inside a word");

const route = extractFunction("route");
check(route.includes("routeScroll.set"), "the router does not remember the scroll position");
check(route.includes("settleRoute(back)"), "the router does not settle the scroll position");
check(route.includes("routeDirection(historyState)"), "the router does not read the history direction");
check(!route.includes("popstate"), "the router still depends on popstate for direction");
const settle = extractFunction("settleRoute");
check(settle.includes("window.scrollTo(0, 0)"), "a new page does not open at the top");
check(settle.includes("heading.focus"), "a new page does not focus the heading");
check(settle.includes("routeScroll.get"), "back does not restore the scroll position");
check(js.includes('<h1 tabindex="-1">'), "the page heading cannot take focus");
const direction = new Function(
  `const appState = { historyIndex: 0, setHistoryIndex(value) { this.historyIndex = value; } };\n${extractFunction("hasHistoryIndex")}\n${extractFunction("routeDirection")}\nreturn { routeDirection, index: () => appState.historyIndex };`,
)();
check(direction.routeDirection(null) === false, "a new entry is treated as back");
check(direction.index() === 1, "a new entry gets no history index");
check(direction.routeDirection({ idx: 2 }) === false, "a forward entry is treated as back");
check(direction.routeDirection({ idx: 1 }) === true, "back was not detected");
check(direction.routeDirection({ idx: 2 }) === false, "forward after back was treated as back");
const limit = new Function(`${extractFunction("latestStartDate")}\nreturn latestStartDate;`)();
check(limit("2026-11-10") === "2036-11-10", "the start bound is not ten years ahead");
check(limit("2028-02-29") === "2038-02-28", "the start bound keeps an impossible February day");
const form = extractFunction("openScheduleForm");
check(form.includes("latestStartDate(local.date)"), "the schedule start has no upper bound");
check(/type="date" required max=/.test(form), "the start field does not carry the bound");
"##;

const DYNAMIC_KEYS: &[&str] = &[
    "manual", "schedule", "hook", "workflow", "relaunch", "ansible", "opentofu", "shell", "once",
    "hourly", "daily", "weekly", "monthly",
];

#[test]
fn audit_details_are_polished() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let js = web_source::source();
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    let probe = POLISH_PROBE
        .replace("__JS__", &serde_json::to_string(&js).expect("js json"))
        .replace("__CSS__", &serde_json::to_string(&css).expect("css json"));
    run_node_probe("ui-polish", &probe);
}

#[test]
fn dynamic_labels_are_in_both_catalogs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for language in ["de", "en"] {
        let path = root.join(format!("web/i18n/{language}.json"));
        let text = std::fs::read_to_string(&path).expect("catalog");
        let value: serde_json::Value = serde_json::from_str(&text).expect("catalog json");
        let object = value.as_object().expect("catalog object");
        for key in DYNAMIC_KEYS {
            let entry = object
                .get(*key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            assert!(
                !entry.trim().is_empty(),
                "{key} is missing from {language}.json"
            );
        }
    }
    let de = std::fs::read_to_string(root.join("web/i18n/de.json")).expect("de");
    let en = std::fs::read_to_string(root.join("web/i18n/en.json")).expect("en");
    assert!(de.contains("\"schedule\": \"Zeitplan\""));
    assert!(en.contains("\"schedule\": \"Schedule\""));
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
