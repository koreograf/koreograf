"use strict";

import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { confirmDialog, configureDialogs } from "../core/dialog.js";
import { can, go, handle, loadMaintenance, markSaved, needTeam, preferredTeam, renderLogin, roleLabel, route, shell } from "../core/app.js";
import { addLink, esc, h, paramRow, selectOptions, table } from "../core/dom.js";
import { clearEditHash, editQuery, openEditDialog, rememberEditHash } from "../core/edit.js";
import { environmentToolLabel, pill, scopeLabel, spanText, when } from "../core/format.js";
import { jsonObject, secretInput } from "../core/forms.js";
import { configureLanguage, currentLanguage, loadCatalog, t } from "../core/i18n.js";
import { registerRedirect, registerRoute } from "../core/router.js";

window.addEventListener("beforeunload", (event) => {
  if (!appState.leaveGuard || !appState.leaveGuard()) return;
  event.preventDefault();
  event.returnValue = "";
});

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

function deny(title, min) {
  const text = min === "admin"
    ? "you_need_an_administrator_account"
    : min === "owner"
      ? "you_need_the_owner_role"
      : "you_need_the_operator_role";
  shell(t(title), `<div class="card">${h(text)}</div>`);
}

function roleOptions(selected) {
  const labels = { viewer: "viewer", operator: "operator", owner: "owner" };
  return ["viewer", "operator", "owner"].map((role) =>
    `<option value="${role}"${role === selected ? " selected" : ""}>${h(labels[role])}</option>`
  ).join("");
}

function sourceLabel(source) {
  if (source === "ldap") return "LDAP";
  if (source === "manual") return t("manual");
  if (source === "local") return t("local");
  return source || "";
}

function authSourceLabel(source) {
  return sourceLabel(source === "local" ? "local" : source);
}

function roleLegend() {
  return `<ul class="role-legend">
    <li><strong>${h("viewer")}</strong> ${h("a_viewer_sees_projects_inventories_templates_jobs_and")}</li>
    <li><strong>${h("operator")}</strong> ${h("an_operator_launches_jobs_and_maintains_projects_inventories")}</li>
    <li><strong>${h("owner")}</strong> ${h("an_owner_also_manages_credentials_environments_and_members")}</li>
    <li><strong>${h("administrator")}</strong> ${h("an_administrator_manages_organizations_teams_users_settings_and")}</li>
  </ul>`;
}

function bytes(value) {
  const n = Number(value) || 0;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / 1024 / 1024).toFixed(1)} MiB`;
}


function preflightNotices(report) {
  if (!report || report.ok) return [];
  const missing = report.missing || [];
  const runtimeMissing = missing.some((item) => item.kind === "runtime");
  return missing.map((item) => {
    if (item.kind === "runtime") {
      const reason = item.reason === "timeout" || item.reason === "not_answering" ? item.reason : "missing";
      return { key: `preflight_runtime_${reason}` };
    }
    if (item.kind === "project") {
      return {
        key: item.reason === "timeout" || item.reason === "failed" ? "preflight_project_unusable" : "preflight_project_missing",
        tool: item.tool,
      };
    }
    if (item.reason === "version") {
      return {
        key: "preflight_tool_too_old",
        tool: item.tool,
        template_tool: item.template_tool,
        version: item.version || "",
        minimum: item.minimum || "",
      };
    }
    if (item.reason === "timeout" || item.reason === "failed") {
      return { key: "preflight_tool_unusable", tool: item.tool, template_tool: item.template_tool };
    }
    return {
      key: runtimeMissing ? "preflight_no_runtime_and_tool_missing" : "preflight_tool_missing",
      tool: item.tool,
      template_tool: item.template_tool,
    };
  });
}

function preflightNoticeHtml(report) {
  const notices = preflightNotices(report);
  if (!notices.length) return "";
  const lines = notices.map((item) => {
    const template = item.template_tool || "";
    const vars = {
      ...item,
      templates: template ? t(template) : "",
      install: item.tool === "sh"
        ? t("preflight_install_shell")
        : item.tool === "ssh"
          ? t("preflight_install_ssh")
          : item.kind === "project"
            ? t("preflight_install_git")
            : (template ? t(template) : item.tool),
    };
    return `<p>${h(item.key, vars)}</p>`;
  }).join("");
  return `<div class="banner warn">${lines}</div>`;
}

function preflightReportHtml(report) {
  return preflightNoticeHtml(report) || `<div class="banner good">${h("preflight_ready")}</div>`;
}

function importNoticeHtml(importReport) {
  const paths = (importReport && importReport.missing_paths) || [];
  if (!paths.length) return "";
  return `<div class="banner warn"><p>${h("imported_paths_need_a_mount_or_a_change")}</p><ul>${paths.map((path) => `<li><code>${esc(path)}</code></li>`).join("")}</ul></div>`;
}

function dashboardTab() {
  const params = new URLSearchParams(location.hash.split("?")[1] || "");
  return params.get("tab") === "dashboard" ? "dashboard" : "overview";
}

function dashboardRange() {
  const params = new URLSearchParams(location.hash.split("?")[1] || "");
  const days = params.get("days");
  return ["1", "7", "14"].includes(days) ? days : "1";
}

function dashboardAddress(tab, range) {
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

function dashboardTabs(tab) {
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

function dashboardBody(data, range) {
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


function hookStatusLabel(value) {
  return {
    firing: t("hook_status_firing"),
    resolved: t("hook_status_resolved"),
    any: t("hook_status_any"),
  }[value] || value;
}

async function pageHooks() {
  if (!needTeam()) return;
  const hooks = await api("GET", "/api/v1/hooks");
  const deliveryRows = hooks.length
    ? await api("GET", `/api/v1/hooks/${encodeURIComponent(hooks[0].id)}/deliveries`)
    : [];
  const origin = location.origin;
  const incoming = deliveryRows.map((item) =>
    `<tr><td>${esc(item.hook_name)}</td><td>${pill(item.status)}</td><td>${esc(item.summary)}</td><td class="small muted">${esc(when(item.created_at))}</td></tr>`
  );
  shell(t("webhooks"), `
    <section class="card" style="margin-bottom:14px">
      <input class="search" id="hook-search" placeholder="${h("search_webhooks")}" autocomplete="off">
      <div id="hook-table"></div>
    </section>
    <section class="card"><h2>${h("deliveries")}</h2>${table([t("hook"), t("status"), t("summary"), t("time")], incoming)}</section>`,
    can("operator") ? addLink("#/hooks?new=1", "webhook_2") : "");
  const drawHooks = () => {
    const needle = document.querySelector("#hook-search").value.trim().toLowerCase();
    const rows = hooks.filter((hook) => !needle || `${hook.name} ${hook.match_alertname} ${hook.match_status}`.toLowerCase().includes(needle)).map((hook) => {
      const url = hook.token ? `${origin}/hooks/${hook.id}/${hook.token}` : "";
      return `<tr>
        <td>${can("operator") ? `<a href="#/hooks?edit=${esc(hook.id)}">${esc(hook.name)}</a>` : esc(hook.name)}<div class="small muted">${esc(hook.match_alertname)} · ${esc(hookStatusLabel(hook.match_status))}</div></td>
        <td class="mono">${url ? esc(url) : h("token_is_visible_to_operators")}</td>
        <td>${can("operator") ? `<button class="btn" type="button" data-test="${esc(hook.id)}">${h("test")}</button>` : ""}</td>
        <td class="actions danger">${can("operator") ? `<button class="btn danger" type="button" data-delete="${esc(hook.id)}">${h("delete")}</button>` : ""}</td>
      </tr>`;
    });
    document.querySelector("#hook-table").innerHTML = table([t("hook"), t("address"), "", ""], rows);
    document.querySelectorAll("[data-test]").forEach((button) => {
      button.addEventListener("click", async () => {
        try {
          const result = await api("POST", `/api/v1/hooks/${encodeURIComponent(button.dataset.test)}/test`);
          if (result.workflow_job_id) go(`#/runs/${result.workflow_job_id}`);
          else if (result.job_id) go(`#/jobs/${result.job_id}`);
          else route();
        } catch (error) {
          handle(error);
        }
      });
    });
    document.querySelectorAll("[data-delete]").forEach((button) => {
      button.addEventListener("click", async () => {
        const hook = hooks.find((row) => row.id === button.dataset.delete);
        if (!(await confirmDialog({ title: t("delete"), body: t("delete_this_webhook", { name: hook ? hook.name : "" }), confirmLabel: t("delete"), danger: true }))) return;
        try {
          await api("DELETE", `/api/v1/hooks/${encodeURIComponent(button.dataset.delete)}`);
          route();
        } catch (error) {
          handle(error);
        }
      });
    });
  };
  document.querySelector("#hook-search").addEventListener("input", drawHooks);
  drawHooks();
  const request = editQuery();
  if (request.create !== null) {
    if (can("operator")) await openHookDialog(null);
    return;
  }
  if (!request.edit) return;
  const item = hooks.find((row) => row.id === request.edit);
  if (!item) {
    clearEditHash();
    return;
  }
  if (can("operator")) await openHookDialog(item);
}

