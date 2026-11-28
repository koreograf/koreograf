"use strict";

const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { mediaBlock, rule, stylesheet } = require("./css.js");
const { catalog } = require("./catalog.js");

require("./test-dom.js").install();

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

test("the shell wraps headers and uses the dynamic viewport", () => {
  const css = stylesheet();
  for (const selector of [".top", ".spread"]) {
    assert.match(rule(css, selector)[1], /flex-wrap:\s*wrap/, `${selector} does not wrap`);
  }
  assert.match(rule(css, ".shell")[1], /min-height:\s*100dvh/, "the shell does not use the dynamic viewport");
  assert.match(rule(css, ".login-wrap")[1], /min-height:\s*100dvh/, "the login page does not use the dynamic viewport");
  assert.match(rule(css, ".side")[1], /safe-area-inset-bottom/, "the drawer ignores the bottom safe area");
  assert.match(rule(css, ".main")[1], /safe-area-inset-bottom/, "the main column ignores the bottom safe area");
});

test("the viewport meta keeps device width", () => {
  const html = readFileSync(join(__dirname, "..", "index.html"), "utf8");
  assert.match(html, /<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">/);
});

test("the drawer toggle and nav links are touch sized", () => {
  const css = stylesheet();
  const narrow = mediaBlock(css, "@media (max-width: 900px)");
  const toggle = /\.nav-toggle\s*\{([^}]*)\}/.exec(narrow);
  assert.ok(toggle, "the narrow drawer toggle is missing");
  assert.match(toggle[1], /width:\s*44px/);
  assert.match(toggle[1], /height:\s*44px/);
  const coarse = mediaBlock(css, "@media (pointer: coarse)");
  assert.match(coarse, /\.nav a\s*\{[^}]*min-height:\s*44px/, "nav links are below 44 px");
});

