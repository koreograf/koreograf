"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "inventories.js")).href);

function control(value, checked = false) {
  return { value, checked };
}

function sourceCard(kind, values = {}) {
  const single = {
    "[name=source_name]": control(values.name || ""),
    "[name=body]": control(values.body || ""),
    "[name=project_id]": control(values.project_id || ""),
    "[name=path]": control(values.path || ""),
    "[name=credential_id]": control(values.credential_id || ""),
    "[name=group_names_raw]": control("", values.group_names_raw),
    "[name=load_config_context]": control("", values.load_config_context),
    "[name=max_hosts]": control(values.max_hosts ?? ""),
    "[name=validate_certs]": control("", values.validate_certs),
    "[name=virtual_machines]": control("", values.virtual_machines),
  };
  return {
    dataset: { kind },
    querySelector(selector) {
      return single[selector];
    },
    querySelectorAll(selector) {
      if (selector === "[name=group_by]:checked") return (values.group_by || []).map((value) => control(value, true));
      if (selector === "[data-filter-row]") return values.filters || [];
      return [];
    },
  };
}

function filterRow(key, value) {
  return {
    querySelector(selector) {
      return {
        "[data-filter-key]": control(key),
        "[data-filter-key-custom]": control(""),
        "[data-filter-value]": control(value),
      }[selector];
    },
  };
}

test("the inventory list filter matches names and source details", async () => {
  const { inventoryMatches } = await load();
  const item = {
    name: "Production",
    sources: [{ kind: "project", name: "Git", source: { path: "hosts.ini" } }],
  };
  assert.equal(inventoryMatches(item, [], ""), true);
  assert.equal(inventoryMatches(item, [], "production"), true);
  assert.equal(inventoryMatches(item, [], "hosts.ini"), true);
  assert.equal(inventoryMatches(item, [], "staging"), false);
});

test("the host list state falls back to the first page and default size", async () => {
  const { inventoryHostState } = await load();
  assert.deepEqual(inventoryHostState("#/inventories/inv-1"), {
    page: 1,
    search: "",
    group: "",
    size: 50,
    sort: "name",
    tab: "hosts",
  });
  assert.deepEqual(inventoryHostState("#/inventories/inv-1?page=0&size=500&tab=unknown"), {
    page: 1,
    search: "",
    group: "",
    size: 50,
    sort: "name",
    tab: "hosts",
  });
});

test("the host hash keeps only filters that differ from the defaults", async () => {
  const { inventoryHostHash } = await load();
  assert.equal(inventoryHostHash("inv-1", { page: 1, search: "", group: "", size: 50, sort: "name", tab: "hosts" }), "#/inventories/inv-1");
  assert.equal(
    inventoryHostHash("inv-1", { page: 2, search: "web", group: "apps", size: 100, sort: "-address", tab: "groups" }),
    "#/inventories/inv-1?page=2&q=web&group=apps&size=100&sort=-address&tab=groups"
  );
});

test("the host request carries paging, search and sorting", async () => {
  const { inventoryHostUrl } = await load();
  assert.equal(inventoryHostUrl("inv-1", { page: 1, search: "", group: "", size: 50, sort: "name" }), "/api/v1/inventories/inv-1/hosts?page=1&page_size=50&sort=name");
  assert.equal(inventoryHostUrl("inv-1", { page: 3, search: "a&b", group: "apps", size: 100, sort: "-address" }), "/api/v1/inventories/inv-1/hosts?page=3&page_size=100&search=a%26b&group=apps&sort=-address");
});

test("a NetBox source draft keeps the filters and defaults", async () => {
  const { readSourceDraft } = await load();
  const card = sourceCard("netbox", {
    name: " NetBox ",
    credential_id: "cred-1",
    group_by: ["sites", "tags"],
    load_config_context: true,
    max_hosts: "0",
    validate_certs: true,
    filters: [filterRow("site", "berlin"), filterRow("status", "active"), filterRow("", "ignored")],
  });
  assert.deepEqual(readSourceDraft(card), {
    kind: "netbox",
    name: "NetBox",
    credential_id: "cred-1",
    group_by: ["sites", "tags"],
    group_names_raw: false,
    load_config_context: true,
    max_hosts: 2000,
    query_filters: ["site=berlin", "status=active"],
    validate_certs: true,
    virtual_machines: false,
  });
});

test("a project source draft carries the project and inventory file", async () => {
  const { readSourceDraft } = await load();
  const card = sourceCard("project", { name: "Git", project_id: "proj-1", path: "hosts.ini" });
  assert.deepEqual(readSourceDraft(card), {
    kind: "project",
    name: "Git",
    project_id: "proj-1",
    path: "hosts.ini",
  });
});
