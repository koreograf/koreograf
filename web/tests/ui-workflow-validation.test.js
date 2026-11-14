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

function node(id, name, x = 20, y = 20, templateId = "t1") {
  return { id, template_id: templateId, name, x, y, convergence: "any" };
}

async function mount(graph, templates, readonly = false) {
  const state = await load("core/state.js");
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "[]" });
  const editor = await load("features/workflows/editor.js");
  const workflow = { id: "w1", name: "Deploy", description: "", graph };
  editor.mountEditor(workflow, templates || [{ id: "t1", name: "Shell", tool: "shell", spec: {} }], readonly, {}, { inventories: [], credentials: [] }, {});
  return { editor, workflow };
}

function startConnect(id) {
  document.querySelector(`[data-id="${id}"] [data-node-menu]`).dispatchEvent({ type: "click" });
  document.querySelector("[data-connect-node]").dispatchEvent({ type: "click" });
}

function clickNode(id) {
  document.querySelector(`[data-id="${id}"]`).dispatchEvent({
    type: "pointerdown",
    preventDefault() {},
    stopPropagation() {},
    clientX: 20,
    clientY: 20,
    pointerId: 1,
  });
}

function edges() {
  return document.querySelectorAll("#edges path.edge-line");
}

test("a connection back to an ancestor is refused and explained", async () => {
  await mount({
    nodes: [node("n1", "One"), node("n2", "Two", 320)],
    edges: [{ from: "n1", to: "n2", when: "success" }],
  });

  startConnect("n2");
  clickNode("n1");
  await tick();

  assert.equal(document.querySelector("#edge-type-dialog"), null, "the cycle opened the edge type dialog");
  assert.equal(document.querySelector("#wf-hint").textContent, "connection_would_create_a_cycle");
  assert.equal(edges().length, 1, "the cycle edge was added");

  startConnect("n1");
  const source = document.querySelector('[data-id="n1"]');
  const original = document.elementFromPoint;
  document.elementFromPoint = () => source;
  source.dispatchEvent({ type: "pointerdown", preventDefault() {}, stopPropagation() {}, clientX: 20, clientY: 20, pointerId: 1 });
  source.dispatchEvent({ type: "pointermove", clientX: 80, clientY: 80, pointerId: 1 });
  source.dispatchEvent({ type: "pointerup", clientX: 80, clientY: 80, pointerId: 1 });
  document.elementFromPoint = original;

  assert.equal(document.querySelector("#edge-type-dialog"), null, "the self-loop opened the edge type dialog");
  assert.equal(document.querySelector("#wf-hint").textContent, "an_edge_cannot_point_at_the_same_node");
  assert.equal(edges().length, 1, "the self-loop edge was added");
});

test("a node whose template left the catalog shows a badge", async () => {
  await mount(
    {
      nodes: [node("n1", "One", 20, 20, "gone"), node("n2", "Two", 320)],
      edges: [{ from: "n1", to: "n2", when: "success" }],
    },
    [{ id: "t1", name: "Shell", tool: "shell", spec: {} }],
  );

  const badge = document.querySelector('[data-id="n1"] .wf-node-badge');
  assert.ok(badge, "the node with the missing template has no badge");
  assert.equal(badge.innerHTML, "node_template_missing_badge");
  assert.equal(badge.getAttribute("title"), "template.missing_named");
  assert.equal(document.querySelector('[data-id="n2"] .wf-node-badge'), null, "a valid node has a badge");
  assert.equal(document.querySelector("#wf-launch").disabled, true, "Launch is enabled with a missing template");
});

test("an unreachable node shows a warning badge", async () => {
  await mount({
    nodes: [node("n1", "One"), node("n2", "Two", 320), node("n3", "Three", 620, 220), node("n4", "Four", 920, 220)],
    edges: [
      { from: "n1", to: "n2", when: "success" },
      { from: "n3", to: "n4", when: "success" },
      { from: "n4", to: "n3", when: "success" },
    ],
  });

  const badge = document.querySelector('[data-id="n3"] .wf-node-badge');
  assert.ok(badge, "the unreachable node has no badge");
  assert.equal(badge.innerHTML, "node_unreachable_badge");
  assert.equal(badge.classList.contains("warning"), true, "the unreachable badge is not a warning");
});

