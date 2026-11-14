"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { stylesheet, withoutComments, mediaBlock } = require("./css.js");

function ruleSelectors(css) {
  const selectors = [];
  let prelude = "";
  for (const ch of withoutComments(css)) {
    if (ch === "{") {
      const value = prelude.trim();
      if (value && !value.startsWith("@")) selectors.push(value);
      prelude = "";
    } else if (ch === "}") {
      prelude = "";
    } else if (ch === ";" && prelude.trimStart().startsWith("@")) {
      prelude = "";
    } else {
      prelude += ch;
    }
  }
  return selectors;
}

function boundary(ch) {
  return ch === undefined || !/[a-zA-Z0-9_-]/.test(ch);
}

function mentionsFormControl(selector) {
  return ["input", "select", "textarea"].some((name) => {
    let rest = selector;
    while (true) {
      const index = rest.indexOf(name);
      if (index < 0) return false;
      if (boundary(rest.slice(0, index).slice(-1)) && boundary(rest[index + name.length])) return true;
      rest = rest.slice(index + name.length);
    }
  });
}

test("form control rules do not start with id selectors", () => {
  const offenders = ruleSelectors(stylesheet())
    .flatMap((rule) => rule.split(",").map((part) => part.trim()))
    .filter((selector) => selector.startsWith("#") && mentionsFormControl(selector));
  assert.deepEqual(offenders, []);
});

test("the narrow workflow editor keeps the board inside the page", () => {
  const css = stylesheet();
  const narrow = mediaBlock(css, "@media (max-width: 900px)");
  assert.match(narrow, /\.editor-body\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)/);
  assert.match(narrow, /\.inspector-panel\s*\{[^}]*position:\s*fixed/);
  assert.match(narrow, /\.run-status-bar\s*\{\s*display:\s*flex/);
  const wide = mediaBlock(css, "@media (min-width: 1024px)");
  assert.match(wide, /\.canvas-column \.canvas\s*\{[^}]*min-height:\s*max\(560px,\s*70vh\)/);
  assert.match(css, /\.editor-body\.inspector-collapsed/);
});
