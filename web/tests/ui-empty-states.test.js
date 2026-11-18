"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
const { catalog } = require("./catalog.js");

const overview = { jobs: [], deliveries: [], cache_bytes: 0, runner_mode: "local" };

async function setup(items = {}) {
  const state = await load("core/state.js");
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  const router = await load("core/router.js");
  globalThis.fetch = async (path) => {
    let body = [];
    if (path.startsWith("/api/v1/overview")) body = { ...overview, ...(items.overview || {}) };
    else if (path.startsWith("/api/v1/jobs/")) body = items.job || {};
    else if (path.startsWith("/api/v1/jobs")) body = items.jobs || { items: [], total: 0 };
    else if (path.includes("/assistant")) body = { enabled: false };
    else if (path.startsWith("/api/v1/maintenance")) body = { active: false };
    else if (path.includes("/version")) body = { version: "1.0.0", commit: "0123456789" };
    else if (items[path]) body = items[path];
    return { ok: true, status: 200, text: async () => JSON.stringify(body) };
  };
  for (const path of ["features/dashboard.js", "features/templates.js", "features/projects.js", "features/inventories.js", "features/workflows/index.js", "features/credentials.js", "features/jobs.js"]) {
    await load(path);
  }
  return { state, router };
}

test("the overview shows the onboarding card only for an empty team", async () => {
  const { state, router } = await setup();
  await router.matchRoute("#/").handler();
  for (const key of ["get_started", "empty_projects", "empty_inventories", "empty_templates", "empty_jobs", "new_project", "new_inventory", "new_template", "launch_a_template"]) {
    assert.ok(state.app.innerHTML.includes(key), `the onboarding card misses ${key}`);
  }
  const withJobs = await setup({ overview: { jobs: [{ id: "j1", name: "Job", tool: "shell", status: "successful", created_at: "2026-01-01T00:00:00Z" }] } });
  await withJobs.router.matchRoute("#/").handler();
  assert.ok(!withJobs.state.app.innerHTML.includes("get_started"), "the onboarding card shows next to jobs");
});

test("empty lists link to their create action", async () => {
  const { state, router } = await setup();
  for (const [hash, key] of [
    ["#/templates", "empty_templates"],
    ["#/projects", "empty_projects"],
    ["#/inventories", "empty_inventories"],
    ["#/workflows", "empty_workflows"],
    ["#/credentials", "empty_credentials"],
    ["#/jobs", "empty_jobs"],
  ]) {
    location.hash = hash;
    const match = router.matchRoute(hash);
    assert.ok(match, `${hash} has no route`);
    await match.handler();
    const empty = state.app.querySelector(".empty-state");
    assert.ok(empty, `${hash} has no guided empty state`);
    assert.ok(empty.innerHTML.includes(key), `${hash} has no ${key} copy`);
    assert.ok(empty.querySelector(".btn.primary"), `${hash} does not gate the create action`);
  }
});

test("the job log is scrollable and its tools hide on the parameters tab", async () => {
  const { state, router } = await setup({
    job: {
      id: "j1",
      name: "Job",
      tool: "shell",
      status: "successful",
      trigger: "manual",
      launched_by: "admin",
      created_at: "2026-01-01T00:00:00Z",
      started_at: "2026-01-01T00:00:01Z",
      finished_at: "2026-01-01T00:00:02Z",
      events: [],
      parameters: null,
      extra_vars: {},
    },
  });
  location.hash = "#/jobs/j1";
  await router.matchRoute("#/jobs/:id").handler({ id: "j1" });
  assert.match(state.app.innerHTML, /id="log" tabindex="0"/);
  assert.match(state.app.innerHTML, /class="job-sticky"/);
  assert.match(state.app.innerHTML, /id="output-tools"/);
  document.querySelector('[data-job-tab="parameters"]').dispatchEvent({ type: "click" });
  assert.equal(document.querySelector("#output-tools").hidden, true, "the parameters tab still shows the log tools");
  assert.equal(document.querySelector("#panel-output").hidden, true);
  assert.equal(document.querySelector("#panel-parameters").hidden, false);
});

