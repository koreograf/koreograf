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

async function mount({ readonly = false, templates, graph, role = "owner" } = {}) {
  const state = await load("core/state.js");
  state.setMe({ is_admin: false, display_name: "Admin", teams: [{ id: "t1", name: "Team", role }] });
  state.setTeamId("t1");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "[]" });
  const editor = await load("features/workflows/editor.js");
  const workflow = {
    id: "w1",
    name: "Deploy",
    description: "",
    graph: graph || {
      nodes: [
        { id: "n1", template_id: "t1", name: "One", x: 20, y: 20, convergence: "any" },
        { id: "n2", template_id: "t1", name: "Two", x: 320, y: 20, convergence: "any" },
      ],
      edges: [{ from: "n1", to: "n2", when: "success" }],
    },
  };
  const list = templates || [
    { id: "t1", name: "Deploy", tool: "ansible", inventory_id: "i1", spec: {} },
    { id: "t2", name: "Shell", tool: "shell", spec: {} },
  ];
  editor.mountEditor(workflow, list, readonly, {}, { inventories: [{ id: "i1", name: "Hosts" }], credentials: [] }, {});
  return { editor, workflow };
}

function openCanvasDialog() {
  document.querySelector("#board").dispatchEvent({ type: "contextmenu", preventDefault() {}, clientX: 100, clientY: 100 });
  const menu = document.querySelector(".node-menu-pop");
  assert.ok(menu.querySelector("[data-add-node]"), "the canvas menu has no Add node item");
  menu.querySelector("[data-add-node]").dispatchEvent({ type: "click" });
  const dialog = document.querySelector("#add-node-dialog");
  assert.ok(dialog, "Add node does not use the shared overlay");
  return dialog;
}

test("the inspector adds nodes only through the canvas menu", async () => {
  await mount();
  const inspector = document.querySelector("#inspector");
  assert.ok(!inspector.querySelector("#add-node"), "the inspector still has an Add node button");
  assert.match(inspector.innerHTML, /right_click_empty_canvas_to_add_a_node/);

  const add = openCanvasDialog();
  for (const id of ["node-name", "node-template", "node-join", "node-inventory", "node-credential", "node-extra"]) {
    assert.ok(add.querySelector(`#${id}`), `Add node has no ${id} field`);
  }
  add.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  await tick();
  assert.equal(document.querySelector("#add-node-dialog"), null, "cancel left the Add node dialog open");
  assert.equal(document.querySelectorAll("#nodes .wf-node").length, 2, "cancel added a node");

  const node = document.querySelector('[data-id="n1"]');
  node.dispatchEvent({ type: "pointerdown", clientX: 20, clientY: 20, pointerId: 1 });
  assert.ok(inspector.querySelector("#node-connect"), "the inspector has no Connect action");
  assert.equal(inspector.querySelector("#node-join"), null, "a node with fewer than two predecessors shows convergence");
  assert.match(inspector.innerHTML, /node-inventory/);
  assert.match(inspector.innerHTML, /node-panel-head/);
  assert.match(inspector.innerHTML, /node-section/);

  node.dispatchEvent({ type: "keydown", key: "Enter" });
  assert.equal(document.querySelector("#node-edit-dialog"), null, "Enter opened the Edit node dialog");
  assert.equal(document.activeElement.id, "node-name", "Enter did not focus the panel's first field");

  document.querySelector("#edges").dispatchEvent({ type: "click", target: document.querySelector("#edges path") });
  assert.ok(inspector.querySelector("#edge-when"), "an existing edge cannot change its type");
  assert.ok(inspector.querySelector("#edge-delete"), "an existing edge cannot be deleted");
});

test("the node menu edit action opens the side panel instead of a dialog", async () => {
  await mount();
  document.querySelector('[data-id="n2"] [data-node-menu]').dispatchEvent({ type: "click" });
  document.querySelector("[data-edit-node]").dispatchEvent({ type: "click" });
  assert.equal(document.querySelector("#node-edit-dialog"), null, "Edit node opened a dialog");
  assert.equal(document.activeElement.id, "node-name", "Edit did not focus the panel's first field");
  assert.equal(document.querySelector('[data-id="n2"]').classList.contains("selected"), true, "Edit did not select the node");
});

test("changing a field in the panel updates the graph", async () => {
  await mount();
  document.querySelector('[data-id="n1"]').dispatchEvent({ type: "pointerdown", clientX: 20, clientY: 20, pointerId: 1 });
  const name = document.querySelector("#node-name");
  name.value = "Renamed";
  name.dispatchEvent({ type: "input", target: name });
  assert.equal(document.querySelector('[data-id="n1"] strong').textContent, "Renamed", "the canvas did not take the new name");
  assert.equal(document.querySelector("#wf-status").textContent, "unsaved_changes");

  const template = document.querySelector("#node-template");
  template.value = "t2";
  template.dispatchEvent({ type: "change", target: template });
  assert.match(document.querySelector("#inspector").innerHTML, /value="t2" selected/, "the panel did not take the new template");
});

