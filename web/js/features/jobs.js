import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { confirmDialog } from "../core/dialog.js";
import { emptyState, esc, h, paramRow, paramRowHtml, table } from "../core/dom.js";
import { t } from "../core/i18n.js";
import { registerRoute } from "../core/router.js";
import { can, go, handle, needTeam, shell } from "../core/app.js";
import { chipReadout } from "../core/chips.js";
import { credentialKindLabel } from "../core/credentials.js";
import { pill, relativeTime, spanText, statusLabel, verbosityLabel, when } from "../core/format.js";
import { explainFailure } from "./assistant.js";

export function jobsState(hash = location.hash || "") {
  const params = new URLSearchParams((hash || "").split("?")[1] || "");
  const size = Number(params.get("size"));
  return {
    page: Math.max(1, Number(params.get("page")) || 1),
    size: [25, 50, 100].includes(size) ? size : 25,
    q: params.get("q") || "",
    status: params.get("status") || "",
    tool: params.get("tool") || "",
    trigger: params.get("trigger") || "",
    template: params.get("template") || "",
  };
}

export function jobsHash(state) {
  const params = new URLSearchParams();
  if (state.page > 1) params.set("page", String(state.page));
  if (state.size !== 25) params.set("size", String(state.size));
  if (state.q) params.set("q", state.q);
  if (state.status) params.set("status", state.status);
  if (state.tool) params.set("tool", state.tool);
  if (state.trigger) params.set("trigger", state.trigger);
  if (state.template) params.set("template", state.template);
  const query = params.toString();
  return `#/jobs${query ? `?${query}` : ""}`;
}

export function jobsUrl(state) {
  const params = new URLSearchParams();
  params.set("limit", String(state.size));
  params.set("offset", String((state.page - 1) * state.size));
  if (state.q) params.set("q", state.q);
  if (state.status) params.set("status", state.status);
  if (state.tool) params.set("tool", state.tool);
  if (state.trigger) params.set("trigger", state.trigger);
  if (state.template) params.set("template_id", state.template);
  return `/api/v1/jobs?${params.toString()}`;
}

async function pageJobs() {
  if (!needTeam()) return;
  const templates = await api("GET", "/api/v1/templates");
  const view = jobsState();
  const choices = (values, selected, label) => [
    `<option value=""${selected ? "" : " selected"}>${h("all")}</option>`,
    ...values.map((value) => `<option value="${esc(value)}"${value === selected ? " selected" : ""}>${esc(label(value))}</option>`),
  ].join("");
  const templateOptions = [
    `<option value=""${view.template ? "" : " selected"}>${h("all")}</option>`,
    ...templates.map((item) => `<option value="${esc(item.id)}"${item.id === view.template ? " selected" : ""}>${esc(item.name)}</option>`),
  ].join("");
  shell(t("jobs"), `
    <section class="card">
      <div class="filter-bar">
        <input class="search" id="job-search" type="search" value="${esc(view.q)}" placeholder="${h("search_jobs")}" aria-label="${h("search_jobs")}" autocomplete="off">
        <label class="field"><span>${h("status")}</span><select id="job-status" aria-label="${h("status")}">${choices(["pending", "running", "successful", "failed", "canceled"], view.status, statusLabel)}</select></label>
        <label class="field"><span>${h("tool")}</span><select id="job-tool" aria-label="${h("tool")}">${choices(["ansible", "opentofu", "shell"], view.tool, (value) => t(value))}</select></label>
        <label class="field"><span>${h("trigger")}</span><select id="job-trigger" aria-label="${h("trigger")}">${choices(["manual", "schedule", "hook", "workflow", "relaunch"], view.trigger, (value) => t(value))}</select></label>
        <label class="field"><span>${h("template")}</span><select id="job-template" aria-label="${h("template")}">${templateOptions}</select></label>
        <label class="field"><span>${h("page_size")}</span><select id="job-size" aria-label="${h("page_size")}">${[25, 50, 100].map((size) => `<option value="${size}"${size === view.size ? " selected" : ""}>${size}</option>`).join("")}</select></label>
        <button class="btn pill${view.status === "failed" ? " primary" : ""}" id="job-only-failed" type="button">${h("only_failed")}</button>
      </div>
      <p class="small muted" id="job-total" tabindex="-1" aria-live="polite"></p>
      <div id="job-table"></div>
      <div class="row" id="job-pager" tabindex="-1"></div>
    </section>`);
  const search = document.querySelector("#job-search");
  const status = document.querySelector("#job-status");
  const tool = document.querySelector("#job-tool");
  const trigger = document.querySelector("#job-trigger");
  const template = document.querySelector("#job-template");
  const size = document.querySelector("#job-size");
  const failed = document.querySelector("#job-only-failed");
  const jobTable = document.querySelector("#job-table");
  const jobTotal = document.querySelector("#job-total");
  const jobPager = document.querySelector("#job-pager");
  let current = [];
  const rememberHash = (push = false) => {
    const next = jobsHash(view);
    if (push) history.pushState({ idx: ((history.state && history.state.idx) || 0) + 1 }, "", next);
    else history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", next);
    appState.setShownHash(next);
  };
  const load = async (focus) => {
    jobTotal.textContent = t("loading");
    try {
      const data = await api("GET", jobsUrl(view));
      const totalPages = Math.max(1, Math.ceil(Number(data.total) / view.size));
      if (view.page > totalPages) {
        view.page = totalPages;
        rememberHash();
        return load(focus);
      }
      draw(data);
      const target = focus ? document.querySelector(focus) : null;
      (target || jobPager).focus();
    } catch (error) {
      jobTotal.textContent = "";
      handle(error);
    }
  };
  const draw = (data) => {
    current = data.items || [];
    const total = Number(data.total) || 0;
    const filters = view.q || view.status || view.tool || view.trigger || view.template;
    if (!current.length) {
      jobTable.innerHTML = filters
        ? `<p class="small muted">${h("no_jobs_match")}</p><button class="btn" id="clear-job-filter" type="button">${h("clear_filter")}</button>`
        : emptyState("empty_jobs", can("operator") ? "#/templates" : "", "launch_a_template");
    } else {
      const rows = current.map((job) =>
        `<tr><td><a href="#/jobs/${esc(job.id)}">${esc(job.name)}</a></td><td class="tool narrow-hide">${esc(job.tool)}</td><td>${pill(job.status)}</td><td class="small narrow-hide">${esc(job.trigger)}</td><td class="small">${esc(job.launched_by)}</td><td class="small muted"><time datetime="${esc(job.created_at)}" title="${esc(when(job.created_at))}">${esc(when(job.created_at))}</time><br><span class="small">${esc(relativeTime(job.created_at))}</span></td><td class="small muted narrow-hide">${esc(spanText(job.started_at, job.finished_at, job.status))}</td><td>${jobListActions(job)}</td></tr>`
      );
      jobTable.innerHTML = table([t("name"), t("tool"), t("status"), t("trigger"), t("launched_by"), t("created"), t("duration"), ""], rows);
    }
    const offset = (view.page - 1) * view.size;
    const from = total ? Math.min(offset + 1, total) : 0;
    const to = Math.min(offset + current.length, total);
    jobTotal.textContent = total ? t("jobs_range", { from, to, total }) : filters ? t("no_jobs_match") : t("nothing_here");
    const totalPages = Math.max(1, Math.ceil(total / view.size));
    jobPager.innerHTML = totalPages > 1
      ? `<button class="btn" type="button" data-job-page="${view.page - 1}"${view.page <= 1 ? " disabled" : ""}>${h("previous_page")}</button>
        <span class="small muted">${h("page_of_pages", { page: view.page, pages: totalPages })}</span>
        <button class="btn" type="button" data-job-page="${view.page + 1}"${view.page >= totalPages ? " disabled" : ""}>${h("next_page")}</button>`
      : "";
  };
  const resetFilters = () => {
    view.q = "";
    view.status = "";
    view.tool = "";
    view.trigger = "";
    view.template = "";
    view.page = 1;
    search.value = "";
    status.value = "";
    tool.value = "";
    trigger.value = "";
    template.value = "";
    failed.classList.remove("primary");
    rememberHash();
    load("#job-search");
  };
  search.addEventListener("input", () => {
    if (search.jobTimer) window.clearTimeout(search.jobTimer);
    search.jobTimer = window.setTimeout(() => {
      view.q = search.value.trim();
      view.page = 1;
      rememberHash();
      load("#job-search");
    }, 250);
  });
  status.addEventListener("change", () => {
    view.status = status.value;
    view.page = 1;
    failed.classList.toggle("primary", view.status === "failed");
    rememberHash(true);
    load("#job-status");
  });
  tool.addEventListener("change", () => {
    view.tool = tool.value;
    view.page = 1;
    rememberHash(true);
    load("#job-tool");
  });
  trigger.addEventListener("change", () => {
    view.trigger = trigger.value;
    view.page = 1;
    rememberHash(true);
    load("#job-trigger");
  });
  template.addEventListener("change", () => {
    view.template = template.value;
    view.page = 1;
    rememberHash(true);
    load("#job-template");
  });
  size.addEventListener("change", () => {
    view.size = Number(size.value) || 25;
    view.page = 1;
    rememberHash(true);
    load("#job-size");
  });
  failed.addEventListener("click", () => {
    view.status = view.status === "failed" ? "" : "failed";
    view.page = 1;
    status.value = view.status;
    failed.classList.toggle("primary", view.status === "failed");
    rememberHash(true);
    load("#job-only-failed");
  });
  jobTable.addEventListener("click", async (event) => {
    if (event.target.closest("#clear-job-filter")) {
      resetFilters();
      return;
    }
    const button = event.target.closest("[data-relaunch]");
    if (!button) return;
    const job = current.find((item) => item.id === button.dataset.relaunch);
    if (!job) return;
    if (!(await confirmDialog({ title: t("relaunch"), body: relaunchPrompt(job, button.dataset.failed === "1"), confirmLabel: t("relaunch") }))) return;
    button.disabled = true;
    try {
      const copy = await api("POST", `/api/v1/jobs/${encodeURIComponent(button.dataset.relaunch)}/relaunch`, {
        only_failed_hosts: button.dataset.failed === "1",
      });
      go(`#/jobs/${copy.id}`);
    } catch (error) {
      button.disabled = false;
      handle(error);
    }
  });
  jobPager.addEventListener("click", (event) => {
    const button = event.target.closest("[data-job-page]");
    if (!button || button.disabled) return;
    view.page = Number(button.dataset.jobPage);
    rememberHash(true);
    load(`[data-job-page="${view.page}"]`);
  });
  await load();
}

function plainLogLine(text) {
  return String(text || "")
    .replace(/\u001b\][^\u001b\u0007]*(?:\u0007|\u001b\\)/g, "")
    .replace(/\u001b\[[0-?]*[ -/]*[@-~]/g, "");
}

// SGR foreground and background codes become allowlisted classes. Anything the
// renderer does not know is dropped, so the log never carries raw escape text.
const ANSI_FG = {
  30: "black",
  31: "red",
  32: "green",
  33: "yellow",
  34: "blue",
  35: "magenta",
  36: "cyan",
  37: "white",
  90: "bright-black",
  91: "bright-red",
  92: "bright-green",
  93: "bright-yellow",
  94: "bright-blue",
  95: "bright-magenta",
  96: "bright-cyan",
  97: "bright-white",
};

const ANSI_BG = {
  40: "black",
  41: "red",
  42: "green",
  43: "yellow",
  44: "blue",
  45: "magenta",
  46: "cyan",
  47: "white",
  100: "bright-black",
  101: "bright-red",
  102: "bright-green",
  103: "bright-yellow",
  104: "bright-blue",
  105: "bright-magenta",
  106: "bright-cyan",
  107: "bright-white",
};

