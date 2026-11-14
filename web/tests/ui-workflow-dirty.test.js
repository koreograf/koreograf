"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
const tick = () => new Promise((resolve) => setImmediate(resolve));
const settle = async () => {
  for (let index = 0; index < 5; index += 1) await tick();
};

async function mount() {
  const state = await load("core/state.js");
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  const calls = [];
  globalThis.fetch = async (path, init = {}) => {
    calls.push({ method: init.method || "GET", path, body: init.body });
    if (path.endsWith("/launch")) return { ok: true, status: 200, text: async () => JSON.stringify({ id: "run-1" }) };
    return { ok: true, status: 200, text: async () => "{}" };
  };
  const editor = await load("features/workflows/editor.js");
  const workflow = {
    id: "w1",
    name: "Deploy",
    description: "",
    graph: {
      nodes: [
        { id: "n1", template_id: "t1", name: "One", x: 20, y: 20, convergence: "any" },
        { id: "n2", template_id: "t1", name: "Two", x: 320, y: 20, convergence: "any" },
      ],
      edges: [{ from: "n1", to: "n2", when: "success" }],
    },
  };
  const templates = [{ id: "t1", name: "Shell", tool: "shell", spec: {} }];
  editor.mountEditor(workflow, templates, false, {}, { inventories: [], credentials: [] }, {});
  return { state, workflow, calls };
}

function moveNode(id = "n1") {
  document.querySelector(`[data-id="${id}"]`).dispatchEvent({ type: "keydown", key: "ArrowRight", preventDefault() {} });
}

function selectNode(id = "n1") {
  document.querySelector(`[data-id="${id}"]`).dispatchEvent({ type: "pointerdown", clientX: 20, clientY: 20, pointerId: 1 });
}

function statusText() {
  return document.querySelector("#wf-status").textContent;
}

test("moving or renaming a node marks the editor dirty and registers the leave guard", async () => {
  const { state } = await mount();
  assert.equal(statusText(), "all_changes_saved");
  assert.equal(state.leaveGuard(), false, "a fresh editor reports unsaved changes");

  moveNode();
  assert.equal(statusText(), "unsaved_changes", "moving a node does not mark the editor dirty");
  assert.equal(state.leaveGuard(), true, "the workflow editor does not register the leave guard");
  assert.ok(document.querySelector("#wf-save").classList.contains("primary"), "Save is not primary while dirty");
  assert.ok(!document.querySelector("#wf-launch").classList.contains("primary"), "Launch stays primary while dirty");

  selectNode();
  const name = document.querySelector("#node-name");
  name.value = "Renamed";
  name.dispatchEvent({ type: "input", target: name });
  assert.equal(document.querySelector('[data-id="n1"] strong').textContent, "Renamed");
  assert.equal(statusText(), "unsaved_changes", "renaming a node does not mark the editor dirty");
});

test("Save clears the dirty state and the leave guard without replacing the hint", async () => {
  const { state, calls } = await mount();
  moveNode();
  document.querySelector("#wf-save").dispatchEvent({ type: "click" });
  await settle();

  assert.equal(calls.filter((call) => call.method === "PUT").length, 1, "Save did not put the workflow once");
  assert.equal(statusText(), "all_changes_saved", "Save does not show the saved state");
  assert.equal(state.leaveGuard, null, "Save keeps the leave guard");
  assert.ok(!document.querySelector("#wf-save").classList.contains("primary"), "Save stays primary after a save");
  assert.ok(document.querySelector("#wf-launch").classList.contains("primary"), "Launch is not primary after a save");
  assert.match(document.querySelector("#wf-hint").innerHTML, /drag_nodes_and_use_the_canvas_menus/);

  const description = document.querySelector("#wf-description");
  description.value = "Changed";
  description.dispatchEvent({ type: "input", target: description });
  assert.equal(statusText(), "unsaved_changes", "editing the description does not mark the editor dirty");
  assert.equal(state.leaveGuard(), true, "the leave guard is not restored after a later edit");
});

test("a dirty launch asks first and Cancel keeps the graph unsaved", async () => {
  const { calls } = await mount();
  moveNode();
  document.querySelector("#wf-launch").dispatchEvent({ type: "click" });
  await settle();

  assert.equal(calls.filter((call) => call.method === "PUT").length, 0, "Launch saved the graph before asking");
  assert.equal(calls.filter((call) => call.method === "POST").length, 0, "Launch started a run before asking");
  const dialog = document.querySelector("#workflow-launch-dialog");
  assert.ok(dialog, "a dirty launch did not show the choice");
  for (const marker of ["data-launch-save", "data-launch-saved", "data-launch-cancel"]) {
    assert.ok(dialog.querySelector(`[${marker}]`), `the launch choice has no ${marker} action`);
  }

  dialog.querySelector("[data-launch-cancel]").dispatchEvent({ type: "click" });
  await settle();
  assert.equal(calls.filter((call) => call.method === "PUT").length, 0, "Cancel saved the graph");
  assert.equal(calls.filter((call) => call.method === "POST").length, 0, "Cancel launched the workflow");
  assert.equal(statusText(), "unsaved_changes", "Cancel cleared the dirty state");
});

