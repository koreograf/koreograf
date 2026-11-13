"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { stylesheet } = require("./css.js");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

test("a navigation shows the loading state before it waits for data", async () => {
  const app = await load("core/app.js");
  const state = await load("core/state.js");
  const router = await load("core/router.js");
  state.setMe({ is_admin: true, teams: [{ id: "t1", role: "owner" }] });
  state.setTeamId("t1");
  let release = null;
  router.registerRoute("#/slow", () => new Promise((resolve) => { release = resolve; }));
  location.hash = "#/slow";
  const navigation = app.route();
  assert.match(state.app.innerHTML, /loading-skeleton/, "the first navigation showed no skeleton");
  release();
  await navigation;
  assert.match(state.app.innerHTML, /class="main"/, "the page shell is missing");

  location.hash = "#/slow?again=1";
  const second = app.route();
  const main = document.querySelector(".main");
  assert.equal(main.getAttribute("aria-busy"), "true", "the loading state is not announced");
  assert.match(main.innerHTML, /loading-skeleton/, "the loading state did not replace the previous page");
  release();
  await second;
});

test("the loading skeleton has a layout", () => {
  const css = stylesheet();
  assert.match(css, /\.loading-skeleton\s*\{/);
  assert.match(css, /\.skeleton-card\s*\{/);
  assert.match(css, /\.skeleton-line\s*\{[^}]*background:/);
});
