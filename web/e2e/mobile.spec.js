"use strict";

const { expect, test } = require("@playwright/test");
const { password, username } = require("./env");
const { post, storeAssistantKey } = require("./api");

async function noHorizontalScroll(page) {
  await expect.poll(
    () => page.evaluate(() => document.scrollingElement.scrollWidth <= window.innerWidth),
    { message: "the page scrolls sideways" },
  ).toBe(true);
}

async function expectTouchTarget(locator, name, { square = true } = {}) {
  const box = await locator.boundingBox();
  expect(box, `${name} is rendered`).not.toBeNull();
  expect(box.height, `${name} is at least 44 px high`).toBeGreaterThanOrEqual(44);
  if (square) expect(box.width, `${name} is at least 44 px wide`).toBeGreaterThanOrEqual(44);
}

async function signIn(page) {
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Sprache" })).toBeVisible();
  await page.getByRole("button", { name: "Sprache" }).click();
  await page.getByRole("option", { name: /English/ }).click();
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();
  await noHorizontalScroll(page);
}

async function signInGerman(page) {
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Sprache" })).toBeVisible();
  await page.getByLabel("Benutzername").fill(username);
  await page.getByLabel("Passwort").fill(password);
  await page.getByRole("button", { name: "Anmelden", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Betrieb" })).toBeVisible();
  await noHorizontalScroll(page);
}

async function openMenu(page, label = "Menu") {
  const toggle = page.getByRole("button", { name: label, exact: true });
  if ((await toggle.getAttribute("aria-expanded")) !== "true") await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
}

async function goTo(page, label, menu = "Menu") {
  await openMenu(page, menu);
  await page.getByRole("navigation").getByRole("link", { name: label, exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: label })).toBeVisible();
  await noHorizontalScroll(page);
}

async function expectPhoneShell(page, where) {
  const toggle = page.getByRole("button", { name: "Menu", exact: true });
  await expect(toggle, `${where} has no hamburger`).toBeVisible();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  const columns = await page.locator(".shell").evaluate((element) => getComputedStyle(element).gridTemplateColumns);
  expect(columns.trim().split(/\s+/), `${where} keeps a desktop sidebar column`).toHaveLength(1);
  const side = await page.locator(".side").boundingBox();
  expect(side, `${where} renders the sidebar`).not.toBeNull();
  expect(side.x + side.width, `${where} shows the off-canvas sidebar`).toBeLessThanOrEqual(1);
  const main = await page.locator(".main").boundingBox();
  expect(main.x, `${where} insets the main column for the sidebar`).toBeLessThanOrEqual(1);
}

async function expectViewOnlyEditor(page, where) {
  await expect(page.locator("#wf-desktop-note"), `${where} hides the desktop note`).toBeVisible();
  await expect(page.locator("#wf-desktop-note")).toContainText("desktop");
  await expect(page.getByRole("button", { name: "Save", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Launch", exact: true })).toHaveCount(0);
  await expect(page.locator("#wf-tab-danger")).toHaveCount(0);
  await expect(page.locator("#wf-delete")).toHaveCount(0);
  await expect(page.locator("#wf-name")).toBeDisabled();
  await expect(page.locator("#canvas")).not.toHaveClass(/editable/);
  await expect(page.locator("[data-node-menu]")).toHaveCount(0);
  await expect(page.locator("[data-add-port]")).toHaveCount(0);
  await noHorizontalScroll(page);
}

async function readCanvasControlsLayout(page) {
  return page.evaluate(() => {
    const controls = document.querySelector(".canvas-controls");
    const panel = document.querySelector(".inspector-panel");
    const c = controls.getBoundingClientRect();
    const p = panel.getBoundingClientRect();
    const hit = document.elementFromPoint(c.left + c.width / 2, c.top + c.height / 2);
    return {
      overlap: !(c.right <= p.left || c.left >= p.right || c.bottom <= p.top || c.top >= p.bottom),
      insideViewport: c.top >= 0 && c.bottom <= window.innerHeight,
      hitInsideControls: Boolean(hit && hit.closest(".canvas-controls")),
      controls: { top: c.top, bottom: c.bottom, left: c.left, right: c.right },
      panel: { top: p.top, bottom: p.bottom, left: p.left, right: p.right },
    };
  });
}

async function readPairLayout(list) {
  return list.evaluate((element) => [...element.querySelectorAll("dt")].map((term) => {
    const label = term.getBoundingClientRect();
    const value = term.nextElementSibling.getBoundingClientRect();
    return { label: label.toJSON(), value: value.toJSON(), text: term.textContent };
  }));
}

async function expectStackedPairs(list, width) {
  const pairs = await readPairLayout(list);
  expect(pairs.length).toBeGreaterThanOrEqual(4);
  for (const pair of pairs) {
    expect(pair.value.top, `${pair.text} stays below its label at ${width}px`).toBeGreaterThanOrEqual(pair.label.bottom - 1);
    expect(Math.abs(pair.value.left - pair.label.left), `${pair.text} shares its label column at ${width}px`).toBeLessThanOrEqual(1);
  }
  const terms = await list.evaluate((element) => [...element.querySelectorAll("dt")].map((term) => ({
    text: term.textContent,
    overflow: term.scrollWidth - term.clientWidth,
  })));
  for (const term of terms) {
    expect(term.overflow, `${term.text} is clipped at ${width}px`).toBeLessThanOrEqual(1);
  }
}

async function launchGeometry(page) {
  return page.evaluate(() => {
    const dialog = document.querySelector("#launch-prompt");
    const body = dialog.querySelector(".dialog-body");
    const actions = dialog.querySelector(".dialog-actions");
    const limit = dialog.querySelector('input[name="limit"]');
    const cancel = dialog.querySelector("[data-dialog-cancel]");
    const submit = dialog.querySelector('button[type="submit"]');
    const box = (node) => {
      const rect = node.getBoundingClientRect();
      return { top: rect.top, bottom: rect.bottom };
    };
    return {
      viewport: { height: window.innerHeight, width: window.innerWidth },
      body: { ...box(body), scrollTop: body.scrollTop, scrollWidth: body.scrollWidth, clientWidth: body.clientWidth },
      actions: box(actions),
      limit: box(limit),
      cancel: box(cancel),
      submit: box(submit),
    };
  });
}

async function expectLimitAboveActions(page, where) {
  await expect.poll(
    async () => {
      const geometry = await launchGeometry(page);
      return Math.round(geometry.actions.top - geometry.limit.bottom);
    },
    { message: `the launch dialog action bar covers the limit field at ${where}` },
  ).toBeGreaterThanOrEqual(1);
  const geometry = await launchGeometry(page);
  expect(geometry.limit.bottom, `the limit field sits inside the scrolling body at ${where}`).toBeLessThanOrEqual(geometry.body.bottom + 1);
  expect(geometry.cancel.top, `Cancel stays below the scrolling body at ${where}`).toBeGreaterThanOrEqual(geometry.body.bottom - 1);
  expect(geometry.cancel.bottom, `Cancel stays inside the viewport at ${where}`).toBeLessThanOrEqual(geometry.viewport.height + 1);
  expect(geometry.submit.bottom, `Launch stays inside the viewport at ${where}`).toBeLessThanOrEqual(geometry.viewport.height + 1);
  expect(geometry.body.scrollWidth, `the launch dialog body does not scroll sideways at ${where}`).toBeLessThanOrEqual(geometry.body.clientWidth + 1);
  await noHorizontalScroll(page);
}

async function teamHeaders(page) {
  const profile = await page.evaluate(async () => (await fetch("/api/v1/me")).json());
  return { "X-Team-Id": profile.teams[0].id };
}

async function createShellTemplate(page, name, script, prompt = false, description = "") {
  const created = await post(page.request, "/api/v1/templates", {
    headers: await teamHeaders(page),
    data: { name, description, tool: "shell", spec: { script }, prompt_on_launch: prompt },
  });
  expect(created.status()).toBe(201);
  return created.json();
}

async function createPartialAnsibleTemplate(page, name) {
  const headers = await teamHeaders(page);
  const inventory = await post(page.request, "/api/v1/inventories", {
    headers,
    data: { name: `Partial hosts ${Date.now().toString(36)}`, body: "[web]\nweb-1\n" },
  });
  expect(inventory.status()).toBe(201);
  const row = await inventory.json();
  const created = await post(page.request, "/api/v1/templates", {
    headers,
    data: {
      name,
      tool: "ansible",
      entrypoint: "site.yml",
      inventory_id: row.id,
      no_environment: true,
      prompt_fields: ["limit"],
      prompt_on_launch: false,
      spec: { job_type: "run", tags: "ping", limit: "web" },
    },
  });
  expect(created.status()).toBe(201);
  return created.json();
}

async function createSchedule(page, name) {
  const headers = await teamHeaders(page);
  const templates = await (await page.request.get("/api/v1/templates", { headers })).json();
  const health = templates.find((item) => item.name === "Healthcheck");
  expect(health).toBeTruthy();
  const created = await post(page.request, `/api/v1/templates/${health.id}/schedules`, {
    headers,
    data: { name, timezone: "UTC", start_date: "2027-06-01", start_time: "06:00", frequency: "daily", end_kind: "never" },
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
  await expect(row.locator('td[data-label="Name"]')).toContainText(template.name);
  await expect(row.locator('td[data-label="Tool"]')).toBeHidden();
  await expect(row.getByRole("button", { name: "Launch", exact: true })).toBeVisible();

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

test("shows the whole template description in the edit overlay on a phone", async ({ page }) => {
  const suffix = `${test.info().project.name}-${Date.now()}`.replace(/\s+/g, "-");
  const description = "Canary checks for the edge fleet: verify the load balancer, the replica lag & certificate expiry before the rollout moves to the next region.";
  await signInGerman(page);
  const template = await createShellTemplate(page, `Mobile description ${suffix}`, "#!/bin/sh\ntrue\n", false, description);

  const check = async (language, viewport) => {
    const where = `${language} ${viewport.width}x${viewport.height}`;
    const heading = language === "de" ? "Vorlage bearbeiten" : "Edit template";
    await page.setViewportSize(viewport);
    await page.goto(`/#/templates?edit=${template.id}`);
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("heading", { name: heading })).toBeVisible();
    const field = dialog.locator('textarea[name="description"]');
    await expect(field, `the description is not a wrapping field at ${where}`).toBeVisible();
    await expect(field).toHaveValue(description);
    await expect(field).toHaveAttribute("maxlength", "1000");
    const geometry = await field.evaluate((element) => ({
      clientHeight: element.clientHeight,
      clientWidth: element.clientWidth,
      scrollHeight: element.scrollHeight,
      scrollWidth: element.scrollWidth,
      value: element.value,
    }));
    expect(geometry.scrollWidth, `the description clips sideways at ${where}`).toBeLessThanOrEqual(geometry.clientWidth + 1);
    expect(geometry.scrollHeight, `the description is cut off at ${where}`).toBeLessThanOrEqual(geometry.clientHeight + 1);
    expect(geometry.value, `the description loses text at ${where}`).toBe(description);
    await noHorizontalScroll(page);

    const body = dialog.locator(".dialog-body");
    expect(await body.evaluate((element) => getComputedStyle(element).overflowY), `the overlay stops scrolling at ${where}`).toBe("auto");
    const fullHeight = viewport.height;
    await page.setViewportSize({ width: viewport.width, height: 480 });
    await expect.poll(
      () => body.evaluate((element) => element.scrollHeight > element.clientHeight),
      { message: `the overlay does not scroll its body at ${where}` },
    ).toBe(true);
    const scrolled = await body.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
      return element.scrollTop;
    });
    expect(scrolled, `the overlay body does not scroll at ${where}`).toBeGreaterThan(0);
    await page.setViewportSize({ width: viewport.width, height: fullHeight });
    await noHorizontalScroll(page);
    await dialog.getByRole("button", { name: language === "de" ? "Schließen" : "Close" }).click();
    await expect(dialog).toHaveCount(0);
  };

  const viewports = [{ width: 360, height: 732 }, { width: 412, height: 923 }];
  for (const viewport of viewports) await check("de", viewport);

  await openMenu(page, "Menü");
  await page.getByRole("button", { name: "Sprache" }).click();
  await page.getByRole("option", { name: /English/ }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Templates" })).toBeVisible();
  for (const viewport of viewports) await check("en", viewport);
});

test("keeps the launch dialog limit field clear of the action bar", async ({ page }) => {
  const viewports = [{ width: 360, height: 732 }, { width: 412, height: 923 }];
  await signInGerman(page);
  await goTo(page, "Vorlagen", "Menü");

  const openLaunchDialog = async (language) => {
    const row = page.locator("#template-table tbody tr").filter({ hasText: "Self-Heal" }).first();
    await expect(row).toBeVisible();
    await row.getByRole("button", { name: language === "de" ? "Mit Optionen starten" : "Launch with options", exact: true }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("heading", { name: language === "de" ? "Starten" : "Launch", exact: true })).toBeVisible();
    await dialog.locator('input[name="limit"]').focus();
  };

  const check = async (language) => {
    for (const viewport of viewports) {
      await page.setViewportSize(viewport);
      await openLaunchDialog(language);
      await expectLimitAboveActions(page, `${language} ${viewport.width}x${viewport.height}`);
      // The virtual keyboard shrinks the viewport while Limit keeps focus.
      await page.setViewportSize({ width: viewport.width, height: Math.round(viewport.height * 0.6) });
      await expectLimitAboveActions(page, `${language} ${viewport.width}x${viewport.height} with keyboard`);
      await page.getByRole("dialog").getByRole("button", { name: language === "de" ? "Abbrechen" : "Cancel", exact: true }).click();
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await page.setViewportSize(viewport);
    }
  };

  await check("de");
  await openMenu(page, "Menü");
  await page.getByRole("button", { name: "Sprache" }).click();
  await page.getByRole("option", { name: /English/ }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Templates" })).toBeVisible();
  await check("en");
});

test("keeps schedules readable and its touch targets at least 44 px", async ({ page }) => {
  const suffix = `${test.info().project.name}-${Date.now()}`.replace(/\s+/g, "-");
  await signIn(page);
  const schedule = await createSchedule(page, `Mobile schedule ${suffix}`);
  await goTo(page, "Schedules");
  const row = page.locator(".schedule-table tbody tr").filter({ hasText: schedule.name }).first();
  await expect(row).toBeVisible();
  await expect(row.locator('td[data-label="Name"]')).toContainText(schedule.name);

  await openMenu(page);
  const navBox = await page.getByRole("navigation").getByRole("link", { name: "Templates", exact: true }).boundingBox();
  expect(navBox.height).toBeGreaterThanOrEqual(44);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: "Menu", exact: true })).toHaveAttribute("aria-expanded", "false");

  const compact = await row.locator(".btn.compact").first().boundingBox();
  expect(compact.height).toBeGreaterThanOrEqual(44);
  expect(compact.width).toBeGreaterThanOrEqual(44);
  const target = await row.locator(".switch").first().boundingBox();
  expect(target.height).toBeGreaterThanOrEqual(44);
  expect(target.width).toBeGreaterThanOrEqual(44);
  await noHorizontalScroll(page);
});

test("keeps every phone touch target at least 44 px", async ({ page }) => {
  test.setTimeout(90000);
  const suffix = `${test.info().project.name}-${Date.now()}`.replace(/\s+/g, "-");
  await signIn(page);
  const schedule = await createSchedule(page, `Touch targets ${suffix}`);
  const jobTemplate = await createShellTemplate(page, `Touch log ${suffix}`, "#!/bin/sh\necho \"touch log ok\"\n");
  const headers = await teamHeaders(page);
  const launched = await post(page.request, `/api/v1/templates/${jobTemplate.id}/launch`, { headers, data: { extra_vars: {} } });
  expect(launched.status()).toBe(202);
  const job = await launched.json();

  for (const viewport of [{ width: 360, height: 732 }, { width: 412, height: 923 }]) {
    const where = `${viewport.width}x${viewport.height}`;
    await page.setViewportSize(viewport);

    await openMenu(page);
    await expectTouchTarget(page.getByRole("navigation").getByRole("link", { name: "Templates", exact: true }), `${where} nav link`);
    await page.keyboard.press("Escape");
    await expect(page.getByRole("button", { name: "Menu", exact: true })).toHaveAttribute("aria-expanded", "false");

    await goTo(page, "Schedules");
    const scheduleRow = page.locator(".schedule-table tbody tr").filter({ hasText: schedule.name }).first();
    await expect(scheduleRow).toBeVisible();
    await expectTouchTarget(scheduleRow.locator(".btn.compact").first(), `${where} compact button`);
    await expectTouchTarget(scheduleRow.locator(".switch").first(), `${where} switch`);

    await goTo(page, "Templates");
    await page.locator("#template-table tbody tr").filter({ hasText: "Self-Heal" }).first().getByRole("link").first().click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("heading", { name: "Edit template" })).toBeVisible();
    await expectTouchTarget(dialog.getByRole("tab", { name: "General", exact: true }), `${where} dialog tab`);
    await expectTouchTarget(dialog.locator(".field.check").filter({ hasText: "Prompt on launch" }), `${where} checkbox row`);
    await dialog.getByRole("tab", { name: "Target", exact: true }).click();
    await expectTouchTarget(dialog.locator('select[name="project_id"]'), `${where} dialog select`);
    await dialog.getByRole("tab", { name: "Options", exact: true }).click();
    await expectTouchTarget(dialog.locator(".chip-entry").first(), `${where} chip entry`);
    await dialog.getByRole("button", { name: "Close" }).click();

    await page.goto(`/#/jobs/${job.id}`);
    await expect(page.locator("#log-wrap")).toBeVisible({ timeout: 30000 });
    await expectTouchTarget(page.locator("label:has(#log-wrap)"), `${where} log wrap checkbox`);
    await noHorizontalScroll(page);

    await openMenu(page);
    await page.getByRole("navigation").getByRole("link", { name: "Overview", exact: true }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();
    await expectTouchTarget(page.getByRole("tab", { name: "Overview", exact: true }), `${where} overview tab`);
    await page.getByRole("tab", { name: "Dashboard", exact: true }).click();
    await expect(page.getByRole("heading", { level: 2, name: "Jobs per hour" })).toBeVisible();
    await expectTouchTarget(page.getByRole("button", { name: "Today", exact: true }), `${where} range segment`);
    await page.getByRole("button", { name: "14 days", exact: true }).click();
    await expect(page.getByRole("heading", { level: 2, name: "Jobs per day" })).toBeVisible();
    const days = page.locator("[data-chart-day]");
    await expect(days).toHaveCount(14);
    const boxes = await days.evaluateAll((elements) => elements.map((element) => {
      const rect = element.getBoundingClientRect();
      return { height: rect.height, width: rect.width };
    }));
    for (const [index, box] of boxes.entries()) {
      expect(box.width, `${where} chart day ${index + 1} is at least 44 px wide`).toBeGreaterThanOrEqual(44);
      expect(box.height, `${where} chart day ${index + 1} is at least 44 px high`).toBeGreaterThanOrEqual(44);
    }
    const scrolls = await page.locator(".chart").evaluateAll((charts) => charts.map((chart) => ({
      left: chart.scrollLeft,
      max: chart.scrollWidth - chart.clientWidth,
      visibleWidth: chart.clientWidth,
    })));
    for (const scroll of scrolls) {
      expect(scroll.max, `${where} the 14-day chart scrolls on a phone`).toBeGreaterThan(0);
      expect(scroll.visibleWidth, `${where} the chart still shows a group of days`).toBeGreaterThan(44 * 4);
      expect(scroll.left, `${where} the chart opens on the newest day`).toBe(scroll.max);
    }
    const detail = page.locator("[data-chart-detail]").first();
    const chart = page.locator(".chart").first();
    await chart.locator("[data-chart-day]").last().focus();
    await expect(detail).toContainText(/started|successful|failed|canceled/i);
    await chart.locator("[data-chart-day]").last().click();
    await expect(page).toHaveURL(/#\/jobs\?.*since=/);
    await noHorizontalScroll(page);
    await page.goto("/#/");
    await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();

    await openMenu(page);
    await page.getByRole("button", { name: "Language" }).click();
    const option = page.getByRole("option", { name: /English/ });
    await expectTouchTarget(option, `${where} language option`);
    await option.click();
  }
});

test("shows an inventory host list with a compact sort control", async ({ page }) => {
  await signIn(page);
  await goTo(page, "Inventories");
  await page.getByRole("link", { name: "local", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "local" })).toBeVisible();
  const sort = page.locator("#host-sort");
  await expect(sort).toBeVisible();
  await sort.selectOption("-name");
  await expect.poll(() => new URL(page.url()).hash).toContain("sort=-name");
  const host = page.locator("#host-table tbody tr").filter({ hasText: "localhost" }).first();
  await expect(host).toBeVisible();
  await expect(host.locator('td[data-label="Name"]')).toContainText("localhost");
  await noHorizontalScroll(page);
});

test("opens the template edit overlay fullscreen with scrollable tabs", async ({ page }) => {
  await signIn(page);
  await goTo(page, "Templates");
  await page.getByRole("link", { name: "Healthcheck", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "Edit template" })).toBeVisible();
  const viewport = page.viewportSize();
  const box = await dialog.boundingBox();
  expect(Math.round(box.width)).toBeGreaterThanOrEqual(viewport.width - 1);
  expect(Math.round(box.height)).toBeGreaterThanOrEqual(viewport.height - 1);

  const body = dialog.locator(".dialog-body");
  expect(await body.evaluate((element) => getComputedStyle(element).overflowY)).toBe("auto");
  const narrowViewport = { width: viewport.width, height: 500 };
  await page.setViewportSize(narrowViewport);
  const scroller = await body.evaluate((element) => ({
    scrollHeight: element.scrollHeight,
    clientHeight: element.clientHeight,
  }));
  expect(scroller.scrollHeight).toBeGreaterThan(scroller.clientHeight);
  await body.evaluate((element) => { element.scrollTop = element.scrollHeight; });
  expect(await body.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);

  await dialog.getByRole("tab", { name: "Schedules", exact: true }).click();
  await expect(dialog.getByRole("button", { name: "+ Schedule", exact: true })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Save", exact: true })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Cancel", exact: true })).toBeVisible();
  await noHorizontalScroll(page);
  await dialog.getByRole("button", { name: "Close" }).click();
  await expect(dialog).toHaveCount(0);
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
  const launched = await post(page.request, `/api/v1/templates/${health.id}/launch`, { headers, data: { extra_vars: {} } });
  expect(launched.status()).toBe(202);
  const job = await launched.json();
  await expect.poll(async () => {
    const response = await page.request.get(`/api/v1/jobs/${job.id}`, { headers });
    return (await response.json()).status;
  }, { timeout: 30000 }).toMatch(/^(successful|failed|canceled)$/);

  await page.getByRole("tab", { name: "Dashboard", exact: true }).click();
  await expect(page.getByRole("heading", { level: 2, name: "Jobs per hour" })).toBeVisible();
  const counts = page.locator(".chart-day-total");
  await expect(counts.first()).toBeVisible();
  const texts = (await counts.allTextContents()).map((text) => text.trim());
  expect(texts).not.toHaveLength(0);
  expect(texts.every((text) => text === "" || /^\d+$/.test(text))).toBe(true);
  expect(texts.some((text) => Number(text) > 0)).toBe(true);
  const series = await page.evaluate(async () => {
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
    const team = localStorage.getItem("koreograf.team");
    const response = await fetch(`/api/v1/dashboard?days=1&tz=${encodeURIComponent(tz)}`, {
      headers: team ? { "X-Team-Id": team } : {},
    });
    return (await response.json()).series;
  });
  const totals = series.map((row) => Math.max(row.started, row.successful + row.failed + row.canceled));
  expect(texts.map((text) => (text === "" ? 0 : Number(text)))).toEqual(totals);
  const day = page.locator("[data-chart-day]").first();
  const detail = page.locator("[data-chart-detail]").first();
  await day.focus();
  await expect(detail).toContainText(/started|successful|failed|canceled/i);
  await day.click();
  await expect(page).toHaveURL(/#\/jobs\?.*since=/);
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
  await expect(page.locator("#wf-tab-danger")).toHaveCount(0);
  await expect(page.locator("#wf-name")).toBeDisabled();
  await noHorizontalScroll(page);

  const node = page.locator('.wf-node[data-id="check"]');
  await node.scrollIntoViewIfNeeded();
  await node.click();
  await expect(page.locator("#inspector")).toContainText("Healthcheck");
  await noHorizontalScroll(page);
});

test("stacks the workflow inspector key/value rows on narrow phones", async ({ page }) => {
  await signInGerman(page);
  await goTo(page, "Workflows", "Menü");
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  await expect(page.locator("#wf-desktop-note")).toBeVisible();

  const node = page.locator('.wf-node[data-id="check"]');
  await node.scrollIntoViewIfNeeded();
  await node.click();
  const general = page.locator("#inspector .node-section-body .readonly-kv").first();
  await expect(general).toContainText("Zusammenführung");

  const viewports = [{ width: 360, height: 732 }, { width: 412, height: 923 }];
  for (const viewport of viewports) {
    await page.setViewportSize(viewport);
    await expect(general).toBeVisible();
    await expectStackedPairs(general, viewport.width);
    await noHorizontalScroll(page);
  }

  await openMenu(page, "Menü");
  await page.getByRole("button", { name: "Sprache" }).click();
  await page.getByRole("option", { name: /English/ }).click();
  await expect(page.locator("#wf-desktop-note")).toContainText("desktop");
  await node.scrollIntoViewIfNeeded();
  await node.click();
  const english = page.locator("#inspector .node-section-body .readonly-kv").first();
  await expect(english).toContainText("Convergence");
  for (const viewport of viewports) {
    await page.setViewportSize(viewport);
    await expect(english).toBeVisible();
    await expectStackedPairs(english, viewport.width);
    await noHorizontalScroll(page);
  }

  await page.setViewportSize({ width: 1280, height: 900 });
  const desktop = await readPairLayout(english);
  for (const pair of desktop) {
    expect(pair.value.left, `${pair.text} leaves the two-column layout on a desktop`).toBeGreaterThan(pair.label.left);
    expect(pair.value.top, `${pair.text} is not on the label row on a desktop`).toBeLessThan(pair.label.bottom);
  }
});

test("lists activity and narrows it to one action", async ({ page }) => {
  const suffix = `${test.info().project.name}-${Date.now()}`.replace(/\s+/g, "-");
  await signIn(page);
  const template = await createShellTemplate(page, `Activity mobile ${suffix}`, "#!/bin/sh\ntrue\n");
  const headers = await teamHeaders(page);
  const launched = await post(page.request, `/api/v1/templates/${template.id}/launch`, { headers, data: { extra_vars: {} } });
  expect(launched.status()).toBe(202);

  await goTo(page, "Activity");
  const rows = page.locator("#activity-table tbody tr");
  const probe = rows.filter({ hasText: template.name });
  await expect(probe).toHaveCount(2);
  await expect(probe.filter({ hasText: "Created" })).toHaveCount(1);
  await expect(probe.filter({ hasText: "Launched" })).toHaveCount(1);
  await expect(rows.first().locator('td[data-label="Object"]')).toBeVisible();

  await page.locator("#activity-action").selectOption("launch");
  await expect(probe).toHaveCount(1);
  await expect(probe.first()).toContainText("Launched");
  await noHorizontalScroll(page);
});

test("keeps the workflow editor view-only on a landscape phone and after rotation", async ({ page }) => {
  test.setTimeout(90000);
  await signIn(page);
  await goTo(page, "Workflows");
  const portrait = page.viewportSize();
  const deviceLandscape = { width: portrait.height, height: portrait.width };
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  await expect(page.locator("#wf-desktop-note")).toBeVisible();

  const node = page.locator('.wf-node[data-id="check"]');
  const viewports = [
    { ...portrait, where: "portrait" },
    { ...deviceLandscape, where: "device landscape" },
    { width: 923, height: 412, where: "Pixel 9 landscape" },
    { ...portrait, where: "portrait after rotation" },
  ];

  for (const viewport of viewports) {
    const where = `${viewport.where} ${viewport.width}x${viewport.height}`;
    await page.setViewportSize({ width: viewport.width, height: viewport.height });
    await expectPhoneShell(page, where);
    await expectViewOnlyEditor(page, where);

    await node.evaluate((element) => element.scrollIntoView({ block: "start" }));
    const style = await node.getAttribute("style");
    const box = await node.boundingBox();
    expect(box, `${where} renders the workflow node`).not.toBeNull();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 48, box.y + box.height / 2 + 36, { steps: 8 });
    await page.mouse.up();
    await expect.poll(() => node.getAttribute("style"), { message: `${where} moves a node` }).toBe(style);

    await expect(page.locator("#inspector")).toBeVisible();
    await expect(page.locator("#inspector")).toContainText("Healthcheck");
    await noHorizontalScroll(page);
  }
});