const ANSI_CSI = /^\u001b\[([0-?]*)([ -/]*)([@-~])/;
const ANSI_CSI_PARTIAL = /^\u001b\[[0-?]*[ -/]*/;
const ANSI_ESC = /^\u001b(?:[ -/]*)([@-~])/;

function ansiState() {
  return { fg: "", bg: "", bold: false, dim: false, italic: false, underline: false };
}

function resetAnsi(state) {
  state.fg = "";
  state.bg = "";
  state.bold = false;
  state.dim = false;
  state.italic = false;
  state.underline = false;
}

function ansiClasses(state) {
  const classes = [];
  if (state.bold) classes.push("ansi-bold");
  if (state.dim) classes.push("ansi-dim");
  if (state.italic) classes.push("ansi-italic");
  if (state.underline) classes.push("ansi-underline");
  if (state.fg) classes.push(`ansi-fg-${state.fg}`);
  if (state.bg) classes.push(`ansi-bg-${state.bg}`);
  return classes;
}

function applySgr(state, params) {
  const parts = params === "" ? ["0"] : params.split(/[;:]/);
  for (let index = 0; index < parts.length; index += 1) {
    const part = parts[index];
    if (part === "") {
      resetAnsi(state);
      continue;
    }
    if (!/^\d+$/.test(part)) continue;
    const code = Number(part);
    // Extended colours (38, 48, 58) take extra parameters that must not be
    // read as bold or underline.
    if (code === 38 || code === 48 || code === 58) {
      const mode = parts[index + 1];
      if (mode === "5") index += 2;
      else if (mode === "2") index += 4;
      else index += 1;
      continue;
    }
    if (code === 0) resetAnsi(state);
    else if (code === 1) state.bold = true;
    else if (code === 2) state.dim = true;
    else if (code === 3) state.italic = true;
    else if (code === 4) state.underline = true;
    else if (code === 21 || code === 22) {
      state.bold = false;
      state.dim = false;
    } else if (code === 23) state.italic = false;
    else if (code === 24) state.underline = false;
    else if (code === 39) state.fg = "";
    else if (code === 49) state.bg = "";
    else if (ANSI_FG[code]) state.fg = ANSI_FG[code];
    else if (ANSI_BG[code]) state.bg = ANSI_BG[code];
  }
}

/** The end of one escape sequence, or the next index after a broken one. */
function ansiEscape(input, start) {
  const rest = input.slice(start);
  const stringControl = /^\u001b[\]PX^_]/.test(rest);
  if (stringControl) {
    const end = rest.search(/\u0007|\u001b\\/);
    if (end < 0) return { next: input.length, params: null };
    return { next: start + end + (rest[end] === "\u0007" ? 1 : 2), params: null };
  }
  const csi = ANSI_CSI.exec(rest);
  if (csi) {
    return {
      next: start + csi[0].length,
      params: csi[3] === "m" && !csi[2] ? csi[1] : null,
    };
  }
  const partial = ANSI_CSI_PARTIAL.exec(rest);
  if (partial && partial[0].length > 2) {
    return { next: start + partial[0].length, params: null };
  }
  const esc = ANSI_ESC.exec(rest);
  if (esc) return { next: start + esc[0].length, params: null };
  return { next: start + 1, params: null };
}

/**
 * One log line split into text runs with an allowlisted class list. SGR
 * colours, bold, dim, italic, and underline survive; OSC, CSI, and every other
 * escape sequence is dropped. The text is never parsed as markup. A caller can
 * pass the state of the previous line so an open colour spans line breaks.
 */
export function ansiRuns(text, state = ansiState()) {
  const input = String(text ?? "");
  const runs = [];
  let buffer = "";
  const flush = () => {
    if (!buffer) return;
    runs.push({ text: buffer, classes: ansiClasses(state) });
    buffer = "";
  };
  let index = 0;
  while (index < input.length) {
    const char = input[index];
    if (char === "\u001b") {
      flush();
      const escape = ansiEscape(input, index);
      if (escape.params !== null) applySgr(state, escape.params);
      index = escape.next;
      continue;
    }
    if ((char < " " && char !== "\t") || (char >= "\u007f" && char <= "\u009f")) {
      index += 1;
      continue;
    }
    buffer += char;
    index += 1;
  }
  flush();
  const merged = [];
  runs.forEach((run) => {
    const previous = merged[merged.length - 1];
    if (previous && previous.classes.join(" ") === run.classes.join(" ")) {
      previous.text += run.text;
    } else {
      merged.push({ text: run.text, classes: run.classes });
    }
  });
  return merged;
}

function ansiText(runs) {
  return runs.map((run) => run.text).join("");
}

// ansible-galaxy prints ERROR! and the playbook still runs from the cache.
function galaxyPhase(log, text) {
  const install = text.includes("ansible-galaxy collection install")
    || text.includes("ansible-galaxy role install");
  const cacheNote = text.includes("Galaxy install failed. The existing cache is used anyway.")
    || text.includes("Role install failed. The existing cache is used anyway.");
  if (install) {
    log.dataset.galaxy = "1";
    return "";
  }
  if (cacheNote) {
    log.dataset.galaxy = "";
    return "warn";
  }
  if (log.dataset.galaxy !== "1") return "";
  if (text.startsWith("KOREOGRAF ")) {
    log.dataset.galaxy = "";
    return "";
  }
  return "output";
}

function failureKind(text) {
  if (/fatal:\s*\[/.test(text) || text.includes("UNREACHABLE!") || text.includes("ERROR!")) return "fail";
  if (/\bfailed=[1-9]\d*\b/.test(text) || /\bunreachable=[1-9]\d*\b/.test(text)) return "fail";
  return "";
}

const RECAP_FIELDS = ["ok", "changed", "unreachable", "failed", "skipped", "rescued", "ignored"];

/** The order of the clickable counts in the totals bar. */
export const RECAP_STATUSES = [...RECAP_FIELDS];

const RECAP_RANK = {
  unreachable: 6,
  failed: 5,
  rescued: 4,
  ignored: 3,
  changed: 2,
  ok: 1,
  skipped: 0,
};

export function parseRecapLine(text) {
  const match = /^\s*(\S+)\s*:\s*ok=(\d+)\s+changed=(\d+)\s+unreachable=(\d+)\s+failed=(\d+)\s+skipped=(\d+)\s+rescued=(\d+)\s+ignored=(\d+)\s*$/.exec(String(text ?? ""));
  if (!match) return null;
  return {
    host: match[1],
    ok: Number(match[2]),
    changed: Number(match[3]),
    unreachable: Number(match[4]),
    failed: Number(match[5]),
    skipped: Number(match[6]),
    rescued: Number(match[7]),
    ignored: Number(match[8]),
  };
}

export function recapFailed(row) {
  return Boolean(row && (row.failed > 0 || row.unreachable > 0));
}

function recapNumber(row, key) {
  const value = Number(row && row[key]);
  return Number.isFinite(value) && value > 0 ? value : 0;
}

/**
 * One status per host for the totals bar. The first matching counter wins, so
 * the counts add up to the host total and each count filters the same bucket.
 */
export function recapHostStatus(row) {
  if (recapNumber(row, "unreachable")) return "unreachable";
  if (recapNumber(row, "failed")) return "failed";
  if (recapNumber(row, "rescued")) return "rescued";
  if (recapNumber(row, "ignored")) return "ignored";
  if (recapNumber(row, "changed")) return "changed";
  if (recapNumber(row, "ok")) return "ok";
  if (recapNumber(row, "skipped")) return "skipped";
  return "ok";
}

export function recapTotals(rows) {
  const counts = {};
  for (const status of RECAP_STATUSES) counts[status] = 0;
  for (const row of rows || []) counts[recapHostStatus(row)] += 1;
  return { hosts: (rows || []).length, counts };
}

/** The compact per-host recap that the callback writes from `v2_playbook_on_stats`. */
export function recapFromEvents(events) {
  const rows = new Map();
  const seen = new Set();
  let total = 0;
  let parts = 0;
  for (const item of events || []) {
    const event = eventPayload(item);
    if (!event || event.type !== "stats") continue;
    const declared = Number(event.total);
    if (Number.isFinite(declared) && declared > total) total = declared;
    const declaredParts = Number(event.parts);
    if (Number.isFinite(declaredParts) && declaredParts > parts) parts = declaredParts;
    const part = Number(event.part);
    if (Number.isFinite(part) && part > 0) seen.add(part);
    for (const host of Array.isArray(event.hosts) ? event.hosts : []) {
      if (!host || typeof host !== "object" || !host.host) continue;
      const row = { host: String(host.host) };
      for (const field of RECAP_FIELDS) row[field] = recapNumber(host, field);
      rows.set(row.host, row);
    }
  }
  return {
    rows: [...rows.values()],
    total: Math.max(total, rows.size),
    parts,
    received: seen.size,
    complete: parts === 0 || seen.size >= parts,
  };
}

/** Stats rows win; a raw `PLAY RECAP` line is the fallback for older jobs. */
export function mergeRecapRows(statsRows, rawRows) {
  const merged = new Map();
  for (const row of rawRows || []) {
    if (row && row.host) merged.set(row.host, row);
  }
  for (const row of statsRows || []) {
    if (row && row.host) merged.set(row.host, row);
  }
  return [...merged.values()];
}

export function recapMatches(row, status) {
  if (!status || status === "all") return true;
  if (status === "problems") return recapFailed(row);
  return recapHostStatus(row) === status;
}

export function sortRecapRows(rows, sort = "severity") {
  const field = String(sort || "severity");
  const descending = field.startsWith("-");
  const key = descending ? field.slice(1) : field;
  const listed = [...(rows || [])];
  const byHost = (left, right) => left.host.localeCompare(right.host);
  if (key === "host") {
    listed.sort(byHost);
    if (descending) listed.reverse();
    return listed;
  }
  if (key === "severity") {
    listed.sort((left, right) =>
      (RECAP_RANK[recapHostStatus(right)] - RECAP_RANK[recapHostStatus(left)]) || byHost(left, right));
    return listed;
  }
  if (RECAP_FIELDS.includes(key)) {
    listed.sort((left, right) => {
      const delta = recapNumber(right, key) - recapNumber(left, key);
      return (descending ? delta : -delta) || byHost(left, right);
    });
    return listed;
  }
  return listed.sort(byHost);
}

export function filterRecapRows(rows, options = {}) {
  const needle = String(options.search || "").trim().toLocaleLowerCase();
  const filtered = (rows || []).filter((row) =>
    recapMatches(row, options.status) && (!needle || row.host.toLocaleLowerCase().includes(needle)));
  return sortRecapRows(filtered, options.sort || "severity");
}

export function pageRecapRows(rows, page = 1, size = 50) {
  const total = (rows || []).length;
  const width = Math.max(1, Number(size) || 50);
  const pages = Math.max(1, Math.ceil(total / width));
  const current = Math.min(Math.max(1, Number(page) || 1), pages);
  const offset = (current - 1) * width;
  return {
    rows: (rows || []).slice(offset, offset + width),
    page: current,
    pages,
    total,
    from: total ? offset + 1 : 0,
    to: Math.min(offset + width, total),
  };
}

export function searchSegments(text, query) {
  const raw = String(text ?? "");
  const needle = String(query ?? "");
  if (!needle) return [{ text: raw, hit: false }];
  const haystack = raw.toLocaleLowerCase();
  const lower = needle.toLocaleLowerCase();
  const segments = [];
  let start = 0;
  let index = haystack.indexOf(lower);
  while (index !== -1) {
    if (index > start) segments.push({ text: raw.slice(start, index), hit: false });
    segments.push({ text: raw.slice(index, index + lower.length), hit: true });
    start = index + lower.length;
    index = haystack.indexOf(lower, start);
  }
  if (start < raw.length) segments.push({ text: raw.slice(start), hit: false });
  if (!segments.length) segments.push({ text: raw, hit: false });
  return segments;
}

function logState(log) {
  if (!log.koreografLog) {
    log.koreografLog = {
      tool: "",
      recap: new Map(),
      firstFailure: null,
      query: "",
      matches: [],
      current: -1,
      atBottom: true,
      onRecap: null,
      sgr: new Map(),
    };
  }
  return log.koreografLog;
}

export const OUTPUT_VIEW_KEY = "koreograf.job-output-view";

export function structuredEventsAvailable(job) {
  if (!job) return false;
  if (job.tool === "ansible") return Boolean(Array.isArray(job.ansible_events) && job.ansible_events.length);
  if (job.tool === "opentofu") return Boolean(Array.isArray(job.tofu_events) && job.tofu_events.length);
  return false;
}

export function outputViewPreference() {
  try {
    return window.localStorage.getItem(OUTPUT_VIEW_KEY) === "raw" ? "raw" : "structured";
  } catch {
    return "structured";
  }
}

export function rememberOutputView(view) {
  try {
    window.localStorage.setItem(OUTPUT_VIEW_KEY, view === "raw" ? "raw" : "structured");
  } catch { /* storage is optional */ }
}

const STATUS_RANK = { unreachable: 5, failed: 4, changed: 3, ok: 2, skipped: 1 };

function statusRank(status) {
  return STATUS_RANK[status] || 0;
}

export function worstStatus(hosts) {
  let worst = "";
  for (const host of hosts || []) {
    if (statusRank(host.status) > statusRank(worst)) worst = host.status;
  }
  return worst;
}

function hostKey(playIndex, taskIndex) {
  return `${playIndex}:${taskIndex}`;
}

function eventPayload(item) {
  if (!item || typeof item !== "object") return null;
  return item.event && typeof item.event === "object" ? item.event : item;
}

/** Turns the `ansible` event stream into plays, tasks, and host results. */
export function buildAnsibleTree(events) {
  const plays = [];
  const playsByIndex = new Map();
  const tasksByKey = new Map();
  let truncated = false;
  let limit = 0;
  let nextPlay = 0;
  let nextTask = 0;
  const playFor = (index, name) => {
    const key = Number.isFinite(index) ? index : `n${nextPlay + 1}`;
    let play = playsByIndex.get(key);
    if (!play) {
      play = { index: key, name: name || "", tasks: [], status: "" };
      playsByIndex.set(key, play);
      plays.push(play);
      if (Number.isFinite(index)) nextPlay = Math.max(nextPlay, index);
    } else if (!play.name && name) {
      play.name = name;
    }
    return play;
  };
  const taskFor = (event) => {
    const playIndex = Number.isFinite(Number(event.play_index)) ? Number(event.play_index) : `n${nextPlay}`;
    const index = Number.isFinite(Number(event.index)) ? Number(event.index) : `t${nextTask + 1}`;
    const key = hostKey(playIndex, index);
    let task = tasksByKey.get(key);
    if (!task) {
      task = {
        index,
        playIndex,
        name: event.task || "",
        role: event.role || "",
        handler: Boolean(event.handler),
        hosts: [],
        status: "",
        duration: null,
      };
      tasksByKey.set(key, task);
      playFor(playIndex, event.play || "").tasks.push(task);
      if (Number.isFinite(index)) nextTask = Math.max(nextTask, index);
    } else {
      if (!task.name && event.task) task.name = event.task;
      if (!task.role && event.role) task.role = event.role;
    }
    return task;
  };
  for (const item of events || []) {
    const event = eventPayload(item);
    if (!event || typeof event.type !== "string") continue;
    if (event.type === "limit") {
      truncated = true;
      limit = Number(event.events) || 0;
      continue;
    }
    if (event.type === "play_start") {
      playFor(Number(event.index), event.play || "");
      continue;
    }
    if (event.type === "task_start") {
      taskFor(event);
      continue;
    }
    if (event.type !== "host_result") continue;
    const task = taskFor(event);
    const duration = Number(event.duration);
    const censored = event.censored === true;
    const reported = STATUS_RANK[event.status] ? event.status : "ok";
    task.hosts.push({
      host: String(event.host || ""),
      status: reported === "ok" && event.changed === true ? "changed" : reported,
      duration: Number.isFinite(duration) && duration >= 0 ? duration : null,
      msg: !censored && typeof event.msg === "string" ? event.msg : "",
      stdout: !censored && typeof event.stdout === "string" ? event.stdout : "",
      stderr: !censored && typeof event.stderr === "string" ? event.stderr : "",
      diff: !censored && typeof event.diff === "string" ? event.diff : "",
      rc: !censored && Number.isInteger(event.rc) ? event.rc : null,
      changed: event.changed === true,
      ignored: event.ignored === true,
      censored,
    });
  }
  for (const play of plays) {
    for (const task of play.tasks) {
      task.status = worstStatus(task.hosts);
      const durations = task.hosts.map((host) => host.duration).filter((value) => value !== null);
      task.duration = durations.length ? Math.max(...durations) : null;
      task.counts = statusCounts(task.hosts);
    }
    play.status = worstStatus(play.tasks.flatMap((task) => task.hosts));
    play.counts = statusCounts(play.tasks.flatMap((task) => task.hosts));
    play.duration = play.tasks.reduce((sum, task) => sum + (task.duration || 0), 0);
  }
  return { plays, truncated, limit };
}

export function statusCounts(hosts) {
  const counts = {};
  for (const host of hosts || []) {
    counts[host.status] = (counts[host.status] || 0) + 1;
  }
  return counts;
}

export const TOFU_ACTIONS = ["create", "update", "delete", "replace", "read", "no-op"];
const TOFU_STATUSES = ["planned", "applying", "applied", "failed"];
const TOFU_SEVERITIES = ["error", "warning"];
const TOFU_ACTION_KEYS = {
  create: "tofu_action_create",
  update: "tofu_action_update",
  delete: "tofu_action_delete",
  replace: "tofu_action_replace",
  read: "tofu_action_read",
  "no-op": "tofu_action_noop",
};
const TOFU_STATUS_KEYS = {
  planned: "tofu_status_planned",
  applying: "tofu_status_applying",
  applied: "tofu_status_applied",
  failed: "tofu_status_failed",
};
const TOFU_SEVERITY_KEYS = {
  error: "tofu_severity_error",
  warning: "tofu_severity_warning",
};

function tofuCount(value) {
  const number = Number(value);
  return Number.isFinite(number) && number > 0 ? Math.floor(number) : 0;
}

/** Turns the `tofu` event stream into a summary, resources, diagnostics, and outputs. */
export function buildTofuModel(events) {
  const model = {
    summary: null,
    resources: [],
    byAddress: new Map(),
    diagnostics: [],
    outputs: [],
    outputByName: new Map(),
    truncated: false,
    limit: 0,
  };
  const resourceFor = (event) => {
    const address = typeof event.addr === "string" ? event.addr : "";
    if (!address) return null;
    let row = model.byAddress.get(address);
    if (!row) {
      row = { address, action: "create", status: "planned", elapsed: null };
      model.byAddress.set(address, row);
      model.resources.push(row);
    }
    if (TOFU_ACTIONS.includes(event.action)) row.action = event.action;
    return row;
  };
  for (const item of events || []) {
    const event = eventPayload(item);
    if (!event || typeof event.type !== "string") continue;
    if (event.type === "limit") {
      model.truncated = true;
      model.limit = tofuCount(event.events);
      continue;
    }
    if (event.type === "change_summary") {
      model.summary = {
        operation: event.operation === "apply" ? "apply" : "plan",
        add: tofuCount(event.add),
        change: tofuCount(event.change),
        remove: tofuCount(event.remove),
        import: tofuCount(event.import),
      };
      continue;
    }
    if (["planned_change", "apply_start", "apply_progress", "apply_complete", "apply_errored"].includes(event.type)) {
      const row = resourceFor(event);
      if (!row) continue;
      if (event.type === "planned_change") row.status = "planned";
      else if (event.type === "apply_start" || event.type === "apply_progress") row.status = "applying";
      else if (event.type === "apply_complete") row.status = "applied";
      else row.status = "failed";
      const elapsed = Number(event.elapsed_seconds);
      if (Number.isFinite(elapsed) && elapsed >= 0) row.elapsed = elapsed;
      continue;
    }
    if (event.type === "diagnostic") {
      model.diagnostics.push({
        severity: event.severity === "warning" ? "warning" : "error",
        summary: typeof event.summary === "string" ? event.summary : "",
        detail: typeof event.detail === "string" ? event.detail : "",
      });
      continue;
    }
    if (event.type === "outputs" && event.outputs && typeof event.outputs === "object") {
      for (const [name, output] of Object.entries(event.outputs)) {
        if (!output || typeof output !== "object") continue;
        const row = {
          name,
          sensitive: output.sensitive === true,
          value: typeof output.value === "string" ? output.value : "",
        };
        if (model.outputByName.has(name)) {
          Object.assign(model.outputByName.get(name), row);
        } else {
          model.outputByName.set(name, row);
          model.outputs.push(row);
        }
      }
    }
  }
  return model;
}

/** Counts for the summary bar, falling back to the planned resources. */
export function tofuCounts(model) {
  if (model && model.summary) return model.summary;
  const counts = { add: 0, change: 0, remove: 0, import: 0 };
  for (const row of (model && model.resources) || []) {
    if (row.action === "create") counts.add += 1;
    else if (row.action === "update" || row.action === "replace") counts.change += 1;
    else if (row.action === "delete") counts.remove += 1;
  }
  return counts;
}

function tofuActionKey(action) {
  return TOFU_ACTION_KEYS[action] || TOFU_ACTION_KEYS.create;
}

function tofuStatusKey(status) {
  return TOFU_STATUS_KEYS[status] || TOFU_STATUS_KEYS.planned;
}

function tofuSeverityKey(severity) {
  return TOFU_SEVERITY_KEYS[severity] || TOFU_SEVERITY_KEYS.error;
}

/**
 * The highest raw event id, which is the cursor for the live stream. The
 * structured rows replay from the same cursor and are ignored by id, so a raw
 * line that lands between the page's raw and structured reads is not skipped.
 */
export function lastRawEventId(job) {
  let last = 0;
  for (const item of (job && job.events) || []) {
    const id = Number(item && item.id);
    if (Number.isFinite(id) && id > last) last = id;
  }
  return last;
}

/** Appends one live event unless its id was already seeded from the job. */
export function pushStructuredEvent(state, item) {
  const id = Number(item && item.id);
  if (Number.isFinite(id) && id > 0) {
    if (state.seen.has(id)) return false;
    state.seen.add(id);
  }
  state.events.push(item);
  return true;
}

export function hostedNames(tree) {
  const names = new Set();
  for (const play of (tree && tree.plays) || []) {
    for (const task of play.tasks) {
      for (const host of task.hosts) {
        if (host.host) names.add(host.host);
      }
    }
  }
  return [...names].sort((left, right) => left.localeCompare(right));
}

/** Applies the host and status filters and the failed-first order. */
export function filterAnsibleTree(tree, options = {}) {
  const hostFilter = String(options.host || "").trim().toLocaleLowerCase();
  const statusFilter = options.status || "";
  const failedFirst = options.failedFirst === true;
  const hostSet = options.hosts instanceof Set
    ? options.hosts
    : Array.isArray(options.hosts) ? new Set(options.hosts) : null;
  const plays = [];
  for (const play of (tree && tree.plays) || []) {
    const tasks = [];
    for (const task of play.tasks) {
      const hosts = task.hosts.filter((host) =>
        (!hostFilter || host.host.toLocaleLowerCase().includes(hostFilter))
        && (!hostSet || hostSet.has(host.host))
        && (!statusFilter || host.status === statusFilter));
      if (!hosts.length) continue;
      const status = worstStatus(hosts);
      const durations = hosts.map((host) => host.duration).filter((value) => value !== null);
      tasks.push({
        ...task,
        hosts,
        status,
        counts: statusCounts(hosts),
        duration: durations.length ? Math.max(...durations) : null,
      });
    }
    if (!tasks.length) continue;
    if (failedFirst) {
      tasks.sort((left, right) => statusRank(right.status) - statusRank(left.status));
    }
    const hosts = tasks.flatMap((task) => task.hosts);
    plays.push({
      ...play,
      tasks,
      status: worstStatus(hosts),
      counts: statusCounts(hosts),
      duration: tasks.reduce((sum, task) => sum + (task.duration || 0), 0),
    });
  }
  if (failedFirst) {
    plays.sort((left, right) => statusRank(right.status) - statusRank(left.status));
  }
  return { ...tree, plays };
}

export function formatDuration(seconds) {
  if (seconds === null || seconds === undefined || !Number.isFinite(Number(seconds))) return "";
  const value = Math.max(0, Number(seconds));
  if (value < 0.1) return "<0.1s";
  if (value < 10) return `${value.toFixed(1)}s`;
  if (value < 60) return `${Math.round(value)}s`;
  const minutes = Math.floor(value / 60);
  const rest = Math.round(value % 60);
  return `${minutes}m ${rest}s`;
}

function paintLogLine(line, query) {
  const groups = [];
  line.replaceChildren();
  const runs = Array.isArray(line.koreografRuns)
    ? line.koreografRuns
    : [{ text: line.koreografRaw ?? "", classes: [] }];
  const raw = runs.map((run) => run.text).join("");
  let offset = 0;
  const segments = searchSegments(raw, query).map((segment) => {
    const start = offset;
    offset += segment.text.length;
    const entry = { ...segment, start, end: offset, marks: [] };
    if (entry.hit) groups.push(entry.marks);
    return entry;
  });
  let runStart = 0;
  runs.forEach((run) => {
    const runEnd = runStart + run.text.length;
    segments.forEach((segment) => {
      const from = Math.max(runStart, segment.start);
      const to = Math.min(runEnd, segment.end);
      if (to <= from) return;
      const text = run.text.slice(from - runStart, to - runStart);
      if (segment.hit) {
        const mark = document.createElement("mark");
        mark.className = run.classes && run.classes.length
          ? `log-hit ${run.classes.join(" ")}`
          : "log-hit";
        mark.textContent = text;
        line.appendChild(mark);
        segment.marks.push(mark);
      } else if (run.classes && run.classes.length) {
        const span = document.createElement("span");
        span.className = run.classes.join(" ");
        span.textContent = text;
        line.appendChild(span);
      } else {
        line.appendChild(document.createTextNode(text));
      }
    });
    runStart = runEnd;
  });
  return groups;
}

function updateRecap(state, text) {
  if (state.tool !== "ansible") return;
  const row = parseRecapLine(text);
  if (!row) return;
  state.recap.set(row.host, row);
  if (typeof state.onRecap === "function") state.onRecap();
}

function updateSearchCount(state) {
  const count = document.querySelector("#log-search-count");
  if (!count) return;
  if (!state.query) {
    count.textContent = "";
    return;
  }
  count.textContent = t("log_search_count", {
    current: state.matches.length ? state.current + 1 : 0,
    total: state.matches.length,
  });
}

function scrollLogLine(log, line) {
  if (!log || !line) return;
  const offset = (Number(line.offsetTop) || 0) - (Number(log.offsetTop) || 0);
  const centered = offset - (Number(log.clientHeight) || 0) / 2 + (Number(line.offsetHeight) || 0) / 2;
  const max = Math.max(0, (Number(log.scrollHeight) || 0) - (Number(log.clientHeight) || 0));
  log.scrollTop = Math.min(Math.max(0, centered), max);
}

function jumpToSearchMatch(log, state, index) {
  if (!state.matches.length) {
    state.current = -1;
    updateSearchCount(state);
    return;
  }
  const total = state.matches.length;
  state.current = ((index % total) + total) % total;
  state.matches.forEach((group) => group.forEach((mark) => mark.classList.remove("current")));
  const group = state.matches[state.current];
  group.forEach((mark) => mark.classList.add("current"));
  scrollLogLine(log, group[0].parentElement);
  updateSearchCount(state);
}

function paintLogSearch(log, state, jump) {
  state.matches = [];
  state.current = -1;
  log.querySelectorAll("div").forEach((line) => {
    if (line.koreografRaw === undefined) return;
    state.matches.push(...paintLogLine(line, state.query));
  });
  if (jump && state.matches.length) jumpToSearchMatch(log, state, 0);
  else updateSearchCount(state);
}

function phoneWidth() {
  return typeof window.matchMedia === "function" && window.matchMedia("(max-width: 600px)").matches;
}

function bindJobLog(log, job) {
  const state = logState(log);
  state.tool = job.tool || "";
  state.recap.clear();
  state.firstFailure = null;
  state.query = "";
  state.matches = [];
  state.current = -1;
  state.atBottom = true;
  state.onRecap = null;
  state.sgr.clear();
  const search = document.querySelector("#log-search");
  const count = document.querySelector("#log-search-count");
  const first = document.querySelector("#log-first-failure");
  const end = document.querySelector("#log-jump-end");
  const wrap = document.querySelector("#log-wrap");
  if (first) first.hidden = true;
  if (end) end.hidden = true;
  if (count) count.textContent = "";
  if (!search) return;
  search.value = "";
  search.addEventListener("input", () => {
    state.query = search.value.trim();
    paintLogSearch(log, state, true);
  });
  search.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || !state.matches.length) return;
    event.preventDefault();
    jumpToSearchMatch(log, state, state.current + (event.shiftKey ? -1 : 1));
  });
  log.addEventListener("scroll", () => {
    const bottom = log.scrollHeight - log.scrollTop - log.clientHeight < 24;
    state.atBottom = bottom;
    if (end) end.hidden = bottom;
  });
  if (wrap) {
    wrap.checked = phoneWidth();
    log.classList.toggle("wrap", wrap.checked);
    wrap.addEventListener("change", () => log.classList.toggle("wrap", wrap.checked));
  }
  if (first) {
    first.addEventListener("click", () => {
      if (state.firstFailure) scrollLogLine(log, state.firstFailure);
    });
  }
  if (end) {
    end.addEventListener("click", () => {
      state.atBottom = true;
      log.scrollTop = log.scrollHeight;
      end.hidden = true;
    });
  }
}

