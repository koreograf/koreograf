"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { catalog } = require("./catalog.js");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
const tick = () => new Promise((resolve) => setImmediate(resolve));

async function mount(role) {
  const state = await load("core/state.js");
  state.setMe({ is_admin: role === "owner", display_name: "User", teams: [{ id: "t1", name: "Team", role }] });
  state.setTeamId("t1");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "[]" });
  const editor = await load("features/workflows/editor.js");
  const workflow = { id: "w1", name: "Deploy", description: "", graph: { nodes: [], edges: [] } };
  editor.mountEditor(workflow, [], false, {}, { inventories: [], credentials: [] }, {});
  return state;
}

test("the workflow editor groups its sections in page tabs", async () => {
  await mount("owner");
  const app = document.querySelector("#app");
  assert.match(app.innerHTML, /role="tablist"/);
  for (const id of ["editor", "schedules", "danger"]) {
    assert.ok(app.innerHTML.includes(`data-workflow-tab="${id}"`), `the ${id} tab is missing`);
  }
  assert.ok(app.innerHTML.includes('id="wf-panel-schedules"'), "the schedules panel is missing");
  assert.ok(app.innerHTML.includes('id="wf-delete"'), "the delete action is gone");
  await tick();
  assert.match(document.querySelector("#schedule-card").innerHTML, /schedule-add/, "schedules are no longer mounted");

  document.querySelector('[data-workflow-tab="schedules"]').dispatchEvent({ type: "click" });
  assert.equal(location.hash, "#/workflows/w1?tab=schedules", "the tab choice is not in the address");
  assert.equal(document.querySelector("#wf-panel-schedules").hidden, false);
  assert.equal(document.querySelector("#wf-panel-editor").hidden, true);

  const editorTab = document.querySelector('[data-workflow-tab="editor"]');
  editorTab.focus();
  editorTab.dispatchEvent({ type: "keydown", key: "ArrowRight", preventDefault() {} });
  assert.equal(document.activeElement.dataset.workflowTab, "schedules", "ArrowRight did not move the tab");
  assert.equal(location.hash, "#/workflows/w1?tab=schedules");
});

test("viewers do not get the danger zone tab", async () => {
  await mount("viewer");
  const app = document.querySelector("#app");
  assert.ok(!app.innerHTML.includes('data-workflow-tab="danger"'), "viewers get the Danger Zone tab");
  assert.ok(!app.innerHTML.includes('id="wf-delete"'), "viewers get the delete action");
  assert.ok(app.innerHTML.includes('data-workflow-tab="schedules"'));
});

test("the docs place schedules and the danger zone in tabs", () => {
  const workflows = readFileSync(join(__dirname, "..", "..", "docs", "workflows-webhooks.md"), "utf8");
  const jobs = readFileSync(join(__dirname, "..", "..", "docs", "jobs.md"), "utf8");
  assert.ok(workflows.includes("?tab=schedules"));
  assert.ok(workflows.includes("Danger Zone"));
  assert.ok(jobs.includes("**Schedules** tab"));
  assert.ok(jobs.includes("?tab=schedules&edit=<id>"));
});

test("the tab label is in both catalogs", () => {
  for (const id of ["de", "en"]) {
    const values = catalog(id);
    assert.ok(typeof values.editor === "string" && values.editor.trim(), `editor is missing from ${id}.json`);
  }
});