test("the convergence control needs two predecessors and explains skipped branches", async () => {
  await mount();
  document.querySelector('[data-id="n2"]').dispatchEvent({ type: "pointerdown", clientX: 320, clientY: 20, pointerId: 1 });
  assert.equal(document.querySelector("#node-join"), null, "one predecessor shows convergence");
  assert.equal(document.querySelector('[data-id="n2"] .wf-join-badge'), null, "one predecessor shows a join badge");

  await mount({
    graph: {
      nodes: [
        { id: "n1", template_id: "t1", name: "One", x: 20, y: 20, convergence: "any" },
        { id: "n2", template_id: "t1", name: "Two", x: 320, y: 20, convergence: "any" },
        { id: "n3", template_id: "t1", name: "Three", x: 20, y: 220, convergence: "any" },
      ],
      edges: [
        { from: "n1", to: "n2", when: "success" },
        { from: "n3", to: "n2", when: "failure" },
      ],
    },
  });
  document.querySelector('[data-id="n2"]').dispatchEvent({ type: "pointerdown", clientX: 320, clientY: 20, pointerId: 1 });
  const join = document.querySelector("#node-join");
  assert.ok(join, "two predecessors hide convergence");
  assert.match(document.querySelector("#inspector").innerHTML, /convergence_help/, "the convergence help text is missing");
  const badge = document.querySelector('[data-id="n2"] .wf-join-badge');
  assert.ok(badge, "two predecessors show no join badge");
  assert.equal(badge.innerHTML, "join_badge_any");
  join.value = "all";
  join.dispatchEvent({ type: "change", target: join });
  assert.equal(document.querySelector('[data-id="n2"] .wf-join-badge').textContent, "join_badge_all");
});

test("Add node creates one node with the chosen fields at the clicked point", async () => {
  await mount();
  const add = openCanvasDialog();
  const name = add.querySelector("#node-name");
  name.value = "Created";
  name.dispatchEvent({ type: "input", target: name });
  const extra = add.querySelector("#node-extra");
  extra.value = '{"answer": 42}';
  add.querySelector(".dialog-form").dispatchEvent({ type: "submit", preventDefault() {} });
  await tick();

  const created = document.querySelectorAll("#nodes .wf-node");
  assert.equal(created.length, 3, "submit did not add exactly one node");
  const markup = document.querySelector("#nodes").innerHTML;
  assert.match(markup, /<strong>Created<\/strong>/);
  assert.match(markup, /left:80px;top:80px/);
  const inspector = document.querySelector("#inspector");
  assert.match(inspector.innerHTML, /value="t1" selected/, "the first template is not pre-filled");
  assert.match(inspector.innerHTML, /&quot;answer&quot;: 42/, "the chosen override is not kept");
});

test("an empty canvas adds the first step from the call to action", async () => {
  await mount({ graph: { nodes: [], edges: [] } });
  const cta = document.querySelector("#wf-add-first");
  assert.ok(cta, "the empty canvas has no Add first step action");
  assert.equal(cta.tagName, "BUTTON", "the first step action is not a button");
  cta.focus();
  assert.equal(document.activeElement, cta, "the first step action cannot take focus");

  cta.dispatchEvent({ type: "click" });
  const dialog = document.querySelector("#add-node-dialog");
  assert.ok(dialog, "Add first step does not open the node editor");
  for (const id of ["node-name", "node-template", "node-join", "node-inventory", "node-credential", "node-extra"]) {
    assert.ok(dialog.querySelector(`#${id}`), `Add first step has no ${id} field`);
  }
  dialog.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  await tick();
  assert.equal(document.querySelector("#add-node-dialog"), null, "cancel left the Add node dialog open");
});

test("the first step action is hidden once a node exists", async () => {
  await mount();
  assert.equal(document.querySelector("#wf-empty").hidden, true, "a graph with nodes still shows Add first step");
});

test("viewers get no first step action and no output handles", async () => {
  await mount({ role: "viewer", graph: { nodes: [], edges: [] } });
  assert.equal(document.querySelector("#wf-add-first"), null, "a viewer sees Add first step");

  await mount({
    role: "viewer",
    graph: {
      nodes: [{ id: "n1", template_id: "t1", name: "One", x: 20, y: 20, convergence: "any" }],
      edges: [],
    },
  });
  assert.equal(document.querySelectorAll("[data-add-port]").length, 0, "a viewer sees output handles");
});

