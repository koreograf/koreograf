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
  assert.match(importNoticeHtml({ missing_paths: ["<b>x</b>"] }), /&lt;b&gt;x&lt;\/b&gt;/);
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

test("a one-day series keeps the column narrow and the failed segment visible", async () => {
  const { dashboardBody } = await load("features/dashboard.js");
  const { stylesheet, rule } = require("./css.js");
  const html = dashboardBody(
    { series: [{ date: "2026-11-18", started: 16, successful: 15, failed: 1, canceled: 0 }], cache: {} },
    "1",
  );
  const chart = html.slice(html.indexOf('class="chart"'));
  assert.match(chart, /class="chart-seg successful" style="flex-grow:15"/);
  assert.match(chart, /class="chart-seg failed" style="flex-grow:1"/);
  assert.match(chart, /<div class="chart-bar" style="height:100%">/);

  const css = stylesheet();
  const day = rule(css, ".chart-day");
  assert.ok(day, "the chart day rule is missing");
  assert.match(day[1], /max-width:\s*64px;/, "a single day still fills the card");
  assert.match(day[1], /flex:\s*1 1 0;/, "the day column does not share the row");
  assert.match(rule(css, ".chart")[1], /justify-content:\s*center;/, "a narrow day is not centered");
  assert.match(rule(css, ".chart-seg")[1], /min-height:\s*4px;/, "a minority segment can collapse");
});

test("the seven and fourteen day series keep every day and the empty ones", async () => {
  const { dashboardBody } = await load("features/dashboard.js");
  const series = (count) => Array.from({ length: count }, (_, index) => ({
    date: `2026-11-${String(index + 1).padStart(2, "0")}`,
    started: index === 2 ? 0 : index + 1,
    successful: index === 2 ? 0 : index + 1,
    failed: index % 5 === 0 ? 1 : 0,
    canceled: 0,
  }));
  for (const count of [7, 14]) {
    const html = dashboardBody({ series: series(count), cache: {} }, String(count));
    const charts = html.slice(html.indexOf('class="chart"'), html.indexOf("dependency_cache"));
    assert.equal((charts.match(/class="chart-day"/g) || []).length, count * 2, `the ${count} day view lost a day`);
    assert.equal((charts.match(/<div class="chart-bars"><\/div>/g) || []).length, 2, `the ${count} day view filled an empty day`);
    assert.match(charts, /class="chart-seg failed"/, `the ${count} day view lost the failed segment`);
    assert.match(charts, /class="chart-bar" style="height:\d+%"/);
  }
});

test("the overview glance links every queue, schedule, host, sync and run", async () => {
  const { overviewGlanceHtml, waitedText } = await load("features/dashboard.js");
  const oldest = new Date(Date.now() - 2 * 60 * 60 * 1000 - 5 * 60 * 1000).toISOString();
  assert.match(waitedText(oldest), /^2h 0[45]m$/);
  const html = overviewGlanceHtml({
    cache_bytes: 2048,
    glance: {
      jobs: {
        running: 1,
        pending: 2,
        oldest_pending_at: oldest,
        items: [
          { id: "job-1", name: "Waiting job", status: "pending", created_at: oldest },
          { id: "job-2", name: "Running job", status: "running", created_at: oldest },
        ],
      },
      concurrency: { team_running: 1, team_limit: 2, global_running: 3, global_limit: 4 },
      schedules: {
        next_run_at: "2026-12-01T06:00:00Z",
        missed_count: 1,
        items: [{ id: "schedule-1", name: "Nightly", target_name: "Healthcheck", next_run_at: "2026-12-01T06:00:00Z", missed_at: "2026-11-30T06:00:00Z" }],
      },
      hosts: {
        failed: 2,
        stale: 1,
        stale_after_days: 7,
        items: [{ inventory_id: "inv-1", inventory_name: "Local", host: "web 1", result: "failed", ts: oldest }],
      },
      syncs: {
        projects: [{ id: "project-1", name: "Demo", last_sync_at: oldest }],
        inventories: [{ id: "inv-1", name: "Local", last_sync_at: oldest }],
      },
      workflows: {
        running: 1,
        failed: 1,
        items: [{ id: "run-1", workflow_id: "workflow-1", name: "Self-Heal", status: "failed", created_at: oldest }],
      },
    },
  });
  for (const id of ["cache", "queue", "concurrency", "schedules", "hosts", "syncs", "workflows"]) {
    assert.match(html, new RegExp(`data-glance-card="${id}"`), `the ${id} card is missing`);
  }
  assert.match(html, /data-glance="queue-running"><strong>1<\/strong>/);
  assert.match(html, /data-glance="queue-pending"><strong>2<\/strong>/);
  assert.match(html, /oldest_pending_waited/);
  assert.match(html, /href="#\/jobs\/job-1"/);
  assert.match(html, /href="#\/jobs\?status=pending"/);
  assert.match(html, /href="#\/jobs\?status=running"/);
  assert.match(html, /data-glance="team-slots"><strong>1 \/ 2<\/strong>/);
  assert.match(html, /data-glance="global-slots"><strong>3 \/ 4<\/strong>/);
  assert.match(html, /href="#\/schedules"/);
  assert.match(html, /schedule_missed_state/);
  assert.match(html, /href="#\/inventories\/inv-1\/hosts\/web%201"/);
  assert.match(html, /hosts_stale_after_days/);
  assert.match(html, /href="#\/projects\/project-1"/);
  assert.match(html, /href="#\/inventories\/inv-1"/);
  assert.match(html, /href="#\/runs\/run-1"/);
  assert.match(html, /href="#\/workflows\/workflow-1"/);
});

test("the overview glance keeps a specific empty state for every list", async () => {
  const { overviewGlanceHtml } = await load("features/dashboard.js");
  const html = overviewGlanceHtml({
    cache_bytes: 0,
    glance: {
      jobs: { running: 0, pending: 0, items: [] },
      concurrency: { team_running: 0, team_limit: 2, global_running: 0, global_limit: 4 },
      schedules: { missed_count: 0, items: [] },
      hosts: { failed: 0, stale: 0, stale_after_days: 7, items: [] },
      syncs: { projects: [], inventories: [] },
      workflows: { running: 0, failed: 0, items: [] },
    },
  });
  for (const key of ["no_pending_job", "no_open_jobs", "no_upcoming_schedules", "no_problem_hosts", "no_failed_syncs", "no_active_workflow_runs"]) {
    assert.match(html, new RegExp(key), `the empty state ${key} is missing`);
  }
  assert.doesNotMatch(html, /<table/, "an empty glance still renders a table");
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
