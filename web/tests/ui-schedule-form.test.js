"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { catalog } = require("./catalog.js");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

async function operator() {
  const state = await load("core/state.js");
  state.setMe({ is_admin: false, teams: [{ id: "t1", role: "operator" }] });
  state.setTeamId("t1");
}

test("the schedules page offers an operator-only empty state and add button", async () => {
  await operator();
  const { mountSchedules } = await load("features/schedules.js");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "[]" });
  const card = document.createElement("section");
  card.dataset.kind = "all";
  document.body.appendChild(card);
  mountSchedules(card);
  await new Promise((resolve) => setImmediate(resolve));
  assert.match(card.innerHTML, /id="schedule-add"/);
  const host = card.querySelector(".schedule-table");
  assert.match(host.innerHTML, /data-schedule-new/);
  assert.match(host.innerHTML, /schedule_empty_hint/);
});

test("the schedule form can target a template or a workflow", async () => {
  await operator();
  const { openScheduleForm } = await load("features/schedules.js");
  openScheduleForm("", "", null, async () => {}, { targets: '<option value="template:t1">Healthcheck</option>' });
  const overlay = document.querySelector("#schedule-form");
  assert.match(overlay.innerHTML, /name="target_ref"/);
  assert.match(overlay.innerHTML, /choose_a_target/);
  assert.match(overlay.innerHTML, /schedule_target_hint/);
  assert.match(overlay.innerHTML, /Healthcheck/);
});

test("the schedule strings are in both catalogs", () => {
  for (const id of ["de", "en"]) {
    const values = catalog(id);
    for (const key of ["choose_a_target", "schedule_empty_hint", "schedule_needs_target", "schedule_target_hint", "template_or_workflow", "template_schedules_hint"]) {
      assert.equal(typeof values[key], "string", `${id}: ${key}`);
      assert.ok(values[key].trim().length > 0, `${id}: ${key}`);
    }
  }
});
