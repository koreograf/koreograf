import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { handle, markSaved, route, shell } from "../core/app.js";
import { addLink, esc, h, table } from "../core/dom.js";
import { editQuery, openEditDialog, rememberEditHash } from "../core/edit.js";
import { t } from "../core/i18n.js";
import { registerRedirect, registerRoute } from "../core/router.js";

export function organizationMatches(item, needle) {
  return !needle || `${item.name} ${item.description || ""}`.toLowerCase().includes(needle);
}

export function organizationPayload(form) {
  return { name: form.name.value, description: form.description.value };
}

function openOrganizationDialog(item, readOnly = false, initialTab = "") {
  const editing = Boolean(item);
  const general = `
    <label class="field required"><span>${h("name")}</span><input name="name" required value="${editing ? esc(item.name) : ""}"></label>
    <label class="field"><span>${h("description")}</span><input name="description" maxlength="1000" value="${editing ? esc(item.description || "") : ""}"></label>`;
  const rows = editing ? (item.teams || []).map((team) =>
    `<tr><td>${esc(team.name)}</td><td>${esc(team.description || "")}</td><td class="small muted">${esc(team.ldap_group || "")}</td></tr>`
  ) : [];
  const tabs = editing ? [
    { id: "general", label: t("general"), body: general },
    { id: "teams", label: t("teams"), body: table([t("name"), t("description"), t("ldap_group")], rows) },
  ] : null;
  openEditDialog({
    id: "organization-dialog",
    title: editing ? item.name : t("new_organization"),
    tabs,
    body: general,
    hint: readOnly ? "" : undefined,
    submit: editing ? t("save") : t("create"),
    actions: !readOnly,
    onSubmit: readOnly ? undefined : async ({ form, close }) => {
      if (editing) {
        await api("PUT", `/api/v1/organizations/${encodeURIComponent(item.id)}`, organizationPayload(form));
        appState.setMe(await api("GET", "/api/v1/me"));
      } else {
        await api("POST", "/api/v1/organizations", organizationPayload(form));
      }
      markSaved();
      close();
      await route();
    },
    initialTab,
    onTab: editing ? (tab) => rememberEditHash({ edit: item.id, tab }, false) : undefined,
  });
}

async function pageOrganizations() {
  const items = await api("GET", "/api/v1/organizations");
  shell(t("organizations"), `
    <section class="card">
      <input class="search" id="organization-search" placeholder="${h("search_organizations")}" autocomplete="off">
      <div id="organization-table"></div>
    </section>`,
    appState.me.is_admin ? addLink("#/organizations?new=1", "organization_2") : "");
  const drawOrganizations = () => {
    const needle = document.querySelector("#organization-search").value.trim().toLowerCase();
    const rows = items.filter((item) => organizationMatches(item, needle)).map((item) =>
      `<tr><td><a href="#/organizations?edit=${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(item.description || "")}</td></tr>`
    );
    document.querySelector("#organization-table").innerHTML = table([t("name"), t("description")], rows);
  };
  document.querySelector("#organization-search").addEventListener("input", drawOrganizations);
  drawOrganizations();
  const request = editQuery();
  if (request.create !== null) {
    if (appState.me.is_admin) openOrganizationDialog(null);
    return;
  }
  if (!request.edit) return;
  try {
    const item = await api("GET", `/api/v1/organizations/${encodeURIComponent(request.edit)}`);
    openOrganizationDialog(item, !appState.me.is_admin, request.tab);
  } catch (error) {
    handle(error);
  }
}

registerRedirect("#/organizations/new", "#/organizations?new=1");
registerRedirect("#/organizations/:id", ({ id }) => `#/organizations?edit=${encodeURIComponent(id)}`);
registerRoute("#/organizations", pageOrganizations);
