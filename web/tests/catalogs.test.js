"use strict";

const assert = require("node:assert/strict");
const { readFileSync, readdirSync, statSync } = require("node:fs");
const { join } = require("node:path");
const test = require("node:test");

const { catalog, directories, readCatalog } = require("./catalog.js");

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

function readQuoted(source, start) {
  const quote = source[start];
  let index = start + 1;
  let text = "";
  while (index < source.length) {
    const ch = source[index];
    if (ch === "\\") {
      text += source[index + 1] || "";
      index += 2;
      continue;
    }
    if (ch === quote) return { next: index + 1, text };
    text += ch;
    index += 1;
  }
  return { next: index, text };
}

function skipTemplate(source, start) {
  let index = start + 1;
  while (index < source.length) {
    if (source[index] === "\\") {
      index += 2;
      continue;
    }
    if (source[index] === "`") return index + 1;
    if (source[index] === "$" && source[index + 1] === "{") {
      let depth = 1;
      index += 2;
      while (index < source.length && depth > 0) {
        if (source[index] === '"' || source[index] === "'") {
          index = readQuoted(source, index).next;
          continue;
        }
        if (source[index] === "{") depth += 1;
        else if (source[index] === "}") depth -= 1;
        index += 1;
      }
      continue;
    }
    index += 1;
  }
  return index;
}

function firstArgStrings(source, open) {
  const keys = [];
  let index = open + 1;
  let paren = 0;
  let brace = 0;
  let bracket = 0;
  let seenComma = false;
  while (index < source.length) {
    const ch = source[index];
    if (ch === '"' || ch === "'") {
      const quoted = readQuoted(source, index);
      if (ch === '"' && !seenComma && paren === 0 && brace === 0 && bracket === 0 && quoted.text) {
        keys.push(quoted.text);
      }
      index = quoted.next;
      continue;
    }
    if (ch === "`") {
      index = skipTemplate(source, index);
      continue;
    }
    if (ch === "/" && source[index + 1] === "/") {
      index += 2;
      while (index < source.length && source[index] !== "\n") index += 1;
      continue;
    }
    if (ch === "/" && source[index + 1] === "*") {
      index += 2;
      while (index + 1 < source.length && !(source[index] === "*" && source[index + 1] === "/")) index += 1;
      index = Math.min(index + 2, source.length);
      continue;
    }
    if (ch === "(") paren += 1;
    else if (ch === ")") {
      if (paren > 0) paren -= 1;
      else break;
    } else if (ch === "{") brace += 1;
    else if (ch === "}" && brace > 0) brace -= 1;
    else if (ch === "[") bracket += 1;
    else if (ch === "]" && bracket > 0) bracket -= 1;
    else if (ch === "," && paren === 0 && brace === 0 && bracket === 0) seenComma = true;
    index += 1;
  }
  return keys;
}

function uiKeys(source) {
  const keys = [];
  const pattern = /(?<![\w$])(renderLogin|Error|t|h)\s*\(/g;
  for (const match of source.matchAll(pattern)) {
    keys.push(...firstArgStrings(source, match.index + match[0].length - 1));
  }
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

test("every feature catalog has both languages with the same sorted keys", () => {
  const found = directories();
  assert.ok(found.length > 0, "no catalog directories");
  for (const directory of found) {
    assert.deepEqual(readdirSync(directory).sort(), ["de.json", "en.json"], directory);
    const german = readCatalog(directory, "de");
    const english = readCatalog(directory, "en");
    assert.deepEqual(Object.keys(german), Object.keys(english), directory);
    assert.deepEqual(Object.keys(german), Object.keys(german).sort(), directory);
    assert.deepEqual(Object.keys(english), Object.keys(english).sort(), directory);
  }
});

test("no catalog key appears in more than one file", () => {
  for (const id of ["de", "en"]) {
    assert.doesNotThrow(() => catalog(id), id);
  }
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

test("the UI key scan reads both branches of a ternary", () => {
  assert.deepEqual(uiKeys('h(flag ? "pause_schedule" : "resume_schedule")'), ["pause_schedule", "resume_schedule"]);
  assert.deepEqual(uiKeys('t("plain", { name })'), ["plain"]);
  assert.deepEqual(uiKeys("h(`schedule.day.${day}`)"), []);
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