test("lists sessions on the account page and ends another session", async ({ page }) => {
  await signIn(page);
  const second = await post(page.request, "/api/v1/login", {
    data: { username, password },
  });
  expect(second.ok()).toBe(true);

  await openMenu(page);
  await page.locator(".account-link").click();
  await expect(page.getByRole("heading", { level: 1, name: "My account" })).toBeVisible();
  await noHorizontalScroll(page);

  const sessions = page.locator("section.card").filter({
    has: page.getByRole("heading", { name: "Sessions", exact: true }),
  });
  const rows = sessions.locator("tbody tr");
  await expect(rows.first()).toBeVisible();
  const before = await rows.count();
  expect(before).toBeGreaterThanOrEqual(2);
  const current = rows.filter({ hasText: "This session" });
  await expect(current).toHaveCount(1);
  await expect(current.locator('td[data-label="Created"]')).toContainText("This session");

  const other = rows.filter({ hasNotText: "This session" }).first();
  const button = other.getByRole("button", { name: "End session", exact: true });
  await expect(button).toBeVisible();
  await expectTouchTarget(button, "End session");
  await button.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "End session", exact: true })).toBeVisible();
  await dialog.getByRole("button", { name: "End session", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(rows).toHaveCount(before - 1);
  await expect(rows.filter({ hasText: "This session" })).toHaveCount(1);
  await noHorizontalScroll(page);
});

