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
  i18n.setCatalog("de", { assistant: "Assistant", assistant_action_done: "Done", assistant_action_canceled: "Canceled" });
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  return state;
}

test("the assistant entry only shows when the server enables it", async () => {
  await setup();
  const { assistantNavItem } = await load("core/app.js");
  assert.deepEqual(assistantNavItem(false), []);
  assert.deepEqual(assistantNavItem(true), [["#/assistant", "Assistant"]]);
});

test("the chat page explains a failed job and confirms a write", async () => {
  const state = await setup();
  state.setAssistantEnabled(true);
  const router = await load("core/router.js");
  await load("features/assistant.js");
  const calls = [];
  globalThis.fetch = async (path, init) => {
    calls.push([path, init && init.method]);
    const body = path.endsWith("/chat")
      ? { answer: "<b>Use the deploy template</b>", pending_action: { id: "a1", tool: "launch_template", target: "Deploy" } }
      : {};
    return { ok: true, status: 200, text: async () => JSON.stringify(body) };
  };
  location.hash = "#/assistant?explain=j1";
  await router.matchRoute("#/assistant").handler();
  assert.ok(calls.some(([path, method]) => path === "/api/v1/assistant/chat" && method === "POST"), "the chat page does not call the assistant route");
  assert.equal(location.hash, "#/assistant");
  await tick();
  await tick();
  const thread = document.querySelector("#assistant-thread");
  assert.match(thread.innerHTML, /&lt;b&gt;Use the deploy template&lt;\/b&gt;/, "the chat page does not escape answers");
  assert.ok(thread.querySelector("[data-assistant-confirm]"), "a write has no confirmation card");
  assert.ok(thread.querySelector("[data-assistant-cancel]"), "a confirmation card cannot be canceled");
  thread.querySelector("[data-assistant-confirm]").dispatchEvent({ type: "click" });
  await tick();
  assert.deepEqual(calls.at(-1), ["/api/v1/assistant/confirm", "POST"]);
});

test("a disabled assistant shows the notice instead of the composer", async () => {
  const state = await setup();
  state.setAssistantEnabled(false);
  const router = await load("core/router.js");
  await load("features/assistant.js");
  location.hash = "#/assistant";
  await router.matchRoute("#/assistant").handler();
  const app = document.querySelector("#app");
  assert.match(app.innerHTML, /assistant_is_not_enabled/);
  assert.equal(document.querySelector("#assistant-form"), null);
});

test("a failed job links to the assistant", async () => {
  const state = await setup();
  state.setAssistantEnabled(true);
  globalThis.fetch = async () => ({
    ok: true,
    status: 200,
    text: async () => JSON.stringify({
      id: "j1",
      name: "Broken",
      tool: "shell",
      status: "failed",
      trigger: "manual",
      launched_by: "admin",
      created_at: "2026-01-01T00:00:00Z",
      events: [],
      parameters: null,
      extra_vars: {},
    }),
  });
  const router = await load("core/router.js");
  await load("features/jobs.js");
  location.hash = "#/jobs/j1";
  await router.matchRoute("#/jobs/:id").handler({ id: "j1" });
  assert.match(document.querySelector("#app").innerHTML, /href="#\/assistant\?explain=j1"/);
});
