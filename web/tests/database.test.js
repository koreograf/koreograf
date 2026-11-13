"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "database.js")).href);

function form(options = {}) {
  return {
    db_host: { value: options.host || " db.example.com " },
    db_port: { value: options.port === undefined ? "5433" : options.port },
    db_database: { value: " koreograf " },
    db_user: { value: " koreograf " },
    db_password: { value: "secret" },
    db_sslmode: { value: "verify-full" },
    db_ca_file: { value: options.ca_file === undefined ? " /etc/koreograf/postgres-ca.pem " : options.ca_file },
  };
}

test("a migration target trims the fields and keeps the port", async () => {
  const { databaseTargetBody } = await load();
  assert.deepEqual(databaseTargetBody(form()), {
    host: "db.example.com",
    port: 5433,
    database: "koreograf",
    user: "koreograf",
    password: "secret",
    sslmode: "verify-full",
    ca_file: "/etc/koreograf/postgres-ca.pem",
  });
});

test("a migration target falls back to the default port and no CA file", async () => {
  const { databaseTargetBody } = await load();
  const body = databaseTargetBody(form({ port: "", ca_file: "" }));
  assert.equal(body.port, 5432);
  assert.equal(body.ca_file, null);
});
