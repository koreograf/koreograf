import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { confirmDialog, openDialog } from "../core/dialog.js";
import { esc, h } from "../core/dom.js";
import { currentLanguage, t } from "../core/i18n.js";
import { registerRoute } from "../core/router.js";
import { can, handle, needTeam, shell } from "../core/app.js";
import { showFieldError } from "../core/forms.js";

const SCHEDULE_ZONES = [
  "Europe/Berlin", "Europe/Vienna", "Europe/Zurich", "Europe/Paris", "Europe/Amsterdam",
  "Europe/London", "UTC", "America/New_York", "America/Chicago", "America/Los_Angeles",
  "Asia/Tokyo", "Asia/Singapore", "Australia/Sydney",
];
const SCHEDULE_DAYS = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

function scheduleQuery(hash = location.hash || "#/") {
  const params = new URLSearchParams((hash || "").split("?")[1] || "");
  return {
    edit: params.get("edit") || "",
    create: params.has("new") ? (params.get("new") || "1") : null,
    kind: params.get("kind") || "",
    target: params.get("target") || "",
  };
}

function scheduleAddress(params, hash = location.hash || "#/") {
  const [path, query] = (hash || "#/").split("?");
  params = params || {};
  const search = new URLSearchParams(query || "");
  ["edit", "new", "kind", "target"].forEach((key) => search.delete(key));
  if (params.edit) search.set("edit", params.edit);
  if (Object.prototype.hasOwnProperty.call(params, "new")) search.set("new", params.new || "1");
  if (params.kind) search.set("kind", params.kind);
  if (params.target) search.set("target", params.target);
  const rest = search.toString();
  return `${path}${rest ? `?${rest}` : ""}`;
}

function scheduleNewTarget(request) {
  const value = request.create || "";
  let kind = request.kind;
  let target = request.target;
  if (value.includes(":")) [kind, target] = value.split(":");
  else if (value.includes("/")) [kind, target] = value.split("/");
  else if (value && value !== "1") target = value;
  if (kind !== "template" && kind !== "workflow") kind = "";
  return target ? { kind, target } : null;
}

function rememberScheduleHash(params, push = true) {
  const next = scheduleAddress(params);
  if (push) history.pushState({ idx: ((history.state && history.state.idx) || 0) + 1 }, "", next);
  else history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", next);
  appState.setShownHash(next);
}

function clearScheduleHash() {
  const next = scheduleAddress();
  if ((location.hash || "#/") === next) return;
  history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", next);
  appState.setShownHash(next);
}

function browserZone() {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "Europe/Berlin";
  } catch (error) {
    return "Europe/Berlin";
  }
}

function zoneChoices(selected) {
  const zones = [...SCHEDULE_ZONES];
  const browser = browserZone();
  if (!zones.includes(browser)) zones.unshift(browser);
  if (selected && !zones.includes(selected)) zones.unshift(selected);
  return zones;
}

function localParts(zone) {
  const parts = new Intl.DateTimeFormat("en-GB", {
    timeZone: zone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
    weekday: "short",
  }).formatToParts(new Date());
  const get = (type) => parts.find((part) => part.type === type)?.value || "";
  const weekday = { Mon: "MO", Tue: "TU", Wed: "WE", Thu: "TH", Fri: "FR", Sat: "SA", Sun: "SU" }[get("weekday")] || "MO";
  let hour = get("hour");
  if (hour === "24") hour = "00";
  return {
    date: `${get("year")}-${get("month")}-${get("day")}`,
    time: `${hour.padStart(2, "0")}:${get("minute").padStart(2, "0")}`,
    weekday,
  };
}

function latestStartDate(date) {
  const value = String(date);
  const year = Number(value.slice(0, 4)) + 10;
  if (value.slice(5, 10) === "02-29") return `${year}-02-28`;
  return `${year}${value.slice(4)}`;
}

function formatLocalDate(isoDate) {
  const [year, month, day] = String(isoDate || "").split("-");
  if (!year || !month || !day) return isoDate || "";
  return currentLanguage() === "de" ? `${day}.${month}.${year}` : `${year}-${month}-${day}`;
}

function dayList(days) {
  return (days || []).map((day) => t(`schedule.day.${String(day).toLowerCase()}`)).join(", ");
}

