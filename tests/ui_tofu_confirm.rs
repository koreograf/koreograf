use std::path::Path;
use std::process::Command;

const HELPERS_PROBE: &str = r##"
const js = __JS__;
function check(ok, message) {
  if (!ok) { console.error(message); process.exit(1); }
}
function extractFunction(name) {
  const found = js.indexOf(`function ${name}(`);
  check(found >= 0, `${name} is missing`);
  const start = js.slice(Math.max(0, found - 6), found) === "async " ? found - 6 : found;
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
      if (depth === 0) return js.slice(start, index + 1);
    }
  }
  check(false, `${name} has no closing brace`);
}
const tofuApply = new Function(`${extractFunction("tofuApply")}\nreturn tofuApply;`)();
check(tofuApply({ tool: "opentofu", spec: { action: "apply" } }), "an OpenTofu apply is not guarded");
check(!tofuApply({ tool: "opentofu", spec: { action: "plan" } }), "an OpenTofu plan asks for confirmation");
check(!tofuApply({ tool: "ansible", spec: { action: "apply" } }), "an Ansible template asks for OpenTofu confirmation");
check(!tofuApply(null), "a missing template asks for confirmation");

const workflowTofuApply = new Function(`${extractFunction("workflowTofuApply")}\nreturn workflowTofuApply;`)();
const templates = [
  { id: "apply", tool: "opentofu", spec: { action: "apply" } },
  { id: "plan", tool: "opentofu", spec: { action: "plan" } },
  { id: "ansible", tool: "ansible", spec: {} },
];
check(workflowTofuApply({ nodes: [{ template_id: "apply" }] }, templates), "a workflow with an apply node is not guarded");
check(!workflowTofuApply({ nodes: [{ template_id: "plan" }] }, templates), "a workflow with only plan nodes asks for confirmation");
check(!workflowTofuApply({ nodes: [{ template_id: "ansible" }] }, templates), "a workflow with Ansible nodes asks for OpenTofu confirmation");
check(workflowTofuApply({ nodes: [{ template_id: "plan", overrides: { spec: { action: "apply" } } }] }, templates), "an apply node override is not guarded");
check(!workflowTofuApply({ nodes: [{ template_id: "apply", overrides: { spec: { action: "plan" } } }] }, templates), "a plan node override still asks for confirmation");

const pageTemplates = extractFunction("pageTemplates");
const templateLaunch = pageTemplates.slice(pageTemplates.indexOf("[data-launch]"), pageTemplates.indexOf("[data-launch-options]"));
check(templateLaunch.includes("confirmTofuApply"), "the one-click template launch has no OpenTofu confirmation");
check(templateLaunch.indexOf("confirmTofuApply") < templateLaunch.indexOf('await api("POST"'), "the template launch posts before confirming");

const pageWorkflows = extractFunction("pageWorkflows");
check(pageWorkflows.includes("workflowTofuApply"), "the workflow list launch has no OpenTofu confirmation");
check(pageWorkflows.indexOf("workflowTofuApply") < pageWorkflows.indexOf('await api("POST"'), "the workflow list launch posts before confirming");

const editor = extractFunction("mountEditor");
const editorLaunch = editor.slice(editor.indexOf('const launch = document.querySelector("#wf-launch")'), editor.indexOf("const remove", editor.indexOf('const launch = document.querySelector("#wf-launch")')));
check(editorLaunch.includes("workflowTofuApply"), "the workflow editor launch has no OpenTofu confirmation");
check(editorLaunch.indexOf("workflowTofuApply") < editorLaunch.indexOf('await api("POST"'), "the workflow editor launch posts before confirming");
"##;

const CONFIRM_PROBE: &str = r##"
const js = __JS__;
function check(ok, message) {
  if (!ok) { console.error(message); process.exit(1); }
}
function extractFunction(name) {
  const found = js.indexOf(`function ${name}(`);
  check(found >= 0, `${name} is missing`);
  const start = js.slice(Math.max(0, found - 6), found) === "async " ? found - 6 : found;
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
      if (depth === 0) return js.slice(start, index + 1);
    }
  }
  check(false, `${name} has no closing brace`);
}
let dialog = null;
const openDialog = (spec) => {
  dialog = spec;
  return { form: {} };
};
const messages = {
  opentofu_apply_confirmation_body: 'Start OpenTofu apply for "{name}"?',
};
const t = (key) => messages[key] || key;
const h = (key, vars) => {
  let out = t(key);
  Object.entries(vars || {}).forEach(([name, value]) => {
    out = out.replaceAll(`{${name}}`, String(value));
  });
  return out;
};
const confirmTofuApply = new Function(
  "openDialog", "t", "h",
  `${extractFunction("confirmTofuApply")}\nreturn confirmTofuApply;`,
)(openDialog, t, h);

