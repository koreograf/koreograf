"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { stylesheet } = require("./css.js");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

async function admin() {
  const state = await load("core/state.js");
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  globalThis.fetch = async (path) => {
    const body = path.includes("/environments") ? [] : {};
    return { ok: true, status: 200, text: async () => JSON.stringify(body) };
  };
}

test("settings are one form with tab panels", async () => {
  await admin();
  const { settingsTab, settingsTabAddress } = await load("features/settings.js");
  const router = await load("core/router.js");
  location.hash = "#/settings";
  await router.matchRoute("#/settings").handler();
  const app = document.querySelector("#app");
  assert.match(app.innerHTML, /<form class="card form" id="settings-form">/);
  for (const id of ["directory", "proxy", "jobs", "assistant", "database"]) {
    assert.ok(app.innerHTML.includes(`id="settings-panel-${id}"`), `the ${id} panel is missing`);
    assert.ok(app.innerHTML.includes(`aria-labelledby="settings-tab-${id}"`), `the ${id} tab is not linked to its panel`);
  }
  assert.ok(app.innerHTML.indexOf('id="settings-panel-jobs"') < app.innerHTML.indexOf('id="preflight-check"'));
  assert.ok(app.innerHTML.indexOf('id="preflight-check"') < app.innerHTML.indexOf('id="settings-reset"'));
  assert.match(app.innerHTML, /role="tablist"/);
  assert.match(app.innerHTML, /data-settings-tab/);

  location.hash = "#/settings?tab=proxy";
  assert.equal(settingsTab(), "proxy");
  assert.equal(settingsTabAddress("proxy"), "#/settings?tab=proxy");
  assert.equal(settingsTabAddress("directory"), "#/settings");

  const tab = document.querySelector('[data-settings-tab="jobs"]');
  tab.focus();
  tab.dispatchEvent({ type: "keydown", key: "ArrowRight", preventDefault() {} });
  assert.equal(document.activeElement.dataset.settingsTab, "assistant", "ArrowRight did not move the settings tab");
  assert.equal(location.hash, "#/settings?tab=assistant", "the selected tab is not kept in the address");
  assert.equal(document.querySelector("#settings-panel-assistant").hidden, false);
  assert.equal(document.querySelector("#settings-panel-jobs").hidden, true);
});

test("settings tab labels and panel styles are present", () => {
  for (const id of ["de", "en"]) {
    const values = JSON.parse(readFileSync(join(__dirname, "..", "i18n", `${id}.json`), "utf8"));
    for (const key of ["directory", "proxy", "jobs"]) {
      assert.ok(typeof values[key] === "string" && values[key].trim(), `${key} is missing from ${id}.json`);
    }
  }
  assert.ok(stylesheet().includes(".settings-panel[hidden]"), "hidden settings panels still show");
});
