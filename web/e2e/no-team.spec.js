"use strict";

const { expect, test } = require("@playwright/test");
const { password, username } = require("./env");
const { post } = require("./api");

const emptyState = "This account has no team. An administrator has to add it to one.";
const viewerPassword = "no-team-viewer-pass";

async function noHorizontalScroll(page) {
  await expect.poll(
    () => page.evaluate(() => document.scrollingElement.scrollWidth <= window.innerWidth),
    { message: "the page scrolls sideways" },
  ).toBe(true);
}

async function signIn(page, user, pass) {
  await page.goto("/");
  const language = page.locator("[data-lang-button]");
  await expect(language).toBeVisible();
  if ((await language.getAttribute("aria-label")) === "Sprache") {
    await language.click();
    await page.getByRole("option", { name: /English/ }).click();
  }
  await page.getByLabel("Username").fill(user);
  await page.getByLabel("Password").fill(pass);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
}

async function signOut(page) {
  await revealSidebar(page);
  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
}

async function revealSidebar(page) {
  const toggle = page.getByRole("button", { name: "Menu", exact: true });
  if (!(await toggle.isVisible())) return;
  if ((await toggle.getAttribute("aria-expanded")) !== "true") await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
}

async function createViewer(page, name) {
  const created = await post(page.request, "/api/v1/users", {
    data: {
      username: name,
      display_name: name,
      password: viewerPassword,
      is_admin: false,
    },
  });
  expect(created.status()).toBe(201);
}

async function expectNoTeamEmptyState(page) {
  await expect(page.getByRole("heading", { level: 1, name: "Overview" })).toBeVisible();
  await expect(page.getByText(emptyState, { exact: true })).toBeVisible();
  await noHorizontalScroll(page);
}

async function expectViewerShellWithoutTeam(page) {
  await expectNoTeamEmptyState(page);
  await expect(page.getByRole("link", { name: "Open team", exact: true })).toHaveCount(0);
  await expect(page.getByRole("navigation")).toHaveCount(0);
  await expect(page.locator('a[href="#/team"]')).toHaveCount(0);
  await expect(page.locator('a[href="#/jobs"]')).toHaveCount(0);

  await revealSidebar(page);
  await expect(page.locator(".account-link")).toBeVisible();
  await expect(page.locator("[data-theme-select]")).toBeVisible();
  await expect(page.locator("[data-lang-button]")).toBeVisible();
  await expect(page.getByRole("button", { name: "Sign out", exact: true })).toBeVisible();
}

test("a viewer without a team sees one empty state and no team navigation", async ({ page }, testInfo) => {
  test.setTimeout(90000);
  const suffix = Date.now().toString(36);
  const viewer = `no-team-viewer-${suffix}`;
  await signIn(page, username, password);
  await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();
  await createViewer(page, viewer);
  await signOut(page);
  await signIn(page, viewer, viewerPassword);
  await expectViewerShellWithoutTeam(page);

  const viewport = page.viewportSize();
  if (testInfo.project.name.includes("Pixel")) expect(viewport).toEqual({ width: 412, height: 923 });
  else expect(viewport).toEqual({ width: 1440, height: 900 });

  const german = "Diesem Konto ist kein Team zugeordnet. Ein Administrator muss dieses Konto einem Team hinzufügen.";
  for (const language of ["de", "en"]) {
    for (const theme of ["light", "dark"]) {
      await page.evaluate(([lang, mode]) => {
        localStorage.setItem("koreograf.lang", lang);
        localStorage.setItem("koreograf.theme", mode);
      }, [language, theme]);
      await page.reload();
      await expect(page.getByRole("heading", { level: 1, name: language === "de" ? "Übersicht" : "Overview" })).toBeVisible();
      await expect(page.getByText(language === "de" ? german : emptyState, { exact: true })).toBeVisible();
      await expect(page.locator('a[href="#/team"]')).toHaveCount(0);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await noHorizontalScroll(page);
    }
  }

  for (const [hash, title] of [["#/jobs", "Jobs"], ["#/templates", "Templates"]]) {
    await page.goto(`/${hash}`);
    await expect(page.getByRole("heading", { level: 1, name: title })).toBeVisible();
    await expect(page.getByText(emptyState, { exact: true })).toBeVisible();
    await expect(page.getByRole("navigation")).toHaveCount(0);
    await expect(page.locator('a[href="#/team"]')).toHaveCount(0);
    await noHorizontalScroll(page);
  }
});

test("an administrator still reaches Team, Organizations and Users", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "no-team", "the desktop viewport project only");
  test.setTimeout(90000);
  await signIn(page, username, password);
  await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();

  const nav = page.getByRole("navigation");
  await expect(nav.getByRole("link", { name: "Team", exact: true })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Organizations", exact: true })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Users", exact: true })).toBeVisible();

  await nav.getByRole("link", { name: "Team", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Team" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Team", exact: true })).toHaveClass(/active/);
  await noHorizontalScroll(page);

  await nav.getByRole("link", { name: "Users", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Users" })).toBeVisible();
  await noHorizontalScroll(page);

  await nav.getByRole("link", { name: "Organizations", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Organizations" })).toBeVisible();
  await noHorizontalScroll(page);
});
