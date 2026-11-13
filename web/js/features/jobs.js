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
      <div class="row">
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
        `<tr><td><a href="#/jobs/${esc(job.id)}">${esc(job.name)}</a></td><td class="tool narrow-hide">${esc(job.tool)}</td><td>${pill(job.status)}</td><td class="small narrow-hide">${esc(job.trigger)}</td><td class="small muted" data-label="${h("created")}"><time datetime="${esc(job.created_at)}" title="${esc(when(job.created_at))}">${esc(when(job.created_at))}</time><br><span class="small">${esc(relativeTime(job.created_at))}</span></td><td class="small muted narrow-hide">${esc(spanText(job.started_at, job.finished_at, job.status))}</td><td>${jobListActions(job)}</td></tr>`
      );
      jobTable.innerHTML = table([t("name"), t("tool"), t("status"), t("trigger"), t("created"), t("duration"), ""], rows);
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
    };
  }
  return log.koreografLog;
}

function paintLogLine(line, query) {
  const marks = [];
  line.replaceChildren();
  searchSegments(line.koreografRaw ?? "", query).forEach((segment) => {
    if (segment.hit) {
      const mark = document.createElement("mark");
      mark.className = "log-hit";
      mark.textContent = segment.text;
      line.appendChild(mark);
      marks.push(mark);
    } else if (segment.text) {
      line.appendChild(document.createTextNode(segment.text));
    }
  });
  return marks;
}

function renderRecap(state) {
  const box = document.querySelector("#log-result");
  if (!box) return;
  const rows = [...state.recap.values()];
  box.hidden = state.tool !== "ansible" || !rows.length;
  if (box.hidden) {
    box.replaceChildren();
    return;
  }
  const table = document.createElement("table");
  const head = document.createElement("tr");
  ["host", ...RECAP_FIELDS].forEach((key) => {
    const cell = document.createElement("th");
    cell.textContent = t(key);
    head.appendChild(cell);
  });
  table.appendChild(head);
  rows.forEach((row) => {
    const line = document.createElement("tr");
    if (recapFailed(row)) {
      line.className = "fail";
      line.dataset.failed = "1";
    }
    [row.host, ...RECAP_FIELDS.map((field) => String(row[field]))].forEach((value, index) => {
      const cell = document.createElement(index === 0 ? "th" : "td");
      cell.textContent = value;
      line.appendChild(cell);
    });
    table.appendChild(line);
  });
  box.replaceChildren(table);
}

function updateRecap(state, text) {
  if (state.tool !== "ansible") return;
  const row = parseRecapLine(text);
  if (!row) return;
  state.recap.set(row.host, row);
  renderRecap(state);
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

function jumpToSearchMatch(state, index) {
  if (!state.matches.length) {
    state.current = -1;
    updateSearchCount(state);
    return;
  }
  const total = state.matches.length;
  state.current = ((index % total) + total) % total;
  state.matches.forEach((mark) => mark.classList.remove("current"));
  const mark = state.matches[state.current];
  mark.classList.add("current");
  mark.scrollIntoView({ block: "center", inline: "nearest" });
  updateSearchCount(state);
}

function paintLogSearch(log, state, jump) {
  state.matches = [];
  state.current = -1;
  log.querySelectorAll("div").forEach((line) => {
    if (line.koreografRaw === undefined) return;
    state.matches.push(...paintLogLine(line, state.query));
  });
  if (jump && state.matches.length) jumpToSearchMatch(state, 0);
  else updateSearchCount(state);
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
  renderRecap(state);
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
    jumpToSearchMatch(state, state.current + (event.shiftKey ? -1 : 1));
  });
  log.addEventListener("scroll", () => {
    const bottom = log.scrollHeight - log.scrollTop - log.clientHeight < 24;
    state.atBottom = bottom;
    if (end) end.hidden = bottom;
  });
  if (wrap) {
    wrap.checked = false;
    log.classList.remove("wrap");
    wrap.addEventListener("change", () => log.classList.toggle("wrap", wrap.checked));
  }
  if (first) {
    first.addEventListener("click", () => {
      if (state.firstFailure) state.firstFailure.scrollIntoView({ block: "center", behavior: "smooth" });
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
  line.textContent = plainLogLine(row.line);
  line.koreografRaw = line.textContent;
  log.appendChild(line);
  if (kind === "fail") {
    markRecap(line);
    if (!state.firstFailure) {
      state.firstFailure = line;
      const first = document.querySelector("#log-first-failure");
      if (first) first.hidden = false;
    }
  }
  pinFailure(plainLogLine(row.line), kind);
  updateRecap(state, plainLogLine(row.line));
  if (state.query) {
    const marks = paintLogLine(line, state.query);
    if (marks.length) {
      state.matches.push(...marks);
      if (state.current < 0) {
        state.current = 0;
        state.matches[0].classList.add("current");
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

async function pageJob(id) {
  const live = appState.generation;
  const job = await api("GET", `/api/v1/jobs/${encodeURIComponent(id)}`);
  const running = job.status === "pending" || job.status === "running";
  const actions = jobActions(job);
  shell(job.name, `
    <div class="spread" style="margin-bottom:12px">
      <div class="row">${pill(job.status)} <span class="tool">${esc(job.tool)}</span> <span class="small muted">${esc(job.trigger)} · ${esc(job.launched_by)} · ${esc(when(job.started_at || job.created_at))} · <span id="job-duration">${esc(spanText(job.started_at, job.finished_at, job.status))}</span></span></div>
    </div>
    <div class="job-sticky">
      <div class="spread tab-bar">
        <div class="tabs">
          <button class="btn pill primary" type="button" data-job-tab="output">${h("output")}</button>
          <button class="btn pill" type="button" data-job-tab="parameters">${h("parameters")}</button>
        </div>
        <div class="row">
          ${actions}
          ${appState.assistantEnabled && job.status === "failed" ? `<a class="btn" href="#/assistant?explain=${encodeURIComponent(job.id)}">${h("assistant_explain_failure")}</a>` : ""}
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
    </div>
    <div id="panel-output">
      <div class="result-summary" id="log-result" tabindex="0" hidden></div>
      <div class="log-summary" id="log-summary" hidden></div>
      <div class="log" id="log" tabindex="0"></div>
    </div>
    <div id="panel-parameters" hidden>${jobParameters(job)}</div>`);
  const badge = document.querySelector(".pill");
  bindJobLog(document.querySelector("#log"), job);
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
  const last = (job.events || []).reduce((max, event) => Math.max(max, event.id), 0);
  appState.setSource(new EventSource(`/api/v1/jobs/${encodeURIComponent(id)}/events?after=${last}`));
  appState.source.addEventListener("log", (event) => {
    if (live !== appState.generation) return;
    appendLog(JSON.parse(event.data));
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
