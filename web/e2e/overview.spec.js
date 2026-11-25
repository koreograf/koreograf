"use strict";

const { join } = require("node:path");
const { expect, test } = require("@playwright/test");
const { password, root, username } = require("./env");
const { post } = require("./api");

async function noHorizontalScroll(page) {
  await expect.poll(
    () => page.evaluate(() => document.scrollingElement.scrollWidth <= window.innerWidth),
    { message: "the page scrolls sideways" },
  ).toBe(true);
}

async function textLines(locator) {
  return locator.evaluate((node) => {
    const range = document.createRange();
    range.selectNodeContents(node);
    return new Set([...range.getClientRects()].map((rect) => Math.round(rect.top))).size;
  });
}

async function signInAs(page, user, secret) {
  await page.goto("/");
  await expect(page.getByRole("button", { name: /^(Sprache|Language)$/ })).toBeVisible();
  const german = page.getByRole("button", { name: "Sprache", exact: true });
  if (await german.isVisible()) {
    await german.click();
    await page.getByRole("option", { name: /English/ }).click();
  }
  await page.getByLabel("Username").fill(user);
  await page.getByLabel("Password").fill(secret);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();
  await noHorizontalScroll(page);
}

async function signIn(page) {
  await signInAs(page, username, password);
}

async function teamHeaders(page) {
  const profile = await page.evaluate(async () => (await fetch("/api/v1/me")).json());
  return { "X-Team-Id": profile.teams[0].id };
}

async function createShellTemplate(page, name, script, prompt = false, teamId = null) {
  const headers = teamId ? { "X-Team-Id": teamId } : await teamHeaders(page);
  const created = await post(page.request, "/api/v1/templates", {
    headers,
    data: { name, description: "", tool: "shell", spec: { script }, prompt_on_launch: prompt },
  });
  expect(created.status()).toBe(201);
  return created.json();
}

