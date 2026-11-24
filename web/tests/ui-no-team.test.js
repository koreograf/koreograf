"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

const viewer = { is_admin: false, display_name: "Carol", teams: [] };
const admin = { is_admin: true, display_name: "Admin", teams: [] };

async function setup(me, teamId = "") {
  const app = await load("core/app.js");
  const state = await load("core/state.js");
  state.setMe(me);
  state.setTeamId(teamId);
  globalThis.fetch = async () => ({ ok: false, status: 503, text: async () => "{}" });
  return { app, state };
}

test("a viewer without a team sees one empty state and no Team link", async () => {
  const { app, state } = await setup(viewer);
  location.hash = "#/";
  assert.equal(app.needTeam(), false);
  const html = state.app.innerHTML;
  assert.match(html, /this_account_has_no_team_an_administrator_creates/);
  assert.doesNotMatch(html, /open_team/);
  assert.doesNotMatch(html, /href="#\/team"/);
  assert.equal(state.app.querySelectorAll(".card").length, 1, "the no-team state stacks cards");
  assert.match(state.app.innerHTML, /<h1[^>]*>overview<\/h1>/);
});

test("the shell hides the team-scoped groups without a team", async () => {
  const { app, state } = await setup(viewer);
  location.hash = "#/";
  app.needTeam();
  assert.equal(state.app.querySelector(".nav"), null, "a viewer without a team keeps a nav tree");
  assert.match(state.app.innerHTML, /account-link/);
  assert.match(state.app.innerHTML, /data-lang-select/);
  assert.match(state.app.innerHTML, /data-theme/);
  assert.match(state.app.innerHTML, /id="logout"/);
  assert.deepEqual(app.navGroups(), []);
});

test("the no-team heading follows the route that asked for a team", async () => {
  const { app, state } = await setup(viewer);
  location.hash = "#/jobs";
  app.needTeam();
  assert.match(state.app.innerHTML, /<h1[^>]*>jobs<\/h1>/);
  location.hash = "#/templates";
  app.needTeam();
  assert.match(state.app.innerHTML, /<h1[^>]*>templates<\/h1>/);
});

test("an administrator without a team keeps Team, Organizations and Users", async () => {
  const { app, state } = await setup(admin);
  location.hash = "#/";
  app.needTeam();
  const html = state.app.innerHTML;
  assert.match(html, /href="#\/team"/);
  assert.match(html, /href="#\/organizations"/);
  assert.match(html, /href="#\/users"/);
  assert.match(html, /open_team/);
  assert.doesNotMatch(html, /href="#\/jobs"/, "the administrator keeps the Operations group");
  assert.doesNotMatch(html, /href="#\/service-accounts"/, "service accounts stay team-scoped");
  assert.match(state.app.innerHTML, /<h1[^>]*>overview<\/h1>/);
});

test("a team keeps the full navigation for a viewer", async () => {
  const { app } = await setup({ ...viewer, teams: [{ id: "t1", name: "Team", role: "viewer" }] }, "t1");
  location.hash = "#/";
  assert.equal(app.needTeam(), true);
  const groups = app.navGroups();
  assert.equal(groups.length, 4);
  assert.ok(groups.some(([, links]) => links.some(([href]) => href === "#/jobs")));
});
