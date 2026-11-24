import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { can, go, handle, markSaved, needTeam, route, shell } from "../core/app.js";
import { confirmDialog, openDialog } from "../core/dialog.js";
import { esc, h, table } from "../core/dom.js";
import { bytes, pill, preflightNoticeHtml, relativeTime, when } from "../core/format.js";
import { currentLanguage, t } from "../core/i18n.js";
import { registerRoute } from "../core/router.js";
import { relaunchJob } from "./jobs.js";
import { filterTemplates, openLaunchPrompt } from "./templates.js";

function onboardingCard() {
  const steps = [
    ["empty_projects", "#/projects?new=1", "new_project"],
    ["empty_inventories", "#/inventories?new=1", "new_inventory"],
    ["empty_templates", "#/templates?new=1", "new_template"],
    ["empty_jobs", "#/templates", "launch_a_template"],
  ];
  const items = steps.map(([text, href, label]) => `<li>
    <div><p>${h(text)}</p></div>
    ${can("operator") ? `<a class="btn" href="${href}">${h(label)}</a>` : ""}
  </li>`).join("");
  return `<section class="card onboarding" aria-labelledby="onboarding-title">
    <h2 id="onboarding-title">${h("get_started")}</h2>
    <p class="muted">${h("get_started_help")}</p>
    <ol class="onboarding-steps">${items}</ol>
  </section>`;
}

export function importNoticeHtml(importReport) {
  const paths = (importReport && importReport.missing_paths) || [];
  if (!paths.length) return "";
  return `<div class="banner warn"><p>${h("imported_paths_need_a_mount_or_a_change")}</p><ul>${paths.map((path) => `<li><code>${esc(path)}</code></li>`).join("")}</ul></div>`;
}

function dashboardTab() {
  const params = new URLSearchParams(location.hash.split("?")[1] || "");
  return params.get("tab") === "dashboard" ? "dashboard" : "overview";
}

export function dashboardRange() {
  const params = new URLSearchParams(location.hash.split("?")[1] || "");
  const days = params.get("days");
  return ["1", "7", "14"].includes(days) ? days : "1";
}

export function dashboardAddress(tab, range) {
  const params = new URLSearchParams();
  if (tab === "dashboard") {
    params.set("tab", "dashboard");
    if (range !== "1") params.set("days", range);
  }
  const query = params.toString();
  return query ? `#/?${query}` : "#/";
}

function shortDay(value) {
  const date = new Date(`${value}T00:00:00`);
  if (Number.isNaN(date.getTime())) return String(value || "");
  return date.toLocaleDateString(currentLanguage() === "en" ? "en-GB" : "de-DE", {
    day: "2-digit",
    month: "2-digit",
  });
}

export function dashboardTabs(tab) {
  const item = (id) => `<button class="btn pill${tab === id ? " primary" : ""}" type="button" role="tab" id="tab-${id}" aria-selected="${tab === id}" data-page-tab="${id}">${h(id)}</button>`;
  return `<div class="spread tab-bar"><div class="tabs" role="tablist" aria-label="${h("overview")} / ${h("dashboard")}">${item("overview")}${item("dashboard")}</div></div>`;
}

export function cacheClearButton(owner) {
  return owner ? `<button class="btn danger" type="button" data-cache-clear>${h("clear_cache")}</button>` : "";
}