async function launchAndWaitForTeam(page, teamId, template) {
  const headers = { "X-Team-Id": teamId };
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

async function createViewer(page) {
  const suffix = Date.now().toString(36);
  const user = {
    username: `overview-viewer-${suffix}`,
    display_name: "Overview Viewer",
    password: "overview-viewer-pass",
  };
  const created = await post(page.request, "/api/v1/users", { data: user });
  expect(created.status()).toBe(201);
  const headers = await teamHeaders(page);
  const member = await post(page.request, `/api/v1/teams/${headers["X-Team-Id"]}/members`, {
    data: { username: user.username, role: "viewer" },
  });
  expect(member.status()).toBe(201);
  return user;
}

async function createTeam(page, name) {
  const created = await post(page.request, "/api/v1/teams", {
    data: { name, description: "" },
  });
  expect(created.status()).toBe(201);
  return created.json();
}

async function selectTeam(page, teamId) {
  await page.evaluate((id) => localStorage.setItem("koreograf.team", id), teamId);
  await page.reload();
  await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();
}

async function createSchedule(page, teamId, templateId, name) {
  const created = await post(page.request, `/api/v1/templates/${templateId}/schedules`, {
    headers: { "X-Team-Id": teamId },
    data: { name, timezone: "UTC", start_date: "2027-06-01", start_time: "06:00", frequency: "daily", end_kind: "never" },
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
  await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();

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

test("the dashboard shows range rates and drills into a failed day", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "overview", "the desktop viewport project only");
  test.setTimeout(120000);
  await signIn(page);
  const failing = await createShellTemplate(page, `Dashboard failing ${Date.now().toString(36)}`, "exit 1");
  await launchAndWait(page, failing);

  await page.goto("/#/?tab=dashboard");
  await page.reload();
  await expect(page.getByRole("heading", { level: 2, name: "Jobs per hour" })).toBeVisible();
  await expect(page.locator("[data-chart-day]")).toHaveCount(24);
  for (const id of ["range-success-rate", "range-failed", "range-mean-duration", "range-jobs"]) {
    await expect(page.locator(`[data-kpi="${id}"]`)).toBeVisible();
  }
  const failed = page.locator(".chart-seg.failed").first();
  await expect(failed).toBeVisible();
  await failed.click();
  await expect(page).toHaveURL(/#\/jobs\?status=failed&since=/);
  await expect(page.locator("#job-status")).toHaveValue("failed");
  await expect(page.locator("[data-job-window]")).toContainText(/Date window/);
  await page.locator("[data-job-window-clear]").click();
  await expect(page).not.toHaveURL(/since=/);
  await expect(page.locator("#job-status")).toHaveValue("");

  await page.goto("/#/?tab=dashboard&days=14");
  await page.reload();
  await expect(page.getByRole("heading", { level: 2, name: "Jobs per day" })).toBeVisible();
  await expect(page.locator("[data-chart-day]")).toHaveCount(14);
  await page.locator("[data-chart-day]").last().locator(".chart-day-label").click();
  await expect(page).toHaveURL(/#\/jobs\?since=/);
  await expect(page.locator("#job-status")).toHaveValue("");
  await noHorizontalScroll(page);
});

test("the 1440 overview launches from the header picker and quick launch", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "overview", "the desktop viewport project only");
  test.setTimeout(120000);
  await signIn(page);
  const suffix = Date.now().toString(36);
  const prompt = await createShellTemplate(page, `Overview prompt ${suffix}`, "echo prompt-ok", true);
  const plain = await createShellTemplate(page, `Overview plain ${suffix}`, "echo plain-ok");
  await launchAndWait(page, prompt);
  await launchAndWait(page, plain);

  await page.goto("/#/");
  await page.reload();
  await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();

  const header = page.locator("[data-launch-template]");
  await expect(header).toBeVisible();
  await expect(header).toHaveClass(/primary/);
  await expect(page.locator(".top .btn.primary")).toHaveCount(1, { timeout: 10000 });

  const quick = page.locator("[data-quick-launch-card]");
  await expect(quick).toBeVisible();
  await expect(quick.locator(`[data-quick-launch-start="${plain.id}"]`)).toBeVisible();

  await header.click();
  const picker = page.locator("#template-picker");
  await expect(picker).toBeVisible();
  await picker.locator(`[data-pick-template="${prompt.id}"]`).click();
  const dialog = page.locator("#launch-prompt");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("heading", { name: "Launch", exact: true })).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(dialog).toHaveCount(0);

  await quick.locator(`[data-quick-launch-start="${plain.id}"]`).click();
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("heading", { name: "Launch", exact: true })).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(dialog).toHaveCount(0);

  const box = await header.boundingBox();
  expect(box.y + box.height, "the header action is below the first viewport").toBeLessThanOrEqual(900);
  await noHorizontalScroll(page);
});

test("the overview hides every launch control from viewers", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "overview", "the desktop viewport project only");
  test.setTimeout(60000);
  await signIn(page);
  const viewer = await createViewer(page);
  await page.context().clearCookies();
  await signInAs(page, viewer.username, viewer.password);

  await expect(page.locator("[data-launch-template]")).toHaveCount(0);
  await expect(page.locator("[data-quick-launch-card]")).toHaveCount(0);
  await expect(page.locator("[data-quick-launch-start]")).toHaveCount(0);
  await noHorizontalScroll(page);
});

test("the 412 overview keeps the header action and a quick start usable", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "Pixel 9 overview", "the phone viewport project only");
  test.setTimeout(120000);
  await signIn(page);
  const template = await createShellTemplate(page, `Overview phone ${Date.now().toString(36)}`, "echo phone-ok");
  await launchAndWait(page, template);

  await page.goto("/#/");
  await page.reload();
  await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();
  const header = page.locator("[data-launch-template]");
  await expect(header).toBeVisible();
  const box = await header.boundingBox();
  expect(box.y + box.height, "the header action is below the phone viewport").toBeLessThanOrEqual(923);

  const quick = page.locator("[data-quick-launch-card]");
  await expect(quick).toBeVisible();
  const start = quick.locator(`[data-quick-launch-start="${template.id}"]`);
  await expect(start).toBeVisible();
  await start.click();
  const dialog = page.locator("#launch-prompt");
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await noHorizontalScroll(page);
});

