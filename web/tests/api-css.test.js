"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { stylesheet, rule, mediaBlock } = require("./css.js");

test("the shared styles keep the log, table and navigation rules", () => {
  const css = stylesheet();
  for (const needle of [
    ".card.danger-zone",
    ".table th { text-align: left; font-size: 13px;",
    ".log div.fail",
    ".log div.warn",
    ".log-summary",
    ".tabs",
    ".tab-bar",
    ".chips",
    ".chip-entry",
    ".nav-label",
    ".schedule-miss",
    ".switch input:checked",
    ".field-error",
    ".field.required",
    "color: var(--ink)",
  ]) {
    assert.ok(css.includes(needle), needle);
  }
  assert.ok(css.includes("[hidden]"));
  assert.ok(css.includes("display: none !important"));
});

test("the schedule overlay uses the shared wide form layout", () => {
  const css = stylesheet();
  assert.ok(css.includes(".schedule-layout { display: grid; grid-template-columns: minmax(0, 1fr) minmax(15rem, 19rem);"));
  assert.ok(css.includes(".schedule-fields { overflow-y: auto;"));
  assert.ok(css.includes(".schedule-preview { overflow-y: auto;"));
  const narrow = mediaBlock(css, "@media (max-width: 900px)");
  assert.ok(narrow.includes(".schedule-layout { grid-template-columns: 1fr; height: auto; }"));
  assert.ok(narrow.includes(".dialog-footer { display: block; }"));
  assert.ok(narrow.includes(".schedule-pin { display: block;"));
  assert.ok(css.includes('.overlay-card.dialog input:not([type="checkbox"]):not([type="radio"])'));
  assert.ok(!css.includes("#schedule-form input"));
});

test("the edit overlay keeps its tabs and panels", () => {
  const css = stylesheet();
  const tabs = rule(css, ".dialog-tabs");
  assert.ok(tabs, "the dialog tab rule is missing");
  assert.match(tabs[1], /display:\s*flex;/);
  assert.match(tabs[1], /flex:\s*none;/);
  assert.ok(css.includes('.dialog-tab[aria-selected="true"]'));
  assert.ok(css.includes(".dialog-tab.has-error::after"));
  assert.ok(css.includes(".dialog-panel[hidden] { display: none; }"));
  assert.ok(css.includes(".overlay-card, .overlay-card.dialog, .overlay-card.dialog.tabbed { width: 100%;"));
  assert.match(rule(css, ".dialog-tab")[1], /font-weight:\s*600;/);
  assert.doesNotMatch(rule(css, '.dialog-tab[aria-selected="true"]')[1], /font-weight/);
  assert.match(rule(css, ".overlay.tabbed")[1], /align-items:\s*flex-start;/);
  const narrow = mediaBlock(css, "@media (max-width: 600px)");
  assert.ok(narrow.includes(".overlay.tabbed"));
  assert.ok(narrow.includes(".overlay-card.dialog.tabbed"));
});
