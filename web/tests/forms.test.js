"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");
const { rule, stylesheet } = require("./css.js");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "core", "forms.js")).href);

test("the shared description field wraps inside the length limit", async () => {
  const { descriptionField } = await load();
  const markup = descriptionField("check <edge> & certificates\nsecond line");
  assert.match(markup, /class="field description-field"/, "the description is not a shared wrapping field");
  assert.match(markup, /<textarea name="description" maxlength="1000">/, "the description is not a textarea");
  assert.doesNotMatch(markup, /<input/, "the description is still a single-line input");
  assert.match(markup, /check &lt;edge&gt; &amp; certificates\nsecond line<\/textarea>/, "the value is not escaped and kept");
  assert.match(descriptionField(undefined), /<textarea name="description" maxlength="1000"><\/textarea>/, "an empty description is not empty");
});

test("a stored secret keeps the remove checkbox beside its label", async () => {
  const { secretInput } = await load();
  const markup = secretInput("api_key", "API key", "password", true);
  assert.match(markup, /<label class="field check"><input type="checkbox" data-clear="api_key"><span>remove<\/span><\/label>/, "the remove checkbox is not a field check row");
  const fieldCheck = rule(stylesheet(), ".field.check");
  assert.ok(fieldCheck, ".field.check is not styled");
  assert.match(fieldCheck[1], /display:\s*flex/, "the checkbox and label still stack");
  assert.match(fieldCheck[1], /align-items:\s*center/, "the checkbox and label are not aligned");
  assert.doesNotMatch(fieldCheck[1], /display:\s*grid/, "the checkbox row is still a grid");
});
