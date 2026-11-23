import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { confirmDialog } from "../core/dialog.js";
import { deny, handle, markSaved, roleLabel, route, shell } from "../core/app.js";
import { addLink, esc, h, roleOptions, table } from "../core/dom.js";
import { editQuery, openEditDialog, rememberEditHash } from "../core/edit.js";
import { authSourceLabel, sourceLabel, tokenExpiryCell, tokenScopeCell, when } from "../core/format.js";
import { t } from "../core/i18n.js";
import { registerRedirect, registerRoute } from "../core/router.js";

let revealToken = null;

export function serviceAccountMatches(item, needle) {
  if (!needle) return true;
  return `${item.display_name} ${item.username} ${item.created_by_username || ""}`.toLowerCase().includes(needle);
}

export function serviceAccountCreatePayload(form) {
  return {
    username: form.username.value,
    display_name: form.display_name.value,
    teams: [{ team_id: form.team_id.value, role: form.role.value }],
  };
}

export function serviceAccountTokenPayload(form) {
  const days = form.expires_in_days.value;
  return {
    name: form.name.value,
    scope: form.scope.value,
    expires_in_days: days === "" ? null : Number(days),
  };
}

export function serviceAccountCreatorCell(item) {
  return item.created_by_username
    ? esc(item.created_by_username)
    : `<span class="muted">${h("service_account_creator_missing")}</span>`;
}

export function manageableTeams() {
  return (appState.me.teams || []).filter((team) => appState.me.is_admin || team.role === "owner");
}

function openCreateDialog() {
  const teams = manageableTeams();
  if (!teams.length) return;
  const teamChoices = teams.map((team) => {
    const label = team.organization_name ? `${team.organization_name} · ${team.name}` : team.name;
    return `<option value="${esc(team.id)}">${esc(label)}</option>`;
  }).join("");
  openEditDialog({
    id: "service-account-dialog",
    title: t("new_service_account"),
    body: `
      <p class="small muted">${h("service_account_hint")}</p>
      <label class="field required"><span>${h("username")}</span><input name="username" required autocomplete="off"></label>
      <p class="small muted">${h("username_2_64_characters_a_z_0_9")}</p>
      <label class="field required"><span>${h("display_name")}</span><input name="display_name" required></label>
      <div class="form-section">
        <h2>${h("teams")}</h2>
        <p class="small muted">${h("service_account_teams_hint")}</p>
        <label class="field"><span>${h("team")}</span><select name="team_id">${teamChoices}</select></label>
        <label class="field"><span>${h("role")}</span><select class="fit" name="role">${roleOptions("operator")}</select></label>
      </div>`,
    submit: t("create"),
    onSubmit: async ({ form, close }) => {
      const created = await api("POST", "/api/v1/service-accounts", serviceAccountCreatePayload(form));
      markSaved();
      close();
      rememberEditHash({ edit: created.id, tab: "tokens" });
      await route();
    },
  });
}

function tokenRows(tokens) {
  return tokens.map((item) => {
    const used = item.last_used_at ? esc(when(item.last_used_at)) : h("never");
    return `<tr>
      <td>${esc(item.name)}</td>
      <td class="mono">${esc(item.prefix)}</td>
      <td>${tokenScopeCell(item)}</td>
      <td class="small muted">${esc(when(item.created_at))}</td>
      <td>${tokenExpiryCell(item)}</td>
      <td class="small muted">${used}</td>
      <td class="actions"><button class="btn compact danger" type="button" data-revoke-token="${esc(item.id)}" data-name="${esc(item.name)}">${h("revoke")}</button></td>
    </tr>`;
  });
}

