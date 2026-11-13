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
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "[]" });
  return state;
}

test("a source card marks itself dirty and can be removed", async () => {
  await setup();
  const { mountSourceDraft } = await load("features/inventories.js");
  const container = document.createElement("div");
  document.body.appendChild(container);
  mountSourceDraft(container, [], "ini", {}, {});
  const card = container.querySelector(".card");
  assert.ok(card, "the source card was not added");
  assert.equal(card.dataset.dirty, undefined);
  card.dispatchEvent({ type: "input" });
  assert.equal(card.dataset.dirty, "1", "input does not mark the source card dirty");
  delete card.dataset.dirty;
  card.dispatchEvent({ type: "change" });
  assert.equal(card.dataset.dirty, "1", "change does not mark the source card dirty");
  card.querySelector("[data-remove-draft]").dispatchEvent({ type: "click" });
  assert.equal(container.querySelector(".card"), null, "cancelling a source card does not remove it");
});

test("a dirty edit asks before leaving and the guard is cleared after the answer", async () => {
  const state = await setup();
  const app = await load("core/app.js");
  const router = await load("core/router.js");
  let routed = 0;
  router.registerRoute("#/guard-target", () => { routed += 1; });
  state.setShownHash("#/source-edit");
  state.setLeaveGuard(() => true);
  location.hash = "#/guard-target";
  const navigation = app.route();
  const confirm = document.querySelector("#confirm-dialog");
  assert.ok(confirm, "a dirty edit left without asking");
  confirm.querySelector(".dialog-form").dispatchEvent({ type: "submit", preventDefault() {} });
  await navigation;
  await tick();
  assert.equal(routed, 1, "the confirmed navigation did not load the page");
  assert.equal(state.leaveGuard, null, "route does not clear the leave guard after the confirm");
});

test("the source edit leave guard sees dirty cards and drafts", async () => {
  await setup();
  const { mountSourceDraft, sourceEditGuard } = await load("features/inventories.js");
  const editor = document.createElement("div");
  editor.id = "source-edit";
  const drafts = document.createElement("div");
  drafts.id = "source-drafts";
  document.body.appendChild(editor);
  document.body.appendChild(drafts);
  assert.equal(sourceEditGuard(), false, "the leave guard fires without a source edit");
  const card = document.createElement("div");
  card.setAttribute("data-dirty", "1");
  editor.appendChild(card);
  assert.equal(sourceEditGuard(), true, "a dirty source card does not trigger the leave guard");
  card.remove();
  mountSourceDraft(drafts, [], "ini", {}, {});
  assert.equal(sourceEditGuard(), true, "a source draft does not trigger the leave guard");
});
