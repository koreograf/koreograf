"use strict";

const { expect, test } = require("@playwright/test");
const { password, username } = require("./env");

async function signIn(page) {
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Sprache" })).toBeVisible();
  await page.getByRole("button", { name: "Sprache" }).click();
  await page.getByRole("option", { name: /English/ }).click();
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Overview" })).toBeVisible();
}

async function launchTofuPlan(page) {
  await page.getByRole("navigation").getByRole("link", { name: "Templates", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Templates" })).toBeVisible();
  await page.getByRole("row", { name: /Infrastruktur-Plan/ }).getByRole("button", { name: "Launch", exact: true }).click();
  await expect(page).toHaveURL(/#\/jobs\/[0-9a-f-]+$/);
  await expect(page.locator(".spread .pill.successful").first()).toBeVisible({ timeout: 60000 });
}

async function noHorizontalScroll(page) {
  await expect.poll(
    () => page.evaluate(() => document.scrollingElement.scrollWidth <= window.innerWidth),
    { message: "the page scrolls sideways" },
  ).toBe(true);
}

test("an OpenTofu plan shows the summary, resources, diagnostics and hidden outputs", async ({ page }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1280, height: 900 });
  await signIn(page);
  await launchTofuPlan(page);

  const tofu = page.locator("#tofu-output");
  await expect(tofu).toBeVisible();
  await expect(page.getByRole("button", { name: "Structured", exact: true })).toHaveClass(/primary/);
  await expect(page.getByRole("button", { name: "Raw log", exact: true })).toBeVisible();
  await expect(page.locator(".tf-count[data-tf-count=add] .tf-count-value")).toHaveText("1");
  await expect(page.locator(".tf-count[data-tf-count=change] .tf-count-value")).toHaveText("1");
  await expect(page.locator(".tf-count[data-tf-count=destroy] .tf-count-value")).toHaveText("1");
  await expect(page.locator("#tf-operation")).toHaveText("Plan");

  const rows = page.locator("#tf-resources tr.tf-resource");
  await expect(rows).toHaveCount(3);
  await expect(page.locator('tr.tf-resource[data-tf-address="terraform_data.web"] .tf-badge.action.create')).toBeVisible();
  await expect(page.locator('tr.tf-resource[data-tf-address="terraform_data.db"] .tf-badge.action.update')).toBeVisible();
  await expect(page.locator('tr.tf-resource[data-tf-address="terraform_data.old"] .tf-badge.action.delete')).toBeVisible();
  await expect(page.locator('tr.tf-resource[data-tf-address="terraform_data.web"] .tf-badge.status.planned')).toBeVisible();

  const diagnostic = page.locator('.tf-diagnostic[data-tf-severity="warning"]');
  await expect(diagnostic).toContainText("Deprecated attribute");
  await expect(diagnostic).toContainText("Use id instead of name.");

  await expect(page.locator('[data-tf-output="message"]')).toContainText("koreograf");
  const secret = page.locator('[data-tf-output="secret"]');
  await expect(secret).toContainText("Sensitive value hidden");
  await expect(secret).not.toContainText("hunter2");
  await expect(page.locator("#tofu-output")).not.toContainText("hunter2");

  await page.getByRole("button", { name: "Raw log", exact: true }).click();
  await expect(tofu).toBeHidden();
  await expect(page.locator("#log")).toBeVisible();
  await expect(page.locator("#log")).toContainText("OpenTofu v1.8.0");
  await expect(page.locator("#log")).toContainText("terraform_data.web: Plan to create");
  await expect(page.locator("#log")).toContainText("Plan: 1 to add, 1 to change, 1 to destroy.");
  await expect(page.locator("#log")).toContainText("Initializing the backend...");
  await expect(page.locator("#log")).not.toContainText('"type":"planned_change"');
  await expect(page.locator("#log")).not.toContainText("hunter2");
  await expect(page.locator("#log-search")).toBeVisible();
  await expect(page.locator("#log-wrap")).toBeVisible();

  await page.reload();
  await expect(page.getByRole("button", { name: "Raw log", exact: true })).toHaveClass(/primary/);
  await expect(page.locator("#log")).toBeVisible();

  await page.getByRole("button", { name: "Structured", exact: true }).click();
  await expect(tofu).toBeVisible();
  await page.reload();
  await expect(page.getByRole("button", { name: "Structured", exact: true })).toHaveClass(/primary/);
  await expect(tofu).toBeVisible();
  await expect(page.locator("#tf-resources tr.tf-resource")).toHaveCount(3);
  await noHorizontalScroll(page);
});

test("the OpenTofu structured view reflows at desktop and phone widths", async ({ page }) => {
  test.setTimeout(120000);
  await signIn(page);
  await launchTofuPlan(page);

  for (const [width, height] of [[1440, 900], [1280, 900], [1024, 700], [412, 923], [360, 732]]) {
    await page.setViewportSize({ width, height });
    await page.evaluate(() => localStorage.setItem("koreograf.job-output-view", "structured"));
    await page.reload();
    await expect(page.locator("#tofu-output")).toBeVisible();
    await expect(page.locator("#tf-operation")).toHaveText("Plan");
    await expect(page.locator("#tf-resources tr.tf-resource")).toHaveCount(3);
    await noHorizontalScroll(page);
    await page.getByRole("button", { name: "Raw log", exact: true }).click();
    await expect(page.locator("#log")).toContainText("Initializing the backend...");
    await noHorizontalScroll(page);
    await page.getByRole("button", { name: "Structured", exact: true }).click();
    await expect(page.locator("#tofu-output")).toBeVisible();
  }
});
