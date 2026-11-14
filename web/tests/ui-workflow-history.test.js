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
  for (let index = 0; index < 5; index += 1) await tick();
};

function node(id, name, x = 20, y = 20) {
  return { id, template_id: "t1", name, x, y, convergence: "any" };
}

async function mount(graph, { role = "owner" } = {}) {
  const state = await load("core/state.js");
  state.setMe({ is_admin: role === "owner", display_name: "Admin", teams: [{ id: "t1", name: "Team", role }] });
  state.setTeamId("t1");
  const calls = [];
  globalThis.fetch = async (path, init = {}) => {
    calls.push({ method: init.method || "GET", path, body: init.body });
    return { ok: true, status: 200, text: async () => "{}" };
  };
  const editor = await load("features/workflows/editor.js");
  const workflow = {
    id: "w1",
    name: "Deploy",
    description: "",
    graph: graph || {
      nodes: [node("n1", "One"), node("n2", "Two", 320)],
      edges: [{ from: "n1", to: "n2", when: "success" }],
    },
  };
  editor.mountEditor(workflow, [{ id: "t1", name: "Shell", tool: "shell", spec: {} }], false, {}, { inventories: [], credentials: [] }, {});
  return { state, workflow, calls };
}

function keydown(target, key, options = {}) {
  target.dispatchEvent({ type: "keydown", key, preventDefault() {}, ...options });
}

function nodeElement(id) {
  return document.querySelector(`[data-id="${id}"]`);
}

function edgeLines() {
  return document.querySelectorAll("#edges path.edge-line");
}

function leftOf(id) {
  const element = nodeElement(id);
  const attribute = /left:([^;]+)/.exec(element.getAttribute("style") || "");
  return element.style.left || (attribute ? attribute[1] : undefined);
}

function nameOf(id) {
  const strong = nodeElement(id).querySelector("strong");
  return strong.textContent || strong.innerHTML;
}

function undo() {
  document.querySelector("#wf-undo").dispatchEvent({ type: "click" });
}

function redo() {
  document.querySelector("#wf-redo").dispatchEvent({ type: "click" });
}

test("moving a node is undoable and redoable from the toolbar and the keyboard", async () => {
  await mount();
  assert.equal(document.querySelector("#wf-undo").disabled, true, "Undo starts enabled");
  assert.equal(document.querySelector("#wf-redo").disabled, true, "Redo starts enabled");

  keydown(nodeElement("n1"), "ArrowRight");
  assert.equal(leftOf("n1"), "30px");
  assert.equal(document.querySelector("#wf-undo").disabled, false, "moving a node did not enable Undo");
  assert.equal(document.querySelector("#wf-status").textContent, "unsaved_changes");

  undo();
  assert.equal(leftOf("n1"), "20px", "Undo did not move the node back");
  assert.equal(document.querySelector("#wf-status").textContent, "all_changes_saved", "Undo to the saved snapshot stays dirty");
  assert.equal(document.querySelector("#wf-undo").disabled, true);
  assert.equal(document.querySelector("#wf-redo").disabled, false, "Undo did not enable Redo");

  redo();
  assert.equal(leftOf("n1"), "30px", "Redo did not move the node again");
  assert.equal(document.querySelector("#wf-status").textContent, "unsaved_changes");

  const canvas = document.querySelector("#canvas");
  keydown(canvas, "z", { ctrlKey: true });
  assert.equal(leftOf("n1"), "20px", "Ctrl+Z did not undo");
  keydown(canvas, "Z", { ctrlKey: true, shiftKey: true });
  assert.equal(leftOf("n1"), "30px", "Ctrl+Shift+Z did not redo");
  keydown(canvas, "z", { ctrlKey: true });
  keydown(canvas, "y", { ctrlKey: true });
  assert.equal(leftOf("n1"), "30px", "Ctrl+Y did not redo");
});

test("adding a connected node is one history entry that removes the node and its edge", async () => {
  await mount({ nodes: [node("n1", "One")], edges: [] });
  nodeElement("n1").querySelector('[data-add-port="success"]').dispatchEvent({ type: "click" });
  const dialog = document.querySelector("#add-node-dialog");
  assert.ok(dialog, "the handle did not open the node editor");
  dialog.querySelector("#node-name").value = "Two";
  dialog.querySelector(".dialog-form").dispatchEvent({ type: "submit", preventDefault() {} });
  await tick();

  assert.equal(document.querySelectorAll("#nodes .wf-node").length, 2);
  assert.equal(edgeLines().length, 1);
  undo();
  assert.equal(document.querySelectorAll("#nodes .wf-node").length, 1, "Undo kept the added node");
  assert.equal(edgeLines().length, 0, "Undo kept the added edge");
  redo();
  assert.equal(document.querySelectorAll("#nodes .wf-node").length, 2, "Redo did not restore the node");
  assert.equal(edgeLines().length, 1, "Redo did not restore the edge");
});

