"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

test("the runtime warning names a missing container runtime", async () => {
  const { environmentRuntimeWarning } = await load("features/templates.js");
  const cases = [
    ["auto", null, "ee", "this_execution_environment_is_ignored_until_docker_or"],
    ["auto", "", "ee", "this_execution_environment_is_ignored_until_docker_or"],
    ["auto", "docker", "ee", ""],
    ["auto", "podman", "ee", ""],
    ["auto", null, "", ""],
    ["docker", null, "ee", "an_execution_environment_is_set_but_neither_docker"],
    ["docker", "docker", "ee", ""],
    ["local", null, "ee", ""],
    ["", null, "ee", "this_execution_environment_is_ignored_until_docker_or"],
    [null, null, "ee", "this_execution_environment_is_ignored_until_docker_or"],
  ];
  for (const [mode, runtime, environmentId, want] of cases) {
    assert.equal(environmentRuntimeWarning(mode, runtime, environmentId), want, `${mode}/${runtime}/${environmentId}`);
  }
});

test("the playbook choice prefers the current path and falls back to the first book", async () => {
  const { playbookChoice } = await load("features/templates.js");
  assert.equal(playbookChoice(["a.yml", "b.yml"], "b.yml"), "b.yml");
  assert.equal(playbookChoice(["a.yml", "b.yml"], ""), "a.yml");
  assert.equal(playbookChoice(["a.yml"], "custom/path.yml"), "");
  assert.equal(playbookChoice([], ""), "");
  assert.equal(playbookChoice([], "x.yml"), "");
});

test("the template environments follow the selected tool", async () => {
  const { environmentsFor } = await load("features/templates.js");
  const environments = [
    { id: "a1", tool: "ansible" },
    { id: "t1", tool: "opentofu" },
    { id: "s1", tool: "shell" },
    { id: "any1", tool: "any" },
    { id: "legacy" },
  ];
  const ids = (tool) => environmentsFor(environments, tool).map((row) => row.id).join(",");
  assert.equal(ids("ansible"), "a1,any1,legacy");
  assert.equal(ids("opentofu"), "t1,any1,legacy");
  assert.equal(ids("shell"), "s1,any1,legacy");
});

test("the template credential kinds follow the tool", async () => {
  const { TOOL_CREDENTIAL_KINDS } = await load("features/templates.js");
  assert.deepEqual(TOOL_CREDENTIAL_KINDS.ansible, ["ssh", "vault"]);
  assert.deepEqual(TOOL_CREDENTIAL_KINDS.opentofu, ["aws", "azure", "gce", "vmware", "openstack", "kubernetes", "env"]);
  assert.deepEqual(TOOL_CREDENTIAL_KINDS.shell, ["ssh", "env"]);
});

test("the template validation errors map to their fields", async () => {
  const { ERROR_FIELD } = await load("core/forms.js");
  const expected = {
    tool_is_ansible_opentofu_or_shell: "tool",
    project_is_missing: "project_id",
    execution_environment_is_missing: "environment_id",
    the_execution_environment_is_for_another_tool: "environment_id",
    action_is_plan_or_apply: "action",
    ansible_needs_a_playbook_as_the_entry: "entrypoint",
    opentofu_needs_a_module_directory: "entrypoint",
    shell_needs_a_path_or_spec_script: "entrypoint",
    the_entry_path_must_be_relative_and_must: "entrypoint",
    extra_variables_must_be_a_json_object: "extra_vars",
    "template.vars_not_objects": "extra_vars",
  };
  for (const [key, field] of Object.entries(expected)) {
    assert.equal(ERROR_FIELD[key], field, key);
  }
});

test("the vault picker adds by search and removes a pill", async () => {
  const { bindVaultPicker } = await load("features/templates.js");
  const form = document.createElement("form");
  form.innerHTML = '<div id="vault-pills"></div><input id="vault-search"><div id="vault-options"></div>';
  document.body.appendChild(form);
  const credentials = Array.from({ length: 30 }, (_item, index) => ({
    id: `vault-${index}`,
    kind: "vault",
    name: index === 3 ? "Alpha vault" : index === 17 ? "Beta vault" : `Vault ${index}`,
  }));
  const picker = bindVaultPicker(form, credentials, [], false);
  const search = form.querySelector("#vault-search");
  const pills = form.querySelector("#vault-pills");
  assert.equal(picker.ids().length, 0, "pills did not start empty");

  search.value = "alpha";
  search.dispatchEvent({ type: "input" });
  search.dispatchEvent({ type: "keydown", key: "Enter" });
  assert.equal(picker.ids().join(","), "vault-3", "search did not add the highlighted vault");

  search.value = "beta";
  search.dispatchEvent({ type: "input" });
  search.dispatchEvent({ type: "keydown", key: "Enter" });
  assert.equal(picker.ids().join(","), "vault-3,vault-17", "second search did not append");

  const removeButton = pills.querySelector(".chip-remove");
  pills.dispatchEvent({ type: "click", target: removeButton });
  assert.equal(picker.ids().join(","), "vault-17", "remove did not drop the first pill");

  const before = picker.ids().join(",");
  search.value = "";
  search.dispatchEvent({ type: "keydown", key: "Backspace" });
  assert.equal(picker.ids().join(","), before, "Backspace removed a pill");

  const readonlyForm = document.createElement("form");
  readonlyForm.innerHTML = '<div id="vault-pills"></div>';
  const readonly = bindVaultPicker(readonlyForm, credentials, ["vault-3"], true);
  assert.equal(readonly.ids().join(","), "vault-3", "read-only picker lost the selected vault");
  const readonlyPills = readonlyForm.querySelector("#vault-pills");
  assert.match(readonlyPills.innerHTML, /Alpha vault/);
  assert.doesNotMatch(readonlyPills.innerHTML, /chip-remove/);
});