test("the shared table labels every cell from the header row", async () => {
  const { table } = await load("core/dom.js");
  const html = table(
    ["Name", "Tool", ""],
    ['<tr><td>one</td><td class="tool">shell</td><td><button class="btn compact" type="button">Edit</button></td></tr>'],
  );
  assert.match(html, /<td data-label="Name">one<\/td>/);
  assert.match(html, /<td class="tool" data-label="Tool">shell<\/td>/);
  assert.match(html, /<td><button/);
  const kept = table(["Name"], ['<tr><td data-label="Custom">one</td></tr>']);
  assert.match(kept, /data-label="Custom"/);
  assert.doesNotMatch(kept, /data-label="Name"/);
  for (const span of ['colspan="2"', "colspan='2'", "colspan=2"]) {
    const merged = table(["First", "Second", "Third"], [`<tr><td ${span}>wide</td><td>last</td></tr>`]);
    assert.match(merged, /<td colspan=['"]?2['"]? data-label="First">wide<\/td>/, `colspan ${span} lost its label`);
    assert.match(merged, /<td data-label="Third">last<\/td>/, `colspan ${span} shifted the next label`);
  }
  const tricky = table(["Name", "Note"], ['<tr><td title="a > b">one</td><td>two</td></tr>']);
  assert.match(tricky, /<td title="a > b" data-label="Name">one<\/td>/);
  assert.match(tricky, /<td data-label="Note">two<\/td>/);
  const nested = table(["Name", "Note", "Last"], ['<tr><td>one</td><td><table><tr><td>inner</td></tr></table></td><td>last</td></tr>']);
  assert.match(nested, /<td data-label="Name">one<\/td>/);
  assert.match(nested, /<td data-label="Note"><table><tr><td>inner<\/td><\/tr><\/table><\/td>/);
  assert.match(nested, /<td data-label="Last">last<\/td>/);
});

test("the narrow card rows cover schedules and inventory hosts", () => {
  const block = mediaBlock(stylesheet(), "@media (max-width: 640px)");
  assert.doesNotMatch(block, /:not\(\.schedule-table\)/, "the schedule table stays out of the card layout");
  assert.doesNotMatch(block, /:not\(\.inventory-hosts-table\)/, "the host table stays out of the card layout");
  assert.match(block, /min-width:\s*0/, "the wide tables keep their minimum width on phones");
  assert.match(block, /\.table-wrap \.table td\[data-label\]::before/, "the card rows have no labels");
});

test("schedule and inventory tables label their stacked rows", async () => {
  const i18n = await load("core/i18n.js");
  i18n.setCatalog("de", catalog("de"));
  i18n.setCatalog("en", catalog("en"));
  await i18n.setLang("en");
  const en = catalog("en");
  const { scheduleTable } = await load("features/schedules.js");
  const schedule = scheduleTable([
    {
      id: "s1",
      name: "Nightly",
      enabled: true,
      frequency: "daily",
      start_date: "2026-11-20",
      start_time: "08:00",
      timezone: "UTC",
      end_kind: "never",
      target_id: "t1",
      target_kind: "template",
      target_name: "Healthcheck",
    },
  ], true, "empty");
  for (const key of ["name", "schedule_target", "repeat", "start_time", "next_run", "ends", "schedule_active"]) {
    assert.ok(schedule.includes(`data-label="${en[key]}"`), `the schedule table lost the ${key} label`);
  }

  const inventories = await load("features/inventories.js");
  const hosts = inventories.inventoryHostTable([{ name: "web01", vars: { ansible_host: "10.0.0.1" }, groups: ["web"] }], { sort: "name" }, "inv1");
  for (const key of ["name", "address", "groups"]) {
    assert.ok(hosts.includes(`data-label="${en[key]}"`), `the host table lost the ${key} label`);
  }
  const groups = inventories.inventoryGroupTable([{ name: "web", host_count: 1, children: [] }], "inv1");
  for (const key of ["name", "hosts", "groups"]) {
    assert.ok(groups.includes(`data-label="${en[key]}"`), `the group table lost the ${key} label`);
  }
  const history = inventories.hostHistoryTable([{ time: "2026-11-20T08:00:00Z", playbook: "site.yml", role: "r", result: "successful", job_id: "j1", job_name: "Job" }]);
  for (const key of ["time", "playbook", "role", "run_result", "job"]) {
    assert.ok(history.includes(`data-label="${en[key]}"`), `the history table lost the ${key} label`);
  }
});

test("coarse pointer controls reach 44 px", () => {
  const coarse = mediaBlock(stylesheet(), "@media (pointer: coarse)");
  for (const selector of [
    ".btn.compact",
    ".switch",
    ".chip-remove",
    ".inspector-toggle",
    ".canvas-controls .btn",
    ".node-menu-button",
    ".wf-port-add",
    ".lang-menu [role=\"option\"]",
    ".dialog-tab",
    ".vault-option",
    ".account-link",
    ".row:has(> input[type=\"checkbox\"])",
    ".field.check",
  ]) {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    assert.match(coarse, new RegExp(`${escaped}\\s*\\{[^}]*min-height:\\s*44px`), `${selector} is below 44 px`);
  }
  for (const selector of [".btn.compact", ".switch", ".chip-remove", ".inspector-toggle", ".canvas-controls .btn", ".node-menu-button", ".wf-port-add"]) {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    assert.match(coarse, new RegExp(`${escaped}\\s*\\{[^}]*min-width:\\s*44px`), `${selector} is below 44 px`);
  }
  assert.match(coarse, /select\s*\{[^}]*min-height:\s*44px/, "form controls stay small on touch screens");
  assert.match(coarse, /textarea\s*\{[^}]*min-height:\s*88px/, "the touch rule shrinks a text area");
  assert.match(coarse, /\.overlay-card\.dialog textarea\s*\{[^}]*min-height:\s*88px/, "dialog text areas lose their height on touch screens");
});

test("the narrow key value list stacks its label above the value", () => {
  const block = mediaBlock(stylesheet(), "@media (max-width: 640px)");
  assert.match(block, /\.kv[^{}]*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)/, "the key value list keeps two columns");
  assert.match(block, /\.kv\.readonly-kv\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)/, "the inspector key value list keeps two columns");
});

test("a fullscreen overlay keeps the shared chrome and scrolls its body", () => {
  const css = stylesheet();
  const full = mediaBlock(css, "@media (max-width: 600px)");
  assert.match(full, /\.overlay-card[^}]*height:\s*100dvh/, "the fullscreen overlay does not use the dynamic viewport");
  assert.match(full, /safe-area-inset-bottom/, "the fullscreen overlay ignores the safe area");
  assert.match(rule(css, ".dialog-body")[1], /overflow:\s*auto/, "the dialog body does not scroll");
  assert.match(rule(css, ".dialog-form")[1], /display:\s*flex/, "the edit chrome is not the shared form layout");
});

