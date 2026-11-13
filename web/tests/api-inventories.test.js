"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "inventories.js")).href);

const vars = [
  { name: "ansible_host", value: "10.0.0.1", origin: "host" },
  { name: "message", value: "hello world", origin: "web" },
  { name: "ansible_become_password", value: "s3cret", origin: "host" },
  { name: "evil", value: "<script>x &lt; y</script>", origin: "group" },
];

test("the variables viewer renders yaml and json and hides secrets", async () => {
  const { effectiveVariablesYaml, effectiveVariablesJson, variablesViewer } = await load();
  const yaml = effectiveVariablesYaml(vars);
  for (const want of [
    '<span class="vars-key">ansible_host</span>: <span class="vars-value">10.0.0.1</span>',
    '<span class="vars-value">&quot;hello world&quot;</span>',
    '<span class="vars-value">&quot;••••••&quot;</span>',
    '<span class="vars-comment"># host</span>',
    '<span class="vars-comment"># web</span>',
  ]) {
    assert.ok(yaml.includes(want), `yaml missing ${want}\n${yaml}`);
  }
  assert.ok(yaml.includes("&lt;script&gt;x &amp;lt; y&lt;/script&gt;"), "yaml value was not escaped");
  assert.ok(!yaml.includes("<script>"), "yaml kept raw markup");

  const json = effectiveVariablesJson(vars);
  for (const want of [
    '<span class="vars-key">&quot;name&quot;</span>',
    '<span class="vars-key">&quot;value&quot;</span>',
    '<span class="vars-key">&quot;origin&quot;</span>',
    '<span class="vars-value">&quot;ansible_host&quot;</span>',
    '<span class="vars-value">&quot;host&quot;</span>',
    '<span class="vars-value">&quot;••••••&quot;</span>',
    "&lt;script&gt;x &amp;lt; y&lt;/script&gt;",
  ]) {
    assert.ok(json.includes(want), `json missing ${want}\n${json}`);
  }
  assert.ok(!json.includes("<script>"), "json kept raw markup");
  assert.ok(json.includes("\n  {\n"), "json is not pretty printed");

  const view = variablesViewer(vars);
  assert.ok(view.includes('<div class="spread tab-bar">'), "missing page tab bar");
  assert.ok(view.includes('<div class="tabs" role="tablist"'), "missing page tabs");
  assert.ok(view.includes('data-vars-tab="yaml" aria-selected="true"'), "yaml is not selected by default");
  assert.ok(view.includes('data-vars-tab="json" aria-selected="false"'), "json tab is not unselected");
  assert.match(view, /<div id="vars-\d+-yaml" role="tabpanel" data-vars-panel="yaml"><pre class="vars">/);
  assert.match(view, /<div id="vars-\d+-json" role="tabpanel" data-vars-panel="json" hidden>/);

  const weird = effectiveVariablesYaml([
    { name: "a", value: "1", origin: "team\n  injected: true" },
    { name: "b", value: "2", origin: "team\u0085  injected: true" },
  ]);
  assert.ok(!weird.includes("\n  injected"), "origin line break added a mapping line");
  assert.match(weird.replace(/\s+/g, " "), /# team injected: true/);
  assert.doesNotMatch(weird, /[\u0085\u2028\u2029]/);

  const empty = variablesViewer([]);
  assert.equal((empty.match(/no_variables/g) || []).length, 2, "empty state is not in both formats");
  assert.ok(!empty.includes('<pre class="vars">'), "empty state rendered a code block");
});

test("the inventory tables drop the variables column and keep addresses and groups", async () => {
  const { inventoryHostTable, inventoryGroupTable } = await load();
  const hosts = inventoryHostTable([{ name: "web-1", vars: { ansible_host: "10.0.0.1" }, groups: ["web"] }], { sort: "" }, "i1");
  const groups = inventoryGroupTable([{ name: "web", host_count: 1, children: [] }], "i1");
  for (const table of [hosts, groups]) {
    assert.doesNotMatch(table, /variables/, "an inventory table still renders a variables header");
    assert.doesNotMatch(table, /formatVars/, "an inventory table still renders variables");
  }
  assert.match(hosts, /address/);
  assert.match(hosts, /groups/);
  assert.match(groups, /hosts/);
  assert.match(groups, /groups/);
});