export function waitedText(value) {
  if (!value) return "";
  const from = Date.parse(value);
  if (Number.isNaN(from)) return "";
  const secs = Math.max(0, Math.round((Date.now() - from) / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return mins % 60 ? `${hours}h ${String(mins % 60).padStart(2, "0")}m` : `${hours}h`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

function glanceStat(value, label, id) {
  return `<div class="stat"${id ? ` data-glance="${esc(id)}"` : ""}><strong>${esc(value)}</strong><span class="small muted">${h(label)}</span></div>`;
}

function healthyLine(id, label) {
  return `<p class="small muted glance-healthy" data-glance-healthy="${esc(id)}">${h(label)}</p>`;
}

function glanceCard(id, title, { links = [], body = "" } = {}) {
  const actions = links
    .map((link) => `<a class="btn compact" href="${esc(link.href)}">${h(link.label)}</a>`)
    .join("");
  return `<section class="card" data-glance-card="${esc(id)}">
    <div class="spread"><h2>${h(title)}</h2>${actions ? `<div class="row">${actions}</div>` : ""}</div>
    ${body}
  </section>`;
}

const PENDING_ATTENTION_MINUTES = 30;

export function kpiTile(id, label, value, { href, tone = "", hint = "" } = {}) {
  return `<a class="kpi${tone ? ` ${tone}` : ""}" data-kpi="${esc(id)}" href="${esc(href)}">
    <span class="kpi-label">${h(label)}</span>
    <strong class="kpi-value">${esc(value)}</strong>
    ${hint ? `<span class="kpi-hint">${hint}</span>` : ""}
  </a>`;
}

export function kpiRow(data) {
  const glance = (data && data.glance) || {};
  const jobs = glance.jobs || {};
  const use = glance.concurrency || {};
  const hosts = glance.hosts || {};
  const running = Number(use.team_running) || 0;
  const limit = Number(use.team_limit) || 0;
  const pending = Number(jobs.pending) || 0;
  const failed = Number(jobs.failed_24h) || 0;
  const successful = Number(jobs.successful_24h) || 0;
  const finished = Number(jobs.finished_24h) || 0;
  const hostFailed = Number(hosts.failed) || 0;
  const hostStale = Number(hosts.stale) || 0;
  const hostTotal = hostFailed + hostStale;
  const full = limit > 0 && running >= limit;
  const pendingHint = pending && jobs.oldest_pending_at
    ? h("oldest_pending_waited", { duration: waitedText(jobs.oldest_pending_at) })
    : h("no_pending_job");
  const rate = finished ? `${Math.round((successful / finished) * 100)}%` : "—";
  return `<div class="kpi-row" data-kpi-row>
    ${kpiTile("running", "running", `${running} / ${limit}`, { href: "#/jobs?status=running", tone: full ? "warn" : "", hint: h("team_slots") })}
    ${kpiTile("pending", "pending", String(pending), { href: "#/jobs?status=pending", hint: pendingHint })}
    ${kpiTile("failed-24h", "failed_24h", String(failed), { href: "#/jobs?status=failed", tone: failed ? "bad" : "", hint: h("last_24_hours") })}
    ${kpiTile("success-rate", "success_rate_24h", rate, { href: "#/jobs?status=successful", hint: h("success_rate_detail", { successful, finished }) })}
    ${kpiTile("problem-hosts", "problem_hosts_tile", String(hostTotal), { href: "#/inventories", tone: hostTotal ? "bad" : "", hint: h("problem_hosts_detail", { failed: hostFailed, stale: hostStale }) })}
  </div>`;
}

function attentionRow(kind, badge, name, href, time, actions) {
  return `<li class="attention-item" data-attention-kind="${esc(kind)}">
    ${badge}
    <div class="attention-body">
      <a href="${esc(href)}">${esc(name)}</a>
      <span class="small muted">${esc(time)}</span>
    </div>
    <div class="row attention-actions">${actions}</div>
  </li>`;
}

function syncRow(kind, entry) {
  const base = kind === "project" ? "projects" : "inventories";
  const href = `#/${base}/${encodeURIComponent(entry.id)}`;
  const syncable = kind === "project"
    ? entry.kind === "git"
    : ["project", "netbox"].includes(entry.kind);
  const open = `<a class="btn compact" href="${href}">${h("open")}</a>`;
  const sync = syncable && can("operator")
    ? `<button class="btn compact" type="button" data-attention-sync="/api/v1/${base}/${encodeURIComponent(entry.id)}/sync">${h("sync_again")}</button>`
    : "";
  return attentionRow(kind, pill("failed"), entry.name, href, relativeTime(entry.last_sync_at), `${open}${sync}`);
}

export function attentionItems(data) {
  const glance = (data && data.glance) || {};
  const jobs = glance.jobs || {};
  const items = [];
  for (const job of jobs.failed_items || []) {
    const href = `#/jobs/${encodeURIComponent(job.id)}`;
    const relaunch = can("operator")
      ? `<button class="btn compact" type="button" data-attention-relaunch="${esc(job.id)}">${h("relaunch")}</button>`
      : "";
    items.push(attentionRow("job", pill(job.status), job.name, href, relativeTime(job.finished_at || job.created_at), `<a class="btn compact" href="${href}">${h("open_log")}</a>${relaunch}`));
  }
  for (const run of (glance.workflows || {}).items || []) {
    if (run.status !== "failed") continue;
    const href = `#/runs/${encodeURIComponent(run.id)}`;
    items.push(attentionRow("workflow", pill(run.status), run.name, href, relativeTime(run.finished_at || run.created_at), `<a class="btn compact" href="${href}">${h("open")}</a>`));
  }
  for (const schedule of (glance.schedules || {}).items || []) {
    if (!schedule.missed_at) continue;
    const href = `#/schedules?edit=${encodeURIComponent(schedule.id)}`;
    items.push(attentionRow("schedule", `<span class="pill warn">${h("schedule_missed_state")}</span>`, schedule.name, href, relativeTime(schedule.missed_at), `<a class="btn compact" href="${href}">${h("edit")}</a>`));
  }
  const syncs = glance.syncs || {};
  for (const entry of syncs.projects || []) items.push(syncRow("project", entry));
  for (const entry of syncs.inventories || []) items.push(syncRow("inventory", entry));
  for (const host of (glance.hosts || {}).items || []) {
    const href = `#/inventories/${encodeURIComponent(host.inventory_id)}/hosts/${encodeURIComponent(host.host)}`;
    const badge = host.result === "failed" || host.result === "unreachable"
      ? pill(host.result)
      : `<span class="pill warn">${h("stale_hosts")}</span>`;
    items.push(attentionRow("host", badge, host.host, href, relativeTime(host.ts), `<a class="btn compact" href="${href}">${h("open")}</a>`));
  }
  const threshold = Date.now() - PENDING_ATTENTION_MINUTES * 60 * 1000;
  let pendingListed = false;
  for (const job of jobs.items || []) {
    if (job.status !== "pending") continue;
    const created = Date.parse(job.created_at);
    if (Number.isNaN(created) || created > threshold) continue;
    const href = `#/jobs/${encodeURIComponent(job.id)}`;
    items.push(attentionRow("pending", pill(job.status), job.name, href, relativeTime(job.created_at), `<a class="btn compact" href="${href}">${h("open_log")}</a>`));
    pendingListed = true;
  }
  // Eight running jobs can fill the busy list before an old pending job
  // appears. Fall back to the team-wide oldest wait so it still shows.
  const oldestPending = jobs.oldest_pending_at ? Date.parse(jobs.oldest_pending_at) : NaN;
  if (!pendingListed && (Number(jobs.pending) || 0) > 0 && !Number.isNaN(oldestPending) && oldestPending <= threshold) {
    const href = "#/jobs?status=pending";
    items.push(attentionRow("pending", pill("pending"), t("oldest_pending_job"), href, relativeTime(jobs.oldest_pending_at), `<a class="btn compact" href="${href}">${h("open")}</a>`));
  }
  return items;
}

export function attentionHtml(data) {
  const items = attentionItems(data);
  return `<section class="card" data-attention aria-labelledby="attention-title">
    <h2 id="attention-title">${h("needs_attention")}</h2>
    ${items.length
      ? `<ul class="attention-list">${items.join("")}</ul>`
      : `<p class="small muted attention-clear" data-attention-clear>${h("attention_all_clear")}</p>`}
  </section>`;
}

export function recentActivityCard(jobs) {
  const rows = (jobs || []).slice(0, 5).map((job) =>
    `<tr><td><a href="#/jobs/${esc(job.id)}">${esc(job.name)}</a></td><td>${pill(job.status)}</td><td class="small muted">${esc(relativeTime(job.created_at))}</td></tr>`
  );
  return `<section class="card" data-recent-activity>
    <div class="spread"><h2>${h("recent_activity")}</h2><a href="#/jobs">${h("all")}</a></div>
    ${table([t("name"), t("status"), t("created")], rows)}
  </section>`;
}

export function overviewActions() {
  return can("operator")
    ? `<button class="btn primary" type="button" data-launch-template>${h("launch_template")}</button>`
    : "";
}

export function quickLaunchHtml(data) {
  if (!can("operator")) return "";
  const templates = ((data && data.glance) || {}).recent_templates || [];
  if (!templates.length) return "";
  const rows = templates.slice(0, 5).map((item) => {
    const detail = item.last_launched_at
      ? `${esc(item.tool)} · ${esc(relativeTime(item.last_launched_at))}`
      : esc(item.tool);
    return `<li class="quick-launch-item" data-quick-launch="${esc(item.id)}">
      <div class="quick-launch-body">
        <a href="#/templates?edit=${encodeURIComponent(item.id)}">${esc(item.name)}</a>
        <span class="small muted">${detail}</span>
      </div>
      <button class="btn compact" type="button" data-quick-launch-start="${esc(item.id)}">${h("start")}</button>
    </li>`;
  }).join("");
  return `<section class="card" data-quick-launch-card>
    <div class="spread"><h2>${h("quick_launch")}</h2><a href="#/templates">${h("all")}</a></div>
    <ul class="quick-launch-list">${rows}</ul>
  </section>`;
}

export function webhookDeliveriesCard(data) {
  const deliveries = ((data && data.deliveries) || []).map((item) =>
    `<tr><td>${esc(item.hook_name)}</td><td>${pill(item.status)}</td><td>${esc(item.summary)}</td></tr>`
  );
  const test = can("operator") && data && data.hook_id
    ? `<button class="btn" id="test-alert" type="button">${h("test_alert")}</button>`
    : "";
  return `<section class="card">
    <div class="spread">
      <h2>${h("webhook_deliveries")}</h2>
      ${test}
    </div>
    ${table([t("hook"), t("status"), t("summary")], deliveries)}
  </section>`;
}

async function openTemplatePicker() {
  const items = await api("GET", "/api/v1/templates");
  if (!items.length) {
    go("#/templates?new=1");
    return;
  }
  let query = "";
  const dialog = openDialog({
    id: "template-picker",
    title: t("launch_template"),
    body: `<p class="small muted">${h("launch_template_help")}</p>
      <input class="search" id="template-picker-search" type="search" placeholder="${h("search_templates")}" autocomplete="off" aria-label="${h("search_templates")}">
      <div class="picker-list" id="template-picker-list"></div>`,
  });
  const list = dialog.form.querySelector("#template-picker-list");
  const draw = () => {
    const shown = filterTemplates(items, "all", query);
    list.innerHTML = shown.length
      ? shown.map((item) => `<button class="btn picker-option" type="button" data-pick-template="${esc(item.id)}">
          <span>${esc(item.name)}</span>
          <span class="small muted">${esc(item.tool)}</span>
        </button>`).join("")
      : `<p class="small muted">${h("nothing_here")}</p>`;
    list.querySelectorAll("[data-pick-template]").forEach((button) => {
      button.addEventListener("click", async () => {
        const item = items.find((row) => row.id === button.dataset.pickTemplate);
        if (!item) return;
        dialog.close();
        try {
          await openLaunchPrompt(item);
        } catch (error) {
          handle(error);
        }
      });
    });
  };
  dialog.form.querySelector("#template-picker-search").addEventListener("input", (event) => {
    query = event.target.value;
    dialog.clean();
    draw();
  });
  draw();
}

export function bindOverviewActions(root = document) {
  const button = root.querySelector("[data-launch-template]");
  if (!button) return;
  button.addEventListener("click", async () => {
    if (button.disabled) return;
    button.disabled = true;
    try {
      await openTemplatePicker();
    } catch (error) {
      handle(error);
    } finally {
      button.disabled = false;
    }
  });
}

export function bindQuickLaunch(root = document) {
  root.querySelectorAll("[data-quick-launch-start]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (button.disabled) return;
      button.disabled = true;
      try {
        const item = await api("GET", `/api/v1/templates/${encodeURIComponent(button.dataset.quickLaunchStart)}`);
        await openLaunchPrompt(item);
      } catch (error) {
        handle(error);
      } finally {
        button.disabled = false;
      }
    });
  });
}