function tagFilterNote(row) {
  return row.stream === "system" && plainLogLine(row.line).includes("Tag filter matched no tasks.");
}

function showTagFilterBanner() {
  if (document.querySelector("#tag-filter-banner")) return;
  const anchor = document.querySelector(".tab-bar");
  if (!anchor) return;
  anchor.insertAdjacentHTML(
    "beforebegin",
    `<p class="banner warn" id="tag-filter-banner">${h("tag_filter_matched_no_tasks")}</p>`,
  );
}

export function classifyLogLine(log, row) {
  const text = plainLogLine(row.line);
  if (tagFilterNote(row)) return "warn";
  const galaxy = galaxyPhase(log, text);
  const failure = failureKind(text);
  if (galaxy === "warn") return "warn";
  if (galaxy === "output") {
    if (failure && !text.includes("ERROR!")) return "fail";
    if (row.stream === "stderr" || text.includes("ERROR!")) return "warn";
    if (row.stream === "system") return "system";
    return "";
  }
  if (failure) return "fail";
  if (row.stream === "system") return "system";
  if (row.stream === "stderr") return "stderr";
  return "";
}

/** The exit code of the runner's single `Exit code: N` system line. */
export function exitCodeFromLine(text) {
  const match = /^Exit code: (-?\d+)$/.exec(plainLogLine(text).trim());
  return match ? Number(match[1]) : null;
}

