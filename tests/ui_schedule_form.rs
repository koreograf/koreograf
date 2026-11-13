#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/web_source.rs"]
mod web_source;

use std::path::Path;
use std::process::Command;

const SCHEDULE_PROBE: &str = r##"
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
const mount = extractFunction("mountSchedules");
check(/const add = can\("operator"\) \?/.test(mount), "the schedule add button is not operator only");
check(mount.includes('id="schedule-add"'), "the schedules page has no create button");
check(mount.includes("data-schedule-new"), "the empty state has no create control");
check(mount.includes("schedule_empty_hint"), "the empty state does not explain a schedule");
check(/all && !items\.length/.test(mount), "the schedules page has no empty state");
const picker = extractFunction("openNewScheduleDialog");
check(picker.includes("/api/v1/templates"), "the picker does not list templates");
check(picker.includes("/api/v1/workflows"), "the picker does not list workflows");
check(picker.includes("schedule_needs_target"), "the picker does not explain a missing target");
const form = extractFunction("openScheduleForm");
check(form.includes('name="target_ref"'), "the schedule form cannot pick a template or workflow");
check(form.includes("options.targets"), "the schedule form has no target choices");
check(form.includes("form.target_ref.value.split"), "the schedule form ignores the chosen target");
check(form.includes("options.hash !== false"), "the schedule form cannot skip the schedule address");
const template = extractFunction("openTemplateDialog");
check(template.includes('id: "schedules"'), "the template overlay has no schedules tab");
check(template.includes("template-schedule-add"), "the template overlay cannot create a schedule");
check(/can\("operator"\) \?/.test(template), "the template schedule control is not operator only");
check(template.includes("{ hash: false }"), "the template schedules tab rewrites the schedule address");
check(/\.empty-state\s*\{/.test(css), "the empty state has no layout");
"##;

const KEYS: &[&str] = &[
    "choose_a_target",
    "schedule_empty_hint",
    "schedule_needs_target",
    "schedule_target_hint",
    "template_or_workflow",
    "template_schedules_hint",
];

#[test]
fn schedules_can_be_created_from_the_schedules_page_and_a_template() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let js = web_source::source();
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    let probe = SCHEDULE_PROBE
        .replace("__JS__", &serde_json::to_string(&js).expect("js json"))
        .replace("__CSS__", &serde_json::to_string(&css).expect("css json"));
    run_node_probe("schedule-form", &probe);
}

#[test]
fn the_new_schedule_strings_are_in_both_catalogs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for language in ["de", "en"] {
        let path = root.join(format!("web/i18n/{language}.json"));
        let text = std::fs::read_to_string(&path).expect("catalog");
        let value: serde_json::Value = serde_json::from_str(&text).expect("catalog json");
        let object = value.as_object().expect("catalog object");
        for key in KEYS {
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
