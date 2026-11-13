"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "backup.js")).href);

test("the backup address includes the OpenTofu state only when asked", async () => {
  const { backupDownloadUrl } = await load();
  assert.equal(backupDownloadUrl(false), "/api/v1/admin/backup");
  assert.equal(backupDownloadUrl(true), "/api/v1/admin/backup?tofu_state=true");
});