function concurrencyCard(item) {
  const use = item || {};
  return glanceCard("concurrency", "concurrency", {
    body: `<div class="row">${glanceStat(`${Number(use.team_running) || 0} / ${Number(use.team_limit) || 0}`, "team_slots", "team-slots")}${glanceStat(`${Number(use.global_running) || 0} / ${Number(use.global_limit) || 0}`, "global_slots", "global-slots")}</div>
      <p class="small muted">${h("concurrency_help")}</p>`,
  });
}

function schedulesCard(item) {
  const schedules = item || {};
  const rows = (schedules.items || []).map((entry) => `<tr>
    <td><a href="#/schedules?edit=${encodeURIComponent(entry.id)}">${esc(entry.name)}</a></td>
    <td>${esc(entry.target_name || "—")}</td>
    <td class="small muted">${entry.next_run_at ? esc(when(entry.next_run_at)) : "—"}</td>
    <td>${entry.missed_at ? `<span class="pill warn">${h("schedule_missed_state")}</span>` : ""}</td>
  </tr>`);
  const links = [{ href: "#/schedules", label: "schedules" }];
  const missed = Number(schedules.missed_count) || 0;
  if (!rows.length && !missed) {
    return glanceCard("schedules", "upcoming_schedules", {
      links,
      body: healthyLine("schedules", "schedules_all_clear"),
    });
  }
  const next = schedules.next_run_at ? esc(when(schedules.next_run_at)) : h("schedule_no_upcoming");
  return glanceCard("schedules", "upcoming_schedules", {
    links,
    body: `<div class="row">${glanceStat(missed, "schedule_missed_state", "missed-schedules")}</div>
      <p class="small muted">${h("next_run")}: ${next}</p>
      ${table([t("name"), t("target"), t("next_run"), t("status")], rows)}`,
  });
}

