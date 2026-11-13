import { api } from "../core/api.js";
import { confirmDialog } from "../core/dialog.js";
import { handle, loadMaintenance } from "../core/app.js";
import { esc, h, table } from "../core/dom.js";
import { t } from "../core/i18n.js";

let checkId = null;

export function resetDatabaseCheck() {
  checkId = null;
}

export function databaseTargetBody(form) {
  return {
    host: form.db_host.value.trim(),
    port: Number(form.db_port.value || 5432),
    database: form.db_database.value.trim(),
    user: form.db_user.value.trim(),
    password: form.db_password.value,
    sslmode: form.db_sslmode.value,
    ca_file: form.db_ca_file.value.trim() || null,
  };
}

export function databaseChecksHtml(result) {
  const checks = (result && result.checks) || [];
  const label = (id) => {
    if (id === "source_schema") return "check_source_schema";
    if (id === "empty") return "check_empty_target";
    return `check_${id}`;
  };
  const rows = checks.map((check) => `<tr><td>${h(label(check.id))}</td><td><span class="pill ${check.ok ? "good" : "bad"}">${check.ok ? h("check_passed") : h("check_failed")}</span></td><td>${h(check.key)}</td></tr>`);
  const summary = result && result.all_passed
    ? `<p class="banner good">${h("all_checks_passed")}</p>`
    : `<p class="banner bad">${h("fix_the_failed_checks_and_run_them_again")}</p>`;
  return `${summary}${table([t("check"), t("result"), t("reason")], rows)}`;
}

export function databaseProgressHtml(status) {
  if (!status || (!status.running && !status.done && !status.failed)) return "";
  const rows = (status.tables || []).map((item) => `<tr><td><code>${esc(item.table)}</code></td><td>${esc(String(item.rows))}</td><td>${item.done ? `<span class="pill good">${h("check_passed")}</span>` : `<span class="pill warn">${h("pending")}</span>`}</td></tr>`);
  const heading = status.failed
    ? `<p class="banner bad">${h("database_migration_failed")}</p>`
    : status.done
      ? `<p class="banner good">${h("database_migration_done")}</p>`
      : `<p class="banner warn">${h("database_migration_running")}</p>`;
  const total = status.done ? `<p class="small muted">${h("database_migration_verified", { rows: String(status.rows || 0) })}</p>` : "";
  return `${heading}${total}${table([t("table"), t("rows"), t("result")], rows)}`;
}

export function databaseSwitchHtml(status) {
  if (!status || !status.done || !status.url) return "";
  const url = String(status.url);
  return `<div class="form-section">
    <h3>${h("step_4_switch")}</h3>
    <p>${h("koreograf_does_not_change_its_own_configuration")}</p>
    <h4>${h("docker_env_file")}</h4>
    <pre class="tool">KOREOGRAF_DATABASE_URL='${esc(url)}'
KOREOGRAF_DATABASE_PASSWORD_FILE=/run/secrets/koreograf_database_password</pre>
    <h4>${h("package_env_file")}</h4>
    <pre class="tool">KOREOGRAF_DATABASE_URL='${esc(url)}'</pre>
    <p>${h("restart_koreograf_after_the_change")}</p>
    <p class="small muted">${h("rollback_switches_the_url_back")}</p>
  </div>`;
}

export function databaseAuditHtml(status) {
  if (!status || !status.audit) return "";
  return `<p class="small muted">${h("database_migration_audit", { user: status.audit.user, time: status.audit.at, host: status.audit.host })}</p>`;
}

