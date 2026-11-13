"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "account.js")).href);

function form(password = "") {
  return {
    display_name: { value: "Ada" },
    email: { value: "ada@example.com" },
    password: { value: password },
  };
}

test("the account payload sends name and email", async () => {
  const { accountPayload } = await load();
  assert.deepEqual(accountPayload(form()), { display_name: "Ada", email: "ada@example.com" });
});

test("the account payload sends a password only when it is filled", async () => {
  const { accountPayload } = await load();
  assert.deepEqual(accountPayload(form("secret123")), {
    display_name: "Ada",
    email: "ada@example.com",
    password: "secret123",
  });
});