const cancelled = confirmTofuApply({ name: "apply-prod" });
check(dialog.id === "tofu-apply-confirm", "the confirmation has no stable id");
check(dialog.title === "opentofu_apply_confirmation_title", "the confirmation has no title key");
check(dialog.submit === "apply", "the confirmation does not use the apply label");
check(dialog.body.includes("apply-prod"), "the confirmation does not name the template or workflow");
dialog.onClose();
check((await cancelled) === false, "cancel did not resolve false");

const confirmed = confirmTofuApply({ name: "apply-prod" });
let closed = 0;
dialog.onSubmit({ close: () => { closed += 1; } });
check((await confirmed) === true, "confirm did not resolve true");
check(closed === 1, "confirm did not close the dialog");
"##;

const LAUNCH_PROBE: &str = r##"
const js = __JS__;
function check(ok, message) {
  if (!ok) { console.error(message); process.exit(1); }
}
function extractFunction(name) {
  const found = js.indexOf(`function ${name}(`);
  check(found >= 0, `${name} is missing`);
  const start = js.slice(Math.max(0, found - 6), found) === "async " ? found - 6 : found;
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
      if (depth === 0) return js.slice(start, index + 1);
    }
  }
  check(false, `${name} has no closing brace`);
}
const button = { disabled: false, attrs: {}, setAttribute(name, value) { this.attrs[name] = value; }, removeAttribute(name) { delete this.attrs[name]; } };
const form = {
  extra_vars: { value: "{}" },
  querySelector: (selector) => (selector === 'button[type="submit"]' ? button : null),
  querySelectorAll: () => [],
};
const item = { id: "tofu-1", name: "apply-prod", tool: "opentofu", extra_vars: {}, spec: { action: "apply" } };
let dialogs = [];
let posts = 0;
let target = "";
let closed = 0;
const api = (method) => {
  if (method === "GET") return Promise.resolve([]);
  posts += 1;
  return Promise.resolve({ id: `job-${posts}` });
};
const openDialog = (spec) => {
  dialogs.push(spec);
  return { form };
};
const openLaunchPrompt = new Function(
  "t", "h", "esc", "api", "openDialog", "selectOptions", "chipBox", "templateCredentialFields",
  "selectedCredentialIds", "flushChips", "jsonObject", "bindChips", "bindVaultPicker", "go", "document", "verbosityLabel",
  `${extractFunction("openLaunchPrompt")}\nreturn openLaunchPrompt;`,
)(
  (key) => key, (key) => key, (value) => String(value ?? ""), api, openDialog, () => "", () => "", () => "",
  () => [], () => true, () => ({}), () => {}, () => {}, (value) => { target = value; },
  { querySelector: () => null }, () => "verbosity",
);
await openLaunchPrompt(item, true);
check(dialogs.length === 1, "the launch dialog did not open");
const cancelRun = dialogs[0].onSubmit({ form, close: () => { closed += 1; } });
check(dialogs.length === 2, "the OpenTofu launch did not ask for confirmation");
dialogs[1].onClose();
await cancelRun;
check(posts === 0, "cancel still launched a job");
check(closed === 0, "cancel closed the launch form");
check(form.extra_vars.value === "{}", "cancel changed the launch form");
check(!button.disabled, "cancel left the launch button disabled");

const run = dialogs[0].onSubmit({ form, close: () => { closed += 1; } });
check(dialogs.length === 3, "the retried OpenTofu launch did not ask for confirmation");
dialogs[2].onSubmit({ close: () => {} });
await run;
check(posts === 1, "confirm did not launch exactly one job");
check(closed === 1, "confirm did not close the launch form");
check(target === "#/jobs/job-1", "confirm did not open the new job");
"##;

#[test]
fn only_opentofu_apply_and_apply_nodes_are_guarded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let js = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let probe = HELPERS_PROBE.replace("__JS__", &serde_json::to_string(&js).expect("js json"));
    run_node_probe("tofu-confirm-helpers", &probe);
}

#[test]
fn confirm_dialog_names_the_target_and_returns_the_answer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let js = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let probe = CONFIRM_PROBE.replace("__JS__", &serde_json::to_string(&js).expect("js json"));
    run_node_probe("tofu-confirm-dialog", &probe);
}

#[test]
fn launch_with_options_waits_for_the_confirmation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let js = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let probe = LAUNCH_PROBE.replace("__JS__", &serde_json::to_string(&js).expect("js json"));
    run_node_probe("tofu-confirm-launch", &probe);
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
