"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "environments.js")).href);

test("the environments list redirects to the runtime page", async () => {
  await import(pathToFileURL(join(__dirname, "..", "js", "features", "index.js")).href);
  const { matchRoute } = await import(pathToFileURL(join(__dirname, "..", "js", "core", "router.js")).href);
  const { navActive } = await import(pathToFileURL(join(__dirname, "..", "js", "core", "app.js")).href);
  const route = matchRoute("#/environments");
  assert.ok(route, "#/environments has no route");
  assert.deepEqual(route.handler(route.params), { redirect: "#/runtime" });
  assert.equal(navActive("#/environments", "#/runtime"), true);
  assert.equal(matchRoute("#/no-such-page"), null);
});

test("the environment list filter matches display fields", async () => {
  const { environmentMatches } = await load();
  const item = { name: "Web", tool: "ansible", image: "quay.io/ansible/awx-ee:latest", pull_policy: "always", scope: "team", team_name: "Ops" };
  assert.equal(environmentMatches(item, ""), true);
  assert.equal(environmentMatches(item, "web"), true);
  assert.equal(environmentMatches(item, "quay.io"), true);
  assert.equal(environmentMatches(item, "always"), true);
  assert.equal(environmentMatches(item, "opentofu"), false);
});

test("the image placeholder follows the tool", async () => {
  const { environmentImagePlaceholder } = await load();
  assert.equal(environmentImagePlaceholder("ansible"), "quay.io/ansible/awx-ee:latest");
  assert.equal(environmentImagePlaceholder("opentofu"), "ghcr.io/opentofu/opentofu:latest");
  assert.equal(environmentImagePlaceholder("shell"), "docker.io/library/alpine:latest");
  assert.equal(environmentImagePlaceholder(""), "registry.example.com/execution-environment:latest");
});

test("an environment payload carries every form field", async () => {
  const { environmentPayload } = await load();
  const form = {
    name: { value: "Web" },
    image: { value: "quay.io/ansible/awx-ee:latest" },
    tool: { value: "ansible" },
    pull_policy: { value: "always" },
    network: { value: "koreograf_default" },
    description: { value: "Runs playbooks" },
    scope: { value: "team" },
  };
  assert.deepEqual(environmentPayload(form), {
    name: "Web",
    image: "quay.io/ansible/awx-ee:latest",
    tool: "ansible",
    pull_policy: "always",
    network: "koreograf_default",
    description: "Runs playbooks",
    scope: "team",
  });
});
