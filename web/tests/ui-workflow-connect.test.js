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
  assert.ok(!document.querySelector("#nodes").innerHTML.includes("data-port"));

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

test("the workflow connect styles are present", () => {
  const css = stylesheet();
  assert.ok(css.includes(".node-menu-button"));
  assert.ok(css.includes(".node-menu-pop"));
  assert.ok(css.includes(".edges path.connecting"));
});
