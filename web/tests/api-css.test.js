"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { stylesheet, rule, mediaBlock, read } = require("./css.js");

function paddingBox(declarations) {
  const match = /padding:\s*([^;]+);/.exec(declarations);
  if (!match) return null;
  const values = match[1].trim().split(/\s+/).map((value) => Number.parseFloat(value));
  if (values.some((value) => !Number.isFinite(value))) return null;
  const [top, right = top, bottom = top, left = right] = values;
  return { top, right, bottom, left };
}

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

test("the jobs filter bar aligns its controls on one baseline", () => {
  const css = stylesheet();
  const bar = rule(css, ".filter-bar");
  assert.ok(bar, "the filter bar rule is missing");
  assert.match(bar[1], /align-items:\s*end;/);
  assert.match(bar[1], /flex-wrap:\s*wrap;/);
  const search = rule(css, ".filter-bar > .search");
  assert.ok(search, "the filter bar search rule is missing");
  assert.match(search[1], /margin:\s*0;/);
  assert.doesNotMatch(search[1], /margin-bottom/);
  assert.match(rule(css, ".row")[1], /align-items:\s*center;/);
  const jobs = read("web/js/features/jobs.js");
  assert.ok(jobs.includes('<div class="filter-bar">'));
  assert.match(jobs, /class="filter-bar">[\s\S]*?id="job-search"[\s\S]*?id="job-only-failed"/);
});

test("the schedule overlay uses the shared wide form layout", () => {
  const css = stylesheet();
  assert.ok(css.includes(".schedule-layout { display: grid; grid-template-columns: minmax(0, 1fr) minmax(15rem, 19rem);"));
  const fields = rule(css, ".schedule-fields");
  assert.ok(fields, "the schedule fields rule is missing");
  assert.match(fields[1], /overflow-y:\s*auto;/);
  const padding = paddingBox(fields[1]);
  assert.ok(padding, "the schedule fields rule has no padding");
  for (const side of ["top", "right", "bottom", "left"]) {
    assert.ok(padding[side] >= 4, `the schedule fields leave no room for the focus ring on the ${side}`);
  }
  assert.ok(css.includes(".schedule-preview { overflow-y: auto;"));
  const narrow = mediaBlock(css, "@media (max-width: 900px)");
  assert.ok(narrow.includes(".schedule-layout { grid-template-columns: 1fr; height: auto; }"));
  assert.ok(narrow.includes(".dialog-footer { display: block; }"));
  assert.ok(narrow.includes(".schedule-pin { display: block;"));
  assert.ok(css.includes('.overlay-card.dialog input:not([type="checkbox"]):not([type="radio"])'));
  assert.ok(!css.includes("#schedule-form input"));
});

test("scroll containers around controls leave room for the focus ring", () => {
  const css = stylesheet();
  for (const selector of [".dialog-body", ".schedule-preview", ".assistant-thread", ".vault-options"]) {
    const found = rule(css, selector);
    assert.ok(found, `the ${selector} rule is missing`);
    const padding = paddingBox(found[1]);
    assert.ok(padding, `the ${selector} rule has no padding`);
    for (const side of ["top", "right", "bottom", "left"]) {
      assert.ok(padding[side] >= 4, `the ${selector} leaves no room for the focus ring on the ${side}`);
    }
  }
  const narrow = mediaBlock(css, "@media (max-width: 900px)");
  const inspector = /\.inspector-panel \.inspector\s*\{([^}]*)\}/.exec(narrow);
  assert.ok(inspector, "the narrow inspector rule is missing");
  const padding = paddingBox(inspector[1]);
  assert.ok(padding, "the narrow inspector rule has no padding");
  for (const side of ["top", "right", "bottom", "left"]) {
    assert.ok(padding[side] >= 4, `the narrow inspector leaves no room for the focus ring on the ${side}`);
  }
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
