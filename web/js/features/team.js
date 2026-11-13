import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { can, handle, markSaved, roleLabel, route, shell } from "../core/app.js";
import { addLink, esc, h, orgOptions, roleLegend, roleOptions, table } from "../core/dom.js";
import { clearEditHash, editQuery, openEditDialog } from "../core/edit.js";
import { sourceLabel } from "../core/format.js";
import { t } from "../core/i18n.js";
import { registerRedirect, registerRoute } from "../core/router.js";

export function teamPayload(form) {
  const payload = {
    name: form.name.value,
    description: form.description.value,
    ldap_group: form.ldap_group.value,
  };
  if (form.organization_id && form.organization_id.value) payload.organization_id = form.organization_id.value;
  return payload;
}

function openTeamDialog(item, organizations, defaultOrganizationId) {
  const editing = Boolean(item);
  const organizationField = appState.me.is_admin
    ? `<label class="field"><span>${h(editing ? "move_organization" : "organization")}</span><select name="organization_id">${editing ? orgOptions(organizations, item.organization_id) : `<option value="">${h("new_organization_named_after_the_team")}</option>${orgOptions(organizations, defaultOrganizationId || "")}`}</select></label>`
    : "";
  openEditDialog({
    id: "team-dialog",
    title: editing ? item.name : t("new_team_2"),
    body: `
      <label class="field required"><span>${h("name")}</span><input name="name" required value="${editing ? esc(item.name) : ""}"></label>
      <label class="field"><span>${h("description")}</span><input name="description" maxlength="1000" value="${editing ? esc(item.description || "") : ""}"></label>
      <label class="field"><span>${h("ldap_or_ad_group")}</span><input name="ldap_group" value="${editing ? esc(item.ldap_group || "") : ""}" placeholder="${h("cn_or_dn_for_example_platform")}"></label>
      ${organizationField}`,
    submit: editing ? t("save_team") : t("create"),
    onSubmit: async ({ form, close }) => {
      const payload = teamPayload(form);
      if (editing) {
        await api("PUT", `/api/v1/teams/${encodeURIComponent(item.id)}`, payload);
      } else {
        const created = await api("POST", "/api/v1/teams", payload);
        appState.setTeamId(created.id);
        localStorage.setItem("koreograf.team", appState.teamId);
      }
      appState.setMe(await api("GET", "/api/v1/me"));
      markSaved();
      close();
      await route();
    },
  });
}

function openMemberDialog() {
  openEditDialog({
    id: "member-dialog",
    title: t("add_member"),
    body: `
      <label class="field required"><span>${h("username")}</span><input name="username" required></label>
      <div class="field-row">
        <label class="field"><span>${h("role")}</span>
          <select class="fit" name="role"><option value="viewer">${h("viewer")}</option><option value="operator" selected>${h("operator")}</option><option value="owner">${h("owner")}</option></select>
        </label>
      </div>`,
    submit: t("add"),
    onSubmit: async ({ form, close }) => {
      await api("POST", `/api/v1/teams/${encodeURIComponent(appState.teamId)}/members`, {
        username: form.username.value,
        role: form.role.value,
      });
      markSaved();
      close();
      await route();
    },
  });
}

async function pageTeam() {
  const teams = appState.me.teams || [];
  const current = teams.find((team) => team.id === appState.teamId);
  const organizations = appState.me.is_admin ? await api("GET", "/api/v1/organizations") : [];
  let members = [];
  if (appState.teamId) members = await api("GET", `/api/v1/teams/${encodeURIComponent(appState.teamId)}/members`);
  const memberRows = members.map((member) => {
    const name = appState.me.is_admin
      ? `<a href="#/users?edit=${esc(member.user_id)}">${esc(member.display_name)}</a>`
      : esc(member.display_name);
    const roleCell = member.source === "manual" && can("owner")
      ? `<select data-member-role="${esc(member.username)}" data-current="${esc(member.role)}">${roleOptions(member.role)}</select>`
      : esc(roleLabel(member.role));
    const sourceCell = member.source === "ldap"
      ? `${esc(sourceLabel(member.source))} <span class="small muted">${h("comes_from_the_directory")}</span>`
      : esc(sourceLabel(member.source));
    const remove = can("owner")
      ? `<button class="btn danger" type="button" data-member="${esc(member.user_id)}">${h("remove")}</button>`
      : "";
    return `<tr><td>${name} <span class="small muted">${esc(member.username)}</span></td><td>${roleCell}</td><td>${sourceCell}</td><td>${remove}</td></tr>`;
  });
  const actions = [
    appState.teamId && can("owner") ? addLink("#/team?new=member", "member") : "",
    appState.me.is_admin ? addLink("#/team?new=1", "team_2") : "",
    appState.me.is_admin ? addLink("#/users?new=1", "user_2") : "",
  ].join("");
  shell(t("team"), `
    ${current ? `<section class="card" style="margin-bottom:14px">
      <h2>${esc(current.name)}</h2>
      <p class="muted">${esc(current.description || "")}</p>
      <p class="small">${h("organization_name_2", { name: current.organization_name || "—" })}</p>
      ${current.role === "owner" || appState.me.is_admin ? `<div class="row"><a class="btn" href="#/team?edit=${esc(current.id)}">${h("edit")}</a></div>` : `<p class="small">${h("ldap_group_group", { group: current.ldap_group || "—" })}</p>`}
    </section>
    <section class="card">
      <h2>${h("members")}</h2>
      ${roleLegend()}
      ${table([t("name"), t("role"), t("source"), ""], memberRows)}
    </section>` : `<div class="card">${h("no_team_selected")}</div>`}`, actions);
  const request = editQuery();
  if (request.create === "member") {
    if (can("owner")) openMemberDialog();
  } else if (request.create !== null) {
    if (appState.me.is_admin) openTeamDialog(null, organizations, current && current.organization_id);
  } else if (request.edit) {
    const item = teams.find((team) => team.id === request.edit);
    if (!item) clearEditHash();
    else if (item.role === "owner" || appState.me.is_admin) openTeamDialog(item, organizations);
  }
  document.querySelectorAll("[data-member-role]").forEach((select) => {
    select.addEventListener("change", async () => {
      const next = select.value;
      try {
        await api("POST", `/api/v1/teams/${encodeURIComponent(appState.teamId)}/members`, {
          username: select.dataset.memberRole,
          role: next,
        });
        select.dataset.current = next;
      } catch (error) {
        select.value = select.dataset.current;
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-member]").forEach((button) => {
    button.addEventListener("click", async () => {
      try {
        await api("DELETE", `/api/v1/teams/${encodeURIComponent(appState.teamId)}/members/${encodeURIComponent(button.dataset.member)}`);
        route();
      } catch (error) { handle(error); }
    });
  });
}

registerRedirect("#/team/members/new", "#/team?new=member");
registerRedirect("#/team/new", "#/team?new=1");
registerRedirect("#/team/:id", ({ id }) => `#/team?edit=${encodeURIComponent(id)}`);
registerRoute("#/team", pageTeam);
