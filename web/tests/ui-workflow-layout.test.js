"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { catalog } = require("./catalog.js");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

function node(id, x = 20, y = 20) {
  return { id, template_id: "t1", name: id, x, y, convergence: "any" };
}

async function mount(graph) {
  const state = await load("core/state.js");
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "{}" });
  const editor = await load("features/workflows/editor.js");
  const workflow = { id: "w1", name: "Deploy", description: "", graph };
  editor.mountEditor(workflow, [{ id: "t1", name: "Shell", tool: "shell", spec: {} }], false, {}, { inventories: [], credentials: [] }, {});
  return { state };
}

function leftOf(id) {
  const element = document.querySelector(`[data-id="${id}"]`);
  const attribute = /left:([^;]+)/.exec(element.getAttribute("style") || "");
  return element.style.left || (attribute ? attribute[1] : undefined);
}

function topOf(id) {
  const element = document.querySelector(`[data-id="${id}"]`);
  const attribute = /top:([^;]+)/.exec(element.getAttribute("style") || "");
  return element.style.top || (attribute ? attribute[1] : undefined);
}

function click(selector) {
  document.querySelector(selector).dispatchEvent({ type: "click" });
}

test("a released node sits on the visible grid", async () => {
  const layout = await load("features/workflows/layout.js");
  await mount({ nodes: [node("n1")], edges: [] });
  const element = document.querySelector('[data-id="n1"]');
  element.dispatchEvent({ type: "pointerdown", clientX: 20, clientY: 20, pointerId: 1 });
  element.dispatchEvent({ type: "pointermove", clientX: 57, clientY: 43, pointerId: 1 });
  element.dispatchEvent({ type: "pointerup", clientX: 57, clientY: 43, pointerId: 1 });

  const x = Number(leftOf("n1").replace("px", ""));
  const y = Number(topOf("n1").replace("px", ""));
  assert.equal(x % layout.GRID_SIZE, 0, `x ${x} is off the grid`);
  assert.equal(y % layout.GRID_SIZE, 0, `y ${y} is off the grid`);
  assert.notEqual(x, 20, "the drag did not move the node");
});

test("keyboard moves stay on the grid", async () => {
  const layout = await load("features/workflows/layout.js");
  await mount({ nodes: [node("n1")], edges: [] });
  const element = document.querySelector('[data-id="n1"]');
  element.dispatchEvent({ type: "keydown", key: "ArrowRight", preventDefault() {} });
  element.dispatchEvent({ type: "keydown", key: "ArrowDown", shiftKey: true, preventDefault() {} });
  const x = Number(leftOf("n1").replace("px", ""));
  const y = Number(topOf("n1").replace("px", ""));
  assert.equal(x % layout.GRID_SIZE, 0, `x ${x} is off the grid`);
  assert.equal(y % layout.GRID_SIZE, 0, `y ${y} is off the grid`);
  assert.equal(x, layout.snapToGrid(20 + layout.GRID_SIZE));
  assert.equal(y, layout.snapToGrid(20 + layout.GRID_SIZE * 4));
});

test("auto-layout orders a chain from left to right without overlaps", async () => {
  const layout = await load("features/workflows/layout.js");
  const graph = {
    nodes: [node("a", 0, 0), node("b", 320, 0), node("c", 20, 300), node("d", 400, 300), node("e", 120, 40)],
    edges: [
      { from: "a", to: "b", when: "success" },
      { from: "a", to: "c", when: "success" },
      { from: "b", to: "d", when: "success" },
      { from: "c", to: "d", when: "success" },
      { from: "d", to: "e", when: "success" },
    ],
  };
  const positions = layout.layoutGraph(graph);
  const byId = new Map(positions.map((position) => [position.id, position]));
  assert.ok(byId.get("a").x < byId.get("b").x, "a is not left of b");
  assert.ok(byId.get("b").x < byId.get("d").x, "b is not left of d");
  assert.ok(byId.get("d").x < byId.get("e").x, "d is not left of e");
  for (let left = 0; left < positions.length; left += 1) {
    for (let right = left + 1; right < positions.length; right += 1) {
      const first = positions[left];
      const second = positions[right];
      const overlap = first.x < second.x + layout.NODE_WIDTH
        && first.x + layout.NODE_WIDTH > second.x
        && first.y < second.y + layout.NODE_HEIGHT
        && first.y + layout.NODE_HEIGHT > second.y;
      assert.equal(overlap, false, `${first.id} overlaps ${second.id}`);
    }
  }
});

test("auto-layout is one undoable change that marks the editor dirty", async () => {
  const graph = {
    nodes: [node("n1", 20, 20), node("n2", 320, 20), node("n3", 20, 300)],
    edges: [
      { from: "n1", to: "n2", when: "success" },
      { from: "n2", to: "n3", when: "success" },
    ],
  };
  const { state } = await mount(graph);
  click("#wf-auto-layout");
  assert.equal(leftOf("n1"), "0px");
  assert.equal(leftOf("n2"), "264px");
  assert.equal(leftOf("n3"), "528px");
  assert.equal(document.querySelector("#wf-status").textContent, "unsaved_changes");
  assert.equal(state.leaveGuard(), true, "auto-layout does not arm the unsaved guard");

  click("#wf-undo");
  assert.equal(leftOf("n1"), "20px");
  assert.equal(leftOf("n2"), "320px");
  assert.equal(leftOf("n3"), "20px");
  assert.equal(document.querySelector("#wf-status").textContent, "all_changes_saved");

  const canvas = document.querySelector("#canvas");
  canvas.dispatchEvent({ type: "keydown", key: "t", shiftKey: true, altKey: true, preventDefault() {} });
  assert.equal(leftOf("n1"), "0px");
  assert.equal(document.querySelector("#wf-status").textContent, "unsaved_changes");
});

test("the shortcut help lists the auto-layout shortcut in both catalogs", () => {
  assert.ok(document.querySelector("#wf-auto-layout"), "the canvas has no auto-layout control");
  click("#wf-shortcuts");
  const dialog = document.querySelector("#shortcuts-dialog");
  assert.ok(dialog, "the shortcut help did not open");
  assert.match(dialog.innerHTML, /Shift\+Alt\+T/);
  assert.match(dialog.innerHTML, /shortcut_layout/);
  for (const id of ["de", "en"]) {
    const values = catalog(id);
    for (const key of ["auto_layout", "auto_layout_done", "shortcut_layout"]) {
      assert.ok(typeof values[key] === "string" && values[key].trim(), `${key} is missing from ${id}.json`);
    }
  }
});
