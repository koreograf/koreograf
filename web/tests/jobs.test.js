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

test("an ok result with changed true is shown as changed", async () => {
  const { buildAnsibleTree } = await load();
  const tree = buildAnsibleTree([
    { event: { type: "host_result", host: "web-1", status: "ok", changed: true } },
  ]);
  assert.equal(tree.plays[0].tasks[0].hosts[0].status, "changed");
  assert.deepEqual(tree.plays[0].tasks[0].counts, { changed: 1 });
});

test("the live cursor stays on the last raw event so a late raw line is not skipped", async () => {
  const { lastRawEventId, pushStructuredEvent } = await load();
  const job = {
    events: [{ id: 100, stream: "stdout" }],
    ansible_events: [{ id: 102, event: { type: "play_start", play: "Site" } }],
  };
  // The structured row was read after the raw page; keeping the cursor on the raw
  // id replays the raw line with id 101 instead of starting after 102.
  assert.equal(lastRawEventId(job), 100);
  assert.equal(lastRawEventId({ events: [], ansible_events: job.ansible_events }), 0);
  assert.equal(lastRawEventId({}), 0);
  const state = { events: [...job.ansible_events], seen: new Set([102]) };
  assert.equal(pushStructuredEvent(state, job.ansible_events[0]), false);
  assert.equal(state.events.length, 1);
});

test("live structured events use every seeded id once", async () => {
  const { pushStructuredEvent } = await load();
  const state = { events: [{ id: 7 }], seen: new Set([7]) };
  assert.equal(pushStructuredEvent(state, { id: 7, event: { type: "playbook_end" } }), false);
  assert.equal(state.events.length, 1);
  assert.equal(pushStructuredEvent(state, { id: 8, event: { type: "playbook_end" } }), true);
  assert.equal(state.events.length, 2);
});

const recapRows = [
  { host: "web-1", ok: 3, changed: 1, unreachable: 0, failed: 0, skipped: 1, rescued: 0, ignored: 0 },
  { host: "web-2", ok: 1, changed: 0, unreachable: 0, failed: 1, skipped: 0, rescued: 0, ignored: 0 },
  { host: "db-1", ok: 0, changed: 0, unreachable: 1, failed: 0, skipped: 0, rescued: 0, ignored: 0 },
  { host: "db-2", ok: 2, changed: 0, unreachable: 0, failed: 0, skipped: 0, rescued: 1, ignored: 0 },
  { host: "web-3", ok: 0, changed: 0, unreachable: 0, failed: 0, skipped: 2, rescued: 0, ignored: 0 },
];

test("the recap totals count one status per host", async () => {
  const { recapTotals, recapHostStatus } = await load();
  const totals = recapTotals(recapRows);
  assert.equal(totals.hosts, 5);
  assert.deepEqual(totals.counts, {
    ok: 0,
    changed: 1,
    unreachable: 1,
    failed: 1,
    skipped: 1,
    rescued: 1,
    ignored: 0,
  });
  assert.equal(recapHostStatus({ host: "empty" }), "ok");
  assert.equal(Object.values(totals.counts).reduce((sum, value) => sum + value, 0), totals.hosts);
});

test("stats chunks aggregate every host and report a missing part", async () => {
  const { recapFromEvents } = await load();
  const events = [
    {
      id: 1,
      event: {
        type: "stats",
        total: 3,
        part: 1,
        parts: 2,
        hosts: [
          { host: "bulk-1", ok: 2, changed: 0, unreachable: 0, failed: 0, skipped: 0, rescued: 0, ignored: 0 },
          { host: "bulk-2", ok: 0, changed: 0, unreachable: 0, failed: 1, skipped: 0, rescued: 0, ignored: 0 },
        ],
      },
    },
    {
      id: 2,
      event: {
        type: "stats",
        total: 3,
        part: 2,
        parts: 2,
        hosts: [
          { host: "bulk-3", ok: 0, changed: 0, unreachable: 1, failed: 0, skipped: 0, rescued: 0, ignored: 0 },
        ],
      },
    },
  ];
  const complete = recapFromEvents(events);
  assert.equal(complete.total, 3);
  assert.equal(complete.rows.length, 3);
  assert.equal(complete.complete, true);
  const partial = recapFromEvents(events.slice(0, 1));
  assert.equal(partial.rows.length, 2);
  assert.equal(partial.complete, false);
});

