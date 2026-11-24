"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { catalog } = require("./catalog.js");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
const tick = () => new Promise((resolve) => setImmediate(resolve));
const settle = async () => {
  for (let index = 0; index < 6; index += 1) await tick();
};
const timer = () => new Promise((resolve) => setTimeout(resolve, 5));

const templates = [
  { id: "t1", name: "Deploy", tool: "shell", spec: {} },
  { id: "t2", name: "Recover", tool: "shell", spec: {} },
];

const graph = {
  nodes: [
    { id: "a", template_id: "t1", name: "Deploy", x: 20, y: 20, convergence: "any" },
    { id: "b", template_id: "t2", name: "Recover", x: 320, y: 20, convergence: "any" },
    { id: "c", template_id: "t2", name: "Skipped", x: 620, y: 20, convergence: "any" },
    { id: "d", template_id: "t2", name: "Running", x: 920, y: 20, convergence: "any" },
  ],
  edges: [
    { from: "a", to: "b", when: "failure" },
    { from: "a", to: "c", when: "success" },
  ],
};

const phases = {
  a: {
    node_id: "a",
    phase: "failed",
    job_id: "job-a",
    started_at: "2026-01-01T00:00:00Z",
    finished_at: "2026-01-01T00:00:01Z",
    job_created_at: "2026-01-01T00:00:00Z",
  },
  b: {
    node_id: "b",
    phase: "successful",
    job_id: "job-b",
    started_at: "2026-01-01T00:00:02Z",
    finished_at: "2026-01-01T00:00:04Z",
    job_created_at: "2026-01-01T00:00:02Z",
  },
  c: {
    node_id: "c",
    phase: "skipped",
    job_id: null,
    started_at: null,
    finished_at: null,
    job_created_at: null,
  },
  d: {
    node_id: "d",
    phase: "running",
    job_id: "job-d",
    started_at: null,
    finished_at: null,
    job_created_at: "2026-01-01T00:00:05Z",
  },
};

function run(status = "successful") {
  return {
    id: "run-1",
    workflow_id: "w1",
    name: "Recover",
    status,
    graph,
    nodes: Object.values(phases),
  };
}

async function signedIn() {
  const state = await load("core/state.js");
  const i18n = await load("core/i18n.js");
  i18n.setCatalog("de", catalog("de"));
  i18n.setCatalog("en", catalog("en"));
  state.setMe({ is_admin: false, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
}

function mockFetch(runBody, jobEvents) {
  globalThis.fetch = async (path) => {
    let body = [];
    if (path === "/api/v1/workflow-jobs/run-1") body = runBody;
    else if (path === "/api/v1/templates") body = templates;
    else if (path === "/api/v1/jobs/job-a") body = { status: "failed", events: jobEvents };
    else if (path === "/api/v1/jobs/job-b") body = { status: "successful", events: [] };
    return { ok: true, status: 200, text: async () => JSON.stringify(body) };
  };
}

test("a run node shows its status text, duration and status in its accessible name", async () => {
  await signedIn();
  const editor = await load("features/workflows/editor.js");
  editor.mountEditor(run(), templates, true, phases, null, { onNode() {} });

  const failed = document.querySelector('[data-id="a"]');
  assert.equal(failed.querySelector(".wf-node-status-text").innerHTML, "Fehlgeschlagen");
  assert.equal(failed.querySelector(".wf-node-duration").innerHTML, "1s");
  assert.match(failed.getAttribute("aria-label"), /Fehlgeschlagen/);
  assert.match(failed.getAttribute("aria-label"), /1s/);

  const skipped = document.querySelector('[data-id="c"]');
  assert.equal(skipped.querySelector(".wf-node-status-text").innerHTML, "Übersprungen");
  assert.equal(skipped.querySelector("[data-status]").dataset.status, "skipped");

  const running = document.querySelector('[data-id="d"]');
  assert.equal(running.querySelector(".wf-node-status-text").innerHTML, "Läuft");
  assert.ok(running.querySelector(".wf-node-spinner"), "a running node has no spinner");
  const top = document.querySelector("#run-state-top");
  assert.ok(top, "the narrow run view has no status above the canvas");
  assert.equal(top.innerHTML, "Erfolgreich");
});

test("taken and untaken edges get distinct classes and the legend includes skipped", async () => {
  await signedIn();
  const editor = await load("features/workflows/editor.js");
  editor.mountEditor(run(), templates, true, phases, null, { onNode() {} });

  assert.equal(document.querySelectorAll("#edges path.edge-line.taken").length, 1);
  assert.equal(document.querySelectorAll("#edges path.edge-line.untaken").length, 1);
  assert.ok(document.querySelector("#edges path.failure.taken"), "the failure edge is not marked as taken");
  assert.ok(document.querySelector("#edges path.success.untaken"), "the success edge is not dimmed");
  assert.match(document.querySelector("#inspector").innerHTML, /Übersprungen/);
  assert.match(document.querySelector("#run-summary").innerHTML, /1 Fehler durch einen Fehlerpfad/);
});

test("the output overlay closes on Escape after a pointer open and returns focus", async () => {
  await signedIn();
  mockFetch(run(), [
    { id: 1, stream: "system", line: "CA bundle: /etc/ssl" },
    { id: 2, stream: "stderr", line: "fatal: [web-1]: FAILED! => changed=false" },
  ]);
  const { pageRun } = await load("features/workflows/run.js");
  await pageRun("run-1");
  await settle();

  const node = document.querySelector('[data-id="a"]');
  node.dispatchEvent({ type: "pointerdown", preventDefault() {}, stopPropagation() {}, clientX: 20, clientY: 20, pointerId: 1 });
  await settle();
  await timer();

  const overlay = document.querySelector("#node-output");
  assert.ok(overlay, "the pointer open did not show the overlay");
  assert.ok(overlay.contains(document.activeElement), "focus did not move into the overlay");

  document.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  await settle();
  assert.equal(document.querySelector("#node-output"), null, "Escape did not close the overlay");
  assert.equal(document.activeElement, node, "focus did not return to the node");
});

test("a failed output collapses the setup and starts at the first error", async () => {
  await signedIn();
  mockFetch(run(), [
    { id: 1, stream: "system", line: "CA bundle: /etc/ssl" },
    { id: 2, stream: "system", line: "uid=1000" },
    { id: 3, stream: "stderr", line: "fatal: [web-1]: FAILED! => changed=false" },
  ]);
  const { pageRun } = await load("features/workflows/run.js");
  await pageRun("run-1");
  await settle();

  document.querySelector('[data-id="a"]').dispatchEvent({ type: "pointerdown", preventDefault() {}, stopPropagation() {}, clientX: 20, clientY: 20, pointerId: 1 });
  await settle();

  const log = document.querySelector("#log");
  const setup = log.querySelector(".log-setup");
  assert.ok(setup, "the runner preamble is not collapsed into a setup section");
  assert.ok(!setup.contains(log.querySelector(".fail")), "the first error is inside the setup section");
  assert.match(log.querySelector(".fail").koreografRaw, /fatal:/);
  assert.equal(log.scrollTop, 0, "the log did not start at the first error");
});
