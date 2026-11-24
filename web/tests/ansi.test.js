"use strict";

require("./test-dom.js").install();
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = (path) => import(pathToFileURL(join(__dirname, "..", "js", path)).href);

function mountLog() {
  document.body.replaceChildren();
  const log = document.createElement("div");
  log.id = "log";
  document.body.appendChild(log);
  return log;
}

test("sgr colour, bold, italic, underline, and background become allowlisted classes", async () => {
  const { ansiRuns } = await load("features/jobs.js");
  assert.deepEqual(ansiRuns("\u001b[1;4;31mwarn\u001b[0m plain"), [
    { text: "warn", classes: ["ansi-bold", "ansi-underline", "ansi-fg-red"] },
    { text: " plain", classes: [] },
  ]);
  assert.deepEqual(ansiRuns("\u001b[3;42mnote\u001b[0m"), [
    { text: "note", classes: ["ansi-italic", "ansi-bg-green"] },
  ]);
  assert.deepEqual(ansiRuns("\u001b[91;104mbright\u001b[0m"), [
    { text: "bright", classes: ["ansi-fg-bright-red", "ansi-bg-bright-blue"] },
  ]);
  assert.deepEqual(ansiRuns("a\u001b[1mb\u001b[22mc"), [
    { text: "a", classes: [] },
    { text: "b", classes: ["ansi-bold"] },
    { text: "c", classes: [] },
  ]);
});

test("osc, csi, extended colours, and unknown sequences are dropped", async () => {
  const { ansiRuns } = await load("features/jobs.js");
  assert.deepEqual(ansiRuns("\u001b]0;window title\u0007visible"), [
    { text: "visible", classes: [] },
  ]);
  assert.deepEqual(ansiRuns("a\u001b[2Kb\u001b[?25lc\u001b[999md"), [
    { text: "abcd", classes: [] },
  ]);
  // 38;2;1;1;1 must not leak the 1 into bold.
  assert.deepEqual(ansiRuns("\u001b[38;2;1;1;1mplain\u001b[0m"), [
    { text: "plain", classes: [] },
  ]);
  assert.deepEqual(ansiRuns("x\u001bPdevice control\u001b\\y"), [
    { text: "xy", classes: [] },
  ]);
  assert.deepEqual(ansiRuns("tail\u001b[31"), [{ text: "tail", classes: [] }]);
});

test("the log renders safe spans and keeps markup literal", async () => {
  const { appendLog } = await load("features/jobs.js");
  const log = mountLog();
  appendLog({ stream: "stdout", line: "\u001b[32m<script>alert(1)</script>\u001b[0m" });
  appendLog({ stream: "stdout", line: "\u001b[31mred\u001b[0m \u001b[1msolid\u001b[0m" });
  appendLog({ stream: "stderr", line: "ansi-err" });

  const lines = log.children;
  assert.equal(lines.length, 3);
  assert.equal(lines[0].textContent, "<script>alert(1)</script>");
  assert.equal(lines[0].querySelector("script"), null, "markup in the payload became an element");
  assert.equal(lines[0].children[0].className, "ansi-fg-green");
  assert.equal(lines[0].children[0].textContent, "<script>alert(1)</script>");

  assert.equal(lines[1].textContent, "red solid");
  assert.deepEqual(lines[1].children.map((child) => child.textContent), ["red", " ", "solid"]);
  assert.equal(lines[1].children[0].className, "ansi-fg-red");
  assert.equal(lines[1].children[2].className, "ansi-bold");

  assert.equal(lines[2].className, "stderr");
  assert.equal(lines[2].textContent, "ansi-err");
});

test("the exit code comes from the system line and fills the header", async () => {
  const { appendLog, exitCodeFromEvents, exitCodeFromLine } = await load("features/jobs.js");
  const i18n = await load("core/i18n.js");
  i18n.setCatalog("de", { exit_code: "Exit-Code {code}" });
  assert.equal(exitCodeFromLine("Exit code: 2"), 2);
  assert.equal(exitCodeFromLine("Exit code: 0"), 0);
  assert.equal(exitCodeFromLine("Cache afterwards 12 bytes"), null);
  assert.equal(exitCodeFromEvents([
    { stream: "system", line: "Local run on the controller." },
    { stream: "system", line: "Exit code: 9" },
    { stream: "system", line: "Exit code: 2" },
  ]), 2);
  assert.equal(exitCodeFromEvents([{ stream: "stdout", line: "Exit code: 2" }]), null);

  mountLog();
  const box = document.createElement("span");
  box.id = "job-exit-code";
  box.hidden = true;
  document.body.appendChild(box);
  appendLog({ stream: "system", line: "Local run on the controller." });
  assert.equal(box.hidden, true, "the header shows an exit code before the process ends");
  appendLog({ stream: "system", line: "Exit code: 2" });
  assert.equal(box.hidden, false, "the header keeps the exit code hidden");
  assert.equal(box.textContent, "Exit-Code 2");
});
