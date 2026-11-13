"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
const catalog = (id) => JSON.parse(readFileSync(join(__dirname, "..", "i18n", `${id}.json`), "utf8"));

async function mount() {
  const state = await load("core/state.js");
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "[]" });
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
  const templates = [{ id: "t1", name: "Deploy", tool: "ansible", inventory_id: "i1", spec: {} }];
  editor.mountEditor(workflow, templates, false, {}, { inventories: [{ id: "i1", name: "Hosts" }], credentials: [] }, {});
  return { editor, workflow };
}

test("workflow nodes use canvas and node context menus", async () => {
  await mount();
  const inspector = document.querySelector("#inspector");
  assert.ok(inspector.querySelector("#add-node"), "the inspector has no Add node button");
  assert.ok(!inspector.innerHTML.includes("add-template"), "the inspector still has a template select");

  inspector.querySelector("#add-node").dispatchEvent({ type: "click" });
  const add = document.querySelector("#add-node-dialog");
  assert.ok(add, "Add node does not use the shared overlay");
  assert.ok(add.querySelector("#add-template"), "Add node has no template picker");
  assert.match(add.innerHTML, /name_optional/);
  add.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  document.querySelector("#add-node-dialog")?.remove();

  document.querySelector("#board").dispatchEvent({ type: "contextmenu", preventDefault() {}, clientX: 100, clientY: 100 });
  const canvasMenu = document.querySelector(".node-menu-pop");
  assert.ok(canvasMenu.querySelector("[data-add-node]"), "the canvas menu has no Add node item");
  canvasMenu.remove();

  const node = document.querySelector('[data-id="n1"]');
  node.dispatchEvent({ type: "pointerdown", clientX: 20, clientY: 20, pointerId: 1 });
  assert.ok(inspector.querySelector("#node-connect"), "the inspector has no Connect action");
  assert.ok(inspector.querySelector("#node-join"), "the inspector has no convergence field");
  assert.match(inspector.innerHTML, /node-inventory/);

  node.dispatchEvent({ type: "keydown", key: "Enter" });
  const edit = document.querySelector("#node-edit-dialog");
  assert.ok(edit, "Edit node does not use the shared overlay");
  assert.ok(edit.querySelector("#node-join"), "Edit node has no convergence field");
  assert.match(edit.innerHTML, /node-inventory/);
  edit.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  document.querySelector("#node-edit-dialog")?.remove();

  document.querySelector("#edges").dispatchEvent({ type: "click", target: document.querySelector("#edges path") });
  assert.ok(inspector.querySelector("#edge-when"), "an existing edge cannot change its type");
  assert.ok(inspector.querySelector("#edge-delete"), "an existing edge cannot be deleted");
});

test("workflow node menu copy is in both catalogs", () => {
  for (const id of ["de", "en"]) {
    const values = catalog(id);
    for (const key of ["edit_node", "name_optional", "right_click_empty_canvas_to_add_a_node"]) {
      assert.ok(typeof values[key] === "string" && values[key].trim(), `${key} is missing from ${id}.json`);
    }
  }
});

test("workflow docs describe the node menus", () => {
  const docs = readFileSync(join(__dirname, "..", "..", "docs", "workflows-webhooks.md"), "utf8");
  assert.ok(docs.includes("Right-click empty canvas to add a node"));
  assert.ok(docs.includes("edit, delete, or connect"));
  assert.ok(!docs.includes("Add node stays in the side panel"));
});