async function openHookDialog(item) {
  const editing = Boolean(item);
  const [templates, workflows] = await Promise.all([
    api("GET", "/api/v1/templates"),
    api("GET", "/api/v1/workflows"),
  ]);
  const targetId = editing ? (item.workflow_id || item.template_id || "") : "";
  const option = (value, label, selected) =>
    `<option value="${value}"${value === selected ? " selected" : ""}>${label}</option>`;
  const status = editing ? item.match_status : "firing";
  const targetOption = (kind, row) =>
    option(`${kind}:${row.id}`, esc(row.name), `${kind}:${targetId}`);
  openEditDialog({
    id: "hook-dialog",
    title: editing ? t("edit_webhook") : t("new_webhook"),
    body: `
      <label class="field required"><span>${h("name")}</span><input name="name" required value="${editing ? esc(item.name) : ""}"></label>
      <div class="field-row">
        <label class="field required"><span>${h("alert_name")}</span><input class="fit" name="match_alertname" required placeholder="DiskSpaceLow" value="${editing ? esc(item.match_alertname) : ""}"></label>
        <label class="field"><span>${h("status")}</span>
          <select class="fit" name="match_status">${option("firing", h("hook_status_firing"), status)}${option("resolved", h("hook_status_resolved"), status)}${option("any", h("hook_status_any"), status)}</select>
        </label>
      </div>
      <label class="field required"><span>${h("target")}</span>
        <select name="target">
          <option value=""${editing ? "" : " selected"}>${h("select")}</option>
          <optgroup label="${h("workflow")}">${workflows.map((row) => targetOption("workflow", row)).join("")}</optgroup>
          <optgroup label="${h("template")}">${templates.map((row) => targetOption("template", row)).join("")}</optgroup>
        </select>
      </label>
      <label class="field"><span>${h("extra_variables_json")}</span><textarea name="extra_vars">${editing ? esc(JSON.stringify(item.extra_vars || {}, null, 2)) : "{}"}</textarea></label>`,
    submit: editing ? t("save") : t("create"),
    onSubmit: async ({ form, close }) => {
      const rawTarget = form.target.value;
      const splitAt = rawTarget.indexOf(":");
      if (splitAt < 1) throw new Error("create_a_template_or_a_workflow_first");
      const kind = rawTarget.slice(0, splitAt);
      const id = rawTarget.slice(splitAt + 1);
      const payload = {
        name: form.name.value,
        match_alertname: form.match_alertname.value,
        match_status: form.match_status.value,
        extra_vars: jsonObject(form.extra_vars.value, "extra_variables"),
        template_id: kind === "template" ? id : null,
        workflow_id: kind === "workflow" ? id : null,
      };
      if (editing) await api("PUT", `/api/v1/hooks/${encodeURIComponent(item.id)}`, payload);
      else await api("POST", "/api/v1/hooks", payload);
      markSaved();
      close();
      await route();
    },
  });
}

async function pageRuntime() {
  if (!needTeam()) return;
  const [cache, plugins, environments, runner] = await Promise.all([
    api("GET", "/api/v1/cache"),
    api("GET", "/api/v1/plugins"),
    api("GET", "/api/v1/environments"),
    api("GET", "/api/v1/runner"),
  ]);
  const collectionRows = (cache.collections || []).map((item) => `<tr><td>${esc(item.name)}</td><td>${esc(item.version)}</td></tr>`);
  const roleRows = (cache.roles || []).map((item) => `<tr><td>${esc(item.name)}</td><td>${esc(item.version)}</td></tr>`);
  const pluginRows = plugins.map((item) => `<tr><td class="tool">${esc(item.id)}</td><td>${esc(t(item.summary))}</td></tr>`);
  shell(t("execution_environments"), `
    <section class="card" style="margin-bottom:14px">
      <h2>${h("execution_environments")}</h2>
      <input class="search" id="environment-search" placeholder="${h("search_environments")}" autocomplete="off">
      <div id="environment-table"></div>
    </section>
    <div class="grid-3" style="margin-bottom:14px">
      <article class="card stat"><span class="small muted">${h("collections")}</span><strong>${esc(bytes(cache.collections_bytes))}</strong></article>
      <article class="card stat"><span class="small muted">${h("roles")}</span><strong>${esc(bytes(cache.roles_bytes))}</strong></article>
      <article class="card stat"><span class="small muted">${h("opentofu")}</span><strong>${esc(bytes(cache.tofu_bytes))}</strong></article>
      <article class="card stat"><span class="small muted">${h("local_jobs")}</span><strong>${esc(runner.local_jobs)}</strong></article>
    </div>
    <p class="small muted" style="margin-top:0">${esc(t(cache.note || ""))}</p>
    <section class="card" style="margin-bottom:14px"><h2>${h("tools")}</h2>${table([t("id"), t("description")], pluginRows)}</section>
    <section class="card" style="margin-bottom:14px"><h2>${h("cached_collections")}</h2>${table([t("name"), t("version")], collectionRows)}</section>
    <section class="card" style="margin-bottom:14px"><h2>${h("cached_roles")}</h2>${table([t("name"), t("version")], roleRows)}</section>`,
    can("owner") ? addLink("#/runtime?new=1", "environment") : "");
  const drawEnvironments = () => {
    const needle = document.querySelector("#environment-search").value.trim().toLowerCase();
    const rows = environments.filter((item) => !needle || `${item.name} ${item.tool || ""} ${item.image} ${item.pull_policy} ${item.scope || ""} ${scopeLabel(item)}`.toLowerCase().includes(needle)).map((item) =>
      `<tr><td><a href="#/environments/${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(scopeLabel(item))}</td><td>${esc(environmentToolLabel(item.tool))}</td><td class="mono">${esc(item.image)}</td><td>${esc(item.pull_policy)}</td><td>${canManageEnvironment(item) ? `<a class="btn" href="#/runtime?edit=${esc(item.id)}">${h("edit")}</a> <button class="btn danger" type="button" data-env="${esc(item.id)}">${h("delete")}</button>` : ""}</td></tr>`
    );
    document.querySelector("#environment-table").innerHTML = table([t("name"), t("applies_to"), t("tool"), t("image"), t("pull"), ""], rows);
    document.querySelectorAll("[data-env]").forEach((button) => {
      button.addEventListener("click", async () => {
        try {
          await api("DELETE", `/api/v1/environments/${encodeURIComponent(button.dataset.env)}`);
          route();
        } catch (error) {
          handle(error);
        }
      });
    });
  };
  document.querySelector("#environment-search").addEventListener("input", drawEnvironments);
  drawEnvironments();
  const request = editQuery();
  if (request.create !== null) {
    if (can("owner")) openEnvironmentDialog(null);
    return;
  }
  if (!request.edit) return;
  const item = environments.find((row) => row.id === request.edit);
  if (!item) {
    clearEditHash();
    return;
  }
  if (canManageEnvironment(item)) openEnvironmentDialog(item);
}

function canManageEnvironment(item) {
  if (!can("owner")) return false;
  if (item && item.scope === "global") return !!appState.me.is_admin;
  return true;
}


function environmentScopeOptions(selected) {
  const choices = [
    ["team", "this_team"],
    ["organization", "this_organization"],
  ];
  if (appState.me.is_admin) choices.push(["global", "global"]);
  return choices.map(([value, label]) =>
    `<option value="${value}"${value === selected ? " selected" : ""}>${h(label)}</option>`
  ).join("");
}

function environmentImagePlaceholder(tool) {
  if (tool === "ansible") return "quay.io/ansible/awx-ee:latest";
  if (tool === "opentofu") return "ghcr.io/opentofu/opentofu:latest";
  if (tool === "shell") return "docker.io/library/alpine:latest";
  return "registry.example.com/execution-environment:latest";
}

function openEnvironmentDialog(item) {
  const editing = Boolean(item);
  const policy = item ? item.pull_policy : "if_not_present";
  const scope = item && item.scope ? item.scope : "team";
  const current = item && item.tool ? item.tool : "";
  const tool = ["ansible", "opentofu", "shell"].includes(current) ? current : "ansible";
  const option = (value) => `<option value="${value}"${policy === value ? " selected" : ""}>${value}</option>`;
  const toolOption = (value) => `<option value="${value}"${tool === value ? " selected" : ""}>${h(value)}</option>`;
  const dialog = openEditDialog({
    id: "environment-dialog",
    title: editing ? t("edit_environment") : t("new_environment"),
    body: `
      <label class="field required"><span>${h("name")}</span><input name="name" required value="${editing ? esc(item.name) : ""}"></label>
      <label class="field"><span>${h("tool")}</span><select class="fit" name="tool">${toolOption("ansible")}${toolOption("opentofu")}${toolOption("shell")}</select></label>
      <label class="field required"><span>${h("image")}</span><input name="image" required value="${editing ? esc(item.image) : ""}" placeholder="${esc(environmentImagePlaceholder(tool))}"></label>
      <label class="field"><span>${h("network")}</span><input name="network" value="${editing ? esc(item.network || "") : ""}" placeholder="koreograf_default"></label>
      <p class="small muted">${h("the_network_is_passed_to_the_container_run_an_empty")}</p>
      <div class="field-row">
        <label class="field"><span>${h("pull")}</span><select class="fit" name="pull_policy">${option("if_not_present")}${option("always")}${option("never")}</select></label>
        <label class="field"><span>${h("applies_to")}</span><select class="fit" name="scope">${environmentScopeOptions(scope)}</select></label>
      </div>
      <p class="small muted">${h("global_applies_to_every_team_the_organization_applies")}</p>
      <label class="field"><span>${h("description")}</span><input name="description" maxlength="1000" value="${editing ? esc(item.description) : ""}"></label>`,
    submit: editing ? t("save") : t("create"),
    onSubmit: async ({ form, close }) => {
      const payload = {
        name: form.name.value,
        image: form.image.value,
        tool: form.tool.value,
        pull_policy: form.pull_policy.value,
        network: form.network.value,
        description: form.description.value,
        scope: form.scope.value,
      };
      if (editing) await api("PUT", `/api/v1/environments/${encodeURIComponent(item.id)}`, payload);
      else await api("POST", "/api/v1/environments", payload);
      markSaved();
      close();
      await route();
    },
  });
  const refreshImagePlaceholder = () => {
    dialog.form.image.placeholder = environmentImagePlaceholder(dialog.form.tool.value);
  };
  dialog.form.tool.addEventListener("change", refreshImagePlaceholder);
  refreshImagePlaceholder();
}

