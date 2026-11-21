"use strict";

const { expect, test } = require("@playwright/test");
const { execFileSync } = require("node:child_process");
const { mkdirSync, mkdtempSync, rmSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
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

async function clickEdge(page, locator, offset = 5) {
  const point = await locator.evaluate((element) => {
    const middle = element.getPointAtLength(element.getTotalLength() / 2);
    const matrix = element.getScreenCTM();
    return {
      x: middle.x * matrix.a + middle.y * matrix.c + matrix.e,
      y: middle.x * matrix.b + middle.y * matrix.d + matrix.f,
    };
  });
  await page.mouse.click(point.x + offset, point.y + offset);
}

test("an administrator navigates every route, launches the demo shell job and switches language", async ({ page }) => {
  test.setTimeout(120000);
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
  const dropped = page.getByRole("dialog");
  await expect(dropped.getByRole("heading", { name: "Add node" })).toBeVisible();
  await dropped.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

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
  await added.click();
  const inspector = page.locator("#inspector");
  await expect(inspector).toContainText("Extra check");
  await expect(inspector).toContainText("Healthcheck");
  await expect(inspector).toContainText("added");
  await expect(inspector.locator("#node-name")).toHaveCount(0);
  await expect(inspector.locator("#add-node")).toHaveCount(0);

  await page.getByRole("button", { name: "Actions for Healthcheck", exact: true }).click();
  await page.getByRole("menuitem", { name: "Connect", exact: true }).click();
  await added.click();
  const edgeType = page.getByRole("dialog");
  await expect(edgeType.getByRole("heading", { name: "Connect Healthcheck to Extra check" })).toBeVisible();
  await edgeType.getByRole("button", { name: "On success", exact: true }).click();
  await expect(page.locator("#edges path.success")).toHaveCount(2);

  await added.click();
  await page.getByRole("button", { name: "Actions for Extra check", exact: true }).click();
  await page.getByRole("menuitem", { name: "Edit", exact: true }).click();
  const editNode = page.getByRole("dialog");
  await expect(editNode.getByRole("heading", { name: "Edit node" })).toBeVisible();
  await editNode.locator("#node-name").fill("Extra check edited");
  await editNode.getByRole("button", { name: "Save", exact: true }).click();
  await expect(editNode).toHaveCount(0);
  await expect(page.locator(".wf-node", { hasText: "Extra check edited" })).toBeVisible();
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
  await page.getByRole("button", { name: "Actions for Extra check edited", exact: true }).click();
  await page.getByRole("menuitem", { name: "Edit", exact: true }).click();
  const unsavedEdit = page.getByRole("dialog");
  await expect(unsavedEdit.getByRole("heading", { name: "Edit node" })).toBeVisible();
  await unsavedEdit.locator("#node-name").fill("Unsaved check");
  await unsavedEdit.getByRole("button", { name: "Save", exact: true }).click();
  await expect(unsavedEdit).toHaveCount(0);
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

test("an owner clears the dashboard cache and a viewer has no button", async ({ page }) => {
  await signInInEnglish(page);
  const team = await page.evaluate(async () => (await (await fetch("/api/v1/me")).json()).teams[0].id);
  await page.getByRole("tab", { name: "Dashboard", exact: true }).click();
  const clear = page.getByRole("button", { name: "Clear cache", exact: true });
  await expect(clear).toBeVisible();
  await clear.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toHaveClass(/overlay-card/);
  await expect(dialog.getByRole("heading", { name: "Clear cache", exact: true })).toBeVisible();
  await expect(dialog.getByText(/OpenTofu state is kept/)).toBeVisible();
  await dialog.getByRole("button", { name: "Clear cache", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByText("Saved.", { exact: true })).toBeVisible();
  const card = page.locator("section").filter({ has: page.getByRole("heading", { name: "Dependency cache", exact: true }) });
  await expect(card.locator(".stat strong").nth(0)).toHaveText("0");
  await expect(card.locator(".stat strong").nth(1)).toHaveText("0");
  await expect(card.locator(".stat strong").nth(2)).toHaveText("0 B");

  const suffix = Date.now();
  const username = `cache-viewer-${suffix}`;
  const headers = { "X-Team-Id": team };
  const created = await page.request.post("/api/v1/users", {
    headers,
    data: { username, display_name: "Cache Viewer", password: "cache-viewer-pass" },
  });
  expect(created.status()).toBe(201);
  const member = await page.request.post(`/api/v1/teams/${team}/members`, {
    headers,
    data: { username, role: "viewer" },
  });
  expect(member.status()).toBe(201);

  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill("cache-viewer-pass");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation")).toBeVisible();
  await page.getByRole("tab", { name: "Dashboard", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Dependency cache", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Clear cache", exact: true })).toHaveCount(0);
});

test("a long template entry stays on one line at 1440 px and hides at 375 px", async ({ page }) => {
  await signInInEnglish(page);
  const teamId = await page.evaluate(async () => {
    const response = await fetch("/api/v1/me");
    const profile = await response.json();
    return profile.teams[0].id;
  });
  const entrypoint = "playbooks/production/webservers/health_check.yml";
  const created = await page.request.post("/api/v1/templates", {
    headers: { "X-Team-Id": teamId },
    data: { name: "Long entry", tool: "ansible", entrypoint },
  });
  expect(created.status()).toBe(201);

  await page.setViewportSize({ width: 1440, height: 900 });
  await page.getByRole("navigation").getByRole("link", { name: "Templates", exact: true }).click();
  const row = page.getByRole("row", { name: /Long entry/ });
  const entry = row.locator("td.entry .entry-path");
  await expect(entry).toHaveText(entrypoint);
  await expect(entry).toHaveAttribute("title", entrypoint);
  const box = await entry.boundingBox();
  expect(box.height).toBeLessThan(24);
  const clipped = await entry.evaluate((element) => element.scrollWidth > element.clientWidth);
  expect(clipped).toBe(true);
  const tool = await row.locator("td.tool").evaluate((element) => getComputedStyle(element).whiteSpace);
  expect(tool).toBe("nowrap");
  const run = row.locator("td.small");
  await expect(run).toHaveText("Run");
  expect(await run.evaluate((element) => getComputedStyle(element).wordBreak)).toBe("normal");
  const pageWidth = await page.evaluate(() => document.documentElement.scrollWidth);
  expect(pageWidth).toBeLessThanOrEqual(1440);

  await page.setViewportSize({ width: 375, height: 812 });
  await expect(entry).toBeHidden();
  const width = await page.evaluate(() => document.documentElement.scrollWidth);
  expect(width).toBeLessThanOrEqual(375);
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(entry).toBeVisible();
});

test("a long workflow description stays complete in the editor header at 1440 px", async ({ page }) => {
  await signInInEnglish(page);
  const teamId = await page.evaluate(async () => {
    const response = await fetch("/api/v1/me");
    const profile = await response.json();
    return profile.teams[0].id;
  });
  const description = "Health check, deploy the web stack and verify the database replica before the traffic switch";
  const created = await page.request.post("/api/v1/workflows", {
    headers: { "X-Team-Id": teamId },
    data: { name: "Long description", description, graph: { nodes: [], edges: [] } },
  });
  expect(created.status()).toBe(201);

  await page.setViewportSize({ width: 1440, height: 900 });
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Long description", exact: true }).click();
  const field = page.locator("#wf-description");
  await expect(field).toHaveValue(description);
  await expect(field).toHaveAttribute("title", description);
  const shape = await field.evaluate((element) => ({
    width: element.getBoundingClientRect().width,
    clipped: element.scrollWidth > element.clientWidth + 1,
    overflow: getComputedStyle(element).textOverflow,
  }));
  expect(shape.width).toBeGreaterThan(256);
  if (shape.clipped) expect(shape.overflow).toBe("ellipsis");
  const name = page.locator("#wf-name");
  await expect(name).toBeVisible();
  expect((await name.boundingBox()).width).toBeGreaterThan(100);
  await name.fill("Renamed workflow");
  await expect(page.locator("#wf-status")).toHaveText("Unsaved changes");
  const pageWidth = await page.evaluate(() => document.documentElement.scrollWidth);
  expect(pageWidth).toBeLessThanOrEqual(1440);
});

test("the launch dialog keeps tags and limit visible at 1440 px", async ({ page }) => {
  await signInInEnglish(page);
  const teamId = await page.evaluate(async () => {
    const response = await fetch("/api/v1/me");
    const profile = await response.json();
    return profile.teams[0].id;
  });
  const headers = { "X-Team-Id": teamId };
  const suffix = Date.now();
  const ssh = await page.request.post("/api/v1/credentials", {
    headers,
    data: { name: `Deploy SSH ${suffix}`, kind: "ssh", secret: { username: "deploy", password: "probe-pass" } },
  });
  expect(ssh.status()).toBe(201);
  const vaults = [];
  for (const vaultId of ["production", "staging", "development", "sandbox"]) {
    const vault = await page.request.post("/api/v1/credentials", {
      headers,
      data: { name: `Vault ${vaultId} ${suffix}`, kind: "vault", secret: { password: "vault-pass", vault_id: vaultId } },
    });
    expect(vault.status()).toBe(201);
    vaults.push((await vault.json()).id);
  }
  const inventory = await page.request.post("/api/v1/inventories", {
    headers,
    data: { name: `Production hosts ${suffix}`, body: "[web]\nweb01 ansible_host=10.0.0.1\n" },
  });
  expect(inventory.status()).toBe(201);
  const created = await page.request.post("/api/v1/templates", {
    headers,
    data: {
      name: `Layout probe ${suffix}`,
      tool: "ansible",
      entrypoint: "playbooks/demo.yml",
      inventory_id: (await inventory.json()).id,
      credential_ids: [(await ssh.json()).id, ...vaults],
      spec: { job_type: "run", verbosity: 1, tags: "web", skip_tags: "slow", limit: "web:&db" },
    },
  });
  expect(created.status()).toBe(201);

  await page.setViewportSize({ width: 1440, height: 900 });
  await page.getByRole("navigation").getByRole("link", { name: "Templates", exact: true }).click();
  const row = page.getByRole("row", { name: new RegExp(`Layout probe ${suffix}`) });
  await row.getByRole("button", { name: "Launch with options", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "Launch", exact: true })).toBeVisible();
  await expect(dialog).toHaveClass(/overlay-card/);
  const body = dialog.locator(".dialog-body");
  const tags = body.locator('[data-chips="tags"]');
  const limit = body.locator('input[name="limit"]');
  await expect(tags).toBeVisible();
  await expect(limit).toBeVisible();

  const layout = await dialog.evaluate((node) => {
    const bodyNode = node.querySelector(".dialog-body");
    const box = (element) => {
      const rect = element.getBoundingClientRect();
      return { top: Math.round(rect.top), bottom: Math.round(rect.bottom) };
    };
    const button = (label) => [...node.querySelectorAll("button")].find((item) => item.textContent.trim() === label);
    return {
      body: {
        ...box(bodyNode),
        scrollTop: bodyNode.scrollTop,
        clientHeight: bodyNode.clientHeight,
        scrollHeight: bodyNode.scrollHeight,
      },
      tags: box(node.querySelector('[data-chips="tags"]')),
      limit: box(node.querySelector('input[name="limit"]')),
      cancel: box(button("Cancel")),
      launch: box(node.querySelector('button[type="submit"]')),
      viewportHeight: window.innerHeight,
    };
  });
  expect(layout.body.scrollTop).toBe(0);
  expect(layout.tags.top).toBeGreaterThanOrEqual(layout.body.top);
  expect(layout.tags.bottom).toBeLessThanOrEqual(layout.body.bottom);
  expect(layout.limit.top).toBeGreaterThanOrEqual(layout.body.top);
  expect(layout.limit.bottom).toBeLessThanOrEqual(layout.body.bottom);
  expect(layout.launch.top).toBeGreaterThanOrEqual(layout.body.bottom);
  expect(layout.cancel.bottom).toBeLessThanOrEqual(layout.viewportHeight);
  expect(layout.launch.bottom).toBeLessThanOrEqual(layout.viewportHeight);

  await page.setViewportSize({ width: 1024, height: 700 });
  const scroller = await body.evaluate((node) => ({ clientHeight: node.clientHeight, scrollHeight: node.scrollHeight }));
  expect(scroller.scrollHeight).toBeGreaterThan(scroller.clientHeight);
  await tags.evaluate((element) => element.scrollIntoView({ block: "nearest" }));
  const tagView = await body.evaluate((node) => {
    const box = (element) => {
      const rect = element.getBoundingClientRect();
      return { top: Math.round(rect.top), bottom: Math.round(rect.bottom) };
    };
    return { ...box(node), tags: box(node.querySelector('[data-chips="tags"]')) };
  });
  expect(tagView.tags.top).toBeGreaterThanOrEqual(tagView.top);
  expect(tagView.tags.bottom).toBeLessThanOrEqual(tagView.bottom);
  await limit.evaluate((element) => element.scrollIntoView({ block: "nearest" }));
  const limitView = await body.evaluate((node) => {
    const box = (element) => {
      const rect = element.getBoundingClientRect();
      return { top: Math.round(rect.top), bottom: Math.round(rect.bottom) };
    };
    return {
      ...box(node),
      scrollTop: node.scrollTop,
      limit: box(node.querySelector('input[name="limit"]')),
    };
  });
  expect(limitView.scrollTop).toBeGreaterThan(0);
  expect(limitView.limit.top).toBeGreaterThanOrEqual(limitView.top);
  expect(limitView.limit.bottom).toBeLessThanOrEqual(limitView.bottom);
});

test("a node card shows its template and limit override", async ({ page }) => {
  await signInInEnglish(page);
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();

  await focusCanvas(page);
  await page.keyboard.press("Shift+F10");
  await expect(page.getByRole("menuitem", { name: "Add node", exact: true })).toBeFocused();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "Add node" })).toBeVisible();
  await dialog.locator("#node-name").fill("Limited deploy");
  await dialog.locator("#node-template").selectOption({ label: "Self-Heal" });
  await dialog.locator("#node-limit").fill("web");
  await dialog.locator('[data-chips="tags"] .chip-entry').fill("web,");
  await expect(dialog.locator('[data-chips="tags"] .chip-text')).toHaveText("web");
  await dialog.getByRole("button", { name: "Create", exact: true }).click();
  await expect(dialog).toHaveCount(0);

  const card = page.locator(".wf-node", { hasText: "Limited deploy" });
  await expect(card).toBeVisible();
  await expect(card.locator(".wf-node-template")).toHaveText("Self-Heal");
  await expect(card.locator(".wf-node-tool")).toHaveAttribute("aria-label", "Ansible");
  await expect(card.locator('[data-chip="limit"]')).toHaveText("Limit: web");
  await expect(card.locator('[data-chip="tags"]')).toHaveText("Tags: web");
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
  await pressUntilFocused(page, "Tab", () => document.activeElement && document.activeElement.id === "edge-edit");
  await page.keyboard.press("Enter");
  const edgeDialog = page.getByRole("dialog");
  await expect(edgeDialog.getByRole("heading", { name: "Edit edge" })).toBeVisible();
  await edgeDialog.locator("#edge-when").selectOption("failure");
  await edgeDialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.locator("#wf-hint")).toHaveText("Edge changed.");

  await focusCanvas(page);
  await pressUntilFocused(page, "Tab", () => {
    const active = document.activeElement;
    const label = active && active.getAttribute("aria-label");
    return Boolean(active && active.closest("#edges") && label && label.includes("Keyboard Beta") && label.includes("Keyboard Alpha"));
  });
  await page.keyboard.press("Enter");
  await pressUntilFocused(page, "Tab", () => document.activeElement && document.activeElement.id === "edge-edit");
  await page.keyboard.press("Enter");
  const removeDialog = page.getByRole("dialog");
  await expect(removeDialog.getByRole("heading", { name: "Edit edge" })).toBeVisible();
  await removeDialog.getByRole("button", { name: "Delete edge", exact: true }).click();
  await expect(page.locator("#wf-hint")).toHaveText("Edge removed.");
  await expect(page.locator("#edges path")).toHaveCount(edgeCount);
});

