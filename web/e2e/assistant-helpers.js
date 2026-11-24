"use strict";

const http = require("node:http");
const { expect } = require("@playwright/test");
const { password, username } = require("./env");
const { post, put } = require("./api");

function settingsBody(endpoint, enabled) {
  return {
    ldap: {},
    proxy: {},
    runner: "local",
    job_timeout_secs: 60,
    job_output_lines: 8000,
    llm: {
      enabled,
      provider: "ollama",
      base_url: endpoint,
      model: "e2e-model",
      api_key: "e2e-assistant-key",
    },
  };
}

async function startFakeLlm(answer = "A focused answer from the panel.") {
  const state = { answer, tool: null, lastBody: null };
  const server = http.createServer((request, response) => {
    let raw = "";
    request.on("data", (chunk) => {
      raw += chunk;
    });
    request.on("end", () => {
      try {
        state.lastBody = JSON.parse(raw || "{}");
      } catch {
        state.lastBody = {};
      }
      let payload;
      if (state.tool) {
        const tool = state.tool;
        state.tool = null;
        payload = {
          choices: [{
            message: {
              role: "assistant",
              content: "",
              tool_calls: [{
                id: "call_e2e",
                type: "function",
                function: { name: tool.name, arguments: JSON.stringify(tool.arguments) },
              }],
            },
          }],
        };
      } else {
        payload = { choices: [{ message: { role: "assistant", content: state.answer } }] };
      }
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify(payload));
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  return {
    state,
    endpoint: `http://127.0.0.1:${port}/v1`,
    close: () => new Promise((resolve) => server.close(resolve)),
  };
}

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

async function enableAssistant(page, endpoint) {
  const saved = await put(page.request, "/api/v1/settings", { data: settingsBody(endpoint, true) });
  expect(saved.status()).toBe(200);
  await page.reload();
}

async function disableAssistant(page) {
  try {
    await put(page.request, "/api/v1/settings", { data: settingsBody("", false) });
  } catch {
    // The page may already be signed out.
  }
}

async function teamId(page) {
  return page.evaluate(async () => {
    const response = await fetch("/api/v1/me");
    const profile = await response.json();
    return profile.teams[0].id;
  });
}

async function createShellTemplate(page, team, name, script) {
  const created = await post(page.request, "/api/v1/templates", {
    headers: { "X-Team-Id": team },
    data: { name, tool: "shell", spec: { script } },
  });
  expect(created.status()).toBe(201);
  return (await created.json()).id;
}

async function createFailedJob(page, team, name) {
  const template = await createShellTemplate(page, team, name, "#!/bin/sh\nexit 1\n");
  const launched = await post(page.request, `/api/v1/templates/${template}/launch`, {
    headers: { "X-Team-Id": team },
    data: {},
  });
  expect(launched.status()).toBe(202);
  const job = await launched.json();
  let settled = job;
  for (let attempt = 0; attempt < 120; attempt += 1) {
    if (["successful", "failed", "canceled"].includes(settled.status)) break;
    await page.waitForTimeout(250);
    settled = await (await page.request.get(`/api/v1/jobs/${job.id}`, {
      headers: { "X-Team-Id": team },
    })).json();
  }
  expect(settled.status).toBe("failed");
  return settled.id;
}

async function createWorkflow(page, team, name) {
  const created = await post(page.request, "/api/v1/workflows", {
    headers: { "X-Team-Id": team },
    data: { name, description: "", graph: { nodes: [], edges: [] } },
  });
  expect(created.status()).toBe(201);
  return (await created.json()).id;
}

async function createInventory(page, team, name) {
  const created = await post(page.request, "/api/v1/inventories", {
    headers: { "X-Team-Id": team },
    data: { name, body: "[web]\nweb-1\n" },
  });
  expect(created.status()).toBe(201);
  return (await created.json()).id;
}

async function noHorizontalScroll(page) {
  await expect.poll(
    () => page.evaluate(() => document.scrollingElement.scrollWidth <= window.innerWidth),
    { message: "the page scrolls sideways" },
  ).toBe(true);
}

async function goHash(page, hash) {
  await page.evaluate((next) => {
    window.location.hash = next;
  }, hash);
}

async function sendMessage(panel, text) {
  await panel.getByLabel("Message").fill(text);
  await panel.getByRole("button", { name: "Send", exact: true }).click();
}

/// Opens the panel from a failed job, keeps it across route changes, checks the
/// context chip, and runs the confirmation card. The phone flag asserts the
/// fullscreen sheet geometry instead of the desktop card.
async function assistantPanelScenario(page, fake, { phone = false } = {}) {
  await signIn(page);
  const team = await teamId(page);
  const suffix = Date.now();
  const jobName = `Assistant job ${suffix}`;
  const workflowName = `Assistant workflow ${suffix}`;
  const inventoryName = `Assistant inventory ${suffix}`;
  const job = await createFailedJob(page, team, jobName);
  const workflow = await createWorkflow(page, team, workflowName);
  const inventory = await createInventory(page, team, inventoryName);
  const template = await createShellTemplate(page, team, `Assistant launch ${suffix}`, "#!/bin/sh\necho launched\n");
  await enableAssistant(page, fake.endpoint);

  await page.goto(`/#/jobs/${job}`);
  const launcher = page.getByRole("button", { name: "Open the assistant" });
  await expect(launcher).toBeVisible();
  await expect(page.locator("#assistant-explain")).toBeVisible();
  await page.getByRole("button", { name: "Explain this failure", exact: true }).click();

  const panel = page.getByRole("dialog", { name: "Assistant" });
  await expect(panel).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`#/jobs/${job}$`));
  await expect(launcher).toHaveAttribute("aria-expanded", "true");
  await expect(panel.locator("#assistant-context-label")).toContainText("Job");
  await expect(panel).toContainText("Explain why this job failed.");
  await expect(panel).toContainText(fake.state.answer);
  await expect(panel.getByLabel("Message")).toBeFocused();

  await goHash(page, `#/workflows/${workflow}`);
  await expect(panel.locator("#assistant-context-label")).toContainText("Workflow");
  await expect(panel).toContainText(fake.state.answer);

  await goHash(page, `#/inventories/${inventory}`);
  await expect(panel.locator("#assistant-context-label")).toContainText("Inventory");
  await expect(panel).toContainText(fake.state.answer);

  await panel.getByRole("button", { name: "Remove the page context" }).click();
  await expect(panel.locator("#assistant-context-row")).toBeHidden();
  await sendMessage(panel, "What changed here?");
  await expect(panel).toContainText(fake.state.answer);
  expect(fake.state.lastBody.context).toBeUndefined();

  fake.state.tool = { name: "launch_template", arguments: { id: template, team_id: team } };
  await sendMessage(panel, "launch the template");
  await expect(panel.locator(".assistant-confirm")).toBeVisible();
  await goHash(page, `#/workflows/${workflow}`);
  await expect(panel.locator(".assistant-confirm")).toBeVisible();
  await panel.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(panel).toContainText("Action canceled. Nothing changed.");

  fake.state.tool = { name: "launch_template", arguments: { id: template, team_id: team } };
  await sendMessage(panel, "launch the template again");
  await expect(panel.locator(".assistant-confirm")).toBeVisible();
  await panel.getByRole("button", { name: "Confirm", exact: true }).click();
  await expect(panel).toContainText("Action executed.");

  await page.keyboard.press("Escape");
  await expect(panel).toBeHidden();
  await expect(launcher).toHaveAttribute("aria-expanded", "false");
  await expect(launcher).toBeFocused();
  await launcher.click();
  await expect(panel).toBeVisible();
  await expect(panel.getByLabel("Message")).toBeFocused();

  await panel.getByRole("button", { name: "Close the assistant" }).click();
  await expect(panel).toBeHidden();
  await expect(launcher).toBeFocused();

  // The old chat hash is a compatibility redirect: it opens the panel and
  // returns to the page the user came from.
  await goHash(page, `#/workflows/${workflow}`);
  await goHash(page, "#/assistant");
  await expect(page).toHaveURL(new RegExp(`#/workflows/${workflow}$`));
  await expect(panel).toBeVisible();
  await expect(panel.locator("#assistant-context-label")).toContainText("Workflow");

  if (phone) {
    const box = await panel.boundingBox();
    const viewport = page.viewportSize();
    expect(box.width).toBe(viewport.width);
    expect(box.height).toBe(viewport.height);
    const position = await panel.evaluate((element) => getComputedStyle(element).position);
    expect(position).toBe("fixed");
  }

  await noHorizontalScroll(page);
  await panel.getByRole("button", { name: "Close the assistant" }).click();
  await expect(panel).toBeHidden();
  await expect(launcher).toBeFocused();
  await disableAssistant(page);
}

module.exports = {
  assistantPanelScenario,
  disableAssistant,
  startFakeLlm,
};
