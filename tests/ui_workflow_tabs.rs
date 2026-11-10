use std::path::Path;
use std::process::Command;

const WORKFLOW_TABS_PROBE: &str = r##"
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
const editor = extractFunction("mountEditor");
check(editor.includes('role="tablist"'), "the workflow page has no tablist");
check(editor.includes('data-workflow-tab="${id}"'), "the tabs are not rendered from the tab list");
check(editor.includes('["editor", "editor"]'), "the Editor tab is missing");
check(editor.includes('["schedules", "schedules"]'), "the Schedules tab is missing");
check(editor.includes('["danger", "danger_zone"]'), "the Danger Zone tab is missing");
check(editor.includes("tabbed = !readonly"), "run view or the editor loses the tab grouping");
check(editor.includes("canDelete ? [[\"danger\""), "viewers get the Danger Zone tab");
check(editor.includes('id="wf-panel-schedules"'), "the schedules panel is missing");
check(editor.includes('id="wf-delete"'), "the delete action is gone");
check(editor.includes("mountSchedules(document.querySelector(\"#schedule-card\"))"), "schedules are no longer mounted");

const bind = extractFunction("bindWorkflowTabs");
for (const key of ["ArrowRight", "ArrowLeft", "Home", "End"]) {
  check(bind.includes(`"${key}"`), `the tablist does not handle ${key}`);
}
check(bind.includes("panel.hidden"), "the tab panels never hide");
check(bind.includes("workflowAddress(next, workflow.id)"), "the tab choice is not in the address");

const locationStub = { hash: "#/workflows/w1?tab=schedules" };
const tab = new Function("location", `${extractFunction("workflowTab")}\nreturn workflowTab;`)(locationStub);
check(tab() === "schedules", "?tab=schedules is not restored");
locationStub.hash = "#/workflows/w1?tab=danger";
check(tab() === "danger", "?tab=danger is not restored");
locationStub.hash = "#/workflows/w1?tab=other";
check(tab() === "editor", "an unknown tab does not fall back to Editor");
const address = new Function(`${extractFunction("workflowAddress")}\nreturn workflowAddress;`)();
check(address("editor", "w1") === "#/workflows/w1", "the Editor tab does not keep the plain address");
check(address("schedules", "w1") === "#/workflows/w1?tab=schedules", "the schedules tab is not linked");
check(address("danger", "w1") === "#/workflows/w1?tab=danger", "the danger tab is not linked");
"##;

#[test]
fn workflow_editor_groups_sections_in_page_tabs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let js = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let probe =
        WORKFLOW_TABS_PROBE.replace("__JS__", &serde_json::to_string(&js).expect("js json"));
    let path =
        std::env::temp_dir().join(format!("koreograf-workflow-tabs-{}.js", std::process::id()));
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
fn the_docs_place_schedules_and_the_danger_zone_in_tabs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workflows =
        std::fs::read_to_string(root.join("docs/workflows-webhooks.md")).expect("workflows docs");
    let jobs = std::fs::read_to_string(root.join("docs/jobs.md")).expect("jobs docs");
    assert!(
        workflows.contains("?tab=schedules"),
        "workflows docs have no tab link"
    );
    assert!(
        workflows.contains("Danger Zone"),
        "workflows docs hide the danger tab"
    );
    assert!(
        jobs.contains("**Schedules** tab"),
        "jobs docs still point below the canvas"
    );
    assert!(
        jobs.contains("?tab=schedules&edit=<id>"),
        "jobs docs do not keep the schedule form on its tab"
    );
}

#[test]
fn the_tab_label_is_in_both_catalogs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for language in ["de", "en"] {
        let text = std::fs::read_to_string(root.join(format!("web/i18n/{language}.json")))
            .expect("catalog");
        let value: serde_json::Value = serde_json::from_str(&text).expect("catalog json");
        let object = value.as_object().expect("catalog object");
        let entry = object
            .get("editor")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(
            !entry.trim().is_empty(),
            "editor is missing from {language}.json"
        );
    }
}
