import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { handle, shell } from "../core/app.js";
import { esc, h } from "../core/dom.js";
import { t } from "../core/i18n.js";
import { registerRoute } from "../core/router.js";

let transcript = [];
let pending = false;
let pendingAction = null;

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

function renderAssistantThread() {
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

function bindAssistantAction() {
  const confirm = document.querySelector("[data-assistant-confirm]");
  const cancel = document.querySelector("[data-assistant-cancel]");
  const finish = (text) => {
    pendingAction = null;
    transcript.push({ role: "assistant", content: text });
    renderAssistantThread();
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
        handle(error);
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
        handle(error);
      }
      finish(t("assistant_action_canceled"));
    });
  }
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
    if (!text || pending || pendingAction) return;
    transcript.push({ role: "user", content: text });
    form.message.value = "";
    pending = true;
    button.disabled = true;
    renderAssistantThread();
    try {
      const body = { message: text, history: assistantHistory(transcript) };
      if (jobId) body.job_id = jobId;
      const result = await api("POST", "/api/v1/assistant/chat", body);
      if (result && result.answer) {
        transcript.push({ role: "assistant", content: String(result.answer) });
      }
      if (result && result.pending_action) pendingAction = result.pending_action;
    } catch (error) {
      handle(error);
    } finally {
      pending = false;
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

registerRoute("#/assistant", pageAssistant);
