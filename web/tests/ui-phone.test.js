"use strict";

const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { mediaBlock, rule, stylesheet } = require("./css.js");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

test("the shell wraps headers and uses the dynamic viewport", () => {
  const css = stylesheet();
  for (const selector of [".top", ".spread"]) {
    assert.match(rule(css, selector)[1], /flex-wrap:\s*wrap/, `${selector} does not wrap`);
  }
  assert.match(rule(css, ".shell")[1], /min-height:\s*100dvh/, "the shell does not use the dynamic viewport");
  assert.match(rule(css, ".login-wrap")[1], /min-height:\s*100dvh/, "the login page does not use the dynamic viewport");
  assert.match(rule(css, ".side")[1], /safe-area-inset-bottom/, "the drawer ignores the bottom safe area");
  assert.match(rule(css, ".main")[1], /safe-area-inset-bottom/, "the main column ignores the bottom safe area");
});

test("the drawer toggle and nav links are touch sized", () => {
  const css = stylesheet();
  const narrow = mediaBlock(css, "@media (max-width: 900px)");
  const toggle = /\.nav-toggle\s*\{([^}]*)\}/.exec(narrow);
  assert.ok(toggle, "the narrow drawer toggle is missing");
  assert.match(toggle[1], /width:\s*44px/);
  assert.match(toggle[1], /height:\s*44px/);
  const coarse = mediaBlock(css, "@media (pointer: coarse)");
  assert.match(coarse, /\.nav a\s*\{[^}]*min-height:\s*44px/, "nav links are below 44 px");
});

test("the shared table labels every cell from the header row", async () => {
  require("./test-dom.js").install();
  const { table } = await load("core/dom.js");
  const html = table(
    ["Name", "Tool", ""],
    ['<tr><td>one</td><td class="tool">shell</td><td><button class="btn compact" type="button">Edit</button></td></tr>'],
  );
  assert.match(html, /<td data-label="Name">one<\/td>/);
  assert.match(html, /<td class="tool" data-label="Tool">shell<\/td>/);
  assert.match(html, /<td><button/);
  const kept = table(["Name"], ['<tr><td data-label="Custom">one</td></tr>']);
  assert.match(kept, /data-label="Custom"/);
  assert.doesNotMatch(kept, /data-label="Name"/);
});

test("the narrow card rows cover schedules and inventory hosts", () => {
  const block = mediaBlock(stylesheet(), "@media (max-width: 640px)");
  assert.doesNotMatch(block, /:not\(\.schedule-table\)/, "the schedule table stays out of the card layout");
  assert.doesNotMatch(block, /:not\(\.inventory-hosts-table\)/, "the host table stays out of the card layout");
  assert.match(block, /min-width:\s*0/, "the wide tables keep their minimum width on phones");
  assert.match(block, /\.table-wrap \.table td\[data-label\]::before/, "the card rows have no labels");
});

test("schedule and inventory tables label their stacked rows", async () => {
  const { scheduleTable } = await load("features/schedules.js");
  const schedule = scheduleTable([
    {
      id: "s1",
      name: "Nightly",
      enabled: true,
      frequency: "daily",
      start_date: "2026-11-20",
      start_time: "08:00",
      timezone: "UTC",
      end_kind: "never",
      target_id: "t1",
      target_kind: "template",
      target_name: "Healthcheck",
    },
  ], true, "empty");
  for (const key of ["name", "schedule_target", "repeat", "start_time", "next_run", "ends", "schedule_active"]) {
    assert.match(schedule, new RegExp(`data-label="${key}"`), `the schedule table lost the ${key} label`);
  }

  const inventories = await load("features/inventories.js");
  const hosts = inventories.inventoryHostTable([{ name: "web01", vars: { ansible_host: "10.0.0.1" }, groups: ["web"] }], { sort: "name" }, "inv1");
  for (const key of ["name", "address", "groups"]) {
    assert.match(hosts, new RegExp(`data-label="${key}"`), `the host table lost the ${key} label`);
  }
  const groups = inventories.inventoryGroupTable([{ name: "web", host_count: 1, children: [] }], "inv1");
  for (const key of ["name", "hosts", "groups"]) {
    assert.match(groups, new RegExp(`data-label="${key}"`), `the group table lost the ${key} label`);
  }
  const history = inventories.hostHistoryTable([{ time: "2026-11-20T08:00:00Z", playbook: "site.yml", role: "r", result: "successful", job_id: "j1", job_name: "Job" }]);
  for (const key of ["time", "playbook", "role", "run_result", "job"]) {
    assert.match(history, new RegExp(`data-label="${key}"`), `the history table lost the ${key} label`);
  }
});

test("coarse pointer controls reach 44 px", () => {
  const coarse = mediaBlock(stylesheet(), "@media (pointer: coarse)");
  for (const selector of [
    ".btn.compact",
    ".switch",
    ".chip-remove",
    ".inspector-toggle",
    ".canvas-controls .btn",
    ".node-menu-button",
    ".wf-port-add",
  ]) {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    assert.match(coarse, new RegExp(`${escaped}\\s*\\{[^}]*min-height:\\s*44px`), `${selector} is below 44 px`);
    assert.match(coarse, new RegExp(`${escaped}\\s*\\{[^}]*min-width:\\s*44px`), `${selector} is below 44 px`);
  }
  assert.match(coarse, /textarea\s*\{[^}]*min-height:\s*44px/, "form controls stay small on touch screens");
});

test("a fullscreen overlay keeps the shared chrome and scrolls its body", () => {
  const css = stylesheet();
  const full = mediaBlock(css, "@media (max-width: 600px)");
  assert.match(full, /\.overlay-card[^}]*height:\s*100dvh/, "the fullscreen overlay does not use the dynamic viewport");
  assert.match(full, /safe-area-inset-bottom/, "the fullscreen overlay ignores the safe area");
  assert.match(rule(css, ".dialog-body")[1], /overflow:\s*auto/, "the dialog body does not scroll");
  assert.match(rule(css, ".dialog-form")[1], /display:\s*flex/, "the edit chrome is not the shared form layout");
});
