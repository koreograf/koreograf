import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { handle, previousRouteHash } from "../core/app.js";
import { esc, h } from "../core/dom.js";
import { t } from "../core/i18n.js";
import { registerRoute } from "../core/router.js";

let transcript = [];
let pending = false;
let pendingAction = null;
let context = null;
let contextHash = null;
let panelOpen = false;
let panelError = "";
let built = false;
let documentKeyBound = false;

const CONTEXT_LABELS = {
  job: "assistant_context_job",
  workflow: "assistant_context_workflow",
  run: "assistant_context_run",
  template: "assistant_context_template",
  inventory: "assistant_context_inventory",
  host: "assistant_context_host",
  group: "assistant_context_group",
  project: "assistant_context_project",
  environment: "assistant_context_environment",
};

export function assistantHistory(turns) {
  return turns.slice(0, -1).slice(-20);
}

export function assistantActionKey(tool) {
  return {
    launch_template: "assistant_confirm_launch_template",
    launch_workflow: "assistant_confirm_launch_workflow",
    cancel_job: "assistant_confirm_cancel_job",
    relaunch_job: "assistant_confirm_relaunch_job",
    sync_project: "assistant_confirm_sync_project",
    sync_inventory: "assistant_confirm_sync_inventory",
  }[tool] || "assistant_confirmation";
}

export function assistantTurnHtml(turn) {
  const fromUser = turn.role === "user";
  return `<div class="assistant-message ${fromUser ? "from-user" : "from-assistant"}">
    <p class="small muted">${h(fromUser ? "assistant_you" : "assistant")}</p>
    <p>${esc(turn.content)}</p>
  </div>`;
}

export function assistantConfirmHtml(action) {
  return `<div class="assistant-confirm" role="group" aria-label="${h("assistant_confirmation")}">
    <p class="small muted">${h("assistant_confirmation")}</p>
    <p><strong>${h(assistantActionKey(action.tool), { name: action.target })}</strong></p>
    <div class="choice-row">
      <button class="btn pill primary" type="button" data-assistant-confirm>${h("confirm")}</button>
      <button class="btn pill" type="button" data-assistant-cancel>${h("cancel")}</button>
    </div>
  </div>`;
}

