"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "schedules.js")).href);

test("a schedule payload carries the repeat and end settings", async () => {
  const { schedulePayload } = await load();
  assert.deepEqual(schedulePayload({
    timezone: "Europe/Berlin",
    start_date: "2026-11-13",
    start_time: "08:00",
    frequency: "weekly",
    weekdays: ["MO", "FR"],
    end_kind: "after",
    end_count: "5",
    end_date: "2026-12-01",
  }), {
    timezone: "Europe/Berlin",
    start_date: "2026-11-13",
    start_time: "08:00",
    frequency: "weekly",
    weekdays: ["MO", "FR"],
    end_kind: "after",
    end_count: 5,
    end_date: null,
  });
});

test("a one-off schedule never ends and keeps no end count", async () => {
  const { schedulePayload } = await load();
  const payload = schedulePayload({
    timezone: "UTC",
    start_date: "2026-11-13",
    start_time: "09:30",
    frequency: "once",
    weekdays: [],
    end_kind: "never",
    end_count: "5",
    end_date: "2026-11-13",
  });
  assert.equal(payload.end_kind, "never");
  assert.equal(payload.end_count, null);
  assert.equal(payload.end_date, null);
});

test("an end date is sent only for the on-date end kind", async () => {
  const { schedulePayload } = await load();
  const payload = schedulePayload({
    timezone: "UTC",
    start_date: "2026-11-13",
    start_time: "09:30",
    frequency: "daily",
    weekdays: [],
    end_kind: "on_date",
    end_count: "5",
    end_date: "2026-12-01",
  });
  assert.equal(payload.end_kind, "on_date");
  assert.equal(payload.end_count, null);
  assert.equal(payload.end_date, "2026-12-01");
});

test("the schedule form needs a non-empty name", async () => {
  const { scheduleNameError } = await load();
  assert.equal(scheduleNameError(""), "this_field_needs_a_valid_value");
  assert.equal(scheduleNameError("   "), "this_field_needs_a_valid_value");
  assert.equal(scheduleNameError("Nightly backup"), "");
});

test("schedule entries filter by target, repeat and next run", async () => {
  const { scheduleHaystack } = await load();
  const item = {
    name: "Nightly backup",
    target_name: "Backup template",
    timezone: "Europe/Berlin",
    start_time: "08:00",
    frequency: "weekly",
    weekdays: ["MO", "FR"],
    start_date: "2026-11-13",
    enabled: true,
  };
  const haystack = scheduleHaystack(item);
  assert.match(haystack, /nightly backup/);
  assert.match(haystack, /backup template/);
  assert.match(haystack, /europe\/berlin/);
  assert.match(haystack, /schedule\.short\.mo, schedule\.short\.fr/);
});

test("schedules sort missed first and then by the next run", async () => {
  const { compareSchedules } = await load();
  const missed = { name: "Missed", missed_at: "2026-11-12T08:00:00Z", enabled: true, next_run_at: "2026-11-14T08:00:00Z" };
  const early = { name: "Early", enabled: true, next_run_at: "2026-11-13T08:00:00Z" };
  const late = { name: "Late", enabled: true, next_run_at: "2026-11-15T08:00:00Z" };
  const disabled = { name: "Disabled", enabled: false, next_run_at: "2026-11-13T08:00:00Z" };
  const items = [disabled, late, early, missed];
  assert.deepEqual(items.sort(compareSchedules).map((item) => item.name), ["Missed", "Early", "Late", "Disabled"]);
});
