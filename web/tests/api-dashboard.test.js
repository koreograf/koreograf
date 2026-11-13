"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

test("the import notice lists the missing paths", async () => {
  const { importNoticeHtml } = await load("features/dashboard.js");
  const notice = importNoticeHtml({ missing_paths: ["/home/user/projects/demo"] });
  assert.match(notice, /imported_paths_need_a_mount_or_a_change/);
  assert.match(notice, /\/home\/user\/projects\/demo/);
  assert.equal(importNoticeHtml({ missing_paths: [] }), "");
});

test("the dashboard page shows the empty state and the cache numbers", async () => {
  const { dashboardRange, dashboardTabs, dashboardBody } = await load("features/dashboard.js");
  location.hash = "#/";
  assert.equal(dashboardRange(), "1", "the default range is not today");
  location.hash = "#/?tab=dashboard&days=7";
  assert.equal(dashboardRange(), "7", "the range did not come from the address");

  const tabs = dashboardTabs("overview");
  assert.match(tabs, /role="tablist"/);
  assert.match(tabs, /id="tab-overview" aria-selected="true"/);
  assert.match(tabs, /id="tab-dashboard" aria-selected="false"/);

  const empty = dashboardBody(
    { series: [{ date: "2026-11-05", started: 0, successful: 0, failed: 0, canceled: 0 }], cache: { collections: 0, roles: 0, tofu_bytes: 0 } },
    "1",
  );
  assert.match(empty, /no_jobs_in_this_range/);
  assert.match(empty, /data-dashboard-range="1" aria-pressed="true"/);
  assert.match(empty, /data-dashboard-range="7" aria-pressed="false"/);
  assert.match(empty, /data-dashboard-range="14" aria-pressed="false"/);

  const filled = dashboardBody(
    { series: [{ date: "2026-11-05", started: 3, successful: 1, failed: 1, canceled: 1 }], cache: { collections: 2, roles: 4, tofu_bytes: 2048 } },
    "7",
  );
  assert.match(filled, /jobs_finished_per_day/);
  assert.match(filled, /jobs_started_per_day/);
  assert.match(filled, /<span class="small muted">collections<\/span><strong>2<\/strong>/);
  assert.match(filled, /<span class="small muted">roles<\/span><strong>4<\/strong>/);
  assert.match(filled, /<span class="small muted">opentofu<\/span><strong>2\.0 KiB<\/strong>/);
  assert.doesNotMatch(filled.slice(filled.indexOf("dependency_cache")), /class="chart"/);
});

test("the preflight notice names the missing tool and runtime", async () => {
  const { preflightNotices } = await load("core/format.js");
  const cases = [
    [null, []],
    [{ ok: true, missing: [] }, []],
    [{ ok: false, missing: [{ kind: "tool", tool: "ansible-playbook", template_tool: "ansible", reason: "missing" }] }, ["preflight_tool_missing"]],
    [{ ok: false, missing: [{ kind: "runtime", tool: "docker/podman", template_tool: "", reason: "missing" }, { kind: "tool", tool: "ansible-playbook", template_tool: "ansible", reason: "missing" }] }, ["preflight_runtime_missing", "preflight_no_runtime_and_tool_missing"]],
    [{ ok: false, missing: [{ kind: "tool", tool: "tofu", template_tool: "opentofu", reason: "version", version: "1.5.0", minimum: "1.6" }] }, ["preflight_tool_too_old"]],
    [{ ok: false, missing: [{ kind: "tool", tool: "sh", template_tool: "shell", reason: "timeout" }] }, ["preflight_tool_unusable"]],
    [{ ok: false, missing: [{ kind: "project", tool: "git", template_tool: "", reason: "missing" }] }, ["preflight_project_missing"]],
    [{ ok: false, missing: [{ kind: "project", tool: "git", template_tool: "", reason: "failed" }] }, ["preflight_project_unusable"]],
    [{ ok: false, missing: [{ kind: "tool", tool: "ssh", template_tool: "ansible", reason: "missing" }] }, ["preflight_tool_missing"]],
  ];
  for (const [report, want] of cases) {
    assert.deepEqual(preflightNotices(report).map((item) => item.key), want);
  }
});

test("the version line links the repository and keeps the commit", async () => {
  const state = await load("core/state.js");
  const { shell, versionSlot, loadVersion } = await load("core/app.js");
  state.setMe({ is_admin: false, display_name: "Admin", auth_source: "local", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  shell("Overview", "");
  assert.match(state.app.innerHTML, /data-version/, "the page shell does not render the build line");
  const slot = versionSlot();
  assert.match(slot, /https:\/\/github\.com\/koreograf\/koreograf/);
  assert.match(slot, /target="_blank"/);
  assert.match(slot, /rel="noopener noreferrer"/);
  const paths = [];
  globalThis.fetch = async (path) => {
    paths.push(path);
    return { ok: true, json: async () => ({ version: "1.2.3", commit: "abc1234def" }) };
  };
  state.setVersionRequested(false);
  await loadVersion();
  assert.deepEqual(paths, ["/api/v1/version"]);
  assert.equal(state.buildVersion, "1.2.3");
  assert.equal(state.buildCommit, "abc1234def");
});

test("a rejected sign in keeps the username and focuses the password", async () => {
  const { renderLogin } = await load("core/app.js");
  renderLogin("invalid_username_or_password", "bad", null, "ada");
  const form = document.querySelector("#login-form");
  assert.ok(form, "the login form is missing");
  assert.equal(form.querySelector("[name=username]").value, "ada");
  assert.equal(document.activeElement, form.querySelector("[name=password]"), "the password field did not take focus");
  assert.match(form.innerHTML, /invalid_username_or_password/);
});

test("the language switch offers both catalogs with their flags", async () => {
  const { LANGUAGES, FLAG_SVG } = await load("core/i18n.js");
  assert.deepEqual(LANGUAGES.map((item) => [item.id, item.code]), [["de", "GER"], ["en", "ENG"]]);
  assert.equal(LANGUAGES.find((item) => item.id === "en").country, "gb");
  assert.match(FLAG_SVG.gb, /<svg/);
  assert.match(FLAG_SVG.de, /<svg/);
});

test("the verbosity labels cover every level", async () => {
  const { verbosityLabel } = await load("core/format.js");
  assert.deepEqual([0, 1, 2, 3, 4].map(verbosityLabel), [
    "0_normal",
    "1_verbose",
    "2_more_verbose",
    "3_debug",
    "4_connection_debug",
  ]);
});