export function scheduleSentence(item) {
  const date = formatLocalDate(item.start_date);
  const time = item.start_time;
  const zone = item.timezone;
  const day = Number(String(item.start_date || "").slice(8, 10)) || "";
  let text = t("schedule.daily", { time, zone });
  if (item.frequency === "once") text = t("schedule.once", { date, time, zone });
  if (item.frequency === "hourly") text = t("schedule.hourly", { date, time, zone });
  if (item.frequency === "weekly") text = t("schedule.weekly", { days: dayList(item.weekdays), time, zone });
  if (item.frequency === "monthly") text = t("schedule.monthly", { day, time, zone });
  if (item.end_kind === "after" && item.end_count) text = `${text} ${t("schedule.ends_after", { count: item.end_count })}`;
  if (item.end_kind === "on_date" && item.end_date) text = `${text} ${t("schedule.ends_on", { date: formatLocalDate(item.end_date) })}`;
  return text;
}

function formatWhen(iso, zone) {
  if (!iso) return "";
  return new Intl.DateTimeFormat(currentLanguage() === "de" ? "de-DE" : "en-GB", {
    dateStyle: "medium",
    timeStyle: "short",
    timeZone: zone || "UTC",
  }).format(new Date(iso));
}

export function mountSchedules(card) {
  if (!card) return;
  const kind = card.dataset.kind;
  const target = card.dataset.target;
  const all = kind === "all";
  const path = all ? "/api/v1/schedules" : `/api/v1/${kind}s/${encodeURIComponent(target)}/schedules`;
  let items = [];
  let query = "";
  let requested = false;
  const empty = all
    ? h("schedule_empty_all")
    : h(kind === "workflow" ? "schedule_empty_workflow" : "schedule_empty_template");
  const openRequested = async () => {
    if (requested) return;
    requested = true;
    const request = scheduleQuery();
    if (request.edit) {
      const item = items.find((row) => row.id === request.edit);
      if (item) openScheduleForm(item.target_kind || kind, item.target_id || target, item, draw);
      return;
    }
    if (request.create === null) return;
    const chosen = scheduleNewTarget(request);
    let openKind = chosen?.kind || (all ? "" : kind);
    const openTarget = chosen?.target || (all ? "" : target);
    if (all && !openKind && !openTarget) {
      await openNewScheduleDialog(draw);
      return;
    }
    if (all && !openKind && openTarget) {
      const [templates, workflows] = await Promise.all([
        api("GET", "/api/v1/templates"),
        api("GET", "/api/v1/workflows"),
      ]);
      if (templates.some((row) => row.id === openTarget)) openKind = "template";
      else if (workflows.some((row) => row.id === openTarget)) openKind = "workflow";
    }
    if (openKind && openTarget) {
      openScheduleForm(openKind, openTarget, null, draw);
    }
  };
  const render = () => {
    const needle = query.trim().toLowerCase();
    const shown = items
      .filter((item) => !needle || scheduleHaystack(item).includes(needle))
      .sort(compareSchedules);
    const host = card.querySelector(".schedule-table");
    if (all && !items.length) {
      host.innerHTML = `<div class="empty-state">
        <p>${empty}</p>
        <p class="small muted">${h("schedule_empty_hint")}</p>
        ${can("operator") ? `<button class="btn pill primary" type="button" data-schedule-new>${h("schedule_add")}</button>` : ""}
      </div>`;
    } else {
      const message = items.length ? h("nothing_here") : empty;
      host.innerHTML = scheduleTable(shown, all, message);
      bindScheduleActions(card, items, kind, target, draw);
    }
    host.querySelectorAll("[data-schedule-new]").forEach((button) => {
      button.addEventListener("click", openNew);
    });
  };
  const draw = async () => {
    items = await api("GET", path);
    render();
    await openRequested();
  };
  const openNew = () => {
    rememberScheduleHash({ new: "1" });
    openNewScheduleDialog(draw).catch(handle);
  };
  const search = all
    ? `<input class="search" id="schedule-search" placeholder="${h("search_schedules")}" autocomplete="off">`
    : "";
  const add = can("operator") ? `<button class="btn pill primary" type="button" id="schedule-add">${h("schedule_add")}</button>` : "";
  const heading = all ? `<div class="spread">${search}${add}</div>` : `<div class="row"><h2>${h("schedules")}</h2>${add}</div>`;
  card.innerHTML = `${heading}<div class="table-wrap schedule-table"></div>`;
  const searchBox = card.querySelector("#schedule-search");
  if (searchBox) {
    searchBox.addEventListener("input", () => {
      query = searchBox.value;
      render();
    });
  }
  const addButton = card.querySelector("#schedule-add");
  if (addButton) addButton.addEventListener("click", () => {
    if (all) {
      openNew();
      return;
    }
    rememberScheduleHash({ new: "1" });
    openScheduleForm(kind, target, null, draw);
  });
  draw().catch(handle);
}