test("connecting two nodes is undoable and redoable", async () => {
  await mount({ nodes: [node("n1", "One"), node("n2", "Two", 320)], edges: [] });
  nodeElement("n1").querySelector("[data-node-menu]").dispatchEvent({ type: "click" });
  document.querySelector("[data-connect-node]").dispatchEvent({ type: "click" });
  nodeElement("n2").dispatchEvent({ type: "pointerdown", preventDefault() {}, stopPropagation() {}, clientX: 320, clientY: 40, pointerId: 1 });
  const dialog = document.querySelector("#edge-type-dialog");
  assert.ok(dialog, "the connection did not ask for the edge type");
  dialog.querySelector('[data-edge-when="success"]').dispatchEvent({ type: "click" });
  await tick();
  await tick();
  assert.equal(edgeLines().length, 1);

  undo();
  assert.equal(edgeLines().length, 0, "Undo kept the connection");
  redo();
  assert.equal(edgeLines().length, 1, "Redo did not restore the connection");
});

test("Delete and Backspace remove a focused node without a dialog and undo restores its edges", async () => {
  for (const key of ["Delete", "Backspace"]) {
    await mount({
      nodes: [node("n1", "One"), node("n2", "Two", 320)],
      edges: [
        { from: "n1", to: "n2", when: "success" },
        { from: "n1", to: "n2", when: "failure" },
      ],
    });
    keydown(nodeElement("n1"), key);
    assert.equal(document.querySelector("#confirm-dialog"), null, `${key} opened a confirmation`);
    assert.equal(nodeElement("n1"), null, `${key} did not remove the node`);
    assert.equal(edgeLines().length, 0, `${key} kept the incident edges`);
    const toast = document.querySelector("#wf-toast");
    assert.equal(toast.hidden, false, `${key} did not show the undo toast`);
    assert.match(toast.innerHTML, /node_deleted/);
    assert.match(toast.innerHTML, /undo/);

    if (key === "Delete") {
      keydown(document.querySelector("#canvas"), "z", { ctrlKey: true });
      assert.ok(nodeElement("n1"), "Ctrl+Z did not restore the deleted node");
      assert.equal(document.querySelectorAll("#edges path.success").length, 1, "Ctrl+Z did not restore the success edge");
      assert.equal(document.querySelectorAll("#edges path.failure").length, 1, "Ctrl+Z did not restore the failure edge");
    }
  }
});

test("deleting a focused edge shows undo and Ctrl+Z restores it", async () => {
  await mount();
  const path = document.querySelector('#edges path[data-edge="0"]');
  document.querySelector("#edges").dispatchEvent({ type: "keydown", key: "Delete", target: path, preventDefault() {} });
  assert.equal(edgeLines().length, 0, "Delete did not remove the edge");
  const toast = document.querySelector("#wf-toast");
  assert.equal(toast.hidden, false, "Delete did not show the undo toast");
  assert.match(toast.innerHTML, /edge_deleted/);
  assert.equal(document.activeElement, document.querySelector("#canvas"));

  keydown(document.querySelector("#canvas"), "z", { ctrlKey: true });
  assert.equal(edgeLines().length, 1, "Ctrl+Z did not restore the edge");
});

test("editing a node name is undoable and redoable", async () => {
  await mount();
  nodeElement("n1").dispatchEvent({ type: "pointerdown", clientX: 20, clientY: 20, pointerId: 1 });
  const name = document.querySelector("#node-name");
  name.value = "Renamed";
  name.dispatchEvent({ type: "input", target: name });
  assert.equal(nameOf("n1"), "Renamed");

  keydown(document.querySelector("#canvas"), "z", { ctrlKey: true });
  assert.equal(nameOf("n1"), "One", "Undo kept the node name");
  keydown(document.querySelector("#canvas"), "Z", { ctrlKey: true, shiftKey: true });
  assert.equal(nameOf("n1"), "Renamed", "Redo did not restore the node name");
});

