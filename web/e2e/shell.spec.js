"use strict";

const { expect, test } = require("@playwright/test");
const { post } = require("./api");
const { password, username } = require("./env");

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

async function noHorizontalScroll(page) {
  await expect.poll(
    () => page.evaluate(() => document.scrollingElement.scrollWidth <= window.innerWidth),
    { message: "the page scrolls sideways" },
  ).toBe(true);
}

async function launchShellJob(page, name, script) {
  const me = await page.request.get("/api/v1/me");
  const team = (await me.json()).teams[0].id;
  const headers = { "X-Team-Id": team };
  const created = await post(page.request, "/api/v1/templates", {
    headers,
    data: { name, tool: "shell", spec: { script } },
  });
  expect(created.status()).toBe(201);
  const template = await created.json();
  const launched = await post(page.request, `/api/v1/templates/${template.id}/launch`, {
    headers,
    data: {},
  });
  expect(launched.status()).toBe(202);
  const job = await launched.json();
  let settled = job;
  for (let attempt = 0; attempt < 120; attempt += 1) {
    if (["successful", "failed", "canceled"].includes(settled.status)) break;
    await page.waitForTimeout(250);
    settled = await (await page.request.get(`/api/v1/jobs/${job.id}`, { headers })).json();
  }
  expect(settled.status).toBe("failed");
  return settled.id;
}

test("a shell job renders ansi colours, a distinct stderr line, and the exit code", async ({ page }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1280, height: 900 });
  await signIn(page);
  const script = [
    "#!/bin/sh",
    "printf '\\033[32mansi-ok\\033[0m done\\n'",
    "printf 'ansi-err\\n' >&2",
    "exit 2",
    "",
  ].join("\n");
  const id = await launchShellJob(page, "Ansi shell", script);

  await page.goto(`/#/jobs/${id}`);
  await expect(page.locator(".spread .pill.failed").first()).toBeVisible();
  await expect(page.locator("#job-exit-code")).toHaveText("Exit code 2");
  await expect(page.locator("#job-exit-code")).toBeVisible();
  await expect(page.locator("#job-duration")).not.toHaveText("—");

  const colour = page.locator("#log .ansi-fg-green");
  await expect(colour).toHaveText("ansi-ok");
  await expect(colour).toHaveCSS("color", "rgb(134, 239, 172)");

  const stderr = page.locator("#log div.stderr").first();
  await expect(stderr).toHaveText("ansi-err");
  const styles = await stderr.evaluate((element) => {
    const style = getComputedStyle(element);
    return { background: style.backgroundColor, shadow: style.boxShadow };
  });
  expect(styles.background).not.toBe("rgba(0, 0, 0, 0)");
  expect(styles.shadow).toContain("inset");

  // A Shell job keeps only the raw log and never shows an empty structured view.
  await expect(page.locator("#output-view")).toBeHidden();

  await page.locator("#log-search").fill("ansi-ok");
  await expect(page.locator("#log-search-count")).toHaveText("1 of 1");
  await expect(page.locator("#log mark.log-hit").first()).toHaveText("ansi-ok");

  // A hit that crosses the colour boundary still highlights every part.
  await page.locator("#log-search").fill("ansi-ok done");
  await expect(page.locator("#log-search-count")).toHaveText("1 of 2");
  await expect(page.locator("#log mark.log-hit")).toHaveCount(2);
  await expect(page.locator("#log mark.log-hit").first()).toHaveClass(/ansi-fg-green/);

  await noHorizontalScroll(page);
  await page.locator("#log-search").fill("");
  for (const [width, height] of [[412, 923], [360, 732], [430, 932]]) {
    await page.setViewportSize({ width, height });
    await noHorizontalScroll(page);
    await expect(page.locator("#job-exit-code")).toBeVisible();
    await expect(page.locator("#log .ansi-fg-green")).toHaveText("ansi-ok");
  }
});
