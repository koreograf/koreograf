"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "team.js")).href);

function form(options = {}) {
  const fields = {
    name: { value: options.name || "Platform" },
    description: { value: options.description || "Shared services" },
    ldap_group: { value: options.ldap_group || "" },
  };
  if (options.organization_id !== undefined) fields.organization_id = { value: options.organization_id };
  return fields;
}

test("a new team payload omits an empty organization", async () => {
  const { teamPayload } = await load();
  assert.deepEqual(teamPayload(form({ organization_id: "" })), {
    name: "Platform",
    description: "Shared services",
    ldap_group: "",
  });
});

test("an edited team payload keeps the chosen organization", async () => {
  const { teamPayload } = await load();
  assert.deepEqual(teamPayload(form({ organization_id: "org-1", ldap_group: "cn=platform" })), {
    name: "Platform",
    description: "Shared services",
    ldap_group: "cn=platform",
    organization_id: "org-1",
  });
});
