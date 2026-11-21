"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

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
