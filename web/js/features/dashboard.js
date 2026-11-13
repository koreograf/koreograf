import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { can, go, handle, needTeam, route, shell } from "../core/app.js";
import { esc, h, table } from "../core/dom.js";
import { bytes, pill, preflightNoticeHtml, spanText, when } from "../core/format.js";
import { currentLanguage, t } from "../core/i18n.js";
import { registerRoute } from "../core/router.js";

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

function dashboardChart(series, values, heading, legend) {
  const totals = series.map((day) => values.reduce((sum, key) => sum + (Number(day[key]) || 0), 0));
  const max = Math.max(1, ...totals);
  const days = series.map((day) => {
    const title = `${day.date} · ${values.map((key) => `${key === "started" ? "" : `${t(key)} `}${Number(day[key]) || 0}`).join(" · ")}`;
    const bars = values.map((key) => {
      const value = Number(day[key]) || 0;
      if (!value) return "";
      const height = Math.max(3, Math.round((value / max) * 100));
      return `<span class="chart-seg ${esc(key)}" style="height:${height}%"></span>`;
    }).join("");
    return `<div class="chart-day" title="${esc(title)}"><div class="chart-bars">${bars}</div><span class="chart-day-label">${esc(shortDay(day.date))}</span></div>`;
  }).join("");
  return `<section class="card"><div class="spread"><h2>${h(heading)}</h2>${legend ? `<div class="row">${legend}</div>` : ""}</div><div class="chart" role="img" aria-label="${h(heading)}">${days}</div></section>`;
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
        <h2>${h("dependency_cache")}</h2>
        <div class="grid-3">
          <article class="card stat"><span class="small muted">${h("collections")}</span><strong>${esc(Number(cache.collections) || 0)}</strong></article>
          <article class="card stat"><span class="small muted">${h("roles")}</span><strong>${esc(Number(cache.roles) || 0)}</strong></article>
          <article class="card stat"><span class="small muted">${h("opentofu")}</span><strong>${esc(bytes(cache.tofu_bytes))}</strong></article>
        </div>
      </section>
    </div>`;
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
  const jobs = (data.jobs || []).map((job) =>
    `<tr><td><a href="#/jobs/${esc(job.id)}">${esc(job.name)}</a></td><td class="tool">${esc(job.tool)}</td><td>${pill(job.status)}</td><td class="small muted">${esc(when(job.created_at))}</td><td class="small muted">${esc(spanText(job.started_at, job.finished_at, job.status))}</td></tr>`
  );
  const deliveries = (data.deliveries || []).map((item) =>
    `<tr><td>${esc(item.hook_name)}</td><td>${pill(item.status)}</td><td>${esc(item.summary)}</td></tr>`
  );
  shell(t("overview"), `
    ${dashboardTabs("overview")}
    <div id="panel-overview" role="tabpanel" aria-labelledby="tab-overview">
      ${data.preflight ? preflightNoticeHtml(data.preflight) : ""}
      ${appState.me.is_admin && data.import ? importNoticeHtml(data.import) : ""}
      ${jobs.length ? "" : onboardingCard()}
      <div class="grid-3" style="margin-top:${jobs.length ? "0" : "14px"}">
        <article class="card stat"><span class="small muted">${h("dependency_cache")}</span><strong>${esc(bytes(data.cache_bytes))}</strong><p class="small muted">${h("collections_roles_and_opentofu_providers_stay_between_runs")}</p></article>
        <article class="card stat"><span class="small muted">${h("workflow")}</span><strong>${data.workflow_id ? `<a href="#/workflows/${esc(data.workflow_id)}">${esc(data.workflow_name)}</a>` : "—"}</strong><p class="small muted">${h("success_edges_and_failure_edges_start_the_next")}</p></article>
        <article class="card stat"><span class="small muted">${h("webhook")}</span><strong>${data.hook_id ? esc(data.hook_name) : "—"}</strong><p class="small muted">${data.ldap ? h("ldap_is_configured") : h("local_accounts")} ${h("runner")}: ${esc(data.runner_mode)}, ${data.runtime ? esc(data.runtime) : h("no_container_runtime")}.</p></article>
      </div>
      <div class="stack" style="margin-top:14px">
        <section class="card">
          <div class="spread"><h2>${h("recent_jobs")}</h2><a href="#/jobs">${h("all")}</a></div>
          ${table([t("name"), t("tool"), t("status"), t("created"), t("duration")], jobs)}
        </section>
        <section class="card">
          <div class="spread">
            <h2>${h("webhook_deliveries")}</h2>
            ${data.hook_id && can("operator") ? `<button class="btn primary" id="test-alert" type="button">${h("test_alert")}</button>` : ""}
          </div>
          ${table([t("hook"), t("status"), t("summary")], deliveries)}
        </section>
      </div>
    </div>`);
  bindDashboard("overview", dashboardRange());
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
