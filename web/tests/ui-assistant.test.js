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
    assistant: "Assistant",
    assistant_action_done: "Done",
    assistant_action_canceled: "Canceled",
    assistant_context_job: "Job",
    assistant_context_workflow: "Workflow",
    assistant_context_inventory: "Inventory",
    assistant_context_host: "Host",
  });
  state.setMe({ is_admin: true, display_name: "Admin", teams: [{ id: "t1", name: "Team", role: "owner" }] });
  state.setTeamId("t1");
  state.setAssistantEnabled(true);
  state.setAssistantModel("local-model");
  return state;
}

function jobResponse() {
  return {
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
  };
}

test("the launcher is absent while the server keeps the assistant off", async () => {
  const state = await setup();
  const assistant = await load("features/assistant.js");
  assistant.syncAssistantPanel();
  assert.ok(document.querySelector("#assistant-toggle"), "the launcher is missing while enabled");
  state.setAssistantEnabled(false);
  assistant.syncAssistantPanel();
  assert.equal(document.querySelector("#assistant-root"), null, "the launcher shows while the assistant is off");
});

test("explain opens the panel in place, escapes the answer, and confirms and cancels the same endpoints", async () => {
  const state = await setup();
  const router = await load("core/router.js");
  const assistant = await load("features/assistant.js");
  await load("features/jobs.js");
  const calls = [];
  let chatCalls = 0;
  globalThis.fetch = async (path, init) => {
    const body = init && init.body ? JSON.parse(init.body) : null;
    calls.push([path, init && init.method, body]);
    if (path === "/api/v1/jobs/j1") {
      return { ok: true, status: 200, text: async () => JSON.stringify(jobResponse()) };
    }
    if (path.endsWith("/chat")) {
      chatCalls += 1;
      return {
        ok: true,
        status: 200,
        text: async () => JSON.stringify({
          answer: "<b>Use the deploy template</b>",
          pending_action: { id: `a${chatCalls}`, tool: "launch_template", target: "Deploy" },
        }),
      };
    }
    return { ok: true, status: 200, text: async () => JSON.stringify({}) };
  };
  location.hash = "#/jobs/j1";
  await router.matchRoute("#/jobs/:id").handler({ id: "j1" });
  assistant.syncAssistantPanel();
  const explain = document.querySelector("#assistant-explain");
  assert.ok(explain, "the failed job has no explain control");
  explain.dispatchEvent({ type: "click" });
  await tick();
  await tick();
  assert.equal(location.hash, "#/jobs/j1", "Explain navigates away from the job");
  const panel = document.querySelector("#assistant-panel");
  assert.equal(panel.hidden, false, "Explain does not open the panel");
  assert.equal(document.querySelector("#assistant-toggle").getAttribute("aria-expanded"), "true");
  assert.match(document.querySelector("#assistant-context-label").textContent, /Job · Broken/);
  const chat = calls.find(([path]) => path === "/api/v1/assistant/chat");
  assert.ok(chat, "Explain does not call the assistant route");
  assert.deepEqual(chat[2].context, { type: "job", id: "j1" }, "Explain does not send only a structured reference");
  const thread = document.querySelector("#assistant-thread");
  assert.match(thread.innerHTML, /&lt;b&gt;Use the deploy template&lt;\/b&gt;/, "the panel does not escape answers");
  assert.ok(thread.querySelector("[data-assistant-confirm]"), "a write has no confirmation card");
  assert.ok(thread.querySelector("[data-assistant-cancel]"), "a confirmation card cannot be canceled");

  thread.querySelector("[data-assistant-confirm]").dispatchEvent({ type: "click" });
  await tick();
  assert.deepEqual(calls.at(-1).slice(0, 2), ["/api/v1/assistant/confirm", "POST"]);

  document.querySelector("#assistant-form").message.value = "launch it again";
  document.querySelector("#assistant-form").dispatchEvent({ type: "submit" });
  await tick();
  await tick();
  assert.ok(thread.querySelector("[data-assistant-cancel]"), "a second write has no confirmation card");
  thread.querySelector("[data-assistant-cancel]").dispatchEvent({ type: "click" });
  await tick();
  assert.deepEqual(calls.at(-1).slice(0, 2), ["/api/v1/assistant/cancel", "POST"]);
  assert.equal(state.assistantModel, "local-model");
});