test("the keyboard opens the canvas menu without a mouse", async () => {
  await mount();
  const canvas = document.querySelector("#canvas");
  canvas.dispatchEvent({ type: "keydown", key: "ContextMenu", preventDefault() {} });
  assert.ok(document.querySelector(".node-menu-pop"), "the context menu key did not open the canvas menu");

  canvas.dispatchEvent({ type: "keydown", key: "F10", shiftKey: true, preventDefault() {} });
  const menu = document.querySelector(".node-menu-pop");
  assert.ok(menu, "Shift+F10 did not open the canvas menu");
  menu.querySelector("[data-add-node]").dispatchEvent({ type: "click" });
  const dialog = document.querySelector("#add-node-dialog");
  assert.ok(dialog, "the keyboard path cannot add a node");
  dialog.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  await tick();
  assert.equal(document.querySelector("#add-node-dialog"), null, "cancel left the Add node dialog open");
});

test("Enter on a target completes the keyboard connection without editing", async () => {
  await mount();
  const source = document.querySelector('[data-id="n1"]');
  source.dispatchEvent({ type: "contextmenu", preventDefault() {}, stopPropagation() {}, clientX: 10, clientY: 10 });
  const menu = document.querySelector(".node-menu-pop");
  menu.querySelector("[data-connect-node]").dispatchEvent({ type: "click" });
  assert.equal(document.activeElement.dataset.id, "n1", "Connect did not focus the source node");

  document.querySelector('[data-id="n2"]').dispatchEvent({ type: "keydown", key: "Enter" });
  assert.equal(document.querySelector("#node-edit-dialog"), null, "connect mode opened Edit node");
  assert.ok(document.querySelector("#edge-type-dialog"), "Enter on the target did not ask for the edge type");
  document.querySelector("#edge-type-dialog [data-edge-cancel]").dispatchEvent({ type: "click" });
  await tick();
  assert.equal(document.querySelector("#wf-hint").textContent, "connect_cancelled");
});

test("Delete on a focused node confirms before removing it", async () => {
  await mount();
  document.querySelector('[data-id="n1"]').dispatchEvent({ type: "keydown", key: "Delete", preventDefault() {} });
  const dialog = document.querySelector("#confirm-dialog");
  assert.ok(dialog, "Delete did not ask before removing the node");
  dialog.querySelector(".dialog-form").dispatchEvent({ type: "submit", preventDefault() {} });
  await tick();
  assert.equal(document.querySelectorAll("#nodes .wf-node").length, 1, "Delete did not remove exactly one node");
  assert.equal(document.querySelector('[data-id="n1"]'), null);
});

test("the canvas menu explains when no template exists", async () => {
  await mount({ templates: [] });
  document.querySelector("#board").dispatchEvent({ type: "contextmenu", preventDefault() {}, clientX: 100, clientY: 100 });
  const menu = document.querySelector(".node-menu-pop");
  menu.querySelector("[data-add-node]").dispatchEvent({ type: "click" });
  assert.equal(document.querySelector("#add-node-dialog"), null, "an empty dialog was opened");
  assert.equal(document.querySelector("#wf-hint").textContent, "create_a_template_or_a_workflow_first");
});

test("viewers get no add path", async () => {
  await mount({ readonly: true });
  assert.equal(document.querySelector("#inspector #add-node"), null);
  document.querySelector("#board").dispatchEvent({ type: "contextmenu", preventDefault() {}, clientX: 100, clientY: 100 });
  assert.equal(document.querySelector(".node-menu-pop"), null, "a viewer can open the canvas menu");
});

test("workflow node menu copy is in both catalogs", () => {
  for (const id of ["de", "en"]) {
    const values = catalog(id);
    for (const key of ["add_node", "connections", "convergence_help", "convergence_needs_two_predecessors", "join_badge_all", "join_badge_any", "resize_node_panel", "right_click_empty_canvas_to_add_a_node", "add_first_step", "add_step_after", "empty_canvas_hint"]) {
      assert.ok(typeof values[key] === "string" && values[key].trim(), `${key} is missing from ${id}.json`);
    }
    assert.ok(!Object.prototype.hasOwnProperty.call(values, "name_optional"), `name_optional is still in ${id}.json`);
  }
});

test("workflow docs describe the node menus", () => {
  const docs = readFileSync(join(__dirname, "..", "..", "docs", "workflows-webhooks.md"), "utf8");
  assert.ok(docs.includes("Right-click empty canvas to add a node"));
  assert.ok(docs.includes("edit, delete, or connect"));
  assert.ok(!docs.includes("Add node stays in the side panel"));
});
