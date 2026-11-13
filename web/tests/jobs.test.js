"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "jobs.js")).href);

test("the job list state falls back to the first page and the default size", async () => {
  const { jobsState } = await load();
  assert.deepEqual(jobsState("#/jobs"), { page: 1, size: 25, q: "", status: "", tool: "", trigger: "", template: "" });
  assert.deepEqual(jobsState("#/jobs?page=0&size=500"), { page: 1, size: 25, q: "", status: "", tool: "", trigger: "", template: "" });
});

test("the job list state reads every filter from the hash", async () => {
  const { jobsState } = await load();
  assert.deepEqual(jobsState("#/jobs?page=3&size=50&q=web&status=failed&tool=ansible&trigger=schedule&template=tpl-1"), {
    page: 3,
    size: 50,
    q: "web",
    status: "failed",
    tool: "ansible",
    trigger: "schedule",
    template: "tpl-1",
  });
});

test("the job hash keeps the filters and leaves out defaults", async () => {
  const { jobsHash } = await load();
  assert.equal(jobsHash({ page: 1, size: 25, q: "", status: "", tool: "", trigger: "", template: "" }), "#/jobs");
  assert.equal(jobsHash({ page: 2, size: 100, q: "a b", status: "failed", tool: "shell", trigger: "hook", template: "tpl-1" }), "#/jobs?page=2&size=100&q=a+b&status=failed&tool=shell&trigger=hook&template=tpl-1");
});

test("the job request carries paging, offset and filters", async () => {
  const { jobsUrl } = await load();
  assert.equal(jobsUrl({ page: 1, size: 25, q: "", status: "", tool: "", trigger: "", template: "" }), "/api/v1/jobs?limit=25&offset=0");
  assert.equal(jobsUrl({ page: 3, size: 50, q: "web", status: "failed", tool: "ansible", trigger: "relaunch", template: "tpl-1" }), "/api/v1/jobs?limit=50&offset=100&q=web&status=failed&tool=ansible&trigger=relaunch&template_id=tpl-1");
  assert.match(jobsUrl({ page: 2, size: 25, q: "a&b", status: "", tool: "", trigger: "", template: "" }), /q=a%26b/);
});