async function openNewScheduleDialog(redraw) {
  const [templates, workflows] = await Promise.all([
    api("GET", "/api/v1/templates"),
    api("GET", "/api/v1/workflows"),
  ]);
  const choices = [
    ...templates.map((item) => `<option value="template:${esc(item.id)}">${esc(item.name)} · ${h("template")}</option>`),
    ...workflows.map((item) => `<option value="workflow:${esc(item.id)}">${esc(item.name)} · ${h("workflow")}</option>`),
  ].join("");
  if (!choices) {
    clearScheduleHash();
    const card = document.querySelector("#schedule-card");
    if (card) {
      card.querySelectorAll(":scope > .banner.warn").forEach((node) => node.remove());
      card.insertAdjacentHTML("afterbegin", `<div class="banner warn">${h("schedule_needs_target")}</div>`);
    }
    return;
  }
  openScheduleForm("", "", null, redraw, { targets: choices });
}

export function bindScheduleActions(card, items, kind, target, draw, options = {}) {
  card.querySelectorAll("[data-schedule-edit]").forEach((button) => {
    button.addEventListener("click", () => {
      const item = items.find((row) => row.id === button.dataset.scheduleEdit);
      if (item) {
        if (options.hash === false) {
          openScheduleForm(item.target_kind || kind, item.target_id || target, item, draw, { hash: false });
        } else {
          rememberScheduleHash({ edit: item.id });
          openScheduleForm(item.target_kind || kind, item.target_id || target, item, draw);
        }
      }
    });
  });
  card.querySelectorAll("[data-schedule-delete]").forEach((button) => {
    button.addEventListener("click", async () => {
      const item = items.find((row) => row.id === button.dataset.scheduleDelete);
      if (!(await confirmDialog({ title: t("delete"), body: t("delete_this_schedule", { name: item ? item.name : "" }), confirmLabel: t("delete"), danger: true }))) return;
      try {
        await api("DELETE", `/api/v1/schedules/${encodeURIComponent(button.dataset.scheduleDelete)}`);
        await draw();
      } catch (error) {
        handle(error);
      }
    });
  });
  card.querySelectorAll("[data-schedule-enabled]").forEach((control) => {
    const apply = async () => {
      const item = items.find((row) => row.id === control.dataset.scheduleEnabled);
      if (!item) return;
      const enabled = control.type === "checkbox" ? control.checked : !item.enabled;
      try {
        await api("POST", `/api/v1/schedules/${encodeURIComponent(item.id)}/enabled`, { enabled });
        await draw();
      } catch (error) {
        handle(error);
        if (control.isConnected && control.type === "checkbox") control.checked = item.enabled;
      }
    };
    if (control.type === "checkbox") control.addEventListener("change", apply);
    else control.addEventListener("click", apply);
  });
  card.querySelectorAll("[data-schedule-accept], [data-schedule-decline]").forEach((button) => {
    button.addEventListener("click", async () => {
      const id = button.dataset.scheduleAccept || button.dataset.scheduleDecline;
      try {
        await api("POST", `/api/v1/schedules/${encodeURIComponent(id)}/missed`, { accept: Boolean(button.dataset.scheduleAccept) });
        await draw();
      } catch (error) {
        handle(error);
      }
    });
  });
}

export function scheduleHaystack(item) {
  return [
    item.name,
    item.target_name,
    item.timezone,
    item.start_time,
    item.created_by,
    item.updated_by,
    scheduleRepeat(item),
    scheduleEnd(item),
    scheduleSentence(item),
  ].join(" ").toLowerCase();
}

export function compareSchedules(a, b) {
  const rank = (item) => {
    if (item.missed_at) return 0;
    if (item.enabled && item.next_run_at) return 1;
    if (item.enabled) return 2;
    return 3;
  };
  const byRank = rank(a) - rank(b);
  if (byRank) return byRank;
  const left = a.next_run_at || "9999";
  const right = b.next_run_at || "9999";
  if (left !== right) return left < right ? -1 : 1;
  return String(a.name).localeCompare(String(b.name));
}

