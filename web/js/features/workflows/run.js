import * as appState from "../../core/state.js";
import { api } from "../../core/api.js";
import { openDialog } from "../../core/dialog.js";
import { handle } from "../../core/app.js";
import { esc, h } from "../../core/dom.js";
import { pill, spanText, statusLabel } from "../../core/format.js";
import { currentLanguage, t } from "../../core/i18n.js";
import { appendLog } from "../jobs.js";
import { mountEditor } from "./editor.js";

function clock(value) {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleTimeString(currentLanguage() === "en" ? "en-GB" : "de-DE", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  });
}

export async function pageRun(id) {
  const live = appState.generation;
  const phases = {};
  let editor = null;
  let openNodeId = null;
  let shownJob = "";
  let outputToken = 0;

  function adopt(run) {
    Object.keys(phases).forEach((key) => delete phases[key]);
    (run.nodes || []).forEach((node) => { phases[node.node_id] = node; });
    return {
      id: run.id,
      workflow_id: run.workflow_id || "",
      name: run.name,
      description: "",
      status: run.status,
      graph: run.graph || { nodes: [], edges: [] },
    };
  }

  function nodeStart(phase) {
    if (!phase) return "";
    return phase.started_at || (phase.phase === "running" ? phase.job_created_at : "") || "";
  }

  function renderLog(run) {
    const host = document.querySelector("#run-log");
    if (!host) return;
    const graphNodes = (run.graph && run.graph.nodes) || [];
    const ranked = graphNodes.map((node, index) => ({ node, index, phase: phases[node.id] }));
    ranked.sort((left, right) => {
      const a = nodeStart(left.phase);
      const b = nodeStart(right.phase);
      if (a && b && a !== b) return a < b ? -1 : 1;
      if (a && !b) return -1;
      if (!a && b) return 1;
      return left.index - right.index;
    });
    const rows = ranked.map(({ node, phase }) => {
      const status = phase ? phase.phase : "pending";
      const start = nodeStart(phase);
      const end = phase && phase.finished_at ? phase.finished_at : "";
      return `<button class="run-line ${esc(status)}" type="button" data-run-node="${esc(node.id)}">
        <span class="mono">${esc(clock(start))}</span>
        <span>${esc(node.name)}</span>
        <span>${esc(statusLabel(status))}</span>
        <span class="mono">${esc(clock(end))}</span>
        <span class="mono">${esc(spanText(start, end, status))}</span>
      </button>`;
    }).join("");
    host.innerHTML = `
      <h2>${h("status")}</h2>
      <div class="run-lines">
        <div class="run-line head">
          <span>${h("start")}</span>
          <span>${h("name")}</span>
          <span>${h("status")}</span>
          <span>${h("end")}</span>
          <span>${h("duration")}</span>
        </div>
        ${rows}
      </div>`;
    host.querySelectorAll("[data-run-node]").forEach((row) => {
      row.addEventListener("click", () => {
        const node = graphNodes.find((item) => item.id === row.dataset.runNode);
        if (node) openOutput(node, phases[node.id] || null);
      });
    });
  }

  function overlayMeta(phase) {
    const status = phase ? phase.phase : "pending";
    const start = nodeStart(phase);
    const end = phase && phase.finished_at ? phase.finished_at : "";
    const tool = phase && phase.tool ? `<span class="tool">${esc(phase.tool)}</span>` : "";
    const link = phase && phase.job_id
      ? `<a href="#/jobs/${esc(phase.job_id)}">${h("open_the_job")}</a>`
      : "";
    return `${pill(status)} ${tool} <span class="small muted">${esc(clock(start))} – ${esc(clock(end))} · ${esc(spanText(start, end, status))}</span> ${link}`;
  }

  function closeOutput() {
    outputToken += 1;
    openNodeId = null;
    shownJob = "";
    const overlay = document.querySelector("#node-output");
    if (overlay) overlay.remove();
    if (appState.source) {
      appState.source.close();
      appState.setSource(null);
    }
  }

  async function openOutput(node, phase) {
    if (live !== appState.generation) return;
    const token = ++outputToken;
    openNodeId = node.id;
    const jobId = phase && phase.job_id ? phase.job_id : "";
    shownJob = jobId;
    if (appState.source) {
      appState.source.close();
      appState.setSource(null);
    }
    let overlay = document.querySelector("#node-output");
    if (!overlay) {
      overlay = openDialog({
        id: "node-output",
        title: node.name,
        body: `<p class="small row" id="overlay-meta"></p><div class="log" id="log"></div>`,
        form: false,
        actions: false,
        wide: true,
        onClose: closeOutput,
      }).overlay;
    }
    overlay.querySelector("h2").textContent = node.name;
    document.querySelector("#overlay-meta").innerHTML = overlayMeta(phase);
    const log = document.querySelector("#log");
    log.replaceChildren();
    delete log.dataset.galaxy;
    const note = (text) => {
      const line = document.createElement("div");
      line.className = "system";
      line.textContent = text;
      log.appendChild(line);
    };
    if (!jobId) {
      note(phase && phase.phase === "skipped"
        ? t("this_node_was_skipped")
        : t("this_node_has_no_output_yet"));
      return;
    }
    try {
      const job = await api("GET", `/api/v1/jobs/${encodeURIComponent(jobId)}`);
      if (token !== outputToken || live !== appState.generation) return;
      (job.events || []).forEach(appendLog);
      if (!log.childNodes.length) note(t("this_node_has_no_output_yet"));
      const running = job.status === "pending" || job.status === "running";
      if (!running) return;
      const last = (job.events || []).reduce((max, event) => Math.max(max, event.id), 0);
      appState.setSource(new EventSource(`/api/v1/jobs/${encodeURIComponent(jobId)}/events?after=${last}`));
      appState.source.addEventListener("log", (event) => {
        if (token !== outputToken || live !== appState.generation) return;
        appendLog(JSON.parse(event.data));
      });
      appState.source.addEventListener("end", (event) => {
        if (token !== outputToken || live !== appState.generation) return;
        const payload = JSON.parse(event.data);
        const badge = document.querySelector("#node-output .pill");
        if (badge) {
          badge.className = `pill ${payload.status}`;
          badge.textContent = statusLabel(payload.status);
        }
        appState.source.close();
        appState.setSource(null);
      });
    } catch (error) {
      if (token === outputToken && live === appState.generation) handle(error);
    }
  }

  function paint(run, templates) {
    const view = adopt(run);
    if (!editor) {
      editor = mountEditor(view, templates || [], true, phases, null, { onNode: openOutput });
    } else if (editor) {
      editor.applyPhases();
      const badge = document.querySelector("#run-state");
      if (badge) {
        badge.className = `pill ${run.status || "pending"}`;
        badge.textContent = statusLabel(run.status || "pending");
      }
    }
    renderLog(run);
    if (!openNodeId || !document.querySelector("#node-output")) return;
    const node = ((run.graph && run.graph.nodes) || []).find((item) => item.id === openNodeId);
    const phase = phases[openNodeId] || null;
    if (!node) return;
    const jobId = phase && phase.job_id ? phase.job_id : "";
    if (jobId && jobId !== shownJob) openOutput(node, phase);
    else {
      const meta = document.querySelector("#overlay-meta");
      if (meta) meta.innerHTML = overlayMeta(phase);
    }
  }

  const [run, templates] = await Promise.all([
    api("GET", `/api/v1/workflow-jobs/${encodeURIComponent(id)}`),
    api("GET", "/api/v1/templates").catch(() => []),
  ]);
  if (live !== appState.generation) return;
  paint(run, templates);
  if (["successful", "failed", "canceled"].includes(run.status)) return;
  appState.setPollTimer(setInterval(async () => {
    if (live !== appState.generation) return;
    try {
      const next = await api("GET", `/api/v1/workflow-jobs/${encodeURIComponent(id)}`);
      if (live !== appState.generation) return;
      paint(next);
      if (["successful", "failed", "canceled"].includes(next.status)) {
        clearInterval(appState.pollTimer);
        appState.setPollTimer(null);
      }
    } catch (error) {
      if (live === appState.generation) handle(error);
    }
  }, 1000));
}
