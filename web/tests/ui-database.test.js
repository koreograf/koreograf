"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
const read = (path) => readFileSync(join(__dirname, "..", "..", path), "utf8");

test("the settings database wizard renders every step", async () => {
  const { databasePanelHtml, databaseChecksHtml, databaseProgressHtml, databaseSwitchHtml } = await load("features/database.js");
  const panel = databasePanelHtml(false);
  assert.match(panel, /id="settings-panel-database"/);
  assert.match(panel, /step_1_target/);
  assert.match(panel, /step_2_checks/);
  assert.match(panel, /step_3_migration/);
  const checks = databaseChecksHtml({ all_passed: false, checks: [{ id: "reachable", ok: false, key: "database_check_unreachable" }] });
  assert.match(checks, /check_failed/);
  assert.match(checks, /database_check_unreachable/);
  const progress = databaseProgressHtml({ done: true, rows: 12, tables: [{ table: "users", rows: 12, done: true }] });
  assert.match(progress, /database_migration_done/);
  assert.match(progress, /users/);
  const switching = databaseSwitchHtml({ done: true, url: "postgres://koreograf@db/koreograf" });
  assert.match(switching, /step_4_switch/);
  assert.match(switching, /postgres:\/\/koreograf@db\/koreograf/);
});

test("the maintenance banner follows the server state", async () => {
  const state = await load("core/state.js");
  const app = await load("core/app.js");
  assert.match(app.maintenanceBanner(), /maintenance-banner" hidden/);
  state.setMaintenanceActive(true);
  state.setMaintenanceSince("2026-01-01T00:00:00Z");
  const banner = app.maintenanceBanner();
  assert.match(banner, /maintenance_mode_banner/);
  assert.match(banner, /2026-01-01T00:00:00Z/);
  const node = document.createElement("div");
  node.id = "maintenance-banner";
  node.innerHTML = '<p class="small"></p>';
  document.body.appendChild(node);
  globalThis.fetch = async () => ({
    ok: true,
    status: 200,
    text: async () => JSON.stringify({ active: true, since: "2026-01-01T00:00:00Z" }),
    json: async () => ({ active: true, since: "2026-01-01T00:00:00Z" }),
  });
  await app.loadMaintenance();
  assert.equal(node.hidden, false);
  assert.match(node.querySelector(".small").innerHTML, /2026-01-01T00:00:00Z/);
});

test("the wizard strings are in both catalogs in order", () => {
  const catalogs = ["de", "en"].map((id) => JSON.parse(readFileSync(join(__dirname, "..", "i18n", `${id}.json`), "utf8")));
  for (const [index, id] of ["de", "en"].entries()) {
    const values = catalogs[index];
    const keys = Object.keys(values);
    assert.deepEqual(keys, [...keys].sort(), `${id}.json is not alphabetical`);
    for (const key of [
      "move_database_to_postgresql",
      "step_1_target",
      "step_2_checks",
      "step_3_migration",
      "step_4_switch",
      "maintenance_mode_banner",
      "no_new_jobs_during_maintenance",
      "database_migration_done",
      "database_migration_failed",
      "database_check_unreachable",
      "run_the_checks_again",
      "finish_or_cancel_running_jobs",
      "restart_koreograf_after_the_change",
      "rollback_switches_the_url_back",
    ]) {
      assert.ok(typeof values[key] === "string" && values[key].trim(), `${key} is missing from ${id}.json`);
    }
  }
  assert.deepEqual(Object.keys(catalogs[0]), Object.keys(catalogs[1]));
});

test("the PostgreSQL docs link the other setup pages", () => {
  const docs = read("docs/postgresql.md");
  for (const text of ["Docker Compose", "OS package", "Helm", "Roll back"]) {
    assert.ok(docs.includes(text), `the PostgreSQL page misses ${text}`);
  }
  for (const page of ["installation.md", "configuration.md", "backup-restore.md"]) {
    assert.ok(read(`docs/${page}`).includes("postgresql.md"), `${page} does not link the PostgreSQL page`);
  }
});
