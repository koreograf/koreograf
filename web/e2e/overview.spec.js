"use strict";

const { expect, test } = require("@playwright/test");
const { password, username } = require("./env");
const { post } = require("./api");

async function noHorizontalScroll(page) {
  await expect.poll(
    () => page.evaluate(() => document.scrollingElement.scrollWidth <= window.innerWidth),
    { message: "the page scrolls sideways" },
  ).toBe(true);
}

async function signIn(page) {
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Sprache" })).toBeVisible();
  await page.getByRole("button", { name: "Sprache" }).click();
  await page.getByRole("option", { name: /English/ }).click();
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Overview" })).toBeVisible();
  await noHorizontalScroll(page);
}

async function teamHeaders(page) {
  const profile = await page.evaluate(async () => (await fetch("/api/v1/me")).json());
  return { "X-Team-Id": profile.teams[0].id };
}

async function createShellTemplate(page, name, script) {
  const headers = await teamHeaders(page);
  const created = await post(page.request, "/api/v1/templates", {
    headers,
    data: { name, description: "", tool: "shell", spec: { script }, prompt_on_launch: false },
  });
  expect(created.status()).toBe(201);
  return created.json();
}

async function launchAndWait(page, template) {
  const headers = await teamHeaders(page);
  const launched = await post(page.request, `/api/v1/templates/${template.id}/launch`, {
    headers,
    data: { extra_vars: {} },
  });
  expect(launched.status()).toBe(202);
  const job = await launched.json();
  await expect.poll(async () => {
    const response = await page.request.get(`/api/v1/jobs/${job.id}`, { headers });
    return (await response.json()).status;
  }, { timeout: 30000 }).toMatch(/^(successful|failed|canceled)$/);
  return job;
}

test("the 1440 overview keeps kpis and five recent jobs in the first viewport", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "overview", "the desktop viewport project only");
  test.setTimeout(120000);
  await signIn(page);
  const suffix = Date.now().toString(36);
  const failing = await createShellTemplate(page, `Overview failing ${suffix}`, "exit 1");
  await launchAndWait(page, failing);
  const passing = await createShellTemplate(page, `Overview passing ${suffix}`, "echo overview-ok");
  for (let index = 0; index < 6; index += 1) await launchAndWait(page, passing);

  await page.goto("/#/");
  await page.reload();
  await expect(page.getByRole("heading", { level: 1, name: "Overview" })).toBeVisible();

  const kpis = page.locator("[data-kpi-row]");
  await expect(kpis).toBeVisible();
  await expect(kpis.locator("[data-kpi]")).toHaveCount(5);
  await expect(page.locator('[data-glance-card="cache"]')).toHaveCount(0);
  await expect(page.locator("[data-attention]")).toBeVisible();
  await expect(page.locator('[data-attention-kind="job"]').first()).toBeVisible();
  await expect(page.locator('[data-kpi="failed-24h"]')).toHaveClass(/bad/);

  const recent = page.locator("[data-recent-activity]");
  await expect(recent.locator("tbody tr")).toHaveCount(5);

  const viewport = page.viewportSize();
  expect(viewport).toEqual({ width: 1440, height: 900 });
  const kpiBox = await kpis.boundingBox();
  expect(kpiBox.y + kpiBox.height, "the kpi row is below the first viewport").toBeLessThanOrEqual(viewport.height);
  const fifth = await recent.locator("tbody tr").nth(4).boundingBox();
  expect(fifth.y + fifth.height, "the fifth recent job is below the first viewport").toBeLessThanOrEqual(viewport.height);

  const columns = await page.locator(".overview-main").evaluate((node) => getComputedStyle(node).gridTemplateColumns);
  expect(columns.trim().split(/\s+/), "the overview does not use two columns").toHaveLength(2);
  await noHorizontalScroll(page);
});

test("the 1280 overview keeps two columns in German and dark", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "overview", "the desktop viewport project only");
  test.setTimeout(60000);
  await signIn(page);
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.evaluate(() => {
    localStorage.setItem("koreograf.lang", "de");
    localStorage.setItem("koreograf.theme", "dark");
  });
  await page.reload();
  await expect(page.getByRole("heading", { level: 1, name: "Übersicht" })).toBeVisible();
  await expect(page.locator("[data-kpi-row] [data-kpi]")).toHaveCount(5);
  const columns = await page.locator(".overview-main").evaluate((node) => getComputedStyle(node).gridTemplateColumns);
  expect(columns.trim().split(/\s+/), "the overview does not use two columns at 1280").toHaveLength(2);
  await noHorizontalScroll(page);
});

test("the 412 overview and its dark theme keep one column without sideways scroll", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "Pixel 9 overview", "the phone viewport project only");
  test.setTimeout(60000);
  await signIn(page);
  const viewport = page.viewportSize();
  expect(viewport).toEqual({ width: 412, height: 923 });
  const failing = await createShellTemplate(page, `Overview phone failing ${Date.now().toString(36)}`, "exit 1");
  await launchAndWait(page, failing);

  await page.evaluate(() => localStorage.setItem("koreograf.theme", "dark"));
  await page.reload();
  await expect(page.getByRole("heading", { level: 1, name: "Overview" })).toBeVisible();

  await expect(page.locator("[data-kpi-row] [data-kpi]")).toHaveCount(5);
  const failed = page.locator('[data-kpi="failed-24h"]');
  await expect(failed).toBeVisible();
  const tone = await failed.evaluate((node) => getComputedStyle(node).backgroundColor);
  expect(tone, "the warning tone has no background in dark mode").not.toBe("rgba(0, 0, 0, 0)");

  const columns = await page.locator(".overview-main").evaluate((node) => getComputedStyle(node).gridTemplateColumns);
  expect(columns.trim().split(/\s+/), "the overview keeps two columns on a phone").toHaveLength(1);
  await expect(page.locator('[data-glance-card="cache"]')).toHaveCount(0);
  await noHorizontalScroll(page);
});
