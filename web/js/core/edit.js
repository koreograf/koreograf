import * as appState from "./state.js";
import { openDialog } from "./dialog.js";
import { esc, h } from "./dom.js";
import { t } from "./i18n.js";
import { ERROR_FIELD, fieldControl } from "./forms.js";

export function editQuery(hash = location.hash || "#/") {
  const params = new URLSearchParams((hash || "").split("?")[1] || "");
  return {
    create: params.has("new") ? (params.get("new") || "1") : null,
    edit: params.get("edit") || "",
    tab: params.get("tab") || "",
  };
}

export function editAddress(params, hash = location.hash || "#/") {
  params = params || {};
  const [path, query] = (hash || "#/").split("?");
  const search = new URLSearchParams(query || "");
  ["new", "edit", "tab"].forEach((key) => search.delete(key));
  if (params.create !== undefined && params.create !== null) search.set("new", params.create || "1");
  if (params.edit) search.set("edit", params.edit);
  if (params.tab) search.set("tab", params.tab);
  const rest = search.toString();
  return `${path}${rest ? `?${rest}` : ""}`;
}

export function rememberEditHash(params, push = true) {
  const next = editAddress(params);
  if (push) history.pushState({ idx: ((history.state && history.state.idx) || 0) + 1 }, "", next);
  else history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", next);
  appState.setShownHash(next);
}

export function clearEditHash() {
  const next = editAddress({});
  if ((location.hash || "#/") === next) return;
  history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", next);
  appState.setShownHash(next);
}

export function openEditDialog({
  id,
  title,
  tabs,
  body = "",
  hint = "fields_marked_with_a_star_are_required",
  submit,
  onSubmit,
  danger,
  form = Boolean(onSubmit),
  actions = true,
  wide = false,
  initialTab = "",
  onClose,
  onTab,
}) {
  const entries = tabs || [];
  const tabbed = entries.length > 0;
  const serial = appState.setEditSerial(appState.editSerial + 1);
  const selected = entries.some((entry) => entry.id === initialTab) ? initialTab : (entries[0] || {}).id;
  const tabBar = tabbed
    ? `<div class="dialog-tabs" role="tablist" aria-label="${h("sections")}">${entries.map((entry) => {
      const active = entry.id === selected;
      return `<button class="dialog-tab" type="button" role="tab" data-dialog-tab="${esc(entry.id)}" aria-selected="${active}" aria-controls="dialog-panel-${serial}-${esc(entry.id)}" tabindex="${active ? "0" : "-1"}">${esc(entry.label)}</button>`;
    }).join("")}</div>`
    : "";
  const panelMarkup = tabbed
    ? `<div class="dialog-panels">${entries.map((entry) => `<div class="dialog-panel" id="dialog-panel-${serial}-${esc(entry.id)}" role="tabpanel" data-dialog-panel="${esc(entry.id)}"${entry.id === selected ? "" : " hidden"}>${entry.body}</div>`).join("")}</div>`
    : body;
  const dialog = openDialog({
    id,
    formId: id ? `${id}-form` : undefined,
    title,
    tabs: tabBar,
    body: `${hint ? `<p class="small muted dialog-hint">${h(hint)}</p>` : ""}${panelMarkup}`,
    submit,
    onSubmit,
    danger,
    form,
    actions,
    wide,
    onClose: () => {
      appState.setLeaveGuard(null);
      clearEditHash();
      if (onClose) onClose();
    },
    onError: (error) => {
      if (!tabbed) return;
      const fieldName = (error.vars && error.vars.field) || ERROR_FIELD[error.message] || "";
      const control = fieldName ? fieldControl(fieldName) : document.activeElement;
      const panel = control && control.closest ? control.closest("[data-dialog-panel]") : null;
      if (panel) {
        mark(panel.dataset.dialogPanel);
        choose(panel.dataset.dialogPanel, false);
      }
    },
  });
  dialog.overlay.setAttribute("data-edit-overlay", "");
  appState.setLeaveGuard(() => dialog.dirty());
  if (!tabbed) return dialog;
  const buttons = [...dialog.overlay.querySelectorAll("[data-dialog-tab]")];
  const panelNodes = [...dialog.overlay.querySelectorAll("[data-dialog-panel]")];
  const activate = (tabId, focusTab) => {
    const current = entries.some((entry) => entry.id === tabId) ? tabId : entries[0].id;
    buttons.forEach((button) => {
      const active = button.dataset.dialogTab === current;
      button.setAttribute("aria-selected", String(active));
      button.tabIndex = active ? 0 : -1;
      if (active && focusTab) button.focus();
    });
    panelNodes.forEach((panel) => {
      panel.hidden = panel.dataset.dialogPanel !== current;
    });
  };
  const choose = (tabId, focusTab) => {
    activate(tabId, focusTab);
    if (onTab) onTab(tabId);
  };
  buttons.forEach((button, index) => {
    button.addEventListener("click", () => choose(button.dataset.dialogTab, false));
    button.addEventListener("keydown", (event) => {
      let next = null;
      if (event.key === "ArrowRight") next = (index + 1) % buttons.length;
      else if (event.key === "ArrowLeft") next = (index - 1 + buttons.length) % buttons.length;
      else if (event.key === "Home") next = 0;
      else if (event.key === "End") next = buttons.length - 1;
      if (next === null) return;
      event.preventDefault();
      choose(buttons[next].dataset.dialogTab, true);
    });
  });
  const mark = (tabId) => {
    const button = buttons.find((item) => item.dataset.dialogTab === tabId);
    if (!button) return;
    button.classList.add("has-error");
    button.setAttribute("aria-invalid", "true");
  };
  dialog.form.addEventListener("invalid", (event) => {
    const panel = event.target.closest && event.target.closest("[data-dialog-panel]");
    if (!panel) return;
    mark(panel.dataset.dialogPanel);
    choose(panel.dataset.dialogPanel, false);
    event.target.focus();
  }, true);
  dialog.form.addEventListener("input", (event) => {
    const panel = event.target.closest && event.target.closest("[data-dialog-panel]");
    if (!panel) return;
    if (panel.querySelector(":invalid")) return;
    const button = buttons.find((item) => item.dataset.dialogTab === panel.dataset.dialogPanel);
    if (!button) return;
    button.classList.remove("has-error");
    button.removeAttribute("aria-invalid");
  });
  return dialog;
}
