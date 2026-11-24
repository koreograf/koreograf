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

test("the overview kpi tiles are links with warning tones", async () => {
  const { kpiRow, waitedText } = await load("features/dashboard.js");
  const oldest = new Date(Date.now() - 2 * 60 * 60 * 1000 - 5 * 60 * 1000).toISOString();
  assert.match(waitedText(oldest), /^2h 0[45]m$/);

  const calm = kpiRow({
    glance: {
      jobs: { running: 0, pending: 0, failed_24h: 0, successful_24h: 0, finished_24h: 0 },
      concurrency: { team_running: 0, team_limit: 2 },
      hosts: { failed: 0, stale: 0 },
    },
  });
  assert.match(calm, /data-kpi="running"[^>]*href="#\/jobs\?status=running"/);
  assert.match(calm, /data-kpi="pending"[^>]*href="#\/jobs\?status=pending"/);
  assert.match(calm, /data-kpi="failed-24h"[^>]*href="#\/jobs\?status=failed"/);
  assert.match(calm, /data-kpi="success-rate"[^>]*href="#\/jobs\?status=successful"/);
  assert.match(calm, /data-kpi="problem-hosts"[^>]*href="#\/inventories"/);
  assert.match(calm, /<strong class="kpi-value">0 \/ 2<\/strong>/);
  assert.match(calm, /<strong class="kpi-value">—<\/strong>/, "an empty window has no success rate");
  assert.match(calm, /no_pending_job/);
  assert.doesNotMatch(calm, /class="kpi warn"/, "an idle team gets a warning tone");
  assert.doesNotMatch(calm, /class="kpi bad"/, "an idle team gets a bad tone");

  const busy = kpiRow({
    glance: {
      jobs: { running: 2, pending: 3, oldest_pending_at: oldest, failed_24h: 4, successful_24h: 6, finished_24h: 10 },
      concurrency: { team_running: 2, team_limit: 2 },
      hosts: { failed: 2, stale: 1 },
    },
  });
  assert.match(busy, /class="kpi warn" data-kpi="running"/, "a full team limit has no warning tone");
  assert.match(busy, /class="kpi bad" data-kpi="failed-24h"/, "failed jobs have no bad tone");
  assert.match(busy, /class="kpi bad" data-kpi="problem-hosts"/, "problem hosts have no bad tone");
  assert.match(busy, /<strong class="kpi-value">2 \/ 2<\/strong>/);
  assert.match(busy, /<strong class="kpi-value">60%<\/strong>/);
  assert.match(busy.slice(busy.indexOf('data-kpi="problem-hosts"')), /<strong class="kpi-value">3<\/strong>/, "stale hosts are missing from the problem count");
  assert.match(busy, /oldest_pending_waited/);
  assert.match(busy, /problem_hosts_detail/);
});