test("stats rows win over a raw recap line and every host is kept", async () => {
  const { mergeRecapRows, recapHostStatus } = await load();
  const raw = [
    { host: "bulk-1", ok: 0, changed: 0, unreachable: 0, failed: 1, skipped: 0, rescued: 0, ignored: 0 },
    { host: "old-1", ok: 1, changed: 0, unreachable: 0, failed: 0, skipped: 0, rescued: 0, ignored: 0 },
  ];
  const stats = [
    { host: "bulk-1", ok: 3, changed: 0, unreachable: 0, failed: 0, skipped: 0, rescued: 0, ignored: 0 },
    { host: "bulk-2", ok: 0, changed: 0, unreachable: 1, failed: 0, skipped: 0, rescued: 0, ignored: 0 },
  ];
  const merged = mergeRecapRows(stats, raw);
  assert.deepEqual(merged.map((row) => row.host).sort(), ["bulk-1", "bulk-2", "old-1"]);
  assert.equal(recapHostStatus(merged.find((row) => row.host === "bulk-1")), "ok");
});

test("the host list filters by status and substring and sorts by severity", async () => {
  const { filterRecapRows } = await load();
  const problems = filterRecapRows(recapRows, { status: "problems" });
  assert.deepEqual(problems.map((row) => row.host), ["db-1", "web-2"]);
  const web = filterRecapRows(recapRows, { status: "all", search: "WEB" });
  assert.deepEqual(web.map((row) => row.host), ["web-2", "web-1", "web-3"]);
  const byHost = filterRecapRows(recapRows, { status: "all", sort: "host" });
  assert.deepEqual(byHost.map((row) => row.host), ["db-1", "db-2", "web-1", "web-2", "web-3"]);
  const reversed = filterRecapRows(recapRows, { status: "all", sort: "-host" });
  assert.deepEqual(reversed.map((row) => row.host), ["web-3", "web-2", "web-1", "db-2", "db-1"]);
  const byFailed = filterRecapRows(recapRows, { status: "all", sort: "failed" });
  assert.deepEqual(byFailed.map((row) => row.host), ["db-1", "db-2", "web-1", "web-3", "web-2"]);
});

test("the host list pages with a bounded number of rows", async () => {
  const { pageRecapRows } = await load();
  const first = pageRecapRows(recapRows, 1, 2);
  assert.deepEqual(first.rows.map((row) => row.host), ["web-1", "web-2"]);
  assert.equal(first.pages, 3);
  assert.equal(first.from, 1);
  assert.equal(first.to, 2);
  const last = pageRecapRows(recapRows, 9, 2);
  assert.equal(last.page, 3);
  assert.deepEqual(last.rows.map((row) => row.host), ["web-3"]);
  assert.equal(last.to, 5);
  assert.deepEqual(pageRecapRows([], 2, 50), { rows: [], page: 1, pages: 1, total: 0, from: 0, to: 0 });
});

test("the host filter accepts a substring and a set of recap hosts", async () => {
  const { buildAnsibleTree, filterAnsibleTree } = await load();
  const tree = buildAnsibleTree(structuredEvents);
  const partial = filterAnsibleTree(tree, { host: "we" });
  assert.equal(partial.plays[0].tasks[0].hosts.length, 2);
  const only = filterAnsibleTree(tree, { hosts: new Set(["web-1"]) });
  assert.deepEqual(only.plays[0].tasks.flatMap((task) => task.hosts).map((host) => host.host), ["web-1", "web-1"]);
  const none = filterAnsibleTree(tree, { hosts: new Set(["db-1"]) });
  assert.deepEqual(none.plays, []);
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
