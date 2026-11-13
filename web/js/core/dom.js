import { t } from "./i18n.js";

export function esc(value) {
  return String(value ?? "").replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }[ch]));
}

export function h(text, vars) {
  return esc(t(text, vars));
}

export function addLink(href, label) {
  return `<a class="btn pill primary" href="${href}">${h(label)}</a>`;
}

export function emptyState(message, href, label) {
  const action = href && label ? `<a class="btn primary" href="${href}">${h(label)}</a>` : "";
  return `<div class="empty-state"><p>${h(message)}</p>${action}</div>`;
}

export function table(headers, rows) {
  const body = rows.length
    ? rows.join("")
    : `<tr><td class="muted" colspan="${headers.length}">${h("nothing_here")}</td></tr>`;
  return `<div class="table-wrap"><table class="table"><thead><tr>${headers.map((item) => `<th>${esc(item)}</th>`).join("")}</tr></thead><tbody>${body}</tbody></table></div>`;
}