function scheduleRepeat(item) {
  if (item.frequency === "weekly") {
    const days = (item.weekdays || []).map((day) => t(`schedule.short.${String(day).toLowerCase()}`));
    return days.length ? days.join(", ") : t("weekly");
  }
  if (item.frequency === "monthly") {
    const day = Number(String(item.start_date || "").slice(8, 10)) || "";
    return day ? t("schedule.day_of_month", { day }) : t("monthly");
  }
  return t(item.frequency || "daily");
}

function scheduleEnd(item) {
  if (item.end_kind === "after" && item.end_count) return t("schedule.after_count", { count: item.end_count });
  if (item.end_kind === "on_date" && item.end_date) return formatLocalDate(item.end_date);
  return t("never");
}

function scheduleNext(item) {
  const when = item.next_run_at
    ? esc(formatWhen(item.next_run_at, item.timezone))
    : `<span class="muted">${h("schedule_no_upcoming")}</span>`;
  const missed = item.missed_at ? `<div><span class="pill warn">${h("schedule_missed_state")}</span></div>` : "";
  return `${missed}${when}`;
}

export function scheduleTable(items, showTarget, emptyMessage) {
  const columns = [
    ["name", ""],
    ...(showTarget ? [["schedule_target", ""]] : []),
    ["repeat", ""],
    ["start_time", ""],
    ["next_run", ""],
    ["ends", ""],
    ["schedule_active", "tight"],
    ["actions", "actions"],
  ];
  const body = items.length
    ? items.map((item) => scheduleTableRows(item, showTarget, columns.length)).join("")
    : `<tr><td class="muted" colspan="${columns.length}">${emptyMessage}</td></tr>`;
  const head = columns.map(([key, kind]) => `<th class="${kind}">${esc(t(key))}</th>`).join("");
  return `<table class="table"><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table>`;
}

function scheduleTableRows(item, showTarget, span) {
  const target = showTarget ? `<td>${scheduleTarget(item)}</td>` : "";
  const edit = can("operator") ? `
    <button class="btn compact" type="button" data-schedule-edit="${esc(item.id)}">${h("schedule_edit")}</button>
    <button class="btn compact danger" type="button" data-schedule-delete="${esc(item.id)}">${h("delete")}</button>` : "";
  const editor = item.updated_by || item.created_by;
  const by = editor ? `<div class="small muted">${h("schedule.by", { name: editor })}</div>` : "";
  const owner = item.launch_block === "schedule_needs_a_new_owner" ? `
    <tr class="schedule-alert">
      <td colspan="${span}">
        <div class="schedule-miss"><span>${h("schedule_needs_a_new_owner")}</span></div>
      </td>
    </tr>` : "";
  const alert = item.missed_at ? `
    <tr class="schedule-alert">
      <td colspan="${span}">
        <div class="schedule-miss">
          <span>${h("schedule.missed", { when: formatWhen(item.missed_at, item.timezone) })}</span>
          ${can("operator") ? `<button class="btn compact primary" type="button" data-schedule-accept="${esc(item.id)}">${h("run_missed_now")}</button><button class="btn compact" type="button" data-schedule-decline="${esc(item.id)}">${h("skip_missed")}</button>` : ""}
        </div>
      </td>
    </tr>` : "";
  return `<tr class="${item.missed_at ? "schedule-due" : ""}">
    <td><strong>${esc(item.name)}</strong>${by}</td>
    ${target}
    <td>${esc(scheduleRepeat(item))}</td>
    <td class="num">${esc(item.start_time || "")}<div class="small muted">${esc(item.timezone || "")}</div></td>
    <td class="num">${scheduleNext(item)}</td>
    <td>${esc(scheduleEnd(item))}</td>
    <td class="tight">${scheduleSwitch(item)}</td>
    <td class="actions">${edit}</td>
  </tr>${owner}${alert}`;
}

function scheduleSwitch(item) {
  const checked = item.enabled ? " checked" : "";
  const disabled = can("operator") ? "" : " disabled";
  return `<label class="switch"><input type="checkbox" data-schedule-enabled="${esc(item.id)}" aria-label="${h(item.enabled ? "pause_schedule" : "resume_schedule")}"${checked}${disabled}><span></span></label>`;
}

function scheduleTarget(item) {
  if (!item.target_id) return "";
  const workflow = item.target_kind === "workflow";
  const href = workflow ? `#/workflows/${encodeURIComponent(item.target_id)}` : `#/templates?edit=${encodeURIComponent(item.target_id)}`;
  const label = workflow ? t("workflow") : t("template");
  const name = item.target_name || label;
  return `<a href="${href}">${esc(name)}</a><div class="small muted">${esc(label)}</div>`;
}