/** The last recorded exit code, or null while the process has not exited. */
export function exitCodeFromEvents(events) {
  let code = null;
  for (const row of Array.isArray(events) ? events : []) {
    if (!row || row.stream !== "system") continue;
    const found = exitCodeFromLine(row.line);
    if (found !== null) code = found;
  }
  return code;
}

function showJobExitCode(code) {
  if (code === null || code === undefined) return;
  const box = document.querySelector("#job-exit-code");
  if (!box) return;
  box.textContent = t("exit_code", { code });
  box.hidden = false;
}

function markRecap(line) {
  const text = plainLogLine(line.textContent);
  if (!(/\bfailed=[1-9]\d*\b/.test(text) || /\bunreachable=[1-9]\d*\b/.test(text))) return;
  let prev = line.previousElementSibling;
  for (let index = 0; index < 8 && prev; index += 1) {
    if (prev.classList.contains("log-summary")) break;
    if (prev.textContent.includes("PLAY RECAP")) {
      prev.classList.add("fail");
      break;
    }
    prev = prev.previousElementSibling;
  }
}

function pinFailure(text, kind) {
  const summary = document.querySelector("#log-summary");
  if (kind !== "fail" || !summary || !summary.hidden) return;
  const shown = text.length > 240 ? `${text.slice(0, 239)}...` : text;
  summary.textContent = `${t("first_failure")}: ${shown}`;
  summary.hidden = false;
}

export function appendLog(row) {
  const log = document.querySelector("#log");
  if (!log) return;
  const state = logState(log);
  const line = document.createElement("div");
  if (row.stream === "gap") {
    line.className = "system";
    line.textContent = t("log_gap", { count: Number(row.line) || 0 });
    line.koreografRaw = line.textContent;
    log.appendChild(line);
    if (state.query) {
      state.matches.push(...paintLogLine(line, state.query));
      updateSearchCount(state);
    }
    if (state.atBottom) log.scrollTop = log.scrollHeight;
    return;
  }
  const kind = classifyLogLine(log, row);
  if (tagFilterNote(row)) showTagFilterBanner();
  if (kind) line.className = kind;
  const sgr = state.sgr.get(row.stream) || ansiState();
  line.koreografRuns = ansiRuns(row.line, sgr);
  state.sgr.set(row.stream, sgr);
  line.koreografRaw = ansiText(line.koreografRuns);
  paintLogLine(line, "");
  log.appendChild(line);
  if (kind === "fail") {
    markRecap(line);
    if (!state.firstFailure) {
      state.firstFailure = line;
      const first = document.querySelector("#log-first-failure");
      if (first) first.hidden = false;
    }
  }
  pinFailure(line.koreografRaw, kind);
  updateRecap(state, line.koreografRaw);
  if (row.stream === "system") showJobExitCode(exitCodeFromLine(line.koreografRaw));
  if (state.query) {
    const groups = paintLogLine(line, state.query);
    if (groups.length) {
      state.matches.push(...groups);
      if (state.current < 0) {
        state.current = 0;
        state.matches[0].forEach((mark) => mark.classList.add("current"));
      }
      updateSearchCount(state);
    }
  }
  if (state.atBottom) log.scrollTop = log.scrollHeight;
}