async function pageEnvironment(id) {
  if (!needTeam()) return;
  const item = await api("GET", `/api/v1/environments/${encodeURIComponent(id)}`);
  const manage = canManageEnvironment(item);
  shell(item.name, `<section class="card"><dl class="kv">
      ${paramRow("name", item.name)}
      ${paramRow("applies_to", scopeLabel(item))}
      ${paramRow("tool", environmentToolLabel(item.tool))}
      ${paramRow("image", item.image)}
      ${paramRow("pull", item.pull_policy)}
      ${paramRow("description", item.description)}
    </dl></section>`, manage
    ? `<a class="btn pill" href="#/runtime?edit=${esc(item.id)}">${h("edit")}</a><button class="btn danger" id="delete-env" type="button">${h("delete")}</button>`
    : "");
  const remove = document.querySelector("#delete-env");
  if (!remove) return;
  remove.addEventListener("click", async () => {
    try {
      await api("DELETE", `/api/v1/environments/${encodeURIComponent(id)}`);
      go("#/runtime");
    } catch (error) {
      handle(error);
    }
  });
}


async function pageOrganizations() {
  const items = await api("GET", "/api/v1/organizations");
  shell(t("organizations"), `
    <section class="card">
      <input class="search" id="organization-search" placeholder="${h("search_organizations")}" autocomplete="off">
      <div id="organization-table"></div>
    </section>`,
    appState.me.is_admin ? addLink("#/organizations?new=1", "organization_2") : "");
  const drawOrganizations = () => {
    const needle = document.querySelector("#organization-search").value.trim().toLowerCase();
    const rows = items.filter((item) => !needle || `${item.name} ${item.description || ""}`.toLowerCase().includes(needle)).map((item) =>
      `<tr><td><a href="#/organizations?edit=${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(item.description || "")}</td></tr>`
    );
    document.querySelector("#organization-table").innerHTML = table([t("name"), t("description")], rows);
  };
  document.querySelector("#organization-search").addEventListener("input", drawOrganizations);
  drawOrganizations();
  const request = editQuery();
  if (request.create !== null) {
    if (appState.me.is_admin) openOrganizationDialog(null);
    return;
  }
  if (!request.edit) return;
  try {
    const item = await api("GET", `/api/v1/organizations/${encodeURIComponent(request.edit)}`);
    openOrganizationDialog(item, !appState.me.is_admin, request.tab);
  } catch (error) {
    handle(error);
  }
}

function openOrganizationDialog(item, readOnly = false, initialTab = "") {
  const editing = Boolean(item);
  const general = `
    <label class="field required"><span>${h("name")}</span><input name="name" required value="${editing ? esc(item.name) : ""}"></label>
    <label class="field"><span>${h("description")}</span><input name="description" maxlength="1000" value="${editing ? esc(item.description || "") : ""}"></label>`;
  const rows = editing ? (item.teams || []).map((team) =>
    `<tr><td>${esc(team.name)}</td><td>${esc(team.description || "")}</td><td class="small muted">${esc(team.ldap_group || "")}</td></tr>`
  ) : [];
  const tabs = editing ? [
    { id: "general", label: t("general"), body: general },
    { id: "teams", label: t("teams"), body: table([t("name"), t("description"), t("ldap_group")], rows) },
  ] : null;
  openEditDialog({
    id: "organization-dialog",
    title: editing ? item.name : t("new_organization"),
    tabs,
    body: general,
    hint: readOnly ? "" : undefined,
    submit: editing ? t("save") : t("create"),
    actions: !readOnly,
    onSubmit: readOnly ? undefined : async ({ form, close }) => {
      if (editing) {
        await api("PUT", `/api/v1/organizations/${encodeURIComponent(item.id)}`, {
          name: form.name.value,
          description: form.description.value,
        });
        appState.setMe(await api("GET", "/api/v1/me"));
      } else {
        await api("POST", "/api/v1/organizations", { name: form.name.value, description: form.description.value });
      }
      markSaved();
      close();
      await route();
    },
    initialTab,
    onTab: editing ? (tab) => rememberEditHash({ edit: item.id, tab }, false) : undefined,
  });
}

async function pageTeam() {
  const teams = appState.me.teams || [];
  const current = teams.find((team) => team.id === appState.teamId);
  const organizations = appState.me.is_admin ? await api("GET", "/api/v1/organizations") : [];
  let members = [];
  if (appState.teamId) members = await api("GET", `/api/v1/teams/${encodeURIComponent(appState.teamId)}/members`);
  const memberRows = members.map((member) => {
    const name = appState.me.is_admin
      ? `<a href="#/users?edit=${esc(member.user_id)}">${esc(member.display_name)}</a>`
      : esc(member.display_name);
    const roleCell = member.source === "manual" && can("owner")
      ? `<select data-member-role="${esc(member.username)}" data-current="${esc(member.role)}">${roleOptions(member.role)}</select>`
      : esc(roleLabel(member.role));
    const sourceCell = member.source === "ldap"
      ? `${esc(sourceLabel(member.source))} <span class="small muted">${h("comes_from_the_directory")}</span>`
      : esc(sourceLabel(member.source));
    const remove = can("owner")
      ? `<button class="btn danger" type="button" data-member="${esc(member.user_id)}">${h("remove")}</button>`
      : "";
    return `<tr><td>${name} <span class="small muted">${esc(member.username)}</span></td><td>${roleCell}</td><td>${sourceCell}</td><td>${remove}</td></tr>`;
  });
  const actions = [
    appState.teamId && can("owner") ? addLink("#/team?new=member", "member") : "",
    appState.me.is_admin ? addLink("#/team?new=1", "team_2") : "",
    appState.me.is_admin ? addLink("#/users?new=1", "user_2") : "",
  ].join("");
  shell(t("team"), `
    ${current ? `<section class="card" style="margin-bottom:14px">
      <h2>${esc(current.name)}</h2>
      <p class="muted">${esc(current.description || "")}</p>
      <p class="small">${h("organization_name_2", { name: current.organization_name || "—" })}</p>
      ${current.role === "owner" || appState.me.is_admin ? `<div class="row"><a class="btn" href="#/team?edit=${esc(current.id)}">${h("edit")}</a></div>` : `<p class="small">${h("ldap_group_group", { group: current.ldap_group || "—" })}</p>`}
    </section>
    <section class="card">
      <h2>${h("members")}</h2>
      ${roleLegend()}
      ${table([t("name"), t("role"), t("source"), ""], memberRows)}
    </section>` : `<div class="card">${h("no_team_selected")}</div>`}`, actions);
  const request = editQuery();
  if (request.create === "member") {
    if (can("owner")) openMemberDialog();
  } else if (request.create !== null) {
    if (appState.me.is_admin) openTeamDialog(null, organizations, current && current.organization_id);
  } else if (request.edit) {
    const item = teams.find((team) => team.id === request.edit);
    if (!item) clearEditHash();
    else if (item.role === "owner" || appState.me.is_admin) openTeamDialog(item, organizations);
  }
  document.querySelectorAll("[data-member-role]").forEach((select) => {
    select.addEventListener("change", async () => {
      const next = select.value;
      try {
        await api("POST", `/api/v1/teams/${encodeURIComponent(appState.teamId)}/members`, {
          username: select.dataset.memberRole,
          role: next,
        });
        select.dataset.current = next;
      } catch (error) {
        select.value = select.dataset.current;
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-member]").forEach((button) => {
    button.addEventListener("click", async () => {
      try {
        await api("DELETE", `/api/v1/teams/${encodeURIComponent(appState.teamId)}/members/${encodeURIComponent(button.dataset.member)}`);
        route();
      } catch (error) { handle(error); }
    });
  });
}

function openTeamDialog(item, organizations, defaultOrganizationId) {
  const editing = Boolean(item);
  const organizationField = appState.me.is_admin
    ? `<label class="field"><span>${h(editing ? "move_organization" : "organization")}</span><select name="organization_id">${editing ? orgOptions(organizations, item.organization_id) : `<option value="">${h("new_organization_named_after_the_team")}</option>${orgOptions(organizations, defaultOrganizationId || "")}`}</select></label>`
    : "";
  openEditDialog({
    id: "team-dialog",
    title: editing ? item.name : t("new_team_2"),
    body: `
      <label class="field required"><span>${h("name")}</span><input name="name" required value="${editing ? esc(item.name) : ""}"></label>
      <label class="field"><span>${h("description")}</span><input name="description" maxlength="1000" value="${editing ? esc(item.description || "") : ""}"></label>
      <label class="field"><span>${h("ldap_or_ad_group")}</span><input name="ldap_group" value="${editing ? esc(item.ldap_group || "") : ""}" placeholder="${h("cn_or_dn_for_example_platform")}"></label>
      ${organizationField}`,
    submit: editing ? t("save_team") : t("create"),
    onSubmit: async ({ form, close }) => {
      const payload = {
        name: form.name.value,
        description: form.description.value,
        ldap_group: form.ldap_group.value,
      };
      if (form.organization_id && form.organization_id.value) payload.organization_id = form.organization_id.value;
      if (editing) {
        await api("PUT", `/api/v1/teams/${encodeURIComponent(item.id)}`, payload);
      } else {
        const created = await api("POST", "/api/v1/teams", payload);
        appState.setTeamId(created.id);
        localStorage.setItem("koreograf.team", appState.teamId);
      }
      appState.setMe(await api("GET", "/api/v1/me"));
      markSaved();
      close();
      await route();
    },
  });
}

function openMemberDialog() {
  openEditDialog({
    id: "member-dialog",
    title: t("add_member"),
    body: `
      <label class="field required"><span>${h("username")}</span><input name="username" required></label>
      <div class="field-row">
        <label class="field"><span>${h("role")}</span>
          <select class="fit" name="role"><option value="viewer">${h("viewer")}</option><option value="operator" selected>${h("operator")}</option><option value="owner">${h("owner")}</option></select>
        </label>
      </div>`,
    submit: t("add"),
    onSubmit: async ({ form, close }) => {
      await api("POST", `/api/v1/teams/${encodeURIComponent(appState.teamId)}/members`, {
        username: form.username.value,
        role: form.role.value,
      });
      markSaved();
      close();
      await route();
    },
  });
}

function emailLine(email) {
  return email ? esc(email) : `<span class="muted">${h("no_email_is_stored")}</span>`;
}

async function pageAccount() {
  appState.setMe(await api("GET", "/api/v1/me"));
  const tokens = await api("GET", "/api/v1/me/tokens");
  const revealed = appState.revealToken;
  appState.setRevealToken(null);
  const rows = (appState.me.memberships || []).map((row) => {
    const org = row.organization_name || "—";
    return `<tr><td>${esc(org)}</td><td>${esc(row.team_name)}</td><td>${esc(roleLabel(row.role))}</td><td>${esc(sourceLabel(row.source))}</td></tr>`;
  });
  const tokenRows = tokens.map((item) => {
    const used = item.last_used_at ? esc(when(item.last_used_at)) : h("never");
    return `<tr>
      <td>${esc(item.name)}</td>
      <td class="mono">${esc(item.prefix)}</td>
      <td class="small muted">${esc(when(item.created_at))}</td>
      <td class="small muted">${used}</td>
      <td class="actions"><button class="btn compact danger" type="button" data-revoke="${esc(item.id)}" data-name="${esc(item.name)}">${h("revoke")}</button></td>
    </tr>`;
  });
  const secret = revealed ? `
      <div class="banner good">
        <p>${h("api_tokens_hint")}</p>
        <p><code>${esc(revealed)}</code> <button class="btn compact" type="button" data-copy="${esc(revealed)}">${h("copy")}</button></p>
      </div>` : "";
  const local = appState.me.auth_source === "local";
  const identity = local ? `
    <form class="card form" id="account-form">
      <h2>${h("account")}</h2>
      <p class="small muted">${h("username")}: ${esc(appState.me.username)} · ${esc(authSourceLabel(appState.me.auth_source))}${appState.me.is_admin ? ` · ${h("administrator")}` : ""}</p>
      <label class="field"><span>${h("display_name")}</span><input name="display_name" value="${esc(appState.me.display_name)}" required></label>
      <label class="field"><span>${h("email")}</span><input name="email" type="email" value="${esc(appState.me.email || "")}" autocomplete="email"></label>
      <label class="field"><span>${h("new_password")}</span><input name="password" type="password" minlength="8" autocomplete="new-password" placeholder="${h("leave_empty_to_keep_the_password")}"></label>
      <button class="btn pill primary" type="submit">${h("save")}</button>
    </form>` : `
    <section class="card">
      <h2>${h("account")}</h2>
      <p class="small muted">${h("username")}: ${esc(appState.me.username)} · ${esc(authSourceLabel(appState.me.auth_source))}${appState.me.is_admin ? ` · ${h("administrator")}` : ""}</p>
      <p>${h("display_name")}: ${esc(appState.me.display_name)}</p>
      <p>${h("email")}: ${emailLine(appState.me.email)}</p>
      <p class="muted">${h("name_and_email_come_from_the_directory")}</p>
    </section>`;
  shell(t("my_account"), `
    <div class="stack">
      ${identity}
      <section class="card">
        <h2>${h("memberships")}</h2>
        ${table([t("organization"), t("team"), t("role"), t("source")], rows)}
      </section>
      <section class="card">
        <h2>${h("api_tokens")}</h2>
        ${secret}
        <form class="form" id="token-form">
          <label class="field required">
            <span>${h("name")}</span>
            <input name="name" required>
          </label>
          <div><button class="btn pill primary" type="submit">${h("create_token")}</button></div>
        </form>
        ${table([t("name"), t("prefix"), t("created"), t("last_used"), t("actions")], tokenRows)}
      </section>
    </div>`);
  const form = document.querySelector("#account-form");
  if (form) {
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        const payload = {
          display_name: form.display_name.value,
          email: form.email.value,
        };
        if (form.password.value) payload.password = form.password.value;
        appState.setMe(await api("PUT", "/api/v1/me", payload));
        markSaved();
        route();
      } catch (error) { handle(error); }
    });
  }
  const tokenForm = document.querySelector("#token-form");
  if (tokenForm) {
    tokenForm.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        const created = await api("POST", "/api/v1/me/tokens", { name: tokenForm.name.value });
        appState.setRevealToken(created.secret);
        await route();
      } catch (error) { handle(error); }
    });
  }
  document.querySelectorAll("[data-copy]").forEach((button) => {
    button.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(button.dataset.copy);
        button.textContent = t("copied");
      } catch {}
    });
  });
  document.querySelectorAll("[data-revoke]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (!(await confirmDialog({ title: t("revoke"), body: t("delete_this_token", { name: button.dataset.name }), confirmLabel: t("revoke"), danger: true }))) return;
      try {
        await api("DELETE", `/api/v1/me/tokens/${encodeURIComponent(button.dataset.revoke)}`);
        await route();
      } catch (error) { handle(error); }
    });
  });
}