test("keeps the landscape workflow inspector clear of the canvas toolbar", async ({ page }) => {
  test.setTimeout(90000);
  await signIn(page);
  await goTo(page, "Workflows");
  await page.getByRole("link", { name: "Self-Heal Ablauf", exact: true }).click();
  await expect(page.locator("#wf-desktop-note")).toBeVisible();

  const node = page.locator('.wf-node[data-id="check"]');
  const viewports = [
    { width: 923, height: 412 },
    { width: 732, height: 360 },
    { width: 756, height: 308 },
  ];

  for (const viewport of viewports) {
    const where = `${viewport.width}x${viewport.height}`;
    await page.setViewportSize(viewport);
    await expectPhoneShell(page, where);
    await expectViewOnlyEditor(page, where);

    await node.evaluate((element) => element.scrollIntoView({ block: "start" }));
    await node.click();
    await expect(page.locator("#inspector")).toContainText("Healthcheck");

    await page.locator(".canvas-controls").scrollIntoViewIfNeeded();
    const layout = await readCanvasControlsLayout(page);
    expect(layout.insideViewport, `${where} keeps the canvas toolbar in the viewport`).toBe(true);
    expect(layout.overlap, `${where} puts the inspector over the canvas toolbar`).toBe(false);
    expect(layout.hitInsideControls, `${where} lets the inspector intercept the canvas toolbar`).toBe(true);

    const zoom = await page.locator("#canvas").getAttribute("data-zoom");
    await page.getByRole("button", { name: "Zoom in", exact: true }).click();
    await expect.poll(() => page.locator("#canvas").getAttribute("data-zoom"), { message: `${where} cannot zoom the canvas` }).not.toBe(zoom);
    await noHorizontalScroll(page);
  }
});

