"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "users.js")).href);

function accountForm(options = {}) {
  return {
    display_name: { value: options.display_name || "Ada" },
    is_admin: { checked: Boolean(options.is_admin) },
    account_disabled: { checked: Boolean(options.disabled) },
    password: { value: options.password || "" },
  };
}

test("the user list filter matches name, username, email and source", async () => {
  const { userMatches } = await load();
  const item = { display_name: "Ada Lovelace", username: "ada", email: "ada@example.com", auth_source: "local" };
  assert.equal(userMatches(item, ""), true);
  assert.equal(userMatches(item, "lovelace"), true);
  assert.equal(userMatches(item, "ada@"), true);
  assert.equal(userMatches(item, "local"), true);
  assert.equal(userMatches(item, "ldap"), false);
});

test("a new user payload carries the first password", async () => {
  const { userCreatePayload } = await load();
  const form = {
    username: { value: "ada" },
    display_name: { value: "Ada" },
    password: { value: "secret123" },
    is_admin: { checked: true },
  };
  assert.deepEqual(userCreatePayload(form), {
    username: "ada",
    display_name: "Ada",
    password: "secret123",
    is_admin: true,
  });
});

test("a user update sends a password only when it is filled", async () => {
  const { userPayload } = await load();
  assert.deepEqual(userPayload(accountForm()), { display_name: "Ada", is_admin: false, disabled: false });
  assert.deepEqual(userPayload(accountForm({ password: "newpass12", disabled: true })), {
    display_name: "Ada",
    is_admin: false,
    disabled: true,
    password: "newpass12",
  });
});
