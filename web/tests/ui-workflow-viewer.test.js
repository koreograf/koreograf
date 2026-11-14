"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { catalog } = require("./catalog.js");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

const templates = [
  { id: "t1", name: "Deploy", tool: "ansible", inventory_id: "i1", spec: { tags: "base", forks: 5 }, extra_vars: { from_template: true } },
  { id: "t2", name: "Recover", tool: "shell", spec: {} },
];

const graph = {
  nodes: [
    {
      id: "n1",
      template_id: "t1",
      name: "One",
      x: 20,
      y: 20,
      convergence: "all",
      overrides: { inventory_id: "i2", spec: { limit: "web", tags: "api,db" }, extra_vars: { from_template: true, added: true } },
    },
    { id: "n2", template_id: "t2", name: "Two", x: 320, y: 20, convergence: "any" },
  ],
  edges: [
    { from: "n1", to: "n2", when: "failure" },
    { from: "n2", to: "n1", when: "success" },
  ],
};

async function mount() {
  const state = await load("core/state.js");
  const i18n = await load("core/i18n.js");
  i18n.setCatalog("de", catalog("de"));
  i18n.setCatalog("en", catalog("en"));
  await i18n.setLang("en");
  state.setMe({ is_admin: false, display_name: "Carol", teams: [{ id: "t1", name: "Plattform", role: "viewer" }] });
  state.setTeamId("t1");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "[]" });
  const editor = await load("features/workflows/editor.js");
  const workflow = { id: "w1", name: "Deploy", description: "", graph };
  editor.mountEditor(workflow, templates, false, {}, {
    inventories: [{ id: "i1", name: "Hosts" }, { id: "i2", name: "Production" }],
    credentials: [{ id: "c1", name: "Deploy key", kind: "ssh" }],
  }, {});
}

function selectNode(id) {
  document.querySelector(`[data-id="${id}"]`).dispatchEvent({
    type: "pointerdown",
    preventDefault() {},
    stopPropagation() {},
    clientX: 20,
    clientY: 20,
    pointerId: 1,
  });
}

function selectEdge(index) {
  const path = document.querySelector(`#edges path[data-edge="${index}"]`);
  document.querySelector("#edges").dispatchEvent({ type: "click", target: path });
}

test("a viewer sees the read-only hint, badge and no editor controls", async () => {
  await mount();
  const hint = document.querySelector("#wf-hint").innerHTML;
  assert.match(hint, /Read-only\. Operators of team Plattform can edit\./);
  assert.doesNotMatch(hint, /drag|right-click|Enter/i, "the viewer hint describes an editor action");
  const help = document.querySelector("#wf-keyboard-help").innerHTML;
  assert.doesNotMatch(help, /Delete|Shift\+F10|connect mode/i, "the viewer help describes an editor action");
  assert.ok(document.querySelector(".top .pill.read-only"), "the header has no read-only badge");
  for (const selector of ["#wf-save", "#wf-launch", "#wf-undo", "#wf-redo", "#wf-delete", "#wf-add-first", "[data-node-menu]", "[data-add-port]"]) {
    assert.equal(document.querySelector(selector), null, `a viewer sees ${selector}`);
  }
});

test("selecting a node shows the template and overrides as read-only details", async () => {
  await mount();
  selectNode("n1");
  const inspector = document.querySelector("#inspector");
  const html = inspector.innerHTML;
  assert.match(html, /Deploy/, "the template is missing");
  assert.match(html, /Ansible/, "the tool is missing");
  assert.match(html, /Production/, "the inventory override is missing");
  assert.match(html, /web/, "the limit override is missing");
  assert.match(html, /api,db/, "the tags override is missing");
  assert.match(html, /added/, "the extra variables override is missing");
  assert.match(html, /All predecessors/, "the convergence is missing");
  assert.match(html, /Outgoing/);
  assert.match(html, /Incoming/);
  for (const selector of ["#node-name", "#node-template", "#node-delete", "#node-connect", "#link-add", "#link-when", "#link-target"]) {
    assert.equal(inspector.querySelector(selector), null, `the read-only panel has ${selector}`);
  }
  assert.equal(inspector.querySelectorAll("input, select, button").length, 0, "the read-only panel has an edit control");
});

test("selecting an edge shows its type and endpoints as read-only details", async () => {
  await mount();
  selectEdge(0);
  const inspector = document.querySelector("#inspector");
  assert.match(inspector.innerHTML, /On failure/);
  assert.match(inspector.innerHTML, /One/);
  assert.match(inspector.innerHTML, /Two/);
  assert.equal(inspector.querySelector("#edge-when"), null);
  assert.equal(inspector.querySelector("#edge-delete"), null);
});
