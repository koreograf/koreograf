"use strict";

const { readFileSync, readdirSync, statSync } = require("node:fs");
const { join } = require("node:path");

const ROOT = join(__dirname, "..", "i18n");

function directories() {
  return readdirSync(ROOT)
    .map((name) => join(ROOT, name))
    .filter((path) => statSync(path).isDirectory())
    .sort();
}

function readCatalog(directory, id) {
  return JSON.parse(readFileSync(join(directory, `${id}.json`), "utf8"));
}

function catalog(id) {
  const merged = {};
  const sources = {};
  for (const directory of directories()) {
    const path = join(directory, `${id}.json`);
    let entries;
    try {
      entries = JSON.parse(readFileSync(path, "utf8"));
    } catch (error) {
      if (error.code === "ENOENT") continue;
      throw error;
    }
    for (const [key, value] of Object.entries(entries)) {
      if (Object.prototype.hasOwnProperty.call(sources, key)) {
        throw new Error(`duplicate catalog key ${key} in ${sources[key]} and ${path}`);
      }
      merged[key] = value;
      sources[key] = path;
    }
  }
  return Object.fromEntries(Object.keys(merged).sort().map((key) => [key, merged[key]]));
}

module.exports = { catalog, directories, readCatalog };