function credentialSummary(params) {
  if (Array.isArray(params.credentials)) {
    return params.credentials.map((item) => {
      const name = item && item.name;
      if (!name) return "";
      const kind = item.kind ? credentialKindLabel(item.kind) : "";
      return [name, kind].filter(Boolean).join(" · ");
    }).filter(Boolean).join(", ");
  }
  const kind = params.credential_kind ? credentialKindLabel(params.credential_kind) : "";
  return [params.credential_name, kind].filter(Boolean).join(" · ");
}

export function jobParameters(job) {
  const params = job.parameters && job.parameters.tool ? job.parameters : null;
  const hosts = Array.isArray(job.failed_hosts) ? job.failed_hosts : [];
  if (!params) {
    const extra = job.extra_vars && Object.keys(job.extra_vars).length
      ? `<pre class="vars">${esc(JSON.stringify(job.extra_vars, null, 2))}</pre>`
      : "";
    return `<p class="muted">${h("no_parameters_were_stored_for_this_job")}</p>${extra}`;
  }
  const spec = params.spec || {};
  const templateId = params.template_id || job.template_id;
  const templateName = params.template_name || job.name;
  const template = templateId
    ? `<a href="#/templates?edit=${esc(templateId)}">${esc(templateName)}</a>`
    : esc(templateName);
  const credential = credentialSummary(params);
  const environment = [params.environment_name, params.environment_image].filter(Boolean).join(" · ");
  const kind = spec.job_type === "check" ? t("check") : spec.job_type === "run" ? t("run") : "";
  const diff = spec.diff === true ? t("yes") : spec.diff === false ? t("no") : "";
  const commit = params.project_commit
    ? `<span title="${esc(params.project_commit_full || params.project_commit)}">${esc(params.project_commit)}</span>`
    : "";
  const hostLine = hosts.length
    ? `<dt>${h("failed_hosts")}</dt><dd>${hosts.map((host) => esc(host)).join(", ")}</dd>`
    : "";
  return `<dl class="kv">
    <dt>${h("template")}</dt><dd>${template}</dd>
    ${paramRow("tool", params.tool)}
    ${paramRow("entry", params.entrypoint)}
    ${paramRow("project", params.project_name)}
    ${paramRowHtml("project_commit", commit)}
    ${typeof params.update_on_launch === "boolean" && params.project_name ? paramRow("project_update", t(params.update_on_launch ? "on_launch" : "use_the_last_sync")) : ""}
    ${paramRow("inventory", params.inventory_name)}
    ${paramRow("credentials", credential)}
    ${paramRow("execution_environment", environment)}
    ${paramRow("type", kind)}
    ${paramRowHtml("tags", chipReadout(spec.tags))}
    ${paramRowHtml("skip_tags", chipReadout(spec.skip_tags))}
    ${paramRow("limit", spec.limit)}
    ${paramRow("diff", diff)}
    ${paramRow("verbosity", spec.verbosity === undefined || spec.verbosity === null || spec.verbosity === "" ? "" : t(verbosityLabel(spec.verbosity)))}
    ${paramRow("forks", spec.forks)}
    ${paramRow("action", spec.action)}
    ${hostLine}
  </dl>
  ${spec.script ? `<h3>${h("script")}</h3><pre class="vars">${esc(spec.script)}</pre>` : ""}
  <h3>${h("extra_variables")}</h3>
  <pre class="vars">${esc(JSON.stringify(params.extra_vars || {}, null, 2))}</pre>`;
}

function relaunchPrompt(job, onlyFailed) {
  const spec = (job.parameters && job.parameters.spec) || {};
  const limit = spec.limit ? String(spec.limit) : t("all_hosts");
  return t(onlyFailed ? "relaunch_this_job_on_failed_hosts" : "relaunch_this_job", { name: job.name, limit });
}

function jobListActions(job) {
  if (!can("operator") || (job.status !== "failed" && job.status !== "canceled")) return "";
  const hosts = Array.isArray(job.failed_hosts) ? job.failed_hosts : [];
  const failedHosts = job.tool === "ansible" && hosts.length
    ? `<button class="btn" type="button" data-relaunch="${esc(job.id)}" data-failed="1">${h("failed_hosts_only")}</button>`
    : "";
  return `<div class="row"><button class="btn" type="button" data-relaunch="${esc(job.id)}" data-failed="0">${h("relaunch")}</button>${failedHosts}</div>`;
}

function jobActions(job) {
  const running = job.status === "pending" || job.status === "running";
  const hosts = Array.isArray(job.failed_hosts) ? job.failed_hosts : [];
  if (!can("operator")) return "";
  if (running) return `<button class="btn danger" type="button" data-action="cancel">${h("cancel")}</button>`;
  if (job.status !== "failed" && job.status !== "canceled") return "";
  const failedHosts = job.tool === "ansible" && hosts.length
    ? `<button class="btn" type="button" data-action="relaunch-failed">${h("failed_hosts_only")}</button>`
    : "";
  return `<button class="btn primary" type="button" data-action="relaunch">${h("relaunch")}</button>${failedHosts}`;
}

