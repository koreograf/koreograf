"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { stylesheet } = require("./css.js");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
const tick = () => new Promise((resolve) => setImmediate(resolve));

globalThis.Element.prototype.getBoundingClientRect = () => ({ left: 0, top: 0, width: 1440, height: 900, right: 1440, bottom: 900 });

function grid(columns, rows) {
  const nodes = [];
  for (let row = 0; row < rows; row += 1) {
    for (let column = 0; column < columns; column += 1) {
      nodes.push({
        id: `n${row}-${column}`,
        template_id: "t1",
        name: `Node ${row}-${column}`,
        x: column * 280 + 40,
        y: row * 180 + 40,
        convergence: "any",
      });
    }
  }
  return nodes;
}

async function mount(nodes) {
  const state = await load("core/state.js");
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "[]" });
  const editor = await load("features/workflows/editor.js");
  const workflow = { id: "w1", name: "Deploy", description: "", graph: { nodes, edges: [] } };
  const mounted = editor.mountEditor(workflow, [{ id: "t1", name: "Shell", tool: "shell", spec: {} }], false, {}, { inventories: [], credentials: [] }, {});
  return { editor, workflow, mounted };
}

function click(selector) {
  document.querySelector(selector).dispatchEvent({ type: "click" });
}

test("zoom keeps the point under the cursor fixed and clamps the scale", async () => {
  const viewport = await load("features/workflows/viewport.js");
  const view = { x: 120, y: 60, scale: 1 };
  const point = { x: 300, y: 220 };
  const before = viewport.toGraphPoint(view, point);
  const zoomed = viewport.zoomAt(view, 2, point);
  const after = viewport.toGraphPoint(zoomed, point);
  assert.ok(Math.abs(before.x - after.x) < 1e-9, "zoom moved the graph point under the cursor");
  assert.ok(Math.abs(before.y - after.y) < 1e-9, "zoom moved the graph point under the cursor");
  assert.equal(zoomed.scale, 2);
  assert.equal(viewport.zoomAt(view, 100, point).scale, viewport.MAX_SCALE);
  assert.equal(viewport.zoomAt(view, 0.0001, point).scale, viewport.MIN_SCALE);
  assert.deepEqual(viewport.panBy({ x: 1, y: 2, scale: 1.5 }, 10, -5), { x: 11, y: -3, scale: 1.5 });
});

test("fit places all 30 nodes inside a 1440 by 900 viewport", async () => {
  const viewport = await load("features/workflows/viewport.js");
  const nodes = grid(6, 5);
  const size = { width: 1440, height: 900 };
  const view = viewport.fitView(nodes, size);
  assert.ok(view.scale > viewport.MIN_SCALE && view.scale < 1, "the big graph does not fit at a smaller scale");
  for (const node of nodes) {
    assert.ok(viewport.nodeInsideViewport(node, view, size), `${node.id} is outside the fitted viewport`);
  }
  assert.deepEqual(viewport.fitView([], size), { x: viewport.FIT_PADDING, y: viewport.FIT_PADDING, scale: 1 });
});

test("auto pan triggers at the edge and stays quiet inside", async () => {
  const viewport = await load("features/workflows/viewport.js");
  const size = { width: 800, height: 600 };
  const view = { x: 0, y: 0, scale: 1 };
  const right = viewport.autoPan(view, { x: 790, y: 300 }, size);
  assert.equal(right.dx, -24);
  assert.equal(right.dy, 0);
  assert.equal(right.view.x, -24);
  const topLeft = viewport.autoPan(view, { x: 10, y: 10 }, size);
  assert.equal(topLeft.dx, 24);
  assert.equal(topLeft.dy, 24);
  const inside = viewport.autoPan(view, { x: 400, y: 300 }, size);
  assert.equal(inside.dx, 0);
  assert.equal(inside.dy, 0);
  assert.equal(inside.view.x, view.x);
});

test("the camera transform and screen conversion round-trip", async () => {
  const viewport = await load("features/workflows/viewport.js");
  const view = { x: 120, y: 60, scale: 0.75 };
  const graph = { x: 200, y: 140 };
  const screen = viewport.toScreenPoint(view, graph);
  assert.deepEqual(viewport.toGraphPoint(view, screen), graph);
  assert.equal(viewport.cameraTransform(view), "translate(120px, 60px) scale(0.75)");
});

test("the minimap maps nodes and the visible box", async () => {
  const viewport = await load("features/workflows/viewport.js");
  const nodes = grid(6, 5);
  const size = { width: 1440, height: 900 };
  const layout = viewport.minimapLayout(nodes, viewport.fitView(nodes, size), size);
  assert.equal(layout.nodes.length, 30);
  for (const rect of layout.nodes) {
    assert.ok(rect.x >= 0 && rect.y >= 0 && rect.x + rect.width <= layout.width && rect.y + rect.height <= layout.height);
  }
  assert.ok(layout.viewport.width > 0 && layout.viewport.height > 0);
});

