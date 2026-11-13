"use strict";

const { expect, test } = require("@playwright/test");
const { password, username } = require("./env");

const catalog = {
  de: {
    language: "Sprache",
    signIn: "Anmelden",
    overview: "Übersicht",
    routeError: "Diese Seite konnte nicht geladen werden.",
    missingPage: "Diese Seite gibt es nicht.",
  },
  en: {
    language: "Language",
    signIn: "Sign in",
    overview: "Overview",
    routeError: "This page could not be loaded.",
    missingPage: "This page does not exist.",
  },
};

test("an administrator navigates every route, launches the demo shell job and switches language", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("button", { name: catalog.de.signIn })).toBeVisible();

  await page.getByRole("button", { name: catalog.de.language }).click();
  await page.getByRole("option", { name: /English/ }).click();
  await expect(page.getByRole("button", { name: catalog.en.signIn })).toBeVisible();

  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: catalog.en.signIn }).click();
  const nav = page.getByRole("navigation");
  await expect(nav.getByRole("link", { name: catalog.en.overview, exact: true })).toBeVisible();

  await page.getByRole("button", { name: catalog.en.language }).click();
  await page.getByRole("option", { name: /Deutsch/ }).click();
  await expect(nav.getByRole("link", { name: catalog.de.overview, exact: true })).toBeVisible();
  await page.getByRole("button", { name: catalog.de.language }).click();
  await page.getByRole("option", { name: /English/ }).click();
  await expect(nav.getByRole("link", { name: catalog.en.overview, exact: true })).toBeVisible();

  const links = nav.getByRole("link");
  const labels = await links.allTextContents();
  expect(labels).toEqual(expect.arrayContaining([
    "Overview",
    "Jobs",
    "Schedules",
    "Templates",
    "Workflows",
    "Webhooks",
    "Projects",
    "Inventories",
    "Credentials",
    "Execution environments",
    "Team",
    "Organizations",
    "Settings",
    "Users",
    "Backup",
  ]));

  for (const label of labels) {
    await nav.getByRole("link", { name: label, exact: true }).click();
    await expect(page.getByRole("heading", { level: 1, name: label })).toBeVisible();
    await expect(page.getByRole("heading", { name: catalog.en.routeError })).toHaveCount(0);
    await expect(page.getByText(catalog.en.missingPage, { exact: true })).toHaveCount(0);
    expect(new URL(page.url()).hash).not.toBe("");
  }

  await nav.getByRole("link", { name: "Team", exact: true }).click();
  await page.evaluate(() => { window.location.hash = "#/team?new=1"; });
  const teamDialog = page.getByRole("dialog");
  await expect(teamDialog).toBeVisible();
  await expect(teamDialog.locator("select[name=organization_id]")).toBeVisible();
  await teamDialog.getByRole("button", { name: "Close" }).click();
  await expect(teamDialog).toHaveCount(0);

  await nav.getByRole("link", { name: "Templates", exact: true }).click();
  await page.getByRole("link", { name: "Healthcheck", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("heading", { name: "Edit template" })).toBeVisible();
  await expect(dialog.getByRole("textbox", { name: /^Name/ })).toHaveValue("Healthcheck");
  await expect(dialog.getByRole("textbox", { name: "Description" })).toHaveValue("A short shell check that runs without Ansible.");
  await dialog.getByRole("button", { name: "Close" }).click();
  await expect(dialog).toHaveCount(0);

  await page.getByRole("row", { name: /Healthcheck/ }).getByRole("button", { name: "Launch", exact: true }).click();
  await expect(page).toHaveURL(/#\/jobs\/[0-9a-f-]+$/);
  await expect(page.getByText("healthcheck ok", { exact: true })).toBeVisible({ timeout: 30000 });
  await expect(page.getByText("alert=manual", { exact: true })).toBeVisible();
  await expect(page.getByText("Successful", { exact: true })).toBeVisible({ timeout: 30000 });

  await nav.getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  const source = page.locator('.wf-node[data-id="check"]');
  await source.scrollIntoViewIfNeeded();
  await expect(source).toBeVisible();
  await page.getByRole("button", { name: "Actions for Healthcheck", exact: true }).click();
  await page.getByRole("menuitem", { name: "Connect", exact: true }).click();
  const sourceBox = await source.boundingBox();
  const boardBox = await page.locator("#board").boundingBox();
  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(boardBox.x + boardBox.width - 80, boardBox.y + boardBox.height - 80, { steps: 5 });
  await page.mouse.up();
  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2);
  await expect(page.locator("#connect-preview")).toHaveCount(0);
});