export function scheduleNameError(name) {
  return String(name || "").trim() ? "" : "this_field_needs_a_valid_value";
}

export function schedulePayload(values) {
  return {
    timezone: values.timezone,
    start_date: values.start_date,
    start_time: values.start_time,
    frequency: values.frequency,
    weekdays: values.weekdays,
    end_kind: values.frequency === "once" ? "never" : values.end_kind,
    end_count: values.end_kind === "after" ? Number(values.end_count) : null,
    end_date: values.end_kind === "on_date" ? values.end_date : null,
  };
}

export function openScheduleForm(kind, target, existing, redraw, options) {
  options = options || {};
  document.querySelector("#schedule-form")?.remove();
  const outerGuard = appState.leaveGuard;
  const zone = existing?.timezone || browserZone();
  const local = localParts(zone);
  const item = existing || {
    name: "",
    timezone: zoneChoices(zone).includes(zone) ? zone : "Europe/Berlin",
    start_date: local.date,
    start_time: "08:00",
    frequency: "daily",
    weekdays: [local.weekday],
    end_kind: "never",
    end_count: 5,
    end_date: local.date,
  };
  const zones = zoneChoices(item.timezone).map((name) => `<option value="${esc(name)}"${name === item.timezone ? " selected" : ""}>${esc(name)}</option>`).join("");
  const days = SCHEDULE_DAYS.map((day) => `
    <label><input type="checkbox" name="weekday" value="${day}"${(item.weekdays || []).includes(day) ? " checked" : ""}> ${h(`schedule.day.${day.toLowerCase()}`)}</label>`).join("");
  const dialog = openDialog({
    id: "schedule-form",
    formId: "schedule-fields",
    title: existing ? t("edit_schedule") : t("new_schedule"),
    wide: true,
    bodyClass: "schedule-body",
    footer: `<p class="schedule-pin" id="schedule-pin"></p>`,
    body: `
      <div class="schedule-layout">
        <div class="schedule-fields">
          <p class="small muted">${h("fields_marked_with_a_star_are_required")}</p>
          ${options.targets ? `<div class="form-section">
            <h2>${h("schedule_target")}</h2>
            <label class="field required"><span>${h("template_or_workflow")}</span>
              <select name="target_ref" required><option value="">${h("choose_a_target")}</option>${options.targets}</select>
            </label>
            <p class="small muted">${h("schedule_target_hint")}</p>
          </div>` : ""}
          <div class="form-section">
            <h2>${h("general")}</h2>
            <label class="field required"><span>${h("name")}</span><input name="name" required value="${esc(item.name)}"></label>
            <label class="field"><span>${h("time_zone")}</span><select name="timezone">${zones}</select></label>
          </div>
          <div class="form-section">
            <h2>${h("start")}</h2>
            <div class="field-row">
              <label class="field required"><span>${h("start_date")}</span><input name="start_date" type="date" required max="${esc(latestStartDate(local.date))}" value="${esc(item.start_date)}"></label>
              <label class="field required"><span>${h("start_time")}</span><input name="start_time" type="time" required value="${esc(item.start_time)}"></label>
            </div>
          </div>
          <div class="form-section">
            <h2>${h("repeat")}</h2>
            <label class="field"><span>${h("repeat")}</span>
              <select name="frequency">
                ${["once", "hourly", "daily", "weekly", "monthly"].map((value) => `<option value="${value}"${item.frequency === value ? " selected" : ""}>${h(value)}</option>`).join("")}
              </select>
            </label>
            <div class="field" id="schedule-weekdays">
              <span>${h("weekdays")}</span>
              <div class="weekday-row">${days}</div>
            </div>
          </div>
          <div class="form-section">
            <h2>${h("end")}</h2>
            <div class="field-row" id="schedule-end">
              <label class="field"><span>${h("ends")}</span>
                <select name="end_kind">
                  <option value="never"${item.end_kind === "never" ? " selected" : ""}>${h("never")}</option>
                  <option value="after"${item.end_kind === "after" ? " selected" : ""}>${h("after_n_occurrences")}</option>
                  <option value="on_date"${item.end_kind === "on_date" ? " selected" : ""}>${h("on_a_date")}</option>
                </select>
              </label>
              <label class="field" id="schedule-count"><span>${h("times")}</span><input class="tiny" name="end_count" type="number" min="1" max="999" value="${esc(item.end_count || 5)}"></label>
              <label class="field" id="schedule-until"><span>${h("end_date")}</span><input name="end_date" type="date" value="${esc(item.end_date || local.date)}"></label>
            </div>
          </div>
        </div>
        <aside class="schedule-preview">
          <div class="form-section">
            <h2>${h("preview")}</h2>
            <p id="schedule-sentence"></p>
            <p class="small muted">${h("schedule_preview")}</p>
            <ul id="schedule-next"></ul>
          </div>
        </aside>
      </div>`,
    submit: t("save"),
    onSubmit: async ({ form, close }) => {
      if (!form.name.value.trim()) {
        showFieldError(form.name, t(scheduleNameError(form.name.value)));
        form.name.focus();
        return;
      }
      const body = { ...payload(), name: form.name.value };
      if (existing) await api("PUT", `/api/v1/schedules/${encodeURIComponent(existing.id)}`, body);
      else {
        const [postKind, postTarget] = form.target_ref ? form.target_ref.value.split(":") : [kind, target];
        await api("POST", `/api/v1/${postKind}s/${encodeURIComponent(postTarget)}/schedules`, body);
      }
      appState.setLeaveGuard(null);
      close();
      await redraw();
      const card = document.querySelector("#schedule-card");
      if (card) {
        card.querySelectorAll(":scope > .banner.good").forEach((node) => node.remove());
        card.insertAdjacentHTML("afterbegin", `<div class="banner good">${h("saved")}</div>`);
      }
    },
    onClose: () => {
      appState.setLeaveGuard(null);
      if (outerGuard) appState.setLeaveGuard(outerGuard);
      if (options.hash !== false) clearScheduleHash();
    },
  });
  appState.setLeaveGuard(() => dialog.dirty());
  const form = dialog.form;
  const showParts = () => {
    const weekly = form.frequency.value === "weekly";
    const ends = form.frequency.value !== "once";
    const after = ends && form.end_kind.value === "after";
    const onDate = ends && form.end_kind.value === "on_date";
    dialog.overlay.querySelector("#schedule-weekdays").hidden = !weekly;
    dialog.overlay.querySelector("#schedule-end").hidden = !ends;
    dialog.overlay.querySelector("#schedule-count").hidden = !after;
    dialog.overlay.querySelector("#schedule-count").classList.toggle("required", after);
    form.end_count.required = after;
    dialog.overlay.querySelector("#schedule-until").hidden = !onDate;
    dialog.overlay.querySelector("#schedule-until").classList.toggle("required", onDate);
    form.end_date.required = onDate;
  };
  const payload = () => schedulePayload({
    timezone: form.timezone.value,
    start_date: form.start_date.value,
    start_time: form.start_time.value,
    frequency: form.frequency.value,
    weekdays: [...form.querySelectorAll("input[name=weekday]:checked")].map((box) => box.value),
    end_kind: form.end_kind.value,
    end_count: form.end_count.value,
    end_date: form.end_date.value,
  });
  let previewTimer = 0;
  const preview = () => {
    window.clearTimeout(previewTimer);
    previewTimer = window.setTimeout(async () => {
      const body = payload();
      const sentence = scheduleSentence({ ...body, name: form.name.value });
      dialog.overlay.querySelector("#schedule-sentence").textContent = sentence;
      dialog.overlay.querySelector("#schedule-pin").textContent = sentence;
      try {
        const result = await api("POST", "/api/v1/schedules/preview", body);
        const lines = (result.occurrences || []).map((at) => `<li>${esc(formatWhen(at, body.timezone))}</li>`).join("");
        dialog.overlay.querySelector("#schedule-next").innerHTML = lines || `<li>${h("schedule_no_upcoming")}</li>`;
      } catch (error) {
        dialog.overlay.querySelector("#schedule-next").innerHTML = `<li>${esc(t(error.message || "error.internal", error.vars))}</li>`;
      }
    }, 200);
  };
  form.addEventListener("input", () => {
    showParts();
    preview();
  });
  form.addEventListener("change", () => {
    showParts();
    preview();
  });
  showParts();
  preview();
}

async function pageSchedules() {
  if (!needTeam()) return;
  shell(t("schedules"), `<section class="card" id="schedule-card" data-kind="all"></section>`);
  mountSchedules(document.querySelector("#schedule-card"));
}

registerRoute("#/schedules", pageSchedules);
