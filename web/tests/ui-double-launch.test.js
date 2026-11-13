"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);
const tick = () => new Promise((resolve) => setImmediate(resolve));

async function setup() {
  const state = await load("core/state.js");
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  const main = document.createElement("main");
  main.className = "main";
  document.body.appendChild(main);
}

function item() {
  return {
    id: "t1",
    name: "Deploy",
    tool: "ansible",
    inventory_id: "inv-1",
    extra_vars: {},
    spec: { verbosity: 0, job_type: "run", tags: "", skip_tags: "", limit: "" },
  };
}

function launchForm() {
  const dialog = document.querySelector("#launch-prompt");
  const form = dialog.querySelector(".dialog-form");
  form.extra_vars.value = "{}";
  form.inventory_id.value = "inv-1";
  form.verbosity.value = "0";
  form.job_type.value = "run";
  form.tags.value = "";
  form.skip_tags.value = "";
  form.limit.value = "";
  return { dialog, form, button: form.querySelector('button[type="submit"]') };
}

test("a double submit creates one job and keeps the button busy", async () => {
  await setup();
  const { openLaunchPrompt } = await load("features/templates.js");
  let posts = 0;
  const pending = [];
  globalThis.fetch = async (path, init) => {
    if (init && init.method === "POST") {
      posts += 1;
      return new Promise((resolve) => pending.push(() => resolve({ ok: true, status: 200, text: async () => JSON.stringify({ id: `job-${posts}` }) })));
    }
    return { ok: true, status: 200, text: async () => "[]" };
  };
  await openLaunchPrompt(item(), true);
  const { form, button } = launchForm();
  form.dispatchEvent({ type: "submit", preventDefault() {} });
  form.dispatchEvent({ type: "submit", preventDefault() {} });
  assert.equal(posts, 1, "a double submit posts the launch twice");
  assert.equal(button.disabled, true, "the dialog submit stays enabled in flight");
  assert.equal(button.getAttribute("aria-busy"), "true", "the dialog submit has no busy state");
  pending.forEach((resolve) => resolve());
  await tick();
  await tick();
  assert.equal(posts, 1, "a double submit creates more than one job");
  assert.equal(document.querySelector("#launch-prompt"), null, "the dialog did not close once");
  assert.equal(location.hash, "#/jobs/job-1", "the dialog did not open the new job");
});

test("a failed launch restores the dialog submit", async () => {
  await setup();
  const { openLaunchPrompt } = await load("features/templates.js");
  let posts = 0;
  const pending = [];
  globalThis.fetch = async (path, init) => {
    if (init && init.method === "POST") {
      posts += 1;
      if (posts === 1) return { ok: false, status: 500, text: async () => JSON.stringify({ error: "boom" }) };
      return new Promise((resolve) => pending.push(() => resolve({ ok: true, status: 200, text: async () => JSON.stringify({ id: `job-${posts}` }) })));
    }
    return { ok: true, status: 200, text: async () => "[]" };
  };
  await openLaunchPrompt(item(), true);
  const { form, button } = launchForm();
  form.dispatchEvent({ type: "submit", preventDefault() {} });
  await tick();
  await tick();
  assert.equal(posts, 1, "the failed launch retried on its own");
  assert.equal(button.disabled, false, "a failed launch leaves the dialog submit disabled");
  assert.equal(button.getAttribute("aria-busy"), null, "a failed launch leaves the busy state");

  form.dispatchEvent({ type: "submit", preventDefault() {} });
  assert.equal(posts, 2, "a retry after failure does not post again");
  assert.equal(button.disabled, true, "a retry does not disable the dialog submit");
  pending.forEach((resolve) => resolve());
  await tick();
  await tick();
  assert.equal(location.hash, "#/jobs/job-2", "the retry did not open the new job");
});

test("the list launch disables its button before posting", async () => {
  await setup();
  const router = await load("core/router.js");
  await load("features/templates.js");
  const templates = [{ id: "shell-1", name: "Healthcheck", tool: "shell", entrypoint: "check.sh", spec: {} }];
  const pending = [];
  globalThis.fetch = async (path, init) => {
    if (init && init.method === "POST") {
      return new Promise((resolve) => pending.push(() => resolve({ ok: true, status: 200, text: async () => JSON.stringify({ id: "job-1" }) })));
    }
    const body = path.startsWith("/api/v1/templates") ? templates : [];
    return { ok: true, status: 200, text: async () => JSON.stringify(body) };
  };
  location.hash = "#/templates";
  await router.matchRoute("#/templates").handler();
  const button = document.querySelector('[data-launch="shell-1"]');
  button.dispatchEvent({ type: "click" });
  await tick();
  assert.equal(button.disabled, true, "the list launch no longer disables its button");
  pending.forEach((resolve) => resolve());
  await tick();
  await tick();
  assert.equal(location.hash, "#/jobs/job-1");
});