test("keeps the workflow inspector key/value pairs in two columns on a desktop", async ({ page }) => {
  await signInInEnglish(page);
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  await page.locator('.wf-node[data-id="check"]').click();
  const list = page.locator("#inspector .readonly-kv").first();
  await expect(list).toContainText("Convergence");
  for (const viewport of [{ width: 1440, height: 900 }, { width: 1024, height: 700 }]) {
    await page.setViewportSize(viewport);
    const pairs = await list.evaluate((element) => [...element.querySelectorAll("dt")].map((term) => {
      const label = term.getBoundingClientRect();
      const value = term.nextElementSibling.getBoundingClientRect();
      return { text: term.textContent, label: label.toJSON(), value: value.toJSON() };
    }));
    expect(pairs.length).toBeGreaterThanOrEqual(4);
    for (const pair of pairs) {
      expect(pair.value.left, `${pair.text} leaves the two-column layout at ${viewport.width}px`).toBeGreaterThan(pair.label.left);
      expect(pair.value.top, `${pair.text} is not on the label row at ${viewport.width}px`).toBeLessThan(pair.label.bottom);
    }
  }
});

test("the workflow edge hit area selects a failure edge and names its nodes", async ({ page }) => {
  await signInInEnglish(page);
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  await page.locator('.wf-node[data-id="check"]').click();
  const inspector = page.locator("#inspector");
  const outgoing = inspector.locator('[data-direction="outgoing"] .edge-row');
  await expect(outgoing.first()).toBeVisible();
  await expect(outgoing.locator(".pill.failure")).toHaveText("On failure");
  expect(await outgoing.locator(".pill.success").count()).toBeGreaterThanOrEqual(1);
  const failure = page.locator("#edges path.failure");
  await expect(failure).toHaveCount(1);
  await clickEdge(page, failure);
  await expect(page.locator("#edge-when")).toHaveCount(0);
  await expect(inspector).toContainText("On failure");
  await expect(inspector).toContainText("Healthcheck");
  await expect(inspector).toContainText("Eskalation");
  await expect(inspector).not.toContainText("Error");
  await page.locator("#edge-edit").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "Edit edge" })).toBeVisible();
  await expect(dialog.locator("#edge-when")).toHaveValue("failure");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(dialog).toHaveCount(0);
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

