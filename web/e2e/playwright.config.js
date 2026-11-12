"use strict";

const { defineConfig, devices } = require("@playwright/test");
const { dataDir, password, port, root, username } = require("./env");

module.exports = defineConfig({
  testDir: __dirname,
  testMatch: "smoke.spec.js",
  fullyParallel: false,
  workers: 1,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: `sh -c 'rm -rf "${dataDir}" && exec ./target/debug/koreograf'`,
    cwd: root,
    url: `http://127.0.0.1:${port}/healthz`,
    timeout: 120000,
    reuseExistingServer: false,
    env: {
      KOREOGRAF_BIND: `127.0.0.1:${port}`,
      KOREOGRAF_DATA: dataDir,
      KOREOGRAF_WEB: "./web",
      KOREOGRAF_EXAMPLES: "./examples",
      KOREOGRAF_DATABASE_URL: "",
      KOREOGRAF_ADMIN_USER: username,
      KOREOGRAF_ADMIN_PASSWORD: password,
      KOREOGRAF_RUNNER: "local",
      KOREOGRAF_LOCAL_RUNNER_FOR: "admins",
      KOREOGRAF_JOB_TIMEOUT_SECS: "120",
      RUST_LOG: "koreograf=warn",
    },
  },
});
