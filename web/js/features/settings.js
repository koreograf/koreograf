import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { confirmDialog } from "../core/dialog.js";
import { deny, handle, shell } from "../core/app.js";
import { esc, h, selectOptions } from "../core/dom.js";
import { environmentToolLabel, preflightReportHtml } from "../core/format.js";
import { secretInput } from "../core/forms.js";
import { t } from "../core/i18n.js";
import { registerRoute } from "../core/router.js";
import { bindDatabaseMigration, databasePanelHtml, resetDatabaseCheck } from "./database.js";

function optionSelected(current, value) {
  return current === value ? " selected" : "";
}

export function settingsBody(form) {
  const secret = (name) => {
    const field = form.elements[name];
    if (!field || field.dataset.storedMask === "1") return "";
    return field.value;
  };
  const clear = [...form.querySelectorAll("[data-clear]:checked")].map((node) => node.getAttribute("data-clear"));
  return {
    ldap: {
      url: form.ldap_url.value.trim(),
      mode: form.ldap_mode.value,
      bind_dn: form.ldap_bind_dn.value.trim(),
      bind_password: secret("ldap_bind_password"),
      user_base: form.ldap_user_base.value.trim(),
      user_filter: form.ldap_user_filter.value.trim(),
      username_attr: form.ldap_username_attr.value.trim(),
      group_attr: form.ldap_group_attr.value.trim(),
      group_base: form.ldap_group_base.value.trim(),
      starttls: form.ldap_starttls.checked,
      tls_insecure: form.ldap_tls_insecure.checked,
    },
    proxy: {
      url: form.proxy_url.value.trim(),
      username: form.proxy_username.value.trim(),
      password: secret("proxy_password"),
      no_proxy: form.proxy_no_proxy.value.trim(),
    },
    runner: form.runner.value,
    job_timeout_secs: Number(form.job_timeout_secs.value),
    job_output_lines: Number(form.job_output_lines.value),
    default_environment_ansible: form.default_environment_ansible.value,
    default_environment_opentofu: form.default_environment_opentofu.value,
    default_environment_shell: form.default_environment_shell.value,
    llm: {
      enabled: form.llm_enabled.checked,
      provider: form.llm_provider.value,
      base_url: form.llm_base_url.value.trim(),
      model: form.llm_model.value.trim(),
      api_key: secret("llm_api_key"),
    },
    clear,
  };
}

function bindStoredMasks(form) {
  form.querySelectorAll("[data-stored-mask]").forEach((field) => {
    field.addEventListener("focus", () => {
      if (field.dataset.storedMask === "1") field.select();
    });
    field.addEventListener("input", () => {
      delete field.dataset.storedMask;
    });
  });
}

export function settingsTab() {
  const params = new URLSearchParams((location.hash || "").split("?")[1] || "");
  const tab = params.get("tab");
  return ["directory", "proxy", "jobs", "assistant", "database"].includes(tab) ? tab : "directory";
}

export function settingsTabAddress(tab) {
  return tab === "directory" ? "#/settings" : `#/settings?tab=${tab}`;
}

function settingsTabBar(active) {
  const item = (id) => `<button class="btn pill${active === id ? " primary" : ""}" type="button" role="tab" id="settings-tab-${id}" aria-selected="${active === id}" aria-controls="settings-panel-${id}" tabindex="${active === id ? "0" : "-1"}" data-settings-tab="${id}">${h(id)}</button>`;
  return `<div class="spread tab-bar"><div class="tabs" role="tablist" aria-label="${h("settings")}">${["directory", "proxy", "jobs", "assistant", "database"].map(item).join("")}</div></div>`;
}