export function databasePanelHtml(active) {
  return `<section class="form-section settings-panel" id="settings-panel-database" role="tabpanel" aria-labelledby="settings-tab-database"${active ? "" : " hidden"}>
      <h2>${h("move_database_to_postgresql")}</h2>
      <p class="small muted">${h("move_database_to_postgresql_hint")}</p>
      <div id="database-status"></div>
      <h3>${h("step_1_target")}</h3>
      <div class="field-row">
        <label class="field required"><span>${h("database_host")}</span><input name="db_host" autocomplete="off" spellcheck="false" placeholder="db.example.com"></label>
        <label class="field"><span>${h("database_port")}</span><input class="tiny" name="db_port" type="number" min="1" max="65535" value="5432"></label>
      </div>
      <div class="field-grid">
        <label class="field required"><span>${h("database_name")}</span><input name="db_database" autocomplete="off" spellcheck="false" placeholder="koreograf"></label>
        <label class="field required"><span>${h("database_user")}</span><input name="db_user" autocomplete="off" spellcheck="false" placeholder="koreograf"></label>
      </div>
      <label class="field required"><span>${h("database_password")}</span><input name="db_password" type="password" autocomplete="new-password"></label>
      <p class="small muted">${h("migration_password_not_stored")}</p>
      <div class="field-row">
        <label class="field"><span>${h("tls_mode")}</span>
          <select class="fit" name="db_sslmode">
            <option value="require">require</option>
            <option value="verify-ca">verify-ca</option>
            <option value="verify-full">verify-full</option>
            <option value="prefer">prefer</option>
            <option value="disable">disable</option>
          </select>
        </label>
        <label class="field"><span>${h("ca_file_optional")}</span><input name="db_ca_file" autocomplete="off" spellcheck="false" placeholder="/etc/koreograf/postgres-ca.pem"></label>
      </div>
      <h3>${h("step_2_checks")}</h3>
      <div class="choice-row">
        <button class="btn pill" id="database-check" type="button">${h("run_checks")}</button>
      </div>
      <div id="database-checks"></div>
      <h3>${h("step_3_migration")}</h3>
      <p class="small muted">${h("migration_starts_maintenance")}</p>
      <div class="choice-row">
        <button class="btn pill primary" id="database-start" type="button" disabled>${h("start_migration")}</button>
      </div>
      <div id="database-progress"></div>
      <div id="database-switch"></div>
      <div id="database-audit"></div>
    </section>`;
}

export function bindDatabaseMigration(form) {
  const databaseCheck = document.querySelector("#database-check");
  const databaseStart = document.querySelector("#database-start");
  const renderDatabaseStatus = (data) => {
    if (!data) return;
    const status = data.migration || {};
    const supported = data.supported !== false;
    document.querySelector("#database-status").innerHTML = supported
      ? ""
      : `<p class="banner warn">${h("database_already_on_postgresql")}</p>`;
    document.querySelector("#database-progress").innerHTML = databaseProgressHtml(status);
    document.querySelector("#database-switch").innerHTML = databaseSwitchHtml(status);
    document.querySelector("#database-audit").innerHTML = databaseAuditHtml(status);
    databaseCheck.disabled = !supported || Boolean(status.running) || Boolean(status.done);
    databaseStart.disabled = !supported || !checkId || Boolean(status.running) || Boolean(status.done);
    if (Boolean(status.running) && document.querySelector("#database-progress")) {
      window.setTimeout(() => refreshDatabaseStatus(renderDatabaseStatus), 1000);
    }
  };
  const refreshDatabaseStatus = async (render) => {
    if (!document.querySelector("#database-progress")) return null;
    try {
      const data = await api("GET", "/api/v1/admin/database/status");
      render(data);
      return data;
    } catch (error) {
      if (error.status !== 401) handle(error);
      return null;
    }
  };
  databaseCheck.addEventListener("click", async () => {
    databaseCheck.disabled = true;
    checkId = null;
    databaseStart.disabled = true;
    try {
      const result = await api("POST", "/api/v1/admin/database/check", databaseTargetBody(form));
      checkId = result && result.id ? String(result.id) : null;
      document.querySelector("#database-checks").innerHTML = databaseChecksHtml(result);
      databaseStart.disabled = !checkId;
    } catch (error) {
      handle(error);
    } finally {
      databaseCheck.disabled = false;
    }
  });
  databaseStart.addEventListener("click", async () => {
    if (!checkId) return;
    if (!(await confirmDialog({ title: t("start_migration"), body: t("migration_starts_maintenance"), confirmLabel: t("start_migration"), danger: true }))) return;
    databaseStart.disabled = true;
    try {
      await api("POST", "/api/v1/admin/database/migrate", { id: checkId });
      checkId = null;
      await loadMaintenance();
      await refreshDatabaseStatus(renderDatabaseStatus);
    } catch (error) {
      handle(error);
    }
  });
  refreshDatabaseStatus(renderDatabaseStatus);
}