/// The reference the client is allowed to send. It carries the page type and
/// the record id only; the server loads names and text with the caller's rights.
export function assistantContextForHash(hash) {
  const [rawPath, rawQuery] = String(hash || "").replace(/^#/, "").split("?");
  const parts = rawPath.split("/").filter(Boolean);
  const params = new URLSearchParams(rawQuery || "");
  const segment = (index) => {
    const value = parts[index];
    if (!value) return "";
    try {
      return decodeURIComponent(value);
    } catch {
      return value;
    }
  };
  switch (parts[0]) {
    case "jobs":
      return parts[1] ? { type: "job", id: segment(1) } : { type: "job" };
    case "workflows":
      return parts[1] ? { type: "workflow", id: segment(1) } : { type: "workflow" };
    case "runs":
      return parts[1] ? { type: "run", id: segment(1) } : null;
    case "templates":
      return params.get("edit") ? { type: "template", id: params.get("edit") } : { type: "template" };
    case "inventories":
      if (parts[2] === "hosts" && parts[3]) {
        return { type: "host", id: `${segment(1)}/${segment(3)}` };
      }
      if (parts[2] === "groups" && parts[3]) {
        return { type: "group", id: `${segment(1)}/${segment(3)}` };
      }
      return parts[1] ? { type: "inventory", id: segment(1) } : { type: "inventory" };
    case "projects":
      return parts[1] ? { type: "project", id: segment(1) } : { type: "project" };
    case "environments":
      return parts[1] ? { type: "environment", id: segment(1) } : { type: "environment" };
    case "runtime":
      return params.get("edit") ? { type: "environment", id: params.get("edit") } : { type: "environment" };
    default:
      return null;
  }
}

export function assistantContextLabel(reference) {
  if (!reference) return "";
  const type = t(CONTEXT_LABELS[reference.type] || "assistant_context");
  const name = reference.name || reference.id;
  return name ? `${type} · ${name}` : type;
}

function renderContext() {
  const row = document.querySelector("#assistant-context-row");
  const label = document.querySelector("#assistant-context-label");
  if (!row || !label) return;
  row.hidden = !context;
  label.textContent = assistantContextLabel(context);
  if (context && context.id) label.title = context.id;
  else label.removeAttribute("title");
}

function adoptRouteContext() {
  const hash = location.hash || "#/";
  if (hash === contextHash) return;
  context = assistantContextForHash(hash);
  contextHash = hash;
  renderContext();
}

function renderThread() {
  const thread = document.querySelector("#assistant-thread");
  if (!thread) return;
  const turns = transcript.map(assistantTurnHtml).join("");
  const waiting = pending
    ? `<div class="assistant-message from-assistant"><p class="small muted">${h("assistant")}</p><p>${h("assistant_thinking")}</p></div>`
    : "";
  const action = pendingAction ? assistantConfirmHtml(pendingAction) : "";
  thread.innerHTML = turns || waiting || action ? `${turns}${waiting}${action}` : `<p class="small muted">${h("assistant_empty")}</p>`;
  bindAssistantAction();
  thread.scrollTop = thread.scrollHeight;
}

function renderError() {
  const box = document.querySelector("#assistant-error");
  if (!box) return;
  box.hidden = !panelError;
  box.innerHTML = panelError ? `<div class="banner bad" role="alert">${esc(panelError)}</div>` : "";
}

function showAssistantError(error) {
  if (error && error.status === 401) {
    handle(error);
    return;
  }
  panelError = t((error && error.message) || "error.generic", error && error.vars);
  renderError();
}

function bindAssistantAction() {
  const confirm = document.querySelector("[data-assistant-confirm]");
  const cancel = document.querySelector("[data-assistant-cancel]");
  const finish = (text) => {
    pendingAction = null;
    transcript.push({ role: "assistant", content: text });
    renderThread();
  };
  if (confirm) {
    confirm.addEventListener("click", async () => {
      const action = pendingAction;
      if (!action) return;
      confirm.disabled = true;
      if (cancel) cancel.disabled = true;
      try {
        await api("POST", "/api/v1/assistant/confirm", { id: action.id });
        finish(t("assistant_action_done"));
      } catch (error) {
        if (cancel) cancel.disabled = false;
        confirm.disabled = false;
        showAssistantError(error);
      }
    });
  }
  if (cancel) {
    cancel.addEventListener("click", async () => {
      const action = pendingAction;
      if (!action) return;
      confirm.disabled = true;
      cancel.disabled = true;
      try {
        await api("POST", "/api/v1/assistant/cancel", { id: action.id });
      } catch (error) {
        if (confirm) confirm.disabled = false;
        cancel.disabled = false;
        showAssistantError(error);
        return;
      }
      finish(t("assistant_action_canceled"));
    });
  }
}

function setSending(sending) {
  const button = document.querySelector("#assistant-send");
  if (button) button.disabled = sending;
}

async function send(text) {
  if (!text || pending || pendingAction) return;
  const reference = context;
  transcript.push({ role: "user", content: text });
  panelError = "";
  renderError();
  const form = document.querySelector("#assistant-form");
  if (form) form.message.value = "";
  pending = true;
  setSending(true);
  renderThread();
  try {
    const body = { message: text, history: assistantHistory(transcript) };
    if (reference) {
      body.context = { type: reference.type };
      if (reference.id) body.context.id = reference.id;
    }
    const result = await api("POST", "/api/v1/assistant/chat", body);
    if (result && result.answer) {
      transcript.push({ role: "assistant", content: String(result.answer) });
    }
    if (result && result.pending_action) pendingAction = result.pending_action;
  } catch (error) {
    showAssistantError(error);
  } finally {
    pending = false;
    setSending(false);
    renderThread();
    if (panelOpen && form && form.message) form.message.focus();
  }
}

function ensurePanel() {
  let root = document.querySelector("#assistant-root");
  if (!root) {
    root = document.createElement("div");
    root.id = "assistant-root";
    root.className = "assistant-dock";
    const model = appState.assistantModel
      ? `<span class="small muted" id="assistant-model">${h("assistant_model", { model: appState.assistantModel })}</span>`
      : `<span class="small muted" id="assistant-model"></span>`;
    root.innerHTML = `
      <button class="assistant-launcher" id="assistant-toggle" type="button" aria-expanded="false" aria-controls="assistant-panel" aria-label="${h("assistant_open")}" title="${h("assistant_open")}">
        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M4.5 5.5h15v10.5h-8.2L7 19.5v-3.5H4.5z" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/><path d="M8 9.5h8M8 12.5h5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>
      </button>
      <section class="assistant-panel" id="assistant-panel" role="dialog" aria-labelledby="assistant-panel-title" hidden>
        <div class="assistant-panel-head">
          <h2 id="assistant-panel-title">${h("assistant")}</h2>
          ${model}
          <button class="btn ghost assistant-close" id="assistant-close" type="button" aria-label="${h("assistant_close")}">×</button>
        </div>
        <div class="assistant-context-row" id="assistant-context-row" hidden>
          <span class="assistant-context-chip">
            <span class="small muted">${h("assistant_context")}</span>
            <span id="assistant-context-label"></span>
            <button class="chip-remove" id="assistant-context-clear" type="button" aria-label="${h("assistant_context_remove")}">×</button>
          </span>
        </div>
        <div class="assistant-panel-body">
          <p class="small muted assistant-hint">${h("assistant_hint")}</p>
          <div id="assistant-error" hidden></div>
          <div class="assistant-thread" id="assistant-thread" aria-live="polite"></div>
        </div>
        <form class="assistant-compose" id="assistant-form">
          <label class="field"><span>${h("assistant_message")}</span><textarea name="message" rows="3" maxlength="8000" required placeholder="${h("assistant_placeholder")}"></textarea></label>
          <div class="spread"><button class="btn pill primary" id="assistant-send" type="submit">${h("assistant_send")}</button></div>
        </form>
      </section>`;
    document.body.appendChild(root);
    root.querySelector("#assistant-toggle").addEventListener("click", () => {
      if (panelOpen) closeAssistantPanel();
      else openAssistantPanel();
    });
    root.querySelector("#assistant-close").addEventListener("click", () => closeAssistantPanel());
    root.querySelector("#assistant-context-clear").addEventListener("click", () => {
      context = null;
      contextHash = location.hash || "#/";
      renderContext();
    });
    const form = root.querySelector("#assistant-form");
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      send(form.message.value.trim());
    });
  }
  built = true;
  if (!documentKeyBound) {
    documentKeyBound = true;
    document.addEventListener("keydown", (event) => {
      if (event.key !== "Escape" || !panelOpen) return;
      if (document.querySelector(".overlay")) return;
      event.preventDefault();
      closeAssistantPanel();
    });
  }
  renderContext();
  renderThread();
  renderError();
}