function tokenForm(secret) {
  const banner = secret
    ? `<div class="banner good">
        <p>${h("api_tokens_hint")}</p>
        <p><code>${esc(secret)}</code> <button class="btn compact" type="button" data-copy="${esc(secret)}">${h("copy")}</button></p>
      </div>`
    : "";
  return `${banner}
    <form class="form" id="service-account-token-form">
      <label class="field required"><span>${h("name")}</span><input name="name" required></label>
      <label class="field"><span>${h("token_scope")}</span>
        <select class="fit" name="scope">
          <option value="write" selected>${h("token_scope_write")}</option>
          <option value="read">${h("token_scope_read")}</option>
        </select>
      </label>
      <label class="field"><span>${h("token_lifetime")}</span>
        <select class="fit" name="expires_in_days">
          <option value="30">${h("token_lifetime_30_days")}</option>
          <option value="90" selected>${h("token_lifetime_90_days")}</option>
          <option value="365">${h("token_lifetime_365_days")}</option>
          <option value="">${h("token_lifetime_never")}</option>
        </select>
      </label>
      <div><button class="btn pill primary" type="submit">${h("create_token")}</button></div>
    </form>`;
}

function openDetailDialog(item, initialTab = "") {
  const revealed = revealToken;
  revealToken = null;
  const memberships = (item.memberships || []).map((row) => {
    const team = row.organization_name ? `${row.organization_name} · ${row.team_name}` : row.team_name;
    const mine = (appState.me.teams || []).find((entry) => entry.id === row.team_id);
    const canRemove = appState.me.is_admin || (mine && mine.role === "owner");
    const remove = canRemove
      ? `<button class="btn danger" type="button" data-leave-team="${esc(row.team_id)}">${h("remove")}</button>`
      : "";
    return `<tr><td>${esc(team)}</td><td>${esc(roleLabel(row.role))}</td><td>${esc(sourceLabel(row.source))}</td><td>${remove}</td></tr>`;
  });
  const flags = item.disabled ? `<span class="pill failed">${h("disabled")}</span>` : "";
  const account = `
    <p class="small muted">${h("username")}: ${esc(item.username)} · ${esc(authSourceLabel("service"))} ${flags}</p>
    <p>${h("display_name")}: ${esc(item.display_name)}</p>
    <p class="small muted">${h("service_account_created_by", { name: item.created_by_username || t("service_account_creator_missing") })}</p>
    <p class="small muted">${h("created")}: ${esc(when(item.created_at))}</p>
    <div class="form-section">
      <h2>${h("teams")}</h2>
      ${table([t("team"), t("role"), t("source"), ""], memberships)}
    </div>`;
  const tokens = `
    <div class="form-section">
      <h2>${h("api_tokens")}</h2>
      <p class="small muted">${h("service_account_tokens_hint")}</p>
      ${tokenForm(revealed)}
      ${table([t("name"), t("prefix"), t("token_scope"), t("created"), t("expires"), t("last_used"), t("actions")], tokenRows(item.tokens || []))}
    </div>`;
  const danger = appState.me.is_admin
    ? `${item.disabled
      ? `<button class="btn" type="button" data-enable-account>${h("enable_service_account")}</button>`
      : `<button class="btn danger" type="button" data-disable-account>${h("disable_service_account")}</button>`}
      <button class="btn danger" type="button" data-delete-account>${h("delete")}</button>`
    : "";
  const dialog = openEditDialog({
    id: "service-account-detail",
    title: item.display_name,
    tabs: [
      { id: "account", label: t("account"), body: account },
      { id: "tokens", label: t("api_tokens"), body: tokens },
    ],
    initialTab,
    hint: "",
    form: false,
    danger,
    onTab: (tab) => rememberEditHash({ edit: item.id, tab }, false),
  });
  const form = dialog.overlay.querySelector("#service-account-token-form");
  if (form) {
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        const created = await api("POST", `/api/v1/service-accounts/${encodeURIComponent(item.id)}/tokens`, serviceAccountTokenPayload(form));
        revealToken = created.secret;
        rememberEditHash({ edit: item.id, tab: "tokens" }, false);
        await route();
      } catch (error) { handle(error); }
    });
  }
  dialog.overlay.querySelectorAll("[data-copy]").forEach((button) => {
    button.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(button.dataset.copy);
        button.textContent = t("copied");
      } catch {}
    });
  });
  dialog.overlay.querySelectorAll("[data-revoke-token]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (!(await confirmDialog({ title: t("revoke"), body: t("delete_this_token", { name: button.dataset.name }), confirmLabel: t("revoke"), danger: true }))) return;
      try {
        await api("DELETE", `/api/v1/service-accounts/${encodeURIComponent(item.id)}/tokens/${encodeURIComponent(button.dataset.revokeToken)}`);
        await route();
      } catch (error) { handle(error); }
    });
  });
  dialog.overlay.querySelectorAll("[data-leave-team]").forEach((button) => {
    button.addEventListener("click", async () => {
      try {
        await api("DELETE", `/api/v1/teams/${encodeURIComponent(button.dataset.leaveTeam)}/members/${encodeURIComponent(item.id)}`);
        markSaved();
        await route();
      } catch (error) { handle(error); }
    });
  });
  const setDisabled = async (disabled) => {
    try {
      await api("PUT", `/api/v1/users/${encodeURIComponent(item.id)}`, {
        display_name: item.display_name,
        is_admin: false,
        disabled,
      });
      markSaved();
      await route();
    } catch (error) { handle(error); }
  };
  const disable = dialog.overlay.querySelector("[data-disable-account]");
  if (disable) disable.addEventListener("click", () => setDisabled(true));
  const enable = dialog.overlay.querySelector("[data-enable-account]");
  if (enable) enable.addEventListener("click", () => setDisabled(false));
  const remove = dialog.overlay.querySelector("[data-delete-account]");
  if (remove) {
    remove.addEventListener("click", async () => {
      if (!(await confirmDialog({ title: t("delete"), body: t("delete_this_service_account", { name: item.display_name }), confirmLabel: t("delete"), danger: true }))) return;
      try {
        await api("DELETE", `/api/v1/service-accounts/${encodeURIComponent(item.id)}`);
        markSaved();
        dialog.close();
        await route();
      } catch (error) { handle(error); }
    });
  }
}

