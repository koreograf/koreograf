"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "organizations.js")).href);

test("the organization list filter matches name and description", async () => {
  const { organizationMatches } = await load();
  const item = { name: "Platform", description: "Shared services" };
  assert.equal(organizationMatches(item, ""), true);
  assert.equal(organizationMatches(item, "plat"), true);
  assert.equal(organizationMatches(item, "shared"), true);
  assert.equal(organizationMatches(item, "network"), false);
});

test("an organization payload carries name and description", async () => {
  const { organizationPayload } = await load();
  const form = { name: { value: "Platform" }, description: { value: "Shared services" } };
  assert.deepEqual(organizationPayload(form), { name: "Platform", description: "Shared services" });
});
