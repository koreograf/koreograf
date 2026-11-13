"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
const tick = () => new Promise((resolve) => setImmediate(resolve));

async function signedIn() {
  const state = await load("core/state.js");
  state.setMe({ is_admin: false, display_name: "Admin", auth_source: "local", teams: [{ id: "t1", role: "owner" }] });
  state.setTeamId("t1");
}

test("the workflow create dialog posts an empty graph and opens the editor", async () => {
  await signedIn();
  const { openWorkflowDialog } = await load("features/workflows/index.js");
  let posted = null;
  globalThis.fetch = async (path, init) => {
    if (init && init.method === "POST") {
      posted = { path, body: JSON.parse(init.body) };
      return { ok: true, status: 201, text: async () => JSON.stringify({ id: "w1" }) };
    }
    return { ok: true, status: 200, text: async () => "[]" };
  };
  location.hash = "#/workflows";
  openWorkflowDialog();
  const overlay = document.querySelector("#workflow-dialog");
  assert.ok(overlay, "the workflow overlay did not open");
  const form = overlay.querySelector(".dialog-form");
  form.name.value = "Heal";
  form.description.value = "restart";
  form.dispatchEvent({ type: "submit", preventDefault() {} });
  await tick();
  await tick();
  assert.deepEqual(posted, {
    path: "/api/v1/workflows",
    body: { name: "Heal", description: "restart", graph: { nodes: [], edges: [] } },
  });
  assert.equal(document.querySelector("#workflow-dialog"), null, "the workflow overlay stayed open");
  assert.equal(location.hash, "#/workflows/w1");
});

test("a failed workflow create keeps the overlay open", async () => {
  await signedIn();
  const { openWorkflowDialog } = await load("features/workflows/index.js");
  globalThis.fetch = async () => ({
    ok: false,
    status: 400,
    text: async () => JSON.stringify({ error: "workflow_error" }),
  });
  location.hash = "#/workflows";
  openWorkflowDialog();
  const overlay = document.querySelector("#workflow-dialog");
  const form = overlay.querySelector(".dialog-form");
  form.name.value = "Heal";
  form.description.value = "";
  form.dispatchEvent({ type: "submit", preventDefault() {} });
  await tick();
  await tick();
  assert.ok(document.querySelector("#workflow-dialog"), "the failed create closed the overlay");
  assert.equal(location.hash, "#/workflows");
});