test("keeps the remove checkbox beside its label and hides the vault hint off Ansible", async ({ page }) => {
  test.setTimeout(60000);
  const vaultHint = "One credential of each kind. Search a vault credential; it appears here as a pill.";
  await signIn(page);

  const saved = await storeAssistantKey(page);
  expect(saved.status()).toBe(200);
  await goTo(page, "Settings");
  await page.getByRole("tab", { name: "Assistant", exact: true }).click();
  const remove = page.getByRole("checkbox", { name: "Remove", exact: true });
  await expect(remove).toBeVisible();
  const row = page.locator("label.field.check").filter({ hasText: "Remove" });
  await expectTouchTarget(row, "remove secret row");
  const layout = await remove.evaluate((input) => {
    const label = input.closest("label.field.check");
    const text = label.querySelector("span");
    const box = (node) => {
      const rect = node.getBoundingClientRect();
      return { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right };
    };
    return {
      display: getComputedStyle(label).display,
      input: box(input),
      text: box(text),
    };
  });
  expect(layout.display).toBe("flex");
  expect(Math.abs((layout.input.top + layout.input.bottom) / 2 - (layout.text.top + layout.text.bottom) / 2)).toBeLessThanOrEqual(1);
  expect(layout.text.left).toBeGreaterThanOrEqual(layout.input.right - 1);
  await noHorizontalScroll(page);

  await goTo(page, "Templates");
  const openTarget = async (name) => {
    await page.locator("#template-search").fill(name);
    await page.getByRole("link", { name, exact: true }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("heading", { name: "Edit template" })).toBeVisible();
    await dialog.getByRole("tab", { name: "Target", exact: true }).click();
    return dialog;
  };

  const shell = await openTarget("Healthcheck");
  await expect(shell.getByText(vaultHint, { exact: true })).toHaveCount(0);
  await noHorizontalScroll(page);
  await shell.getByRole("button", { name: "Close" }).click();
  await expect(shell).toHaveCount(0);

  const ansible = await openTarget("Self-Heal");
  await expect(ansible.getByText(vaultHint, { exact: true })).toBeVisible();
  await noHorizontalScroll(page);
});