test("the assistant sheet is fullscreen at 640 px and keeps the safe area", () => {
  const block = mediaBlock(stylesheet(), "@media (max-width: 640px)");
  assert.match(block, /\.assistant-panel\s*\{[^}]*position:\s*fixed/, "the assistant does not become a fixed sheet");
  assert.match(block, /\.assistant-panel\s*\{[^}]*height:\s*100dvh/, "the assistant sheet does not use the dynamic viewport");
  assert.match(block, /\.assistant-panel\s*\{[^}]*padding:\s*env\(safe-area-inset-top\)[\s\S]*env\(safe-area-inset-bottom\)/, "the assistant sheet ignores the safe area");
  assert.match(block, /\.assistant-dock\s*\{[^}]*env\(safe-area-inset-bottom\)/, "the launcher ignores the bottom safe area");
});

test("the dashboard writes the chart count into the visible day", async () => {
  const i18n = await load("core/i18n.js");
  i18n.setCatalog("de", catalog("de"));
  i18n.setCatalog("en", catalog("en"));
  await i18n.setLang("en");
  const { dashboardBody } = await load("features/dashboard.js");
  const en = catalog("en");
  const html = dashboardBody({ series: [{ date: "2026-11-20", started: 9, successful: 3, failed: 1, canceled: 0 }], cache: {} }, "1");
  assert.match(html, new RegExp(en.jobs_per_hour));
  assert.match(html, /class="chart-day-total"[^>]*>9</, "the day total adds started and finished");
  assert.match(html, /data-chart-day data-chart-target="[^"]+" data-chart-title="[^"]*Failed 1[^"]*"/, "the status split is not reachable on tap");
  assert.match(html, /class="chart-swatch canceled"/, "the legend lost its swatches");
  assert.match(stylesheet(), /\.chart-day-total\s*\{[^}]*font-size:\s*11px/, "the day total does not fit a narrow chart");
  assert.match(rule(stylesheet(), ".chart-day")[1], /cursor:\s*pointer/, "the day detail has no tap affordance");
});

