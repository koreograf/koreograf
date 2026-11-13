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
  const i18n = await load("core/i18n.js");
  i18n.setCatalog("de", {
    apply: "Apply",
    opentofu_apply_confirmation_title: "Apply OpenTofu?",
    opentofu_apply_confirmation_body: 'Start OpenTofu apply for "{name}"?',
  });
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  return state;
}

test("only an OpenTofu apply asks for confirmation", async () => {
  await setup();
  const { confirmTofuApply } = await load("core/tofu.js");
  const answer = confirmTofuApply({ name: "apply-prod" });
  const dialog = document.querySelector("#tofu-apply-confirm");
  assert.ok(dialog, "the confirmation has no stable id");
  assert.match(dialog.innerHTML, /Apply OpenTofu\?/);
  assert.match(dialog.innerHTML, /apply-prod/);
  assert.match(dialog.innerHTML, /Apply/);
  dialog.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  assert.equal(await answer, false, "cancel did not resolve false");

  const confirmed = confirmTofuApply({ name: "apply-prod" });
  const form = document.querySelector("#tofu-apply-confirm").querySelector(".dialog-form");
  form.dispatchEvent({ type: "submit", preventDefault() {} });
  assert.equal(await confirmed, true, "confirm did not resolve true");
  await tick();
  assert.equal(document.querySelector("#tofu-apply-confirm"), null, "confirm did not close the dialog");
});

test("the launch dialog waits for the OpenTofu confirmation", async () => {
  await setup();
  const { openLaunchPrompt } = await load("features/templates.js");
  let posts = 0;
  globalThis.fetch = async (path, init) => {
    if (init && init.method === "POST") {
      posts += 1;
      return { ok: true, status: 200, text: async () => JSON.stringify({ id: "job-1" }) };
    }
    return { ok: true, status: 200, text: async () => "[]" };
  };
  const item = { id: "tofu-1", name: "apply-prod", tool: "opentofu", extra_vars: {}, spec: { action: "apply" } };
  await openLaunchPrompt(item, true);
  let dialog = document.querySelector("#launch-prompt");
  assert.ok(dialog, "the launch dialog did not open");
  const form = dialog.querySelector(".dialog-form");
  form.extra_vars.value = "{}";
  const button = form.querySelector('button[type="submit"]');

  form.dispatchEvent({ type: "submit", preventDefault() {} });
  let confirm = document.querySelector("#tofu-apply-confirm");
  assert.ok(confirm, "the OpenTofu launch did not ask for confirmation");
  confirm.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  await tick();
  await tick();
  assert.equal(posts, 0, "cancel still launched a job");
  assert.ok(document.querySelector("#launch-prompt"), "cancel closed the launch form");
  assert.equal(form.extra_vars.value, "{}");
  assert.equal(button.disabled, false, "cancel left the launch button disabled");

  form.dispatchEvent({ type: "submit", preventDefault() {} });
  confirm = document.querySelector("#tofu-apply-confirm");
  assert.ok(confirm, "the retried OpenTofu launch did not ask for confirmation");
  confirm.querySelector(".dialog-form").dispatchEvent({ type: "submit", preventDefault() {} });
  await tick();
  await tick();
  assert.equal(posts, 1, "confirm did not launch exactly one job");
  assert.equal(document.querySelector("#launch-prompt"), null, "confirm did not close the launch form");
  assert.equal(location.hash, "#/jobs/job-1", "confirm did not open the new job");
});

test("the template list confirms an OpenTofu apply before posting", async () => {
  await setup();
  const router = await load("core/router.js");
  await load("features/templates.js");
  const templates = [{ id: "tofu-1", name: "apply-prod", tool: "opentofu", spec: { action: "apply" } }];
  let posts = 0;
  globalThis.fetch = async (path, init) => {
    if (init && init.method === "POST") {
      posts += 1;
      return { ok: true, status: 200, text: async () => JSON.stringify({ id: "job-1" }) };
    }
    const body = path.startsWith("/api/v1/templates") ? templates : [];
    return { ok: true, status: 200, text: async () => JSON.stringify(body) };
  };
  location.hash = "#/templates";
  await router.matchRoute("#/templates").handler();
  document.querySelector('[data-launch="tofu-1"]').dispatchEvent({ type: "click" });
  await tick();
  assert.ok(document.querySelector("#tofu-apply-confirm"), "the one-click launch skipped the confirmation");
  assert.equal(posts, 0, "the template launch posted before confirming");
  document.querySelector("#tofu-apply-confirm .dialog-form").dispatchEvent({ type: "submit", preventDefault() {} });
  await tick();
  await tick();
  assert.equal(posts, 1, "the confirmed launch did not post");
});
