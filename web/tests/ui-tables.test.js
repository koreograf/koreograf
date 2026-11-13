"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { mediaBlock, rule, stylesheet } = require("./css.js");

test("narrow tables use card rows", () => {
  const css = stylesheet();
  const block = mediaBlock(css, "@media (max-width: 640px)");
  assert.ok(block.includes(".table-wrap:not(.schedule-table):not(.inventory-hosts-table) .table"));
  assert.ok(block.includes("display: block;"));
  assert.ok(block.includes(".table-wrap:not(.schedule-table):not(.inventory-hosts-table) .table thead { display: none; }"));
  assert.ok(block.includes(".table-wrap:not(.schedule-table):not(.inventory-hosts-table) .table .narrow-hide { display: none; }"));
  assert.ok(block.includes("content: attr(data-label)"));
  assert.ok(css.includes(".schedule-table .table { min-width: 52rem; }"));
  assert.ok(css.includes(".inventory-hosts-table .table { min-width: 44rem; }"));
  assert.ok(css.includes(".table { width: 100%; border-collapse: collapse; }"));
});

test("table text does not break mid word", () => {
  const css = stylesheet();
  const cells = css.split("\n").find((line) => line.startsWith(".table th, .table td {"));
  assert.ok(cells, "the table cell rule is missing");
  assert.ok(cells.includes("overflow-wrap: break-word;"));
  assert.ok(!cells.includes("overflow-wrap: anywhere;"));
  assert.ok(css.includes(".table .pill, .table time { white-space: nowrap; }"));
});

test("the shared table wraps cells with a data label", async () => {
  require("./test-dom.js").install();
  const { join } = require("node:path");
  const { pathToFileURL } = require("node:url");
  const { table } = await import(pathToFileURL(join(__dirname, "..", "js", "core", "dom.js")).href);
  const html = table(["Name", "Tool"], ['<tr><td data-label="Name">one</td></tr>']);
  assert.match(html, /class="table-wrap"/);
  assert.match(html, /class="table"/);
  assert.match(html, /data-label="Name"/);
  const empty = table(["Name"], []);
  assert.match(empty, /nothing_here/);
  assert.match(empty, /colspan="1"/);
});

test("list tables hide low priority cells on narrow windows", async () => {
  require("./test-dom.js").install();
  const { join } = require("node:path");
  const { pathToFileURL } = require("node:url");
  const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
  const state = await load("core/state.js");
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  const jobs = {
    items: [{ id: "j1", name: "Job", tool: "ansible", status: "successful", trigger: "manual", created_at: "2026-01-01T00:00:00Z" }],
    total: 1,
  };
  const templates = [{ id: "t1", name: "Template", tool: "ansible", entrypoint: "site.yml", spec: {} }];
  globalThis.fetch = async (path) => {
    let body = [];
    if (path.startsWith("/api/v1/jobs")) body = jobs;
    else if (path.startsWith("/api/v1/templates")) body = templates;
    return { ok: true, status: 200, text: async () => JSON.stringify(body) };
  };
  const router = await load("core/router.js");
  await load("features/jobs.js");
  await load("features/templates.js");
  location.hash = "#/jobs";
  await router.matchRoute("#/jobs").handler();
  const table = document.querySelector("#job-table");
  assert.match(table.innerHTML, /class="tool narrow-hide"/);
  assert.match(table.innerHTML, /data-label="created"/);
  assert.ok((table.innerHTML.match(/narrow-hide/g) || []).length >= 3);
  location.hash = "#/templates";
  await router.matchRoute("#/templates").handler();
  assert.match(document.querySelector("#template-table").innerHTML, /class="mono narrow-hide"/);
});
