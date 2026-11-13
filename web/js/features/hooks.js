import { api } from "../core/api.js";
import { confirmDialog } from "../core/dialog.js";
import { can, go, handle, markSaved, needTeam, route, shell } from "../core/app.js";
import { addLink, esc, h, table } from "../core/dom.js";
import { clearEditHash, editQuery, openEditDialog } from "../core/edit.js";
import { pill, when } from "../core/format.js";
import { jsonObject } from "../core/forms.js";
import { t } from "../core/i18n.js";
import { registerRedirect, registerRoute } from "../core/router.js";

function hookStatusLabel(value) {
  return {
    firing: t("hook_status_firing"),
    resolved: t("hook_status_resolved"),
    any: t("hook_status_any"),
  }[value] || value;
}

export function hookMatches(hook, needle) {
  return !needle || `${hook.name} ${hook.match_alertname} ${hook.match_status}`.toLowerCase().includes(needle);
}

export function hookTarget(rawTarget) {
  const splitAt = rawTarget.indexOf(":");
  if (splitAt < 1) throw new Error("create_a_template_or_a_workflow_first");
  return { kind: rawTarget.slice(0, splitAt), id: rawTarget.slice(splitAt + 1) };
}

export function hookPayload(form) {
  const target = hookTarget(form.target.value);
  return {
    name: form.name.value,
    match_alertname: form.match_alertname.value,
    match_status: form.match_status.value,
    extra_vars: jsonObject(form.extra_vars.value, "extra_variables"),
    template_id: target.kind === "template" ? target.id : null,
    workflow_id: target.kind === "workflow" ? target.id : null,
  };
}

async function pageHooks() {
  if (!needTeam()) return;
  const hooks = await api("GET", "/api/v1/hooks");
  const deliveryRows = hooks.length
    ? await api("GET", `/api/v1/hooks/${encodeURIComponent(hooks[0].id)}/deliveries`)
    : [];
  const origin = location.origin;
  const incoming = deliveryRows.map((item) =>
    `<tr><td>${esc(item.hook_name)}</td><td>${pill(item.status)}</td><td>${esc(item.summary)}</td><td class="small muted">${esc(when(item.created_at))}</td></tr>`
  );
  shell(t("webhooks"), `
    <section class="card" style="margin-bottom:14px">
      <input class="search" id="hook-search" placeholder="${h("search_webhooks")}" autocomplete="off">
      <div id="hook-table"></div>
    </section>
    <section class="card"><h2>${h("deliveries")}</h2>${table([t("hook"), t("status"), t("summary"), t("time")], incoming)}</section>`,
    can("operator") ? addLink("#/hooks?new=1", "webhook_2") : "");
  const drawHooks = () => {
    const needle = document.querySelector("#hook-search").value.trim().toLowerCase();
    const rows = hooks.filter((hook) => hookMatches(hook, needle)).map((hook) => {
      const url = hook.token ? `${origin}/hooks/${hook.id}/${hook.token}` : "";
      return `<tr>
        <td>${can("operator") ? `<a href="#/hooks?edit=${esc(hook.id)}">${esc(hook.name)}</a>` : esc(hook.name)}<div class="small muted">${esc(hook.match_alertname)} · ${esc(hookStatusLabel(hook.match_status))}</div></td>
        <td class="mono">${url ? esc(url) : h("token_is_visible_to_operators")}</td>
        <td>${can("operator") ? `<button class="btn" type="button" data-test="${esc(hook.id)}">${h("test")}</button>` : ""}</td>
        <td class="actions danger">${can("operator") ? `<button class="btn danger" type="button" data-delete="${esc(hook.id)}">${h("delete")}</button>` : ""}</td>
      </tr>`;
    });
    document.querySelector("#hook-table").innerHTML = table([t("hook"), t("address"), "", ""], rows);
    document.querySelectorAll("[data-test]").forEach((button) => {
      button.addEventListener("click", async () => {
        try {
          const result = await api("POST", `/api/v1/hooks/${encodeURIComponent(button.dataset.test)}/test`);
          if (result.workflow_job_id) go(`#/runs/${result.workflow_job_id}`);
          else if (result.job_id) go(`#/jobs/${result.job_id}`);
          else route();
        } catch (error) {
          handle(error);
        }
      });
    });
    document.querySelectorAll("[data-delete]").forEach((button) => {
      button.addEventListener("click", async () => {
        const hook = hooks.find((row) => row.id === button.dataset.delete);
        if (!(await confirmDialog({ title: t("delete"), body: t("delete_this_webhook", { name: hook ? hook.name : "" }), confirmLabel: t("delete"), danger: true }))) return;
        try {
          await api("DELETE", `/api/v1/hooks/${encodeURIComponent(button.dataset.delete)}`);
          route();
        } catch (error) {
          handle(error);
        }
      });
    });
  };
  document.querySelector("#hook-search").addEventListener("input", drawHooks);
  drawHooks();
  const request = editQuery();
  if (request.create !== null) {
    if (can("operator")) await openHookDialog(null);
    return;
  }
  if (!request.edit) return;
  const item = hooks.find((row) => row.id === request.edit);
  if (!item) {
    clearEditHash();
    return;
  }
  if (can("operator")) await openHookDialog(item);
}