async function pageSettings(notice) {
  if (!appState.me.is_admin) {
    deny("settings", "admin");
    return;
  }
  resetDatabaseCheck();
  const item = await api("GET", "/api/v1/settings");
  const environments = appState.teamId ? await api("GET", "/api/v1/environments") : [];
  const ldap = item.ldap || {};
  const proxy = item.proxy || {};
  const llm = item.llm || {};
  const llmProvider = ["openai", "ollama"].includes(llm.provider) ? llm.provider : "openai";
  const mode = ldap.mode === "ad" ? "ad" : "ldap";
  const runner = ["auto", "docker", "local"].includes(item.runner) ? item.runner : "auto";
  const defaultsFor = (tool) => environments.filter((row) => row.scope === "global" && (!row.tool || row.tool === "any" || row.tool === tool));
  const defaultOptions = (tool, selected) => selectOptions(defaultsFor(tool), selected || "", true, (row) => `${row.name} (${environmentToolLabel(row.tool)})`);
  const source = item.source === "saved"
    ? "these_values_are_saved_and_apply_immediately"
    : "nothing_is_saved_yet_the_environment_variables_from";
  const unsupported = proxy.unsupported_env
    ? `<p class="banner warn">${h("the_environment_uses_a_proxy_that_is_neither")}</p>`
    : "";
  const saved = notice ? `<p class="banner good">${h(notice)}</p>` : "";
  const activeTab = settingsTab();
  const panelHidden = (id) => activeTab === id ? "" : " hidden";
  shell(t("settings"), `<form class="card form" id="settings-form">
    ${saved}
    <p class="small muted">${h(source)}</p>
    <p class="small muted">${h("the_bind_address_data_directory_and_first_admin")}</p>
    ${settingsTabBar(activeTab)}
    <section class="form-section settings-panel" id="settings-panel-directory" role="tabpanel" aria-labelledby="settings-tab-directory"${panelHidden("directory")}>
      <h2>${h("directory")}</h2>
      <p class="small muted">${h("ldap_or_active_directory_an_empty_address_turns")}</p>
      <div class="field-row">
        <label class="field"><span>${h("mode")}</span>
          <select class="fit" name="ldap_mode">
            <option value="ldap"${optionSelected(mode, "ldap")}>LDAP</option>
            <option value="ad"${optionSelected(mode, "ad")}>${h("active_directory")}</option>
          </select>
        </label>
      </div>
      <label class="field"><span>${h("address")}</span><input name="ldap_url" value="${esc(ldap.url || "")}" placeholder="ldap://ldap.example.com" autocomplete="off" spellcheck="false"></label>
      <label class="field"><span>${h("bind_dn")}</span><input name="ldap_bind_dn" value="${esc(ldap.bind_dn || "")}" placeholder="cn=svc,dc=example,dc=com" autocomplete="off" spellcheck="false"></label>
      ${secretInput("ldap_bind_password", "bind_password", "password", Boolean(ldap.bind_password_set))}
      <label class="field"><span>${h("user_base")}</span><input name="ldap_user_base" value="${esc(ldap.user_base || "")}" placeholder="ou=users,dc=example,dc=com" autocomplete="off" spellcheck="false"></label>
      <div class="field-grid">
        <label class="field"><span>${h("user_filter")}</span><input name="ldap_user_filter" value="${esc(ldap.user_filter || "")}" spellcheck="false"></label>
        <label class="field"><span>${h("username_attribute")}</span><input name="ldap_username_attr" value="${esc(ldap.username_attr || "")}" spellcheck="false"></label>
        <label class="field"><span>${h("group_attribute")}</span><input name="ldap_group_attr" value="${esc(ldap.group_attr || "")}" spellcheck="false"></label>
        <label class="field"><span>${h("group_base")}</span><input name="ldap_group_base" value="${esc(ldap.group_base || "")}" autocomplete="off" spellcheck="false"></label>
      </div>
      <label class="row"><input name="ldap_starttls" type="checkbox"${ldap.starttls ? " checked" : ""}> ${h("starttls")}</label>
      <label class="row"><input name="ldap_tls_insecure" type="checkbox"${ldap.tls_insecure ? " checked" : ""}> ${h("skip_certificate_checks_only_on_a_network_you")}</label>
    </section>
    <section class="form-section settings-panel" id="settings-panel-proxy" role="tabpanel" aria-labelledby="settings-tab-proxy"${panelHidden("proxy")}>
      <h2>${h("proxy")}</h2>
      <p class="small muted">${h("one_address_is_used_for_http_and_https")}</p>
      <p class="small muted">${h("an_empty_address_turns_the_proxy_off")}</p>
      ${unsupported}
      <label class="field"><span>${h("proxy_address")}</span><input name="proxy_url" value="${esc(proxy.url || "")}" placeholder="http://proxy.example:3128" autocomplete="off" spellcheck="false"></label>
      <div class="field-row">
        <label class="field"><span>${h("proxy_user")}</span><input class="fit" name="proxy_username" value="${esc(proxy.username || "")}" autocomplete="off" spellcheck="false"></label>
        ${secretInput("proxy_password", "proxy_password", "password", Boolean(proxy.password_set))}
      </div>
      <label class="field"><span>${h("exceptions")}</span><input name="proxy_no_proxy" value="${esc(proxy.no_proxy || "")}" placeholder="localhost,127.0.0.1,.internal" autocomplete="off" spellcheck="false"></label>
    </section>
    <section class="form-section settings-panel" id="settings-panel-jobs" role="tabpanel" aria-labelledby="settings-tab-jobs"${panelHidden("jobs")}>
      <div class="spread"><h2>${h("jobs")}</h2><button class="btn pill" id="preflight-check" type="button">${h("check_again")}</button></div>
      <div id="preflight-result"></div>
      <p class="small muted">${h("the_mode_and_the_time_limit_apply_to")}</p>
      <div class="field-row">
        <label class="field"><span>${h("runner")}</span>
          <select class="fit" name="runner">
            <option value="auto"${optionSelected(runner, "auto")}>${h("automatic")}</option>
            <option value="docker"${optionSelected(runner, "docker")}>${h("container")}</option>
            <option value="local"${optionSelected(runner, "local")}>${h("local")}</option>
          </select>
        </label>
        <label class="field"><span>${h("time_limit_in_seconds")}</span><input class="tiny" name="job_timeout_secs" type="number" min="30" max="86400" required value="${esc(String(item.job_timeout_secs || 1800))}"></label>
        <label class="field"><span>${h("output_lines")}</span><input class="tiny" name="job_output_lines" type="number" min="100" max="100000" required value="${esc(String(item.job_output_lines || 8000))}"></label>
      </div>
      <p class="small muted">${h("the_log_keeps_the_first_three_quarters_and_the_last")}</p>
      <p class="small muted">${h("automatic_uses_an_execution_environment_when_docker_or")}</p>
      <p class="small muted">${h("a_template_without_an_environment_uses_the_default")}</p>
      <div class="field-grid">
        <label class="field"><span>${h("default_environment_ansible")}</span><select name="default_environment_ansible">${defaultOptions("ansible", item.default_environment_ansible)}</select></label>
        <label class="field"><span>${h("default_environment_opentofu")}</span><select name="default_environment_opentofu">${defaultOptions("opentofu", item.default_environment_opentofu)}</select></label>
        <label class="field"><span>${h("default_environment_shell")}</span><select name="default_environment_shell">${defaultOptions("shell", item.default_environment_shell)}</select></label>
      </div>
    </section>
    <section class="form-section settings-panel" id="settings-panel-assistant" role="tabpanel" aria-labelledby="settings-tab-assistant"${panelHidden("assistant")}>
      <h2>${h("assistant")}</h2>
      <p class="small muted">${h("assistant_settings_hint")}</p>
      <label class="row"><input name="llm_enabled" type="checkbox"${llm.enabled ? " checked" : ""}> ${h("assistant_enable")}</label>
      <div class="field-row">
        <label class="field"><span>${h("assistant_provider")}</span>
          <select class="fit" name="llm_provider">
            <option value="openai"${optionSelected(llmProvider, "openai")}>${h("assistant_provider_openai")}</option>
            <option value="ollama"${optionSelected(llmProvider, "ollama")}>${h("assistant_provider_ollama")}</option>
          </select>
        </label>
      </div>
      <label class="field"><span>${h("assistant_base_url")}</span><input name="llm_base_url" value="${esc(llm.base_url || "")}" placeholder="http://127.0.0.1:11434/v1" autocomplete="off" spellcheck="false"></label>
      <label class="field"><span>${h("assistant_model_name")}</span><input name="llm_model" value="${esc(llm.model || "")}" placeholder="llama3" autocomplete="off" spellcheck="false"></label>
      ${secretInput("llm_api_key", "assistant_api_key", "password", Boolean(llm.api_key_set))}
      <p class="small muted">${h("assistant_key_hint")}</p>
    </section>
    ${databasePanelHtml(activeTab === "database")}
    <p class="small muted">${h("saving_replaces_the_environment_variables_an_empty_password")}</p>
    <div class="choice-row">
      <button class="btn pill primary" type="submit">${h("save")}</button>
      <button class="btn pill" id="settings-reset" type="button">${h("use_environment_variables")}</button>
    </div>
  </form>`);
  const form = document.querySelector("#settings-form");
  bindStoredMasks(form);
  const tabButtons = [...document.querySelectorAll("[data-settings-tab]")];
  const tabPanels = {
    directory: document.querySelector("#settings-panel-directory"),
    proxy: document.querySelector("#settings-panel-proxy"),
    jobs: document.querySelector("#settings-panel-jobs"),
    assistant: document.querySelector("#settings-panel-assistant"),
    database: document.querySelector("#settings-panel-database"),
  };
  let currentTab = activeTab;
  const chooseTab = (next, focus) => {
    currentTab = ["directory", "proxy", "jobs", "assistant", "database"].includes(next) ? next : "directory";
    tabButtons.forEach((button) => {
      const active = button.dataset.settingsTab === currentTab;
      button.classList.toggle("primary", active);
      button.setAttribute("aria-selected", String(active));
      button.tabIndex = active ? 0 : -1;
    });
    Object.entries(tabPanels).forEach(([id, panel]) => {
      if (panel) panel.hidden = id !== currentTab;
    });
    const address = settingsTabAddress(currentTab);
    if ((location.hash || "#/") !== address) {
      history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", address);
      appState.setShownHash(address);
    }
    if (focus) {
      const active = tabButtons.find((button) => button.dataset.settingsTab === currentTab);
      if (active) active.focus();
    }
  };
  tabButtons.forEach((button, index) => {
    button.addEventListener("click", () => {
      if (button.dataset.settingsTab !== currentTab) chooseTab(button.dataset.settingsTab, false);
    });
    button.addEventListener("keydown", (event) => {
      let target = null;
      if (event.key === "ArrowRight") target = tabButtons[(index + 1) % tabButtons.length];
      else if (event.key === "ArrowLeft") target = tabButtons[(index - 1 + tabButtons.length) % tabButtons.length];
      else if (event.key === "Home") target = tabButtons[0];
      else if (event.key === "End") target = tabButtons[tabButtons.length - 1];
      if (!target) return;
      event.preventDefault();
      chooseTab(target.dataset.settingsTab, true);
    });
  });
  chooseTab(activeTab, false);
  form.ldap_mode.addEventListener("change", () => {
    const ad = form.ldap_mode.value === "ad";
    const next = ad
      ? { filter: "(sAMAccountName={username})", attr: "sAMAccountName" }
      : { filter: "(uid={username})", attr: "uid" };
    const previous = ad
      ? { filter: "(uid={username})", attr: "uid" }
      : { filter: "(sAMAccountName={username})", attr: "sAMAccountName" };
    if (!form.ldap_user_filter.value.trim() || form.ldap_user_filter.value.trim() === previous.filter) {
      form.ldap_user_filter.value = next.filter;
    }
    if (!form.ldap_username_attr.value.trim() || form.ldap_username_attr.value.trim() === previous.attr) {
      form.ldap_username_attr.value = next.attr;
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    try {
      await api("PUT", "/api/v1/settings", settingsBody(form));
      await pageSettings("settings_saved");
    } catch (error) {
      handle(error);
    }
  });
  document.querySelector("#preflight-check").addEventListener("click", async (event) => {
    const button = event.currentTarget;
    button.disabled = true;
    try {
      const report = await api("GET", "/api/v1/runner/preflight");
      document.querySelector("#preflight-result").innerHTML = preflightReportHtml(report);
    } catch (error) {
      handle(error);
    } finally {
      button.disabled = false;
    }
  });
  document.querySelector("#settings-reset").addEventListener("click", async () => {
    if (!(await confirmDialog({ title: t("discard_the_saved_settings_and_use_the_environment"), confirmLabel: t("ok") }))) return;
    try {
      await api("PUT", "/api/v1/settings", { reset: true });
      await pageSettings();
    } catch (error) {
      handle(error);
    }
  });
  bindDatabaseMigration(form);
}

registerRoute("#/settings", pageSettings);
