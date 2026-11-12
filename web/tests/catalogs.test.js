"use strict";

const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const test = require("node:test");

const catalog = (id) => JSON.parse(readFileSync(join(__dirname, "..", "i18n", `${id}.json`), "utf8"));
const de = catalog("de");
const en = catalog("en");

function placeholders(value) {
  return [...value.matchAll(/\{([a-z_][a-z0-9_]*)\}/gi)].map((match) => match[1]).sort();
}

test("the German and English catalogs share the same keys", () => {
  assert.deepEqual(Object.keys(de).sort(), Object.keys(en).sort());
});

test("every catalog value is a trimmed non-empty string", () => {
  for (const [id, table] of Object.entries({ de, en })) {
    for (const [key, value] of Object.entries(table)) {
      assert.equal(typeof value, "string", `${id}: ${key}`);
      assert.ok(value.trim().length > 0, `${id}: ${key}`);
      assert.equal(value, value.trim(), `${id}: ${key}`);
    }
  }
});

test("both catalogs use the same placeholders", () => {
  for (const key of Object.keys(de)) {
    assert.deepEqual(placeholders(de[key]), placeholders(en[key]), key);
  }
});
