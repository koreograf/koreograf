"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { rule, stylesheet } = require("./css.js");
const { catalog } = require("./catalog.js");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

test("the job log hides escape codes without losing the raw line", async () => {
  const { appendLog } = await load("features/jobs.js");
  const log = document.createElement("div");
  log.id = "log";
  document.body.appendChild(log);
  appendLog({ line: "\u001b[31mred\u001b[0m", stream: "stdout" });
  appendLog({ line: "\u001b[1;32mok\u001b[0m run", stream: "stdout" });
  appendLog({ line: "\u001b[2Kline", stream: "stdout" });
  assert.deepEqual(log.children.map((line) => line.textContent), ["red", "ok run", "line"]);
  assert.equal(log.children[0].koreografRaw, "red");
});

test("the sidebar shows the short commit and keeps the full one in the title", async () => {
  const state = await load("core/state.js");
  const { fillVersion } = await load("core/app.js");
  state.setBuildVersion("1.2.3");
  state.setBuildCommit("0123456789abcdef");
  const line = document.createElement("p");
  line.setAttribute("data-version", "");
  line.innerHTML = '<a href="#"></a>';
  line.hidden = true;
  document.body.appendChild(line);
  fillVersion();
  const link = line.querySelector("a");
  assert.equal(link.textContent, "build_version");
  assert.match(link.title, /^0123456789abcdef$/);
  assert.equal(line.hidden, false);
});

test("the router tracks direction and restores the scroll position", async () => {
  const app = await load("core/app.js");
  const state = await load("core/state.js");
  const calls = [];
  window.scrollTo = (left, top) => calls.push([left, top]);
  state.setHistoryIndex(0);
  assert.equal(app.routeDirection(null), false);
  assert.equal(state.historyIndex, 1);
  assert.equal(app.routeDirection({ idx: 2 }), false);
  assert.equal(app.routeDirection({ idx: 1 }), true);
  assert.equal(app.routeDirection({ idx: 2 }), false);

  location.hash = "#/jobs";
  state.routeScroll.set("#/jobs", 240);
  const main = document.createElement("main");
  main.className = "main";
  main.innerHTML = '<h1 tabindex="-1">Jobs</h1>';
  document.body.appendChild(main);
  app.settleRoute(true);
  assert.deepEqual(calls.at(-1), [0, 240]);
  document.activeElement = document.body;
  app.settleRoute(false);
  assert.deepEqual(calls.at(-1), [0, 0]);
  assert.equal(document.activeElement, main.querySelector("h1"), "a new page did not focus the heading");
});

test("the schedule start date is bounded ten years ahead", async () => {
  const { latestStartDate, openScheduleForm } = await load("features/schedules.js");
  assert.equal(latestStartDate("2026-11-10"), "2036-11-10");
  assert.equal(latestStartDate("2028-02-29"), "2038-02-28");
  openScheduleForm("template", "t1", null, async () => {}, {});
  const overlay = document.querySelector("#schedule-form");
  assert.ok(overlay, "the schedule form did not open");
  assert.match(overlay.innerHTML, /type="date" required max="2036-/);
});

test("long values and pills stay on one line", () => {
  const css = stylesheet();
  const version = rule(css, ".version-line a");
  assert.ok(version, "the version line has no rule");
  assert.match(version[1], /overflow:\s*hidden/);
  assert.match(version[1], /text-overflow:\s*ellipsis/);
  const pill = rule(css, ".pill");
  assert.ok(pill, "the pill rule is missing");
  assert.match(pill[1], /white-space:\s*nowrap/);
  const tablePill = rule(css, ".table .pill");
  assert.ok(tablePill, "table pills have no minimum width");
  assert.match(tablePill[1], /min-width:/);
  const tool = rule(css, ".tool");
  assert.ok(tool, "the tool rule is missing");
  assert.match(tool[1], /white-space:\s*nowrap/);
});

test("dynamic labels are in both catalogs", () => {
  const de = catalog("de");
  const en = catalog("en");
  for (const key of ["manual", "schedule", "hook", "workflow", "relaunch", "ansible", "opentofu", "shell", "once", "hourly", "daily", "weekly", "monthly"]) {
    assert.ok(de[key] && de[key].trim(), `${key} is missing from de.json`);
    assert.ok(en[key] && en[key].trim(), `${key} is missing from en.json`);
  }
  assert.equal(de.schedule, "Zeitplan");
  assert.equal(en.schedule, "Schedule");
});
