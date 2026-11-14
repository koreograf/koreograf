"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { stylesheet } = require("./css.js");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
const tick = () => new Promise((resolve) => setImmediate(resolve));

async function setup() {
  const state = await load("core/state.js");
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "[]" });
  const editor = await load("features/workflows/editor.js");
  const menus = await load("features/workflows/menus.js");
  return { state, editor, menus };
}

test("the node menu offers edit, delete and connect and restores focus", async () => {
  const { menus } = await setup();
  const node = { id: "n1", name: "Deploy" };
  const opener = document.createElement("button");
  opener.setAttribute("data-node-menu", "");
  opener.setAttribute("aria-expanded", "false");
  document.body.appendChild(opener);
  const calls = [];
  menus.openNodeMenu(node, 10, 10, opener, {
    edit: (item) => calls.push(["edit", item.id]),
    remove: (item) => calls.push(["remove", item.id]),
    connect: (item) => calls.push(["connect", item.id]),
  });
  const menu = document.querySelector(".node-menu-pop");
  assert.equal(menu.getAttribute("role"), "menu");
  for (const marker of ["data-edit-node", "data-delete-node", "data-connect-node"]) {
    assert.ok(menu.querySelector(`[${marker}]`), `the node menu has no ${marker} item`);
  }
  assert.equal(document.activeElement, menu.querySelector('[role="menuitem"]'));
  menu.querySelector("[data-connect-node]").dispatchEvent({ type: "click" });
  assert.deepEqual(calls, [["connect", "n1"]]);
  assert.equal(opener.getAttribute("aria-expanded"), "false");
  assert.equal(document.activeElement, opener, "closing the menu did not restore focus");
});

test("the canvas menu adds a node at the clicked point", async () => {
  const { menus } = await setup();
  let added = null;
  menus.openCanvasMenu({ x: 40, y: 60 }, 100, 120, (point) => { added = point; });
  const menu = document.querySelector(".node-menu-pop");
  menu.querySelector("[data-add-node]").dispatchEvent({ type: "click" });
  assert.deepEqual(added, { x: 40, y: 60 });
});

test("the edge type overlay returns the chosen kind or null", async () => {
  const { menus } = await setup();
  const success = menus.chooseEdgeType({ name: "A" }, { name: "B" });
  const dialog = document.querySelector("#edge-type-dialog");
  assert.ok(dialog, "the edge type does not use the shared overlay");
  assert.ok(dialog.querySelector("[data-edge-when]"));
  dialog.querySelector('[data-edge-when="failure"]').dispatchEvent({ type: "click" });
  assert.equal(await success, "failure");

  const cancelled = menus.chooseEdgeType({ name: "A" }, { name: "B" });
  const second = document.querySelector("#edge-type-dialog");
  second.querySelector("[data-edge-cancel]").dispatchEvent({ type: "click" });
  assert.equal(await cancelled, null);
});