test("the overview shares one attention list and drops the cache card", async () => {
  const state = await load("core/state.js");
  const { overviewGlanceHtml, attentionItems, attentionHtml } = await load("features/dashboard.js");
  state.setMe({ is_admin: false, display_name: "Operator", auth_source: "local", teams: [{ id: "t1", name: "Team", role: "operator" }] });
  state.setTeamId("t1");
  const old = new Date(Date.now() - 40 * 60 * 1000).toISOString();
  const now = new Date().toISOString();
  const data = {
    cache_bytes: 2048,
    jobs: Array.from({ length: 8 }, (_, index) => ({
      id: `recent-${index}`,
      name: `Recent ${index}`,
      status: "successful",
      created_at: now,
    })),
    glance: {
      jobs: {
        running: 1,
        pending: 1,
        oldest_pending_at: old,
        failed_24h: 1,
        successful_24h: 8,
        finished_24h: 9,
        items: [
          { id: "still-pending", name: "Waiting job", status: "pending", created_at: old },
          ...Array.from({ length: 7 }, (_, index) => ({ id: `recent-${index}`, name: `Recent ${index}`, status: "successful", created_at: now })),
        ],
        failed_items: [{ id: "failed-hidden", name: "Failed outside recents", status: "failed", created_at: old, finished_at: old }],
      },
      concurrency: { team_running: 1, team_limit: 2, global_running: 3, global_limit: 4 },
      schedules: {
        next_run_at: "2026-12-01T06:00:00Z",
        missed_count: 1,
        items: [{ id: "schedule-1", name: "Nightly", target_name: "Healthcheck", next_run_at: "2026-12-01T06:00:00Z", missed_at: old }],
      },
      hosts: {
        failed: 2,
        stale: 1,
        stale_after_days: 7,
        items: [{ inventory_id: "inv-1", inventory_name: "Local", host: "web 1", result: "failed", ts: old }],
      },
      syncs: {
        projects: [{ id: "project-1", name: "Demo", kind: "git", last_sync_at: old }],
        inventories: [{ id: "inv-1", name: "Local", kind: "project", last_sync_at: old }],
      },
      workflows: {
        running: 1,
        failed: 1,
        failed_window_hours: 24,
        items: [{ id: "run-1", workflow_id: "workflow-1", name: "Self-Heal", status: "failed", created_at: old, finished_at: old }],
      },
    },
  };
  const html = overviewGlanceHtml(data);
  assert.doesNotMatch(html, /data-glance-card="cache"/, "the overview still shows the dependency cache");
  assert.doesNotMatch(html, /cache_bytes/, "the overview renders the cache number");
  assert.match(html, /class="kpi-row"/);
  assert.match(html, /class="overview-main"/);
  assert.match(html, /data-attention/);
  assert.match(html, /data-recent-activity/);
  const items = attentionItems(data);
  assert.deepEqual(items.map((item) => /data-attention-kind="([^"]+)"/.exec(item)[1]), [
    "job", "workflow", "schedule", "project", "inventory", "host", "pending",
  ], "the attention list lost its order");
  assert.match(html, /Failed outside recents/, "a failed job outside the eight recents is missing");
  assert.match(html, /data-attention-relaunch="failed-hidden"/);
  assert.match(html, /href="#\/jobs\/failed-hidden"/);
  assert.match(html, /data-attention-sync="\/api\/v1\/projects\/project-1\/sync"/);
  assert.match(html, /data-attention-sync="\/api\/v1\/inventories\/inv-1\/sync"/);
  assert.match(html, /href="#\/runs\/run-1"/);
  assert.match(html, /href="#\/schedules\?edit=schedule-1"/);
  assert.match(html, /href="#\/inventories\/inv-1\/hosts\/web%201"/);
  assert.match(html, /data-attention-kind="pending"/);
  const recentStart = html.indexOf("data-recent-activity");
  const recent = html.slice(recentStart, html.indexOf("</section>", recentStart));
  assert.equal((recent.match(/<tr>/g) || []).length, 6, "the recent card does not hold a header and five jobs");

  const empty = attentionHtml({ glance: {} });
  assert.match(empty, /data-attention-clear/);
  assert.match(empty, /attention_all_clear/);
  assert.doesNotMatch(empty, /attention-item/, "an empty attention list still renders rows");
});

test("the attention list falls back to the oldest wait when the queue is full", async () => {
  const state = await load("core/state.js");
  const { attentionItems } = await load("features/dashboard.js");
  state.setMe({ is_admin: false, display_name: "Operator", auth_source: "local", teams: [{ id: "t1", name: "Team", role: "operator" }] });
  state.setTeamId("t1");
  const old = new Date(Date.now() - 90 * 60 * 1000).toISOString();
  const recent = new Date().toISOString();
  const running = Array.from({ length: 8 }, (_, index) => ({
    id: `running-${index}`,
    name: `Running ${index}`,
    status: "running",
    created_at: recent,
  }));
  const items = attentionItems({
    glance: {
      jobs: { running: 8, pending: 2, oldest_pending_at: old, items: running },
    },
  });
  assert.equal(items.length, 1, "the full queue hid the waiting job");
  assert.match(items[0], /data-attention-kind="pending"/);
  assert.match(items[0], /href="#\/jobs\?status=pending"/);
  assert.match(items[0], /oldest_pending_job/);
});

test("the overview detail cards keep an empty state", async () => {
  const { overviewGlanceHtml } = await load("features/dashboard.js");
  const html = overviewGlanceHtml({
    jobs: [],
    glance: {
      jobs: { running: 0, pending: 0, items: [], failed_items: [] },
      concurrency: { team_running: 0, team_limit: 2, global_running: 0, global_limit: 4 },
      schedules: { missed_count: 0, items: [] },
      hosts: { failed: 0, stale: 0, stale_after_days: 7, items: [] },
      syncs: { projects: [], inventories: [] },
      workflows: { running: 0, failed: 0, failed_window_hours: 24, items: [] },
    },
  });
  for (const key of ["no_pending_job", "no_upcoming_schedules", "no_problem_hosts", "no_failed_syncs", "no_active_workflow_runs"]) {
    assert.match(html, new RegExp(key), `the empty state ${key} is missing`);
  }
  assert.doesNotMatch(html, /data-glance-card="queue"/, "the queue card came back");
});

