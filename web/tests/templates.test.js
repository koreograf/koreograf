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

test("prompt field choices follow the tool", async () => {
  const { promptFieldsForTool, normalizePromptFields } = await load();
  assert.deepEqual(promptFieldsForTool("ansible"), [
    "inventory", "credentials", "extra_vars", "limit", "tags", "skip_tags", "job_type", "verbosity",
  ]);
  assert.deepEqual(promptFieldsForTool("opentofu"), ["credentials", "extra_vars", "action"]);
  assert.deepEqual(promptFieldsForTool("shell"), ["credentials", "extra_vars"]);
  assert.deepEqual(promptFieldsForTool("unknown"), []);
  assert.deepEqual(normalizePromptFields("ansible", ["action", "tags", "limit", "limit", "forks"]), ["limit", "tags"]);
  assert.deepEqual(normalizePromptFields("shell", ["limit", "extra_vars", "credentials"]), ["credentials", "extra_vars"]);
  assert.deepEqual(normalizePromptFields("opentofu", ["inventory", "action", "tags", "credentials"]), ["credentials", "action"]);
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

test("templates group into sorted sections with one group each", async () => {
  const { groupTemplates } = await load();
  const groups = [
    { id: "g2", name: "Kubernetes", position: 1 },
    { id: "g1", name: "Patchday", position: 0 },
  ];
  const items = [
    { id: "a", name: "Deploy", tool: "ansible", group_id: "g1" },
    { id: "b", name: "Backup", tool: "shell", group_id: null },
    { id: "c", name: "Patch", tool: "shell", group_id: "g1" },
  ];
  const sections = groupTemplates(items, groups);
  assert.deepEqual(sections.map((section) => (section.ungrouped ? "ungrouped" : section.id)), ["g1", "g2", "ungrouped"]);
  assert.deepEqual(sections[0].templates.map((item) => item.id), ["a", "c"]);
  assert.deepEqual(sections[1].templates.map((item) => item.id), []);
  assert.deepEqual(sections[2].templates.map((item) => item.id), ["b"]);
  assert.equal(groupTemplates([{ id: "a", group_id: "g1" }], groups).length, 2);
});

test("the template list filters by group and by ungrouped", async () => {
  const { filterTemplates } = await load();
  const items = [
    { name: "Deploy", tool: "ansible", group_id: "g1", spec: {} },
    { name: "Patch", tool: "shell", group_id: "g1", spec: {} },
    { name: "Backup", tool: "shell", group_id: null, spec: {} },
  ];
  assert.deepEqual(filterTemplates(items, "all", "", "g1").map((item) => item.name), ["Deploy", "Patch"]);
  assert.deepEqual(filterTemplates(items, "all", "", "g2").map((item) => item.name), []);
  assert.deepEqual(filterTemplates(items, "all", "", "ungrouped").map((item) => item.name), ["Backup"]);
  assert.deepEqual(filterTemplates(items, "shell", "", "g1").map((item) => item.name), ["Patch"]);
  assert.deepEqual(filterTemplates(items, "all", "back", "all").map((item) => item.name), ["Backup"]);
});

test("the collapsed group state is remembered per team", async () => {
  const { collapsedGroups, rememberCollapsed } = await load();
  const store = new Map();
  globalThis.localStorage = {
    getItem(key) { return store.has(key) ? store.get(key) : null; },
    setItem(key, value) { store.set(key, String(value)); },
    removeItem(key) { store.delete(key); },
  };
  assert.deepEqual([...collapsedGroups("team-a")], []);
  rememberCollapsed("team-a", new Set(["g1", "__ungrouped__"]));
  rememberCollapsed("team-b", new Set(["g2"]));
  assert.deepEqual([...collapsedGroups("team-a")], ["g1", "__ungrouped__"]);
  assert.deepEqual([...collapsedGroups("team-b")], ["g2"]);
  assert.deepEqual([...collapsedGroups("team-c")], []);
});