function hostsCard(item) {
  const hosts = item || {};
  const rows = (hosts.items || []).map((entry) => `<tr>
    <td><a class="mono" href="#/inventories/${encodeURIComponent(entry.inventory_id)}/hosts/${encodeURIComponent(entry.host)}">${esc(entry.host)}</a></td>
    <td>${esc(entry.inventory_name || "—")}</td>
    <td>${pill(entry.result)}</td>
    <td class="small muted">${esc(when(entry.ts))}</td>
  </tr>`);
  const links = [{ href: "#/inventories", label: "inventories" }];
  const failed = Number(hosts.failed) || 0;
  const stale = Number(hosts.stale) || 0;
  if (!rows.length && !failed && !stale) {
    return glanceCard("hosts", "problem_hosts", {
      links,
      body: healthyLine("hosts", "hosts_all_clear"),
    });
  }
  return glanceCard("hosts", "problem_hosts", {
    links,
    body: `<div class="row">${glanceStat(failed, "failed_hosts", "failed-hosts")}${glanceStat(stale, "stale_hosts", "stale-hosts")}</div>
      <p class="small muted">${h("hosts_stale_after_days", { days: Number(hosts.stale_after_days) || 7 })}</p>
      ${table([t("host"), t("inventory"), t("run_result"), t("last_run")], rows)}`,
  });
}

