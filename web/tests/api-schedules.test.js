"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

test("the schedule address reads and writes edit and new targets", async () => {
  const { scheduleQuery, scheduleAddress, scheduleNewTarget } = await load("features/schedules.js");
  const edit = scheduleQuery("#/schedules?edit=s1");
  assert.equal(edit.edit, "s1");
  assert.equal(edit.create, null);
  assert.equal(scheduleQuery("#/schedules?new=1").create, "1");
  assert.deepEqual(scheduleNewTarget({ create: "template:t1", kind: "", target: "" }), { kind: "template", target: "t1" });
  assert.deepEqual(scheduleNewTarget({ create: "1", kind: "workflow", target: "w1" }), { kind: "workflow", target: "w1" });
  assert.equal(scheduleAddress({ edit: "s1" }, "#/schedules?page=2"), "#/schedules?page=2&edit=s1");
  assert.equal(scheduleAddress({ new: "1" }, "#/templates/t1"), "#/templates/t1?new=1");
  assert.equal(scheduleAddress({}, "#/schedules?edit=s1&page=2"), "#/schedules?page=2");
});

test("the schedule form guards unsaved edits", async () => {
  const state = await load("core/state.js");
  const { openScheduleForm } = await load("features/schedules.js");
  openScheduleForm("template", "t1", null, async () => {});
  const form = document.querySelector("#schedule-form");
  assert.ok(form, "the schedule form did not open");
  assert.equal(typeof state.leaveGuard, "function", "the schedule form did not install a leave guard");
  assert.equal(state.leaveGuard(), false, "a fresh schedule form is dirty");
  const name = form.querySelector('[name="name"]');
  name.value = "Nightly";
  form.querySelector(".dialog-form").dispatchEvent({ type: "input" });
  assert.equal(state.leaveGuard(), true, "an edited schedule form is not dirty");

  form.querySelector("[data-dialog-cancel]").dispatchEvent({ type: "click" });
  const confirm = document.querySelector("#confirm-dialog");
  assert.ok(confirm, "a dirty schedule form closed without asking");
  confirm.querySelector(".dialog-form").dispatchEvent({ type: "submit" });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(state.leaveGuard, null, "the leave guard survived the close");
});
