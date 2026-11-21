"use strict";

const fs = require("node:fs");
const { join } = require("node:path");
const { expect, test } = require("@playwright/test");
const { dataDir, password, username } = require("./env");
const { post } = require("./api");

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

async function launchBulkJob(page, hosts) {
  const me = await page.request.get("/api/v1/me");
  const team = (await me.json()).teams[0].id;
  const headers = { "X-Team-Id": team };
  const templates = await (await page.request.get("/api/v1/templates", { headers })).json();
  const demo = templates.find((item) => item.name === "Self-Heal");
  expect(demo).toBeTruthy();
  const lines = [];
  for (let index = 1; index <= hosts; index += 1) {
    lines.push(`bulk-${String(index).padStart(4, "0")} ansible_connection=local`);
  }
  const inventoryResponse = await post(page.request, "/api/v1/inventories", {
    headers,
    data: { name: "Bulk hosts", body: `[all]\n${lines.join("\n")}\n` },
  });
  expect(inventoryResponse.ok()).toBeTruthy();
  const inventory = await inventoryResponse.json();
  const templateResponse = await post(page.request, "/api/v1/templates", {
    headers,
    data: {
      name: "Bulk hosts",
      tool: "ansible",
      project_id: demo.project_id,
      inventory_id: inventory.id,
      entrypoint: demo.entrypoint,
      no_environment: true,
    },
  });
  expect(templateResponse.ok()).toBeTruthy();
  const template = await templateResponse.json();
  const launchResponse = await post(page.request, `/api/v1/templates/${template.id}/launch`, {
    headers,
    data: {},
  });
  expect(launchResponse.ok()).toBeTruthy();
  const launched = await launchResponse.json();
  let job = launched;
  for (let attempt = 0; attempt < 240; attempt += 1) {
    if (["successful", "failed", "canceled"].includes(job.status)) break;
    await page.waitForTimeout(250);
    job = await (await page.request.get(`/api/v1/jobs/${launched.id}`, { headers })).json();
  }
  expect(job.status).toBe("failed");
  return job.id;
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
  await expect(page.locator("#st-recap-total")).toContainText("1");
  await expect(page.locator(".st-count.failed .st-count-value")).toHaveText("1");
  await expect(page.locator('[data-st-select-host="localhost"]')).toBeVisible();
  await expect(page.locator("#host-table tbody tr")).toHaveCount(1);

  const failedHost = page.locator('.st-host[data-status="failed"]').first();
  await failedHost.locator("summary").click();
  await expect(failedHost).toContainText("Service is not running");
  await expect(failedHost).toContainText("nginx: down");

  const secretTask = page.locator(".st-task", { hasText: "Read a secret" });
  await secretTask.locator("summary").first().click();
  const secretHost = secretTask.locator(".st-host").first();
  await secretHost.locator("summary").click();
  await expect(secretHost).toContainText("hidden by no_log");

  await page.locator("#st-host").fill("localhost");
  await expect(page.locator(".st-task")).toHaveCount(3);
  await page.locator("#st-status").selectOption("failed");
  await expect(page.locator(".st-task")).toHaveCount(1);
  await expect(page.locator(".st-task")).toHaveAttribute("data-status", "failed");
  await page.locator("#st-status").selectOption("");
  await page.locator("#st-host").fill("");
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

test("a job with 5000 hosts keeps the summary complete and the DOM bounded", async ({ page }) => {
  test.setTimeout(240000);
  await page.setViewportSize({ width: 1280, height: 900 });
  await signIn(page);
  const id = await launchBulkJob(page, 5000);
  await page.evaluate(() => localStorage.setItem("koreograf.job-output-view", "structured"));
  await page.goto(`/#/jobs/${id}`);

  const structured = page.locator("#structured-output");
  await expect(structured).toBeVisible({ timeout: 60000 });
  await expect(page.locator("#st-recap-total")).toContainText("5000");
  await expect(structured).toHaveAttribute("data-recap-complete", "true");
  await expect(page.locator(".st-count.unreachable .st-count-value")).toHaveText("1");
  await expect(page.locator(".st-count.failed .st-count-value")).toHaveText("1");

  const rows = page.locator("#host-table tbody tr");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toHaveAttribute("data-status", "unreachable");
  await expect(rows.nth(0)).toContainText("bulk-4999");
  await expect(rows.nth(1)).toHaveAttribute("data-status", "failed");
  await expect(rows.nth(1)).toContainText("bulk-5000");

  await page.locator(".st-count.unreachable").click();
  await expect(page.locator("#host-status")).toHaveValue("unreachable");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText("bulk-4999");
  await expect(page.locator(".st-task")).toHaveCount(0);
  await page.locator(".st-count.unreachable").click();
  await expect(page.locator("#host-status")).toHaveValue("all");

  await page.locator("#host-status").selectOption("all");
  await expect(rows).toHaveCount(50);
  await expect(page.locator("#host-total")).toContainText("50 of 5000");
  expect(await page.locator(".st-host").count()).toBeLessThanOrEqual(50);

  await page.locator("#host-search").fill("bulk-5000");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText("bulk-5000");
  await page.locator("#host-search").fill("bulk-0001");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText("bulk-0001");
  await rows.first().locator("[data-st-select-host]").click();
  await expect(page.locator("#st-host")).toHaveValue("bulk-0001");
  await expect(page.locator("#st-host-hints")).toBeHidden();
  await expect(page.locator(".st-task")).toHaveCount(1);
  await expect(page.locator(".st-host")).toHaveCount(1);
  await expect(page.locator(".st-host")).toContainText("bulk-0001");

  const detailUrl = "**/api/v1/jobs/*/ansible?host=*";
  await page.route(detailUrl, async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 1500));
    await route.continue();
  });
  await page.locator("#host-search").fill("bulk-4500");
  await expect(page.locator('[data-st-select-host="bulk-4500"]')).toBeVisible();
  await page.locator('[data-st-select-host="bulk-4500"]').click();
  await expect(page.locator(".st-detail-note")).toContainText("Loading task detail for bulk-4500", { timeout: 1000 });
  await expect(page.locator(".st-task")).toHaveCount(1);
  await expect(page.locator(".st-host")).toHaveCount(1);
  await expect(page.locator(".st-host")).toContainText("bulk-4500");
  await expect(page.locator(".st-host")).toContainText("bulk ping bulk-4500");
  await page.unroute(detailUrl);

  const renderMs = Number(await structured.getAttribute("data-render-ms"));
  expect(renderMs).toBeGreaterThan(0);
  expect(renderMs).toBeLessThan(1000);

  await page.getByRole("button", { name: "Raw log", exact: true }).click();
  await expect(structured).toBeHidden();
  await expect(page.locator("#log")).toContainText("RAW LOG MARKER");
  await expect(page.locator("#log")).toContainText("PLAY RECAP");
  await noHorizontalScroll(page);

  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "Structured", exact: true }).click();
  await expect(structured).toBeVisible();
  await expect(page.locator("#st-recap-total")).toContainText("5000");
  await noHorizontalScroll(page);

  fs.rmSync(join(dataDir, "jobs", id, "ansible-events.jsonl"), { force: true });
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.reload();
  await expect(structured).toBeVisible();
  await page.locator("#host-status").selectOption("all");
  await page.locator("#host-search").fill("bulk-4500");
  await expect(page.locator('[data-st-select-host="bulk-4500"]')).toBeVisible();
  await page.locator('[data-st-select-host="bulk-4500"]').click();
  await expect(page.locator(".st-detail-note")).toContainText("no longer available");
  await expect(page.locator(".st-task")).toHaveCount(0);
  await noHorizontalScroll(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await noHorizontalScroll(page);
});
