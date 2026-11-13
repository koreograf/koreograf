"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { read, stylesheet } = require("./css.js");

function codeSpans(markdown) {
  const spans = [];
  let fence = false;
  for (const line of markdown.split("\n")) {
    if (line.trimStart().startsWith("```")) {
      fence = !fence;
      continue;
    }
    if (fence) {
      spans.push(line);
      continue;
    }
    let rest = line;
    while (true) {
      const start = rest.indexOf("`");
      if (start < 0) break;
      const after = rest.slice(start + 1);
      const end = after.indexOf("`");
      if (end < 0) break;
      spans.push(after.slice(0, end));
      rest = after.slice(end + 1);
    }
  }
  return spans;
}

const TOKEN_CHAR = /[a-zA-Z0-9_-]/;

function guideClasses(markdown) {
  const tokens = new Set();
  for (const span of codeSpans(markdown)) {
    const trimmed = span.trim();
    if (trimmed.startsWith(".")) {
      for (const part of trimmed.slice(1).split(".")) {
        const name = part.split(/[^a-zA-Z0-9_-]/)[0];
        if (/^[a-z]/.test(name)) tokens.add(name);
      }
    }
    let rest = span;
    while (true) {
      const start = rest.indexOf('class="');
      if (start < 0) break;
      const after = rest.slice(start + 7);
      const end = after.indexOf('"');
      if (end < 0) break;
      for (const name of after.slice(0, end).split(/\s+/)) {
        if ([...name].every((ch) => TOKEN_CHAR.test(ch))) tokens.add(name);
      }
      rest = after.slice(end);
    }
  }
  return tokens;
}

function guideTokens(markdown) {
  const tokens = new Set();
  for (const span of codeSpans(markdown)) {
    let rest = span;
    while (true) {
      const start = rest.indexOf("--");
      if (start < 0) break;
      const after = rest.slice(start);
      const name = after.split(/[^a-zA-Z0-9-]/)[0];
      if (name.length > 2) tokens.add(name);
      rest = after.slice(Math.max(name.length, 1));
    }
  }
  return tokens;
}

function containsToken(source, token) {
  let rest = source;
  while (true) {
    const index = rest.indexOf(token);
    if (index < 0) return false;
    const before = rest.slice(0, index).slice(-1);
    const after = rest[index + token.length];
    if (!(before && TOKEN_CHAR.test(before)) && !(after && TOKEN_CHAR.test(after))) return true;
    rest = rest.slice(index + token.length);
  }
}

test("style guide class names exist in the stylesheet", () => {
  const guide = read("docs/ui-style-guide.md");
  const css = stylesheet();
  const missing = [...guideClasses(guide)].filter((token) => !containsToken(css, token));
  assert.deepEqual(missing, []);
});

test("style guide custom properties are defined in the stylesheet", () => {
  const guide = read("docs/ui-style-guide.md");
  const css = stylesheet();
  const missing = [...guideTokens(guide)].filter((token) => !css.includes(`${token}:`));
  assert.deepEqual(missing, []);
});

test("agent and contributor docs link the guide", () => {
  for (const path of ["AGENTS.md", "CONTRIBUTING.md"]) {
    assert.match(read(path), /docs\/ui-style-guide\.md/, `${path} must link the UI style guide`);
  }
});