test("nodes connect from the menu and duplicate edges are refused", async () => {
  const { editor } = await setup();
  const workflow = {
    id: "w1",
    name: "Deploy",
    description: "",
    graph: {
      nodes: [
        { id: "n1", template_id: "t1", name: "One", x: 20, y: 20, convergence: "any" },
        { id: "n2", template_id: "t1", name: "Two", x: 320, y: 20, convergence: "any" },
      ],
      edges: [],
    },
  };
  const templates = [{ id: "t1", name: "Shell", tool: "shell", spec: {} }];
  editor.mountEditor(workflow, templates, false, {}, { inventories: [], credentials: [] }, {});
  assert.match(document.querySelector("#nodes").innerHTML, /data-node-menu/);
  assert.ok(!document.querySelector("#add-node"), "the inspector still has an Add node button");
  assert.equal(document.querySelectorAll('[data-id="n1"] [data-add-port]').length, 3, "the node has no output handles");
  for (const [when, symbol] of [["success", "✓"], ["failure", "✗"], ["always", "↻"]]) {
    const port = document.querySelector(`[data-id="n1"] [data-add-port="${when}"]`);
    assert.ok(port, `the node has no ${when} handle`);
    assert.ok(port.getAttribute("aria-label"), `the ${when} handle has no text alternative`);
    assert.ok(port.closest(".wf-port").innerHTML.includes(symbol), `the ${when} handle has no ${symbol} symbol`);
  }

  document.querySelector('[data-id="n1"] [data-node-menu]').dispatchEvent({ type: "click" });
  document.querySelector("[data-connect-node]").dispatchEvent({ type: "click" });
  assert.equal(document.querySelector("#wf-hint").textContent, "connect_click_or_drag_target");
  assert.ok(document.querySelector('[data-id="n1"]').classList.contains("connecting"));

  document.querySelector('[data-id="n2"]').dispatchEvent({ type: "pointerdown", preventDefault() {}, stopPropagation() {}, clientX: 320, clientY: 40, pointerId: 1 });
  const dialog = document.querySelector("#edge-type-dialog");
  assert.ok(dialog, "clicking a target did not ask for the edge type");
  dialog.querySelector('[data-edge-when="success"]').dispatchEvent({ type: "click" });
  await tick();
  await tick();
  assert.equal(document.querySelectorAll("#edges path.success").length, 1, "the chosen edge is not drawn");

  document.querySelector('[data-id="n1"] [data-node-menu]').dispatchEvent({ type: "click" });
  document.querySelector("[data-connect-node]").dispatchEvent({ type: "click" });
  document.querySelector('[data-id="n2"]').dispatchEvent({ type: "pointerdown", preventDefault() {}, stopPropagation() {}, clientX: 320, clientY: 40, pointerId: 1 });
  document.querySelector('#edge-type-dialog [data-edge-when="success"]').dispatchEvent({ type: "click" });
  await tick();
  await tick();
  assert.equal(document.querySelectorAll("#edges path.success").length, 1, "the same edge was added twice");
  assert.equal(document.querySelector("#wf-hint").textContent, "the_same_edge_is_present_twice");
});

test("a handle plus adds exactly one node connected on that output", async () => {
  for (const when of ["success", "failure", "always"]) {
    await mountGraph({ nodes: [node("n1", "One", 20, 20)], edges: [] });
    const port = document.querySelector(`[data-id="n1"] [data-add-port="${when}"]`);
    assert.ok(port, `the node has no ${when} handle`);
    port.dispatchEvent({ type: "click" });
    const dialog = document.querySelector("#add-node-dialog");
    assert.ok(dialog, "the handle does not open the full node editor");
    const name = dialog.querySelector("#node-name");
    name.value = "Next";
    dialog.querySelector(".dialog-form").dispatchEvent({ type: "submit", preventDefault() {} });
    await tick();
    assert.equal(document.querySelectorAll("#nodes .wf-node").length, 2, `${when} handle did not add exactly one node`);
    assert.equal(document.querySelectorAll(`#edges path.${when}`).length, 1, `${when} handle did not add one ${when} edge`);
    assert.equal(document.querySelectorAll("#edges path").length, 1, `${when} handle added another edge`);
  }
});

test("dropping a connection on empty canvas adds a connected node", async () => {
  await mountGraph({ nodes: [node("n1", "One", 20, 20)], edges: [] });
  document.querySelector('[data-id="n1"] [data-node-menu]').dispatchEvent({ type: "click" });
  document.querySelector("[data-connect-node]").dispatchEvent({ type: "click" });
  const source = document.querySelector('[data-id="n1"]');
  source.dispatchEvent({ type: "pointerdown", preventDefault() {}, stopPropagation() {}, clientX: 20, clientY: 20, pointerId: 1 });
  source.dispatchEvent({ type: "pointerup", clientX: 500, clientY: 300, pointerId: 1 });
  const dialog = document.querySelector("#add-node-dialog");
  assert.ok(dialog, "dropping on empty canvas did not open the Add node editor");
  const name = dialog.querySelector("#node-name");
  name.value = "Next";
  dialog.querySelector(".dialog-form").dispatchEvent({ type: "submit", preventDefault() {} });
  await tick();
  assert.equal(document.querySelectorAll("#nodes .wf-node").length, 2, "the drop added more than one node");
  assert.equal(document.querySelectorAll("#edges path.success").length, 1, "the dropped connection did not add a success edge");
});