function tallJob(failed) {
  const events = Array.from({ length: 40 }, (_, index) => ({ id: index + 1, stream: "stdout", line: `line ${index}` }));
  if (failed) events[12] = { id: 13, stream: "stderr", line: "fatal: [web-1]: FAILED! => changed=false" };
  return {
    id: "j1",
    name: "Job",
    tool: "shell",
    status: failed ? "failed" : "successful",
    trigger: "manual",
    launched_by: "admin",
    created_at: "2026-01-01T00:00:00Z",
    started_at: "2026-01-01T00:00:01Z",
    finished_at: "2026-01-01T00:00:02Z",
    events,
    parameters: null,
    extra_vars: {},
  };
}

async function jobPage(job) {
  const { router } = await setup({ job });
  location.hash = "#/jobs/j1";
  await router.matchRoute("#/jobs/:id").handler({ id: "j1" });
}

test("jump to first failure scrolls only the log", async () => {
  await jobPage(tallJob(true));
  const log = document.querySelector("#log");
  const first = document.querySelector("#log-first-failure");
  const main = document.querySelector("main.main");
  assert.ok(main, "the job page has no main element");
  assert.equal(first.hidden, false, "the jump button is hidden with a failed line");
  log.clientHeight = 300;
  log.scrollHeight = 1200;
  log.offsetTop = 0;
  log.children.forEach((line, index) => {
    line.offsetTop = index * 20;
    line.offsetHeight = 20;
  });
  const scrolled = [];
  const intoView = globalThis.Element.prototype.scrollIntoView;
  globalThis.Element.prototype.scrollIntoView = function scroll() {
    scrolled.push(this);
  };
  const pageScrolls = [];
  const scrollTo = window.scrollTo;
  window.scrollTo = (...args) => pageScrolls.push(args);
  window.scrollY = 140;
  main.scrollTop = 60;
  try {
    first.dispatchEvent({ type: "click" });
    assert.equal(log.scrollTop, 100, "the failed line is not centered in the log");
    assert.deepEqual(scrolled, [], "jumping scrolled an element into view");
    assert.deepEqual(pageScrolls, [], "jumping scrolled the page");
    assert.equal(window.scrollY, 140, "the page scroll position changed");
    assert.equal(main.scrollTop, 60, "the main scroll position changed");
    log.scrollTop = 0;
    const wrap = document.querySelector("#log-wrap");
    wrap.checked = true;
    wrap.dispatchEvent({ type: "change" });
    first.dispatchEvent({ type: "click" });
    assert.equal(log.classList.contains("wrap"), true, "wrap lines did not stay on");
    assert.equal(log.scrollTop, 100, "the wrapped failed line is not centered in the log");
    assert.deepEqual(scrolled, [], "jumping with wrapped lines scrolled an element into view");
    assert.deepEqual(pageScrolls, [], "jumping with wrapped lines scrolled the page");
    const search = document.querySelector("#log-search");
    search.value = "line 35";
    search.dispatchEvent({ type: "input" });
    assert.equal(log.scrollTop, 560, "the search jump is not centered in the log");
    assert.deepEqual(scrolled, [], "the search jump scrolled an element into view");
    assert.deepEqual(pageScrolls, [], "the search jump scrolled the page");
    assert.equal(window.scrollY, 140, "the search jump changed the page scroll position");
    assert.equal(main.scrollTop, 60, "the search jump changed the main scroll position");
  } finally {
    globalThis.Element.prototype.scrollIntoView = intoView;
    window.scrollTo = scrollTo;
  }
});

test("the jump button stays hidden without a failed line", async () => {
  await jobPage(tallJob(false));
  assert.equal(document.querySelector("#log-first-failure").hidden, true, "the jump button shows without a failed line");
});

test("the empty state copy is in both catalogs", () => {
  for (const id of ["de", "en"]) {
    const values = catalog(id);
    for (const key of ["empty_credentials", "empty_inventories", "empty_jobs", "empty_projects", "empty_templates", "empty_workflows", "get_started", "get_started_help", "launch_a_template"]) {
      assert.ok(typeof values[key] === "string" && values[key].trim(), `${key} is missing from ${id}.json`);
    }
  }
});
