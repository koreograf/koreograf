"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { rule, stylesheet } = require("./css.js");

test("the edit overlay leaves the sidebar clickable", () => {
  const css = stylesheet();
  const shell = rule(css, ".shell");
  assert.ok(shell, "the shell rule is missing");
  const width = /grid-template-columns:\s*(\d+)px/.exec(shell[1]);
  assert.ok(width, "the sidebar grid width is missing");
  const side = Number(width[1]);
  assert.ok(side > 0);
  const wide = /@media \(min-width: (\d+)px\) \{[\s\S]*?\.overlay\[data-edit-overlay\][^{]*\{([^}]*)\}/.exec(css);
  assert.ok(wide, "the edit overlay still covers the sidebar on wide windows");
  assert.ok(Number(wide[1]) >= 901, "the edit overlay offset applies below the desktop breakpoint");
  const left = /left:\s*(\d+)px/.exec(wide[2]);
  assert.ok(left, "the edit overlay has no left offset");
  assert.equal(Number(left[1]), side, `the edit overlay starts at ${left[1]}px but the sidebar is ${side}px wide`);
  const schedule = /\.overlay#schedule-form[^{]*\{([^}]*)\}/.exec(css);
  assert.ok(schedule, "the schedule overlay still covers the sidebar");
  const scheduleLeft = /left:\s*(\d+)px/.exec(schedule[1]);
  assert.equal(scheduleLeft && Number(scheduleLeft[1]), side, "the schedule overlay starts away from the sidebar edge");
  assert.match(css, /\.overlay \{[^}]*inset:\s*0/, "the normal overlay lost its full-page backdrop");
});
