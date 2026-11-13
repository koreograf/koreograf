"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

test("the edit address round-trips edit, new and tab", async () => {
  const state = await load("core/state.js");
  const { editQuery, editAddress, rememberEditHash, clearEditHash } = await load("core/edit.js");
  location.hash = "#/credentials";
  const edit = editQuery("#/credentials?edit=c1&tab=memberships");
  assert.equal(edit.edit, "c1");
  assert.equal(edit.tab, "memberships");
  assert.equal(edit.create, null);
  const create = editQuery("#/organizations?new=1");
  assert.equal(create.create, "1");
  assert.equal(create.edit, "");
  assert.equal(editQuery("#/projects?edit=p1&tab=source").tab, "source");
  assert.equal(editQuery("#/hooks?edit=h1").edit, "h1");
  assert.equal(editQuery("#/team?new=member").create, "member");

  assert.equal(editAddress({ create: "1" }, "#/credentials"), "#/credentials?new=1");
  assert.equal(editAddress({ create: "1" }, "#/runtime"), "#/runtime?new=1");
  assert.equal(editAddress({ edit: "e1" }, "#/runtime"), "#/runtime?edit=e1");
  assert.equal(editAddress({ create: "member" }, "#/team"), "#/team?new=member");
  assert.equal(editAddress({ create: "1" }, "#/inventories"), "#/inventories?new=1");
  assert.equal(editAddress({ edit: "p1", tab: "source" }, "#/projects"), "#/projects?edit=p1&tab=source");
  assert.equal(editAddress({ edit: "u1", tab: "account" }, "#/users?page=2"), "#/users?page=2&edit=u1&tab=account");
  assert.equal(editAddress({ edit: "t1" }, "#/team?edit=t1&tab=general"), "#/team?edit=t1");
  assert.equal(editAddress({}, "#/team?edit=t1&tab=general"), "#/team");

  let pushed = 0;
  let replaced = 0;
  history.pushState = (_state, _title, url) => { pushed += 1; location.hash = url; };
  history.replaceState = (_state, _title, url) => { replaced += 1; location.hash = url; };
  rememberEditHash({ create: "1" });
  assert.equal(pushed, 1);
  assert.equal(replaced, 0);
  assert.equal(location.hash, "#/credentials?new=1");
  assert.equal(state.shownHash, location.hash);
  rememberEditHash({ edit: "c1", tab: "memberships" }, false);
  assert.equal(pushed, 1);
  assert.equal(replaced, 1);
  assert.equal(location.hash, "#/credentials?edit=c1&tab=memberships");
  clearEditHash();
  assert.equal(replaced, 2);
  assert.equal(location.hash, "#/credentials");
  assert.equal(state.shownHash, "#/credentials");
});

test("the shared edit dialog defaults to a form only when it submits", async () => {
  const { openEditDialog } = await load("core/edit.js");
  const readOnly = openEditDialog({ title: "read only", body: "" });
  assert.notEqual(readOnly.form.tagName, "FORM", "a read-only dialog is a form");
  readOnly.close();
  const explicit = openEditDialog({ title: "read only", body: "", form: true });
  assert.equal(explicit.form.tagName, "FORM", "an explicit form was not forwarded");
  explicit.close();
  const submit = openEditDialog({ title: "edit", body: "", onSubmit: async () => {} });
  assert.equal(submit.form.tagName, "FORM", "a submit dialog did not default to a form");
  submit.close();
});

test("the tabbed dialog renders its tab row before the scrolling body", async () => {  const { openEditDialog } = await load("core/edit.js");
  const dialog = openEditDialog({
    title: "edit",
    tabs: [
      { id: "general", label: "General", body: '<label class="field"><input name="name"></label>' },
      { id: "source", label: "Source", body: '<label class="field"><input name="source"></label>' },
    ],
  });
  const html = dialog.overlay.innerHTML;
  const tabsAt = html.indexOf('class="dialog-tabs"');
  const formAt = html.indexOf('class="dialog-form"');
  assert.ok(tabsAt >= 0, "the tab row is missing");
  assert.ok(formAt > tabsAt, "the tab row sits inside the scrolling body");
  assert.equal(dialog.overlay.querySelector(".dialog-body .dialog-tabs"), null);
  const source = [...dialog.overlay.querySelectorAll("[data-dialog-tab]")].find((tab) => tab.dataset.dialogTab === "source");
  source.dispatchEvent({ type: "click" });
  const panels = [...dialog.overlay.querySelectorAll("[data-dialog-panel]")];
  assert.equal(panels.find((panel) => panel.dataset.dialogPanel === "source").hidden, false);
  assert.equal(panels.find((panel) => panel.dataset.dialogPanel === "general").hidden, true);
  source.dispatchEvent({ type: "keydown", key: "Home" });
  assert.equal(source.getAttribute("aria-selected"), "false", "Home did not move to the first tab");
  const general = panels.find((panel) => panel.dataset.dialogPanel === "general");
  const invalid = general.querySelector("[name=name]");
  dialog.form.dispatchEvent({ type: "invalid", target: invalid });
  assert.equal(source.getAttribute("aria-invalid"), null);
  assert.equal(general.hidden, false);
  dialog.close();
});

test("the old edit pages redirect to the overlay addresses", async () => {
  await load("features/index.js");
  const { matchRoute } = await load("core/router.js");
  const redirects = [
    ["#/credentials/new", "#/credentials?new=1"],
    ["#/credentials/c1", "#/credentials?edit=c1"],
    ["#/organizations/new", "#/organizations?new=1"],
    ["#/organizations/o1", "#/organizations?edit=o1"],
    ["#/team/new", "#/team?new=1"],
    ["#/team/t1", "#/team?edit=t1"],
    ["#/users/new", "#/users?new=1"],
    ["#/users/u1", "#/users?edit=u1"],
    ["#/projects/new", "#/projects?new=1"],
    ["#/inventories/new", "#/inventories?new=1"],
    ["#/hooks/new", "#/hooks?new=1"],
    ["#/hooks/h1", "#/hooks?edit=h1"],
    ["#/environments/new", "#/runtime?new=1"],
    ["#/team/members/new", "#/team?new=member"],
    ["#/templates/new", "#/templates?new=1"],
    ["#/templates/t1", "#/templates?edit=t1"],
    ["#/workflows/new", "#/workflows?new=1"],
  ];
  for (const [old, next] of redirects) {
    const route = matchRoute(old);
    assert.ok(route, `${old} has no route`);
    assert.deepEqual(route.handler(route.params), { redirect: next }, old);
  }
});
