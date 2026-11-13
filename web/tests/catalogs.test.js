"use strict";

const assert = require("node:assert/strict");
const { readFileSync, readdirSync, statSync } = require("node:fs");
const { join } = require("node:path");
const test = require("node:test");

const catalog = (id) => JSON.parse(readFileSync(join(__dirname, "..", "i18n", `${id}.json`), "utf8"));
const de = catalog("de");
const en = catalog("en");

const DYNAMIC_PREFIXES = ["check_", "preflight_runtime_", "schedule.day.", "schedule.short."];

function files(dir, extension) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) out.push(...files(path, extension));
    else if (path.endsWith(extension)) out.push(path);
  }
  return out;
}

function uiKeys(source) {
  const keys = [];
  const pattern = /(?<![\w$])(renderLogin|Error|t|h)\s*\(\s*"((?:\\.|[^"\\])*)"/g;
  for (const match of source.matchAll(pattern)) if (match[2]) keys.push(match[2]);
  return keys;
}

function quotedLiteral(source, key) {
  return ["\"", "'", "`"].some((quote) => source.includes(`${quote}${key}${quote}`));
}


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

test("the catalog keys are sorted", () => {
  for (const [id, table] of Object.entries({ de, en })) {
    assert.deepEqual(Object.keys(table), Object.keys(table).sort(), id);
  }
});

test("every key the UI and the API use exists in the catalogs", () => {
  const web = files(join(__dirname, "..", "js"), ".js").map((path) => readFileSync(path, "utf8")).join("\n");
  const missing = [...new Set(uiKeys(web))].filter((key) => !Object.prototype.hasOwnProperty.call(de, key));
  assert.deepEqual(missing, [], "js keys missing from the catalogs");
});

test("the web sources keep no German copy", () => {
  const hits = [];
  for (const path of files(join(__dirname, "..", "js"), ".js")) {
    const text = readFileSync(path, "utf8");
    text.split("\n").forEach((line, index) => {
      if (/[äöüÄÖÜß]/.test(line)) hits.push(`${path}:${index + 1}: ${line.trim()}`);
    });
  }
  assert.deepEqual(hits, [], "German left in web/js");
});

test("every catalog key is used by the UI or the API", () => {
  const web = files(join(__dirname, "..", "js"), ".js").map((path) => readFileSync(path, "utf8")).join("\n");
  const index = readFileSync(join(__dirname, "..", "index.html"), "utf8");
  const rust = files(join(__dirname, "..", "..", "src"), ".rs").map((path) => readFileSync(path, "utf8")).join("\n");
  const source = `${web}\n${index}\n${rust}`;
  const unused = Object.keys(de).filter((key) =>
    !quotedLiteral(source, key) && !DYNAMIC_PREFIXES.some((prefix) => key.startsWith(prefix))
  );
  assert.deepEqual(unused, [], "catalog keys used by neither the UI nor the API");
});