async function openHookDialog(item) {
  const editing = Boolean(item);
  const [templates, workflows] = await Promise.all([
    api("GET", "/api/v1/templates"),
    api("GET", "/api/v1/workflows"),
  ]);
  const targetId = editing ? (item.workflow_id || item.template_id || "") : "";
  const option = (value, label, selected) =>
    `<option value="${value}"${value === selected ? " selected" : ""}>${label}</option>`;
  const status = editing ? item.match_status : "firing";
  const targetOption = (kind, row) =>
    option(`${kind}:${row.id}`, esc(row.name), `${kind}:${targetId}`);
  openEditDialog({
    id: "hook-dialog",
    title: editing ? t("edit_webhook") : t("new_webhook"),
    body: `
      <label class="field required"><span>${h("name")}</span><input name="name" required value="${editing ? esc(item.name) : ""}"></label>
      <div class="field-row">
        <label class="field required"><span>${h("alert_name")}</span><input class="fit" name="match_alertname" required placeholder="DiskSpaceLow" value="${editing ? esc(item.match_alertname) : ""}"></label>
        <label class="field"><span>${h("status")}</span>
          <select class="fit" name="match_status">${option("firing", h("hook_status_firing"), status)}${option("resolved", h("hook_status_resolved"), status)}${option("any", h("hook_status_any"), status)}</select>
        </label>
      </div>
      <label class="field required"><span>${h("target")}</span>
        <select name="target">
          <option value=""${editing ? "" : " selected"}>${h("select")}</option>
          <optgroup label="${h("workflow")}">${workflows.map((row) => targetOption("workflow", row)).join("")}</optgroup>
          <optgroup label="${h("template")}">${templates.map((row) => targetOption("template", row)).join("")}</optgroup>
        </select>
      </label>
      <label class="field"><span>${h("extra_variables_json")}</span><textarea name="extra_vars">${editing ? esc(JSON.stringify(item.extra_vars || {}, null, 2)) : "{}"}</textarea></label>`,
    submit: editing ? t("save") : t("create"),
    onSubmit: async ({ form, close }) => {
      const payload = hookPayload(form);
      if (editing) await api("PUT", `/api/v1/hooks/${encodeURIComponent(item.id)}`, payload);
      else await api("POST", "/api/v1/hooks", payload);
      markSaved();
      close();
      await route();
    },
  });
}

registerRedirect("#/hooks/new", "#/hooks?new=1");
registerRedirect("#/hooks/:id", ({ id }) => `#/hooks?edit=${encodeURIComponent(id)}`);
registerRoute("#/hooks", pageHooks);