test("the editor fits on open and the controls zoom, fit and reset", async () => {
  const { mounted } = await mount(grid(4, 4));
  const size = mounted.viewport.size();
  const fitted = mounted.viewport.view;
  assert.ok(fitted.scale > 0 && fitted.scale <= 1);
  for (const node of grid(4, 4)) {
    assert.ok(viewportInside(mounted, node), `${node.id} is outside the fitted canvas`);
  }
  assert.equal(document.querySelector("#canvas").getAttribute("data-zoom"), String(Math.round(fitted.scale * 100)));
  assert.ok(document.querySelector("[data-canvas-zoom-in]"), "the zoom in control is missing");
  assert.ok(document.querySelector("[data-canvas-zoom-out]"), "the zoom out control is missing");
  assert.ok(document.querySelector("[data-canvas-fit]"), "the fit control is missing");
  assert.ok(document.querySelector("[data-canvas-reset]"), "the reset control is missing");

  click("[data-canvas-zoom-in]");
  assert.ok(mounted.viewport.view.scale > fitted.scale, "the zoom in control did not zoom");
  click("[data-canvas-reset]");
  assert.ok(Math.abs(mounted.viewport.view.scale - 1) < 1e-9, "reset did not return to 100 %");
  click("[data-canvas-fit]");
  assert.ok(mounted.viewport.view.scale < 1);

  const canvas = document.querySelector("#canvas");
  const zoomed = mounted.viewport.view.scale;
  canvas.dispatchEvent({ type: "wheel", ctrlKey: true, deltaY: -120, clientX: size.width / 2, clientY: size.height / 2, preventDefault() {} });
  assert.ok(mounted.viewport.view.scale > zoomed, "Ctrl+wheel did not zoom");
  canvas.dispatchEvent({ type: "keydown", key: "+", preventDefault() {} });
  assert.ok(mounted.viewport.view.scale > zoomed, "the keyboard did not zoom");
});

function viewportInside(mounted, node) {
  const size = mounted.viewport.size();
  const view = mounted.viewport.view;
  const width = 200 * view.scale;
  const height = 96 * view.scale;
  const screen = mounted.viewport.graphToScreen(node);
  return screen.x >= -0.001 && screen.y >= -0.001 && screen.x + width <= size.width + 0.001 && screen.y + height <= size.height + 0.001;
}

test("the editor pans the empty canvas and auto pans a node at the edge", async () => {
  const { mounted } = await mount(grid(3, 3));
  const canvas = document.querySelector("#canvas");
  const before = mounted.viewport.view.x;
  const beforeY = mounted.viewport.view.y;
  canvas.dispatchEvent({ type: "pointerdown", pointerId: 1, clientX: 700, clientY: 400 });
  canvas.dispatchEvent({ type: "pointermove", pointerId: 1, clientX: 760, clientY: 430 });
  canvas.dispatchEvent({ type: "pointerup", pointerId: 1, clientX: 760, clientY: 430 });
  assert.equal(mounted.viewport.view.x, before + 60);
  assert.equal(mounted.viewport.view.y, beforeY + 30);

  const node = document.querySelector('[data-id="n0-0"]');
  const nodeBefore = mounted.viewport.view.x;
  node.dispatchEvent({ type: "pointerdown", pointerId: 2, clientX: 300, clientY: 300 });
  node.dispatchEvent({ type: "pointermove", pointerId: 2, clientX: 1430, clientY: 450 });
  assert.ok(mounted.viewport.view.x < nodeBefore, "dragging a node to the edge did not auto pan");
  node.dispatchEvent({ type: "pointerup", pointerId: 2, clientX: 1430, clientY: 450 });
});

test("the minimap appears for more than 12 nodes", async () => {
  await mount(grid(4, 4));
  const minimap = document.querySelector("#minimap");
  assert.equal(minimap.hidden, false);
  assert.equal(document.querySelectorAll("#minimap .minimap-node").length, 16);

  await mount(grid(3, 3));
  assert.equal(document.querySelector("#minimap").hidden, true);
  assert.equal(document.querySelectorAll("#minimap .minimap-node").length, 0);
});

test("the canvas styles expose the viewport controls and full-height layout", () => {
  const css = stylesheet();
  assert.match(css, /\.canvas\s*\{[^}]*overflow:\s*hidden/);
  assert.match(css, /\.canvas\s*\{[^}]*min-height:\s*560px/);
  assert.ok(css.includes(".canvas-controls"));
  assert.ok(css.includes(".minimap"));
  assert.match(css, /@media \(min-width: 1920px\)[\s\S]*calc\(100vh/);
});
