import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { confirmDialog } from "../core/dialog.js";
import { deny, handle, markSaved, roleLabel, route, shell } from "../core/app.js";
import { addLink, esc, h, roleLegend, roleOptions, table } from "../core/dom.js";
import { editQuery, openEditDialog, rememberEditHash } from "../core/edit.js";
import { authSourceLabel, emailLine, sourceLabel, when } from "../core/format.js";
import { t } from "../core/i18n.js";
import { registerRedirect, registerRoute } from "../core/router.js";

export function userMatches(item, needle) {
  if (!needle) return true;
  return `${item.display_name} ${item.username} ${item.email || ""} ${item.auth_source}`.toLowerCase().includes(needle);
}

export function userCreatePayload(form) {
  return {
    username: form.username.value,
    display_name: form.display_name.value,
    password: form.password.value,
    is_admin: form.is_admin.checked,
  };
}

export function userPayload(form) {
  const payload = {
    display_name: form.display_name.value,
    is_admin: form.is_admin.checked,
    disabled: form.account_disabled.checked,
  };
  if (form.password && form.password.value) payload.password = form.password.value;
  return payload;
}

function openUserCreateDialog() {
  openEditDialog({
    id: "user-dialog",
    title: t("new_user"),
    body: `
      <label class="field required"><span>${h("username")}</span><input name="username" required autocomplete="off"></label>
      <p class="small muted">${h("username_2_64_characters_a_z_0_9")}</p>
      <label class="field required"><span>${h("display_name")}</span><input name="display_name" required></label>
      <label class="field required"><span>${h("password")}</span><input name="password" type="password" required minlength="8" autocomplete="new-password"></label>
      <label class="row"><input name="is_admin" type="checkbox"> ${h("administrator")}</label>`,
    submit: t("create"),
    onSubmit: async ({ form, close }) => {
      await api("POST", "/api/v1/users", userCreatePayload(form));
      markSaved();
      close();
      await route();
    },
  });
}

function openUserDialog(item, teams, initialTab = "") {
  const self = item.id === appState.me.id;
  const memberIds = new Set((item.memberships || []).map((row) => row.team_id));
  const openTeams = teams.filter((team) => !memberIds.has(team.id));
  const rows = (item.memberships || []).map((row) => {
    const org = row.organization_name ? `${row.organization_name} · ` : "";
    const roleCell = row.source === "manual"
      ? `<select data-role="${esc(row.team_id)}" data-current="${esc(row.role)}">${roleOptions(row.role)}</select>`
      : `${esc(roleLabel(row.role))} <span class="small muted">${h("comes_from_the_directory")}</span>`;
    const remove = row.source === "manual"
      ? `<button class="btn danger" type="button" data-leave="${esc(row.team_id)}">${h("remove")}</button>`
      : "";
    return `<tr><td>${esc(org + row.team_name)}</td><td>${roleCell}</td><td>${esc(sourceLabel(row.source))}</td><td>${remove}</td></tr>`;
  });
  const passwordField = item.auth_source === "local"
    ? `<label class="field"><span>${h("new_password")}</span><input name="password" type="password" minlength="8" autocomplete="new-password" placeholder="${h("leave_empty_to_keep_the_password")}"></label>`
    : `<p class="small muted">${h("the_password_comes_from_ldap_or_active_directory")}</p>`;
  const teamChoices = openTeams.map((team) => {
    const label = team.organization_name ? `${team.organization_name} · ${team.name}` : team.name;
    return `<option value="${esc(team.id)}">${esc(label)}</option>`;
  }).join("");
  const account = `
    <p class="small muted">${h("username")}: ${esc(item.username)} · ${esc(authSourceLabel(item.auth_source))}</p>
    <p class="small">${h("email")}: ${emailLine(item.email)}</p>
    <label class="field required"><span>${h("display_name")}</span><input name="display_name" value="${esc(item.display_name)}" required></label>
    <label class="row"><input name="is_admin" type="checkbox"${item.is_admin ? " checked" : ""}> ${h("administrator")}</label>
    <label class="row"><input name="account_disabled" type="checkbox"${item.disabled ? " checked" : ""}${self ? " disabled" : ""}> ${h("disable_account")}</label>
    ${passwordField}
    <div class="form-section">
      <h2>${h("permissions")}</h2>
      ${roleLegend()}
    </div>`;
  const memberships = `
    ${table([t("team"), t("role"), t("source"), ""], rows)}
    ${openTeams.length ? `<div class="form-section">
      <h2>${h("add_to_a_team")}</h2>
      <label class="field"><span>${h("team")}</span><select name="grant_team">${teamChoices}</select></label>
      <div class="field-row">
        <label class="field"><span>${h("role")}</span><select class="fit" name="grant_role">${roleOptions("operator")}</select></label>
      </div>
      <button class="btn pill primary" type="button" data-user-grant>${h("add")}</button>
    </div>` : `<p class="muted">${h("the_user_is_in_every_team")}</p>`}`;
  const dialog = openEditDialog({
    id: "user-dialog",
    title: item.display_name,
    tabs: [
      { id: "account", label: t("account"), body: account },
      { id: "memberships", label: t("memberships"), body: memberships },
    ],
    initialTab,
    submit: t("save"),
    danger: self ? "" : `<button class="btn danger" type="button" data-user-delete>${h("delete")}</button>`,
    onSubmit: async ({ form, close }) => {
      await api("PUT", `/api/v1/users/${encodeURIComponent(item.id)}`, userPayload(form));
      if (self) appState.setMe(await api("GET", "/api/v1/me"));
      markSaved();
      close();
      await route();
    },
    onTab: (tab) => rememberEditHash({ edit: item.id, tab }, false),
  });
  const remove = dialog.overlay.querySelector("[data-user-delete]");
  if (remove) {
    remove.addEventListener("click", async () => {
      if (!(await confirmDialog({ title: t("delete"), body: t("delete_this_account", { name: item.display_name }), confirmLabel: t("delete"), danger: true }))) return;
      try {
        await api("DELETE", `/api/v1/users/${encodeURIComponent(item.id)}`);
        markSaved();
        dialog.close();
        await route();
      } catch (error) { handle(error); }
    });
  }
  dialog.overlay.querySelectorAll("[data-role]").forEach((select) => {
    select.addEventListener("input", (event) => event.stopPropagation());
    select.addEventListener("change", async (event) => {
      event.stopPropagation();
      const next = select.value;
      if (dialog.dirty() && !(await confirmDialog({ title: t("discard_unsaved_changes"), confirmLabel: t("ok") }))) {
        select.value = select.dataset.current;
        return;
      }
      try {
        await api("POST", `/api/v1/teams/${encodeURIComponent(select.dataset.role)}/members`, {
          username: item.username,
          role: next,
        });
        select.dataset.current = next;
        dialog.clean();
        await route();
      } catch (error) {
        select.value = select.dataset.current;
        handle(error);
      }
    });
  });
  dialog.overlay.querySelectorAll("[data-leave]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (dialog.dirty() && !(await confirmDialog({ title: t("discard_unsaved_changes"), confirmLabel: t("ok") }))) return;
      try {
        await api("DELETE", `/api/v1/teams/${encodeURIComponent(button.dataset.leave)}/members/${encodeURIComponent(item.id)}`);
        dialog.clean();
        await route();
      } catch (error) { handle(error); }
    });
  });
  const grant = dialog.overlay.querySelector("[data-user-grant]");
  if (grant) {
    dialog.overlay.querySelectorAll("[name=grant_team], [name=grant_role]").forEach((field) => {
      field.addEventListener("input", (event) => event.stopPropagation());
      field.addEventListener("change", (event) => event.stopPropagation());
    });
    grant.addEventListener("click", async () => {
      const team = dialog.overlay.querySelector("[name=grant_team]");
      const role = dialog.overlay.querySelector("[name=grant_role]");
      if (dialog.dirty() && !(await confirmDialog({ title: t("discard_unsaved_changes"), confirmLabel: t("ok") }))) return;
      try {
        await api("POST", `/api/v1/teams/${encodeURIComponent(team.value)}/members`, {
          username: item.username,
          role: role.value,
        });
        markSaved();
        dialog.clean();
        await route();
      } catch (error) { handle(error); }
    });
  }
}

