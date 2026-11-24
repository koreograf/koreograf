"use strict";

const { test } = require("@playwright/test");
const { assistantPanelScenario, disableAssistant, startFakeLlm } = require("./assistant-helpers");

test.describe("assistant panel on a phone", () => {
  let fake;

  test.beforeAll(async () => {
    fake = await startFakeLlm();
  });

  test.afterAll(async () => {
    if (fake) await fake.close();
  });

  test.afterEach(async ({ page }) => {
    await disableAssistant(page);
  });

  test("opens as a fullscreen sheet and keeps the conversation across pages", async ({ page }) => {
    test.setTimeout(120000);
    await assistantPanelScenario(page, fake, { phone: true });
  });
});