test("the context chip follows the page, clears the next message, and maps every detail route", async () => {
  await setup();
  const assistant = await load("features/assistant.js");
  assert.deepEqual(assistant.assistantContextForHash("#/jobs/j1"), { type: "job", id: "j1" });
  assert.deepEqual(assistant.assistantContextForHash("#/workflows/w1"), { type: "workflow", id: "w1" });
  assert.deepEqual(assistant.assistantContextForHash("#/runs/r1"), { type: "run", id: "r1" });
  assert.deepEqual(assistant.assistantContextForHash("#/templates?edit=t1"), { type: "template", id: "t1" });
  assert.deepEqual(assistant.assistantContextForHash("#/inventories/i1"), { type: "inventory", id: "i1" });
  assert.deepEqual(assistant.assistantContextForHash("#/inventories/i1/hosts/web%201"), { type: "host", id: "i1/web 1" });
  assert.deepEqual(assistant.assistantContextForHash("#/inventories/i1/groups/db"), { type: "group", id: "i1/db" });
  assert.deepEqual(assistant.assistantContextForHash("#/projects/p1"), { type: "project", id: "p1" });
  assert.deepEqual(assistant.assistantContextForHash("#/environments/e1"), { type: "environment", id: "e1" });
  assert.deepEqual(assistant.assistantContextForHash("#/jobs"), { type: "job" });
  assert.deepEqual(assistant.assistantContextForHash("#/jobs/%E0"), { type: "job", id: "%E0" });
  assert.equal(assistant.assistantContextForHash("#/settings"), null);

  location.hash = "#/inventories/i1";
  assistant.syncAssistantPanel();
  assistant.openAssistantPanel();
  assert.equal(document.querySelector("#assistant-context-row").hidden, false);
  assert.match(document.querySelector("#assistant-context-label").textContent, /Inventory · i1/);
  document.querySelector("#assistant-context-clear").dispatchEvent({ type: "click" });
  assert.equal(document.querySelector("#assistant-context-row").hidden, true, "the chip cannot be dismissed");

  const calls = [];
  globalThis.fetch = async (path, init) => {
    calls.push([path, init && init.method, init && init.body ? JSON.parse(init.body) : null]);
    return { ok: true, status: 200, text: async () => JSON.stringify({ answer: "ok" }) };
  };
  document.querySelector("#assistant-form").message.value = "Why?";
  document.querySelector("#assistant-form").dispatchEvent({ type: "submit" });
  await tick();
  await tick();
  const chat = calls.find(([path]) => path === "/api/v1/assistant/chat");
  assert.ok(chat);
  assert.equal(chat[2].context, undefined, "a dismissed chip still sends page context");

  location.hash = "#/workflows/w1";
  window.dispatchEvent({ type: "hashchange" });
  assert.equal(document.querySelector("#assistant-context-row").hidden, false, "navigation does not update the chip");
  assert.match(document.querySelector("#assistant-context-label").textContent, /Workflow · w1/);
});

test("the old assistant hash opens the panel and redirects to the previous page", async () => {
  await setup();
  const router = await load("core/router.js");
  const assistant = await load("features/assistant.js");
  assistant.syncAssistantPanel();
  location.hash = "#/assistant";
  const result = await router.matchRoute("#/assistant").handler();
  assert.equal(result.redirect, "#/");
  assert.equal(document.querySelector("#assistant-panel").hidden, false, "the old hash does not open the panel");
});