test("a left click shows a read-only summary and right-click Edit opens the overlay", async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 900 });
  await signInInEnglish(page);
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();

  const node = page.locator('.wf-node[data-id="check"]');
  await node.click();
  const inspector = page.locator("#inspector");
  await expect(inspector).toContainText("Healthcheck");
  await expect(inspector.locator("#node-name")).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Edit node" })).toHaveCount(0);
  await expect(page.getByRole("dialog")).toHaveCount(0);

  await node.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Edit", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "Edit node" })).toBeVisible();
  await expect(dialog.locator("#node-name")).toHaveValue("Healthcheck");
  const overflow = await dialog.evaluate((element) => element.scrollWidth - element.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.locator("#wf-status")).toHaveText("All changes saved");
});

test("the workflow editor reflows at 375 px and opens the inspector on selection", async ({ page }) => {
  await signInInEnglish(page);
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.setViewportSize({ width: 375, height: 812 });
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  await expect(page.locator("#canvas")).toBeVisible();

  const pageWidth = async () => page.evaluate(() => ({
    content: document.documentElement.scrollWidth,
    viewport: window.innerWidth,
  }));
  await expect.poll(async () => (await pageWidth()).content).toBeLessThanOrEqual(375);

  const toggle = page.locator("#inspector-toggle");
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  const node = page.locator('.wf-node[data-id="check"]');
  await node.scrollIntoViewIfNeeded();
  await node.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator("#inspector .node-panel")).toBeVisible();
  await expect(page.locator("#inspector #node-name")).toHaveCount(0);
  const after = await pageWidth();
  expect(after.content).toBeLessThanOrEqual(after.viewport);
  const inspector = await page.locator("#inspector").boundingBox();
  expect(inspector.y).toBeLessThan(812);

  await page.setViewportSize({ width: 1024, height: 900 });
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator("#inspector")).toBeVisible();
  await page.setViewportSize({ width: 375, height: 812 });
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator("#inspector")).toBeHidden();
});