async function pageUsers() {
  if (!appState.me.is_admin) {
    deny("users", "admin");
    return;
  }
  const items = await api("GET", "/api/v1/users");
  shell(t("users"), `
    <section class="card">
      <input class="search" id="user-search" placeholder="${h("search_users")}" autocomplete="off">
      <div id="user-table"></div>
    </section>`,
    addLink("#/users?new=1", "user_2"));
  const draw = () => {
    const needle = document.querySelector("#user-search").value.trim().toLowerCase();
    const shown = items.filter((item) => {
      if (!needle) return true;
      return `${item.display_name} ${item.username} ${item.email || ""} ${item.auth_source}`.toLowerCase().includes(needle);
    });
    const rows = shown.map((item) => {
      const flags = [
        item.is_admin ? `<span class="pill">${h("administrator")}</span>` : "",
        item.disabled ? `<span class="pill failed">${h("disabled")}</span>` : "",
      ].filter(Boolean).join(" ");
      const created = when(item.created_at);
      return `<tr>
        <td><a href="#/users?edit=${esc(item.id)}">${esc(item.display_name)}</a> ${flags}</td>
        <td>${esc(item.username)}</td>
        <td>${esc(authSourceLabel(item.auth_source))}</td>
        <td>${esc(item.team_count)}</td>
        <td class="small muted">${esc(created)}</td>
      </tr>`;
    });
    document.querySelector("#user-table").innerHTML = table(
      [t("name"), t("username"), t("source"), t("teams"), t("created")],
      rows
    );
  };
  document.querySelector("#user-search").addEventListener("input", draw);
  draw();
  const request = editQuery();
  if (request.create !== null) {
    openUserCreateDialog();
    return;
  }
  if (!request.edit) return;
  try {
    const [item, teams] = await Promise.all([
      api("GET", `/api/v1/users/${encodeURIComponent(request.edit)}`),
      api("GET", "/api/v1/teams"),
    ]);
    openUserDialog(item, teams, request.tab);
  } catch (error) {
    handle(error);
  }
}

function openUserCreateDialog() {
  openEditDialog({
    id: "user-dialog",
    title: t("new_user"),
    body: `
      <label class="field required"><span>${h("username")}</span><input name="username" required autocomplete="off"></label>
      <p class="small muted">${h("username_2_64_characters_a_z_0_9")}</p>
      <label class="field required"><span>${h("display_name")}</span><input name="display_name" required></label>
      <label class="field required"><span>${h("password")}</span><input name="password" type="password" required minlength="8" autocomplete="new-password"></label>
      <label class="row"><input name="is_admin" type="checkbox"> ${h("administrator")}</label>`,
    submit: t("create"),
    onSubmit: async ({ form, close }) => {
      await api("POST", "/api/v1/users", {
        username: form.username.value,
        display_name: form.display_name.value,
        password: form.password.value,
        is_admin: form.is_admin.checked,
      });
      markSaved();
      close();
      await route();
    },
  });
}

