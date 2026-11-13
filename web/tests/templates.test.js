"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "templates.js")).href);

function form(overrides = {}) {
  const values = {
    tool: "ansible",
    job_type: "run",
    tags: "",
    skip_tags: "",
    limit: "",
    forks: "",
    diff: false,
    verbosity: "0",
    action: "plan",
    script: "",
    ...overrides,
  };
  return {
    tool: { value: values.tool },
    job_type: { value: values.job_type },
    tags: { value: values.tags },
    skip_tags: { value: values.skip_tags },
    limit: { value: values.limit },
    forks: { value: values.forks },
    diff: { checked: values.diff },
    verbosity: { value: values.verbosity },
    action: { value: values.action },
    script: { value: values.script },
  };
}

test("template specs keep only the values that differ from the defaults", async () => {
  const { templateSpec } = await load();
  assert.deepEqual(templateSpec(form()), { job_type: "run" });
  assert.deepEqual(templateSpec(form({
    job_type: "check",
    tags: "web",
    skip_tags: "slow",
    limit: "web:&db",
    forks: "5",
    diff: true,
    verbosity: "2",
  })), {
    job_type: "check",
    tags: "web",
    skip_tags: "slow",
    limit: "web:&db",
    diff: true,
    verbosity: 2,
    forks: 5,
  });
  assert.deepEqual(templateSpec(form({ tool: "opentofu", action: "apply" })), { action: "apply" });
  assert.deepEqual(templateSpec(form({ tool: "shell", script: "echo ok" })), { script: "echo ok" });
  assert.deepEqual(templateSpec(form({ tool: "shell", script: "  " })), {});
});

test("template specs reject a forks value that is not a whole number", async () => {
  const { templateSpec } = await load();
  assert.throws(() => templateSpec(form({ forks: "2.5" })), /forks_is_a_whole_number/);
});

test("the template entry point is required for Ansible and OpenTofu", async () => {
  const { templateEntrypointError } = await load();
  assert.equal(templateEntrypointError("ansible", ""), "ansible_needs_a_playbook_as_the_entry");
  assert.equal(templateEntrypointError("ansible", "  "), "ansible_needs_a_playbook_as_the_entry");
  assert.equal(templateEntrypointError("opentofu", ""), "opentofu_needs_a_module_directory");
  assert.equal(templateEntrypointError("shell", ""), "");
  assert.equal(templateEntrypointError("ansible", "site.yml"), "");
});

test("the template list filters by tool and across name, description and spec", async () => {
  const { filterTemplates } = await load();
  const items = [
    { name: "Deploy web", description: "frontend", tool: "ansible", entrypoint: "site.yml", spec: { tags: "web" } },
    { name: "Network", description: "vpc", tool: "opentofu", entrypoint: "infra", spec: { action: "plan" } },
    { name: "Backup", description: "cron", tool: "shell", entrypoint: "backup.sh", spec: {} },
  ];
  assert.deepEqual(filterTemplates(items, "all", "").map((item) => item.name), ["Deploy web", "Network", "Backup"]);
  assert.deepEqual(filterTemplates(items, "shell", "").map((item) => item.name), ["Backup"]);
  assert.deepEqual(filterTemplates(items, "all", "frontend").map((item) => item.name), ["Deploy web"]);
  assert.deepEqual(filterTemplates(items, "all", "WEB").map((item) => item.name), ["Deploy web"]);
  assert.deepEqual(filterTemplates(items, "all", "backup.sh").map((item) => item.name), ["Backup"]);
  assert.deepEqual(filterTemplates(items, "ansible", "vpc"), []);
});
