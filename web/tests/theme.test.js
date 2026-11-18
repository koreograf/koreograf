"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "core", "theme.js")).href);

const stored = new Map();
globalThis.localStorage = {
  getItem: (key) => (stored.has(key) ? stored.get(key) : null),
  setItem: (key, value) => stored.set(key, String(value)),
  removeItem: (key) => stored.delete(key),
};

let prefersDark = true;
let changeListener = null;
globalThis.window.matchMedia = (query) => ({
  matches: prefersDark,
  media: query,
  addEventListener: (type, listener) => {
    if (type === "change") changeListener = listener;
  },
  removeEventListener: () => {},
});

function storedTheme() {
  return stored.get("koreograf.theme");
}

test("the theme id falls back to system", async () => {
  const { themeId } = await load();
  assert.equal(themeId("dark"), "dark");
  assert.equal(themeId("light"), "light");
  assert.equal(themeId("system"), "system");
  assert.equal(themeId("sepia"), "system");
  assert.equal(themeId(null), "system");
});

test("the resolved theme follows the system preference only in system mode", async () => {
  const { resolveTheme } = await load();
  assert.equal(resolveTheme("system", true), "dark");
  assert.equal(resolveTheme("system", false), "light");
  assert.equal(resolveTheme("light", true), "light");
  assert.equal(resolveTheme("dark", false), "dark");
});

test("boot follows the system preference when nothing is stored", async () => {
  stored.clear();
  prefersDark = true;
  const { initTheme, themeChoice } = await load();
  initTheme();
  assert.equal(themeChoice(), "system");
  assert.equal(document.documentElement.dataset.theme, "dark");
  prefersDark = false;
  initTheme();
  assert.equal(document.documentElement.dataset.theme, "light");
});

test("a stored theme wins over the system preference", async () => {
  stored.set("koreograf.theme", "dark");
  prefersDark = false;
  const { initTheme } = await load();
  initTheme();
  assert.equal(document.documentElement.dataset.theme, "dark");
  stored.set("koreograf.theme", "light");
  prefersDark = true;
  initTheme();
  assert.equal(document.documentElement.dataset.theme, "light");
});

test("an explicit choice is persisted and applied at once", async () => {
  stored.clear();
  prefersDark = true;
  const { initTheme, setTheme, themeChoice } = await load();
  initTheme();
  setTheme("light");
  assert.equal(themeChoice(), "light");
  assert.equal(storedTheme(), "light");
  assert.equal(document.documentElement.dataset.theme, "light");
  setTheme("dark");
  assert.equal(storedTheme(), "dark");
  assert.equal(document.documentElement.dataset.theme, "dark");
});

test("system mode follows a live system change", async () => {
  stored.clear();
  prefersDark = true;
  const { initTheme, setTheme } = await load();
  initTheme();
  setTheme("system");
  assert.equal(document.documentElement.dataset.theme, "dark");
  prefersDark = false;
  assert.equal(typeof changeListener, "function", "no system change listener was registered");
  changeListener();
  assert.equal(document.documentElement.dataset.theme, "light");
  setTheme("dark");
  prefersDark = true;
  changeListener();
  assert.equal(document.documentElement.dataset.theme, "dark", "an explicit choice followed the system");
});

test("the switch offers system, light and dark", async () => {
  stored.set("koreograf.theme", "light");
  const { initTheme, themeSwitch } = await load();
  initTheme();
  const markup = themeSwitch();
  for (const value of ["system", "light", "dark"]) {
    assert.match(markup, new RegExp(`value="${value}"`), `the ${value} option is missing`);
  }
  assert.match(markup, /value="light" selected/);
  assert.match(markup, /data-theme-select/);
});