function syncsCard(item) {
  const syncs = item || {};
  const projects = syncs.projects || [];
  const inventories = syncs.inventories || [];
  const row = (entry, kind, href) => `<tr>
    <td><a href="${href}">${esc(entry.name)}</a></td>
    <td>${h(kind)}</td>
    <td class="small muted">${entry.last_sync_at ? esc(when(entry.last_sync_at)) : h("never")}</td>
  </tr>`;
  const rows = [
    ...projects.slice(0, 5).map((entry) => row(entry, "project", `#/projects/${encodeURIComponent(entry.id)}`)),
    ...inventories.slice(0, 5).map((entry) => row(entry, "inventory", `#/inventories/${encodeURIComponent(entry.id)}`)),
  ];
  const links = [
    { href: "#/projects", label: "projects" },
    { href: "#/inventories", label: "inventories" },
  ];
  if (!rows.length) {
    return glanceCard("syncs", "failed_syncs", {
      links,
      body: healthyLine("syncs", "syncs_all_clear"),
    });
  }
  return glanceCard("syncs", "failed_syncs", {
    links,
    body: `<div class="row">${glanceStat(projects.length, "projects", "failed-projects")}${glanceStat(inventories.length, "inventories", "failed-inventories")}</div>
      <p class="small muted">${h("last_successful_sync_hint")}</p>
      ${table([t("name"), t("type"), t("last_sync")], rows)}`,
  });
}