async function pageServiceAccounts() {
  if (!appState.me.is_admin && !manageableTeams().length) {
    deny("service_accounts", "owner");
    return;
  }
  const items = await api("GET", "/api/v1/service-accounts");
  shell(t("service_accounts"), `
    <section class="card">
      <input class="search" id="service-account-search" placeholder="${h("search_service_accounts")}" aria-label="${h("search_service_accounts")}" autocomplete="off">
      <div id="service-account-table"></div>
    </section>`,
    manageableTeams().length ? addLink("#/service-accounts?new=1", "new_service_account") : "");
  const draw = () => {
    const needle = document.querySelector("#service-account-search").value.trim().toLowerCase();
    const rows = items.filter((item) => serviceAccountMatches(item, needle)).map((item) => {
      const flags = item.disabled ? ` <span class="pill failed">${h("disabled")}</span>` : "";
      return `<tr>
        <td><a href="#/service-accounts?edit=${esc(item.id)}">${esc(item.display_name)}</a>${flags}</td>
        <td>${esc(item.username)}</td>
        <td>${esc(item.team_count)}</td>
        <td class="small muted">${serviceAccountCreatorCell(item)}</td>
        <td class="small muted">${esc(when(item.created_at))}</td>
      </tr>`;
    });
    document.querySelector("#service-account-table").innerHTML = table(
      [t("name"), t("username"), t("teams"), t("creator"), t("created")],
      rows
    );
  };
  document.querySelector("#service-account-search").addEventListener("input", draw);
  draw();
  const request = editQuery();
  if (request.create !== null) {
    openCreateDialog();
    return;
  }
  if (!request.edit) return;
  try {
    const [item, tokens] = await Promise.all([
      api("GET", `/api/v1/service-accounts/${encodeURIComponent(request.edit)}`),
      api("GET", `/api/v1/service-accounts/${encodeURIComponent(request.edit)}/tokens`),
    ]);
    item.tokens = tokens;
    openDetailDialog(item, request.tab);
  } catch (error) {
    handle(error);
  }
}

registerRedirect("#/service-accounts/new", "#/service-accounts?new=1");
registerRedirect("#/service-accounts/:id", ({ id }) => `#/service-accounts?edit=${encodeURIComponent(id)}`);
registerRoute("#/service-accounts", pageServiceAccounts);
