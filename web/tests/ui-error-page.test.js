"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
const tick = () => new Promise((resolve) => setImmediate(resolve));

async function setup() {
  const app = await load("core/app.js");
  const state = await load("core/state.js");
  const router = await load("core/router.js");
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "{}" });
  return { app, state, router };
}

test("a route failure renders an error page with retry and an overview link", async () => {
  const { app, state, router } = await setup();
  app.renderRouteError(Object.assign(new Error("not_found"), {}));
  assert.match(state.app.innerHTML, /route_error_title/);
  assert.match(state.app.innerHTML, /not_found/);
  assert.match(state.app.innerHTML, /id="route-retry"/);
  assert.match(state.app.innerHTML, /href="#\/"/);
  assert.match(state.app.innerHTML, /back_to_overview/);

  let routed = 0;
  router.registerRoute("#/retry-target", () => { routed += 1; });
  location.hash = "#/retry-target";
  document.querySelector("#route-retry").dispatchEvent({ type: "click" });
  await tick();
  assert.equal(routed, 1, "the retry action does not rerun the route");
});

test("the error page escapes the message", async () => {
  const { app, state } = await setup();
  app.renderRouteError(new Error("<b>unsafe</b>"));
  assert.ok(!state.app.innerHTML.includes("<b>unsafe</b>"), "the error page leaks raw HTML");
  assert.match(state.app.innerHTML, /&lt;b&gt;unsafe&lt;\/b&gt;/);
});

test("a failing route handler shows the error page", async () => {
  const { app, state, router } = await setup();
  router.registerRoute("#/boom", async () => { throw new Error("not_found"); });
  location.hash = "#/boom";
  await app.route();
  assert.match(state.app.innerHTML, /route_error_title/);
  assert.match(state.app.innerHTML, /not_found/);
});

test("an overlay error keeps the banner inside the open dialog", async () => {
  const { app, state } = await setup();
  const main = document.createElement("main");
  main.className = "main";
  document.body.appendChild(main);
  const overlay = document.createElement("div");
  overlay.className = "overlay";
  overlay.innerHTML = '<div class="overlay-card"><div class="dialog-body"></div></div>';
  document.body.appendChild(overlay);
  app.handle(new Error("not_found"));
  const body = overlay.querySelector(".dialog-body");
  assert.match(body.innerHTML, /banner bad/);
  assert.match(body.innerHTML, /not_found/);
  assert.equal(main.querySelector(".banner.bad"), null, "the error banner went to the page instead of the overlay");
  state.setMe(null);
});

test("the route error copy is in both catalogs", () => {
  const { readFileSync } = require("node:fs");
  for (const id of ["de", "en"]) {
    const values = JSON.parse(readFileSync(join(__dirname, "..", "i18n", `${id}.json`), "utf8"));
    for (const key of ["back_to_overview", "retry", "route_error_explanation", "route_error_title"]) {
      assert.ok(typeof values[key] === "string" && values[key].length > 0, `${id}.json misses ${key}`);
    }
    const keys = Object.keys(values);
    assert.deepEqual(keys, [...keys].sort(), `${id}.json is not alphabetical`);
  }
});