test("the overview shows kpis, attention and recent activity on a phone", async ({ page }) => {
  test.setTimeout(60000);
  const suffix = `${test.info().project.name}-${Date.now()}`.replace(/\s+/g, "-");
  await signIn(page);
  const schedule = await createSchedule(page, `Glance schedule ${suffix}`);
  await page.goto("/#/");
  await page.reload();
  await expect(page.getByRole("heading", { level: 1, name: "Operations" })).toBeVisible();

  const kpis = page.locator("[data-kpi-row] [data-kpi]");
  await expect(kpis).toHaveCount(5);
  await expect(page.locator('[data-kpi="running"]')).toHaveAttribute("href", "#/jobs?status=running");
  await expect(page.locator('[data-kpi="pending"]')).toHaveAttribute("href", "#/jobs?status=pending");
  await expect(page.locator('[data-kpi="failed-24h"]')).toHaveAttribute("href", "#/jobs?status=failed");
  await expectTouchTarget(page.locator('[data-kpi="running"]'), "the running kpi", { square: false });

  const attention = page.locator("[data-attention]");
  await expect(attention).toBeVisible();
  await expect(page.locator("[data-recent-activity]")).toBeVisible();
  await expect(page.locator('[data-glance-card="cache"]')).toHaveCount(0);
  await expect(page.locator('[data-glance-card="schedules"]')).toContainText(schedule.name);
  await expect(page.locator('[data-glance-card="hosts"]')).toContainText("Problem hosts");
  await expect(page.locator('[data-glance-card="syncs"]')).toContainText("Failed syncs");
  await expect(page.locator('[data-glance-card="workflows"]')).toContainText("Workflow runs");

  const columns = await page.locator(".overview-main").evaluate((node) => getComputedStyle(node).gridTemplateColumns);
  expect(columns.trim().split(/\s+/), "the overview keeps two columns on a phone").toHaveLength(1);
  await noHorizontalScroll(page);

  for (const lang of ["de", "en"]) {
    for (const theme of ["dark", "light"]) {
      await page.evaluate(([language, mode]) => {
        localStorage.setItem("koreograf.lang", language);
        localStorage.setItem("koreograf.theme", mode);
      }, [lang, theme]);
      await page.reload();
      await expect(page.getByRole("heading", { level: 1, name: lang === "de" ? "Betrieb" : "Operations" })).toBeVisible();
      await expect(page.locator('[data-kpi="failed-24h"]')).toBeVisible();
      await noHorizontalScroll(page);
    }
  }

  await page.locator('[data-glance-card="schedules"]').getByRole("link", { name: "View schedules", exact: true }).first().click();
  await expect(page.getByRole("heading", { level: 1, name: "Schedules" })).toBeVisible();
  await noHorizontalScroll(page);
});