function openUserDialog(item, teams, initialTab = "") {
  const self = item.id === appState.me.id;
  const memberIds = new Set((item.memberships || []).map((row) => row.team_id));
  const openTeams = teams.filter((team) => !memberIds.has(team.id));
  const rows = (item.memberships || []).map((row) => {
    const org = row.organization_name ? `${row.organization_name} · ` : "";
    const roleCell = row.source === "manual"
      ? `<select data-role="${esc(row.team_id)}" data-current="${esc(row.role)}">${roleOptions(row.role)}</select>`
      : `${esc(roleLabel(row.role))} <span class="small muted">${h("comes_from_the_directory")}</span>`;
    const remove = row.source === "manual"
      ? `<button class="btn danger" type="button" data-leave="${esc(row.team_id)}">${h("remove")}</button>`
      : "";
    return `<tr><td>${esc(org + row.team_name)}</td><td>${roleCell}</td><td>${esc(sourceLabel(row.source))}</td><td>${remove}</td></tr>`;
  });
  const passwordField = item.auth_source === "local"
    ? `<label class="field"><span>${h("new_password")}</span><input name="password" type="password" minlength="8" autocomplete="new-password" placeholder="${h("leave_empty_to_keep_the_password")}"></label>`
    : `<p class="small muted">${h("the_password_comes_from_ldap_or_active_directory")}</p>`;
  const teamChoices = openTeams.map((team) => {
    const label = team.organization_name ? `${team.organization_name} · ${team.name}` : team.name;
    return `<option value="${esc(team.id)}">${esc(label)}</option>`;
  }).join("");
  const account = `
    <p class="small muted">${h("username")}: ${esc(item.username)} · ${esc(authSourceLabel(item.auth_source))}</p>
    <p class="small">${h("email")}: ${emailLine(item.email)}</p>
    <label class="field required"><span>${h("display_name")}</span><input name="display_name" value="${esc(item.display_name)}" required></label>
    <label class="row"><input name="is_admin" type="checkbox"${item.is_admin ? " checked" : ""}> ${h("administrator")}</label>
    <label class="row"><input name="account_disabled" type="checkbox"${item.disabled ? " checked" : ""}${self ? " disabled" : ""}> ${h("disable_account")}</label>
    ${passwordField}
    <div class="form-section">
      <h2>${h("permissions")}</h2>
      ${roleLegend()}
    </div>`;
  const memberships = `
    ${table([t("team"), t("role"), t("source"), ""], rows)}
    ${openTeams.length ? `<div class="form-section">
      <h2>${h("add_to_a_team")}</h2>
      <label class="field"><span>${h("team")}</span><select name="grant_team">${teamChoices}</select></label>
      <div class="field-row">
        <label class="field"><span>${h("role")}</span><select class="fit" name="grant_role">${roleOptions("operator")}</select></label>
      </div>
      <button class="btn pill primary" type="button" data-user-grant>${h("add")}</button>
    </div>` : `<p class="muted">${h("the_user_is_in_every_team")}</p>`}`;
  const dialog = openEditDialog({
    id: "user-dialog",
    title: item.display_name,
    tabs: [
      { id: "account", label: t("account"), body: account },
      { id: "memberships", label: t("memberships"), body: memberships },
    ],
    initialTab,
    submit: t("save"),
    danger: self ? "" : `<button class="btn danger" type="button" data-user-delete>${h("delete")}</button>`,
    onSubmit: async ({ form, close }) => {
      const payload = {
        display_name: form.display_name.value,
        is_admin: form.is_admin.checked,
        disabled: form.account_disabled.checked,
      };
      if (form.password && form.password.value) payload.password = form.password.value;
      await api("PUT", `/api/v1/users/${encodeURIComponent(item.id)}`, payload);
      if (self) appState.setMe(await api("GET", "/api/v1/me"));
      markSaved();
      close();
      await route();
    },
    onTab: (tab) => rememberEditHash({ edit: item.id, tab }, false),
  });
  const remove = dialog.overlay.querySelector("[data-user-delete]");
  if (remove) {
    remove.addEventListener("click", async () => {
      if (!(await confirmDialog({ title: t("delete"), body: t("delete_this_account", { name: item.display_name }), confirmLabel: t("delete"), danger: true }))) return;
      try {
        await api("DELETE", `/api/v1/users/${encodeURIComponent(item.id)}`);
        markSaved();
        dialog.close();
        await route();
      } catch (error) { handle(error); }
    });
  }
  dialog.overlay.querySelectorAll("[data-role]").forEach((select) => {
    select.addEventListener("input", (event) => event.stopPropagation());
    select.addEventListener("change", async (event) => {
      event.stopPropagation();
      const next = select.value;
      if (dialog.dirty() && !(await confirmDialog({ title: t("discard_unsaved_changes"), confirmLabel: t("ok") }))) {
        select.value = select.dataset.current;
        return;
      }
      try {
        await api("POST", `/api/v1/teams/${encodeURIComponent(select.dataset.role)}/members`, {
          username: item.username,
          role: next,
        });
        select.dataset.current = next;
        dialog.clean();
        await route();
      } catch (error) {
        select.value = select.dataset.current;
        handle(error);
      }
    });
  });
  dialog.overlay.querySelectorAll("[data-leave]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (dialog.dirty() && !(await confirmDialog({ title: t("discard_unsaved_changes"), confirmLabel: t("ok") }))) return;
      try {
        await api("DELETE", `/api/v1/teams/${encodeURIComponent(button.dataset.leave)}/members/${encodeURIComponent(item.id)}`);
        dialog.clean();
        await route();
      } catch (error) { handle(error); }
    });
  });
  const grant = dialog.overlay.querySelector("[data-user-grant]");
  if (grant) {
    dialog.overlay.querySelectorAll("[name=grant_team], [name=grant_role]").forEach((field) => {
      field.addEventListener("input", (event) => event.stopPropagation());
      field.addEventListener("change", (event) => event.stopPropagation());
    });
    grant.addEventListener("click", async () => {
      const team = dialog.overlay.querySelector("[name=grant_team]");
      const role = dialog.overlay.querySelector("[name=grant_role]");
      if (dialog.dirty() && !(await confirmDialog({ title: t("discard_unsaved_changes"), confirmLabel: t("ok") }))) return;
      try {
        await api("POST", `/api/v1/teams/${encodeURIComponent(team.value)}/members`, {
          username: item.username,
          role: role.value,
        });
        markSaved();
        dialog.clean();
        await route();
      } catch (error) { handle(error); }
    });
  }
}

function optionSelected(current, value) {
  return current === value ? " selected" : "";
}

function assistantTurnHtml(turn) {
  const fromUser = turn.role === "user";
  return `<div class="assistant-message ${fromUser ? "from-user" : "from-assistant"}">
    <p class="small muted">${h(fromUser ? "assistant_you" : "assistant")}</p>
    <p>${esc(turn.content)}</p>
  </div>`;
}

function assistantActionKey(tool) {
  return {
    launch_template: "assistant_confirm_launch_template",
    launch_workflow: "assistant_confirm_launch_workflow",
    cancel_job: "assistant_confirm_cancel_job",
    relaunch_job: "assistant_confirm_relaunch_job",
    sync_project: "assistant_confirm_sync_project",
    sync_inventory: "assistant_confirm_sync_inventory",
  }[tool] || "assistant_confirmation";
}

function assistantConfirmHtml(action) {
  return `<div class="assistant-confirm" role="group" aria-label="${h("assistant_confirmation")}">
    <p class="small muted">${h("assistant_confirmation")}</p>
    <p><strong>${h(assistantActionKey(action.tool), { name: action.target })}</strong></p>
    <div class="choice-row">
      <button class="btn pill primary" type="button" data-assistant-confirm>${h("confirm")}</button>
      <button class="btn pill" type="button" data-assistant-cancel>${h("cancel")}</button>
    </div>
  </div>`;
}

function bindAssistantAction() {
  const confirm = document.querySelector("[data-assistant-confirm]");
  const cancel = document.querySelector("[data-assistant-cancel]");
  const finish = (text) => {
    appState.setAssistantPendingAction(null);
    appState.assistantTranscript.push({ role: "assistant", content: text });
    renderAssistantThread();
  };
  if (confirm) {
    confirm.addEventListener("click", async () => {
      const action = appState.assistantPendingAction;
      if (!action) return;
      confirm.disabled = true;
      if (cancel) cancel.disabled = true;
      try {
        await api("POST", "/api/v1/assistant/confirm", { id: action.id });
        finish(t("assistant_action_done"));
      } catch (error) {
        if (cancel) cancel.disabled = false;
        confirm.disabled = false;
        handle(error);
      }
    });
  }
  if (cancel) {
    cancel.addEventListener("click", async () => {
      const action = appState.assistantPendingAction;
      if (!action) return;
      confirm.disabled = true;
      cancel.disabled = true;
      try {
        await api("POST", "/api/v1/assistant/cancel", { id: action.id });
      } catch (error) {
        handle(error);
      }
      finish(t("assistant_action_canceled"));
    });
  }
}

function renderAssistantThread() {
  const thread = document.querySelector("#assistant-thread");
  if (!thread) return;
  const turns = appState.assistantTranscript.map(assistantTurnHtml).join("");
  const pending = appState.assistantPending
    ? `<div class="assistant-message from-assistant"><p class="small muted">${h("assistant")}</p><p>${h("assistant_thinking")}</p></div>`
    : "";
  const action = appState.assistantPendingAction ? assistantConfirmHtml(appState.assistantPendingAction) : "";
  thread.innerHTML = turns || pending || action ? `${turns}${pending}${action}` : `<p class="small muted">${h("assistant_empty")}</p>`;
  bindAssistantAction();
  thread.scrollTop = thread.scrollHeight;
}

async function pageAssistant() {
  if (!appState.assistantEnabled) {
    shell(t("assistant"), `<div class="card">${h("assistant_is_not_enabled")}</div>`);
    return;
  }
  const params = new URLSearchParams((location.hash || "").split("?")[1] || "");
  const explainJob = params.get("explain") || "";
  const model = appState.assistantModel ? `<span class="small muted">${h("assistant_model", { model: appState.assistantModel })}</span>` : "";
  shell(t("assistant"), `<div class="card assistant">
    <p class="small muted">${h("assistant_hint")}</p>
    <div class="assistant-thread" id="assistant-thread" aria-live="polite"></div>
    <form class="assistant-compose" id="assistant-form">
      <label class="field"><span>${h("assistant_message")}</span><textarea name="message" rows="3" maxlength="8000" required placeholder="${h("assistant_placeholder")}"></textarea></label>
      <div class="spread"><button class="btn pill primary" id="assistant-send" type="submit">${h("assistant_send")}</button>${model}</div>
    </form>
  </div>`);
  const form = document.querySelector("#assistant-form");
  const button = document.querySelector("#assistant-send");
  renderAssistantThread();
  const send = async (text, jobId) => {
    if (!text || appState.assistantPending || appState.assistantPendingAction) return;
    appState.assistantTranscript.push({ role: "user", content: text });
    form.message.value = "";
    appState.setAssistantPending(true);
    button.disabled = true;
    renderAssistantThread();
    try {
      const body = { message: text, history: appState.assistantTranscript.slice(0, -1).slice(-20) };
      if (jobId) body.job_id = jobId;
      const result = await api("POST", "/api/v1/assistant/chat", body);
      if (result && result.answer) {
        appState.assistantTranscript.push({ role: "assistant", content: String(result.answer) });
      }
      if (result && result.pending_action) appState.setAssistantPendingAction(result.pending_action);
    } catch (error) {
      handle(error);
    } finally {
      appState.setAssistantPending(false);
      button.disabled = false;
      renderAssistantThread();
      form.message.focus();
    }
  };
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    send(form.message.value.trim(), "");
  });
  if (explainJob) {
    history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", "#/assistant");
    appState.setShownHash("#/assistant");
    send(t("assistant_explain_prompt"), explainJob);
  }
}