test("an empty workflow builds a success chain without the context menu", async ({ page }) => {
  await signInInEnglish(page);
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "+ Workflow", exact: true }).click();
  const create = page.getByRole("dialog");
  await expect(create.getByRole("heading", { name: "New workflow" })).toBeVisible();
  await create.getByRole("textbox", { name: /^Name/ }).fill("Handle chain");
  await create.getByRole("button", { name: "Create and open", exact: true }).click();
  await expect(create).toHaveCount(0);

  await expect(page.locator("#wf-add-first")).toBeVisible();
  await expect(page.locator("[data-add-port]")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Launch", exact: true })).toBeDisabled();
  await expect(page.locator("#wf-validation")).toHaveText("The workflow has no nodes yet.");

  await page.locator("#wf-add-first").focus();
  await page.keyboard.press("Enter");
  const first = page.getByRole("dialog");
  await expect(first.getByRole("heading", { name: "Add node" })).toBeVisible();
  await first.locator("#node-name").fill("First step");
  await first.getByRole("button", { name: "Create", exact: true }).click();
  await expect(first).toHaveCount(0);
  await expect(page.locator(".wf-node")).toHaveCount(1);
  await expect(page.locator("#wf-add-first")).toBeHidden();

  const firstNode = page.locator(".wf-node", { hasText: "First step" });
  await firstNode.locator('[data-add-port="success"]').click();
  const second = page.getByRole("dialog");
  await expect(second.getByRole("heading", { name: "Add node" })).toBeVisible();
  await second.locator("#node-name").fill("Second step");
  await second.getByRole("button", { name: "Create", exact: true }).click();
  await expect(second).toHaveCount(0);

  await expect(page.locator(".wf-node")).toHaveCount(2);
  await expect(page.locator("#edges path.success")).toHaveCount(1);

  await page.locator(".wf-node", { hasText: "Second step" }).click();
  await page.getByRole("button", { name: "Actions for Second step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Connect", exact: true }).click();
  const sourceBox = await page.locator(".wf-node", { hasText: "Second step" }).boundingBox();
  const targetBox = await page.locator(".wf-node", { hasText: "First step" }).boundingBox();
  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(targetBox.x + targetBox.width / 2, targetBox.y + targetBox.height / 2, { steps: 5 });
  await page.mouse.up();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator("#wf-hint")).toHaveText("This connection would create a cycle.");
  await expect(page.locator("#edges path.edge-line")).toHaveCount(1);
});

