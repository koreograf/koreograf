import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { confirmDialog } from "../core/dialog.js";
import { handle, markSaved, roleLabel, route, shell } from "../core/app.js";
import { esc, h, table } from "../core/dom.js";
import { authSourceLabel, emailLine, sourceLabel, when } from "../core/format.js";
import { t } from "../core/i18n.js";
import { registerRoute } from "../core/router.js";

let revealToken = null;

export function accountPayload(form) {
  const payload = {
    display_name: form.display_name.value,
    email: form.email.value,
  };
  if (form.password.value) payload.password = form.password.value;
  return payload;
}

async function pageAccount() {
  appState.setMe(await api("GET", "/api/v1/me"));
  const tokens = await api("GET", "/api/v1/me/tokens");
  const revealed = revealToken;
  revealToken = null;
  const rows = (appState.me.memberships || []).map((row) => {
    const org = row.organization_name || "—";
    return `<tr><td>${esc(org)}</td><td>${esc(row.team_name)}</td><td>${esc(roleLabel(row.role))}</td><td>${esc(sourceLabel(row.source))}</td></tr>`;
  });
  const tokenRows = tokens.map((item) => {
    const used = item.last_used_at ? esc(when(item.last_used_at)) : h("never");
    return `<tr>
      <td>${esc(item.name)}</td>
      <td class="mono">${esc(item.prefix)}</td>
      <td class="small muted">${esc(when(item.created_at))}</td>
      <td class="small muted">${used}</td>
      <td class="actions"><button class="btn compact danger" type="button" data-revoke="${esc(item.id)}" data-name="${esc(item.name)}">${h("revoke")}</button></td>
    </tr>`;
  });
  const secret = revealed ? `
      <div class="banner good">
        <p>${h("api_tokens_hint")}</p>
        <p><code>${esc(revealed)}</code> <button class="btn compact" type="button" data-copy="${esc(revealed)}">${h("copy")}</button></p>
      </div>` : "";
  const local = appState.me.auth_source === "local";
  const identity = local ? `
    <form class="card form" id="account-form">
      <h2>${h("account")}</h2>
      <p class="small muted">${h("username")}: ${esc(appState.me.username)} · ${esc(authSourceLabel(appState.me.auth_source))}${appState.me.is_admin ? ` · ${h("administrator")}` : ""}</p>
      <label class="field"><span>${h("display_name")}</span><input name="display_name" value="${esc(appState.me.display_name)}" required></label>
      <label class="field"><span>${h("email")}</span><input name="email" type="email" value="${esc(appState.me.email || "")}" autocomplete="email"></label>
      <label class="field"><span>${h("new_password")}</span><input name="password" type="password" minlength="8" autocomplete="new-password" placeholder="${h("leave_empty_to_keep_the_password")}"></label>
      <button class="btn pill primary" type="submit">${h("save")}</button>
    </form>` : `
    <section class="card">
      <h2>${h("account")}</h2>
      <p class="small muted">${h("username")}: ${esc(appState.me.username)} · ${esc(authSourceLabel(appState.me.auth_source))}${appState.me.is_admin ? ` · ${h("administrator")}` : ""}</p>
      <p>${h("display_name")}: ${esc(appState.me.display_name)}</p>
      <p>${h("email")}: ${emailLine(appState.me.email)}</p>
      <p class="muted">${h("name_and_email_come_from_the_directory")}</p>
    </section>`;
  shell(t("my_account"), `
    <div class="stack">
      ${identity}
      <section class="card">
        <h2>${h("memberships")}</h2>
        ${table([t("organization"), t("team"), t("role"), t("source")], rows)}
      </section>
      <section class="card">
        <h2>${h("api_tokens")}</h2>
        ${secret}
        <form class="form" id="token-form">
          <label class="field required">
            <span>${h("name")}</span>
            <input name="name" required>
          </label>
          <div><button class="btn pill primary" type="submit">${h("create_token")}</button></div>
        </form>
        ${table([t("name"), t("prefix"), t("created"), t("last_used"), t("actions")], tokenRows)}
      </section>
    </div>`);
  const form = document.querySelector("#account-form");
  if (form) {
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        appState.setMe(await api("PUT", "/api/v1/me", accountPayload(form)));
        markSaved();
        route();
      } catch (error) { handle(error); }
    });
  }
  const tokenForm = document.querySelector("#token-form");
  if (tokenForm) {
    tokenForm.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        const created = await api("POST", "/api/v1/me/tokens", { name: tokenForm.name.value });
        revealToken = created.secret;
        await route();
      } catch (error) { handle(error); }
    });
  }
  document.querySelectorAll("[data-copy]").forEach((button) => {
    button.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(button.dataset.copy);
        button.textContent = t("copied");
      } catch {}
    });
  });
  document.querySelectorAll("[data-revoke]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (!(await confirmDialog({ title: t("revoke"), body: t("delete_this_token", { name: button.dataset.name }), confirmLabel: t("revoke"), danger: true }))) return;
      try {
        await api("DELETE", `/api/v1/me/tokens/${encodeURIComponent(button.dataset.revoke)}`);
        await route();
      } catch (error) { handle(error); }
    });
  });
}

registerRoute("#/account", pageAccount);
