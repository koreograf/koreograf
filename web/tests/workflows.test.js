"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const loadIndex = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "workflows", "index.js")).href);
const loadEditor = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "workflows", "editor.js")).href);
const loadMenus = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "workflows", "menus.js")).href);

test("the workflow list filters by name and description", async () => {
  const { filterWorkflows } = await loadIndex();
  const items = [
    { name: "Deploy web", description: "frontend" },
    { name: "Network", description: "vpc" },
    { name: "Backup", description: "nightly" },
  ];
  assert.deepEqual(filterWorkflows(items, "").map((item) => item.name), ["Deploy web", "Network", "Backup"]);
  assert.deepEqual(filterWorkflows(items, "  ").map((item) => item.name), ["Deploy web", "Network", "Backup"]);
  assert.deepEqual(filterWorkflows(items, "frontend").map((item) => item.name), ["Deploy web"]);
  assert.deepEqual(filterWorkflows(items, "VPC").map((item) => item.name), ["Network"]);
  assert.deepEqual(filterWorkflows(items, "network").map((item) => item.name), ["Network"]);
  assert.deepEqual(filterWorkflows(items, "missing"), []);
});

test("a workflow needs an apply node for the OpenTofu confirmation", async () => {
  const { workflowTofuApply } = await loadEditor();
  const templates = [
    { id: "plan", tool: "opentofu", spec: { action: "plan" } },
    { id: "apply", tool: "opentofu", spec: { action: "apply" } },
    { id: "ansible", tool: "ansible", spec: {} },
  ];
  const graph = (...nodes) => ({ nodes, edges: [] });
  assert.equal(workflowTofuApply(graph({ template_id: "plan" }), templates), false);
  assert.equal(workflowTofuApply(graph({ template_id: "ansible" }), templates), false);
  assert.equal(workflowTofuApply(graph({ template_id: "apply" }), templates), true);
  assert.equal(workflowTofuApply(graph({ template_id: "plan", overrides: { spec: { action: "apply" } } }), templates), true);
  assert.equal(workflowTofuApply(graph({ template_id: "apply", overrides: { spec: { action: "plan" } } }), templates), false);
  assert.equal(workflowTofuApply({ nodes: [{ template_id: "missing" }] }, templates), false);
  assert.equal(workflowTofuApply(null, templates), false);
});

test("edges leave from the right side at the port of their kind", async () => {
  const { edgePoint } = await loadEditor();
  const node = { x: 10, y: 20 };
  assert.deepEqual(edgePoint(node, "success"), { x: 210, y: 60 });
  assert.deepEqual(edgePoint(node, "failure"), { x: 210, y: 96 });
  assert.deepEqual(edgePoint(node, "always"), { x: 210, y: 78 });
  assert.deepEqual(edgePoint(node, "unknown"), { x: 210, y: 60 });
});

test("edge kinds use the error label for a failure edge", async () => {
  const { edgeWhenLabel } = await loadMenus();
  assert.equal(edgeWhenLabel("success"), "success");
  assert.equal(edgeWhenLabel("failure"), "error.generic");
  assert.equal(edgeWhenLabel("always"), "always");
});

test("the workflow tab comes from the query string", async () => {
  const { workflowTab } = await loadEditor();
  assert.equal(workflowTab("#/workflows/abc"), "editor");
  assert.equal(workflowTab("#/workflows/abc?tab=editor"), "editor");
  assert.equal(workflowTab("#/workflows/abc?tab=schedules"), "schedules");
  assert.equal(workflowTab("#/workflows/abc?tab=danger"), "danger");
  assert.equal(workflowTab("#/workflows/abc?tab=unknown"), "editor");
  assert.equal(workflowTab(""), "editor");
});

test("the workflow address keeps the editor tab out of the query", async () => {
  const { workflowAddress } = await loadEditor();
  assert.equal(workflowAddress("editor", "abc"), "#/workflows/abc");
  assert.equal(workflowAddress("schedules", "abc"), "#/workflows/abc?tab=schedules");
  assert.equal(workflowAddress("danger", "a b/c"), "#/workflows/a%20b%2Fc?tab=danger");
});
