import { esc, h } from "./dom.js";
import { t } from "./i18n.js";

let focusTrigger = null;
let dialogSerial = 0;
let handle = () => {};

export function configureDialogs(onError) {
  handle = onError;
}

document.addEventListener("click", (event) => {
  const target = event.target instanceof Element ? event.target.closest("a[href], button") : null;
  if (target) focusTrigger = target;
}, true);

export function focusOpener() {
  const active = document.activeElement;
  if (active && active !== document.body && active !== document.documentElement && !active.classList.contains("skip-link")) return active;
  return focusTrigger || document.getElementById("main");
}

export function focusReturn(node) {
  if (!node) return;
  if (document.contains(node)) {
    node.focus();
  } else if (node.getAttribute && node.getAttribute("href")) {
    const href = node.getAttribute("href");
    const match = [...document.querySelectorAll("a[href]")].find((item) => item.getAttribute("href") === href);
    if (match) match.focus();
  } else if (node.id) {
    const match = document.getElementById(node.id);
    if (match) match.focus();
  }
}

export function makeModal(overlay, close) {
  const opener = focusOpener();
  const dismiss = async () => {
    if (await close() === false) return;
    focusReturn(opener);
    focusTrigger = null;
  };
  const card = overlay.querySelector(".overlay-card");
  const focusables = () => [...card.querySelectorAll("input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href], button:not([disabled])")].filter((node) => node.offsetParent !== null);
  const initial = focusables().find((node) => node.matches("input, select, textarea")) || focusables()[0];
  if (initial) initial.focus();
  overlay.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      dismiss();
      return;
    }
    if (event.key !== "Tab") return;
    const items = focusables();
    if (!items.length) return;
    const active = document.activeElement;
    const outside = !overlay.contains(active);
    if (event.shiftKey && (outside || active === items[0])) {
      event.preventDefault();
      items[items.length - 1].focus();
    } else if (!event.shiftKey && (outside || active === items[items.length - 1])) {
      event.preventDefault();
      items[0].focus();
    }
  });
  return dismiss;
}

export function openDialog({ id, formId, title, body, submit, onSubmit, danger, form = Boolean(onSubmit), actions = true, wide = false, bodyClass = "", footer = "", tabs = "", onClose, onError }) {
  const serial = ++dialogSerial;
  const titleId = `dialog-title-${serial}`;
  const shellTag = form ? "form" : "div";
  const overlay = document.createElement("div");
  overlay.className = tabs ? "overlay tabbed" : "overlay";
  if (id) overlay.id = id;
  overlay.innerHTML = `
    <div class="overlay-card dialog${wide ? " wide" : ""}${tabs ? " tabbed" : ""}" role="dialog" aria-modal="true" aria-labelledby="${titleId}">
      <div class="dialog-head">
        <h2 id="${titleId}">${esc(title)}</h2>
        <button class="btn ghost dialog-close" type="button" aria-label="${h("close")}">×</button>
      </div>
      ${tabs}
      <${shellTag} class="dialog-form">
        <div class="dialog-body${bodyClass ? ` ${bodyClass}` : ""}">${body}</div>
        ${footer ? `<div class="dialog-footer">${footer}</div>` : ""}
        ${actions ? `<div class="dialog-actions">
          <div class="dialog-danger">${danger || ""}</div>
          <div class="dialog-buttons">
            <button class="btn" type="button" data-dialog-cancel>${h("cancel")}</button>
            ${onSubmit ? `<button class="btn pill primary" type="submit">${esc(submit || t("save"))}</button>` : ""}
          </div>
        </div>` : ""}
      </${shellTag}>
    </div>`;
  document.body.appendChild(overlay);
  const card = overlay.querySelector(".overlay-card");
  const shell = overlay.querySelector(".dialog-form");
  if (formId && form) shell.id = formId;
  let dirty = false;
  shell.addEventListener("input", () => { dirty = true; });
  shell.addEventListener("change", () => { dirty = true; });
  const remove = () => {
    if (!overlay.isConnected) return true;
    if (onClose) onClose();
    overlay.remove();
    return true;
  };
  const requestClose = async () => {
    if (dirty && !(await confirmDialog({ title: t("discard_unsaved_changes"), confirmLabel: t("ok") }))) return false;
    return remove();
  };
  const dismiss = makeModal(overlay, requestClose);
  overlay.querySelector(".dialog-close").addEventListener("click", dismiss);
  const cancel = overlay.querySelector("[data-dialog-cancel]");
  if (cancel) cancel.addEventListener("click", dismiss);
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) dismiss();
  });
  if (onSubmit) {
    shell.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        await onSubmit({ form: shell, close: () => { dirty = false; dismiss(); } });
      } catch (error) {
        handle(error);
        if (onError) onError(error);
      }
    });
  }
  return { overlay, card, form: shell, close: () => { dirty = false; dismiss(); }, clean: () => { dirty = false; }, dirty: () => dirty };
}

export function confirmDialog({ title = t("confirm"), body = "", confirmLabel = t("ok"), danger = false } = {}) {
  return new Promise((resolve) => {
    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      resolve(value);
    };
    const dialog = openDialog({
      id: "confirm-dialog",
      title,
      body: body ? `<p>${esc(body)}</p>` : "",
      submit: confirmLabel,
      onSubmit: ({ close }) => {
        finish(true);
        close();
      },
      onClose: () => finish(false),
    });
    const button = dialog.form.querySelector('button[type="submit"]');
    if (danger) {
      button.classList.remove("primary");
      button.classList.add("danger");
    }
    button.focus();
  });
}
