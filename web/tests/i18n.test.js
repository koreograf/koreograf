"use strict";

const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "core", "i18n.js")).href);

test("a missing English key falls back to German", async () => {
  const { lookup, setCatalog } = await load();
  setCatalog("de", { only_german: "Nur Deutsch" });
  setCatalog("en", {});
  assert.equal(lookup("en", "only_german"), "Nur Deutsch");
  assert.equal(lookup("de", "only_german"), "Nur Deutsch");
});

test("an unknown key stays as the key", async () => {
  const { lookup, setCatalog } = await load();
  setCatalog("de", {});
  setCatalog("en", {});
  assert.equal(lookup("en", "missing.key"), "missing.key");
});

test("the requested language wins over the German fallback", async () => {
  const { lookup, setCatalog } = await load();
  setCatalog("de", { greeting: "Hallo" });
  setCatalog("en", { greeting: "Hello" });
  assert.equal(lookup("en", "greeting"), "Hello");
});

test("{field} and other placeholders are substituted", async () => {
  const { translate, setCatalog } = await load();
  setCatalog("de", { username: "Benutzername", greeting: "Hallo {field}, {count} neu" });
  setCatalog("en", { username: "Username", greeting: "Hello {field}, {count} new" });
  assert.equal(
    translate("en", "greeting", { field: "username", count: 2 }),
    "Hello Username, 2 new"
  );
  assert.equal(
    translate("de", "greeting", { field: "username", count: 2 }),
    "Hallo Benutzername, 2 neu"
  );
});