function workflowsCard(item) {
  const workflows = item || {};
  const rows = (workflows.items || []).map((entry) => `<tr>
    <td><a href="#/runs/${encodeURIComponent(entry.id)}">${esc(entry.name)}</a></td>
    <td>${entry.workflow_id ? `<a href="#/workflows/${encodeURIComponent(entry.workflow_id)}">${h("workflow")}</a>` : "—"}</td>
    <td>${pill(entry.status)}</td>
    <td class="small muted">${esc(when(entry.created_at))}</td>
  </tr>`);
  const links = [{ href: "#/workflows", label: "workflows" }];
  const running = Number(workflows.running) || 0;
  const failed = Number(workflows.failed) || 0;
  if (!rows.length && !running && !failed) {
    return glanceCard("workflows", "workflow_runs", {
      links,
      body: healthyLine("workflows", "runs_all_clear"),
    });
  }
  return glanceCard("workflows", "workflow_runs", {
    links,
    body: `<div class="row">${glanceStat(running, "running", "running-runs")}${glanceStat(failed, "failed", "failed-runs")}</div>
      <p class="small muted">${h("workflow_failed_window", { hours: Number(workflows.failed_window_hours) || 24 })}</p>
      ${table([t("run"), t("workflow"), t("status"), t("created")], rows)}`,
  });
}

export function overviewSystemLine(data) {
  const runner = data && data.runner_mode ? esc(data.runner_mode) : "—";
  const runtime = data && data.runtime ? esc(data.runtime) : h("no_container_runtime");
  const directory = data && data.ldap ? h("ldap_is_configured") : h("local_accounts");
  return `<p class="small muted" data-overview-system>${h("runner")}: ${runner} · ${runtime} · ${directory}</p>`;
}

export function overviewGlanceHtml(data) {
  const glance = (data && data.glance) || {};
  return `${kpiRow(data)}
    <div class="overview-main">
      <div class="stack">${quickLaunchHtml(data)}${attentionHtml(data)}</div>
      ${recentActivityCard(data && data.jobs)}
    </div>
    <div class="grid-3 glance-grid">${concurrencyCard(glance.concurrency)}${schedulesCard(glance.schedules)}${hostsCard(glance.hosts)}</div>
    <div class="grid-3 glance-grid">${syncsCard(glance.syncs)}${workflowsCard(glance.workflows)}</div>`;
}

let chartSerial = 0;

function dashboardChart(series, values, heading, legend) {
  const chartId = `chart-${chartSerial += 1}`;
  const totals = series.map((day) => values.reduce((sum, key) => sum + (Number(day[key]) || 0), 0));
  const max = Math.max(1, ...totals);
  const days = series.map((day, index) => {
    const title = `${day.date} · ${values.map((key) => `${key === "started" ? "" : `${t(key)} `}${Number(day[key]) || 0}`).join(" · ")}`;
    const bars = values.map((key) => {
      const value = Number(day[key]) || 0;
      if (!value) return "";
      return `<span class="chart-seg ${esc(key)}" style="flex-grow:${value}"></span>`;
    }).join("");
    const height = Math.max(4, Math.round((totals[index] / max) * 100));
    const bar = bars ? `<div class="chart-bar" style="height:${height}%">${bars}</div>` : "";
    return `<button class="chart-day" type="button" data-chart-day data-chart-target="${chartId}" data-chart-title="${esc(title)}" aria-label="${esc(title)}"><span class="chart-day-total">${totals[index]}</span><div class="chart-bars">${bar}</div><span class="chart-day-label">${esc(shortDay(day.date))}</span></button>`;
  }).join("");
  return `<section class="card"><div class="spread"><h2>${h(heading)}</h2>${legend ? `<div class="row">${legend}</div>` : ""}</div><div class="chart" role="group" aria-label="${h(heading)}">${days}</div><p class="small muted chart-detail" data-chart-detail="${chartId}" aria-live="polite">${h("chart_day_hint")}</p></section>`;
}