test("the 1280 overview keeps one-line names and distinct tabs in German and dark", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "overview", "the desktop viewport project only");
  test.setTimeout(90000);
  await signIn(page);
  const slow = await createShellTemplate(page, "H Slow", "echo slow");
  await launchAndWait(page, slow);
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.evaluate(() => {
    localStorage.setItem("koreograf.lang", "de");
    localStorage.setItem("koreograf.theme", "dark");
  });
  await page.reload();
  await expect(page.getByRole("heading", { level: 1, name: "Betrieb" })).toBeVisible();
  await expect(page.locator("[data-kpi-row] [data-kpi]")).toHaveCount(5);
  const columns = await page.locator(".overview-main").evaluate((node) => getComputedStyle(node).gridTemplateColumns);
  expect(columns.trim().split(/\s+/), "the overview does not use two columns at 1280").toHaveLength(2);

  const overviewTab = page.getByRole("tab", { name: "Übersicht", exact: true });
  await expect(overviewTab).toHaveClass(/page-tab/);
  await expect(overviewTab).not.toHaveClass(/btn/);
  const tabStyle = await overviewTab.evaluate((node) => ({
    background: getComputedStyle(node).backgroundColor,
    border: getComputedStyle(node).borderBottomColor,
  }));
  expect(tabStyle.background, "the selected page tab still looks like a pill").toBe("rgba(0, 0, 0, 0)");
  expect(tabStyle.border, "the selected page tab has no underline").not.toBe("rgba(0, 0, 0, 0)");

  const nameCell = page.locator("[data-recent-activity] tbody tr").filter({ hasText: "H Slow" }).first().locator("td").first();
  await expect(nameCell).toHaveCSS("white-space", "nowrap");
  expect(await textLines(nameCell.locator("a")), "H Slow wraps at 1280").toBe(1);

  await page.getByRole("tab", { name: "Dashboard", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Betrieb" })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Dashboard", exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".page-tabs .btn")).toHaveCount(0);
  const range = page.getByRole("group", { name: "Zeitraum", exact: true });
  await expect(range).toBeVisible();
  const today = range.getByRole("button", { name: "Heute", exact: true });
  await expect(today).toHaveAttribute("aria-pressed", "true");
  const rangeStyle = await today.evaluate((node) => getComputedStyle(node).backgroundColor);
  expect(rangeStyle, "the range control still looks like the page tabs").not.toBe("rgba(0, 0, 0, 0)");
  await noHorizontalScroll(page);
});

test("the 1280 inventory host list keeps a short host name on one line", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "overview", "the desktop viewport project only");
  test.setTimeout(60000);
  await signIn(page);
  await page.setViewportSize({ width: 1280, height: 800 });
  const headers = await teamHeaders(page);
  const created = await post(page.request, "/api/v1/inventories", {
    headers,
    data: { name: `Ghost hosts ${Date.now().toString(36)}`, body: "[all]\nghost\n" },
  });
  expect(created.status()).toBe(201);
  const inventory = await created.json();
  await page.goto(`/#/inventories/${inventory.id}`);
  await expect(page.getByRole("heading", { level: 1, name: inventory.name })).toBeVisible();
  const host = page.locator("#host-table tbody tr").filter({ hasText: "ghost" }).first();
  const link = host.locator("a.host-name");
  await expect(link).toHaveCSS("white-space", "nowrap");
  await expect(link).toHaveCSS("word-break", "normal");
  expect(await textLines(link), "ghost wraps at 1280").toBe(1);
  await noHorizontalScroll(page);
});

