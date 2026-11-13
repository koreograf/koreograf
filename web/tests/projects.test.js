"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "projects.js")).href);
const loadDom = () => import(pathToFileURL(join(__dirname, "..", "js", "core", "dom.js")).href);

function form(overrides = {}) {
  const values = {
    name: "Web",
    source_type: "git",
    source: "https://example.com/repo.git",
    revision: "main",
    credential_id: "cred-1",
    ...overrides,
  };
  const fields = {
    name: { value: values.name },
    source_type: { value: values.source_type },
    source: { value: values.source },
    revision: { value: values.revision },
    credential_id: { value: values.credential_id },
  };
  if (values.organization_id !== undefined) fields.organization_id = { value: values.organization_id };
  return {
    ...fields,
    querySelectorAll(selector) {
      assert.equal(selector, "input[name=team_ids]:checked");
      return (values.team_ids || []).map((id) => ({ value: id }));
    },
  };
}

test("the project list filter matches name, organization and type", async () => {
  const { projectMatches } = await load();
  const item = { name: "Web", organization_name: "Ops", source_type: "git", team_ids: ["team-1"] };
  assert.equal(projectMatches(item, ""), true);
  assert.equal(projectMatches(item, "web"), true);
  assert.equal(projectMatches(item, "ops"), true);
  assert.equal(projectMatches(item, "git"), true);
  assert.equal(projectMatches(item, "ansible"), false);
});

test("the shared organization options mark the selected entry and escape names", async () => {
  const { orgOptions } = await loadDom();
  assert.equal(
    orgOptions([{ id: "org-1", name: "A & B" }, { id: "org-2", name: "Ops" }], "org-2"),
    `<option value="org-1">A &amp; B</option><option value="org-2" selected>Ops</option>`
  );
});

test("a new project payload carries the source and no team ids", async () => {
  const { projectPayload } = await load();
  assert.deepEqual(projectPayload(form({ organization_id: "org-1" }), false), {
    name: "Web",
    source_type: "git",
    source: "https://example.com/repo.git",
    revision: "main",
    credential_id: "cred-1",
    organization_id: "org-1",
    team_ids: [],
  });
});

test("an edited project payload keeps the sorted team ids", async () => {
  const { projectPayload } = await load();
  assert.deepEqual(projectPayload(form({ team_ids: ["team-b", "team-a"] }), true), {
    name: "Web",
    source_type: "git",
    source: "https://example.com/repo.git",
    revision: "main",
    credential_id: "cred-1",
    team_ids: ["team-a", "team-b"],
  });
});
