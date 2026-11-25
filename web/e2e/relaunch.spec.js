"use strict";

const { expect, test } = require("@playwright/test");
const { password, username } = require("./env");
const { post } = require("./api");

async function signIn(page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Sprache" }).click();
  await page.getByRole("option", { name: /English/ }).click();
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();
}

async function goTo(page, label) {
  const menu = page.getByRole("button", { name: "Menu", exact: true });
  if (await menu.isVisible()) await menu.click();
  await page.getByRole("navigation").getByRole("link", { name: label, exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: label })).toBeVisible();
}

async function teamHeaders(page) {
  const profile = await page.evaluate(async () => (await fetch("/api/v1/me")).json());
  return { "X-Team-Id": profile.teams[0].id };
}

async function createShellTemplate(page, name, script, prompt = false) {
  const created = await post(page.request, "/api/v1/templates", {
    headers: await teamHeaders(page),
    data: {
      name,
      tool: "shell",
      spec: { script },
      extra_vars: { stored: "yes" },
      prompt_on_launch: prompt,
    },
  });
  expect(created.status()).toBe(201);
  return created.json();
}

async function launchAndWait(page, templateId) {
  const headers = await teamHeaders(page);
  const launched = await post(page.request, `/api/v1/templates/${templateId}/launch`, {
    headers,
    data: { extra_vars: {} },
  });
  expect(launched.status()).toBe(202);
  const job = await launched.json();
  let finished = job;
  await expect.poll(async () => {
    const response = await page.request.get(`/api/v1/jobs/${job.id}`, { headers });
    finished = await response.json();
    return finished.status;
  }, { timeout: 30000 }).toMatch(/^(successful|failed|canceled)$/);
  return finished;
}

async function waitFinished(page, id) {
  const headers = await teamHeaders(page);
  await expect.poll(async () => {
    const response = await page.request.get(`/api/v1/jobs/${id}`, { headers });
    return (await response.json()).status;
  }, { timeout: 30000 }).toMatch(/^(successful|failed|canceled)$/);
}

test("relaunches a successful job from the list and shows the source link", async ({ page }) => {
  test.setTimeout(60000);
  const suffix = `${test.info().project.name}-${Date.now()}`.replace(/\s+/g, "-");
  await signIn(page);
  const template = await createShellTemplate(page, `Relaunch green ${suffix}`, "#!/bin/sh\necho \"relaunch green\"\n");
  const source = await launchAndWait(page, template.id);
  expect(source.status).toBe("successful");

  await goTo(page, "Jobs");
  await page.locator("#job-search").fill(template.name);
  const row = page.locator("#job-table tbody tr").filter({ hasText: template.name }).first();
  await expect(row).toBeVisible();
  await expect(row.getByRole("button", { name: "Relaunch", exact: true })).toBeVisible();
  await expect(row.getByRole("button", { name: "Failed hosts only", exact: true })).toHaveCount(0);

  await row.getByRole("button", { name: "Relaunch", exact: true }).click();
  const confirm = page.getByRole("dialog");
  await expect(confirm.getByRole("heading", { name: "Relaunch", exact: true })).toBeVisible();
  await confirm.getByRole("button", { name: "Relaunch", exact: true }).click();
  await expect(page).toHaveURL(/#\/jobs\/[0-9a-f-]+$/);
  const copyId = page.url().split("/").pop();
  expect(copyId).not.toBe(source.id);
  await waitFinished(page, copyId);

  await page.getByRole("button", { name: "Parameters", exact: true }).click();
  const parameters = page.locator("#panel-parameters");
  await expect(parameters).toContainText("Source job");
  await expect(parameters.locator(`a[href="#/jobs/${source.id}"]`)).toBeVisible();

  await page.goto(`/#/jobs/${source.id}`);
  await expect(page.getByRole("button", { name: "Relaunch", exact: true })).toBeVisible();
});

test("relaunches a successful job with prompted changes", async ({ page }) => {
  test.setTimeout(60000);
  const suffix = `${test.info().project.name}-${Date.now()}`.replace(/\s+/g, "-");
  await signIn(page);
  const template = await createShellTemplate(
    page,
    `Relaunch prompt ${suffix}`,
    "#!/bin/sh\necho \"relaunch prompt\"\n",
    true,
  );
  const source = await launchAndWait(page, template.id);
  expect(source.status).toBe("successful");

  await goTo(page, "Jobs");
  await page.locator("#job-search").fill(template.name);
  const row = page.locator("#job-table tbody tr").filter({ hasText: template.name }).first();
  await expect(row.getByRole("button", { name: "Relaunch with changes", exact: true })).toBeVisible();
  await row.getByRole("button", { name: "Relaunch with changes", exact: true }).click();
  const dialog = page.locator("#launch-prompt");
  await expect(dialog.getByRole("heading", { name: "Relaunch with changes", exact: true })).toBeVisible();
  const extra = dialog.locator('textarea[name="extra_vars"]');
  await expect(extra).toHaveValue(/"stored": "yes"/);
  await extra.fill(JSON.stringify({ stored: "yes", changed: "yes" }, null, 2));
  await dialog.getByRole("button", { name: "Relaunch", exact: true }).click();
  await expect(page).toHaveURL(/#\/jobs\/[0-9a-f-]+$/);
  const copyId = page.url().split("/").pop();
  expect(copyId).not.toBe(source.id);
  await waitFinished(page, copyId);

  const headers = await teamHeaders(page);
  const copy = await (await page.request.get(`/api/v1/jobs/${copyId}`, { headers })).json();
  expect(copy.trigger).toBe("relaunch");
  expect(copy.parameters.source_job_id).toBe(source.id);
  expect(copy.parameters.extra_vars.stored).toBe("yes");
  expect(copy.parameters.extra_vars.changed).toBe("yes");
});
