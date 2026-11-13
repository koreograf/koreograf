#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/web_source.rs"]
mod web_source;

use std::process::Command;

const DOUBLE_SUBMIT_PROBE: &str = r##"
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
function makeButton() {
  return {
    disabled: false,
    attrs: {},
    setAttribute(name, value) { this.attrs[name] = value; },
    removeAttribute(name) { delete this.attrs[name]; },
  };
}
function makeForm(button) {
  return {
    button,
    extra_vars: { value: "{}" },
    inventory_id: { value: "inv-1" },
    verbosity: { value: "0" },
    job_type: { value: "run" },
    tags: { value: "" },
    skip_tags: { value: "" },
    limit: { value: "" },
    querySelector: (selector) => (selector === 'button[type="submit"]' ? button : null),
    querySelectorAll: () => [],
  };
}
const item = {
  id: "t1",
  tool: "ansible",
  inventory_id: "inv-1",
  extra_vars: {},
  spec: { verbosity: 0, job_type: "run", tags: "", skip_tags: "", limit: "" },
};
let dialog = null;
let form = null;
let posts = [];
let pending = [];
let closed = 0;
let target = "";
const api = (method, path, body) => {
  if (method === "GET") return Promise.resolve([]);
  posts.push({ path, body });
  return new Promise((resolve) => { pending.push(() => resolve({ id: `job-${posts.length}` })); });
};
const openDialog = (spec) => {
  dialog = spec;
  return { form };
};
const go = (value) => { target = value; };
const document = { querySelector: () => null };
const openLaunchPrompt = new Function(
  "t", "h", "esc", "api", "openDialog", "selectOptions", "chipBox", "templateCredentialFields",
  "selectedCredentialIds", "flushChips", "jsonObject", "bindChips", "bindVaultPicker", "go", "document", "verbosityLabel",
  `${extractFunction("openLaunchPrompt")}\nreturn openLaunchPrompt;`,
)(
  (key) => key, (key) => key, (value) => String(value ?? ""), api, openDialog, () => "", () => "", () => "",
  () => [], () => true, () => ({}), () => {}, () => {}, go, document, () => "verbosity",
);
form = makeForm(makeButton());
await openLaunchPrompt(item, true);
const close = () => { closed += 1; };
const first = dialog.onSubmit({ form, close });
const second = dialog.onSubmit({ form, close });
check(posts.length === 1, "a double submit posts the launch twice");
check(form.button.disabled, "the dialog submit stays enabled in flight");
check(form.button.attrs["aria-busy"] === "true", "the dialog submit has no busy state");
check(posts[0].body.launch_with_options === true, "the launch-with-options flag is missing");
pending.forEach((resolve) => resolve());
await Promise.all([first, second]);
check(posts.length === 1, "a double submit creates more than one job");
check(closed === 1, "the dialog did not close once");
check(target === "#/jobs/job-1", "the dialog did not open the new job");
const page = extractFunction("pageTemplates");
const listLaunch = page.slice(page.indexOf("[data-launch]"), page.indexOf("[data-launch-options]"));
check(listLaunch.includes("button.disabled = true;"), "the list launch no longer disables its button");
check(listLaunch.indexOf("button.disabled = true;") < listLaunch.indexOf('await api("POST"'), "the list launch disables after posting");
"##;

const FAILURE_PROBE: &str = r##"
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
function makeButton() {
  return {
    disabled: false,
    attrs: {},
    setAttribute(name, value) { this.attrs[name] = value; },
    removeAttribute(name) { delete this.attrs[name]; },
  };
}
function makeForm(button) {
  return {
    button,
    extra_vars: { value: "{}" },
    inventory_id: { value: "inv-1" },
    verbosity: { value: "0" },
    job_type: { value: "run" },
    tags: { value: "" },
    skip_tags: { value: "" },
    limit: { value: "" },
    querySelector: (selector) => (selector === 'button[type="submit"]' ? button : null),
    querySelectorAll: () => [],
  };
}
const item = {
  id: "t1",
  tool: "ansible",
  inventory_id: "inv-1",
  extra_vars: {},
  spec: { verbosity: 0, job_type: "run", tags: "", skip_tags: "", limit: "" },
};
let dialog = null;
let form = null;
let posts = [];
let pending = [];
let failNext = true;
let target = "";
const api = (method, path, body) => {
  if (method === "GET") return Promise.resolve([]);
  posts.push({ path, body });
  if (failNext) {
    failNext = false;
    return Promise.reject(new Error("boom"));
  }
  return new Promise((resolve) => { pending.push(() => resolve({ id: `job-${posts.length}` })); });
};
const openDialog = (spec) => {
  dialog = spec;
  return { form };
};
const go = (value) => { target = value; };
const document = { querySelector: () => null };
const openLaunchPrompt = new Function(
  "t", "h", "esc", "api", "openDialog", "selectOptions", "chipBox", "templateCredentialFields",
  "selectedCredentialIds", "flushChips", "jsonObject", "bindChips", "bindVaultPicker", "go", "document", "verbosityLabel",
  `${extractFunction("openLaunchPrompt")}\nreturn openLaunchPrompt;`,
)(
  (key) => key, (key) => key, (value) => String(value ?? ""), api, openDialog, () => "", () => "", () => "",
  () => [], () => true, () => ({}), () => {}, () => {}, go, document, () => "verbosity",
);
form = makeForm(makeButton());
await openLaunchPrompt(item, true);
let failed = false;
const first = dialog.onSubmit({ form, close: () => {} }).catch(() => { failed = true; });
const second = dialog.onSubmit({ form, close: () => {} });
await Promise.all([first, second]);
check(failed, "the failed launch did not reject");
check(posts.length === 1, "the double submit posts after a failure");
check(!form.button.disabled, "a failed launch leaves the dialog submit disabled");
check(!("aria-busy" in form.button.attrs), "a failed launch leaves the busy state");
const retry = dialog.onSubmit({ form, close: () => {} });
check(posts.length === 2, "a retry after failure does not post again");
check(form.button.disabled, "a retry does not disable the dialog submit");
pending.forEach((resolve) => resolve());
await retry;
check(target === "#/jobs/job-2", "the retry did not open the new job");
"##;

#[test]
fn launch_dialog_double_submit_creates_one_job() {
    let js = web_source::source();
    let probe =
        DOUBLE_SUBMIT_PROBE.replace("__JS__", &serde_json::to_string(&js).expect("js json"));
    run_node_probe("double-launch", &probe);
}

#[test]
fn failed_launch_restores_the_dialog_submit() {
    let js = web_source::source();
    let probe = FAILURE_PROBE.replace("__JS__", &serde_json::to_string(&js).expect("js json"));
    run_node_probe("double-launch-failure", &probe);
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