export function dashboardBody(data, range) {
  const series = Array.isArray(data.series) ? data.series : [];
  const started = series.reduce((sum, day) => sum + (Number(day.started) || 0), 0);
  const finished = series.reduce((sum, day) => sum + (Number(day.successful) || 0) + (Number(day.failed) || 0) + (Number(day.canceled) || 0), 0);
  const empty = started + finished === 0;
  const ranges = [["1", "today"], ["7", "last_7_days"], ["14", "last_14_days"]].map(([value, key]) =>
    `<button class="btn pill${range === value ? " primary" : ""}" type="button" data-dashboard-range="${value}" aria-pressed="${range === value}">${h(key)}</button>`
  ).join("");
  const charts = empty
    ? `<section class="card"><p class="muted chart-empty">${h("no_jobs_in_this_range")}</p></section>`
    : `<div class="charts">${dashboardChart(series, ["successful", "failed", "canceled"], "jobs_finished_per_day", `${pill("successful")}${pill("failed")}${pill("canceled")}`)}${dashboardChart(series, ["started"], "jobs_started_per_day", "")}</div>`;
  const cache = data.cache || {};
  return `<div class="spread tab-bar"><div class="row" role="group" aria-label="${h("range")}">${ranges}</div></div>
    <div class="stack">
      ${charts}
      <section>
        <div class="spread"><h2>${h("dependency_cache")}</h2>${cacheClearButton(can("owner"))}</div>
        <div class="grid-3">
          <article class="card stat"><span class="small muted">${h("collections")}</span><strong>${esc(Number(cache.collections) || 0)}</strong></article>
          <article class="card stat"><span class="small muted">${h("roles")}</span><strong>${esc(Number(cache.roles) || 0)}</strong></article>
          <article class="card stat"><span class="small muted">${h("opentofu")}</span><strong>${esc(bytes(cache.tofu_bytes))}</strong></article>
        </div>
      </section>
    </div>`;
}

function bindDashboardCharts() {
  const details = [...document.querySelectorAll("[data-chart-detail]")];
  // A phone shows fewer than 14 days at once; keep the newest day in view.
  document.querySelectorAll(".chart").forEach((chart) => {
    if (chart.scrollWidth > chart.clientWidth) chart.scrollLeft = chart.scrollWidth;
  });
  document.querySelectorAll("[data-chart-day]").forEach((button) => {
    const detail = details.find((node) => node.dataset.chartDetail === button.dataset.chartTarget);
    if (!detail) return;
    const show = () => {
      detail.textContent = button.dataset.chartTitle || "";
    };
    button.addEventListener("click", show);
    button.addEventListener("focus", show);
  });
}

async function clearCache(button) {
  const confirmed = await confirmDialog({
    title: t("clear_cache"),
    body: t("clear_cache_confirmation"),
    confirmLabel: t("clear_cache"),
    danger: true,
  });
  if (!confirmed) return;
  button.disabled = true;
  button.setAttribute("aria-busy", "true");
  button.textContent = t("clearing_cache");
  try {
    await api("DELETE", "/api/v1/cache");
    markSaved();
    await route();
  } catch (error) {
    button.disabled = false;
    button.removeAttribute("aria-busy");
    button.textContent = t("clear_cache");
    handle(error);
  }
}

