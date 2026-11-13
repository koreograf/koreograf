"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { rule, stylesheet } = require("./css.js");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "core", "dialog.js")).href);

function element(attributes = {}) {
  const node = document.createElement(attributes.tag || "button");
  for (const [name, value] of Object.entries(attributes)) {
    if (name !== "tag") node.setAttribute(name, value);
  }
  return node;
}

test("overlays restore focus to the trigger", async () => {
  const { focusOpener, focusReturn, makeModal } = await load();
  const trigger = element({ href: "#/templates?edit=t1", tag: "a" });
  const replacement = element({ href: "#/templates?edit=t1", tag: "a" });
  document.body.appendChild(trigger);
  document.body.appendChild(replacement);
  const main = element({ id: "main", tag: "main" });
  document.body.appendChild(main);
  document.activeElement = document.body;
  document.dispatchEvent({ type: "click", target: trigger });
  assert.equal(focusOpener(), trigger, "focusOpener ignores the last trigger when BODY has focus");

  trigger.focus();
  const overlay = element();
  overlay.innerHTML = '<div class="overlay-card"><input name="title"></div>';
  const opened = makeModal(overlay, async () => true);
  assert.equal(document.activeElement, overlay.querySelector("input"), "the overlay did not focus its first field");
  overlay.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  await opened();
  assert.equal(document.activeElement, trigger, "the overlay did not restore the opener");

  trigger.focus();
  focusReturn(trigger);
  assert.equal(document.activeElement, trigger, "focusReturn does not focus a connected opener");
  trigger.remove();
  focusReturn(trigger);
  assert.equal(document.activeElement, replacement, "focusReturn does not focus the matching control after a rerender");

  document.activeElement = main;
  assert.equal(focusOpener(), main, "focusOpener ignores a focused control");
  document.dispatchEvent({ type: "click", target: trigger });
  const skip = element({ tag: "a" });
  skip.className = "skip-link";
  document.activeElement = skip;
  assert.equal(focusOpener(), trigger, "focusOpener keeps the skip link as the opener");
});

test("the login language switch is placed in the card", () => {
  const css = stylesheet();
  const switchRule = rule(css, ".login-card > .lang-switch");
  assert.ok(switchRule, "the login language switch has no placement rule");
  assert.match(switchRule[1], /position:\s*absolute/);
  assert.match(switchRule[1], /top:\s*28px/);
  assert.match(switchRule[1], /right:\s*28px/);
  assert.match(css, /\.login-card\s*\{[^}]*position:\s*relative/, "the login card is not the placement container");
});

test("action targets meet the minimum size", () => {
  const css = stylesheet();
  for (const selector of [".btn", ".btn.compact", ".table a"]) {
    const found = rule(css, selector);
    assert.ok(found, `the ${selector} rule is missing`);
    assert.match(found[1], /min-width:\s*24px/, `${selector} may shrink below 24px`);
    assert.match(found[1], /min-height:\s*24px/, `${selector} may shrink below 24px`);
  }
  const coarse = /@media \(pointer: coarse\)\s*\{([\s\S]*?)\n\}/.exec(css);
  assert.ok(coarse, "the coarse pointer rule is missing");
  assert.match(coarse[1], /min-height:\s*44px/);
  assert.match(coarse[1], /min-width:\s*44px/);
  for (const selector of [".btn", ".table a", ".lang-button", ".dialog-close"]) {
    assert.ok(coarse[1].includes(selector), `coarse pointer targets miss ${selector}`);
  }
});