test("an unconnected node shows a warning badge without blocking Launch", async () => {
  await mount({
    nodes: [node("n1", "One"), node("n2", "Two", 320), node("n3", "Three", 620, 220)],
    edges: [{ from: "n1", to: "n2", when: "success" }],
  });

  const badge = document.querySelector('[data-id="n3"] .wf-node-badge');
  assert.ok(badge, "the unconnected node has no badge");
  assert.equal(badge.innerHTML, "node_unconnected_badge");
  assert.equal(badge.classList.contains("warning"), true, "the unconnected badge is not a warning");
  assert.equal(document.querySelector("#wf-launch").disabled, false, "a warning blocks Launch");
});

test("Launch is disabled for an empty graph and for a graph with errors", async () => {
  await mount({ nodes: [], edges: [] });
  let launch = document.querySelector("#wf-launch");
  assert.equal(launch.disabled, true, "Launch is enabled for an empty graph");
  assert.equal(document.querySelector("#wf-validation").textContent, "the_workflow_has_no_nodes_yet");
  assert.equal(launch.getAttribute("title"), "the_workflow_has_no_nodes_yet");

  await mount({
    nodes: [node("n1", "One"), node("n2", "Two", 320), node("n3", "Three", 620)],
    edges: [
      { from: "n1", to: "n2", when: "success" },
      { from: "n2", to: "n3", when: "success" },
      { from: "n3", to: "n2", when: "success" },
    ],
  });
  launch = document.querySelector("#wf-launch");
  assert.equal(launch.disabled, true, "Launch is enabled with a cycle");
  assert.equal(document.querySelector("#wf-validation").textContent, "the_workflow_contains_a_cycle");

  await mount({
    nodes: [node("n1", "One"), node("n2", "Two", 320)],
    edges: [{ from: "n1", to: "n2", when: "success" }],
  });
  launch = document.querySelector("#wf-launch");
  assert.equal(launch.disabled, false, "Launch is disabled for a valid graph");
  assert.equal(document.querySelector("#wf-validation").textContent, "graph_is_valid");
});

test("a mocked server cycle error marks the involved edge", async () => {
  await mount({
    nodes: [node("n1", "One"), node("n2", "Two", 320)],
    edges: [
      { from: "n1", to: "n2", when: "success" },
      { from: "n2", to: "n1", when: "success" },
    ],
  });

  globalThis.fetch = async (path, init = {}) => {
    if (init.method === "PUT") {
      return { ok: false, status: 400, text: async () => JSON.stringify({ error: "the_workflow_contains_a_cycle" }) };
    }
    return { ok: true, status: 200, text: async () => "{}" };
  };

  document.querySelector("#wf-save").dispatchEvent({ type: "click" });
  await settle();

  const marked = document.querySelectorAll("#edges path.server-invalid");
  assert.equal(marked.length, 2, "the server cycle error did not mark both cycle edges");
  assert.ok(document.querySelector('#edges path[data-edge="0"]').classList.contains("server-invalid"));
  assert.ok(document.querySelector('#edges path[data-edge="1"]').classList.contains("server-invalid"));
  assert.equal(document.querySelector("#wf-hint").textContent, "the_workflow_contains_a_cycle");
});

test("a server missing-template error marks the affected node", async () => {
  await mount({
    nodes: [node("n1", "One"), node("n2", "Two", 320)],
    edges: [{ from: "n1", to: "n2", when: "success" }],
  });

  globalThis.fetch = async (path, init = {}) => {
    if (init.method === "PUT") {
      return {
        ok: false,
        status: 400,
        text: async () => JSON.stringify({ error: "template.missing_named", vars: { name: "One" } }),
      };
    }
    return { ok: true, status: 200, text: async () => "{}" };
  };

  document.querySelector("#wf-save").dispatchEvent({ type: "click" });
  await settle();

  assert.ok(document.querySelector('[data-id="n1"] .wf-node-badge'), "the server error did not badge the node");
  assert.ok(document.querySelector('[data-id="n1"]').classList.contains("invalid"));
  assert.equal(document.querySelector("#wf-hint").textContent, "template.missing_named");
});

test("the run view leaves editor validation out", async () => {
  await mount(
    { nodes: [node("n1", "One", 20, 20, "gone")], edges: [] },
    [{ id: "t1", name: "Shell", tool: "shell", spec: {} }],
    true,
  );

  assert.equal(document.querySelector('[data-id="n1"] .wf-node-badge'), null, "the run view shows an editor badge");
  assert.equal(document.querySelector('[data-id="n1"]').classList.contains("invalid"), false);
});
