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

async function launchStructuredJob(page) {
  await page.getByRole("navigation").getByRole("link", { name: "Templates", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Templates" })).toBeVisible();
  await page.getByRole("row", { name: /Self-Heal/ }).getByRole("button", { name: "Launch", exact: true }).click();
  await expect(page).toHaveURL(/#\/jobs\/[0-9a-f-]+$/);
  await expect(page.locator(".spread .pill.failed").first()).toBeVisible({ timeout: 60000 });
}

async function noHorizontalScroll(page) {
  await expect.poll(
    () => page.evaluate(() => document.scrollingElement.scrollWidth <= window.innerWidth),
    { message: "the page scrolls sideways" },
  ).toBe(true);
}

test("an Ansible job shows a structured tree, the recap, and the unchanged raw log", async ({ page }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1280, height: 900 });
  await signIn(page);
  await launchStructuredJob(page);

  const tree = page.locator("#structured-output");
  await expect(tree).toBeVisible();
  await expect(page.getByRole("button", { name: "Structured", exact: true })).toHaveClass(/primary/);
  await expect(page.getByRole("button", { name: "Raw log", exact: true })).toBeVisible();
  await expect(page.locator(".st-play-name")).toHaveText("Structured demo");
  await expect(page.locator(".st-task")).toHaveCount(3);
  await expect(page.locator(".st-task").first()).toHaveAttribute("data-status", "failed");
  await expect(page.locator(".st-badge.failed").first()).toBeVisible();
  await expect(page.locator(".st-badge.changed").first()).toBeVisible();

  const recap = page.locator("#log-result");
  await expect(recap).toBeVisible();
  await expect(recap).toContainText("localhost");
  await expect(recap.locator("tr.fail")).toHaveCount(1);

  const failedHost = page.locator('.st-host[data-status="failed"]').first();
  await failedHost.locator("summary").click();
  await expect(failedHost).toContainText("Service is not running");
  await expect(failedHost).toContainText("nginx: down");

  const secretTask = page.locator(".st-task", { hasText: "Read a secret" });
  await secretTask.locator("summary").first().click();
  const secretHost = secretTask.locator(".st-host").first();
  await secretHost.locator("summary").click();
  await expect(secretHost).toContainText("hidden by no_log");

  await page.locator("#st-host").selectOption("localhost");
  await expect(page.locator(".st-task")).toHaveCount(3);
  await page.locator("#st-status").selectOption("failed");
  await expect(page.locator(".st-task")).toHaveCount(1);
  await expect(page.locator(".st-task")).toHaveAttribute("data-status", "failed");
  await page.locator("#st-status").selectOption("");
  await page.locator("#st-host").selectOption("");
  await noHorizontalScroll(page);

  await page.getByRole("button", { name: "Raw log", exact: true }).click();
  await expect(page.locator("#structured-output")).toBeHidden();
  await expect(page.locator("#log")).toBeVisible();
  await expect(page.locator("#log")).toContainText("RAW LOG MARKER");
  await expect(page.locator("#log div.fail").first()).toBeVisible();
  await expect(page.locator("#log-search")).toBeVisible();
  await expect(page.locator("#log-wrap")).toBeVisible();
  await page.locator("#log-search").fill("RAW LOG MARKER");
  await expect(page.locator("#log-search-count")).toHaveText("1 of 1");
  await noHorizontalScroll(page);

  await page.reload();
  await expect(page.getByRole("button", { name: "Raw log", exact: true })).toHaveClass(/primary/);
  await expect(page.locator("#log")).toBeVisible();
  await expect(page.locator("#log")).toContainText("RAW LOG MARKER");
  await expect(page.locator("#structured-output")).toBeHidden();

  await page.getByRole("button", { name: "Structured", exact: true }).click();
  await expect(tree).toBeVisible();
  await page.reload();
  await expect(page.getByRole("button", { name: "Structured", exact: true })).toHaveClass(/primary/);
  await expect(page.locator(".st-play-name")).toHaveText("Structured demo");
});

test("the structured job page reflows at phone and desktop widths", async ({ page }) => {
  test.setTimeout(120000);
  await signIn(page);
  await launchStructuredJob(page);
  await expect(page.locator(".st-play-name")).toHaveText("Structured demo");

  for (const [width, height] of [[1440, 900], [1280, 900], [1024, 700]]) {
    await page.setViewportSize({ width, height });
    await page.reload();
    await expect(page.locator("#structured-output")).toBeVisible();
    await expect(page.locator(".st-play-name")).toHaveText("Structured demo");
    await expect(page.locator("#log-result")).toBeVisible();
    await noHorizontalScroll(page);
    const expanded = page.locator('.st-host[data-status="failed"]').first();
    await expanded.locator("summary").click();
    await expect(expanded).toContainText("Service is not running");
    await noHorizontalScroll(page);
  }

  // The maintainer asked to verify the layout through assertions instead of
  // screenshots: every phone width in both languages and both themes.
  for (const [width, height] of [[1280, 900], [412, 923], [360, 732]]) {
    await page.setViewportSize({ width, height });
    for (const lang of ["de", "en"]) {
      for (const theme of ["light", "dark"]) {
        await page.evaluate(([language, mode]) => {
          localStorage.setItem("koreograf.lang", language);
          localStorage.setItem("koreograf.theme", mode);
          localStorage.setItem("koreograf.job-output-view", "structured");
        }, [lang, theme]);
        await page.reload();
        await expect(page.locator("#structured-output")).toBeVisible();
        await expect(page.locator(".st-play-name")).toHaveText("Structured demo");
        await expect(page.locator("#log-result")).toBeVisible();
        await expect(page.locator("#output-view [data-output-view=structured]")).toHaveClass(/primary/);
        await expect.poll(() => page.evaluate(() => document.documentElement.lang)).toBe(lang);
        await expect.poll(() => page.evaluate(() => document.documentElement.dataset.theme)).toBe(theme);
        await noHorizontalScroll(page);
      }
    }
  }

  await page.evaluate(() => localStorage.setItem("koreograf.lang", "en"));
  await page.setViewportSize({ width: 360, height: 732 });
  await page.reload();
  await expect(page.getByRole("button", { name: "Raw log", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Raw log", exact: true }).click();
  await expect(page.locator("#log")).toBeVisible();
  await expect(page.locator("#log")).toContainText("RAW LOG MARKER");
  await noHorizontalScroll(page);
});