function stNode(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function stBadges(counts, parent) {
  for (const status of ["failed", "unreachable", "changed", "ok", "skipped"]) {
    const count = counts && counts[status];
    if (!count) continue;
    parent.appendChild(stNode("span", `st-badge ${status}`, `${count} ${t(status)}`));
  }
}

function stField(body, label, text) {
  if (!text) return;
  const field = stNode("div", "st-field");
  field.appendChild(stNode("h4", "", label));
  field.appendChild(stNode("pre", "st-pre", text));
  body.appendChild(field);
}

function stHost(host, key, state) {
  const item = stNode("details", `st-host ${host.status}`);
  item.dataset.host = host.host;
  item.dataset.status = host.status;
  item.open = state.open.has(key) ? state.open.get(key) : false;
  item.addEventListener("toggle", () => state.open.set(key, item.open));
  const summary = stNode("summary", "st-host-head");
  summary.appendChild(stNode("span", `st-badge ${host.status}`, t(host.status)));
  summary.appendChild(stNode("span", "st-host-name", host.host));
  if (host.duration !== null) {
    summary.appendChild(stNode("span", "st-duration", formatDuration(host.duration)));
  }
  if (host.ignored) summary.appendChild(stNode("span", "st-note", t("ignored")));
  item.appendChild(summary);
  const body = stNode("div", "st-body");
  if (host.censored) {
    body.appendChild(stNode("p", "small muted", t("output_hidden_by_no_log")));
  } else {
    if (host.rc !== null) body.appendChild(stNode("p", "small muted", `${t("return_code")}: ${host.rc}`));
    stField(body, t("message"), host.msg);
    stField(body, t("stdout"), host.stdout);
    stField(body, t("stderr"), host.stderr);
    stField(body, t("diff"), host.diff);
    if (!body.childElementCount) body.appendChild(stNode("p", "small muted", t("no_result_detail")));
  }
  item.appendChild(body);
  return item;
}

const TASK_HOST_PAGE = 50;
const DETAIL_CACHE = 8;

function bindStructuredOutput(job, startedAt) {
  const switchBox = document.querySelector("#output-view");
  const box = document.querySelector("#structured-output");
  const treeBox = document.querySelector("#st-tree");
  const log = document.querySelector("#log");
  const tools = document.querySelector("#output-tools");
  const hostInput = document.querySelector("#st-host");
  const hostHints = document.querySelector("#st-host-hints");
  const statusSelect = document.querySelector("#st-status");
  const failedFirst = document.querySelector("#st-failed-first");
  const recapBox = document.querySelector("#log-result");
  const recapTotal = document.querySelector("#st-recap-total");
  const recapCounts = document.querySelector("#st-counts");
  const recapNote = document.querySelector("#st-recap-note");
  const hostPanel = document.querySelector("#st-host-panel");
  const hostSearch = document.querySelector("#host-search");
  const hostStatus = document.querySelector("#host-status");
  const hostSort = document.querySelector("#host-sort");
  const hostSize = document.querySelector("#host-size");
  const hostTable = document.querySelector("#host-table");
  const hostTotal = document.querySelector("#host-total");
  const hostPager = document.querySelector("#host-pager");
  const hostCount = document.querySelector("#st-host-count");
  if (!switchBox || !box || !treeBox || !hostInput || !statusSelect || !failedFirst
    || !recapBox || !recapCounts || !hostTable || !hostStatus || !hostSort || !hostSize
    || !hostTotal || !hostPager) return null;
  const state = {
    events: [...(job.ansible_events || [])],
    seen: new Set((job.ansible_events || []).map((item) => Number(item && item.id)).filter((id) => Number.isFinite(id) && id > 0)),
    tree: { plays: [], truncated: false, limit: 0 },
    stats: { rows: new Map(), total: 0, parts: 0, received: new Set(), complete: true },
    open: new Map(),
    taskHosts: new Map(),
    details: new Map(),
    explicit: false,
    view: "raw",
    frame: 0,
    tab: true,
    rendered: false,
    startedAt: Number(startedAt) || 0,
    hostQuery: "",
    hintsOpen: false,
    listStatus: "problems",
    listSearch: "",
    listSort: "severity",
    listPage: 1,
    listSize: 50,
    countFilter: "",
  };
  const absorbStats = (item) => {
    const event = eventPayload(item);
    if (!event || event.type !== "stats") return;
    const declaredTotal = Number(event.total);
    if (Number.isFinite(declaredTotal) && declaredTotal > state.stats.total) {
      state.stats.total = declaredTotal;
    }
    const declaredParts = Number(event.parts);
    if (Number.isFinite(declaredParts) && declaredParts > state.stats.parts) {
      state.stats.parts = declaredParts;
    }
    const part = Number(event.part);
    if (Number.isFinite(part) && part > 0) state.stats.received.add(part);
    for (const host of Array.isArray(event.hosts) ? event.hosts : []) {
      if (!host || typeof host !== "object" || !host.host) continue;
      const row = { host: String(host.host) };
      for (const field of RECAP_FIELDS) row[field] = recapNumber(host, field);
      state.stats.rows.set(row.host, row);
    }
    state.stats.complete = state.stats.parts === 0 || state.stats.received.size >= state.stats.parts;
  };
  for (const item of state.events) absorbStats(item);
  state.tree = buildAnsibleTree(state.events);
  state.view = state.tree.plays.length ? outputViewPreference() : "raw";
  if (state.view === "structured") state.explicit = true;
  if (log) logState(log).onRecap = () => schedule();

  const recapRows = () => {
    const rawRows = log ? [...logState(log).recap.values()] : [];
    return mergeRecapRows([...state.stats.rows.values()], rawRows);
  };
  const totalHosts = (rows) => Math.max(state.stats.total, rows.length);

  const applyView = () => {
    const raw = state.view !== "structured";
    const available = state.tree.plays.length > 0 || state.stats.total > 0 || recapRows().length > 0;
    switchBox.hidden = !available || !state.tab;
    switchBox.querySelectorAll("[data-output-view]").forEach((button) => {
      const active = button.dataset.outputView === state.view;
      button.classList.toggle("primary", active);
      button.setAttribute("aria-pressed", active ? "true" : "false");
    });
    box.hidden = raw || !state.tab;
    if (tools) tools.hidden = !raw || !state.tab;
    if (log) log.hidden = !raw || !state.tab;
  };

  const isOpen = (key, fallback) => (state.open.has(key) ? state.open.get(key) : fallback);
  const remember = (key, details) => {
    state.open.set(key, details.open);
  };

  const allHostNames = (rows) => {
    const names = new Set(hostedNames(state.tree));
    for (const row of rows) names.add(row.host);
    return [...names].sort((left, right) => left.localeCompare(right));
  };

  const detailEvents = () => {
    const items = [];
    for (const entry of state.details.values()) items.push(...entry.events);
    return items;
  };

  const trimDetails = () => {
    if (state.details.size <= DETAIL_CACHE) return;
    for (const [host, entry] of state.details) {
      if (state.details.size <= DETAIL_CACHE) break;
      if (!entry.loading && host !== state.hostQuery) state.details.delete(host);
    }
  };

  const ensureHostDetail = (name) => {
    const host = String(name || "");
    if (!host || !job || job.tool !== "ansible") return;
    if (hostedNames(state.tree).includes(host)) return;
    const previous = state.details.get(host);
    if (previous && !previous.failed) return;
    if (previous) state.details.delete(host);
    const entry = { host, loading: true, events: [], truncated: false, reason: "", failed: false };
    state.details.set(host, entry);
    trimDetails();
    api("GET", `/api/v1/jobs/${encodeURIComponent(job.id)}/ansible?host=${encodeURIComponent(host)}`)
      .then((data) => {
        entry.events = Array.isArray(data && data.events) ? data.events : [];
        entry.truncated = Boolean(data && data.truncated);
        entry.reason = typeof (data && data.reason) === "string" ? data.reason : "";
      })
      .catch(() => {
        entry.failed = true;
      })
      .finally(() => {
        entry.loading = false;
        refresh();
      });
  };

  const selectHost = (name) => {
    const host = String(name || "");
    state.hostQuery = host;
    state.hintsOpen = false;
    hostInput.value = host;
    if (hostHints) {
      hostHints.hidden = true;
      hostHints.replaceChildren();
    }
    ensureHostDetail(host);
    draw();
  };

  const renderHints = (rows) => {
    if (!hostHints) return;
    const query = state.hostQuery.trim().toLocaleLowerCase();
    if (!query || !state.hintsOpen) {
      hostHints.hidden = true;
      hostHints.replaceChildren();
      return;
    }
    const matches = allHostNames(rows)
      .filter((name) => name.toLocaleLowerCase().includes(query))
      .slice(0, 8);
    hostHints.replaceChildren(...matches.map((name) => {
      const button = stNode("button", "btn compact st-hint", name);
      button.type = "button";
      button.setAttribute("role", "option");
      button.addEventListener("mousedown", (event) => {
        event.preventDefault();
        selectHost(name);
        hostInput.focus();
      });
      return button;
    }));
    hostHints.hidden = matches.length === 0;
  };

  const recapCount = (row) => {
    const counters = RECAP_FIELDS
      .filter((field) => recapNumber(row, field))
      .map((field) => `<span class="st-count-pair"><span>${esc(t(field))}</span><strong>${recapNumber(row, field)}</strong></span>`)
      .join("");
    const status = recapHostStatus(row);
    return `<tr data-host="${esc(row.host)}" data-status="${status}"${recapFailed(row) ? ' class="fail"' : ""}>
      <td data-label="${esc(t("host"))}"><button class="btn compact st-host-link" type="button" data-st-select-host="${esc(row.host)}">${esc(row.host)}</button></td>
      <td data-label="${esc(t("status"))}"><span class="st-badge ${status}">${esc(t(status))}</span></td>
      <td class="small" data-label="${esc(t("counters"))}">${counters}</td>
    </tr>`;
  };

  const drawRecap = (rows, total) => {
    recapBox.hidden = total === 0;
    if (recapTotal) {
      recapTotal.textContent = total ? t("hosts_total", { count: total }) : "";
    }
    if (!total) {
      recapCounts.replaceChildren();
      if (recapNote) recapNote.hidden = true;
      return;
    }
    const totals = recapTotals(rows);
    recapCounts.replaceChildren(...RECAP_STATUSES.map((status) => {
      const active = state.countFilter === status;
      const button = stNode("button", `btn compact st-count ${status}${active ? " primary" : ""}`);
      button.type = "button";
      button.dataset.stCount = status;
      button.setAttribute("aria-pressed", active ? "true" : "false");
      button.disabled = !totals.counts[status];
      button.appendChild(stNode("span", "st-count-label", t(status)));
      button.appendChild(stNode("span", "st-count-value", String(totals.counts[status] || 0)));
      return button;
    }));
    if (recapNote) recapNote.hidden = state.stats.parts === 0 || state.stats.complete;
  };

  const drawHostList = (rows, total) => {
    const filtered = filterRecapRows(rows, {
      status: state.listStatus,
      search: state.listSearch,
      sort: state.listSort,
    });
    const page = pageRecapRows(filtered, state.listPage, state.listSize);
    state.listPage = page.page;
    if (hostPanel) hostPanel.hidden = total === 0;
    if (hostCount) hostCount.textContent = t("hosts_shown_of_total", { shown: page.rows.length, total });
    hostTotal.textContent = total
      ? t("hosts_shown_of_total", { shown: page.rows.length, total })
      : t("no_hosts_in_recap");
    if (page.rows.length) {
      hostTable.innerHTML = `<div class="table-wrap st-host-table"><table class="table"><thead><tr><th>${h("host")}</th><th>${h("status")}</th><th>${h("counters")}</th></tr></thead><tbody>${page.rows.map(recapCount).join("")}</tbody></table></div>`;
    } else if (rows.length && state.listStatus === "problems" && !rows.some(recapFailed)) {
      hostTable.innerHTML = `<p class="small muted">${h("no_problem_hosts")}</p><button class="btn" id="show-all-hosts" type="button">${h("all_hosts")}</button>`;
    } else if (rows.length) {
      hostTable.innerHTML = `<p class="small muted">${h("no_hosts_match")}</p><button class="btn" id="clear-host-filter" type="button">${h("clear_filter")}</button>`;
    } else {
      hostTable.innerHTML = `<p class="small muted">${h("no_hosts_in_recap")}</p>`;
    }
    hostPager.innerHTML = page.pages > 1
      ? `<button class="btn" type="button" data-host-page="${page.page - 1}"${page.page <= 1 ? " disabled" : ""}>${h("previous_page")}</button>
        <span class="small muted">${h("page_of_pages", { page: page.page, pages: page.pages })}</span>
        <button class="btn" type="button" data-host-page="${page.page + 1}"${page.page >= page.pages ? " disabled" : ""}>${h("next_page")}</button>`
      : "";
  };

  const renderTaskHosts = (task, taskKey, container) => {
    const limit = state.taskHosts.get(taskKey) || TASK_HOST_PAGE;
    const shown = task.hosts.slice(0, limit);
    container.replaceChildren(...shown.map((host) =>
      stHost(host, `${taskKey}:h:${host.host}`, state)));
    if (task.hosts.length > shown.length) {
      const more = stNode("button", "btn compact st-more", t("show_more_hosts", { count: Math.min(TASK_HOST_PAGE, task.hosts.length - shown.length) }));
      more.type = "button";
      more.addEventListener("click", () => {
        state.taskHosts.set(taskKey, limit + TASK_HOST_PAGE);
        draw();
      });
      container.appendChild(more);
    }
  };

  const drawTree = (rows) => {
    const countHosts = state.countFilter
      ? new Set(rows.filter((row) => recapMatches(row, state.countFilter)).map((row) => row.host))
      : null;
    const filtered = filterAnsibleTree(state.tree, {
      host: state.hostQuery,
      status: statusSelect.value,
      failedFirst: failedFirst.checked,
      hosts: countHosts,
    });
    const detailHost = state.hostQuery.trim();
    const detail = detailHost ? state.details.get(detailHost) : null;
    const detailNotice = Boolean(detail && (detail.loading || detail.failed || detail.reason));
    const notes = [];
    if (detail && detail.loading) {
      const note = stNode("p", "small muted st-detail-note", t("host_detail_loading", { host: detail.host }));
      note.setAttribute("role", "status");
      notes.push(note);
    } else if (detail && detail.failed) {
      notes.push(stNode("p", "banner bad st-detail-note", t("host_detail_load_failed", { host: detail.host })));
    } else if (detail && detail.reason) {
      const note = stNode("p", "small muted st-detail-note", t(detail.reason));
      note.setAttribute("role", "status");
      notes.push(note);
    } else if (detail && detail.truncated) {
      notes.push(stNode("p", "small muted st-detail-note", t("host_detail_truncated", {
        host: detail.host,
        count: detail.events.length,
      })));
    }
    const nodes = [];
    for (const play of filtered.plays) {
      const playKey = `p:${play.index}`;
      const details = stNode("details", `st-play ${play.status || ""}`);
      details.open = isOpen(playKey, true);
      details.addEventListener("toggle", () => remember(playKey, details));
      const summary = stNode("summary", "st-play-head");
      summary.appendChild(stNode("span", "st-play-name", play.name || t("play")));
      stBadges(play.counts, summary);
      if (play.duration) summary.appendChild(stNode("span", "st-duration", formatDuration(play.duration)));
      details.appendChild(summary);
      const tasks = stNode("div", "st-tasks");
      for (const task of play.tasks) {
        const taskKey = `p:${play.index}:t:${task.index}`;
        const taskDetails = stNode("details", `st-task ${task.status || ""}`);
        const reveal = Boolean(state.hostQuery) || task.status === "failed" || task.status === "unreachable";
        taskDetails.open = isOpen(taskKey, reveal);
        taskDetails.dataset.task = task.name;
        taskDetails.dataset.status = task.status || "";
        const taskHead = stNode("summary", "st-task-head");
        const name = task.role ? `${task.role} · ${task.name}` : task.name;
        taskHead.appendChild(stNode("span", "st-task-name", name || t("task")));
        if (task.handler) taskHead.appendChild(stNode("span", "st-note", t("handler")));
        stBadges(task.counts, taskHead);
        if (task.duration !== null) {
          taskHead.appendChild(stNode("span", "st-duration", formatDuration(task.duration)));
        }
        taskDetails.appendChild(taskHead);
        const hosts = stNode("div", "st-hosts");
        taskDetails.appendChild(hosts);
        if (taskDetails.open) renderTaskHosts(task, taskKey, hosts);
        taskDetails.addEventListener("toggle", () => {
          remember(taskKey, taskDetails);
          if (taskDetails.open) renderTaskHosts(task, taskKey, hosts);
          else hosts.replaceChildren();
        });
        tasks.appendChild(taskDetails);
      }
      details.appendChild(tasks);
      nodes.push(details);
    }
    if (!filtered.plays.length && !detailNotice) {
      nodes.push(stNode("p", "small muted st-empty", t("no_tasks_match")));
    }
    if (filtered.truncated) {
      nodes.push(
        stNode("p", "small muted st-limit", t("structured_output_limited", { count: filtered.limit || 2000 })),
      );
    }
    treeBox.replaceChildren(...notes, ...nodes);
  };

  const draw = () => {
    state.frame = 0;
    const rows = recapRows();
    const total = totalHosts(rows);
    drawRecap(rows, total);
    drawHostList(rows, total);
    drawTree(rows);
    renderHints(rows);
    box.dataset.hostTotal = String(total);
    box.dataset.recapComplete = String(state.stats.parts === 0 || state.stats.complete);
    if (!state.rendered && state.view === "structured") {
      state.rendered = true;
      if (state.startedAt) box.dataset.renderMs = String(Math.max(0, Math.round(performance.now() - state.startedAt)));
    }
  };

  const schedule = () => {
    if (state.frame) return;
    const run = () => draw();
    state.frame = window.requestAnimationFrame ? window.requestAnimationFrame(run) : window.setTimeout(run, 16);
  };

  switchBox.querySelectorAll("[data-output-view]").forEach((button) => {
    button.addEventListener("click", () => {
      state.view = button.dataset.outputView === "raw" ? "raw" : "structured";
      state.explicit = true;
      rememberOutputView(state.view);
      applyView();
    });
  });
  recapCounts.addEventListener("click", (event) => {
    const button = event.target.closest("[data-st-count]");
    if (!button || button.disabled) return;
    const status = button.dataset.stCount;
    state.countFilter = state.countFilter === status ? "" : status;
    state.listStatus = state.countFilter || "all";
    state.listPage = 1;
    hostStatus.value = state.listStatus;
    draw();
  });
  hostSearch.addEventListener("input", () => {
    if (hostSearch.hostTimer) window.clearTimeout(hostSearch.hostTimer);
    hostSearch.hostTimer = window.setTimeout(() => {
      state.listSearch = hostSearch.value.trim();
      state.listPage = 1;
      draw();
    }, 150);
  });
  hostStatus.addEventListener("change", () => {
    state.listStatus = hostStatus.value || "problems";
    state.countFilter = "";
    state.listPage = 1;
    draw();
  });
  hostSort.addEventListener("change", () => {
    state.listSort = hostSort.value || "severity";
    state.listPage = 1;
    draw();
  });
  hostSize.addEventListener("change", () => {
    state.listSize = Number(hostSize.value) || 50;
    state.listPage = 1;
    draw();
  });
  hostTable.addEventListener("click", (event) => {
    if (event.target.closest("#show-all-hosts")) {
      state.listStatus = "all";
      state.listPage = 1;
      hostStatus.value = "all";
      draw();
      hostStatus.focus();
      return;
    }
    if (event.target.closest("#clear-host-filter")) {
      state.listSearch = "";
      state.listStatus = "problems";
      state.listPage = 1;
      hostSearch.value = "";
      hostStatus.value = "problems";
      draw();
      hostSearch.focus();
      return;
    }
    const pick = event.target.closest("[data-st-select-host]");
    if (pick) {
      selectHost(pick.dataset.stSelectHost);
      treeBox.scrollIntoView({ block: "start" });
    }
  });
  hostPager.addEventListener("click", (event) => {
    const button = event.target.closest("[data-host-page]");
    if (!button || button.disabled) return;
    state.listPage = Number(button.dataset.hostPage);
    draw();
    hostPager.focus();
  });
  hostInput.addEventListener("input", () => {
    state.hintsOpen = true;
    if (hostInput.hostTimer) window.clearTimeout(hostInput.hostTimer);
    hostInput.hostTimer = window.setTimeout(() => {
      state.hostQuery = hostInput.value.trim();
      if (state.hostQuery && (state.stats.rows.has(state.hostQuery) || hostedNames(state.tree).includes(state.hostQuery))) {
        ensureHostDetail(state.hostQuery);
      }
      draw();
    }, 120);
  });
  hostInput.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      state.hintsOpen = false;
      if (hostHints) hostHints.hidden = true;
      return;
    }
    if (event.key !== "Enter" || !hostHints || hostHints.hidden) return;
    const first = hostHints.querySelector("button");
    if (!first) return;
    event.preventDefault();
    selectHost(first.textContent);
  });
  hostInput.addEventListener("blur", () => {
    window.setTimeout(() => {
      state.hintsOpen = false;
      if (hostHints) hostHints.hidden = true;
    }, 150);
  });
  [statusSelect, failedFirst].forEach((control) => {
    control.addEventListener("change", () => draw());
  });
  const refresh = () => {
    state.tree = buildAnsibleTree([...state.events, ...detailEvents()]);
    const available = state.tree.plays.length > 0 || state.stats.total > 0;
    if (available && state.view === "raw" && !state.explicit && outputViewPreference() === "structured") {
      state.view = "structured";
    }
    if (!available) state.view = "raw";
    applyView();
    schedule();
  };
  applyView();
  refresh();

  return {
    append(item) {
      if (!pushStructuredEvent(state, item)) return;
      absorbStats(item);
      refresh();
    },
    reload() {
      refresh();
    },
    tab(show) {
      state.tab = show;
      applyView();
    },
  };
}

