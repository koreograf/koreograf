"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "dashboard.js")).href);

test("the dashboard range keeps the known values and falls back to today", async () => {
  const { dashboardRange } = await load();
  location.hash = "#/?tab=dashboard&days=7";
  assert.equal(dashboardRange(), "7");
  location.hash = "#/?tab=dashboard&days=14";
  assert.equal(dashboardRange(), "14");
  location.hash = "#/?tab=dashboard&days=30";
  assert.equal(dashboardRange(), "1");
});

test("the dashboard address omits the default range", async () => {
  const { dashboardAddress } = await load();
  assert.equal(dashboardAddress("dashboard", "1"), "#/?tab=dashboard");
  assert.equal(dashboardAddress("dashboard", "14"), "#/?tab=dashboard&days=14");
  assert.equal(dashboardAddress("overview", "7"), "#/");
});