function bindAttention() {
  document.querySelectorAll("[data-attention-relaunch]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (button.disabled) return;
      button.disabled = true;
      try {
        const job = await api("GET", `/api/v1/jobs/${encodeURIComponent(button.dataset.attentionRelaunch)}`);
        const relaunched = await relaunchJob(job);
        if (!relaunched) button.disabled = false;
      } catch (error) {
        button.disabled = false;
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-attention-sync]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (button.disabled) return;
      button.disabled = true;
      button.setAttribute("aria-busy", "true");
      try {
        await api("POST", button.dataset.attentionSync, {});
        markSaved();
        await route();
      } catch (error) {
        button.disabled = false;
        button.removeAttribute("aria-busy");
        handle(error);
      }
    });
  });
}

function bindDashboard(tab, range) {
  const tabs = [...document.querySelectorAll("[data-page-tab]")];
  const selectTab = async (next) => {
    history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", dashboardAddress(next, range));
    await route();
    const active = document.querySelector(`[data-page-tab="${next}"]`);
    if (active) active.focus();
  };
  tabs.forEach((button, index) => {
    button.addEventListener("click", () => {
      if (button.dataset.pageTab !== tab) selectTab(button.dataset.pageTab);
    });
    button.addEventListener("keydown", (event) => {
      let target = null;
      if (event.key === "ArrowRight") target = tabs[(index + 1) % tabs.length];
      else if (event.key === "ArrowLeft") target = tabs[(index - 1 + tabs.length) % tabs.length];
      else if (event.key === "Home") target = tabs[0];
      else if (event.key === "End") target = tabs[tabs.length - 1];
      if (!target) return;
      event.preventDefault();
      if (target.dataset.pageTab !== tab) selectTab(target.dataset.pageTab);
    });
  });
  document.querySelectorAll("[data-dashboard-range]").forEach((button) => {
    button.addEventListener("click", async () => {
      const next = button.dataset.dashboardRange;
      if (next === range) return;
      history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", dashboardAddress("dashboard", next));
      await route();
      const active = document.querySelector(`[data-dashboard-range="${next}"]`);
      if (active) active.focus();
    });
  });
  const clear = document.querySelector("[data-cache-clear]");
  if (clear) clear.addEventListener("click", () => clearCache(clear));
  bindDashboardCharts();
}

async function pageDashboardTab() {
  const range = dashboardRange();
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  const data = await api("GET", `/api/v1/dashboard?days=${encodeURIComponent(range)}&tz=${encodeURIComponent(timezone)}`);
  shell(t("dashboard"), `
    ${dashboardTabs("dashboard")}
    <div id="panel-dashboard" role="tabpanel" aria-labelledby="tab-dashboard">
      ${dashboardBody(data, range)}
    </div>`);
  bindDashboard("dashboard", range);
}

async function pageDashboard() {
  if (!needTeam()) return;
  if (dashboardTab() === "dashboard") return pageDashboardTab();
  const data = await api("GET", "/api/v1/overview");
  const hasJobs = (data.jobs || []).length > 0;
  shell(t("overview"), `
    ${dashboardTabs("overview")}
    <div id="panel-overview" role="tabpanel" aria-labelledby="tab-overview">
      ${data.preflight ? preflightNoticeHtml(data.preflight) : ""}
      ${appState.me.is_admin && data.import ? importNoticeHtml(data.import) : ""}
      ${overviewSystemLine(data)}
      ${hasJobs ? "" : onboardingCard()}
      <div class="stack">
        ${overviewGlanceHtml(data)}
        ${webhookDeliveriesCard(data)}
      </div>
    </div>`, overviewActions());
  bindDashboard("overview", dashboardRange());
  bindOverviewActions();
  bindQuickLaunch();
  bindAttention();
  const test = document.querySelector("#test-alert");
  if (test) {
    test.addEventListener("click", async () => {
      test.disabled = true;
      try {
        const result = await api("POST", `/api/v1/hooks/${encodeURIComponent(data.hook_id)}/test`);
        if (result.workflow_job_id) go(`#/runs/${result.workflow_job_id}`);
        else if (result.job_id) go(`#/jobs/${result.job_id}`);
        else route();
      } catch (error) {
        test.disabled = false;
        handle(error);
      }
    });
  }
}

registerRoute("#/", pageDashboard);