function bindTofuOutput(job, startedAt) {
  const switchBox = document.querySelector("#output-view");
  const box = document.querySelector("#tofu-output");
  const countsBox = document.querySelector("#tf-counts");
  const operation = document.querySelector("#tf-operation");
  const resourceBody = document.querySelector("#tf-resources");
  const noResources = document.querySelector("#tf-no-resources");
  const diagnosticsBox = document.querySelector("#tf-diagnostics");
  const noDiagnostics = document.querySelector("#tf-no-diagnostics");
  const outputsBox = document.querySelector("#tf-outputs");
  const noOutputs = document.querySelector("#tf-no-outputs");
  const limitNote = document.querySelector("#tf-limit");
  const log = document.querySelector("#log");
  const tools = document.querySelector("#output-tools");
  if (!switchBox || !box || !countsBox || !resourceBody || !diagnosticsBox || !outputsBox || !log) return null;
  const state = {
    events: [...(job.tofu_events || [])],
    seen: new Set((job.tofu_events || []).map((item) => Number(item && item.id)).filter((id) => Number.isFinite(id) && id > 0)),
    model: buildTofuModel(job.tofu_events || []),
    view: "raw",
    explicit: false,
    tab: true,
    frame: 0,
    rendered: false,
    startedAt: Number(startedAt) || 0,
  };
  const available = () => state.events.length > 0;

  const applyView = () => {
    const raw = state.view !== "structured";
    const has = available();
    switchBox.hidden = !has || !state.tab;
    switchBox.querySelectorAll("[data-output-view]").forEach((button) => {
      const active = button.dataset.outputView === state.view;
      button.classList.toggle("primary", active);
      button.setAttribute("aria-pressed", active ? "true" : "false");
    });
    box.hidden = raw || !state.tab;
    if (tools) tools.hidden = !raw || !state.tab;
    log.hidden = !raw || !state.tab;
  };

  const draw = () => {
    state.frame = 0;
    const model = state.model;
    const counts = tofuCounts(model);
    const parts = [
      ["tofu_add", counts.add, "add"],
      ["tofu_change", counts.change, "change"],
      ["tofu_destroy", counts.remove, "destroy"],
    ];
    countsBox.replaceChildren(...parts.map(([key, value, kind]) => {
      const item = stNode("div", `tf-count ${kind}`);
      item.dataset.tfCount = kind;
      item.appendChild(stNode("span", "tf-count-label", t(key)));
      const strong = stNode("strong", "tf-count-value", String(value));
      item.appendChild(strong);
      return item;
    }));
    if (operation) {
      operation.textContent = model.summary ? t(model.summary.operation === "apply" ? "tofu_operation_apply" : "tofu_operation_plan") : "";
      operation.hidden = !model.summary;
    }
    const rows = model.resources.map((row) => {
      const tr = stNode("tr", "tf-resource");
      tr.dataset.tfAction = row.action;
      tr.dataset.tfStatus = row.status;
      tr.dataset.tfAddress = row.address;
      const action = stNode("td", "", "");
      action.dataset.label = t("action");
      const badge = stNode("span", `tf-badge action ${TOFU_ACTIONS.includes(row.action) ? row.action : "create"}`, t(tofuActionKey(row.action)));
      action.appendChild(badge);
      const address = stNode("td", "tf-address", row.address);
      address.dataset.label = t("address");
      const status = stNode("td", "", "");
      status.dataset.label = t("status");
      const statusBadge = stNode("span", `tf-badge status ${TOFU_STATUSES.includes(row.status) ? row.status : "planned"}`, t(tofuStatusKey(row.status)));
      status.appendChild(statusBadge);
      if (row.elapsed !== null) status.appendChild(stNode("span", "tf-elapsed", formatDuration(row.elapsed)));
      tr.appendChild(action);
      tr.appendChild(address);
      tr.appendChild(status);
      return tr;
    });
    resourceBody.replaceChildren(...rows);
    if (noResources) noResources.hidden = rows.length > 0;

    const diagnostics = model.diagnostics.map((diagnostic) => {
      const severity = TOFU_SEVERITIES.includes(diagnostic.severity) ? diagnostic.severity : "error";
      const item = stNode("article", `tf-diagnostic ${severity}`);
      item.dataset.tfSeverity = severity;
      const head = stNode("div", "tf-diagnostic-head");
      head.appendChild(stNode("span", `tf-badge severity ${severity}`, t(tofuSeverityKey(severity))));
      head.appendChild(stNode("span", "tf-diagnostic-summary", diagnostic.summary));
      item.appendChild(head);
      if (diagnostic.detail) item.appendChild(stNode("pre", "st-pre", diagnostic.detail));
      return item;
    });
    diagnosticsBox.replaceChildren(...diagnostics);
    if (noDiagnostics) noDiagnostics.hidden = diagnostics.length > 0;

    const outputs = model.outputs.map((output) => {
      const item = stNode("div", "tf-output");
      item.dataset.tfOutput = output.name;
      item.dataset.tfSensitive = String(output.sensitive);
      item.appendChild(stNode("span", "tf-output-name", output.name));
      item.appendChild(stNode(
        "span",
        output.sensitive ? "small muted tf-output-value" : "tf-output-value mono",
        output.sensitive ? t("tofu_sensitive") : output.value,
      ));
      return item;
    });
    outputsBox.replaceChildren(...outputs);
    if (noOutputs) noOutputs.hidden = outputs.length > 0;
    if (limitNote) {
      limitNote.hidden = !model.truncated;
      limitNote.textContent = model.truncated ? t("structured_output_limited", { count: model.limit || 2000 }) : "";
    }
    box.dataset.resourceCount = String(model.resources.length);
    box.dataset.summaryAdd = String(counts.add);
    box.dataset.summaryChange = String(counts.change);
    box.dataset.summaryRemove = String(counts.remove);
    if (!state.rendered && state.view === "structured") {
      state.rendered = true;
      if (state.startedAt) box.dataset.renderMs = String(Math.max(0, Math.round(performance.now() - state.startedAt)));
    }
  };

  const schedule = () => {
    if (state.frame) return;
    const run = () => draw();
    state.frame = window.requestAnimationFrame ? window.requestAnimationFrame(run) : window.setTimeout(run, 16);
  };

  switchBox.querySelectorAll("[data-output-view]").forEach((button) => {
    button.addEventListener("click", () => {
      state.view = button.dataset.outputView === "raw" ? "raw" : "structured";
      state.explicit = true;
      rememberOutputView(state.view);
      applyView();
    });
  });

  const refresh = () => {
    state.model = buildTofuModel(state.events);
    if (available() && state.view === "raw" && !state.explicit && outputViewPreference() === "structured") {
      state.view = "structured";
    }
    if (!available()) state.view = "raw";
    applyView();
    schedule();
  };

  applyView();
  refresh();

  return {
    append(item) {
      if (!pushStructuredEvent(state, item)) return;
      refresh();
    },
    reload() {
      refresh();
    },
    tab(show) {
      state.tab = show;
      applyView();
    },
  };
}