function settingsBody(form) {
  const secret = (name) => {
    const field = form.elements[name];
    if (!field || field.dataset.storedMask === "1") return "";
    return field.value;
  };
  const clear = [...form.querySelectorAll("[data-clear]:checked")].map((node) => node.getAttribute("data-clear"));
  return {
    ldap: {
      url: form.ldap_url.value.trim(),
      mode: form.ldap_mode.value,
      bind_dn: form.ldap_bind_dn.value.trim(),
      bind_password: secret("ldap_bind_password"),
      user_base: form.ldap_user_base.value.trim(),
      user_filter: form.ldap_user_filter.value.trim(),
      username_attr: form.ldap_username_attr.value.trim(),
      group_attr: form.ldap_group_attr.value.trim(),
      group_base: form.ldap_group_base.value.trim(),
      starttls: form.ldap_starttls.checked,
      tls_insecure: form.ldap_tls_insecure.checked,
    },
    proxy: {
      url: form.proxy_url.value.trim(),
      username: form.proxy_username.value.trim(),
      password: secret("proxy_password"),
      no_proxy: form.proxy_no_proxy.value.trim(),
    },
    runner: form.runner.value,
    job_timeout_secs: Number(form.job_timeout_secs.value),
    job_output_lines: Number(form.job_output_lines.value),
    default_environment_ansible: form.default_environment_ansible.value,
    default_environment_opentofu: form.default_environment_opentofu.value,
    default_environment_shell: form.default_environment_shell.value,
    llm: {
      enabled: form.llm_enabled.checked,
      provider: form.llm_provider.value,
      base_url: form.llm_base_url.value.trim(),
      model: form.llm_model.value.trim(),
      api_key: secret("llm_api_key"),
    },
    clear,
  };
}

function bindStoredMasks(form) {
  form.querySelectorAll("[data-stored-mask]").forEach((field) => {
    field.addEventListener("focus", () => {
      if (field.dataset.storedMask === "1") field.select();
    });
    field.addEventListener("input", () => {
      delete field.dataset.storedMask;
    });
  });
}

function settingsTab() {
  const params = new URLSearchParams((location.hash || "").split("?")[1] || "");
  const tab = params.get("tab");
  return ["directory", "proxy", "jobs", "assistant", "database"].includes(tab) ? tab : "directory";
}

function settingsTabAddress(tab) {
  return tab === "directory" ? "#/settings" : `#/settings?tab=${tab}`;
}

function settingsTabBar(active) {
  const item = (id) => `<button class="btn pill${active === id ? " primary" : ""}" type="button" role="tab" id="settings-tab-${id}" aria-selected="${active === id}" aria-controls="settings-panel-${id}" tabindex="${active === id ? "0" : "-1"}" data-settings-tab="${id}">${h(id)}</button>`;
  return `<div class="spread tab-bar"><div class="tabs" role="tablist" aria-label="${h("settings")}">${["directory", "proxy", "jobs", "assistant", "database"].map(item).join("")}</div></div>`;
}

function databaseTargetBody(form) {
  return {
    host: form.db_host.value.trim(),
    port: Number(form.db_port.value || 5432),
    database: form.db_database.value.trim(),
    user: form.db_user.value.trim(),
    password: form.db_password.value,
    sslmode: form.db_sslmode.value,
    ca_file: form.db_ca_file.value.trim() || null,
  };
}

function databaseChecksHtml(result) {
  const checks = (result && result.checks) || [];
  const label = (id) => {
    if (id === "source_schema") return "check_source_schema";
    if (id === "empty") return "check_empty_target";
    return `check_${id}`;
  };
  const rows = checks.map((check) => `<tr><td>${h(label(check.id))}</td><td><span class="pill ${check.ok ? "good" : "bad"}">${check.ok ? h("check_passed") : h("check_failed")}</span></td><td>${h(check.key)}</td></tr>`);
  const summary = result && result.all_passed
    ? `<p class="banner good">${h("all_checks_passed")}</p>`
    : `<p class="banner bad">${h("fix_the_failed_checks_and_run_them_again")}</p>`;
  return `${summary}${table([t("check"), t("result"), t("reason")], rows)}`;
}

function databaseProgressHtml(status) {
  if (!status || (!status.running && !status.done && !status.failed)) return "";
  const rows = (status.tables || []).map((item) => `<tr><td><code>${esc(item.table)}</code></td><td>${esc(String(item.rows))}</td><td>${item.done ? `<span class="pill good">${h("check_passed")}</span>` : `<span class="pill warn">${h("pending")}</span>`}</td></tr>`);
  const heading = status.failed
    ? `<p class="banner bad">${h("database_migration_failed")}</p>`
    : status.done
      ? `<p class="banner good">${h("database_migration_done")}</p>`
      : `<p class="banner warn">${h("database_migration_running")}</p>`;
  const total = status.done ? `<p class="small muted">${h("database_migration_verified", { rows: String(status.rows || 0) })}</p>` : "";
  return `${heading}${total}${table([t("table"), t("rows"), t("result")], rows)}`;
}

function databaseSwitchHtml(status) {
  if (!status || !status.done || !status.url) return "";
  const url = String(status.url);
  return `<div class="form-section">
    <h3>${h("step_4_switch")}</h3>
    <p>${h("koreograf_does_not_change_its_own_configuration")}</p>
    <h4>${h("docker_env_file")}</h4>
    <pre class="tool">KOREOGRAF_DATABASE_URL='${esc(url)}'
KOREOGRAF_DATABASE_PASSWORD_FILE=/run/secrets/koreograf_database_password</pre>
    <h4>${h("package_env_file")}</h4>
    <pre class="tool">KOREOGRAF_DATABASE_URL='${esc(url)}'</pre>
    <p>${h("restart_koreograf_after_the_change")}</p>
    <p class="small muted">${h("rollback_switches_the_url_back")}</p>
  </div>`;
}

function databaseAuditHtml(status) {
  if (!status || !status.audit) return "";
  return `<p class="small muted">${h("database_migration_audit", { user: status.audit.user, time: status.audit.at, host: status.audit.host })}</p>`;
}