test("Delete removes a connected node and Ctrl+Z restores it with its edge", async ({ page }) => {
  await signInInEnglish(page);
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  const source = page.locator('.wf-node[data-id="check"]');
  await expect(source).toBeVisible();
  await expect(page.locator("#edges path.edge-line").first()).toBeVisible();
  const edgeCount = await page.locator("#edges path.edge-line").count();
  await source.locator('[data-add-port="success"]').click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "Add node" })).toBeVisible();
  await dialog.locator("#node-name").fill("Undo target");
  await dialog.getByRole("button", { name: "Create", exact: true }).click();
  await expect(dialog).toHaveCount(0);

  const added = page.locator(".wf-node", { hasText: "Undo target" });
  await expect(added).toBeVisible();
  await expect(page.locator("#edges path.edge-line")).toHaveCount(edgeCount + 1);

  await added.focus();
  await page.keyboard.press("Delete");
  await expect(page.locator(".wf-node", { hasText: "Undo target" })).toHaveCount(0);
  await expect(page.locator("#edges path.edge-line")).toHaveCount(edgeCount);
  await expect(page.locator("#wf-toast")).toContainText("Node deleted.");

  await page.keyboard.press("Control+z");
  await expect(page.locator(".wf-node", { hasText: "Undo target" })).toBeVisible();
  await expect(page.locator("#edges path.edge-line")).toHaveCount(edgeCount + 1);
});