test("the last saved launch skips the PUT and the save choice persists first", async () => {
  const { state, calls } = await mount();
  moveNode();
  document.querySelector("#wf-launch").dispatchEvent({ type: "click" });
  await settle();
  document.querySelector("#workflow-launch-dialog [data-launch-saved]").dispatchEvent({ type: "click" });
  await settle();

  assert.equal(calls.filter((call) => call.method === "PUT").length, 0, "the last saved launch persisted the graph");
  assert.equal(calls.filter((call) => call.method === "POST").length, 1, "the last saved launch did not start a run");
  assert.equal(state.leaveGuard, null, "the run navigation still asks to discard the unsaved graph");

  moveNode("n2");
  document.querySelector("#wf-launch").dispatchEvent({ type: "click" });
  await settle();
  document.querySelector("#workflow-launch-dialog [data-launch-save]").dispatchEvent({ type: "click" });
  await settle();

  const methods = calls.filter((call) => call.method === "PUT" || call.method === "POST").map((call) => call.method);
  assert.deepEqual(methods, ["POST", "PUT", "POST"], "Save and launch did not persist before starting");
});

test("Ctrl+S and Cmd+S save a dirty editor and do nothing while clean", async () => {
  const { calls } = await mount();
  const canvas = document.querySelector("#canvas");
  canvas.dispatchEvent({ type: "keydown", key: "s", ctrlKey: true, preventDefault() {} });
  await settle();
  assert.equal(calls.filter((call) => call.method === "PUT").length, 0, "a clean editor saved on Ctrl+S");

  moveNode();
  canvas.dispatchEvent({ type: "keydown", key: "s", ctrlKey: true, preventDefault() {} });
  await settle();
  assert.equal(calls.filter((call) => call.method === "PUT").length, 1, "Ctrl+S did not save");
  assert.equal(statusText(), "all_changes_saved");

  moveNode();
  canvas.dispatchEvent({ type: "keydown", key: "s", metaKey: true, preventDefault() {} });
  await settle();
  assert.equal(calls.filter((call) => call.method === "PUT").length, 2, "Cmd+S did not save");
  assert.equal(statusText(), "all_changes_saved");
});

test("typing an override marks the editor dirty before the field loses focus", async () => {
  const { calls } = await mount();
  selectNode();
  const extra = document.querySelector("#node-extra");
  extra.value = '{"typed": true}';
  extra.dispatchEvent({ type: "input", target: extra });
  assert.equal(statusText(), "unsaved_changes", "typing an override does not mark the editor dirty");

  document.querySelector("#canvas").dispatchEvent({ type: "keydown", key: "s", ctrlKey: true, preventDefault() {} });
  await settle();
  const puts = calls.filter((call) => call.method === "PUT");
  assert.equal(puts.length, 1, "Ctrl+S did not save the focused override");
  const saved = JSON.parse(puts[0].body).graph.nodes.find((node) => node.id === "n1");
  assert.deepEqual(saved.overrides.extra_vars, { typed: true });
});

test("creating a workflow shows Created once instead of a lingering Saved banner", async () => {
  await mount();
  const app = await load("core/app.js");
  const index = await load("features/workflows/index.js");
  globalThis.fetch = async (path, init = {}) => {
    if (init.method === "POST") return { ok: true, status: 200, text: async () => JSON.stringify({ id: "w9" }) };
    return { ok: true, status: 200, text: async () => "[]" };
  };
  index.openWorkflowDialog();
  const dialog = document.querySelector("#workflow-dialog");
  assert.ok(dialog, "the create dialog did not open");
  const form = dialog.querySelector(".dialog-form");
  assert.ok(form, "the create dialog has no form");
  const name = form.querySelector("[name=name]");
  name.value = "New workflow";
  name.dispatchEvent({ type: "input", target: name });
  form.dispatchEvent({ type: "submit", preventDefault() {} });
  await settle();

  assert.equal(document.querySelector("#workflow-dialog"), null, "the create dialog stayed open");
  const banner = app.savedBanner();
  assert.match(banner, /created/, "the created workflow did not show Created");
  assert.doesNotMatch(banner, /saved/i, "the created workflow still shows a saved banner");
});
