"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "core", "dialog.js")).href);
const tick = () => new Promise((resolve) => setImmediate(resolve));

test("the confirm dialog uses the shared overlay and resolves true on submit", async () => {
  const { confirmDialog } = await load();
  const answer = confirmDialog({ title: "Discard changes?", body: "The form is dirty.", confirmLabel: "Discard", danger: true });
  const overlay = document.querySelector("#confirm-dialog");
  assert.ok(overlay, "the confirmation has no overlay");
  assert.match(overlay.innerHTML, /Discard changes\?/);
  assert.match(overlay.innerHTML, /The form is dirty\./);
  const form = overlay.querySelector(".dialog-form");
  const button = form.querySelector('button[type="submit"]');
  assert.ok(button.classList.contains("danger"), "the confirm button is not a danger button");
  assert.equal(document.activeElement, button, "the confirm button did not receive focus");
  form.dispatchEvent({ type: "submit", preventDefault() {} });
  assert.equal(await answer, true);
  await tick();
  assert.equal(document.querySelector("#confirm-dialog"), null, "the dialog did not close");
});

test("closing the confirm dialog resolves false", async () => {
  const { confirmDialog } = await load();
  const answer = confirmDialog({ title: "Delete?", danger: true });
  const overlay = document.querySelector("#confirm-dialog");
  overlay.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  assert.equal(await answer, false);
  await tick();
  assert.equal(document.querySelector("#confirm-dialog"), null, "the dialog did not close");
});

test("the default title comes from the catalog", async () => {
  const { confirmDialog } = await load();
  const answer = confirmDialog();
  const overlay = document.querySelector("#confirm-dialog");
  assert.match(overlay.innerHTML, /confirm/);
  overlay.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  await answer;
});
