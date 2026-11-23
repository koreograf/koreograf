"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "service-accounts.js")).href);

test("the service-account filter matches the username, the display name and the creator", async () => {
  const { serviceAccountMatches } = await load();
  const item = { display_name: "Deploy bot", username: "ci-deploy", created_by_username: "ada" };
  assert.equal(serviceAccountMatches(item, ""), true);
  assert.equal(serviceAccountMatches(item, "deploy"), true);
  assert.equal(serviceAccountMatches(item, "ci-dep"), true);
  assert.equal(serviceAccountMatches(item, "ada"), true);
  assert.equal(serviceAccountMatches(item, "grace"), false);
});

test("a new service account carries one team and its role", async () => {
  const { serviceAccountCreatePayload } = await load();
  const payload = serviceAccountCreatePayload({
    username: { value: "ci-deploy" },
    display_name: { value: "Deploy bot" },
    team_id: { value: "team-1" },
    role: { value: "operator" },
  });
  assert.deepEqual(payload, {
    username: "ci-deploy",
    display_name: "Deploy bot",
    teams: [{ team_id: "team-1", role: "operator" }],
  });
});

test("a service-account token sends a numeric lifetime, or null for no expiry", async () => {
  const { serviceAccountTokenPayload } = await load();
  const form = (days) => ({
    name: { value: "pipeline" },
    scope: { value: "write" },
    expires_in_days: { value: days },
  });
  assert.deepEqual(serviceAccountTokenPayload(form("90")), {
    name: "pipeline",
    scope: "write",
    expires_in_days: 90,
  });
  assert.deepEqual(serviceAccountTokenPayload(form("")), {
    name: "pipeline",
    scope: "write",
    expires_in_days: null,
  });
});

test("a deleted creator is shown as missing", async () => {
  const { serviceAccountCreatorCell } = await load();
  const i18n = await import(pathToFileURL(join(__dirname, "..", "js", "core", "i18n.js")).href);
  i18n.setCatalog("de", { service_account_creator_missing: "Ersteller gelöscht" });
  assert.match(serviceAccountCreatorCell({ created_by_username: "ada" }), /ada/);
  assert.match(serviceAccountCreatorCell({ created_by_username: null }), /Ersteller gelöscht/);
});