test("editing an edge type is undoable and redoable", async () => {
  await mount();
  document.querySelector("#edges").dispatchEvent({ type: "click", target: document.querySelector("#edges path.edge-line") });
  const when = document.querySelector("#edge-when");
  when.value = "failure";
  when.dispatchEvent({ type: "change", target: when });
  assert.equal(document.querySelectorAll("#edges path.failure").length, 1);

  keydown(document.querySelector("#canvas"), "z", { ctrlKey: true });
  assert.equal(document.querySelectorAll("#edges path.success").length, 1, "Undo kept the failure edge");
  keydown(document.querySelector("#canvas"), "Z", { ctrlKey: true, shiftKey: true });
  assert.equal(document.querySelectorAll("#edges path.failure").length, 1, "Redo did not restore the failure edge");
});

test("a node drag is one history entry", async () => {
  await mount({ nodes: [node("n1", "One")], edges: [] });
  const element = nodeElement("n1");
  element.dispatchEvent({ type: "pointerdown", clientX: 20, clientY: 20, pointerId: 1 });
  element.dispatchEvent({ type: "pointermove", clientX: 60, clientY: 60, pointerId: 1 });
  element.dispatchEvent({ type: "pointermove", clientX: 100, clientY: 100, pointerId: 1 });
  element.dispatchEvent({ type: "pointerup", clientX: 100, clientY: 100, pointerId: 1 });
  assert.notEqual(leftOf("n1"), "20px", "the drag did not move the node");

  undo();
  assert.equal(leftOf("n1"), "20px", "Undo did not restore the drag start");
  assert.equal(document.querySelector("#wf-undo").disabled, true, "the drag created more than one history entry");
});

test("undoing back to the saved snapshot clears the unsaved guard", async () => {
  const { state, calls } = await mount();
  keydown(nodeElement("n1"), "ArrowRight");
  document.querySelector("#wf-save").dispatchEvent({ type: "click" });
  await settle();
  assert.equal(calls.filter((call) => call.method === "PUT").length, 1, "Save did not persist the move");
  assert.equal(document.querySelector("#wf-status").textContent, "all_changes_saved");
  assert.equal(state.leaveGuard, null);

  keydown(document.querySelector("#canvas"), "z", { ctrlKey: true });
  assert.equal(document.querySelector("#wf-status").textContent, "unsaved_changes", "Undo before the saved snapshot stayed clean");
  assert.equal(state.leaveGuard(), true, "Undo did not restore the leave guard");

  keydown(document.querySelector("#canvas"), "Z", { ctrlKey: true, shiftKey: true });
  assert.equal(document.querySelector("#wf-status").textContent, "all_changes_saved", "Redo to the saved snapshot stayed dirty");
  assert.equal(state.leaveGuard, null, "Redo to the saved snapshot kept the leave guard");
});

test("the toolbar and the question mark open the shortcut help", async () => {
  await mount();
  const help = document.querySelector("#wf-shortcuts");
  assert.ok(help, "the editor has no shortcut help control");
  assert.equal(help.tagName, "BUTTON");
  help.focus();
  assert.equal(document.activeElement, help, "the shortcut help control cannot take focus");

  help.dispatchEvent({ type: "click" });
  let dialog = document.querySelector("#shortcuts-dialog");
  assert.ok(dialog, "the shortcut help did not open");
  for (const key of ["shortcut_undo", "shortcut_redo", "shortcut_delete", "shortcut_connect", "shortcut_save", "shortcut_canvas", "shortcut_help"]) {
    assert.match(dialog.innerHTML, new RegExp(key), `the shortcut help has no ${key}`);
  }
  dialog.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  await tick();
  assert.equal(document.querySelector("#shortcuts-dialog"), null, "Escape left the shortcut help open");

  keydown(document.querySelector("#canvas"), "?");
  dialog = document.querySelector("#shortcuts-dialog");
  assert.ok(dialog, "? did not open the shortcut help");
  dialog.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  await tick();
});

test("workflow history copy is in both catalogs", () => {
  for (const id of ["de", "en"]) {
    const values = catalog(id);
    for (const key of ["edge_deleted", "history", "keyboard_shortcuts", "node_deleted", "redo", "shortcut_canvas", "shortcut_connect", "shortcut_delete", "shortcut_help", "shortcut_redo", "shortcut_save", "shortcut_undo", "undo"]) {
      assert.ok(typeof values[key] === "string" && values[key].trim(), `${key} is missing from ${id}.json`);
    }
  }
});
