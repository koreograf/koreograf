"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "hooks.js")).href);

test("the webhook list filter matches name, alert and status", async () => {
  const { hookMatches } = await load();
  const hook = { name: "Disk", match_alertname: "DiskSpaceLow", match_status: "firing" };
  assert.equal(hookMatches(hook, ""), true);
  assert.equal(hookMatches(hook, "disk"), true);
  assert.equal(hookMatches(hook, "diskspacelow"), true);
  assert.equal(hookMatches(hook, "firing"), true);
  assert.equal(hookMatches(hook, "resolved"), false);
});

test("a webhook target splits the kind from the id", async () => {
  const { hookTarget } = await load();
  assert.deepEqual(hookTarget("template:tpl-1"), { kind: "template", id: "tpl-1" });
  assert.deepEqual(hookTarget("workflow:wf:1"), { kind: "workflow", id: "wf:1" });
  assert.throws(() => hookTarget(""), /create_a_template_or_a_workflow_first/);
});

test("a webhook payload points at exactly one target", async () => {
  const { hookPayload } = await load();
  const form = {
    name: { value: "Disk" },
    match_alertname: { value: "DiskSpaceLow" },
    match_status: { value: "firing" },
    extra_vars: { value: `{"host":"web-1"}` },
    target: { value: "workflow:wf-1" },
  };
  assert.deepEqual(hookPayload(form), {
    name: "Disk",
    match_alertname: "DiskSpaceLow",
    match_status: "firing",
    extra_vars: { host: "web-1" },
    template_id: null,
    workflow_id: "wf-1",
  });
});
