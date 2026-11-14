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

async function signInInEnglish(page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Sprache" }).click();
  await page.getByRole("option", { name: /English/ }).click();
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("navigation")).toBeVisible();
}

async function focusCanvas(page) {
  await page.locator("#canvas").focus();
  await expect(page.locator("#canvas")).toBeFocused();
}

async function addNodeWithKeyboard(page, name) {
  await focusCanvas(page);
  await page.keyboard.press("Shift+F10");
  await expect(page.getByRole("menuitem", { name: "Add node", exact: true })).toBeFocused();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "Add node" })).toBeVisible();
  await expect(dialog.locator("#node-name")).toBeFocused();
  await page.keyboard.type(name);
  await page.keyboard.press("Enter");
  await expect(dialog).toHaveCount(0);
}

async function pressUntilFocused(page, key, predicate, attempts = 40) {
  for (let index = 0; index < attempts; index += 1) {
    if (await page.evaluate(predicate)) return;
    await page.keyboard.press(key);
  }
  throw new Error(`focus did not reach the expected control after ${attempts} ${key} presses`);
}

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
  await dialog.getByRole("tab", { name: "Schedules", exact: true }).click();
  await expect(dialog.getByRole("button", { name: "+ Schedule", exact: true })).toBeVisible();
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

  await page.locator("#board").click({ button: "right", position: { x: boardBox.width - 80, y: boardBox.height - 80 } });
  await page.getByRole("menuitem", { name: "Add node", exact: true }).click();
  const addDialog = page.getByRole("dialog");
  await expect(addDialog.getByRole("heading", { name: "Add node" })).toBeVisible();
  await addDialog.locator("#node-name").fill("Extra check");
  await addDialog.locator("#node-template").selectOption({ label: "Healthcheck" });
  await addDialog.locator("#node-extra").fill('{"alert":"added"}');
  await addDialog.getByRole("button", { name: "Create", exact: true }).click();
  await expect(addDialog).toHaveCount(0);
  const added = page.locator(".wf-node", { hasText: "Extra check" });
  await expect(added).toBeVisible();
  await expect(page.locator("#node-name")).toHaveValue("Extra check");
  await expect(page.locator("#node-template option:checked")).toHaveText("Healthcheck");
  await expect(page.locator("#node-extra")).toHaveValue(/"alert": "added"/);
  await expect(page.locator("#inspector #add-node")).toHaveCount(0);

  await page.locator('.wf-node[data-id="check"]').click();
  await expect(page.locator("#outgoing-edges .edge-row")).toHaveCount(2);
  await page.locator("#link-when").selectOption("success");
  await page.locator("#link-target").selectOption({ label: "Extra check" });
  await page.locator("#link-add").click();
  await expect(page.locator("#outgoing-edges .edge-row")).toHaveCount(3);
  await expect(page.locator("#edges path.success")).toHaveCount(2);

  const successRows = page.locator('#outgoing-edges .edge-row[data-edge-when="success"]');
  await expect(successRows).toHaveCount(2);
  await successRows.nth(0).locator(".edge-target").selectOption("escalate");
  await expect(page.locator("#wf-hint")).toHaveText("Edge changed.");
  await expect(page.locator("#outgoing-edges .edge-row")).toHaveCount(3);
  await expect(page.locator("#edges path.success")).toHaveCount(2);
  await expect(page.locator('#outgoing-edges .edge-row[data-edge-when="success"] .edge-target option:checked')).toHaveText(["Eskalation", "Extra check"]);

  await successRows.nth(1).locator("[data-remove-edge]").click();
  await expect(page.locator("#wf-hint")).toHaveText("Edge removed.");
  await expect(page.locator("#outgoing-edges .edge-row")).toHaveCount(2);
  await expect(page.locator("#edges path.success")).toHaveCount(1);
  await expect(page.locator("#edges path.failure")).toHaveCount(1);
  await expect(page.locator('#outgoing-edges .edge-row[data-edge-when="success"] .edge-target option:checked')).toHaveText("Eskalation");

  await added.click();
  await page.locator("#node-name").fill("Extra check edited");
  await expect(page.locator("#wf-status")).toHaveText("Unsaved changes");

  await nav.getByRole("link", { name: "Jobs", exact: true }).click();
  const discard = page.getByRole("dialog");
  await expect(discard.getByRole("heading", { name: "Discard unsaved changes?" })).toBeVisible();
  await discard.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(new URL(page.url()).hash).toMatch(/^#\/workflows\/[^?]+$/);

  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.locator("#wf-status")).toHaveText("All changes saved");
  await expect(page.getByRole("button", { name: "Launch", exact: true })).toHaveClass(/primary/);

  await nav.getByRole("link", { name: "Jobs", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Jobs" })).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  await nav.getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  const saved = page.locator(".wf-node", { hasText: "Extra check edited" });
  await expect(saved).toBeVisible();
  await saved.click();
  await page.locator("#node-name").fill("Unsaved check");
  await expect(page.locator("#wf-status")).toHaveText("Unsaved changes");

  await page.getByRole("button", { name: "Launch", exact: true }).click();
  const launchChoice = page.getByRole("dialog");
  await expect(launchChoice.getByRole("heading", { name: "Unsaved changes" })).toBeVisible();
  await expect(launchChoice.getByRole("button", { name: "Save and launch", exact: true })).toBeVisible();
  await expect(launchChoice.getByRole("button", { name: "Launch last saved version", exact: true })).toBeVisible();
  await launchChoice.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator("#wf-status")).toHaveText("Unsaved changes");
  expect(new URL(page.url()).hash).toMatch(/^#\/workflows\/[^?]+$/);
});

