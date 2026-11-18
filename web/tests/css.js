"use strict";

const { readFileSync } = require("node:fs");
const { join } = require("node:path");

function read(relative) {
  return readFileSync(join(__dirname, "..", "..", relative), "utf8");
}

function stylesheet() {
  return readFileSync(join(__dirname, "..", "app.css"), "utf8");
}

const CSS_COMMENT = /\/\*[\s\S]*?\*\//g;

function withoutComments(css) {
  return css.replace(CSS_COMMENT, "");
}

function declarations(block) {
  return block
    .split(";")
    .map((declaration) => declaration.split(":"))
    .filter((parts) => parts.length >= 2)
    .map((parts) => [parts[0].trim(), parts.slice(1).join(":").trim()]);
}

function tokensIn(css, selector) {
  const text = withoutComments(css);
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`${escaped}\\s*\\{([^}]*)\\}`).exec(text);
  return match ? declarations(match[1]) : [];
}

function rootTokens(css) {
  return tokensIn(css, ":root");
}

function themeTokens(css, theme) {
  return tokensIn(css, `:root[data-theme="${theme}"]`);
}

function token(tokens, name) {
  const row = tokens.find(([key]) => key === name);
  return row ? row[1] : "";
}

function rules(css) {
  const text = withoutComments(css);
  const result = [];
  for (const segment of text.split("}")) {
    const open = segment.lastIndexOf("{");
    if (open < 0) continue;
    const selector = segment.slice(0, open).split(/[{}]/).pop().trim();
    if (!selector || selector.startsWith("@")) continue;
    result.push([selector, segment.slice(open + 1)]);
  }
  return result;
}

function rule(css, selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`${escaped}\\s*\\{([^}]*)\\}`).exec(withoutComments(css));
}

function mediaBlock(css, query) {
  const text = withoutComments(css);
  const start = text.indexOf(query);
  if (start < 0) return "";
  let depth = 0;
  for (let index = start; index < text.length; index += 1) {
    if (text[index] === "{") depth += 1;
    else if (text[index] === "}") {
      depth -= 1;
      if (depth === 0) return text.slice(start, index + 1);
    }
  }
  return text.slice(start);
}

module.exports = { read, stylesheet, withoutComments, rootTokens, themeTokens, token, rules, rule, mediaBlock };