test("an empty overview keeps every glance card compact", async ({ page }) => {
  test.setTimeout(60000);
  await signIn(page);
  const team = await createTeam(page, `Zz overview healthy ${Date.now().toString(36)}`);
  const template = await createShellTemplate(page, `Overview healthy ${Date.now().toString(36)}`, "echo ok", false, team.id);
  await launchAndWaitForTeam(page, team.id, template);
  await selectTeam(page, team.id);

  const cards = [
    ["schedules", "No upcoming or missed schedules."],
    ["hosts", "No problem hosts."],
    ["syncs", "No failed syncs."],
    ["workflows", "No running or failed runs."],
  ];
  for (const [id, text] of cards) {
    const card = page.locator(`[data-glance-card="${id}"]`);
    await expect(card).toBeVisible();
    await expect(card.locator("[data-glance-healthy]")).toHaveCount(1);
    await expect(card.getByText(text, { exact: true })).toBeVisible();
    await expect(card.locator("table")).toHaveCount(0);
    await expect(card.locator(".stat")).toHaveCount(0);
    const box = await card.boundingBox();
    expect(box, `${id} has no box`).not.toBeNull();
    // The header may wrap on a narrow card; the body must stay one short line.
    expect(box.height, `the empty ${id} card is ${box.height}px tall`).toBeLessThanOrEqual(200);
  }

  const viewport = page.viewportSize();
  if (viewport.width > 900) {
    const syncs = await page.locator('[data-glance-card="syncs"]').boundingBox();
    const workflows = await page.locator('[data-glance-card="workflows"]').boundingBox();
    expect(Math.abs(syncs.y - workflows.y), "the empty glance cards do not share one row").toBeLessThanOrEqual(1);
    expect(Math.max(syncs.height, workflows.height), "an empty glance row is too tall").toBeLessThanOrEqual(200);
  }
  await noHorizontalScroll(page);
});

test("a mixed glance row keeps the healthy card short", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "overview", "the desktop viewport project only");
  test.setTimeout(60000);
  await signIn(page);
  const team = await createTeam(page, `Zz overview mixed ${Date.now().toString(36)}`);
  const created = await post(page.request, "/api/v1/templates", {
    headers: { "X-Team-Id": team.id },
    data: { name: `Overview scheduled ${Date.now().toString(36)}`, description: "", tool: "shell", spec: { script: "echo ok" } },
  });
  expect(created.status()).toBe(201);
  const template = await created.json();
  await createSchedule(page, team.id, template.id, "Nightly check");
  await launchAndWaitForTeam(page, team.id, template);
  await selectTeam(page, team.id);

  const schedules = page.locator('[data-glance-card="schedules"]');
  await expect(schedules.locator("table")).toBeVisible();
  await expect(schedules.locator(".stat")).toHaveCount(1);
  const hosts = page.locator('[data-glance-card="hosts"]');
  await expect(hosts.locator("[data-glance-healthy]")).toBeVisible();
  const scheduleBox = await schedules.boundingBox();
  const hostBox = await hosts.boundingBox();
  expect(Math.abs(hostBox.y - scheduleBox.y), "the mixed cards do not share one row").toBeLessThanOrEqual(1);
  expect(hostBox.height, "the healthy card stretched to the taller schedule card").toBeLessThan(scheduleBox.height);
  expect(hostBox.height, `the healthy host card is ${hostBox.height}px tall`).toBeLessThanOrEqual(200);
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
  await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();

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

test("an empty team shows only onboarding and advances the next step", async ({ page }) => {
  test.setTimeout(60000);
  await signIn(page);
  const suffix = Date.now().toString(36);
  const team = await createTeam(page, `Zz onboarding ${suffix}`);
  await selectTeam(page, team.id);

  await expect(page.locator("[data-onboarding]")).toBeVisible();
  await expect(page.locator("[data-glance-card]")).toHaveCount(0);
  await expect(page.locator("[data-webhook-deliveries]")).toHaveCount(0);
  await expect(page.locator('[data-onboarding-step="projects"] .btn.primary')).toBeVisible();

  const created = await post(page.request, "/api/v1/projects", {
    headers: { "X-Team-Id": team.id },
    data: {
      name: `Onboarding project ${suffix}`,
      source_type: "path",
      source: join(root, "examples/demo"),
    },
  });
  expect(created.status()).toBe(201);

  await page.reload();
  await expect(page.locator('[data-onboarding-step="projects"]')).toHaveClass(/done/);
  await expect(page.locator('[data-onboarding-step="inventories"] .btn.primary')).toBeVisible();
  await noHorizontalScroll(page);
});
