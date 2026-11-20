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

// Each row is one complete <tr> string. Cells without a label borrow the
// matching header text so the stacked card rows stay readable on phones.
function labelCells(row, headers) {
  let column = 0;
  return String(row).replace(/<td\b((?:[^>"']|"[^"]*"|'[^']*')*)>/g, (tag, attrs) => {
    const label = headers[column];
    const span = /colspan\s*=\s*(?:"(\d+)"|'(\d+)'|(\d+))/i.exec(attrs);
    column += span ? Number(span[1] || span[2] || span[3]) : 1;
    if (!label || /data-label\s*=/i.test(attrs)) return tag;
    return `<td${attrs} data-label="${esc(label)}">`;
  });
}

export function table(headers, rows) {
  const names = headers || [];
  const body = rows.length
    ? rows.map((row) => labelCells(row, names)).join("")
    : `<tr><td class="muted" colspan="${names.length}">${h("nothing_here")}</td></tr>`;
  return `<div class="table-wrap"><table class="table"><thead><tr>${names.map((item) => `<th>${esc(item)}</th>`).join("")}</tr></thead><tbody>${body}</tbody></table></div>`;
}

export function roleOptions(selected) {
  const labels = { viewer: "viewer", operator: "operator", owner: "owner" };
  return ["viewer", "operator", "owner"].map((role) =>
    `<option value="${role}"${role === selected ? " selected" : ""}>${h(labels[role])}</option>`
  ).join("");
}

export function roleLegend() {
  return `<ul class="role-legend">
    <li><strong>${h("viewer")}</strong> ${h("a_viewer_sees_projects_inventories_templates_jobs_and")}</li>
    <li><strong>${h("operator")}</strong> ${h("an_operator_launches_jobs_and_maintains_projects_inventories")}</li>
    <li><strong>${h("owner")}</strong> ${h("an_owner_also_manages_credentials_environments_and_members")}</li>
    <li><strong>${h("administrator")}</strong> ${h("an_administrator_manages_organizations_teams_users_settings_and")}</li>
  </ul>`;
}

export function orgOptions(organizations, selected) {
  return organizations.map((item) =>
    `<option value="${esc(item.id)}"${item.id === selected ? " selected" : ""}>${esc(item.name)}</option>`
  ).join("");
}

export function selectOptions(items, selected, allowEmpty, label) {
  const empty = allowEmpty ? `<option value="">—</option>` : "";
  const options = items.map((item) =>
    `<option value="${esc(item.id)}"${item.id === selected ? " selected" : ""}>${esc(label(item))}</option>`
  ).join("");
  return empty + options;
}

export function paramRow(label, value) {
  if (value === undefined || value === null || value === "") return "";
  return `<dt>${h(label)}</dt><dd>${esc(value)}</dd>`;
}

export function paramRowHtml(label, html) {
  if (!html) return "";
  return `<dt>${h(label)}</dt><dd>${html}</dd>`;
}