test("releasing a connection on the source or controls keeps connect mode", async () => {
  await mountGraph({ nodes: [node("n1", "One", 20, 20), node("n2", "Two", 320, 20)], edges: [] });
  document.querySelector('[data-id="n1"] [data-node-menu]').dispatchEvent({ type: "click" });
  document.querySelector("[data-connect-node]").dispatchEvent({ type: "click" });
  const source = document.querySelector('[data-id="n1"]');
  const original = document.elementFromPoint;
  document.elementFromPoint = () => source;
  source.dispatchEvent({ type: "pointerdown", preventDefault() {}, stopPropagation() {}, clientX: 20, clientY: 20, pointerId: 1 });
  source.dispatchEvent({ type: "pointerup", clientX: 20, clientY: 20, pointerId: 1 });
  assert.equal(document.querySelector("#add-node-dialog"), null, "a click on the source opened Add node");
  assert.equal(document.querySelector("#wf-hint").textContent, "connect_click_or_drag_target");
  assert.ok(source.classList.contains("connecting"), "the source left connect mode");

  document.elementFromPoint = () => document.querySelector("[data-canvas-fit]");
  source.dispatchEvent({ type: "pointerdown", preventDefault() {}, stopPropagation() {}, clientX: 20, clientY: 20, pointerId: 2 });
  source.dispatchEvent({ type: "pointerup", clientX: 20, clientY: 20, pointerId: 2 });
  document.elementFromPoint = original;
  assert.equal(document.querySelector("#add-node-dialog"), null, "a drop on the canvas controls opened Add node");
  assert.equal(document.querySelector("#wf-hint").textContent, "connect_click_or_drag_target");
});

async function mountGraph(graph, { readonly = false, phases = {}, templates } = {}) {
  const { editor } = await setup();
  const workflow = { id: "w1", name: "Deploy", description: "", graph };
  editor.mountEditor(workflow, templates || [{ id: "t1", name: "Shell", tool: "shell", spec: {} }], readonly, phases, { inventories: [], credentials: [] }, {});
  return { editor, workflow };
}

function selectNode(id) {
  document.querySelector(`[data-id="${id}"]`).dispatchEvent({ type: "pointerdown", preventDefault() {}, clientX: 20, clientY: 20, pointerId: 1 });
}

function node(id, name, x, y) {
  return { id, template_id: "t1", name, x, y, convergence: "any" };
}

function outgoingRows() {
  return document.querySelectorAll("#outgoing-edges .edge-row");
}

test("a fan-out renders one row per outgoing edge and drops the single selects", async () => {
  await mountGraph({
    nodes: [node("n1", "One", 20, 20), node("n2", "Two", 320, 20), node("n3", "Three", 320, 220)],
    edges: [
      { from: "n1", to: "n2", when: "success" },
      { from: "n1", to: "n3", when: "success" },
    ],
  });
  selectNode("n1");
  const rows = outgoingRows();
  assert.equal(rows.length, 2, "two success edges do not render two rows");
  assert.equal(rows.filter((row) => row.dataset.edgeWhen === "success").length, 2);
  assert.equal(document.querySelectorAll("#outgoing-edges .pill").length, 2);
  assert.equal(document.querySelectorAll("#outgoing-edges .edge-target").length, 2);
  for (const id of ["link-success", "link-failure", "link-always"]) {
    assert.equal(document.querySelector(`#${id}`), null, `the old ${id} select is still present`);
  }
});

test("changing or removing one fan-out row leaves the other edge", async () => {
  await mountGraph({
    nodes: [node("n1", "One", 20, 20), node("n2", "Two", 320, 20), node("n3", "Three", 320, 220), node("n4", "Four", 620, 20)],
    edges: [
      { from: "n1", to: "n2", when: "success" },
      { from: "n1", to: "n3", when: "success" },
      { from: "n1", to: "n4", when: "failure" },
    ],
  });
  selectNode("n1");
  assert.equal(outgoingRows().length, 3);

  const first = outgoingRows()[0].querySelector(".edge-target");
  first.value = "n3";
  first.dispatchEvent({ type: "change", target: first });
  assert.equal(document.querySelector("#wf-hint").textContent, "the_same_edge_is_present_twice");
  assert.equal(first.value, "n2", "a refused change did not restore the target");

  first.value = "n4";
  first.dispatchEvent({ type: "change", target: first });
  assert.equal(document.querySelector("#wf-hint").textContent, "edge_changed");
  assert.equal(outgoingRows().length, 3, "changing one target dropped another edge");
  assert.equal(document.querySelectorAll("#edges path.success").length, 2);
  assert.equal(document.querySelectorAll("#edges path.failure").length, 1);

  const third = [...outgoingRows()].find((row) => row.querySelector(".edge-target").value === "n3");
  assert.ok(third, "the untouched success edge is missing");
  third.querySelector("[data-remove-edge]").dispatchEvent({ type: "click" });
  assert.equal(document.querySelector("#wf-hint").textContent, "edge_removed");
  assert.equal(outgoingRows().length, 2);
  assert.equal(document.querySelectorAll("#edges path.success").length, 1);
  assert.equal(document.querySelectorAll("#edges path.failure").length, 1);
  assert.equal(outgoingRows()[0].querySelector(".edge-target").value, "n4", "the edited edge did not survive the removal");
});

