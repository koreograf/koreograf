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

test("the token expiry cell shows the date and marks an expired token", async () => {
  const { tokenExpiryCell } = await load();
  const i18n = await import(pathToFileURL(join(__dirname, "..", "js", "core", "i18n.js")).href);
  i18n.setCatalog("de", {
    token_is_expired: "Abgelaufen",
    token_never_expires: "Läuft nie ab",
  });
  const live = tokenExpiryCell(
    { expires_at: "2026-01-01T00:00:00Z", expired: false },
    Date.parse("2025-12-31T00:00:00Z")
  );
  assert.doesNotMatch(live, /pill failed/);
  assert.match(live, /2026/);
  const expired = tokenExpiryCell({ expires_at: "2020-01-01T00:00:00Z", expired: true });
  assert.match(expired, /class="pill failed"/);
  assert.match(expired, /Abgelaufen/);
  const past = tokenExpiryCell(
    { expires_at: "2020-01-01T00:00:00Z", expired: false },
    Date.parse("2026-01-01T00:00:00Z")
  );
  assert.match(past, /class="pill failed"/);
  const never = tokenExpiryCell({ expires_at: null, expired: false });
  assert.match(never, /Läuft nie ab/);
  assert.doesNotMatch(never, /class="pill"/);
});