test("a finished workflow run shows its path, closes output on Escape and is reachable from the Runs tab", async ({ page }) => {
  test.setTimeout(120000);
  await signInInEnglish(page);
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  await page.locator("#wf-launch").click();
  await expect(page).toHaveURL(/#\/runs\/[0-9a-f-]+$/);

  const state = page.locator("#run-state");
  await expect(state).toHaveText(/Successful|Failed/, { timeout: 90000 });
  const node = page.locator('.wf-node[data-id="check"]');
  await expect(node.locator(".wf-node-status-text")).toHaveText(/Successful|Failed/);
  await expect(page.getByRole("link", { name: "Back to the workflow", exact: true })).toHaveCount(1);

  await node.click();
  const overlay = page.locator("#node-output");
  await expect(overlay).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(overlay).toHaveCount(0);
  await expect(node).toBeFocused();

  const runHash = new URL(page.url()).hash;
  await page.getByRole("link", { name: "Back to the workflow", exact: true }).click();
  await page.getByRole("tab", { name: "Runs", exact: true }).click();
  const runLink = page.locator(`#wf-panel-runs a[href="${runHash}"]`).first();
  await expect(runLink).toBeVisible();
  await runLink.click();
  await expect(page).toHaveURL(new RegExp(`${runHash.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`));

  await page.setViewportSize({ width: 375, height: 812 });
  const topStatus = page.locator("#run-state-top");
  await expect(topStatus).toBeVisible();
  const statusBox = await topStatus.boundingBox();
  expect(statusBox.y).toBeLessThan(812);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
});

test("the workflow editor turns view-only when the window becomes a phone", async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 900 });
  await signInInEnglish(page);
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  await expect(page.getByRole("button", { name: "Save", exact: true })).toBeVisible();

  await page.setViewportSize({ width: 844, height: 390 });
  await expect(page.locator("#wf-desktop-note")).toBeVisible();
  await expect(page.getByRole("button", { name: "Save", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Launch", exact: true })).toHaveCount(0);
  await expect(page.locator("#wf-tab-danger")).toHaveCount(0);

  await page.setViewportSize({ width: 1024, height: 900 });
  await expect(page.getByRole("button", { name: "Save", exact: true })).toBeVisible();
  await expect(page.locator("#wf-desktop-note")).toHaveCount(0);
});

test("a viewer inspects a workflow node without edit controls", async ({ page }) => {  await signInInEnglish(page);
  const teams = await (await page.request.get("/api/v1/teams")).json();
  const team = teams.find((item) => item.name === "Plattform") || teams[0];
  const users = await (await page.request.get("/api/v1/users")).json();
  if (!users.some((user) => user.username === "carol")) {
    const created = await page.request.post("/api/v1/users", {
      data: { username: "carol", password: "carol-viewer-pass", display_name: "Carol Viewer" },
    });
    expect(created.status()).toBe(201);
  }
  const member = await page.request.post(`/api/v1/teams/${team.id}/members`, {
    data: { username: "carol", role: "viewer" },
  });
  expect(member.status()).toBe(201);

  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
  await page.getByLabel("Username").fill("carol");
  await page.getByLabel("Password").fill("carol-viewer-pass");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation")).toBeVisible();
  await page.locator("#team-select").selectOption(team.id);

  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  await expect(page.locator(".top .pill.read-only")).toHaveText("Read-only");
  await expect(page.locator("#wf-hint")).toHaveText(`Read-only. Operators of team ${team.name} can edit.`);
  await expect(page.getByRole("button", { name: "Save", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Launch", exact: true })).toHaveCount(0);
  await expect(page.locator("#wf-add-first")).toHaveCount(0);

  const node = page.locator('.wf-node[data-id="heal"]');
  await node.click();
  const inspector = page.locator("#inspector");
  await expect(inspector).toContainText("Self-Heal");
  await expect(inspector).toContainText("Inventory");
  await expect(inspector).toContainText("Limit");
  await expect(inspector).toContainText("Tags");
  await expect(inspector).toContainText("Extra variables");
  await expect(inspector).toContainText("Connections");
  await expect(inspector).toContainText("Outgoing");
  await expect(inspector).toContainText("Incoming");
  await expect(page.locator("#node-name")).toHaveCount(0);
  await expect(page.locator("#node-delete")).toHaveCount(0);
  await expect(page.locator("#node-connect")).toHaveCount(0);
  await expect(page.locator("#node-edit-dialog")).toHaveCount(0);
  await expect(page.locator("#edge-edit-dialog")).toHaveCount(0);
  await expect(page.locator("[data-node-menu]")).toHaveCount(0);
  await expect(page.locator("[data-add-port]")).toHaveCount(0);

  const failure = page.locator("#edges path.failure");
  const point = await failure.evaluate((element) => {
    const middle = element.getPointAtLength(element.getTotalLength() / 2);
    const matrix = element.getScreenCTM();
    return {
      x: middle.x * matrix.a + middle.y * matrix.c + matrix.e,
      y: middle.x * matrix.b + middle.y * matrix.d + matrix.f,
    };
  });
  await page.mouse.click(point.x + 5, point.y + 5);
  await expect(inspector).toContainText("On failure");
  await expect(page.locator("#edge-when")).toHaveCount(0);
  await expect(page.locator("#edge-delete")).toHaveCount(0);

  await page.locator("#board").click({ button: "right", position: { x: 40, y: 40 } });
  await expect(page.getByRole("menuitem", { name: "Add node", exact: true })).toHaveCount(0);
});

test("a dragged workflow node snaps to the grid and tidy up avoids overlaps", async ({ page }) => {
  await signInInEnglish(page);
  await page.getByRole("navigation").getByRole("link", { name: "Workflows", exact: true }).click();
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();

  const node = page.locator('.wf-node[data-id="check"]');
  await node.scrollIntoViewIfNeeded();
  const box = await node.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 37, box.y + box.height / 2 + 23, { steps: 5 });
  await page.mouse.up();

  const dragged = await node.evaluate((element) => ({
    x: Number.parseFloat(element.style.left),
    y: Number.parseFloat(element.style.top),
  }));
  expect(dragged.x % 22).toBe(0);
  expect(dragged.y % 22).toBe(0);

  await page.locator("#wf-auto-layout").click();
  await expect(page.locator("#wf-status")).toHaveText("Unsaved changes");
  const positions = await page.locator(".wf-node").evaluateAll((elements) => elements.map((element) => ({
    x: Number.parseFloat(element.style.left),
    y: Number.parseFloat(element.style.top),
    width: element.offsetWidth,
    height: element.offsetHeight,
  })));
  for (let left = 0; left < positions.length; left += 1) {
    for (let right = left + 1; right < positions.length; right += 1) {
      const first = positions[left];
      const second = positions[right];
      const overlap = first.x < second.x + second.width
        && first.x + first.width > second.x
        && first.y < second.y + second.height
        && first.y + first.height > second.y;
      expect(overlap).toBe(false);
    }
  }
});

function rgbLuminance(value) {
  const channels = (String(value).match(/[\d.]+/g) || []).slice(0, 3).map(Number);
  const linear = channels.map((channel) => {
    const unit = channel / 255;
    return unit <= 0.04045 ? unit / 12.92 : ((unit + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
}

function rgbContrast(first, second) {
  const a = rgbLuminance(first);
  const b = rgbLuminance(second);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

async function computedColors(locator) {
  return locator.first().evaluate((element) => {
    const style = getComputedStyle(element);
    return { color: style.color, background: style.backgroundColor, border: style.borderColor };
  });
}

async function themeAttribute(page) {
  return page.evaluate(() => document.documentElement.dataset.theme);
}

test("the activity page lists a sync and a launch and the filters narrow it", async ({ page }) => {
  await signInInEnglish(page);
  const teamId = await page.evaluate(async () => {
    const response = await fetch("/api/v1/me");
    const profile = await response.json();
    return profile.teams[0].id;
  });
  const headers = { "X-Team-Id": teamId };
  const suffix = Date.now();
  const projects = await (await page.request.get("/api/v1/projects", { headers })).json();
  const synced = await page.request.post(`/api/v1/projects/${projects[0].id}/sync`, { headers, data: {} });
  expect(synced.status()).toBe(200);
  const created = await page.request.post("/api/v1/templates", {
    headers,
    data: { name: `Activity probe ${suffix}`, tool: "shell", spec: { script: "#!/bin/sh\ntrue\n" } },
  });
  expect(created.status()).toBe(201);
  const template = await created.json();
  const launched = await page.request.post(`/api/v1/templates/${template.id}/launch`, { headers, data: { extra_vars: {} } });
  expect(launched.status()).toBe(202);

  const nav = page.getByRole("navigation");
  await nav.getByRole("link", { name: "Activity", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Activity" })).toBeVisible();
  const rows = page.locator("#activity-table tbody tr");
  const probe = rows.filter({ hasText: `Activity probe ${suffix}` });
  await expect(probe).toHaveCount(2);
  await expect(probe.filter({ hasText: "Created" })).toHaveCount(1);
  await expect(probe.filter({ hasText: "Launched" })).toHaveCount(1);
  await expect(rows.filter({ hasText: "Synced" }).filter({ hasText: projects[0].name }).first()).toBeVisible();

  await page.locator("#activity-action").selectOption("create");
  await expect(page).toHaveURL(/action=create/);
  await expect(probe).toHaveCount(1);
  await expect(probe.first()).toContainText("Created");
  await expect(probe.first().locator(".pill.successful")).toHaveText("Successful");

  await page.locator("#activity-object").selectOption("job");
  await expect(probe).toHaveCount(0);

  await page.getByRole("button", { name: "Clear filter", exact: true }).click();
  await expect(probe.filter({ hasText: "Launched" })).toHaveCount(1);
  await expect(page.locator("#activity-action")).toHaveValue("");
  await expect(page.locator("#activity-object")).toHaveValue("");
});

test.describe("dark theme", () => {
  test.use({ colorScheme: "dark" });

  test("the sign-in page, a list page, the template overlay and a job log follow the system preference", async ({ page }) => {
    test.setTimeout(120000);
    await page.goto("/");
    await expect(page.locator(".login-card")).toBeVisible();
    expect(await themeAttribute(page)).toBe("dark");

    const login = await computedColors(page.locator(".login-card"));
    expect(rgbLuminance(login.background)).toBeLessThan(0.1);
    expect(rgbContrast(login.color, login.background)).toBeGreaterThanOrEqual(4.5);
    const loginField = await computedColors(page.locator('.login-card input[name="username"]'));
    expect(rgbLuminance(loginField.background)).toBeLessThan(0.3);

    await page.getByRole("button", { name: "Sprache" }).click();
    await page.getByRole("option", { name: /English/ }).click();
    await page.getByLabel("Username").fill(username);
    await page.getByLabel("Password").fill(password);
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.getByRole("navigation")).toBeVisible();

    const body = await computedColors(page.locator("body"));
    expect(rgbLuminance(body.background)).toBeLessThan(0.1);
    const side = await computedColors(page.locator(".side"));
    expect(rgbLuminance(side.background)).toBeLessThan(0.1);
    const navLink = await computedColors(page.locator(".nav a").first());
    expect(rgbContrast(navLink.color, side.background)).toBeGreaterThanOrEqual(4.5);

    await page.getByRole("navigation").getByRole("link", { name: "Templates", exact: true }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Templates" })).toBeVisible();
    const list = await computedColors(page.locator("#template-table .table tbody tr").first());
    const card = await computedColors(page.locator("#template-table").locator("xpath=ancestor::section[contains(@class,'card')]"));
    expect(rgbLuminance(card.background)).toBeLessThan(0.2);
    expect(rgbContrast(list.color, card.background)).toBeGreaterThanOrEqual(4.5);

    await page.getByRole("link", { name: "Healthcheck", exact: true }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("heading", { name: "Edit template" })).toBeVisible();
    const overlay = await computedColors(dialog);
    expect(rgbLuminance(overlay.background)).toBeLessThan(0.2);
    const field = await computedColors(dialog.getByRole("textbox", { name: /^Name/ }));
    expect(rgbLuminance(field.background)).toBeLessThan(0.3);
    expect(rgbContrast(field.color, field.background)).toBeGreaterThanOrEqual(4.5);
    const tab = await computedColors(dialog.getByRole("tab", { name: "Schedules", exact: true }));
    expect(rgbContrast(tab.color, overlay.background)).toBeGreaterThanOrEqual(4.5);
    await dialog.getByRole("button", { name: "Close" }).click();
    await expect(dialog).toHaveCount(0);

    await page.getByRole("row", { name: /Healthcheck/ }).getByRole("button", { name: "Launch", exact: true }).click();
    await expect(page).toHaveURL(/#\/jobs\/[0-9a-f-]+$/);
    await expect(page.getByText("healthcheck ok", { exact: true })).toBeVisible({ timeout: 30000 });
    const log = await computedColors(page.locator(".log"));
    expect(rgbLuminance(log.background)).toBeLessThan(0.1);
    expect(rgbContrast(log.color, log.background)).toBeGreaterThanOrEqual(4.5);
    const status = await computedColors(page.locator(".pill.successful").first());
    expect(rgbContrast(status.color, status.background)).toBeGreaterThanOrEqual(4.5);
    expect(status.background).not.toBe(log.background);
  });

  test("the theme control persists an explicit choice and returns to the system preference", async ({ page }) => {
    await page.goto("/");
    await signInInEnglish(page);
    const select = page.locator("[data-theme-select]").first();
    await expect(select).toHaveValue("system");
    expect(await themeAttribute(page)).toBe("dark");

    await select.selectOption("light");
    expect(await themeAttribute(page)).toBe("light");
    expect(await page.evaluate(() => localStorage.getItem("koreograf.theme"))).toBe("light");
    await expect.poll(async () => rgbLuminance(await page.evaluate(() => getComputedStyle(document.body).backgroundColor))).toBeGreaterThan(0.8);

    await page.reload();
    await expect(page.getByRole("navigation")).toBeVisible();
    expect(await themeAttribute(page)).toBe("light");
    await expect(page.locator("[data-theme-select]").first()).toHaveValue("light");

    await page.locator("[data-theme-select]").first().selectOption("system");
    expect(await themeAttribute(page)).toBe("dark");
    expect(await page.evaluate(() => localStorage.getItem("koreograf.theme"))).toBe("system");
  });
});

test("an operator syncs a project and an inventory from their lists", async ({ page }) => {
  test.setTimeout(120000);
  await signInInEnglish(page);
  const teams = await (await page.request.get("/api/v1/teams")).json();
  const team = teams.find((item) => item.name === "Plattform") || teams[0];
  const members = await (await page.request.get(`/api/v1/teams/${team.id}/members`)).json();
  for (const [username, displayName, password, role] of [
    ["sync-operator", "Sync Operator", "operator-pass", "operator"],
    ["sync-viewer", "Sync Viewer", "viewer-pass", "viewer"],
  ]) {
    const users = await (await page.request.get("/api/v1/users")).json();
    if (!users.some((user) => user.username === username)) {
      const created = await page.request.post("/api/v1/users", {
        data: { username, display_name: displayName, password },
      });
      expect(created.status()).toBe(201);
    }
    if (!members.some((member) => member.username === username)) {
      const member = await page.request.post(`/api/v1/teams/${team.id}/members`, {
        data: { username, role },
      });
      expect(member.status()).toBe(201);
    }
  }

  const suffix = Date.now();
  const projectName = `List sync project ${suffix}`;
  const inventoryName = `List sync inventory ${suffix}`;
  mkdirSync(join(tmpdir(), "koreograf-e2e-projects"), { recursive: true });
  const repo = mkdtempSync(join(tmpdir(), "koreograf-e2e-projects", "repo-"));
  execFileSync("git", ["init", "-b", "main", repo]);
  execFileSync("git", ["-C", repo, "config", "user.email", "test@example.com"]);
  execFileSync("git", ["-C", repo, "config", "user.name", "Koreograf Test"]);
  mkdirSync(join(repo, "inventory"));
  writeFileSync(join(repo, "inventory", "hosts.ini"), "[all]\nlocalhost ansible_connection=local\n");
  writeFileSync(join(repo, "site.yml"), "- hosts: all\n  tasks: []\n");
  execFileSync("git", ["-C", repo, "add", "inventory/hosts.ini", "site.yml"]);
  execFileSync("git", ["-C", repo, "commit", "-m", "main"]);
  const demo = `file://${repo}`;
  const createdProject = await page.request.post("/api/v1/projects", {
    headers: { "X-Team-Id": team.id },
    data: { name: projectName, source_type: "git", source: demo, revision: "main" },
  });
  expect(createdProject.status()).toBe(201);
  const project = await createdProject.json();
  const synced = await page.request.post(`/api/v1/projects/${project.id}/sync`, {
    headers: { "X-Team-Id": team.id },
    data: {},
  });
  expect(synced.status()).toBe(200);
  const createdInventory = await page.request.post("/api/v1/inventories", {
    headers: { "X-Team-Id": team.id },
    data: {
      name: inventoryName,
      sources: [{ kind: "project", project_id: project.id, path: "inventory/hosts.ini" }],
    },
  });
  expect(createdInventory.status()).toBe(201);

  const delayed = (pattern) => page.route(pattern, async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 500));
    await route.continue();
  });
  await delayed("**/api/v1/projects/*/sync");
  await delayed("**/api/v1/inventories/*/sync");

  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
  await page.getByLabel("Username").fill("sync-operator");
  await page.getByLabel("Password").fill("operator-pass");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation")).toBeVisible();
  await page.locator("#team-select").selectOption(team.id);

  await page.getByRole("navigation").getByRole("link", { name: "Projects", exact: true }).click();
  const projectRow = page.getByRole("row", { name: new RegExp(projectName) });
  await expect(projectRow).toBeVisible();
  const projectSync = projectRow.getByRole("button", { name: "Sync", exact: true });
  await expect(projectSync).toBeVisible();
  await projectSync.click();
  await expect(projectRow.getByRole("button", { name: "Syncing…", exact: true })).toBeDisabled();
  await expect(projectRow.locator('td[data-label="Last sync"]')).toContainText("sync-operator", { timeout: 30000 });
  await expect(projectRow.locator('td[data-label="Last sync"]')).toContainText("Success");

  await page.getByRole("navigation").getByRole("link", { name: "Inventories", exact: true }).click();
  const inventoryRow = page.getByRole("row", { name: new RegExp(inventoryName) });
  await expect(inventoryRow).toBeVisible();
  const inventorySync = inventoryRow.getByRole("button", { name: "Refresh", exact: true });
  await expect(inventorySync).toBeVisible();
  await inventorySync.click();
  await expect(inventoryRow.getByRole("button", { name: "Refreshing…", exact: true })).toBeDisabled();
  await expect(inventoryRow.locator('td[data-label="Last sync"]')).toContainText("sync-operator", { timeout: 30000 });
  await expect(inventoryRow.locator('td[data-label="Last sync"]')).toContainText("Success");

  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
  await page.getByLabel("Username").fill("sync-viewer");
  await page.getByLabel("Password").fill("viewer-pass");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation")).toBeVisible();
  await page.locator("#team-select").selectOption(team.id);
  await page.getByRole("navigation").getByRole("link", { name: "Projects", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Projects" })).toBeVisible();
  await expect(page.locator("[data-sync-project]")).toHaveCount(0);
  await page.getByRole("navigation").getByRole("link", { name: "Inventories", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Inventories" })).toBeVisible();
  await expect(page.locator("[data-sync-inventory]")).toHaveCount(0);
  rmSync(repo, { recursive: true, force: true });
});

test("the account page creates a token with a lifetime and shows its expiry", async ({ page }) => {
  await signInInEnglish(page);
  await page.goto("/#/account");
  await expect(page.getByRole("heading", { level: 1, name: "My account" })).toBeVisible();

  const form = page.locator("#token-form");
  const lifetime = form.locator('select[name="expires_in_days"]');
  await expect(lifetime).toBeVisible();
  await expect(lifetime).toHaveValue("90");
  await expect(lifetime.locator("option")).toHaveText(["30 days", "90 days", "365 days", "No expiry"]);
  await lifetime.selectOption("30");
  await form.locator('input[name="name"]').fill("Playwright token");
  await form.getByRole("button", { name: "Create token", exact: true }).click();

  const row = page.getByRole("row", { name: /Playwright token/ });
  await expect(row).toBeVisible();
  const expires = row.locator('td[data-label="Expires"]');
  await expect(expires).not.toHaveText("Never expires");
  await expect(expires).toContainText(/\d{2}\/\d{2}\/\d{4}/);

  const tokens = await page.evaluate(async () => (await (await fetch("/api/v1/me/tokens")).json()));
  const created = tokens.find((item) => item.name === "Playwright token");
  expect(created).toBeTruthy();
  expect(created.expired).toBe(false);
  expect(typeof created.expires_at).toBe("string");
});