test("the workflow editor connects, retypes and removes an edge with the keyboard", async ({ page }) => {
  await signInInEnglish(page);
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  await expect(page).toHaveTitle(/Self-Heal Ablauf/);
  await expect(page).toHaveTitle(/Workflow/);

  const edgeCount = await page.locator("#edges path").count();
  await addNodeWithKeyboard(page, "Keyboard Alpha");
  await addNodeWithKeyboard(page, "Keyboard Beta");
  await expect(page.locator(".wf-node", { hasText: "Keyboard Beta" })).toBeVisible();

  await page.locator(".wf-node", { hasText: "Keyboard Beta" }).focus();
  await page.keyboard.press("Shift+F10");
  await expect(page.getByRole("menuitem", { name: "Edit", exact: true })).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitem", { name: "Connect", exact: true })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.locator("#wf-hint")).toContainText("Connecting from Keyboard Beta");

  await pressUntilFocused(page, "Shift+Tab", () => {
    const active = document.activeElement;
    return Boolean(active && active.classList.contains("wf-node") && active.textContent.includes("Keyboard Alpha"));
  });
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator("#wf-hint")).toContainText("added");

  await focusCanvas(page);
  await pressUntilFocused(page, "Tab", () => {
    const active = document.activeElement;
    const label = active && active.getAttribute("aria-label");
    return Boolean(active && active.closest("#edges") && label && label.includes("Keyboard Beta") && label.includes("Keyboard Alpha"));
  });
  await page.keyboard.press("Enter");
  await pressUntilFocused(page, "Tab", () => document.activeElement && document.activeElement.id === "edge-when");
  await page.keyboard.press("ArrowDown");
  await expect(page.locator("#wf-hint")).toHaveText("Edge changed.");

  await page.keyboard.press("Tab");
  await expect(page.locator("#edge-delete")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.locator("#wf-hint")).toHaveText("Edge removed.");
  await expect(page.locator("#edges path")).toHaveCount(edgeCount);
});

test("the workflow canvas fits, zooms and pans", async ({ page }) => {
  await signInInEnglish(page);
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  const canvas = page.locator("#canvas");
  await expect(canvas).toBeVisible();

  await page.locator("[data-canvas-fit]").click();
  const box = await canvas.boundingBox();
  const nodes = page.locator(".wf-node");
  const count = await nodes.count();
  expect(count).toBeGreaterThan(1);
  for (let index = 0; index < count; index += 1) {
    const nodeBox = await nodes.nth(index).boundingBox();
    expect(nodeBox.x).toBeGreaterThanOrEqual(box.x - 1);
    expect(nodeBox.y).toBeGreaterThanOrEqual(box.y - 1);
    expect(nodeBox.x + nodeBox.width).toBeLessThanOrEqual(box.x + box.width + 1);
    expect(nodeBox.y + nodeBox.height).toBeLessThanOrEqual(box.y + box.height + 1);
  }

  const zoom = async () => Number(await canvas.getAttribute("data-zoom"));
  const fitted = await zoom();
  await page.locator("[data-canvas-zoom-in]").click();
  await expect.poll(zoom).toBeGreaterThan(fitted);

  const wheelBefore = await zoom();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -120);
  await page.keyboard.up("Control");
  await expect.poll(zoom).toBeGreaterThan(wheelBefore);

  const viewBefore = Number(await page.locator("#board").getAttribute("data-view-x"));
  await page.mouse.move(box.x + 30, box.y + 30);
  await page.mouse.down();
  await page.mouse.move(box.x + 90, box.y + 50, { steps: 5 });
  await page.mouse.up();
  await expect.poll(async () => Number(await page.locator("#board").getAttribute("data-view-x"))).toBeGreaterThan(viewBefore);
});