test("adding a row inserts exactly one edge and duplicate rows are refused", async () => {
  await mountGraph({
    nodes: [node("n1", "One", 20, 20), node("n2", "Two", 320, 20), node("n3", "Three", 320, 220), node("n4", "Four", 620, 20)],
    edges: [
      { from: "n1", to: "n2", when: "success" },
      { from: "n1", to: "n3", when: "failure" },
    ],
  });
  selectNode("n1");
  assert.equal(outgoingRows().length, 2);
  assert.equal(document.querySelector("#link-add").tagName, "BUTTON", "the add action is not a keyboard-reachable button");
  assert.equal(document.querySelector("[data-remove-edge]").tagName, "BUTTON", "the remove action is not a keyboard-reachable button");

  const target = document.querySelector("#link-target");
  target.value = "n4";
  document.querySelector("#link-add").dispatchEvent({ type: "click" });
  assert.equal(document.querySelector("#wf-hint").textContent, "edge_added");
  assert.equal(outgoingRows().length, 3, "adding one row did not add exactly one edge");
  assert.equal(document.querySelectorAll("#edges path.success").length, 2);
  assert.equal(document.querySelectorAll("#edges path.failure").length, 1);
  const targets = [...document.querySelectorAll("#outgoing-edges .edge-target")].map((select) => select.value).sort();
  assert.deepEqual(targets, ["n2", "n3", "n4"]);

  const duplicate = document.querySelector("#link-target");
  duplicate.value = "n2";
  document.querySelector("#link-add").dispatchEvent({ type: "click" });
  assert.equal(document.querySelector("#wf-hint").textContent, "the_same_edge_is_present_twice");
  assert.equal(outgoingRows().length, 3, "a duplicate add changed the edge count");
});

test("Enter on a target in connect mode connects instead of opening Edit node", async () => {
  await mountGraph({
    nodes: [node("n1", "One", 20, 20), node("n2", "Two", 320, 20)],
    edges: [],
  });
  document.querySelector('[data-id="n1"] [data-node-menu]').dispatchEvent({ type: "click" });
  document.querySelector("[data-connect-node]").dispatchEvent({ type: "click" });
  assert.equal(document.querySelector("#wf-hint").textContent, "connect_click_or_drag_target");

  document.querySelector('[data-id="n2"]').dispatchEvent({ type: "keydown", key: "Enter" });
  assert.equal(document.querySelector("#node-edit-dialog"), null, "connect mode opened Edit node");
  const dialog = document.querySelector("#edge-type-dialog");
  assert.ok(dialog, "Enter on the target did not open the edge type choice");
  dialog.querySelector('[data-edge-when="success"]').dispatchEvent({ type: "click" });
  await tick();
  await tick();
  assert.equal(document.querySelectorAll("#edges path.success").length, 1, "Enter did not add the edge");
  assert.equal(document.querySelector("#wf-hint").textContent, "edge_added");
});

test("Space on a target in connect mode connects instead of opening Edit node", async () => {
  await mountGraph({
    nodes: [node("n1", "One", 20, 20), node("n2", "Two", 320, 20)],
    edges: [],
  });
  document.querySelector('[data-id="n1"] [data-node-menu]').dispatchEvent({ type: "click" });
  document.querySelector("[data-connect-node]").dispatchEvent({ type: "click" });

  document.querySelector('[data-id="n2"]').dispatchEvent({ type: "keydown", key: " " });
  assert.equal(document.querySelector("#node-edit-dialog"), null, "connect mode opened Edit node");
  document.querySelector('#edge-type-dialog [data-edge-when="failure"]').dispatchEvent({ type: "click" });
  await tick();
  await tick();
  assert.equal(document.querySelectorAll("#edges path.failure").length, 1, "Space did not add the edge");
});

