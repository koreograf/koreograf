"use strict";

const { tmpdir } = require("node:os");
const { join } = require("node:path");

module.exports = {
  root: join(__dirname, "..", ".."),
  port: 43181,
  username: "admin",
  password: "test-admin-pass",
  dataDir: join(tmpdir(), "koreograf-e2e"),
};
