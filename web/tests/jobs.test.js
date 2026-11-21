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

const structuredEvents = [
  { id: 1, event: { type: "play_start", play: "Site", index: 1 } },
  { id: 2, event: { type: "task_start", play: "Site", play_index: 1, index: 1, task: "Deploy", role: "web" } },
  { id: 3, event: { type: "host_result", play: "Site", play_index: 1, index: 1, task: "Deploy", role: "web", host: "web-1", status: "changed", duration: 1.25, msg: "changed" } },
  { id: 4, event: { type: "host_result", play: "Site", play_index: 1, index: 1, task: "Deploy", role: "web", host: "web-2", status: "failed", duration: 2.5, msg: "boom" } },
  { id: 5, event: { type: "task_start", play: "Site", play_index: 1, index: 2, task: "Facts" } },
  { id: 6, event: { type: "host_result", play: "Site", play_index: 1, index: 2, task: "Facts", host: "web-1", status: "ok", duration: 0.05 } },
  { id: 7, event: { type: "host_result", play: "Site", play_index: 1, index: 2, task: "Facts", host: "web-2", status: "skipped" } },
];

test("structured output builds plays, tasks and host results", async () => {
  const { buildAnsibleTree, hostedNames } = await load();
  const tree = buildAnsibleTree(structuredEvents);
  assert.equal(tree.plays.length, 1);
  const play = tree.plays[0];
  assert.equal(play.name, "Site");
  assert.equal(play.status, "failed");
  assert.equal(play.tasks.length, 2);
  assert.equal(play.tasks[0].name, "Deploy");
  assert.equal(play.tasks[0].role, "web");
  assert.equal(play.tasks[0].status, "failed");
  assert.equal(play.tasks[0].duration, 2.5);
  assert.deepEqual(play.tasks[0].counts, { changed: 1, failed: 1 });
  assert.deepEqual(hostedNames(tree), ["web-1", "web-2"]);
});

test("structured output filters by host and status and puts failures first", async () => {
  const { buildAnsibleTree, filterAnsibleTree } = await load();
  const tree = buildAnsibleTree(structuredEvents);
  const host = filterAnsibleTree(tree, { host: "web-1" });
  assert.equal(host.plays[0].tasks.length, 2);
  assert.equal(host.plays[0].tasks[0].hosts.length, 1);
  assert.equal(host.plays[0].tasks[0].status, "changed");
  const failed = filterAnsibleTree(tree, { status: "failed" });
  assert.equal(failed.plays[0].tasks.length, 1);
  assert.equal(failed.plays[0].tasks[0].name, "Deploy");
  const first = filterAnsibleTree(tree, { failedFirst: true });
  assert.equal(first.plays[0].tasks[0].status, "failed");
  assert.equal(first.plays[0].tasks[1].status, "ok");
  const none = filterAnsibleTree(tree, { host: "db-1" });
  assert.deepEqual(none.plays, []);
});

test("structured output keeps no_log placeholders and durations readable", async () => {
  const { buildAnsibleTree, formatDuration } = await load();
  const tree = buildAnsibleTree([
    { event: { type: "host_result", host: "db-1", status: "failed", censored: true, duration: 0.05, msg: "secret" } },
  ]);
  assert.equal(tree.plays[0].tasks[0].hosts[0].censored, true);
  assert.equal(tree.plays[0].tasks[0].hosts[0].msg, "");
  assert.equal(formatDuration(0.05), "<0.1s");
  assert.equal(formatDuration(1.25), "1.3s");
  assert.equal(formatDuration(65), "1m 5s");
  assert.equal(formatDuration(null), "");
});

test("the output view preference falls back to structured", async () => {
  const { outputViewPreference, rememberOutputView, structuredEventsAvailable } = await load();
  const store = new Map();
  window.localStorage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => store.set(key, value),
  };
  assert.equal(outputViewPreference(), "structured");
  rememberOutputView("raw");
  assert.equal(outputViewPreference(), "raw");
  rememberOutputView("structured");
  assert.equal(outputViewPreference(), "structured");
  assert.equal(structuredEventsAvailable({ tool: "ansible", ansible_events: structuredEvents }), true);
  assert.equal(structuredEventsAvailable({ tool: "ansible", ansible_events: [] }), false);
  assert.equal(structuredEventsAvailable({ tool: "shell", ansible_events: structuredEvents }), false);
});
