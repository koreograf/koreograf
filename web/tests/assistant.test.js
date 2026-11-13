"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "assistant.js")).href);

test("the assistant history keeps the twenty turns before the new message", async () => {
  const { assistantHistory } = await load();
  const turns = Array.from({ length: 25 }, (_, index) => ({ role: "user", content: String(index) }));
  const history = assistantHistory(turns);
  assert.equal(history.length, 20);
  assert.equal(history[0].content, "4");
  assert.equal(history[19].content, "23");
});

test("an assistant action maps to its confirmation copy", async () => {
  const { assistantActionKey } = await load();
  assert.equal(assistantActionKey("launch_template"), "assistant_confirm_launch_template");
  assert.equal(assistantActionKey("sync_inventory"), "assistant_confirm_sync_inventory");
  assert.equal(assistantActionKey("unknown"), "assistant_confirmation");
});
