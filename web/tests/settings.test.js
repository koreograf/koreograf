"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "settings.js")).href);

function form(options = {}) {
  const elements = {};
  Object.entries(options.secrets || {}).forEach(([name, value]) => {
    elements[name] = { value, dataset: (options.stored || []).includes(name) ? { storedMask: "1" } : {} };
  });
  return {
    elements,
    ldap_url: { value: " ldap://ldap.example.com " },
    ldap_mode: { value: "ad" },
    ldap_bind_dn: { value: " cn=svc,dc=example,dc=com " },
    ldap_user_base: { value: " ou=users,dc=example,dc=com " },
    ldap_user_filter: { value: " (sAMAccountName={username}) " },
    ldap_username_attr: { value: " sAMAccountName " },
    ldap_group_attr: { value: " memberOf " },
    ldap_group_base: { value: " dc=example,dc=com " },
    ldap_starttls: { checked: true },
    ldap_tls_insecure: { checked: false },
    proxy_url: { value: " http://proxy.example:3128 " },
    proxy_username: { value: " bob " },
    proxy_no_proxy: { value: " localhost,.internal " },
    runner: { value: "docker" },
    job_timeout_secs: { value: "3600" },
    job_output_lines: { value: "9000" },
    default_environment_ansible: { value: "env-1" },
    default_environment_opentofu: { value: "" },
    default_environment_shell: { value: "env-2" },
    llm_enabled: { checked: true },
    llm_provider: { value: "ollama" },
    llm_base_url: { value: " http://127.0.0.1:11434/v1 " },
    llm_model: { value: " llama3 " },
    querySelectorAll(selector) {
      assert.equal(selector, "[data-clear]:checked");
      return (options.clear || []).map((name) => ({ getAttribute: () => name }));
    },
  };
}

test("the settings payload trims text and keeps the numbers", async () => {
  const { settingsBody } = await load();
  const body = settingsBody(form({
    secrets: { ldap_bind_password: "bind", proxy_password: "", llm_api_key: "key" },
    clear: ["proxy_password"],
  }));
  assert.equal(body.ldap.url, "ldap://ldap.example.com");
  assert.equal(body.ldap.mode, "ad");
  assert.equal(body.ldap.bind_password, "bind");
  assert.equal(body.proxy.password, "");
  assert.equal(body.runner, "docker");
  assert.equal(body.job_timeout_secs, 3600);
  assert.equal(body.job_output_lines, 9000);
  assert.equal(body.llm.enabled, true);
  assert.equal(body.llm.provider, "ollama");
  assert.equal(body.llm.api_key, "key");
  assert.deepEqual(body.clear, ["proxy_password"]);
});

test("a stored secret is left out of the payload", async () => {
  const { settingsBody } = await load();
  const body = settingsBody(form({
    secrets: { ldap_bind_password: "********", proxy_password: "stored", llm_api_key: "********" },
    stored: ["ldap_bind_password", "llm_api_key"],
  }));
  assert.equal(body.ldap.bind_password, "");
  assert.equal(body.proxy.password, "stored");
  assert.equal(body.llm.api_key, "");
});

test("the settings tab falls back to the directory", async () => {
  const { settingsTab } = await load();
  location.hash = "#/settings?tab=assistant";
  assert.equal(settingsTab(), "assistant");
  location.hash = "#/settings?tab=database";
  assert.equal(settingsTab(), "database");
  location.hash = "#/settings?tab=unknown";
  assert.equal(settingsTab(), "directory");
});
