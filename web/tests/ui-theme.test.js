"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { read, rootTokens, rule, stylesheet, themeTokens, token } = require("./css.js");

const THEME_FREE_TOKENS = ["--sans", "--serif", "--mono"];

test("the dark palette redefines every colour token", () => {
  const css = stylesheet();
  const root = rootTokens(css).map(([name]) => name).filter((name) => name.startsWith("--") && !THEME_FREE_TOKENS.includes(name));
  const dark = themeTokens(css, "dark").map(([name]) => name).filter((name) => name.startsWith("--"));
  assert.ok(dark.length > 0, "the dark palette is missing");
  assert.deepEqual(root.filter((name) => !dark.includes(name)), [], "colour tokens without a dark value");
  assert.deepEqual(dark.filter((name) => !root.includes(name)), [], "dark-only tokens");
});

test("the dark palette switches the native colour scheme", () => {
  const css = stylesheet();
  assert.match(css, /:root\s*\{[^}]*color-scheme:\s*light;/);
  assert.match(css, /:root\[data-theme="dark"\]\s*\{[^}]*color-scheme:\s*dark;/);
});

test("surfaces and controls read from the theme tokens", () => {
  const css = stylesheet();
  assert.match(css, /body\s*\{[^}]*background:\s*var\(--paper\)/);
  assert.match(rule(css, ".btn")[1], /background:\s*var\(--control\)/);
  assert.match(rule(css, ".btn.primary")[1], /color:\s*var\(--on-accent\)/);
  assert.match(rule(css, ".side select, .main select, .main input, .main textarea")[1], /background:\s*var\(--control\)/);
  assert.match(rule(css, ".side select")[1], /background:\s*var\(--side-control\)/);
  assert.match(rule(css, ".lang-menu")[1], /background:\s*var\(--control\)/);
  assert.match(rule(css, ".chips")[1], /background:\s*var\(--control\)/);
  assert.match(rule(css, ".search")[1], /background:\s*var\(--control\)/);
  assert.match(rule(css, ".weekday-row label")[1], /background:\s*var\(--control\)/);
  assert.match(rule(css, ".overlay")[1], /background:\s*var\(--overlay\)/);
  assert.match(css, /\.canvas\s*\{[^}]*var\(--canvas\)/);
  assert.match(rule(css, ".log")[1], /background:\s*var\(--log-bg\)/);
  assert.match(rule(css, ".log")[1], /color:\s*var\(--log-text\)/);
  assert.match(rule(css, ".switch span")[1], /background:\s*var\(--switch-off\)/);
  assert.doesNotMatch(css, /background:\s*white/);
});

test("the theme control is in the shell and on the sign-in page", () => {
  const app = read("web/js/core/app.js");
  const theme = read("web/js/core/theme.js");
  const main = read("web/js/main.js");
  assert.match(app, /themeSwitch\(\)/, "the shell and the login page have no theme switch");
  assert.equal((app.match(/bindThemeSwitch\(\)/g) || []).length, 2, "both switches must be bound");
  assert.match(theme, /koreograf\.theme/);
  assert.match(theme, /prefers-color-scheme: dark/);
  assert.match(main, /initTheme\(\)/);
});

test("the theme switch labels exist in both catalogs", () => {
  const de = JSON.parse(read("web/i18n/core/de.json"));
  const en = JSON.parse(read("web/i18n/core/en.json"));
  for (const key of ["theme", "theme_system", "theme_light", "theme_dark"]) {
    assert.equal(typeof de[key], "string", `de ${key}`);
    assert.equal(typeof en[key], "string", `en ${key}`);
  }
});

test("the dark log keeps a readable text pair", () => {
  const tokens = themeTokens(stylesheet(), "dark");
  assert.notEqual(token(tokens, "--log-bg"), token(tokens, "--panel"), "the log blends into the dark page");
  assert.notEqual(token(tokens, "--log-text"), token(tokens, "--log-bg"));
});