test("a phone prompts for one field and keeps the prompt checkboxes usable", async ({ page }) => {
  test.setTimeout(60000);
  const suffix = `${test.info().project.name}-${Date.now()}`.replace(/\s+/g, "-");
  await signIn(page);
  const template = await createPartialAnsibleTemplate(page, `Mobile partial ${suffix}`);

  await goTo(page, "Templates");
  const row = page.locator("#template-table tbody tr").filter({ hasText: template.name }).first();
  await row.getByRole("button", { name: "Launch", exact: true }).click();
  const dialog = page.locator("#launch-prompt");
  await expect(dialog).toBeVisible();
  await expect(dialog.locator('input[name="limit"]')).toHaveValue("web");
  await expect(dialog.locator('select[name="inventory_id"]')).toHaveCount(0);
  await expect(dialog.locator("[data-credential-field]")).toHaveCount(0);
  await expect(dialog.locator('textarea[name="extra_vars"]')).toHaveCount(0);
  await expect(dialog.locator('input[name="tags"]')).toHaveCount(0);
  await dialog.locator('input[name="limit"]').fill("db");
  await dialog.getByRole("button", { name: "Launch", exact: true }).click();
  await expect(page).toHaveURL(/#\/jobs\/[0-9a-f-]+$/);

  const jobId = new URL(page.url()).hash.match(/^#\/jobs\/(.+)$/)[1];
  const headers = await teamHeaders(page);
  const job = await (await page.request.get(`/api/v1/jobs/${jobId}`, { headers })).json();
  expect(job.parameters.spec.limit).toBe("db");
  expect(job.parameters.spec.tags).toBe("ping");

  await page.goto(`/#/templates?edit=${template.id}`);
  const edit = page.getByRole("dialog");
  await expect(edit.getByRole("heading", { name: "Edit template" })).toBeVisible();
  const boxes = edit.locator('input[name="prompt_field"]');
  await expect(boxes).toHaveCount(8);
  await expect(edit.locator('input[name="prompt_field"][value="limit"]')).toBeChecked();
  await expect(edit.locator('input[name="prompt_field"][value="tags"]')).not.toBeChecked();
  for (const label of await edit.locator("#prompt-fields .check-row").all()) {
    await expectTouchTarget(label, "prompt field checkbox row");
  }
  const master = edit.locator('input[name="prompt_on_launch"]');
  await master.check();
  await expect(boxes.first()).toBeChecked();
  await expect(boxes.first()).toBeDisabled();
  await noHorizontalScroll(page);
});
