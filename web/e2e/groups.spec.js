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

async function signInEnglish(page) {
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Sprache" })).toBeVisible();
  await page.getByRole("button", { name: "Sprache" }).click();
  await page.getByRole("option", { name: /English/ }).click();
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();
}

async function teamHeaders(page) {
  const profile = await page.evaluate(async () => (await fetch("/api/v1/me")).json());
  return { "X-Team-Id": profile.teams[0].id };
}

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function section(page, name) {
  return page.locator(".template-group").filter({
    has: page.getByRole("button", { name: new RegExp(`^${escapeRegex(name)}`) }),
  });
}

test("groups templates into collapsible sections with a group filter", async ({ page }) => {
  test.setTimeout(60000);
  const suffix = `${test.info().project.name}-${Date.now()}`.replace(/\s+/g, "-");
  const patchday = `Patchday ${suffix}`;
  const kubernetes = `Kubernetes ${suffix}`;
  const templateName = `Grouped shell ${suffix}`;

  await signInEnglish(page);
  const headers = await teamHeaders(page);
  const created = await post(page.request, "/api/v1/templates", {
    headers,
    data: {
      name: templateName,
      tool: "shell",
      entrypoint: "",
      spec: { script: "true" },
      no_environment: true,
    },
  });
  expect(created.status()).toBe(201);

  await page.goto("/#/templates");
  await expect(page.getByRole("heading", { level: 1, name: "Templates" })).toBeVisible();
  await noHorizontalScroll(page);

  await page.getByRole("button", { name: "Groups", exact: true }).click();
  const groupDialog = page.locator("#template-groups-dialog");
  await expect(groupDialog).toBeVisible();
  for (const name of [patchday, kubernetes]) {
    await groupDialog.locator('input[name="new_group_name"]').fill(name);
    await groupDialog.getByRole("button", { name: "Add group", exact: true }).click();
  }
  await groupDialog.getByRole("button", { name: "Save groups", exact: true }).click();
  await expect(groupDialog).toHaveCount(0);

  const patchdaySection = section(page, patchday);
  const kubernetesSection = section(page, kubernetes);
  await expect(patchdaySection).toBeVisible();
  await expect(kubernetesSection).toBeVisible();

  await page.getByRole("link", { name: templateName, exact: true }).click();
  const templateDialog = page.getByRole("dialog");
  await expect(templateDialog.getByRole("heading", { name: "Edit template" })).toBeVisible();
  await templateDialog.locator('select[name="group_id"]').selectOption({ label: patchday });
  await templateDialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(templateDialog).toHaveCount(0);

  await expect(patchdaySection.getByRole("link", { name: templateName, exact: true })).toBeVisible();
  await expect(kubernetesSection.getByRole("link", { name: templateName, exact: true })).toHaveCount(0);
  await expect(page.getByRole("link", { name: templateName, exact: true })).toHaveCount(1);
  const ungrouped = page.locator(".template-group").filter({
    has: page.getByRole("button", { name: /^Ungrouped/ }),
  });
  await expect(ungrouped.getByRole("link", { name: templateName, exact: true })).toHaveCount(0);
  await noHorizontalScroll(page);

  await page.getByRole("link", { name: templateName, exact: true }).click();
  await expect(templateDialog.getByRole("heading", { name: "Edit template" })).toBeVisible();
  await templateDialog.locator('select[name="group_id"]').selectOption({ label: kubernetes });
  await templateDialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(templateDialog).toHaveCount(0);

  await expect(kubernetesSection.getByRole("link", { name: templateName, exact: true })).toBeVisible();
  await expect(patchdaySection.getByRole("link", { name: templateName, exact: true })).toHaveCount(0);

  await page.getByRole("link", { name: templateName, exact: true }).click();
  await expect(templateDialog.getByRole("heading", { name: "Edit template" })).toBeVisible();
  await templateDialog.locator('select[name="group_id"]').selectOption({ label: patchday });
  await templateDialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(templateDialog).toHaveCount(0);

  await expect(patchdaySection.getByRole("link", { name: templateName, exact: true })).toBeVisible();
  await expect(kubernetesSection.getByRole("link", { name: templateName, exact: true })).toHaveCount(0);

  const toggle = patchdaySection.getByRole("button", { name: new RegExp(`^${escapeRegex(patchday)}`) });
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(patchdaySection.getByRole("link", { name: templateName, exact: true })).toBeHidden();

  await page.reload();
  await expect(page.getByRole("heading", { level: 1, name: "Templates" })).toBeVisible();
  const reloadedPatchday = section(page, patchday);
  const reloadedToggle = reloadedPatchday.getByRole("button", { name: new RegExp(`^${escapeRegex(patchday)}`) });
  await expect(reloadedToggle).toHaveAttribute("aria-expanded", "false");
  await expect(reloadedPatchday.getByRole("link", { name: templateName, exact: true })).toBeHidden();
  await expect(section(page, kubernetes).getByRole("link", { name: templateName, exact: true })).toHaveCount(0);
  await noHorizontalScroll(page);

  await reloadedToggle.click();
  await expect(reloadedToggle).toHaveAttribute("aria-expanded", "true");
  await page.locator("#template-group-filter").selectOption({ label: patchday });
  await expect(section(page, patchday)).toBeVisible();
  await expect(section(page, kubernetes)).toHaveCount(0);
  await expect(page.getByRole("link", { name: templateName, exact: true })).toHaveCount(1);

  await page.locator("#template-group-filter").selectOption({ label: "Ungrouped" });
  await expect(page.getByRole("link", { name: templateName, exact: true })).toHaveCount(0);
  await noHorizontalScroll(page);
});
