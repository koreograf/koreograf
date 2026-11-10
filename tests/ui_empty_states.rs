use std::path::Path;
use std::process::Command;

const EMPTY_STATE_PROBE: &str = r##"
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
check(js.includes("function emptyState("), "the shared empty state helper is missing");
for (const [name, key] of [
  ["pageTemplates", "empty_templates"],
  ["pageProjects", "empty_projects"],
  ["pageInventories", "empty_inventories"],
  ["pageWorkflows", "empty_workflows"],
  ["pageCredentials", "empty_credentials"],
  ["pageJobs", "empty_jobs"],
]) {
  const body = extractFunction(name);
  check(body.includes(`emptyState("${key}"`), `${name} has no guided empty state`);
  check(body.includes("can("), `${name} does not gate the create action by role`);
}
check(js.includes("function onboardingCard("), "the overview has no onboarding card");
const onboarding = extractFunction("onboardingCard");
for (const key of ["new_project", "new_inventory", "new_template", "launch_a_template"]) {
  check(onboarding.includes(`"${key}"`), `the onboarding card misses ${key}`);
}
check(onboarding.includes('can("operator")'), "the onboarding card shows write actions to viewers");
const dashboard = extractFunction("pageDashboard");
check(dashboard.includes("onboardingCard()"), "the overview never shows the onboarding card");
check(/jobs\.length\s*\?\s*""\s*:\s*onboardingCard\(\)/.test(dashboard), "the onboarding card is not reserved for an empty team");

const job = extractFunction("pageJob");
check(job.includes('id="log" tabindex="0"'), "the job log is not keyboard scrollable");
check(job.includes('class="job-sticky"'), "the job actions are not sticky");
check(job.includes('id="output-tools"'), "the log tools are not grouped with the sticky header");
check(job.includes("output-tools") && job.includes("hidden = tab !== \"output\""), "the parameters tab still shows the log tools");

const log = /\.log\s*\{([^}]*)\}/.exec(css);
check(log, "the log rule is missing");
check(/max-height:\s*70vh/.test(log[1]), "the log does not use a viewport height cap");
check(/overflow:\s*auto/.test(log[1]), "the log does not scroll on its own");
const sticky = /\.job-sticky\s*\{([^}]*)\}/.exec(css);
check(sticky, "the sticky job header rule is missing");
check(/position:\s*sticky/.test(sticky[1]), "the job header is not sticky");
"##;

#[test]
fn empty_lists_and_long_logs_are_guided_and_contained() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let js = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    let probe = EMPTY_STATE_PROBE
        .replace("__JS__", &serde_json::to_string(&js).expect("js json"))
        .replace("__CSS__", &serde_json::to_string(&css).expect("css json"));
    let path =
        std::env::temp_dir().join(format!("koreograf-empty-states-{}.js", std::process::id()));
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
fn the_new_copy_is_in_both_catalogs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for language in ["de", "en"] {
        let text = std::fs::read_to_string(root.join(format!("web/i18n/{language}.json")))
            .expect("catalog");
        let value: serde_json::Value = serde_json::from_str(&text).expect("catalog json");
        let object = value.as_object().expect("catalog object");
        for key in [
            "empty_credentials",
            "empty_inventories",
            "empty_jobs",
            "empty_projects",
            "empty_templates",
            "empty_workflows",
            "get_started",
            "get_started_help",
            "launch_a_template",
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
