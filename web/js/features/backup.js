import * as appState from "../core/state.js";
import { deny, handle, renderLogin, shell } from "../core/app.js";
import { h } from "../core/dom.js";
import { t } from "../core/i18n.js";
import { registerRoute } from "../core/router.js";

export function backupDownloadUrl(includeTofuState) {
  return includeTofuState ? "/api/v1/admin/backup?tofu_state=true" : "/api/v1/admin/backup";
}

async function pageBackup() {
  if (!appState.me.is_admin) {
    deny("backup", "admin");
    return;
  }
  shell(t("backup"), `<div class="stack">
    <div class="card">
      <p>${h("koreograf_stores_data_in_sqlite_the_backup_contains")}</p>
      <p class="muted">${h("move_database_to_postgresql_hint")}</p>
      <label class="check-row"><input id="backup-tofu-state" type="checkbox"><span>${h("include_opentofu_state")}</span></label>
      <p class="small muted">${h("opentofu_state_can_contain_secrets")}</p>
      <div><button class="btn pill primary" id="backup-download" type="button">${h("download_backup")}</button></div>
    </div>
    <form class="card form danger-zone" id="backup-restore">
      <h2>${h("restore_backup")}</h2>
      <p>${h("this_replaces_organizations_teams_users_projects_inventories_credentials")}</p>
      <label class="field"><span>${h("file")}</span><input name="file" type="file" accept=".zip,application/zip" required></label>
      <label class="row"><input name="confirm" type="checkbox" required> ${h("the_current_installation_will_be_replaced")}</label>
      <button class="btn pill danger" type="submit">${h("restore_backup")}</button>
    </form>
  </div>`);
  document.querySelector("#backup-download").addEventListener("click", async () => {
    const button = document.querySelector("#backup-download");
    button.disabled = true;
    try {
      const headers = {};
      if (appState.teamId) headers["X-Team-Id"] = appState.teamId;
      const includeTofuState = document.querySelector("#backup-tofu-state").checked;
      const response = await fetch(backupDownloadUrl(includeTofuState), { headers, credentials: "same-origin" });
      const blob = await response.blob();
      if (!response.ok) {
        const text = await blob.text();
        let message = "request.failed";
        let vars = null;
        try {
          const data = JSON.parse(text);
          message = data.error || message;
          vars = data.vars || null;
        } catch { /* keep */ }
        const error = new Error(message);
        error.vars = vars;
        throw error;
      }
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = "koreograf-backup.zip";
      link.click();
      URL.revokeObjectURL(url);
    } catch (error) {
      handle(error);
    } finally {
      button.disabled = false;
    }
  });
  document.querySelector("#backup-restore").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const file = form.file.files[0];
    if (!file || !form.confirm.checked) return;
    const button = form.querySelector("button[type=submit]");
    button.disabled = true;
    try {
      const headers = { "X-Restore-Confirm": "ERSETZEN" };
      if (appState.teamId) headers["X-Team-Id"] = appState.teamId;
      const response = await fetch("/api/v1/admin/backup", {
        method: "POST",
        headers,
        body: file,
        credentials: "same-origin",
      });
      const data = await response.json().catch(() => ({}));
      if (!response.ok) {
        const error = new Error(data.error || "request.failed");
        error.vars = data.vars || null;
        throw error;
      }
      appState.setMe(null);
      renderLogin("backup_restored_the_previous_installation_is_restore_previous", "good");
    } catch (error) {
      handle(error);
      button.disabled = false;
    }
  });
}

registerRoute("#/backup", pageBackup);