async function pageJob(id) {
  const live = appState.generation;
  const startedAt = performance.now();
  const job = await api("GET", `/api/v1/jobs/${encodeURIComponent(id)}`);
  const running = job.status === "pending" || job.status === "running";
  const actions = jobActions(job);
  shell(job.name, `
    <div class="spread" style="margin-bottom:12px">
      <div class="row">${pill(job.status)} <span class="tool">${esc(job.tool)}</span> <span class="small muted">${esc(job.trigger)} · ${esc(job.launched_by)} · ${esc(when(job.started_at || job.created_at))} · <span id="job-duration">${esc(spanText(job.started_at, job.finished_at, job.status))}</span></span><span class="small muted" id="job-exit-code" hidden></span></div>
    </div>
    <div class="job-sticky">
      <div class="spread tab-bar">
        <div class="tabs">
          <button class="btn pill primary" type="button" data-job-tab="output">${h("output")}</button>
          <button class="btn pill" type="button" data-job-tab="parameters">${h("parameters")}</button>
        </div>
        <div class="row">
          ${actions}
          ${appState.assistantEnabled && job.status === "failed" ? `<button class="btn" type="button" id="assistant-explain">${h("assistant_explain_failure")}</button>` : ""}
          <button class="btn" type="button" id="download-output">${h("download_output")}</button>
          ${job.output_capped ? `<span class="small muted" id="output-capped">${h("output_download_is_incomplete")}</span>` : ""}
        </div>
      </div>
      <div class="log-tools" id="output-tools">
        <input class="search" id="log-search" placeholder="${h("search_log")}" aria-label="${h("search_log")}" autocomplete="off">
        <span class="small muted" id="log-search-count"></span>
        <div class="row">
          <button class="btn" type="button" id="log-first-failure" hidden>${h("jump_to_first_failure")}</button>
          <label class="row"><input id="log-wrap" type="checkbox"> ${h("wrap_lines")}</label>
          <button class="btn" type="button" id="log-jump-end" hidden>${h("jump_to_end")}</button>
        </div>
      </div>
      <div class="output-view" id="output-view" hidden>
        <div class="tabs" role="group" aria-label="${h("output_view")}">
          <button class="btn pill" type="button" data-output-view="structured" aria-pressed="false">${h("structured_output")}</button>
          <button class="btn pill primary" type="button" data-output-view="raw" aria-pressed="true">${h("raw_output")}</button>
        </div>
      </div>
    </div>
    <div id="panel-output">
      <div class="log-summary" id="log-summary" hidden></div>
      <div class="structured tofu" id="tofu-output" hidden>
        <section class="tf-summary" id="tf-summary">
          <div class="spread tf-summary-head">
            <h2>${h("tofu_summary")}</h2>
            <span class="small muted" id="tf-operation" hidden></span>
          </div>
          <div class="tf-counts" id="tf-counts"></div>
        </section>
        <section class="tf-section">
          <h2>${h("resources")}</h2>
          <div class="table-wrap">
            <table class="table tf-resources">
              <thead><tr><th>${h("action")}</th><th>${h("address")}</th><th>${h("status")}</th></tr></thead>
              <tbody id="tf-resources"></tbody>
            </table>
          </div>
          <p class="small muted tf-empty" id="tf-no-resources" hidden>${h("tofu_no_resources")}</p>
        </section>
        <section class="tf-section">
          <h2>${h("tofu_diagnostics")}</h2>
          <div class="tf-diagnostics" id="tf-diagnostics"></div>
          <p class="small muted tf-empty" id="tf-no-diagnostics" hidden>${h("tofu_no_diagnostics")}</p>
        </section>
        <section class="tf-section">
          <h2>${h("tofu_outputs")}</h2>
          <div class="tf-outputs" id="tf-outputs"></div>
          <p class="small muted tf-empty" id="tf-no-outputs" hidden>${h("tofu_no_outputs")}</p>
        </section>
        <p class="small muted tf-empty" id="tf-limit" hidden></p>
      </div>
      <div class="structured" id="structured-output" hidden>
        <section class="st-recap" id="log-result" hidden>
          <div class="spread st-recap-head">
            <h2>${h("host_summary")}</h2>
            <span class="small muted" id="st-recap-total" aria-live="polite"></span>
          </div>
          <div class="st-counts" id="st-counts"></div>
          <p class="small muted st-recap-note" id="st-recap-note" hidden>${h("recap_incomplete")}</p>
        </section>
        <details class="st-host-panel" id="st-host-panel" open hidden>
          <summary class="st-host-panel-head">
            <h2>${h("host_list")}</h2>
            <span class="small muted" id="st-host-count"></span>
          </summary>
          <div class="filter-bar st-host-filters">
            <input class="search" id="host-search" type="search" placeholder="${h("search_hosts")}" aria-label="${h("search_hosts")}" autocomplete="off">
            <label class="field"><span>${h("status")}</span><select id="host-status" aria-label="${h("status")}">
              <option value="problems" selected>${h("problem_hosts")}</option>
              <option value="all">${h("all_hosts")}</option>
              ${RECAP_STATUSES.map((status) => `<option value="${status}">${esc(t(status))}</option>`).join("")}
            </select></label>
            <label class="field"><span>${h("sort_by")}</span><select id="host-sort" aria-label="${h("sort_by")}">
              <option value="severity" selected>${h("severity")}</option>
              <option value="host">${h("host")} ↑</option>
              <option value="-host">${h("host")} ↓</option>
              ${RECAP_FIELDS.map((field) => `<option value="${field}">${esc(t(field))} ↓</option>`).join("")}
            </select></label>
            <label class="field"><span>${h("page_size")}</span><select id="host-size" aria-label="${h("page_size")}">${[25, 50, 100].map((size) => `<option value="${size}"${size === 50 ? " selected" : ""}>${size}</option>`).join("")}</select></label>
          </div>
          <p class="small muted st-host-total" id="host-total" tabindex="-1" aria-live="polite"></p>
          <div class="st-host-table-body" id="host-table"></div>
          <div class="row st-host-pager" id="host-pager" tabindex="-1"></div>
        </details>
        <div class="filter-bar st-filters">
          <label class="field st-host-field"><span>${h("host")}</span>
            <input class="search" id="st-host" type="search" placeholder="${h("filter_by_host")}" aria-label="${h("filter_by_host")}" autocomplete="off">
            <div class="st-hints" id="st-host-hints" role="listbox" hidden></div>
          </label>
          <label class="field"><span>${h("status")}</span><select id="st-status" aria-label="${h("status")}">
            <option value="">${h("all_statuses")}</option>
            ${["failed", "unreachable", "changed", "ok", "skipped"].map((status) => `<option value="${status}">${esc(t(status))}</option>`).join("")}
          </select></label>
          <label class="row"><input id="st-failed-first" type="checkbox" checked> ${h("failed_first")}</label>
        </div>
        <div class="st-tree" id="st-tree"></div>
      </div>
      <div class="log" id="log" tabindex="0"></div>
    </div>
    <div id="panel-parameters" hidden>${jobParameters(job)}</div>`);
  const badge = document.querySelector(".pill");
  const explain = document.querySelector("#assistant-explain");
  if (explain) explain.addEventListener("click", () => explainFailure(job));
  bindJobLog(document.querySelector("#log"), job);
  showJobExitCode(exitCodeFromEvents(job.events));
  const structured = job.tool === "opentofu"
    ? bindTofuOutput(job, startedAt)
    : bindStructuredOutput(job, startedAt);
  (job.events || []).forEach(appendLog);
  const relaunchButtons = () => document.querySelectorAll("[data-action=relaunch], [data-action=relaunch-failed]");
  const startAgain = async (onlyFailed) => {
    if (!(await confirmDialog({ title: t("relaunch"), body: relaunchPrompt(job, onlyFailed), confirmLabel: t("relaunch") }))) return;
    relaunchButtons().forEach((button) => {
      button.disabled = true;
    });
    try {
      const copy = await api("POST", `/api/v1/jobs/${encodeURIComponent(id)}/relaunch`, { only_failed_hosts: onlyFailed });
      go(`#/jobs/${copy.id}`);
    } catch (error) {
      relaunchButtons().forEach((button) => {
        button.disabled = false;
      });
      handle(error);
    }
  };
  document.querySelectorAll("[data-action=cancel]").forEach((cancel) => {
    cancel.addEventListener("click", async () => {
      try {
        await api("POST", `/api/v1/jobs/${encodeURIComponent(id)}/cancel`, {});
        if (badge) {
          badge.className = "pill canceled";
          badge.textContent = statusLabel("canceled");
        }
        document.querySelectorAll("[data-action=cancel]").forEach((button) => button.remove());
      } catch (error) {
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-action=relaunch]").forEach((button) => {
    button.addEventListener("click", () => startAgain(false));
  });
  document.querySelectorAll("[data-action=relaunch-failed]").forEach((button) => {
    button.addEventListener("click", () => startAgain(true));
  });
  document.querySelectorAll("[data-job-tab]").forEach((button) => {
    button.addEventListener("click", () => {
      const tab = button.dataset.jobTab;
      document.querySelectorAll("[data-job-tab]").forEach((item) => {
        item.classList.toggle("primary", item === button);
      });
      document.querySelector("#panel-output").hidden = tab !== "output";
      document.querySelector("#panel-parameters").hidden = tab !== "parameters";
      document.querySelector("#output-tools").hidden = tab !== "output";
      document.querySelector("#output-view").hidden = tab !== "output";
      if (structured) structured.tab(tab === "output");
    });
  });
  document.querySelector("#download-output").addEventListener("click", async () => {
    const button = document.querySelector("#download-output");
    button.disabled = true;
    try {
      const headers = {};
      if (appState.teamId) headers["X-Team-Id"] = appState.teamId;
      const response = await fetch(`/api/v1/jobs/${encodeURIComponent(id)}/output`, { headers, credentials: "same-origin" });
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
        error.status = response.status;
        error.vars = vars;
        throw error;
      }
      const match = /filename="([^"]+)"/.exec(response.headers.get("content-disposition") || "");
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = match ? match[1] : "job.txt";
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
    } catch (error) {
      handle(error);
    } finally {
      button.disabled = false;
    }
  });
  if (!running || live !== appState.generation) return;
  const duration = document.querySelector("#job-duration");
  let durationEnd = job.finished_at;
  let durationStatus = job.status;
  const tickDuration = () => {
    if (live !== appState.generation) {
      clearInterval(durationTimer);
      return;
    }
    if (duration) duration.textContent = spanText(job.started_at, durationEnd, durationStatus);
  };
  const durationTimer = setInterval(tickDuration, 1000);
  const last = lastRawEventId(job);
  appState.setSource(new EventSource(`/api/v1/jobs/${encodeURIComponent(id)}/events?after=${last}`));
  appState.source.addEventListener("log", (event) => {
    if (live !== appState.generation) return;
    appendLog(JSON.parse(event.data));
  });
  appState.source.addEventListener("ansible", (event) => {
    if (live !== appState.generation) return;
    if (structured) structured.append(JSON.parse(event.data));
  });
  appState.source.addEventListener("tofu", (event) => {
    if (live !== appState.generation) return;
    if (structured) structured.append(JSON.parse(event.data));
  });
  appState.source.addEventListener("end", (event) => {
    if (live !== appState.generation) return;
    const payload = JSON.parse(event.data);
    if (badge) {
      badge.className = `pill ${payload.status}`;
      badge.textContent = statusLabel(payload.status);
    }
    clearInterval(durationTimer);
    durationStatus = payload.status;
    durationEnd = payload.finished_at || new Date().toISOString();
    if (duration) duration.textContent = spanText(job.started_at, durationEnd, durationStatus);
    document.querySelectorAll("[data-action=cancel]").forEach((button) => button.remove());
    appState.source.close();
    appState.setSource(null);
    if (payload.status === "failed" || payload.status === "canceled") pageJob(id);
  });
}

registerRoute("#/jobs/:id", (params) => pageJob(params.id));
registerRoute("#/jobs", pageJobs);
