"use strict";

const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { defineConfig, devices } = require("@playwright/test");
const { dataDir, password, port, root, username } = require("./env");

module.exports = defineConfig({
  testDir: __dirname,
  testMatch: /(smoke|mobile|overview|structured|tofu|shell|assistant|assistant-phone|relaunch)\.spec\.js/,
  timeout: 30000,
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
  projects: [
    { name: "chromium", testMatch: "smoke.spec.js", use: { ...devices["Desktop Chrome"] } },
    { name: "structured", testMatch: "structured.spec.js", use: { ...devices["Desktop Chrome"] } },
    { name: "tofu", testMatch: "tofu.spec.js", use: { ...devices["Desktop Chrome"] } },
    { name: "shell", testMatch: "shell.spec.js", use: { ...devices["Desktop Chrome"] } },
    { name: "overview", testMatch: "overview.spec.js", use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 } } },
    { name: "Pixel 9 overview", testMatch: "overview.spec.js", use: { ...devices["Pixel 9"], viewport: { width: 412, height: 923 } } },
    { name: "iPhone 13", testMatch: "mobile.spec.js", use: { ...devices["iPhone 13"], browserName: "chromium" } },
    { name: "Pixel 7", testMatch: "mobile.spec.js", use: { ...devices["Pixel 7"] } },
    { name: "assistant", testMatch: "assistant.spec.js", use: { ...devices["Desktop Chrome"] } },
    { name: "Pixel 9 assistant", testMatch: "assistant-phone.spec.js", use: { ...devices["Pixel 9"] } },
    { name: "relaunch", testMatch: "relaunch.spec.js", use: { ...devices["Desktop Chrome"] } },
    { name: "Pixel 8 relaunch", testMatch: "relaunch.spec.js", use: { ...devices["Pixel 8"] } },
  ],
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
      KOREOGRAF_PROJECT_ROOTS: join(tmpdir(), "koreograf-e2e-projects"),
      KOREOGRAF_RUNNER: "local",
      KOREOGRAF_LOCAL_RUNNER_FOR: "admins",
      KOREOGRAF_JOB_TIMEOUT_SECS: "120",
      RUST_LOG: "koreograf=warn",
      PATH: `${join(__dirname, "fake-bin")}:${process.env.PATH || ""}`,
    },
  },
});
