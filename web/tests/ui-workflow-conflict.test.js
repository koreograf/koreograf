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

function response(status, body) {
  return { ok: status >= 200 && status < 300, status, text: async () => JSON.stringify(body) };
}

function graph(name) {
  return {
    nodes: [{ id: "n1", template_id: "t1", name, x: 20, y: 20, convergence: "any" }],
    edges: [],
  };
}

async function mount(handler) {
  const state = await load("core/state.js");
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  const calls = [];
  globalThis.fetch = async (path, init = {}) => {
    const method = init.method || "GET";
    const body = init.body ? JSON.parse(init.body) : null;
    calls.push({ method, path, body });
    return handler(method, path, body);
  };
  const editor = await load("features/workflows/editor.js");
  const workflow = { id: "w1", name: "Deploy", description: "", version: 4, graph: graph("One") };
  editor.mountEditor(workflow, [{ id: "t1", name: "Shell", tool: "shell", spec: {} }], false, {}, { inventories: [], credentials: [] }, {});
  return { state, workflow, calls };
}

function moveNode() {
  document.querySelector('[data-id="n1"]').dispatchEvent({ type: "keydown", key: "ArrowRight", preventDefault() {} });
}

function save() {
  document.querySelector("#wf-save").dispatchEvent({ type: "click" });
}

test("a stale save opens the conflict dialog and reload fetches without another PUT", async () => {
  let puts = 0;
  let gets = 0;
  const { state } = await mount((method, path) => {
    if (method === "PUT") {
      puts += 1;
      return response(409, { error: "workflow_changed", vars: { version: "5" } });
    }
    if (method === "GET" && path.endsWith("/w1")) {
      gets += 1;
      return response(200, { id: "w1", name: "Deploy", description: "", version: 5, graph: graph("Server") });
    }
    return response(200, {});
  });

  moveNode();
  save();
  await settle();

  const dialog = document.querySelector("#workflow-conflict-dialog");
  assert.ok(dialog, "a stale save did not open the conflict dialog");
  for (const marker of ["data-conflict-reload", "data-conflict-overwrite", "data-conflict-cancel"]) {
    assert.ok(dialog.querySelector(`[${marker}]`), `the conflict dialog has no ${marker} action`);
  }
  assert.equal(puts, 1, "the stale save did not try once");

  dialog.querySelector("[data-conflict-reload]").dispatchEvent({ type: "click" });
  await settle();

  assert.equal(puts, 1, "reload saved the workflow again");
  assert.equal(gets, 1, "reload did not fetch the current workflow");
  assert.equal(document.querySelector("#workflow-conflict-dialog"), null, "reload kept the conflict dialog open");
  const label = document.querySelector('[data-id="n1"] strong');
  assert.equal(label.textContent || label.innerHTML, "Server", "reload kept the old graph");
  assert.equal(document.querySelector("#wf-status").textContent, "all_changes_saved", "reload left the editor dirty");
  assert.equal(state.leaveGuard, null, "reload kept the leave guard");
});

test("overwrite retries with the current version and keeps the local graph", async () => {
  const puts = [];
  const { workflow } = await mount((method, path, body) => {
    if (method === "PUT") {
      puts.push(body);
      if (puts.length === 1) return response(409, { error: "workflow_changed", vars: { version: "5" } });
      return response(200, { id: "w1", name: body.name, description: body.description, graph: body.graph, version: 6 });
    }
    if (method === "GET" && path.endsWith("/w1")) {
      return response(200, { id: "w1", name: "Deploy", description: "", version: 5, graph: graph("Server") });
    }
    return response(200, {});
  });

  moveNode();
  save();
  await settle();
  assert.ok(document.querySelector("#workflow-conflict-dialog"), "a stale save did not open the conflict dialog");
  assert.equal(puts[0].version, 4, "the first save did not send the loaded version");

  document.querySelector("#workflow-conflict-dialog [data-conflict-overwrite]").dispatchEvent({ type: "click" });
  await settle();

  assert.equal(puts.length, 2, "overwrite did not retry the save");
  assert.equal(puts[1].version, 5, "overwrite did not send the current version");
  assert.equal(puts[1].graph.nodes[0].name, "One", "overwrite sent the server graph instead of the local one");
  assert.equal(document.querySelector("#workflow-conflict-dialog"), null, "overwrite kept the conflict dialog open");
  assert.equal(document.querySelector("#wf-status").textContent, "all_changes_saved", "overwrite left the editor dirty");
  assert.equal(workflow.version, 6, "overwrite did not keep the saved version");

  moveNode();
  save();
  await settle();
  assert.equal(puts.length, 3, "the save after overwrite did not run");
  assert.equal(puts[2].version, 6, "the save after overwrite did not send the new version");
});
