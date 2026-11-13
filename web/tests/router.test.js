"use strict";

const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "core", "router.js")).href);

test("routes match static paths, parameters and ignore the query", async () => {
  const { createRouter } = await load();
  const router = createRouter();
  const calls = [];
  router.registerRoute("#/", () => calls.push("overview"));
  router.registerRoute("#/jobs", () => calls.push("jobs"));
  router.registerRoute("#/jobs/:id", (params) => calls.push(`job:${params.id}`));

  assert.deepEqual(router.matchRoute("#/").params, {});
  assert.equal(router.matchRoute("#/jobs").pattern, "#/jobs");
  const job = router.matchRoute("#/jobs/abc");
  assert.equal(job.pattern, "#/jobs/:id");
  assert.deepEqual(job.params, { id: "abc" });
  assert.equal(router.matchRoute("#/jobs?status=failed").pattern, "#/jobs");
  assert.equal(router.matchRoute("#/jobs/abc/logs"), null);
  assert.equal(router.matchRoute("#/missing"), null);
});

test("dynamic segments capture every nested parameter", async () => {
  const { createRouter } = await load();
  const router = createRouter();
  router.registerRoute("#/inventories/:id/hosts/:name", () => {});
  const matched = router.matchRoute("#/inventories/inv-1/hosts/web-1");
  assert.deepEqual(matched.params, { id: "inv-1", name: "web-1" });
});

test("static redirects resolve to their table entry", async () => {
  const { createRouter } = await load();
  const router = createRouter();
  router.registerRedirect("#/templates/new", "#/templates?new=1");
  const matched = router.matchRoute("#/templates/new");
  assert.equal(matched.handler(matched.params).redirect, "#/templates?new=1");
});

test("dynamic redirects escape their parameters", async () => {
  const { createRouter } = await load();
  const router = createRouter();
  router.registerRedirect("#/templates/:id", ({ id }) => `#/templates?edit=${encodeURIComponent(id)}`);
  const matched = router.matchRoute("#/templates/a b");
  assert.equal(matched.handler(matched.params).redirect, "#/templates?edit=a%20b");
});

test("specific routes win when they are registered first", async () => {
  const { createRouter } = await load();
  const router = createRouter();
  router.registerRedirect("#/templates/new", "#/templates?new=1");
  router.registerRedirect("#/templates/:id", ({ id }) => `#/templates?edit=${id}`);
  assert.equal(router.matchRoute("#/templates/new").pattern, "#/templates/new");
});
