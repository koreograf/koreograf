"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "activity.js")).href);

test("the activity state falls back to the first page and the default size", async () => {
  const { activityState } = await load();
  assert.deepEqual(activityState("#/activity"), { page: 1, size: 25, actor: "", action: "", object: "", from: "", to: "" });
  assert.deepEqual(activityState("#/activity?page=0&size=500"), { page: 1, size: 25, actor: "", action: "", object: "", from: "", to: "" });
});

test("the activity state reads every filter from the hash", async () => {
  const { activityState } = await load();
  assert.deepEqual(activityState("#/activity?page=3&size=50&actor=alice&action=create&object=template&from=2026-01-01&to=2026-02-01"), {
    page: 3,
    size: 50,
    actor: "alice",
    action: "create",
    object: "template",
    from: "2026-01-01",
    to: "2026-02-01",
  });
});

test("the activity hash keeps the filters and leaves out defaults", async () => {
  const { activityHash } = await load();
  assert.equal(activityHash({ page: 1, size: 25, actor: "", action: "", object: "", from: "", to: "" }), "#/activity");
  assert.equal(
    activityHash({ page: 2, size: 100, actor: "a b", action: "launch", object: "job", from: "2026-01-01", to: "2026-01-31" }),
    "#/activity?page=2&size=100&actor=a+b&action=launch&object=job&from=2026-01-01&to=2026-01-31"
  );
});

test("the activity request carries paging, exact filters and UTC time bounds", async () => {
  const { activityUrl } = await load();
  assert.equal(activityUrl({ page: 1, size: 25, actor: "", action: "", object: "", from: "", to: "" }), "/api/v1/activity?limit=25&offset=0");
  assert.equal(
    activityUrl({ page: 3, size: 50, actor: "a b", action: "create", object: "template", from: "2026-01-01", to: "2026-02-01" }),
    "/api/v1/activity?limit=50&offset=100&actor=a+b&action=create&object_type=template&since=2026-01-01T00%3A00%3A00Z&until=2026-02-01T23%3A59%3A59Z"
  );
  assert.equal(
    activityUrl({ page: 1, size: 25, actor: "", action: "", object: "", from: "not-a-date", to: "" }),
    "/api/v1/activity?limit=25&offset=0"
  );
});

test("the activity table renders and links the actor, action, object and result", async () => {
  const { activityTable } = await load();
  const html = activityTable([
    {
      created_at: "2026-01-01T00:00:00Z",
      actor: "alice",
      action: "create",
      object_type: "template",
      object_id: "tpl-1",
      object_name: "Demo template",
      result: "success",
    },
  ]);
  assert.match(html, /href="#\/templates\?edit=tpl-1"/);
  assert.match(html, /Demo template/);
  assert.match(html, />alice</);
  assert.match(html, /action_create/);
  assert.match(html, /object_template/);
  assert.match(html, /class="pill successful"/);
  assert.match(html, /column_time/);
  assert.match(html, /<div class="table-wrap">/);
});

test("an unknown activity action and object type stay readable", async () => {
  const { activityTable, actionLabel, objectLabel } = await load();
  assert.equal(actionLabel("archive"), "archive");
  assert.equal(objectLabel("report"), "report");
  const html = activityTable([
    {
      created_at: "2026-01-01T00:00:00Z",
      actor: "system",
      action: "archive",
      object_type: "report",
      object_id: "",
      object_name: "Monthly report",
      result: "failed",
    },
  ]);
  assert.match(html, /Monthly report/);
  assert.match(html, /class="pill failed"/);
  assert.doesNotMatch(html, /href=/);
});