async function pageSettings(notice) {
  if (!appState.me.is_admin) {
    deny("settings", "admin");
    return;
  }
  appState.setDatabaseCheckId(null);
  const item = await api("GET", "/api/v1/settings");
  const environments = appState.teamId ? await api("GET", "/api/v1/environments") : [];
  const ldap = item.ldap || {};
  const proxy = item.proxy || {};
  const llm = item.llm || {};
  const llmProvider = ["openai", "ollama"].includes(llm.provider) ? llm.provider : "openai";
  const mode = ldap.mode === "ad" ? "ad" : "ldap";
  const runner = ["auto", "docker", "local"].includes(item.runner) ? item.runner : "auto";
  const defaultsFor = (tool) => environments.filter((row) => row.scope === "global" && (!row.tool || row.tool === "any" || row.tool === tool));
  const defaultOptions = (tool, selected) => selectOptions(defaultsFor(tool), selected || "", true, (row) => `${row.name} (${environmentToolLabel(row.tool)})`);
  const source = item.source === "saved"
    ? "these_values_are_saved_and_apply_immediately"
    : "nothing_is_saved_yet_the_environment_variables_from";
  const unsupported = proxy.unsupported_env
    ? `<p class="banner warn">${h("the_environment_uses_a_proxy_that_is_neither")}</p>`
    : "";
  const saved = notice ? `<p class="banner good">${h(notice)}</p>` : "";
  const activeTab = settingsTab();
  const panelHidden = (id) => activeTab === id ? "" : " hidden";
  shell(t("settings"), `<form class="card form" id="settings-form">
    ${saved}
    <p class="small muted">${h(source)}</p>
    <p class="small muted">${h("the_bind_address_data_directory_and_first_admin")}</p>
    ${settingsTabBar(activeTab)}
    <section class="form-section settings-panel" id="settings-panel-directory" role="tabpanel" aria-labelledby="settings-tab-directory"${panelHidden("directory")}>
      <h2>${h("directory")}</h2>
      <p class="small muted">${h("ldap_or_active_directory_an_empty_address_turns")}</p>
      <div class="field-row">
        <label class="field"><span>${h("mode")}</span>
          <select class="fit" name="ldap_mode">
            <option value="ldap"${optionSelected(mode, "ldap")}>LDAP</option>
            <option value="ad"${optionSelected(mode, "ad")}>${h("active_directory")}</option>
          </select>
        </label>
      </div>
      <label class="field"><span>${h("address")}</span><input name="ldap_url" value="${esc(ldap.url || "")}" placeholder="ldap://ldap.example.com" autocomplete="off" spellcheck="false"></label>
      <label class="field"><span>${h("bind_dn")}</span><input name="ldap_bind_dn" value="${esc(ldap.bind_dn || "")}" placeholder="cn=svc,dc=example,dc=com" autocomplete="off" spellcheck="false"></label>
      ${secretInput("ldap_bind_password", "bind_password", "password", Boolean(ldap.bind_password_set))}
      <label class="field"><span>${h("user_base")}</span><input name="ldap_user_base" value="${esc(ldap.user_base || "")}" placeholder="ou=users,dc=example,dc=com" autocomplete="off" spellcheck="false"></label>
      <div class="field-grid">
        <label class="field"><span>${h("user_filter")}</span><input name="ldap_user_filter" value="${esc(ldap.user_filter || "")}" spellcheck="false"></label>
        <label class="field"><span>${h("username_attribute")}</span><input name="ldap_username_attr" value="${esc(ldap.username_attr || "")}" spellcheck="false"></label>
        <label class="field"><span>${h("group_attribute")}</span><input name="ldap_group_attr" value="${esc(ldap.group_attr || "")}" spellcheck="false"></label>
        <label class="field"><span>${h("group_base")}</span><input name="ldap_group_base" value="${esc(ldap.group_base || "")}" autocomplete="off" spellcheck="false"></label>
      </div>
      <label class="row"><input name="ldap_starttls" type="checkbox"${ldap.starttls ? " checked" : ""}> ${h("starttls")}</label>
      <label class="row"><input name="ldap_tls_insecure" type="checkbox"${ldap.tls_insecure ? " checked" : ""}> ${h("skip_certificate_checks_only_on_a_network_you")}</label>
    </section>
    <section class="form-section settings-panel" id="settings-panel-proxy" role="tabpanel" aria-labelledby="settings-tab-proxy"${panelHidden("proxy")}>
      <h2>${h("proxy")}</h2>
      <p class="small muted">${h("one_address_is_used_for_http_and_https")}</p>
      <p class="small muted">${h("an_empty_address_turns_the_proxy_off")}</p>
      ${unsupported}
      <label class="field"><span>${h("proxy_address")}</span><input name="proxy_url" value="${esc(proxy.url || "")}" placeholder="http://proxy.example:3128" autocomplete="off" spellcheck="false"></label>
      <div class="field-row">
        <label class="field"><span>${h("proxy_user")}</span><input class="fit" name="proxy_username" value="${esc(proxy.username || "")}" autocomplete="off" spellcheck="false"></label>
        ${secretInput("proxy_password", "proxy_password", "password", Boolean(proxy.password_set))}
      </div>
      <label class="field"><span>${h("exceptions")}</span><input name="proxy_no_proxy" value="${esc(proxy.no_proxy || "")}" placeholder="localhost,127.0.0.1,.internal" autocomplete="off" spellcheck="false"></label>
    </section>
    <section class="form-section settings-panel" id="settings-panel-jobs" role="tabpanel" aria-labelledby="settings-tab-jobs"${panelHidden("jobs")}>
      <div class="spread"><h2>${h("jobs")}</h2><button class="btn pill" id="preflight-check" type="button">${h("check_again")}</button></div>
      <div id="preflight-result"></div>
      <p class="small muted">${h("the_mode_and_the_time_limit_apply_to")}</p>
      <div class="field-row">
        <label class="field"><span>${h("runner")}</span>
          <select class="fit" name="runner">
            <option value="auto"${optionSelected(runner, "auto")}>${h("automatic")}</option>
            <option value="docker"${optionSelected(runner, "docker")}>${h("container")}</option>
            <option value="local"${optionSelected(runner, "local")}>${h("local")}</option>
          </select>
        </label>
        <label class="field"><span>${h("time_limit_in_seconds")}</span><input class="tiny" name="job_timeout_secs" type="number" min="30" max="86400" required value="${esc(String(item.job_timeout_secs || 1800))}"></label>
        <label class="field"><span>${h("output_lines")}</span><input class="tiny" name="job_output_lines" type="number" min="100" max="100000" required value="${esc(String(item.job_output_lines || 8000))}"></label>
      </div>
      <p class="small muted">${h("the_log_keeps_the_first_three_quarters_and_the_last")}</p>
      <p class="small muted">${h("automatic_uses_an_execution_environment_when_docker_or")}</p>
      <p class="small muted">${h("a_template_without_an_environment_uses_the_default")}</p>
      <div class="field-grid">
        <label class="field"><span>${h("default_environment_ansible")}</span><select name="default_environment_ansible">${defaultOptions("ansible", item.default_environment_ansible)}</select></label>
        <label class="field"><span>${h("default_environment_opentofu")}</span><select name="default_environment_opentofu">${defaultOptions("opentofu", item.default_environment_opentofu)}</select></label>
        <label class="field"><span>${h("default_environment_shell")}</span><select name="default_environment_shell">${defaultOptions("shell", item.default_environment_shell)}</select></label>
      </div>
    </section>
    <section class="form-section settings-panel" id="settings-panel-assistant" role="tabpanel" aria-labelledby="settings-tab-assistant"${panelHidden("assistant")}>
      <h2>${h("assistant")}</h2>
      <p class="small muted">${h("assistant_settings_hint")}</p>
      <label class="row"><input name="llm_enabled" type="checkbox"${llm.enabled ? " checked" : ""}> ${h("assistant_enable")}</label>
      <div class="field-row">
        <label class="field"><span>${h("assistant_provider")}</span>
          <select class="fit" name="llm_provider">
            <option value="openai"${optionSelected(llmProvider, "openai")}>${h("assistant_provider_openai")}</option>
            <option value="ollama"${optionSelected(llmProvider, "ollama")}>${h("assistant_provider_ollama")}</option>
          </select>
        </label>
      </div>
      <label class="field"><span>${h("assistant_base_url")}</span><input name="llm_base_url" value="${esc(llm.base_url || "")}" placeholder="http://127.0.0.1:11434/v1" autocomplete="off" spellcheck="false"></label>
      <label class="field"><span>${h("assistant_model_name")}</span><input name="llm_model" value="${esc(llm.model || "")}" placeholder="llama3" autocomplete="off" spellcheck="false"></label>
      ${secretInput("llm_api_key", "assistant_api_key", "password", Boolean(llm.api_key_set))}
      <p class="small muted">${h("assistant_key_hint")}</p>
    </section>
    <section class="form-section settings-panel" id="settings-panel-database" role="tabpanel" aria-labelledby="settings-tab-database"${panelHidden("database")}>
      <h2>${h("move_database_to_postgresql")}</h2>
      <p class="small muted">${h("move_database_to_postgresql_hint")}</p>
      <div id="database-status"></div>
      <h3>${h("step_1_target")}</h3>
      <div class="field-row">
        <label class="field required"><span>${h("database_host")}</span><input name="db_host" autocomplete="off" spellcheck="false" placeholder="db.example.com"></label>
        <label class="field"><span>${h("database_port")}</span><input class="tiny" name="db_port" type="number" min="1" max="65535" value="5432"></label>
      </div>
      <div class="field-grid">
        <label class="field required"><span>${h("database_name")}</span><input name="db_database" autocomplete="off" spellcheck="false" placeholder="koreograf"></label>
        <label class="field required"><span>${h("database_user")}</span><input name="db_user" autocomplete="off" spellcheck="false" placeholder="koreograf"></label>
      </div>
      <label class="field required"><span>${h("database_password")}</span><input name="db_password" type="password" autocomplete="new-password"></label>
      <p class="small muted">${h("migration_password_not_stored")}</p>
      <div class="field-row">
        <label class="field"><span>${h("tls_mode")}</span>
          <select class="fit" name="db_sslmode">
            <option value="require">require</option>
            <option value="verify-ca">verify-ca</option>
            <option value="verify-full">verify-full</option>
            <option value="prefer">prefer</option>
            <option value="disable">disable</option>
          </select>
        </label>
        <label class="field"><span>${h("ca_file_optional")}</span><input name="db_ca_file" autocomplete="off" spellcheck="false" placeholder="/etc/koreograf/postgres-ca.pem"></label>
      </div>
      <h3>${h("step_2_checks")}</h3>
      <div class="choice-row">
        <button class="btn pill" id="database-check" type="button">${h("run_checks")}</button>
      </div>
      <div id="database-checks"></div>
      <h3>${h("step_3_migration")}</h3>
      <p class="small muted">${h("migration_starts_maintenance")}</p>
      <div class="choice-row">
        <button class="btn pill primary" id="database-start" type="button" disabled>${h("start_migration")}</button>
      </div>
      <div id="database-progress"></div>
      <div id="database-switch"></div>
      <div id="database-audit"></div>
    </section>
    <p class="small muted">${h("saving_replaces_the_environment_variables_an_empty_password")}</p>
    <div class="choice-row">
      <button class="btn pill primary" type="submit">${h("save")}</button>
      <button class="btn pill" id="settings-reset" type="button">${h("use_environment_variables")}</button>
    </div>
  </form>`);
  const form = document.querySelector("#settings-form");
  bindStoredMasks(form);
  const tabButtons = [...document.querySelectorAll("[data-settings-tab]")];
  const tabPanels = {
    directory: document.querySelector("#settings-panel-directory"),
    proxy: document.querySelector("#settings-panel-proxy"),
    jobs: document.querySelector("#settings-panel-jobs"),
    assistant: document.querySelector("#settings-panel-assistant"),
    database: document.querySelector("#settings-panel-database"),
  };
  let currentTab = activeTab;
  const chooseTab = (next, focus) => {
    currentTab = ["directory", "proxy", "jobs", "assistant", "database"].includes(next) ? next : "directory";
    tabButtons.forEach((button) => {
      const active = button.dataset.settingsTab === currentTab;
      button.classList.toggle("primary", active);
      button.setAttribute("aria-selected", String(active));
      button.tabIndex = active ? 0 : -1;
    });
    Object.entries(tabPanels).forEach(([id, panel]) => {
      if (panel) panel.hidden = id !== currentTab;
    });
    const address = settingsTabAddress(currentTab);
    if ((location.hash || "#/") !== address) {
      history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", address);
      appState.setShownHash(address);
    }
    if (focus) {
      const active = tabButtons.find((button) => button.dataset.settingsTab === currentTab);
      if (active) active.focus();
    }
  };
  tabButtons.forEach((button, index) => {
    button.addEventListener("click", () => {
      if (button.dataset.settingsTab !== currentTab) chooseTab(button.dataset.settingsTab, false);
    });
    button.addEventListener("keydown", (event) => {
      let target = null;
      if (event.key === "ArrowRight") target = tabButtons[(index + 1) % tabButtons.length];
      else if (event.key === "ArrowLeft") target = tabButtons[(index - 1 + tabButtons.length) % tabButtons.length];
      else if (event.key === "Home") target = tabButtons[0];
      else if (event.key === "End") target = tabButtons[tabButtons.length - 1];
      if (!target) return;
      event.preventDefault();
      chooseTab(target.dataset.settingsTab, true);
    });
  });
  chooseTab(activeTab, false);
  form.ldap_mode.addEventListener("change", () => {
    const ad = form.ldap_mode.value === "ad";
    const next = ad
      ? { filter: "(sAMAccountName={username})", attr: "sAMAccountName" }
      : { filter: "(uid={username})", attr: "uid" };
    const previous = ad
      ? { filter: "(uid={username})", attr: "uid" }
      : { filter: "(sAMAccountName={username})", attr: "sAMAccountName" };
    if (!form.ldap_user_filter.value.trim() || form.ldap_user_filter.value.trim() === previous.filter) {
      form.ldap_user_filter.value = next.filter;
    }
    if (!form.ldap_username_attr.value.trim() || form.ldap_username_attr.value.trim() === previous.attr) {
      form.ldap_username_attr.value = next.attr;
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    try {
      await api("PUT", "/api/v1/settings", settingsBody(form));
      await pageSettings("settings_saved");
    } catch (error) {
      handle(error);
    }
  });
  document.querySelector("#preflight-check").addEventListener("click", async (event) => {
    const button = event.currentTarget;
    button.disabled = true;
    try {
      const report = await api("GET", "/api/v1/runner/preflight");
      document.querySelector("#preflight-result").innerHTML = preflightReportHtml(report);
    } catch (error) {
      handle(error);
    } finally {
      button.disabled = false;
    }
  });
  document.querySelector("#settings-reset").addEventListener("click", async () => {
    if (!(await confirmDialog({ title: t("discard_the_saved_settings_and_use_the_environment"), confirmLabel: t("ok") }))) return;
    try {
      await api("PUT", "/api/v1/settings", { reset: true });
      await pageSettings();
    } catch (error) {
      handle(error);
    }
  });
  const databaseCheck = document.querySelector("#database-check");
  const databaseStart = document.querySelector("#database-start");
  const renderDatabaseStatus = (data) => {
    if (!data) return;
    const status = data.migration || {};
    const supported = data.supported !== false;
    document.querySelector("#database-status").innerHTML = supported
      ? ""
      : `<p class="banner warn">${h("database_already_on_postgresql")}</p>`;
    document.querySelector("#database-progress").innerHTML = databaseProgressHtml(status);
    document.querySelector("#database-switch").innerHTML = databaseSwitchHtml(status);
    document.querySelector("#database-audit").innerHTML = databaseAuditHtml(status);
    databaseCheck.disabled = !supported || Boolean(status.running) || Boolean(status.done);
    databaseStart.disabled = !supported || !appState.databaseCheckId || Boolean(status.running) || Boolean(status.done);
    if (Boolean(status.running) && document.querySelector("#database-progress")) {
      window.setTimeout(() => refreshDatabaseStatus(renderDatabaseStatus), 1000);
    }
  };
  const refreshDatabaseStatus = async (render) => {
    if (!document.querySelector("#database-progress")) return null;
    try {
      const data = await api("GET", "/api/v1/admin/database/status");
      render(data);
      return data;
    } catch (error) {
      if (error.status !== 401) handle(error);
      return null;
    }
  };
  databaseCheck.addEventListener("click", async () => {
    databaseCheck.disabled = true;
    appState.setDatabaseCheckId(null);
    databaseStart.disabled = true;
    try {
      const result = await api("POST", "/api/v1/admin/database/check", databaseTargetBody(form));
      appState.setDatabaseCheckId(result && result.id ? String(result.id) : null);
      document.querySelector("#database-checks").innerHTML = databaseChecksHtml(result);
      databaseStart.disabled = !appState.databaseCheckId;
    } catch (error) {
      handle(error);
    } finally {
      databaseCheck.disabled = false;
    }
  });
  databaseStart.addEventListener("click", async () => {
    if (!appState.databaseCheckId) return;
    if (!(await confirmDialog({ title: t("start_migration"), body: t("migration_starts_maintenance"), confirmLabel: t("start_migration"), danger: true }))) return;
    databaseStart.disabled = true;
    try {
      await api("POST", "/api/v1/admin/database/migrate", { id: appState.databaseCheckId });
      appState.setDatabaseCheckId(null);
      await loadMaintenance();
      await refreshDatabaseStatus(renderDatabaseStatus);
    } catch (error) {
      handle(error);
    }
  });
  refreshDatabaseStatus(renderDatabaseStatus);
}

function backupDownloadUrl(includeTofuState) {
  return includeTofuState ? "/api/v1/admin/backup?tofu_state=true" : "/api/v1/admin/backup";
}

async function pageBackup() {
  if (!appState.me.is_admin) {
    deny("backup", "admin");
    return;
  }
  shell(t("backup"), `<div class="stack">
    <div class="card">
      <p>${h("koreograf_stores_data_in_sqlite_the_backup_contains")}</p>
      <p class="muted">${h("move_database_to_postgresql_hint")}</p>
      <label class="check-row"><input id="backup-tofu-state" type="checkbox"><span>${h("include_opentofu_state")}</span></label>
      <p class="small muted">${h("opentofu_state_can_contain_secrets")}</p>
      <div><button class="btn pill primary" id="backup-download" type="button">${h("download_backup")}</button></div>
    </div>
    <form class="card form danger-zone" id="backup-restore">
      <h2>${h("restore_backup")}</h2>
      <p>${h("this_replaces_organizations_teams_users_projects_inventories_credentials")}</p>
      <label class="field"><span>${h("file")}</span><input name="file" type="file" accept=".zip,application/zip" required></label>
      <label class="row"><input name="confirm" type="checkbox" required> ${h("the_current_installation_will_be_replaced")}</label>
      <button class="btn pill danger" type="submit">${h("restore_backup")}</button>
    </form>
  </div>`);
  document.querySelector("#backup-download").addEventListener("click", async () => {
    const button = document.querySelector("#backup-download");
    button.disabled = true;
    try {
      const headers = {};
      if (appState.teamId) headers["X-Team-Id"] = appState.teamId;
      const includeTofuState = document.querySelector("#backup-tofu-state").checked;
      const response = await fetch(backupDownloadUrl(includeTofuState), { headers, credentials: "same-origin" });
      const blob = await response.blob();
      if (!response.ok) {
        const text = await blob.text();
        let message = "request.failed";
        let vars = null;
        try {
          const data = JSON.parse(text);
          message = data.error || message;
          vars = data.vars || null;
        } catch { /* keep */ }
        const error = new Error(message);
        error.vars = vars;
        throw error;
      }
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = "koreograf-backup.zip";
      link.click();
      URL.revokeObjectURL(url);
    } catch (error) {
      handle(error);
    } finally {
      button.disabled = false;
    }
  });
  document.querySelector("#backup-restore").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const file = form.file.files[0];
    if (!file || !form.confirm.checked) return;
    const button = form.querySelector("button[type=submit]");
    button.disabled = true;
    try {
      const headers = { "X-Restore-Confirm": "ERSETZEN" };
      if (appState.teamId) headers["X-Team-Id"] = appState.teamId;
      const response = await fetch("/api/v1/admin/backup", {
        method: "POST",
        headers,
        body: file,
        credentials: "same-origin",
      });
      const data = await response.json().catch(() => ({}));
      if (!response.ok) {
        const error = new Error(data.error || "request.failed");
        error.vars = data.vars || null;
        throw error;
      }
      appState.setMe(null);
      renderLogin("backup_restored_the_previous_installation_is_restore_previous", "good");
    } catch (error) {
      handle(error);
      button.disabled = false;
    }
  });
}