async function pageUsers() {
  if (!appState.me.is_admin) {
    deny("users", "admin");
    return;
  }
  const items = await api("GET", "/api/v1/users");
  shell(t("users"), `
    <section class="card">
      <input class="search" id="user-search" placeholder="${h("search_users")}" autocomplete="off">
      <div id="user-table"></div>
    </section>`,
    addLink("#/users?new=1", "user_2"));
  const draw = () => {
    const needle = document.querySelector("#user-search").value.trim().toLowerCase();
    const shown = items.filter((item) => userMatches(item, needle));
    const rows = shown.map((item) => {
      const flags = [
        item.is_admin ? `<span class="pill">${h("administrator")}</span>` : "",
        item.disabled ? `<span class="pill failed">${h("disabled")}</span>` : "",
      ].filter(Boolean).join(" ");
      const created = when(item.created_at);
      return `<tr>
        <td><a href="#/users?edit=${esc(item.id)}">${esc(item.display_name)}</a> ${flags}</td>
        <td>${esc(item.username)}</td>
        <td>${esc(authSourceLabel(item.auth_source))}</td>
        <td>${esc(item.team_count)}</td>
        <td class="small muted">${esc(created)}</td>
      </tr>`;
    });
    document.querySelector("#user-table").innerHTML = table(
      [t("name"), t("username"), t("source"), t("teams"), t("created")],
      rows
    );
  };
  document.querySelector("#user-search").addEventListener("input", draw);
  draw();
  const request = editQuery();
  if (request.create !== null) {
    openUserCreateDialog();
    return;
  }
  if (!request.edit) return;
  try {
    const [item, teams] = await Promise.all([
      api("GET", `/api/v1/users/${encodeURIComponent(request.edit)}`),
      api("GET", "/api/v1/teams"),
    ]);
    openUserDialog(item, teams, request.tab);
  } catch (error) {
    handle(error);
  }
}

registerRedirect("#/users/new", "#/users?new=1");
registerRedirect("#/users/:id", ({ id }) => `#/users?edit=${encodeURIComponent(id)}`);
registerRoute("#/users", pageUsers);
