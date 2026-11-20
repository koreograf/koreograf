"use strict";

const { expect, test } = require("@playwright/test");
const { password, username } = require("./env");

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

async function openMenu(page) {
  const toggle = page.getByRole("button", { name: "Menu", exact: true });
  if ((await toggle.getAttribute("aria-expanded")) !== "true") await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
}

async function goTo(page, label) {
  await openMenu(page);
  await page.getByRole("navigation").getByRole("link", { name: label, exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: label })).toBeVisible();
  await noHorizontalScroll(page);
}

async function teamHeaders(page) {
  const profile = await page.evaluate(async () => (await fetch("/api/v1/me")).json());
  return { "X-Team-Id": profile.teams[0].id };
}

async function createShellTemplate(page, name, script, prompt = false) {
  const created = await page.request.post("/api/v1/templates", {
    headers: await teamHeaders(page),
    data: { name, tool: "shell", spec: { script }, prompt_on_launch: prompt },
  });
  expect(created.status()).toBe(201);
  return created.json();
}

test("signs in, opens a labelled template list and launches from a fullscreen dialog", async ({ page }) => {
  const suffix = `${test.info().project.name}-${Date.now()}`.replace(/\s+/g, "-");
  await signIn(page);
  const template = await createShellTemplate(page, `Mobile prompt ${suffix}`, "#!/bin/sh\necho \"mobile prompt ok\"\n", true);

  await goTo(page, "Templates");
  const row = page.locator("#template-table tbody tr").filter({ hasText: template.name }).first();
  await expect(row).toBeVisible();
  await expect(row.locator('td[data-label="Name"]')).toHaveCount(1);
  await expect(row.locator('td[data-label="Tool"]')).toContainText("shell");

  await row.getByRole("button", { name: "Launch", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "Launch", exact: true })).toBeVisible();
  await expect(dialog).toHaveClass(/overlay-card/);
  const viewport = page.viewportSize();
  const box = await dialog.boundingBox();
  expect(Math.round(box.width)).toBeGreaterThanOrEqual(viewport.width - 1);
  expect(Math.round(box.height)).toBeGreaterThanOrEqual(viewport.height - 1);
  await noHorizontalScroll(page);

  await dialog.getByRole("button", { name: "Launch", exact: true }).click();
  await expect(page).toHaveURL(/#\/jobs\/[0-9a-f-]+$/);
  await expect(page.getByText("mobile prompt ok", { exact: true })).toBeVisible({ timeout: 30000 });
  await noHorizontalScroll(page);
});

test("watches a wrapped job log, cancels the job and relaunches it", async ({ page }) => {
  test.setTimeout(60000);
  const suffix = `${test.info().project.name}-${Date.now()}`.replace(/\s+/g, "-");
  await signIn(page);
  const template = await createShellTemplate(page, `Mobile slumber ${suffix}`, "#!/bin/sh\necho \"mobile start\"\nsleep 30\necho \"mobile done\"\n");

  await goTo(page, "Templates");
  const row = page.locator("#template-table tbody tr").filter({ hasText: template.name }).first();
  await expect(row).toBeVisible();
  await row.getByRole("button", { name: "Launch", exact: true }).click();
  await expect(page).toHaveURL(/#\/jobs\/[0-9a-f-]+$/);
  await expect(page.getByText("mobile start", { exact: true })).toBeVisible({ timeout: 30000 });
  await expect(page.locator("#log-wrap")).toBeChecked();
  await expect(page.locator("#log")).toHaveClass(/wrap/);
  const firstLine = page.locator("#log div").first();
  expect(await firstLine.evaluate((element) => getComputedStyle(element).whiteSpace)).toBe("pre-wrap");
  await noHorizontalScroll(page);

  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.locator(".main .pill").first()).toHaveText("Canceled", { timeout: 30000 });

  await page.reload();
  const relaunch = page.getByRole("button", { name: "Relaunch", exact: true });
  await expect(relaunch).toBeVisible();
  await relaunch.click();
  const confirm = page.getByRole("dialog");
  await expect(confirm.getByRole("heading", { name: "Relaunch", exact: true })).toBeVisible();
  await confirm.getByRole("button", { name: "Relaunch", exact: true }).click();
  await expect(page.getByRole("button", { name: "Cancel", exact: true })).toBeVisible({ timeout: 30000 });
  await noHorizontalScroll(page);

  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.locator(".main .pill").first()).toHaveText("Canceled", { timeout: 30000 });
});

test("shows the dashboard chart counts without a hover", async ({ page }) => {
  await signIn(page);
  const headers = await teamHeaders(page);
  const templates = await (await page.request.get("/api/v1/templates", { headers })).json();
  const health = templates.find((item) => item.name === "Healthcheck");
  expect(health).toBeTruthy();
  const launched = await page.request.post(`/api/v1/templates/${health.id}/launch`, { headers, data: { extra_vars: {} } });
  expect(launched.status()).toBe(202);
  const job = await launched.json();
  await expect.poll(async () => {
    const response = await page.request.get(`/api/v1/jobs/${job.id}`, { headers });
    return (await response.json()).status;
  }, { timeout: 30000 }).toMatch(/^(successful|failed|canceled)$/);

  await page.getByRole("tab", { name: "Dashboard", exact: true }).click();
  await expect(page.getByRole("heading", { level: 2, name: "Jobs finished per day" })).toBeVisible();
  const counts = page.locator(".chart-day-total");
  await expect(counts.first()).toBeVisible();
  expect(await counts.allTextContents()).not.toHaveLength(0);
  expect((await counts.allTextContents()).every((text) => /^\d+$/.test(text.trim()))).toBe(true);
  await noHorizontalScroll(page);
});

test("keeps the workflow editor view-only with a desktop note", async ({ page }) => {
  await signIn(page);
  await goTo(page, "Workflows");
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  await expect(page.locator("#wf-desktop-note")).toBeVisible();
  await expect(page.locator("#wf-desktop-note")).toContainText("desktop");
  await expect(page.getByRole("button", { name: "Save", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Launch", exact: true })).toHaveCount(0);
  await expect(page.locator("#wf-name")).toBeDisabled();
  await noHorizontalScroll(page);

  const node = page.locator('.wf-node[data-id="check"]');
  await node.scrollIntoViewIfNeeded();
  await node.click();
  await expect(page.locator("#inspector")).toContainText("Healthcheck");
  await noHorizontalScroll(page);
});