export async function boot() {
  const catalogs = currentLanguage() === "de" ? ["de"] : ["de", currentLanguage()];
  await Promise.all(catalogs.map(loadCatalog));
  try {
    appState.setMe(await api("GET", "/api/v1/me"));
    const known = (appState.me.teams || []).some((team) => team.id === appState.teamId);
    if (!known) {
      appState.setTeamId(preferredTeam());
      localStorage.setItem("koreograf.team", appState.teamId);
    }
    try {
      const assistant = await api("GET", "/api/v1/assistant");
      appState.setAssistantEnabled(Boolean(assistant && assistant.enabled));
      appState.setAssistantModel(assistant && assistant.model ? String(assistant.model) : "");
    } catch {
      appState.setAssistantEnabled(false);
      appState.setAssistantModel("");
    }
  } catch {
    appState.setMe(null);
  }
  window.addEventListener("hashchange", route);
  route();
}

configureLanguage({
  ask: (options) => confirmDialog(options),
  leaveGuard: () => appState.leaveGuard,
  clearLeaveGuard: () => appState.setLeaveGuard(null),
  signedIn: () => Boolean(appState.me),
  renderLogin: (message) => renderLogin(message),
  route: () => route(),
});
configureDialogs((error) => handle(error));

registerRoute("#/", pageDashboard);
registerRedirect("#/hooks/new", "#/hooks?new=1");
registerRedirect("#/hooks/:id", ({ id }) => `#/hooks?edit=${encodeURIComponent(id)}`);
registerRoute("#/hooks", pageHooks);
registerRedirect("#/environments/new", "#/runtime?new=1");
registerRoute("#/environments/:id", (params) => pageEnvironment(params.id));
registerRoute("#/runtime", pageRuntime);
registerRedirect("#/organizations/new", "#/organizations?new=1");
registerRedirect("#/organizations/:id", ({ id }) => `#/organizations?edit=${encodeURIComponent(id)}`);
registerRoute("#/organizations", pageOrganizations);
registerRoute("#/catalog", () => { location.hash = "#/projects"; });
registerRedirect("#/team/members/new", "#/team?new=member");
registerRedirect("#/team/new", "#/team?new=1");
registerRedirect("#/team/:id", ({ id }) => `#/team?edit=${encodeURIComponent(id)}`);
registerRoute("#/account", pageAccount);
registerRedirect("#/users/new", "#/users?new=1");
registerRedirect("#/users/:id", ({ id }) => `#/users?edit=${encodeURIComponent(id)}`);
registerRoute("#/users", pageUsers);
registerRoute("#/settings", pageSettings);
registerRoute("#/assistant", pageAssistant);
registerRoute("#/backup", pageBackup);
registerRoute("#/team", pageTeam);