test("the overview keeps the runner, runtime and directory line", async () => {
  const { overviewSystemLine } = await load("features/dashboard.js");
  const line = overviewSystemLine({ runner_mode: "local", runtime: null, ldap: false });
  assert.match(line, /data-overview-system/);
  assert.match(line, /runner/);
  assert.match(line, /no_container_runtime/);
  assert.match(line, /local_accounts/);
  assert.match(overviewSystemLine({ runner_mode: "docker", runtime: "podman", ldap: true }), /ldap_is_configured/);
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

test("the overview gives operators one primary launch action and a quiet test alert", async () => {
  const state = await load("core/state.js");
  const { overviewActions, quickLaunchHtml, webhookDeliveriesCard } = await load("features/dashboard.js");
  state.setMe({ is_admin: false, display_name: "Operator", auth_source: "local", teams: [{ id: "t1", name: "Team", role: "operator" }] });
  state.setTeamId("t1");
  const actions = overviewActions();
  assert.equal((actions.match(/class="btn primary"/g) || []).length, 1, "the header does not carry one primary action");
  assert.match(actions, /data-launch-template/);
  assert.match(actions, /launch_template/);
  assert.doesNotMatch(actions, /test-alert/);

  const webhook = webhookDeliveriesCard({
    hook_id: "hook-1",
    deliveries: [{ hook_name: "Deploy", status: "successful", summary: "ok" }],
  });
  assert.match(webhook, /class="btn" id="test-alert"/);
  assert.doesNotMatch(webhook, /btn primary/, "the test alert is still primary");

  const recent = Array.from({ length: 6 }, (_, index) => ({
    id: `tpl-${index}`,
    name: `Template ${index}`,
    tool: "shell",
    last_launched_at: new Date().toISOString(),
  }));
  const quick = quickLaunchHtml({ glance: { recent_templates: recent } });
  assert.match(quick, /data-quick-launch-card/);
  assert.equal((quick.match(/data-quick-launch-start=/g) || []).length, 5, "the quick launch card does not cap at five");
  assert.doesNotMatch(quick, /btn primary/, "a quick launch Start is primary");
  assert.match(quickLaunchHtml({ glance: { recent_templates: [] } }), /^$/);

  state.setMe({ is_admin: false, display_name: "Viewer", auth_source: "local", teams: [{ id: "t1", name: "Team", role: "viewer" }] });
  assert.equal(overviewActions(), "", "a viewer still gets a launch action");
  assert.equal(quickLaunchHtml({ glance: { recent_templates: recent } }), "", "a viewer still gets the quick launch card");
  assert.doesNotMatch(webhookDeliveriesCard({ hook_id: "hook-1", deliveries: [] }), /test-alert/);
});

test("the overview picker and a quick launch start open the shared launch dialog", async () => {
  const state = await load("core/state.js");
  const { overviewActions, quickLaunchHtml, bindOverviewActions, bindQuickLaunch } = await load("features/dashboard.js");
  const tick = () => new Promise((resolve) => setImmediate(resolve));
  state.setMe({ is_admin: false, display_name: "Operator", auth_source: "local", teams: [{ id: "t1", name: "Team", role: "operator" }] });
  state.setTeamId("t1");
  const template = {
    id: "tpl-echo",
    name: "Echo",
    tool: "shell",
    description: "",
    spec: { script: "true" },
    prompt_on_launch: false,
    prompt_fields: [],
    extra_vars: {},
  };
  const calls = [];
  globalThis.fetch = async (path) => {
    calls.push(path);
    const body = path === "/api/v1/templates" ? [template] : template;
    return { ok: true, status: 200, text: async () => JSON.stringify(body) };
  };
  const host = document.createElement("div");
  document.body.appendChild(host);
  host.innerHTML = `<div class="row">${overviewActions()}${quickLaunchHtml({ glance: { recent_templates: [{ id: template.id, name: template.name, tool: "shell" }] } })}</div>`;
  bindOverviewActions(host);
  bindQuickLaunch(host);

  host.querySelector("[data-launch-template]").dispatchEvent({ type: "click" });
  await tick();
  await tick();
  const picker = document.querySelector("#template-picker");
  assert.ok(picker, "the header action did not open the template picker");
  picker.querySelector("[data-pick-template]").dispatchEvent({ type: "click" });
  await tick();
  await tick();
  assert.equal(document.querySelector("#template-picker"), null, "the picker stayed open behind the launch dialog");
  assert.ok(document.querySelector("#launch-prompt"), "a picker option did not open the shared launch dialog");
  document.querySelector("#launch-prompt").remove();

  host.querySelector("[data-quick-launch-start]").dispatchEvent({ type: "click" });
  await tick();
  await tick();
  assert.ok(document.querySelector("#launch-prompt"), "a quick launch Start did not open the shared launch dialog");
  assert.deepEqual(calls, [
    "/api/v1/templates",
    "/api/v1/templates/tpl-echo",
  ]);
  document.querySelector("#launch-prompt").remove();
  host.remove();
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
