"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "jobs.js")).href);

test("the recap parser reads every host and flags a failure", async () => {
  const { parseRecapLine, recapFailed } = await load();
  const rows = [
    "web-1                      : ok=3    changed=1    unreachable=0    failed=0    skipped=1    rescued=0    ignored=0",
    "web-2                      : ok=1    changed=0    unreachable=1    failed=1    skipped=0    rescued=0    ignored=0",
  ].map(parseRecapLine);
  assert.equal(rows.length, 2);
  assert.deepEqual(rows[0], { host: "web-1", ok: 3, changed: 1, unreachable: 0, failed: 0, skipped: 1, rescued: 0, ignored: 0 });
  assert.equal(recapFailed(rows[0]), false);
  assert.equal(rows[1].host, "web-2");
  assert.equal(rows[1].unreachable, 1);
  assert.equal(recapFailed(rows[1]), true);
  assert.equal(parseRecapLine("not a recap line"), null);
});

test("the log search keeps the raw text and marks every hit", async () => {
  const { searchSegments } = await load();
  const raw = '<script>alert("&lt;")</script> &lt; <b>x</b>';
  for (const query of ["script", "&lt;", "<b>"]) {
    const segments = searchSegments(raw, query);
    assert.equal(segments.map((item) => item.text).join(""), raw);
    assert.ok(segments.some((item) => item.hit), query);
  }
  assert.deepEqual(searchSegments("plain", ""), [{ text: "plain", hit: false }]);
});

test("the log line classifier marks fatal, recap and stderr lines", async () => {
  const { classifyLogLine } = await load();
  const log = { dataset: {} };
  assert.equal(classifyLogLine(log, { stream: "stdout", line: "fatal: [web-1]: FAILED! => {}" }), "fail");
  assert.equal(classifyLogLine(log, { stream: "stdout", line: "web-1 : ok=1 failed=1" }), "fail");
  assert.equal(classifyLogLine(log, { stream: "stdout", line: "PLAY RECAP ***" }), "");
  assert.equal(classifyLogLine(log, { stream: "stderr", line: "plain warning" }), "stderr");
  assert.equal(classifyLogLine(log, { stream: "system", line: "Tag filter matched no tasks." }), "warn");
});

test("the job parameters name the credentials and keep the full commit", async () => {
  const { jobParameters } = await load();
  const html = jobParameters({
    name: "Healthcheck",
    template_id: "tpl-1",
    parameters: {
      tool: "ansible",
      template_id: "tpl-1",
      template_name: "Healthcheck",
      project_commit: "abc1234",
      project_commit_full: "abc1234def5678",
      credentials: [{ name: "Deploy", kind: "ssh" }],
      extra_vars: {},
    },
  });
  assert.match(html, /<dt>credentials<\/dt><dd>Deploy · machine_ssh<\/dd>/);
  assert.doesNotMatch(html, /nav\.access/);
  assert.match(html, /title="abc1234def5678"/);
});
