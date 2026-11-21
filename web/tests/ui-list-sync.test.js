"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
const tick = () => new Promise((resolve) => setImmediate(resolve));

const gitProject = {
  id: "git-1",
  name: "Git one",
  organization_name: "Org",
  source_type: "git",
  source: "https://example.com/one.git",
  last_sync_status: "",
  last_sync_at: "",
  last_sync_by: "",
};
const secondGitProject = {
  ...gitProject,
  id: "git-2",
  name: "Git two",
};
const pathProject = {
  ...gitProject,
  id: "path-1",
  name: "Path one",
  source_type: "path",
  source: "/srv/one",
};
const externalInventory = {
  id: "inv-external",
  name: "External",
  source_kind: "project",
  source: {},
  source_count: 1,
  host_count: 1,
  group_count: 1,
  sources: [{ id: "src-1", kind: "project", source: {} }],
  last_sync_status: "",
  last_sync_at: "",
  last_sync_by: "",
};
const manualInventory = {
  ...externalInventory,
  id: "inv-manual",
  name: "Manual",
  source_kind: "ini",
  sources: [{ id: "src-2", kind: "ini", source: {} }],
};

async function asRole(role) {
  const state = await load("core/state.js");
  state.setMe({ is_admin: false, display_name: role, teams: [{ id: "team-1", name: "Team", role }] });
  state.setTeamId("team-1");
}

test("sync is offered only for the right source kinds and roles", async () => {
  await asRole("operator");
  const projects = await load("features/projects.js");
  const inventories = await load("features/inventories.js");
  assert.equal(projects.canSyncProject(gitProject), true);
  assert.equal(projects.canSyncProject(pathProject), false);
  assert.equal(inventories.canSyncInventory(externalInventory), true);
  assert.equal(inventories.canSyncInventory(manualInventory), false);

  await asRole("viewer");
  assert.equal(projects.canSyncProject(gitProject), false);
  assert.equal(inventories.canSyncInventory(externalInventory), false);

  await asRole("owner");
  assert.equal(projects.canSyncProject(gitProject), true);
  assert.equal(inventories.canSyncInventory(externalInventory), true);
});

test("the project list disables only the syncing row while the request runs", async () => {
  await asRole("operator");
  const router = await load("core/router.js");
  await load("features/projects.js");
  const pending = [];
  globalThis.fetch = async (path, init) => {
    if (init && init.method === "POST") {
      return new Promise((resolve) => pending.push(() => resolve({ ok: true, status: 200, text: async () => JSON.stringify({ path: "/tmp/repo" }) })));
    }
    return { ok: true, status: 200, text: async () => JSON.stringify([gitProject, secondGitProject, pathProject]) };
  };
  location.hash = "#/projects";
  await router.matchRoute("#/projects").handler();
  assert.equal(document.querySelectorAll("[data-sync-project]").length, 2);
  assert.equal(document.querySelector('[data-sync-project="path-1"]'), null);

  const button = document.querySelector('[data-sync-project="git-1"]');
  button.dispatchEvent({ type: "click" });
  await tick();
  assert.equal(button.disabled, true, "the syncing row button is not disabled");
  assert.equal(button.getAttribute("aria-busy"), "true", "the syncing row has no busy state");
  assert.equal(document.querySelector('[data-sync-project="git-2"]').disabled, false, "the other row was disabled");
  pending.forEach((resolve) => resolve());
  await tick();
  await tick();
});

test("the inventory list disables only the syncing row and hides ini-only rows", async () => {
  await asRole("operator");
  const router = await load("core/router.js");
  await load("features/inventories.js");
  const pending = [];
  globalThis.fetch = async (path, init) => {
    if (init && init.method === "POST") {
      return new Promise((resolve) => pending.push(() => resolve({ ok: true, status: 200, text: async () => JSON.stringify({}) })));
    }
    if (path === "/api/v1/credentials") {
      return { ok: true, status: 200, text: async () => "[]" };
    }
    return { ok: true, status: 200, text: async () => JSON.stringify([externalInventory, manualInventory]) };
  };
  location.hash = "#/inventories";
  await router.matchRoute("#/inventories").handler();
  assert.equal(document.querySelectorAll("[data-sync-inventory]").length, 1);
  assert.equal(document.querySelector('[data-sync-inventory="inv-manual"]'), null);

  const button = document.querySelector('[data-sync-inventory="inv-external"]');
  button.dispatchEvent({ type: "click" });
  await tick();
  assert.equal(button.disabled, true, "the refreshing row button is not disabled");
  assert.equal(button.getAttribute("aria-busy"), "true", "the refreshing row has no busy state");
  pending.forEach((resolve) => resolve());
  await tick();
  await tick();
});

test("a viewer sees no sync buttons on either list", async () => {
  await asRole("viewer");
  const router = await load("core/router.js");
  await load("features/projects.js");
  await load("features/inventories.js");
  globalThis.fetch = async (path) => {
    if (path === "/api/v1/credentials") {
      return { ok: true, status: 200, text: async () => "[]" };
    }
    const body = path === "/api/v1/projects"
      ? [gitProject, pathProject]
      : [externalInventory, manualInventory];
    return { ok: true, status: 200, text: async () => JSON.stringify(body) };
  };
  location.hash = "#/projects";
  await router.matchRoute("#/projects").handler();
  assert.equal(document.querySelectorAll("[data-sync-project]").length, 0);
  location.hash = "#/inventories";
  await router.matchRoute("#/inventories").handler();
  assert.equal(document.querySelectorAll("[data-sync-inventory]").length, 0);
});
