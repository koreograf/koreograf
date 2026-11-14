"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { catalog } = require("./catalog.js");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

const templates = [
  { id: "t1", name: "Deploy", tool: "ansible", inventory_id: "i1", spec: {} },
  { id: "t2", name: "Shell", tool: "shell", spec: {} },
  { id: "t3", name: "Tofu", tool: "opentofu", spec: { action: "plan" } },
];

async function mount(graph) {
  const state = await load("core/state.js");
  const i18n = await load("core/i18n.js");
  i18n.setCatalog("de", catalog("de"));
  state.setMe({ is_admin: false, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "[]" });
  const editor = await load("features/workflows/editor.js");
  const workflow = { id: "w1", name: "Deploy", description: "", graph };
  editor.mountEditor(workflow, templates, false, {}, { inventories: [{ id: "i1", name: "Hosts" }, { id: "i2", name: "Production" }], credentials: [] }, {});
}

function card(id) {
  return document.querySelector(`[data-id="${id}"]`);
}

function select(id) {
  card(id).dispatchEvent({ type: "pointerdown", preventDefault() {}, stopPropagation() {}, clientX: 20, clientY: 20, pointerId: 1 });
}

test("a node card shows its tool and template without the inspector", async () => {
  await mount({
    nodes: [
      { id: "n1", template_id: "t1", name: "One", x: 20, y: 20, convergence: "any" },
      { id: "n2", template_id: "t3", name: "Tofu", x: 320, y: 20, convergence: "any" },
    ],
    edges: [],
  });

  const ansible = card("n1");
  assert.equal(ansible.querySelector(".wf-node-template").innerHTML, "Deploy");
  const tool = ansible.querySelector(".wf-node-tool");
  assert.ok(tool, "the card has no tool icon");
  assert.equal(tool.dataset.tool, "ansible");
  assert.equal(tool.getAttribute("aria-label"), "Ansible");
  assert.equal(ansible.querySelector(".wf-node-chip"), null, "a node without overrides shows chips");

  const tofu = card("n2");
  assert.equal(tofu.querySelector(".wf-node-template").innerHTML, "Tofu");
  assert.equal(tofu.querySelector(".wf-node-tool").dataset.tool, "opentofu");
  assert.equal(tofu.querySelector('[data-chip="apply"]'), null, "a plan node shows the apply chip");
});

test("active overrides appear as chips", async () => {
  await mount({
    nodes: [
      {
        id: "n1",
        template_id: "t1",
        name: "One",
        x: 20,
        y: 20,
        convergence: "any",
        overrides: { inventory_id: "i2", spec: { limit: "web", tags: "api,db" } },
      },
      { id: "n2", template_id: "t3", name: "Tofu", x: 320, y: 20, convergence: "any", overrides: { spec: { action: "apply" } } },
      { id: "n3", template_id: "t3", name: "Plan", x: 620, y: 20, convergence: "any" },
    ],
    edges: [],
  });

  const node = card("n1");
  assert.equal(node.querySelector('[data-chip="inventory"]').innerHTML, "Inventar: Production");
  assert.equal(node.querySelector('[data-chip="limit"]').innerHTML, "Limit: web");
  assert.equal(node.querySelector('[data-chip="tags"]').innerHTML, "Tags: api,db");
  assert.equal(card("n2").querySelector('[data-chip="apply"]').innerHTML, "Übernehmen");
  assert.equal(card("n3").querySelector('[data-chip="apply"]'), null, "a plan node shows the apply chip");
});

test("editing an override updates the card chips", async () => {
  await mount({
    nodes: [{ id: "n1", template_id: "t1", name: "One", x: 20, y: 20, convergence: "any" }],
    edges: [],
  });

  select("n1");
  assert.equal(card("n1").querySelector('[data-chip="limit"]'), null, "an unset limit shows a chip");
  const limit = document.querySelector("#node-limit");
  limit.value = "web";
  limit.dispatchEvent({ type: "change", target: limit });
  assert.equal(card("n1").querySelector('[data-chip="limit"]').innerHTML, "Limit: web");
  assert.equal(card("n1").querySelector('[data-chip="tags"]'), null, "an unset tags override shows a chip");
});