test("the workflow run lines reflow into labelled rows", async () => {
  const { readFileSync } = require("node:fs");
  const source = readFileSync(join(__dirname, "..", "js", "features", "workflows", "run.js"), "utf8");
  for (const key of ["start", "name", "status", "end", "duration"]) {
    assert.match(source, new RegExp(`data-label="\\$\\{h\\("${key}"\\)\\}"`), `the run line lost the ${key} label`);
  }
  const block = mediaBlock(stylesheet(), "@media (max-width: 640px)");
  assert.match(block, /\.run-line\s*\{[^}]*min-width:\s*0/, "the run line keeps its wide minimum");
  assert.match(block, /\.run-line\.head\s*\{\s*display:\s*none/, "the run line keeps its desktop header");
  assert.match(block, /\.run-line > span\[data-label\]::before/, "the narrow run line has no labels");
});

async function mountJobLog(matches) {
  const state = await load("core/state.js");
  const previous = window.matchMedia;
  window.matchMedia = (query) => ({ media: query, matches, addEventListener() {}, removeEventListener() {} });
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  const job = {
    id: "j1",
    name: "Job",
    tool: "shell",
    status: "successful",
    trigger: "manual",
    launched_by: "admin",
    created_at: "2026-01-01T00:00:00Z",
    events: [{ id: 1, stream: "stdout", line: "one long line" }],
    parameters: null,
    extra_vars: {},
  };
  globalThis.fetch = async (path) => {
    let body = [];
    if (path.startsWith("/api/v1/jobs/")) body = job;
    else if (path.startsWith("/api/v1/maintenance")) body = { active: false };
    else if (path.includes("/version")) body = { version: "1", commit: "abcdef" };
    else if (path.includes("/assistant")) body = { enabled: false };
    return { ok: true, status: 200, text: async () => JSON.stringify(body) };
  };
  const router = await load("core/router.js");
  await load("features/jobs.js");
  location.hash = "#/jobs/j1";
  await router.matchRoute("#/jobs/:id").handler({ id: "j1" });
  return () => { window.matchMedia = previous; };
}

test("the phone job log starts wrapped", async () => {
  const restore = await mountJobLog(true);
  try {
    assert.equal(document.querySelector("#log-wrap").checked, true, "the phone log does not start wrapped");
    assert.equal(document.querySelector("#log").classList.contains("wrap"), true, "the phone log ignores its first wrap state");
  } finally {
    restore();
  }
});

test("the desktop job log still starts unwrapped", async () => {
  const restore = await mountJobLog(false);
  try {
    assert.equal(document.querySelector("#log-wrap").checked, false, "the desktop log starts wrapped");
    assert.equal(document.querySelector("#log").classList.contains("wrap"), false, "the desktop log ignores its wrap state");
  } finally {
    restore();
  }
});

test("the workflow editor is view-only on a phone", async () => {
  const state = await load("core/state.js");
  const i18n = await load("core/i18n.js");
  i18n.setCatalog("de", catalog("de"));
  i18n.setCatalog("en", catalog("en"));
  await i18n.setLang("en");
  state.setMe({ is_admin: false, display_name: "Olga", teams: [{ id: "t1", name: "Plattform", role: "operator" }] });
  state.setTeamId("t1");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "[]" });
  const previous = window.matchMedia;
  window.matchMedia = (query) => ({ media: query, matches: true, addEventListener() {}, removeEventListener() {} });
  try {
    const editor = await load("features/workflows/editor.js");
    editor.mountEditor(
      { id: "w1", name: "Deploy", description: "", graph: { nodes: [{ id: "n1", template_id: "t1", name: "One", x: 20, y: 20 }], edges: [] } },
      [{ id: "t1", name: "Deploy", tool: "shell", spec: {} }],
      false,
      {},
      { inventories: [], credentials: [] },
      {},
    );
    const note = document.querySelector("#wf-desktop-note");
    assert.ok(note, "the phone editor has no desktop note");
    assert.match(note.innerHTML, /desktop/i);
    assert.match(document.querySelector("#wf-hint").innerHTML, /desktop/i);
    assert.match(document.querySelector(".top .pill.read-only").innerHTML, /read-only/i);
    for (const selector of ["#wf-save", "#wf-launch", "#wf-undo", "#wf-redo", "#wf-delete", "[data-node-menu]", "[data-add-port]", "#wf-add-first"]) {
      assert.equal(document.querySelector(selector), null, `the phone editor still shows ${selector}`);
    }
    assert.equal(document.querySelector("#wf-name").disabled, true, "the phone editor keeps the name field editable");
    assert.equal(document.querySelector("#wf-description").disabled, true, "the phone editor keeps the description field editable");
  } finally {
    window.matchMedia = previous;
  }
});

test("a phone viewer also gets the desktop note and no danger tab", async () => {
  const state = await load("core/state.js");
  state.setMe({ is_admin: false, display_name: "Vera", teams: [{ id: "t1", name: "Plattform", role: "viewer" }] });
  state.setTeamId("t1");
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "[]" });
  const previous = window.matchMedia;
  window.matchMedia = (query) => ({ media: query, matches: true, addEventListener() {}, removeEventListener() {} });
  try {
    const editor = await load("features/workflows/editor.js");
    editor.mountEditor(
      { id: "w2", name: "Read", description: "", graph: { nodes: [], edges: [] } },
      [],
      false,
      {},
      { inventories: [], credentials: [] },
      {},
    );
    assert.match(document.querySelector("#wf-desktop-note").innerHTML, /desktop/i, "a phone viewer has no desktop note");
    assert.match(document.querySelector("#wf-hint").innerHTML, /desktop/i, "the phone viewer hint still promises editing");
    assert.equal(document.querySelector("#wf-tab-danger"), null, "the phone viewer gets the danger tab");
  } finally {
    window.matchMedia = previous;
  }
});

test("the host list keeps a compact sort control on phones", () => {
  const source = readFileSync(join(__dirname, "..", "js", "features", "inventories.js"), "utf8");
  assert.match(source, /id="host-sort"/);
  assert.match(source, /hostSort\.addEventListener\("change"/);
  const css = stylesheet();
  assert.match(css, /\.host-sort-field\s*\{\s*display:\s*none/);
  const block = mediaBlock(css, "@media (max-width: 640px)");
  assert.match(block, /\.host-sort-field\s*\{\s*display:\s*grid/);
});
