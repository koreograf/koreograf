import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { emptyState, esc, h, table } from "../core/dom.js";
import { t } from "../core/i18n.js";
import { registerRoute } from "../core/router.js";
import { handle, needTeam, shell } from "../core/app.js";
import { pill, relativeTime, when } from "../core/format.js";

const ACTIONS = ["sync", "create", "update", "delete", "launch", "cancel", "relaunch"];
const OBJECT_TYPES = ["project", "inventory", "inventory_source", "template", "credential", "environment", "schedule", "workflow", "job", "workflow_run"];

const ACTION_KEYS = {
  sync: "action_sync",
  create: "action_create",
  update: "action_update",
  delete: "action_delete",
  launch: "action_launch",
  cancel: "action_cancel",
  relaunch: "action_relaunch",
};

const OBJECT_KEYS = {
  project: "object_project",
  inventory: "object_inventory",
  inventory_source: "object_inventory_source",
  template: "object_template",
  credential: "object_credential",
  environment: "object_environment",
  schedule: "object_schedule",
  workflow: "object_workflow",
  job: "object_job",
  workflow_run: "object_workflow_run",
};

export function actionLabel(action) {
  const key = ACTION_KEYS[action];
  return key ? t(key) : action;
}

export function objectLabel(objectType) {
  const key = OBJECT_KEYS[objectType];
  return key ? t(key) : objectType;
}

export function activityState(hash = location.hash || "") {
  const params = new URLSearchParams((hash || "").split("?")[1] || "");
  const size = Number(params.get("size"));
  return {
    page: Math.max(1, Number(params.get("page")) || 1),
    size: [25, 50, 100].includes(size) ? size : 25,
    actor: params.get("actor") || "",
    action: params.get("action") || "",
    object: params.get("object") || "",
    from: params.get("from") || "",
    to: params.get("to") || "",
  };
}

export function activityHash(state) {
  const params = new URLSearchParams();
  if (state.page > 1) params.set("page", String(state.page));
  if (state.size !== 25) params.set("size", String(state.size));
  if (state.actor) params.set("actor", state.actor);
  if (state.action) params.set("action", state.action);
  if (state.object) params.set("object", state.object);
  if (state.from) params.set("from", state.from);
  if (state.to) params.set("to", state.to);
  const query = params.toString();
  return `#/activity${query ? `?${query}` : ""}`;
}

function dateBound(value, end) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(String(value || ""))) return "";
  return `${value}T${end ? "23:59:59" : "00:00:00"}Z`;
}

export function activityUrl(state) {
  const params = new URLSearchParams();
  params.set("limit", String(state.size));
  params.set("offset", String((state.page - 1) * state.size));
  if (state.actor) params.set("actor", state.actor);
  if (state.action) params.set("action", state.action);
  if (state.object) params.set("object_type", state.object);
  const since = dateBound(state.from, false);
  const until = dateBound(state.to, true);
  if (since) params.set("since", since);
  if (until) params.set("until", until);
  return `/api/v1/activity?${params.toString()}`;
}

export function objectHref(item) {
  const id = encodeURIComponent(item.object_id || "");
  if (!id) return "";
  switch (item.object_type) {
    case "project": return `#/projects/${id}`;
    case "inventory": return `#/inventories/${id}`;
    case "template": return `#/templates?edit=${id}`;
    case "credential": return `#/credentials?edit=${id}`;
    case "environment": return `#/runtime?edit=${id}`;
    case "workflow": return `#/workflows/${id}`;
    case "job": return `#/jobs/${id}`;
    case "workflow_run": return `#/runs/${id}`;
    default: return "";
  }
}

export function activityTable(items) {
  const rows = items.map((item) => {
    const label = item.object_name || item.object_id || "";
    const href = objectHref(item);
    const object = href ? `<a href="${href}">${esc(label)}</a>` : esc(label);
    const result = item.result === "success" ? "successful" : "failed";
    return `<tr><td class="small muted"><time datetime="${esc(item.created_at)}" title="${esc(when(item.created_at))}">${esc(when(item.created_at))}</time><br><span class="small">${esc(relativeTime(item.created_at))}</span></td><td>${esc(item.actor)}</td><td>${esc(actionLabel(item.action))}</td><td>${object}<div class="small muted">${esc(objectLabel(item.object_type))}</div></td><td>${pill(result)}</td></tr>`;
  });
  return table([t("column_time"), t("column_actor"), t("column_action"), t("column_object"), t("column_result")], rows);
}

function selectChoices(values, selected, label) {
  return [
    `<option value=""${selected ? "" : " selected"}>${h("all")}</option>`,
    ...values.map((value) => `<option value="${esc(value)}"${value === selected ? " selected" : ""}>${esc(label(value))}</option>`),
  ].join("");
}

