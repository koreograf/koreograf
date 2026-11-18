"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { rootTokens, themeTokens, rules, stylesheet, token } = require("./css.js");

const AA_NORMAL_TEXT = 4.5;

function luminance(hex) {
  const value = String(hex).trim().replace(/^#/, "");
  assert.equal(value.length, 6, `expected a six digit hex colour, got ${hex}`);
  const channels = [0, 2, 4].map((index) => parseInt(value.slice(index, index + 2), 16) / 255);
  const linear = channels.map((channel) => (channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4));
  return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
}

function contrast(first, second) {
  const a = luminance(first);
  const b = luminance(second);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

test("muted text meets WCAG AA on light surfaces", () => {
  const tokens = rootTokens(stylesheet());
  const muted = token(tokens, "--muted");
  const surfaces = [
    ["--paper", token(tokens, "--paper")],
    ["--panel", token(tokens, "--panel")],
    ["--line", token(tokens, "--line")],
    ["--accent-soft", token(tokens, "--accent-soft")],
    ["--good-bg", token(tokens, "--good-bg")],
    ["--bad-bg", token(tokens, "--bad-bg")],
    ["--info-bg", token(tokens, "--info-bg")],
    ["--warn-bg", token(tokens, "--warn-bg")],
    ["white", "#ffffff"],
    ["skipped pill", "#f5f5f4"],
  ];
  for (const [name, surface] of surfaces) {
    assert.ok(contrast(muted, surface) >= AA_NORMAL_TEXT, `--muted ${muted} on ${name} ${surface} is below ${AA_NORMAL_TEXT}:1`);
  }
});

test("side muted text meets WCAG AA on dark surfaces", () => {
  const tokens = rootTokens(stylesheet());
  const muted = token(tokens, "--side-muted");
  for (const [name, surface] of [["--side", token(tokens, "--side")], ["sidebar control", "#292524"]]) {
    assert.ok(contrast(muted, surface) >= AA_NORMAL_TEXT, `--side-muted ${muted} on ${name} ${surface} is below ${AA_NORMAL_TEXT}:1`);
  }
});

test("muted text stays quieter than primary text", () => {
  const tokens = rootTokens(stylesheet());
  const paper = token(tokens, "--paper");
  const side = token(tokens, "--side");
  assert.ok(contrast(token(tokens, "--muted"), paper) < contrast(token(tokens, "--ink"), paper));
  assert.ok(contrast(token(tokens, "--side-muted"), side) < contrast(token(tokens, "--side-text"), side));
});

test("dark surfaces do not use the light muted token", () => {
  const offenders = rules(stylesheet())
    .filter(([selector, declarations]) => (selector.split(",").some((part) => part.trim().startsWith(".side")) || selector.includes("pre.vars") || selector.includes(".log")) && declarations.includes("var(--muted)"))
    .map(([selector]) => selector);
  assert.deepEqual(offenders, []);
});

function assertPairs(tokens, pairs) {
  for (const [foreground, background] of pairs) {
    const first = token(tokens, foreground);
    const second = token(tokens, background);
    assert.ok(
      contrast(first, second) >= AA_NORMAL_TEXT,
      `${foreground} ${first} on ${background} ${second} is below ${AA_NORMAL_TEXT}:1`,
    );
  }
}

test("the dark palette defines the colour tokens", () => {
  const tokens = themeTokens(stylesheet(), "dark");
  assert.ok(tokens.length > 0, "the dark palette is missing");
  for (const name of ["--ink", "--paper", "--panel", "--line", "--muted", "--accent", "--side", "--log-bg"]) {
    assert.match(token(tokens, name), /^#[0-9a-f]{6}$/, `${name} is not a dark palette colour`);
  }
});

test("dark muted text meets WCAG AA on dark surfaces", () => {
  const tokens = themeTokens(stylesheet(), "dark");
  assertPairs(tokens, [
    ["--muted", "--paper"],
    ["--muted", "--panel"],
    ["--muted", "--line"],
    ["--muted", "--accent-soft"],
    ["--muted", "--good-bg"],
    ["--muted", "--bad-bg"],
    ["--muted", "--info-bg"],
    ["--muted", "--warn-bg"],
    ["--muted", "--control"],
    ["--muted", "--skipped-bg"],
    ["--muted", "--canvas"],
    ["--muted", "--hover"],
  ]);
});

test("dark primary text meets WCAG AA on dark surfaces", () => {
  const tokens = themeTokens(stylesheet(), "dark");
  assertPairs(tokens, [
    ["--ink", "--paper"],
    ["--ink", "--panel"],
    ["--ink", "--line"],
    ["--ink", "--control"],
    ["--ink", "--accent-soft"],
    ["--ink", "--canvas"],
    ["--ink", "--hover"],
  ]);
});

test("dark sidebar text meets WCAG AA on dark surfaces", () => {
  const tokens = themeTokens(stylesheet(), "dark");
  assertPairs(tokens, [
    ["--side-muted", "--side"],
    ["--side-muted", "--side-control"],
    ["--side-link", "--side"],
    ["--side-text", "--side"],
  ]);
});

test("dark status pills stay readable", () => {
  const tokens = themeTokens(stylesheet(), "dark");
  assertPairs(tokens, [
    ["--good", "--good-bg"],
    ["--bad", "--bad-bg"],
    ["--warn", "--warn-bg"],
    ["--info", "--info-bg"],
    ["--muted", "--skipped-bg"],
    ["--accent", "--accent-soft"],
  ]);
});

test("dark links, primary buttons and logs meet WCAG AA", () => {
  const tokens = themeTokens(stylesheet(), "dark");
  assertPairs(tokens, [
    ["--accent", "--paper"],
    ["--accent", "--panel"],
    ["--accent", "--line"],
    ["--on-accent", "--accent"],
    ["--log-text", "--log-bg"],
    ["--log-key", "--log-bg"],
    ["--log-value", "--log-bg"],
  ]);
});

test("dark muted text stays quieter than primary text", () => {
  const tokens = themeTokens(stylesheet(), "dark");
  const paper = token(tokens, "--paper");
  const side = token(tokens, "--side");
  assert.ok(contrast(token(tokens, "--muted"), paper) < contrast(token(tokens, "--ink"), paper));
  assert.ok(contrast(token(tokens, "--side-muted"), side) < contrast(token(tokens, "--side-text"), side));
});

test("dark status colours stay distinguishable", () => {
  const tokens = themeTokens(stylesheet(), "dark");
  const backgrounds = ["--good-bg", "--bad-bg", "--warn-bg", "--info-bg", "--skipped-bg"].map((name) => token(tokens, name));
  assert.equal(new Set(backgrounds).size, backgrounds.length, "status pills share a background");
  const foregrounds = ["--good", "--bad", "--warn", "--info"].map((name) => token(tokens, name));
  assert.equal(new Set(foregrounds).size, foregrounds.length, "status pills share a text colour");
});