test("Escape cancels connect mode and restores the default hint", async () => {
  await mountGraph({
    nodes: [node("n1", "One", 20, 20), node("n2", "Two", 320, 20)],
    edges: [],
  });
  document.querySelector('[data-id="n1"] [data-node-menu]').dispatchEvent({ type: "click" });
  document.querySelector("[data-connect-node]").dispatchEvent({ type: "click" });
  document.querySelector('[data-id="n1"]').dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  assert.equal(document.querySelector('[data-id="n1"]').classList.contains("connecting"), false);
  assert.equal(document.querySelector("#wf-hint").textContent, "drag_nodes_and_use_the_canvas_menus");
});

test("a focused edge is selectable and removable from the keyboard", async () => {
  await mountGraph({
    nodes: [node("n1", "One", 20, 20), node("n2", "Two", 320, 20)],
    edges: [{ from: "n1", to: "n2", when: "success" }],
  });
  const path = document.querySelector('#edges path[data-edge="0"]');
  assert.equal(path.getAttribute("tabindex"), "0", "the edge is not focusable");
  assert.equal(path.getAttribute("role"), "button", "the edge has no button role");
  assert.equal(path.getAttribute("aria-label"), "success: One → Two");

  document.querySelector("#edges").dispatchEvent({ type: "keydown", key: "Enter", target: path });
  assert.equal(document.activeElement.getAttribute("data-edge"), "0", "the selected edge lost focus");
  assert.ok(document.querySelector("#edge-when"), "Enter did not select the edge");

  const selected = document.querySelector('#edges path[data-edge="0"]');
  document.querySelector("#edges").dispatchEvent({ type: "keydown", key: "Delete", target: selected });
  assert.equal(document.querySelectorAll("#edges path").length, 0, "Delete did not remove the edge");
  assert.equal(document.querySelector("#wf-hint").textContent, "edge_removed");
  assert.equal(document.activeElement, document.querySelector("#canvas"));
});

test("the workflow fields are labelled and the canvas is described", async () => {
  await mountGraph({
    nodes: [node("n1", "One", 20, 20)],
    edges: [],
  });
  const hint = document.querySelector("#wf-hint");
  assert.equal(hint.getAttribute("role"), "status");
  assert.equal(hint.getAttribute("aria-live"), "polite");

  for (const [id, key] of [["wf-name", "name"], ["wf-description", "description"]]) {
    const input = document.querySelector(`#${id}`);
    const label = input.closest("label");
    assert.ok(label, `#${id} has no visible label`);
    assert.match(label.innerHTML, new RegExp(`<span>${key}</span>`));
  }

  const canvas = document.querySelector("#canvas");
  assert.equal(canvas.getAttribute("role"), "application");
  assert.equal(canvas.getAttribute("aria-label"), "workflow_graph");
  assert.equal(canvas.getAttribute("aria-describedby"), "wf-keyboard-help");
  assert.match(document.querySelector("#wf-keyboard-help").innerHTML, /workflow_keyboard_help/);
  assert.match(document.querySelector('[data-id="n1"]').getAttribute("aria-label"), /One · Shell/);
});

test("the editor and run views name the workflow in the tab title", async () => {
  await mountGraph({
    nodes: [node("n1", "One", 20, 20)],
    edges: [],
  });
  assert.match(document.title, /Deploy/);
  assert.match(document.title, /workflow/);

  const app = await load("core/app.js");
  app.showLoading("Workflows");
  assert.equal(document.title, "Workflows – Koreograf");

  await mountGraph({
    nodes: [node("n1", "One", 20, 20)],
    edges: [],
  }, { readonly: true, phases: { n1: { phase: "successful" } } });
  assert.match(document.title, /Deploy/);
  assert.match(document.title, /run_2/);
  const label = document.querySelector('[data-id="n1"]').getAttribute("aria-label");
  assert.match(label, /Shell/);
  assert.match(label, /successful/);
});

test("the workflow connect styles are present", () => {
  const css = stylesheet();
  assert.ok(css.includes(".node-menu-button"));
  assert.ok(css.includes(".node-menu-pop"));
  assert.ok(css.includes(".edges path.connecting"));
  assert.ok(css.includes(".edge-row"));
  assert.ok(css.includes(".edge-add"));
  assert.ok(css.includes(".canvas-empty"));
  assert.ok(css.includes(".wf-port"));
  assert.ok(css.includes(".wf-port-add"));
});