function removePanel() {
  const root = document.querySelector("#assistant-root");
  if (root) root.remove();
  transcript = [];
  pending = false;
  pendingAction = null;
  context = null;
  contextHash = null;
  panelOpen = false;
  panelError = "";
  built = false;
}

export function openAssistantPanel(options = {}) {
  if (!appState.assistantEnabled || !appState.me) return false;
  ensurePanel();
  const panel = document.querySelector("#assistant-panel");
  const toggle = document.querySelector("#assistant-toggle");
  if (!panel || !toggle) return false;
  if (options.context) {
    context = { ...options.context };
    contextHash = location.hash || "#/";
  } else {
    adoptRouteContext();
  }
  panelOpen = true;
  panel.hidden = false;
  toggle.setAttribute("aria-expanded", "true");
  renderContext();
  renderThread();
  renderError();
  const message = document.querySelector("#assistant-form [name=message]");
  if (message) message.focus();
  if (options.message) send(options.message);
  return true;
}

export function closeAssistantPanel() {
  const panel = document.querySelector("#assistant-panel");
  const toggle = document.querySelector("#assistant-toggle");
  if (!panel || !toggle || !panelOpen) return;
  panel.hidden = true;
  panelOpen = false;
  toggle.setAttribute("aria-expanded", "false");
  toggle.focus();
}

export function explainFailure(job) {
  if (!appState.assistantEnabled || !job || !job.id) return false;
  return openAssistantPanel({
    context: { type: "job", id: String(job.id), name: job.name ? String(job.name) : "" },
    message: t("assistant_explain_prompt"),
  });
}

export function syncAssistantPanel() {
  if (!appState.me || !appState.assistantEnabled) {
    removePanel();
    return;
  }
  ensurePanel();
  adoptRouteContext();
}

window.addEventListener("hashchange", () => {
  if (built) adoptRouteContext();
});
window.addEventListener("koreograf:session", () => syncAssistantPanel());

// Compatibility: the old dedicated chat page and its explain links open the
// panel on the page the user came from, then replace the chat hash with it.
registerRoute("#/assistant", () => {
  const params = new URLSearchParams((location.hash || "").split("?")[1] || "");
  const explain = params.get("explain");
  syncAssistantPanel();
  if (appState.assistantEnabled) {
    if (explain) explainFailure({ id: explain });
    else openAssistantPanel();
  }
  const back = previousRouteHash();
  return {
    redirect: back && back.startsWith("#/") && !back.startsWith("#/assistant") ? back : "#/",
  };
});