async function pageActivity() {
  if (!needTeam()) return;
  const view = activityState();
  shell(t("activity"), `
    <section class="card">
      <p class="small muted">${h("activity_help")}</p>
      <div class="filter-bar">
        <label class="field"><span>${h("filter_actor")}</span><input class="search" id="activity-actor" type="search" value="${esc(view.actor)}" placeholder="${h("search_actor")}" aria-label="${h("search_actor")}" autocomplete="off"></label>
        <label class="field"><span>${h("filter_action")}</span><select id="activity-action" aria-label="${h("filter_action")}">${selectChoices(ACTIONS, view.action, actionLabel)}</select></label>
        <label class="field"><span>${h("filter_object_type")}</span><select id="activity-object" aria-label="${h("filter_object_type")}">${selectChoices(OBJECT_TYPES, view.object, objectLabel)}</select></label>
        <label class="field"><span>${h("filter_from")}</span><input id="activity-from" type="date" value="${esc(view.from)}"></label>
        <label class="field"><span>${h("filter_to")}</span><input id="activity-to" type="date" value="${esc(view.to)}"></label>
        <label class="field"><span>${h("page_size")}</span><select id="activity-size" aria-label="${h("page_size")}">${[25, 50, 100].map((size) => `<option value="${size}"${size === view.size ? " selected" : ""}>${size}</option>`).join("")}</select></label>
      </div>
      <p class="small muted" id="activity-total" tabindex="-1" aria-live="polite"></p>
      <div id="activity-table"></div>
      <div class="row" id="activity-pager" tabindex="-1"></div>
    </section>`);
  const actor = document.querySelector("#activity-actor");
  const action = document.querySelector("#activity-action");
  const object = document.querySelector("#activity-object");
  const from = document.querySelector("#activity-from");
  const to = document.querySelector("#activity-to");
  const size = document.querySelector("#activity-size");
  const activityHost = document.querySelector("#activity-table");
  const activityTotal = document.querySelector("#activity-total");
  const activityPager = document.querySelector("#activity-pager");
  const rememberHash = (push = false) => {
    const next = activityHash(view);
    if (push) history.pushState({ idx: ((history.state && history.state.idx) || 0) + 1 }, "", next);
    else history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", next);
    appState.setShownHash(next);
  };
  const load = async (focus) => {
    activityTotal.textContent = t("loading");
    try {
      const data = await api("GET", activityUrl(view));
      const totalPages = Math.max(1, Math.ceil(Number(data.total) / view.size));
      if (view.page > totalPages) {
        view.page = totalPages;
        rememberHash();
        return load(focus);
      }
      draw(data);
      const target = focus ? document.querySelector(focus) : null;
      (target || activityPager).focus();
    } catch (error) {
      activityTotal.textContent = "";
      handle(error);
    }
  };
  const draw = (data) => {
    const items = data.items || [];
    const total = Number(data.total) || 0;
    const filters = view.actor || view.action || view.object || view.from || view.to;
    if (!items.length) {
      activityHost.innerHTML = filters
        ? `<p class="small muted">${h("no_activity_match")}</p><button class="btn" id="clear-activity-filter" type="button">${h("clear_filter")}</button>`
        : emptyState("empty_activity");
    } else {
      activityHost.innerHTML = activityTable(items);
    }
    const offset = (view.page - 1) * view.size;
    const begin = total ? Math.min(offset + 1, total) : 0;
    const end = Math.min(offset + items.length, total);
    activityTotal.textContent = total ? t("activity_range", { from: begin, to: end, total }) : filters ? t("no_activity_match") : t("nothing_here");
    const totalPages = Math.max(1, Math.ceil(total / view.size));
    activityPager.innerHTML = totalPages > 1
      ? `<button class="btn" type="button" data-activity-page="${view.page - 1}"${view.page <= 1 ? " disabled" : ""}>${h("previous_page")}</button>
        <span class="small muted">${h("page_of_pages", { page: view.page, pages: totalPages })}</span>
        <button class="btn" type="button" data-activity-page="${view.page + 1}"${view.page >= totalPages ? " disabled" : ""}>${h("next_page")}</button>`
      : "";
  };
  const resetFilters = () => {
    view.actor = "";
    view.action = "";
    view.object = "";
    view.from = "";
    view.to = "";
    view.page = 1;
    actor.value = "";
    action.value = "";
    object.value = "";
    from.value = "";
    to.value = "";
    rememberHash();
    load("#activity-actor");
  };
  actor.addEventListener("input", () => {
    if (actor.activityTimer) window.clearTimeout(actor.activityTimer);
    actor.activityTimer = window.setTimeout(() => {
      view.actor = actor.value.trim();
      view.page = 1;
      rememberHash();
      load("#activity-actor");
    }, 250);
  });
  action.addEventListener("change", () => {
    view.action = action.value;
    view.page = 1;
    rememberHash(true);
    load("#activity-action");
  });
  object.addEventListener("change", () => {
    view.object = object.value;
    view.page = 1;
    rememberHash(true);
    load("#activity-object");
  });
  from.addEventListener("change", () => {
    view.from = from.value;
    view.page = 1;
    rememberHash(true);
    load("#activity-from");
  });
  to.addEventListener("change", () => {
    view.to = to.value;
    view.page = 1;
    rememberHash(true);
    load("#activity-to");
  });
  size.addEventListener("change", () => {
    view.size = Number(size.value) || 25;
    view.page = 1;
    rememberHash(true);
    load("#activity-size");
  });
  activityHost.addEventListener("click", (event) => {
    if (event.target.closest("#clear-activity-filter")) resetFilters();
  });
  activityPager.addEventListener("click", (event) => {
    const button = event.target.closest("[data-activity-page]");
    if (!button || button.disabled) return;
    view.page = Number(button.dataset.activityPage);
    rememberHash(true);
    load(`[data-activity-page="${view.page}"]`);
  });
  await load();
}

registerRoute("#/activity", pageActivity);
