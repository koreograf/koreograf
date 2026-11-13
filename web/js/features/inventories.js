import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { confirmDialog } from "../core/dialog.js";
import { can, go, handle, markSaved, needTeam, route, shell } from "../core/app.js";
import { addLink, emptyState, esc, h, selectOptions, table } from "../core/dom.js";
import { clearEditHash, editQuery, openEditDialog } from "../core/edit.js";
import { t } from "../core/i18n.js";
import { registerRedirect, registerRoute } from "../core/router.js";
import { openCredentialCreator } from "./credentials.js";
import { openLaunchPrompt } from "./templates.js";

let sourceNotice = null;
let variablesSerial = 0;

function sourceKindLabel(kind) {
  return { ini: t("manual"), project: t("project"), netbox: t("netbox"), mixed: t("several") }[kind] || kind || t("manual");
}

function sourceTitle(source) {
  const name = (source.name || "").trim();
  const fallback = { ini: "manual", project: "project", netbox: "netbox" }[source.kind];
  if (!name || name === fallback) return sourceKindLabel(source.kind);
  return name;
}

function sourceSummary(item, credentials = []) {
  const data = item.source || {};
  if (item.kind === "netbox") {
    const credential = credentials.find((row) => row.id === item.credential_id);
    return (credential && credential.name) || data.api_endpoint || t("netbox");
  }
  if (item.kind === "project") return data.path || t("project");
  return t("text");
}

export function inventoryMatches(item, credentials, needle) {
  if (!needle) return true;
  const kinds = (item.sources || []).map((source) => `${source.name} ${sourceKindLabel(source.kind)} ${sourceSummary(source, credentials)}`).join(" ");
  return `${item.name} ${kinds}`.toLowerCase().includes(needle);
}

function certificateCheckOff(source) {
  const data = (source && source.source) || {};
  return source && source.kind === "netbox" && data.validate_certs === false;
}

function certificateBadge(on) {
  return on ? ` <span class="pill warn">${h("certificate_check_off")}</span>` : "";
}

const NETBOX_GROUPINGS = [
  ["sites", "sites"],
  ["region", "regions"],
  ["site_group", "site_groups"],
  ["location", "locations"],
  ["device_roles", "roles"],
  ["platforms", "platforms"],
  ["manufacturers", "manufacturers"],
  ["device_types", "device_types"],
  ["tenants", "tenants"],
  ["tags", "tags"],
  ["cluster", "cluster"],
  ["status", "status"],
  ["is_virtual", "virtual_and_physical"],
];

const NETBOX_FILTER_KEYS = ["status", "site", "tag", "role"];

function netboxFilters(data) {
  if (Array.isArray(data.query)) {
    return data.query
      .map((item) => Array.isArray(item) ? { key: String(item[0] || ""), value: String(item[1] || "") } : null)
      .filter((item) => item && item.key);
  }
  if (Array.isArray(data.query_filters)) {
    return data.query_filters
      .map((line) => String(line).split("="))
      .filter((parts) => parts.length > 1 && parts[0].trim())
      .map((parts) => ({ key: parts[0].trim(), value: parts.slice(1).join("=").trim() }));
  }
  return [];
}

function netboxFilterRow(filter = {}) {
  const key = String(filter.key || "status");
  const value = String(filter.value || "");
  const custom = !NETBOX_FILTER_KEYS.includes(key);
  const options = NETBOX_FILTER_KEYS.map((name) => `<option value="${esc(name)}"${name === key ? " selected" : ""}>${h(name)}</option>`).join("");
  const extra = custom ? `<option value="${esc(key)}" selected>${esc(key)}</option>` : "";
  return `<div class="filter-row" data-filter-row>
    <select data-filter-key>${options}${extra}<option value="__custom">${h("custom_key")}</option></select>
    <input data-filter-key-custom${custom ? "" : " hidden"} value="${custom ? esc(key) : ""}" placeholder="${h("key")}" autocomplete="off">
    <input data-filter-value value="${esc(value)}" placeholder="${h("value")}" autocomplete="off">
    <button class="btn ghost" type="button" data-remove-filter>${h("remove")}</button>
  </div>`;
}

function netboxGroupingFields(selected) {
  return `<div class="check-grid">${NETBOX_GROUPINGS.map(([key, label]) => `
    <label class="check-row"><input type="checkbox" name="group_by" value="${esc(key)}"${selected.includes(key) ? " checked" : ""}><span>${h(label)}</span></label>`).join("")}</div>`;
}

export function mountSourceDraft(container, projects, kind, preset, options = {}) {
  const data = preset || {};
  const credentials = options.credentials || [];
  const editing = Boolean(options.editing);
  const card = document.createElement("section");
  card.className = "card";
  card.dataset.kind = kind;
  if (editing) card.dataset.editing = editing;
  card.style.marginTop = "10px";
  const projectOptions = projects.map((item) => `<option value="${esc(item.id)}">${esc(item.name)}</option>`).join("");
  const title = kind === "ini" ? t("manual") : kind === "project" ? t("project") : t("netbox");
  let fields = "";
  if (kind === "ini") {
    fields = `<label class="field required"><span>${h("ini_or_yaml")}</span><textarea name="body" required>${esc(data.body || "[all]\nlocalhost ansible_connection=local")}</textarea></label>`;
  } else if (kind === "project") {
    const initialPath = data.path ? `<option value="${esc(data.path)}" selected>${esc(data.path)}</option>` : "";
    fields = `<label class="field required"><span>${h("project")}</span><select name="project_id" required><option value="">—</option>${projectOptions}</select></label>
      <label class="field required"><span>${h("inventory_file")}</span><select name="path" required><option value="">—</option>${initialPath}</select></label>
      <p class="small muted" data-plugin-note></p>`;
  } else {
    const selectedGroups = Array.isArray(data.group_by)
      ? data.group_by.map(String)
      : String(data.group_by || "").split(",").map((item) => item.trim()).filter(Boolean);
    const filters = netboxFilters(data);
    const filterRows = (filters.length ? filters : [{ key: "status", value: "" }])
      .map(netboxFilterRow)
      .join("");
    const maxHosts = Number(data.max_hosts) > 0 ? Number(data.max_hosts) : 2000;
    const netboxCredentials = credentials.filter((item) => item.kind === "netbox");
    const selectedCredential = String(data.credential_id || "");
    const credentialPick = `<label class="field required"><span>${h("netbox_credential")}</span>
      <div class="row">
        <select class="fit" name="credential_id" required>${selectOptions(netboxCredentials, selectedCredential, true, (row) => row.name)}</select>
        ${can("owner") ? `<button class="btn pill" type="button" data-new-netbox-credential>${h("credential")}</button>` : ""}
      </div>
    </label>
    <p class="small muted"><span>${h("api_address")}: </span><span class="mono" data-netbox-address></span></p>`;
    fields = `${credentialPick}
      <div class="field"><span>${h("grouping")}</span>${netboxGroupingFields(selectedGroups)}</div>
      <p class="small muted">${h("netbox_collision_note")}</p>
      <div class="field"><span>${h("filters")}</span>
        <div class="filter-rows" data-filter-rows>${filterRows}</div>
        <button class="btn pill" type="button" data-add-filter>${h("add_filter")}</button>
      </div>
      <details class="advanced-fieldset">
        <summary>${h("advanced")}</summary>
        <div class="form">
          <label class="field check"><span>${h("group_names_raw")}</span><input name="group_names_raw" type="checkbox"${data.group_names_raw ? " checked" : ""}></label>
          <label class="field check"><span>${h("load_config_context")}</span><input name="load_config_context" type="checkbox"${data.load_config_context ? " checked" : ""}></label>
          <label class="field"><span>${h("max_hosts")}</span><input name="max_hosts" type="number" min="1" max="10000" value="${esc(maxHosts)}"><span class="small muted">${h("netbox_max_hosts_hint", { max: 10000 })}</span></label>
          <label class="field check"><span>${h("verify_certificate")}</span><input name="validate_certs" type="checkbox"${data.validate_certs === false ? "" : " checked"}></label>
          <label class="field check"><span>${h("virtual_machines")}</span><input name="virtual_machines" type="checkbox"${data.virtual_machines === false ? "" : " checked"}></label>
        </div>
      </details>
      <div class="row"><button class="btn" type="button" data-netbox-test>${h("test_connection")}</button><button class="btn" type="button" data-netbox-preview>${h("preview")}</button><span class="small muted" data-netbox-test-status aria-live="polite"></span></div>
      <div class="preview-result" data-netbox-preview-result hidden></div>`;
  }
  card.innerHTML = `<div class="spread"><h2>${esc(title)}</h2><button class="btn ghost" type="button" data-remove-draft>${h(editing ? "cancel" : "remove")}</button></div>
    <div class="form"><p class="small muted">${h("fields_marked_with_a_star_are_required")}</p>${fields}
      <label class="field"><span>${h("source_name")}</span><input name="source_name" value="${esc(data.name || "")}" placeholder="${esc(title)}"></label>
      ${editing ? `<div class="row"><button class="btn pill primary" type="button" data-save-draft>${h("save_source")}</button></div>
      <p class="small muted" data-source-error aria-live="polite"></p>` : ""}
    </div>`;
  container.appendChild(card);
  card.addEventListener("input", () => { card.dataset.dirty = "1"; });
  card.addEventListener("change", () => { card.dataset.dirty = "1"; });
  if (kind === "netbox") {
    let addressToken = 0;
    const refreshCredential = async () => {
      const select = card.querySelector("[name=credential_id]");
      const address = card.querySelector("[data-netbox-address]");
      const token = ++addressToken;
      const selected = select.value;
      address.textContent = "";
      if (!selected) {
        address.textContent = t("netbox_credential_is_required");
        return;
      }
      try {
        const item = await api("GET", `/api/v1/credentials/${encodeURIComponent(selected)}`);
        if (token !== addressToken) return;
        address.textContent = item.fields && item.fields.url
          ? item.fields.url
          : t("netbox_credential_has_no_address");
      } catch (error) {
        if (token !== addressToken) return;
        address.textContent = t(error.message || "error.generic", error.vars);
      }
    };
    card.querySelector("[name=credential_id]").addEventListener("change", refreshCredential);
    const create = card.querySelector("[data-new-netbox-credential]");
    if (create) {
      create.addEventListener("click", () => {
        openCredentialCreator("netbox", (created) => {
          const select = card.querySelector("[name=credential_id]");
          credentials.push(created);
          select.insertAdjacentHTML("beforeend", `<option value="${esc(created.id)}">${esc(created.name)}</option>`);
          select.value = created.id;
          refreshCredential();
        });
      });
    }
    refreshCredential();
    const rows = card.querySelector("[data-filter-rows]");
    card.querySelector("[data-add-filter]").addEventListener("click", () => {
      rows.insertAdjacentHTML("beforeend", netboxFilterRow({ key: "status", value: "" }));
      card.dataset.dirty = "1";
    });
    rows.addEventListener("change", (event) => {
      if (!event.target.matches("[data-filter-key]")) return;
      const row = event.target.closest("[data-filter-row]");
      const custom = row.querySelector("[data-filter-key-custom]");
      custom.hidden = event.target.value !== "__custom";
      if (!custom.hidden) custom.focus();
    });
    rows.addEventListener("click", (event) => {
      const button = event.target.closest("[data-remove-filter]");
      if (!button) return;
      const row = button.closest("[data-filter-row]");
      if (rows.querySelectorAll("[data-filter-row]").length > 1) {
        row.remove();
      } else {
        row.querySelector("[data-filter-value]").value = "";
      }
      card.dataset.dirty = "1";
    });
  }
  card.querySelector("[data-remove-draft]").addEventListener("click", () => card.remove());
  if (kind === "netbox") {
    const test = card.querySelector("[data-netbox-test]");
    const status = card.querySelector("[data-netbox-test-status]");
    test.addEventListener("click", async () => {
      const draft = readSourceDraft(card);
      test.disabled = true;
      status.textContent = t("testing");
      try {
        const result = await api("POST", "/api/v1/inventories/netbox/test", {
          credential_id: draft.credential_id,
          validate_certs: draft.validate_certs,
        });
        status.textContent = result.version
          ? t("netbox.test_ok", { version: result.version })
          : t("netbox.test_ok_no_version");
      } catch (error) {
        status.textContent = t(error.message || "error.generic", error.vars);
      } finally {
        test.disabled = false;
      }
    });
    const preview = card.querySelector("[data-netbox-preview]");
    const resultBox = card.querySelector("[data-netbox-preview-result]");
    preview.addEventListener("click", async () => {
      const draft = readSourceDraft(card);
      preview.disabled = true;
      status.textContent = t("previewing");
      resultBox.hidden = true;
      try {
        const result = await api("POST", "/api/v1/inventories/netbox/preview", {
          credential_id: draft.credential_id,
          validate_certs: draft.validate_certs,
          group_by: draft.group_by,
          group_names_raw: draft.group_names_raw,
          load_config_context: draft.load_config_context,
          max_hosts: draft.max_hosts,
          query_filters: draft.query_filters,
          virtual_machines: draft.virtual_machines,
        });
        status.textContent = "";
        const groups = (result.groups || []).map((group) => `<li>${esc(group.name)} <span class="small muted">${h("count_hosts", { count: group.host_count })}</span></li>`).join("");
        const sample = result.sample_host
          ? `<h3>${h("sample_host")}</h3><p class="mono">${esc(result.sample_host.name)}</p>${variablesViewer(Object.entries(result.sample_host.vars || {}).map(([name, value]) => ({ name, value, origin: "host" })))}`
          : `<p class="small muted">${h("preview_no_hosts")}</p>`;
        const collisions = (result.collisions || []).length
          ? `<p class="small warn">${h("netbox_collisions", { groups: result.collisions.join(", ") })}</p>`
          : "";
        resultBox.innerHTML = `<p><strong>${h("count_hosts", { count: result.host_count })}</strong></p>${collisions}
          <h3>${h("groups")}</h3><ul>${groups || `<li>${h("none")}</li>`}</ul>${sample}`;
        bindVariablesViews(resultBox);
        resultBox.hidden = false;
      } catch (error) {
        status.textContent = t(error.message || "error.generic", error.vars);
      } finally {
        preview.disabled = false;
      }
    });
  }
  if (kind === "project") {
    const select = card.querySelector("[name=project_id]");
    const files = card.querySelector("[name=path]");
    const note = card.querySelector("[data-plugin-note]");
    const loadFiles = async (preferred) => {
      files.innerHTML = `<option value="">—</option>`;
      note.textContent = "";
      if (!select.value) return;
      try {
        const contents = await api("GET", `/api/v1/projects/${encodeURIComponent(select.value)}/contents`);
        const inventories = contents.inventories || [];
        const staticFiles = inventories.filter((file) => file.format !== "plugin");
        files.innerHTML = `<option value="">—</option>${staticFiles.map((file) => `<option value="${esc(file.path)}">${esc(file.path)}</option>`).join("")}`;
        if (preferred && [...files.options].some((option) => option.value === preferred)) files.value = preferred;
        const plugins = inventories.filter((file) => file.format === "plugin");
        if (plugins.length) {
          note.textContent = t("files_is_not_executed_here_add_netbox_as", {
            files: plugins.map((file) => `${file.path} (${file.plugin || "Plugin"})`).join(", "),
          });
        } else if (!staticFiles.length) {
          note.textContent = t("no_inventory_files_sync_the_git_project_first");
        }
      } catch (error) {
        note.textContent = t(error.message || "the_project_cannot_be_read", error.vars);
      }
    };
    select.addEventListener("change", () => loadFiles(""));
    if (data.project_id) {
      select.value = data.project_id;
      loadFiles(data.path || "");
    }
  }
  return card;
}

export function readSourceDraft(card) {
  const kind = card.dataset.kind;
  const name = card.querySelector("[name=source_name]").value.trim();
  if (kind === "ini") return { kind, name, body: card.querySelector("[name=body]").value };
  if (kind === "project") {
    return {
      kind,
      name,
      project_id: card.querySelector("[name=project_id]").value,
      path: card.querySelector("[name=path]").value,
    };
  }
  const query_filters = [...card.querySelectorAll("[data-filter-row]")].map((row) => {
    const select = row.querySelector("[data-filter-key]");
    const key = select.value === "__custom"
      ? row.querySelector("[data-filter-key-custom]").value.trim()
      : select.value;
    const value = row.querySelector("[data-filter-value]").value.trim();
    return key && value ? `${key}=${value}` : "";
  }).filter(Boolean);
  return {
    kind,
    name,
    credential_id: card.querySelector("[name=credential_id]").value,
    group_by: [...card.querySelectorAll("[name=group_by]:checked")].map((input) => input.value),
    group_names_raw: card.querySelector("[name=group_names_raw]").checked,
    load_config_context: card.querySelector("[name=load_config_context]").checked,
    max_hosts: Number(card.querySelector("[name=max_hosts]").value) || 2000,
    query_filters,
    validate_certs: card.querySelector("[name=validate_certs]").checked,
    virtual_machines: card.querySelector("[name=virtual_machines]").checked,
  };
}

function readSourceUpdate(card) {
  const draft = readSourceDraft(card);
  delete draft.kind;
  return draft;
}

function bindSourcePicks(projects, credentials) {
  document.querySelectorAll("[data-add-kind]").forEach((button) => {
    button.addEventListener("click", () => {
      mountSourceDraft(document.querySelector("#source-drafts"), projects, button.dataset.addKind, undefined, { credentials });
    });
  });
}

async function pageInventories() {
  if (!needTeam()) return;
  const [items, credentials] = await Promise.all([
    api("GET", "/api/v1/inventories"),
    api("GET", "/api/v1/credentials"),
  ]);
  shell(t("inventories"), `
    <section class="card">
      <input class="search" id="inventory-search" placeholder="${h("search_inventories")}" autocomplete="off">
      <div id="inventory-table"></div>
    </section>`,
    can("operator") ? addLink("#/inventories?new=1", "inventory_2") : "");
  const draw = () => {
    const needle = document.querySelector("#inventory-search").value.trim().toLowerCase();
    const shown = items.filter((item) => inventoryMatches(item, credentials, needle));
    const rows = shown.map((item) => {
      const kinds = (item.sources || []).map((source) => sourceKindLabel(source.kind));
      const label = kinds.length ? [...new Set(kinds)].join(", ") : sourceKindLabel(item.source_kind);
      const count = item.source_count != null ? item.source_count : kinds.length;
      const insecure = (item.sources || []).some(certificateCheckOff) || certificateCheckOff({ kind: item.source_kind, source: item.source });
      return `<tr><td><a href="#/inventories/${esc(item.id)}">${esc(item.name)}</a></td><td class="narrow-hide">${esc(count)}</td><td class="narrow-hide">${esc(label)}${certificateBadge(insecure)}</td><td data-label="${h("hosts")}">${esc(item.host_count)}</td><td>${can("operator") ? `<a class="btn" href="#/inventories?edit=${esc(item.id)}">${h("edit")}</a>` : ""}</td></tr>`;
    });
    const host = document.querySelector("#inventory-table");
    if (!shown.length) {
      host.innerHTML = items.length
        ? `<p class="small muted">${h("nothing_here")}</p>`
        : emptyState("empty_inventories", can("operator") ? "#/inventories?new=1" : "", "new_inventory");
      return;
    }
    host.innerHTML = table([t("name"), t("sources"), t("type"), t("hosts"), ""], rows);
  };
  document.querySelector("#inventory-search").addEventListener("input", draw);
  draw();
  const request = editQuery();
  if (request.create !== null) {
    if (can("operator")) await openInventoryDialog(null);
    return;
  }
  if (!request.edit) return;
  const item = items.find((row) => row.id === request.edit);
  if (!item) {
    clearEditHash();
    return;
  }
  if (can("operator")) await openInventoryDialog(item);
}

async function openInventoryDialog(item) {
  const editing = Boolean(item);
  const [projects, credentials] = editing
    ? [[], []]
    : await Promise.all([api("GET", "/api/v1/projects"), api("GET", "/api/v1/credentials")]);
  const sources = `
    <div>
      <h2>${h("sources")}</h2>
      <p class="small muted">${h("an_inventory_can_have_several_sources_they_are")}</p>
      <div class="choice-row">
        <button class="btn pill" type="button" data-add-kind="ini">${h("manual_2")}</button>
        <button class="btn pill" type="button" data-add-kind="project">${h("project_2")}</button>
        <button class="btn pill" type="button" data-add-kind="netbox">${h("netbox_2")}</button>
      </div>
      <div id="source-drafts"></div>
    </div>`;
  const dialog = openEditDialog({
    id: "inventory-dialog",
    title: editing ? item.name : t("new_inventory"),
    body: `<label class="field required"><span>${h("name")}</span><input name="name" required value="${editing ? esc(item.name) : ""}"></label>${editing ? "" : sources}`,
    submit: editing ? t("save") : t("create"),
    onSubmit: async ({ form, close }) => {
      if (editing) {
        await api("PUT", `/api/v1/inventories/${encodeURIComponent(item.id)}`, { name: form.name.value });
        markSaved();
        close();
        await route();
      } else {
        const drafts = [...dialog.overlay.querySelectorAll("#source-drafts [data-kind]")].map(readSourceDraft);
        const created = await api("POST", "/api/v1/inventories", { name: form.name.value, sources: drafts });
        markSaved();
        close();
        go(`#/inventories/${created.id}`);
      }
    },
  });
  if (!editing) bindSourcePicks(projects, credentials);
}

export function inventoryHostState() {
  const params = new URLSearchParams((location.hash || "").split("?")[1] || "");
  const size = Number(params.get("size"));
  return {
    page: Math.max(1, Number(params.get("page")) || 1),
    search: params.get("q") || "",
    group: params.get("group") || "",
    size: [25, 50, 100].includes(size) ? size : 50,
    sort: params.get("sort") || "name",
    tab: params.get("tab") === "groups" ? "groups" : "hosts",
  };
}

export function inventoryHostHash(id, state) {
  const params = new URLSearchParams();
  if (state.page > 1) params.set("page", String(state.page));
  if (state.search) params.set("q", state.search);
  if (state.group) params.set("group", state.group);
  if (state.size !== 50) params.set("size", String(state.size));
  if (state.sort && state.sort !== "name") params.set("sort", state.sort);
  if (state.tab === "groups") params.set("tab", "groups");
  const query = params.toString();
  return `#/inventories/${encodeURIComponent(id)}${query ? `?${query}` : ""}`;
}

export function inventoryHostUrl(id, state) {
  const params = new URLSearchParams({
    page: String(state.page),
    page_size: String(state.size),
  });
  if (state.search) params.set("search", state.search);
  if (state.group) params.set("group", state.group);
  if (state.sort) params.set("sort", state.sort);
  return `/api/v1/inventories/${encodeURIComponent(id)}/hosts?${params}`;
}

function inventoryHostTable(hosts, state, id) {
  const head = (label, field) => {
    const active = state.sort === field || state.sort === `-${field}`;
    const descending = active && state.sort.startsWith("-");
    const next = active && !descending ? `-${field}` : field;
    const aria = active ? (descending ? "descending" : "ascending") : "none";
    return `<th aria-sort="${aria}"><button class="btn compact" type="button" data-host-sort="${next}">${h(label)}${active ? (descending ? " ↓" : " ↑") : ""}</button></th>`;
  };
  const rows = hosts.map((host) => {
    const link = id
      ? `<a class="mono" href="#/inventories/${encodeURIComponent(id)}/hosts/${encodeURIComponent(host.name)}">${esc(host.name)}</a>`
      : `<span class="mono">${esc(host.name)}</span>`;
    return `<tr><td>${link}</td><td class="mono">${esc((host.vars || {}).ansible_host || "")}</td><td>${esc((host.groups || []).join(", "))}</td></tr>`;
  });
  return `<div class="table-wrap inventory-hosts-table"><table class="table"><thead><tr>${head("name", "name")}${head("address", "address")}${head("groups", "groups")}</tr></thead><tbody>${rows.join("")}</tbody></table></div>`;
}

function inventoryGroupTable(groups, id) {
  const rows = groups.map((group) => {
    const link = `<a href="#/inventories/${encodeURIComponent(id)}/groups/${encodeURIComponent(group.name)}">${esc(group.name)}</a>`;
    return `<tr><td>${link}</td><td>${esc(group.host_count)}</td><td>${esc((group.children || []).join(", "))}</td></tr>`;
  });
  return `<div class="table-wrap inventory-groups-table"><table class="table"><thead><tr><th>${h("name")}</th><th>${h("hosts")}</th><th>${h("groups")}</th></tr></thead><tbody>${rows.join("")}</tbody></table></div>`;
}

async function pageInventory(id) {
  if (!needTeam()) return;
  const view = inventoryHostState();
  const [item, projects, credentials, firstPage, groupPage] = await Promise.all([
    api("GET", `/api/v1/inventories/${encodeURIComponent(id)}`),
    can("operator") ? api("GET", "/api/v1/projects") : Promise.resolve([]),
    api("GET", "/api/v1/credentials"),
    api("GET", inventoryHostUrl(id, view)),
    api("GET", `/api/v1/inventories/${encodeURIComponent(id)}/groups`),
  ]);
  let page = firstPage;
  const pages = () => Math.max(1, Math.ceil(Number(page.total) / view.size));
  if (view.page > pages()) {
    view.page = pages();
    appState.setShownHash(inventoryHostHash(id, view));
    history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", appState.shownHash);
    page = await api("GET", inventoryHostUrl(id, view));
  }
  const notice = sourceNotice && sourceNotice.inventoryId === id ? sourceNotice : null;
  const external = (item.sources || []).some((source) => source.kind === "project" || source.kind === "netbox");
  const groups = page.groups || [];
  const groupOptions = [
    `<option value="">${h("all_groups")}</option>`,
    ...groups.map((group) => `<option value="${esc(group)}"${group === view.group ? " selected" : ""}>${esc(group)}</option>`),
  ].join("");
  const sourceCards = (item.sources || []).map((source) => `
    <section class="card" style="margin-top:10px">
      <div class="spread">
        <div><strong>${esc(sourceTitle(source))}</strong>${certificateBadge(certificateCheckOff(source))} <span class="small muted">${esc(sourceKindLabel(source.kind))} · ${esc(sourceSummary(source, credentials))}</span></div>
        <div class="row">
          ${can("operator") ? `<span class="small muted" data-source-status="${esc(source.id)}" aria-live="polite">${notice && notice.sourceId === source.id ? esc(notice.text) : ""}</span>` : ""}
          ${can("operator") ? `<button class="btn" type="button" data-edit-source="${esc(source.id)}">${h("edit")}</button>` : ""}
          ${can("operator") && source.kind !== "ini" ? `<button class="btn" type="button" data-sync-source="${esc(source.id)}">${h("refresh")}</button>` : ""}
          ${can("operator") ? `<button class="btn danger" type="button" data-drop-source="${esc(source.id)}">${h("remove")}</button>` : ""}
        </div>
      </div>
      <p class="small muted">${h("count_hosts", { count: source.host_count })}</p>
      <button class="btn" type="button" data-raw-source="${esc(source.id)}" aria-expanded="false" aria-controls="source-raw-${esc(source.id)}">${h("show_raw_data")}</button>
      <div class="source-raw" id="source-raw-${esc(source.id)}" hidden><pre class="mono" data-raw-body></pre></div>
    </section>`).join("");
  shell(item.name, `
    <section class="card" style="margin-bottom:14px">
      <div class="spread">
        <h2>${h("hosts")}</h2>
        ${external && can("operator") ? `<button class="btn" id="sync-inventory" type="button">${h("refresh_all_sources")}</button>` : ""}
      </div>
      <div class="row" id="inventory-tabs">
        <button class="btn pill${view.tab === "hosts" ? " primary" : ""}" type="button" data-inventory-tab="hosts">${h("hosts")}</button>
        <button class="btn pill${view.tab === "groups" ? " primary" : ""}" type="button" data-inventory-tab="groups">${h("groups")}</button>
      </div>
      <div id="host-view"${view.tab === "groups" ? " hidden" : ""}>
        <div class="row">
          <input class="search" id="host-search" type="search" value="${esc(view.search)}" placeholder="${h("search_hosts")}" aria-label="${h("search_hosts")}" autocomplete="off">
          <label class="field"><span>${h("group")}</span><select id="host-group" aria-label="${h("group")}">${groupOptions}</select></label>
          <label class="field"><span>${h("page_size")}</span><select id="host-size" aria-label="${h("page_size")}">${[25, 50, 100].map((size) => `<option value="${size}"${size === view.size ? " selected" : ""}>${size}</option>`).join("")}</select></label>
        </div>
        <p class="small muted" id="host-total" tabindex="-1" aria-live="polite"></p>
        <div id="host-table"></div>
        <div class="row" id="host-pager" tabindex="-1"></div>
      </div>
      <div id="group-view"${view.tab === "hosts" ? " hidden" : ""}>
        <p class="small muted" id="group-total" tabindex="-1" aria-live="polite"></p>
        <div id="group-table"></div>
      </div>
    </section>
    <section>
      <h2>${h("sources")}</h2>
      ${sourceCards || `<p class="small muted">${h("no_source_yet")}</p>`}
      <div id="source-edit"></div>
      ${can("operator") ? `<div class="card" id="add-source-card" style="margin-top:14px">
        <h2>${h("add_source")}</h2>
        <p class="small muted">${h("an_inventory_can_have_several_sources_they_are")}</p>
        <div class="choice-row">
          <button class="btn pill" type="button" data-add-kind="ini">${h("manual_2")}</button>
          <button class="btn pill" type="button" data-add-kind="project">${h("project_2")}</button>
          <button class="btn pill" type="button" data-add-kind="netbox">${h("netbox_2")}</button>
        </div>
        <div id="source-drafts"></div>
        <button class="btn pill primary" id="add-sources" type="button" style="margin-top:12px">${h("add")}</button>
      </div>` : ""}
    </section>
    ${can("operator") ? `<section class="card danger-zone" style="margin-top:14px">
      <h2>${h("danger_zone")}</h2>
      <p class="small muted">${h("deleting_cannot_be_undone")}</p>
      <button class="btn danger" id="delete-inventory" type="button">${h("delete_inventory")}</button>
    </section>` : ""}`,
    can("operator") ? `<a class="btn pill" href="#/inventories?edit=${esc(item.id)}">${h("edit")}</a><a class="btn pill" href="#/inventories?new=1">${h("inventory_2")}</a>` : "");
  if (notice) sourceNotice = null;
  const hostSearch = document.querySelector("#host-search");
  const hostGroup = document.querySelector("#host-group");
  const hostSize = document.querySelector("#host-size");
  const hostTable = document.querySelector("#host-table");
  const hostTotal = document.querySelector("#host-total");
  const hostPager = document.querySelector("#host-pager");
  const groupTotal = document.querySelector("#group-total");
  const groupTable = document.querySelector("#group-table");
  const drawGroups = (data) => {
    const groups = data.groups || [];
    groupTotal.textContent = groups.length
      ? t("groups_shown", { count: groups.length })
      : t("no_groups_yet");
    groupTable.innerHTML = groups.length
      ? inventoryGroupTable(groups, id)
      : `<p class="small muted">${h("no_groups_yet")}</p>`;
  };
  const rememberHash = (push = false) => {
    const next = inventoryHostHash(id, view);
    if (push) history.pushState({ idx: ((history.state && history.state.idx) || 0) + 1 }, "", next);
    else history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", next);
    appState.setShownHash(next);
  };
  const drawHosts = (data) => {
    view.page = Math.max(1, Number(data.page) || view.page);
    const hosts = data.hosts || [];
    const total = Number(data.total) || 0;
    if (!hosts.length) {
      hostTable.innerHTML = view.search || view.group
        ? `<p class="small muted">${h("no_hosts_match")}</p><button class="btn" id="clear-host-filter" type="button">${h("clear_filter")}</button>`
        : `<p class="small muted">${h("no_hosts_yet")}</p>${can("operator") ? `<button class="btn" id="host-add-source" type="button">${h("add_source")}</button>` : ""}`;
    } else {
      hostTable.innerHTML = inventoryHostTable(hosts, view, id);
    }
    hostTotal.textContent = total
      ? t("hosts_shown_of_total", { shown: hosts.length, total })
      : view.search || view.group ? t("no_hosts_match") : t("no_hosts_yet");
    const totalPages = Math.max(1, Math.ceil(total / view.size));
    hostPager.innerHTML = totalPages > 1
      ? `<button class="btn" type="button" data-host-page="${view.page - 1}"${view.page <= 1 ? " disabled" : ""}>${h("previous_page")}</button>
        <span class="small muted">${h("page_of_pages", { page: view.page, pages: totalPages })}</span>
        <button class="btn" type="button" data-host-page="${view.page + 1}"${view.page >= totalPages ? " disabled" : ""}>${h("next_page")}</button>`
      : "";
  };
  const focusHost = (selector) => {
    const target = selector ? document.querySelector(selector) : null;
    (target || hostPager).focus();
  };
  const loadHosts = async (focus) => {
    hostTotal.textContent = t("loading");
    try {
      const data = await api("GET", inventoryHostUrl(id, view));
      const totalPages = Math.max(1, Math.ceil(Number(data.total) / view.size));
      if (view.page > totalPages) {
        view.page = totalPages;
        rememberHash();
        return loadHosts(focus);
      }
      page = data;
      drawHosts(data);
      focusHost(focus);
    } catch (error) {
      hostTotal.textContent = "";
      handle(error);
    }
  };
  hostSearch.addEventListener("input", () => {
    if (hostSearch.hostTimer) clearTimeout(hostSearch.hostTimer);
    hostSearch.hostTimer = setTimeout(() => {
      view.search = hostSearch.value.trim();
      view.page = 1;
      rememberHash();
      loadHosts("#host-search");
    }, 200);
  });
  hostGroup.addEventListener("change", () => {
    view.group = hostGroup.value;
    view.page = 1;
    rememberHash(true);
    loadHosts("#host-group");
  });
  hostSize.addEventListener("change", () => {
    view.size = Number(hostSize.value) || 50;
    view.page = 1;
    rememberHash(true);
    loadHosts("#host-size");
  });
  hostTable.addEventListener("click", (event) => {
    const clear = event.target.closest("#clear-host-filter");
    if (clear) {
      view.search = "";
      view.group = "";
      view.page = 1;
      hostSearch.value = "";
      hostGroup.value = "";
      rememberHash();
      loadHosts("#host-search");
      return;
    }
    const add = event.target.closest("#host-add-source");
    if (add) {
      document.querySelector("#add-source-card")?.scrollIntoView({ block: "start" });
      document.querySelector("#add-source-card [data-add-kind]")?.focus();
      return;
    }
    const sort = event.target.closest("[data-host-sort]");
    if (sort) {
      view.sort = sort.dataset.hostSort;
      view.page = 1;
      rememberHash(true);
      loadHosts(`[data-host-sort="${view.sort}"]`);
    }
  });
  hostPager.addEventListener("click", (event) => {
    const button = event.target.closest("[data-host-page]");
    if (!button || button.disabled) return;
    view.page = Number(button.dataset.hostPage);
    rememberHash(true);
    loadHosts(`[data-host-page="${view.page}"]`);
  });
  drawHosts(page);
  drawGroups(groupPage);
  document.querySelectorAll("[data-inventory-tab]").forEach((button) => {
    button.addEventListener("click", () => {
      view.tab = button.dataset.inventoryTab === "groups" ? "groups" : "hosts";
      view.page = 1;
      rememberHash(true);
      route();
    });
  });
  document.querySelectorAll("[data-raw-source]").forEach((button) => {
    button.addEventListener("click", async () => {
      const panel = document.getElementById(`source-raw-${button.dataset.rawSource}`);
      if (!panel) return;
      const show = panel.hidden;
      if (show && !button.dataset.loaded) {
        button.disabled = true;
        try {
          const source = await api("GET", `/api/v1/inventories/${encodeURIComponent(id)}/sources/${encodeURIComponent(button.dataset.rawSource)}`);
          panel.querySelector("[data-raw-body]").textContent = source.body || "";
          button.dataset.loaded = "1";
        } catch (error) {
          handle(error);
          button.disabled = false;
          return;
        }
        button.disabled = false;
      }
      panel.hidden = !show;
      button.textContent = t(show ? "hide_raw_data" : "show_raw_data");
      button.setAttribute("aria-expanded", String(show));
    });
  });
  const sync = document.querySelector("#sync-inventory");
  if (sync) {
    sync.addEventListener("click", async () => {
      sync.disabled = true;
      try {
        await api("POST", `/api/v1/inventories/${encodeURIComponent(id)}/sync`);
        route();
      } catch (error) {
        sync.disabled = false;
        handle(error);
      }
    });
  }
  document.querySelectorAll("[data-edit-source]").forEach((button) => {
    button.addEventListener("click", async () => {
      const target = document.querySelector("#source-edit");
      const summary = (item.sources || []).find((row) => row.id === button.dataset.editSource);
      if (!target || !summary) return;
      target.innerHTML = `<p class="small muted">${h("loading")}</p>`;
      try {
        const source = await api("GET", `/api/v1/inventories/${encodeURIComponent(id)}/sources/${encodeURIComponent(summary.id)}`);
        target.innerHTML = "";
        const card = mountSourceDraft(target, projects, source.kind, {
          ...(source.source || {}),
          name: source.name,
          body: source.body,
          credential_id: source.credential_id || "",
        }, { editing: source.id, credentials });
        const errorNote = card.querySelector("[data-source-error]");
        card.querySelector("[data-save-draft]").addEventListener("click", async () => {
          const invalid = [...card.querySelectorAll("input, select, textarea")].find((control) => !control.checkValidity());
          if (invalid) {
            invalid.reportValidity();
            return;
          }
          errorNote.textContent = "";
          const save = card.querySelector("[data-save-draft]");
          save.disabled = true;
          try {
            const saved = await api("PUT", `/api/v1/inventories/${encodeURIComponent(id)}/sources/${encodeURIComponent(source.id)}`, readSourceUpdate(card));
            const collisions = saved.collisions || [];
            sourceNotice = saved.sync && saved.sync.ok
              ? {
                inventoryId: id,
                sourceId: source.id,
                text: collisions.length
                  ? t("netbox_collisions", { groups: collisions.join(", ") })
                  : t("hosts_before_after", { before: saved.sync.before, after: saved.sync.after }),
              }
              : null;
            markSaved();
            route();
          } catch (error) {
            save.disabled = false;
            errorNote.textContent = t(error.message || "error.generic", error.vars);
            handle(error);
          }
        });
      } catch (error) {
        target.innerHTML = "";
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-sync-source]").forEach((button) => {
    button.addEventListener("click", async () => {
      const status = document.querySelector(`[data-source-status="${button.dataset.syncSource}"]`);
      button.disabled = true;
      button.textContent = t("refreshing");
      if (status) status.textContent = t("refreshing");
      try {
        const item = await api("POST", `/api/v1/inventories/${encodeURIComponent(id)}/sources/${encodeURIComponent(button.dataset.syncSource)}/sync`);
        const collisions = item.collisions || [];
        await route();
        const fresh = document.querySelector(`[data-source-status="${button.dataset.syncSource}"]`);
        const synced = (item.sources || []).find((row) => row.id === button.dataset.syncSource);
        if (fresh) {
          fresh.textContent = collisions.length
            ? t("netbox_collisions", { groups: collisions.join(", ") })
            : t("updated_hosts", { count: synced ? synced.host_count : 0 });
        }
      } catch (error) {
        button.disabled = false;
        button.textContent = t("refresh");
        if (status) status.textContent = t(error.message || "error.generic", error.vars);
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-drop-source]").forEach((button) => {
    button.addEventListener("click", async () => {
      const source = (item.sources || []).find((row) => row.id === button.dataset.dropSource);
      if (!(await confirmDialog({ title: t("remove"), body: t("remove_this_source", { name: source ? sourceTitle(source) : "" }), confirmLabel: t("remove"), danger: true }))) return;
      try {
        await api("DELETE", `/api/v1/inventories/${encodeURIComponent(id)}/sources/${encodeURIComponent(button.dataset.dropSource)}`);
        route();
      } catch (error) { handle(error); }
    });
  });
  if (can("operator")) {
    bindSourcePicks(projects, credentials);
    document.querySelector("#add-sources").addEventListener("click", async () => {
      const drafts = [...document.querySelectorAll("#source-drafts [data-kind]")].map(readSourceDraft);
      if (!drafts.length) return;
      try {
        for (const source of drafts) {
          await api("POST", `/api/v1/inventories/${encodeURIComponent(id)}/sources`, source);
        }
        markSaved();
        route();
      } catch (error) { handle(error); }
    });
    document.querySelector("#delete-inventory").addEventListener("click", async () => {
      if (!(await confirmDialog({ title: t("delete_inventory"), body: t("delete_this_inventory", { name: item.name }), confirmLabel: t("delete"), danger: true }))) return;
      try {
        await api("DELETE", `/api/v1/inventories/${encodeURIComponent(id)}`);
        go("#/inventories");
      } catch (error) { handle(error); }
    });
  }
  appState.setLeaveGuard(sourceEditGuard);
}

export function sourceEditGuard() {
  return document.querySelector("#source-edit [data-dirty]") !== null
    || document.querySelector("#source-drafts [data-kind]") !== null;
}

function looksSecretVar(name) {
  return /(pass(word|phrase)?|secret|token|api[-_]?key|private[-_]?key|access[-_]?key|auth[-_]?key|credential|vault)/i.test(name);
}

function variableText(item) {
  return looksSecretVar(item.name) ? "••••••" : String(item.value ?? "");
}

function variableOrigin(item) {
  const origin = item.origin === "host" ? t("host") : String(item.origin ?? "");
  return origin.replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]+/g, " ").trim();
}

function yamlPlain(text) {
  return /^[A-Za-z0-9_][A-Za-z0-9_.\/-]*$/.test(text)
    && !/^(true|false|yes|no|on|off|null|none)$/i.test(text)
    && !/^[+-]?(\d+(\.\d+)?|\.\d+)$/.test(text);
}

function variableScalar(value) {
  const text = String(value ?? "");
  return yamlPlain(text) ? esc(text) : esc(JSON.stringify(text));
}

function variableKey(name) {
  const text = String(name ?? "");
  return /^[A-Za-z_][A-Za-z0-9_.-]*$/.test(text) ? esc(text) : esc(JSON.stringify(text));
}

function effectiveVariablesYaml(vars) {
  return vars.map((item) => {
    const value = variableScalar(variableText(item));
    return `<span class="vars-key">${variableKey(item.name)}</span>: <span class="vars-value">${value}</span> <span class="vars-comment"># ${esc(variableOrigin(item))}</span>`;
  }).join("\n");
}

function jsonVariableMarkup(text) {
  const pattern = /("(?:\\.|[^"\\])*")(?=\s*:)|("(?:\\.|[^"\\])*")|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|\b(true|false|null)\b/g;
  let result = "";
  let last = 0;
  let match;
  while ((match = pattern.exec(text)) !== null) {
    result += esc(text.slice(last, match.index));
    if (match[1]) result += `<span class="vars-key">${esc(match[1])}</span>`;
    else if (match[2]) result += `<span class="vars-value">${esc(match[2])}</span>`;
    else result += esc(match[0]);
    last = pattern.lastIndex;
  }
  return result + esc(text.slice(last));
}

function effectiveVariablesJson(vars) {
  const rows = vars.map((item) => ({
    name: String(item.name ?? ""),
    value: looksSecretVar(item.name) ? "••••••" : (item.value ?? ""),
    origin: variableOrigin(item),
  }));
  return jsonVariableMarkup(JSON.stringify(rows, null, 2));
}

function variablesViewer(vars) {
  variablesSerial += 1;
  const id = `vars-${variablesSerial}`;
  const empty = `<p class="small muted">${h("no_variables")}</p>`;
  const yaml = vars.length ? `<pre class="vars">${effectiveVariablesYaml(vars)}</pre>` : empty;
  const json = vars.length ? `<pre class="vars">${effectiveVariablesJson(vars)}</pre>` : empty;
  return `<div class="vars-view" data-vars-view>
    <div class="spread tab-bar">
      <div class="tabs" role="tablist" aria-label="${h("variables")}">
        <button class="btn pill primary" type="button" role="tab" data-vars-tab="yaml" aria-selected="true" aria-controls="${id}-yaml" tabindex="0">${h("yaml")}</button>
        <button class="btn pill" type="button" role="tab" data-vars-tab="json" aria-selected="false" aria-controls="${id}-json" tabindex="-1">${h("json")}</button>
      </div>
    </div>
    <div id="${id}-yaml" role="tabpanel" data-vars-panel="yaml">${yaml}</div>
    <div id="${id}-json" role="tabpanel" data-vars-panel="json" hidden>${json}</div>
  </div>`;
}

function bindVariablesViews(root = document) {
  root.querySelectorAll("[data-vars-view]").forEach((view) => {
    const buttons = [...view.querySelectorAll("[data-vars-tab]")];
    const panels = [...view.querySelectorAll("[data-vars-panel]")];
    const choose = (format, focus) => {
      buttons.forEach((button) => {
        const active = button.dataset.varsTab === format;
        button.classList.toggle("primary", active);
        button.setAttribute("aria-selected", String(active));
        button.tabIndex = active ? 0 : -1;
        if (active && focus) button.focus();
      });
      panels.forEach((panel) => {
        panel.hidden = panel.dataset.varsPanel !== format;
      });
    };
    buttons.forEach((button, index) => {
      button.addEventListener("click", () => choose(button.dataset.varsTab, false));
      button.addEventListener("keydown", (event) => {
        let next = null;
        if (event.key === "ArrowRight") next = (index + 1) % buttons.length;
        else if (event.key === "ArrowLeft") next = (index - 1 + buttons.length) % buttons.length;
        else if (event.key === "Home") next = 0;
        else if (event.key === "End") next = buttons.length - 1;
        if (next === null) return;
        event.preventDefault();
        choose(buttons[next].dataset.varsTab, true);
      });
    });
  });
}

function inventoryLaunchCard(title, templates, buttonLabel, buttonId, selectId) {
  if (!templates.length) {
    return `<section class="card"><h2>${h(title)}</h2><p class="small muted">${h("no_ansible_templates")}</p></section>`;
  }
  return `<section class="card">
    <h2>${h(title)}</h2>
    <div class="row">
      <label class="field"><span>${h("template")}</span><select id="${selectId}">${templates.map((item) => `<option value="${esc(item.id)}">${esc(item.name)}</option>`).join("")}</select></label>
      <button class="btn pill primary" id="${buttonId}" type="button">${h(buttonLabel)}</button>
    </div>
  </section>`;
}

function bindInventoryLaunch(buttonId, selectId, templates, preset) {
  const button = document.querySelector(`#${buttonId}`);
  if (!button) return;
  button.addEventListener("click", async () => {
    const item = templates.find((row) => row.id === document.querySelector(`#${selectId}`).value);
    if (!item) return;
    try {
      await openLaunchPrompt(item, !item.prompt_on_launch, preset);
    } catch (error) {
      handle(error);
    }
  });
}

async function pageInventoryHost(id, name) {
  if (!needTeam()) return;
  const [item, host, templates] = await Promise.all([
    api("GET", `/api/v1/inventories/${encodeURIComponent(id)}`),
    api("GET", `/api/v1/inventories/${encodeURIComponent(id)}/hosts/${encodeURIComponent(name)}`),
    can("operator") ? api("GET", "/api/v1/templates") : Promise.resolve([]),
  ]);
  const ansibleTemplates = (templates || []).filter((item) => item.tool === "ansible");
  const groupLinks = (host.groups || []).map((group) => `<a href="#/inventories/${encodeURIComponent(id)}/groups/${encodeURIComponent(group)}">${esc(group)}</a>`).join(", ");
  const source = host.source || {};
  const netbox = host.netbox_url
    ? `<p><a href="${esc(host.netbox_url)}" target="_blank" rel="noopener">${h("open_in_netbox")}</a></p>`
    : "";
  const launch = can("operator") ? inventoryLaunchCard("run_a_template_on_this_host", ansibleTemplates, "run_a_template_on_this_host", "host-launch", "host-template") : "";
  shell(host.name, `
    <p class="small muted"><a href="#/inventories/${encodeURIComponent(id)}">${h("back_to_inventory")}</a> · ${esc(item.name)}</p>
    <section class="card">
      <h2>${h("host")}</h2>
      <p><strong>${esc(host.name)}</strong></p>
      <p>${h("groups")}: ${groupLinks || `<span class="small muted">${h("no_child_groups")}</span>`}</p>
      <p>${h("source")}: ${esc(source.name || "")} <span class="small muted">${esc(sourceKindLabel(source.kind || ""))}</span></p>
      ${netbox}
    </section>
    <section class="card">
      <h2>${h("effective_variables")}</h2>
      <p class="small muted">${h("secret_looking_variables_are_masked")}</p>
      ${variablesViewer(host.vars || [])}
    </section>
    ${launch}`);
  bindVariablesViews();
  bindInventoryLaunch("host-launch", "host-template", ansibleTemplates, { inventoryId: id, limit: host.name });
}

async function pageInventoryGroup(id, name) {
  if (!needTeam()) return;
  const params = new URLSearchParams((location.hash || "").split("?")[1] || "");
  const page = Math.max(1, Number(params.get("page")) || 1);
  const size = 50;
  const [item, group, templates] = await Promise.all([
    api("GET", `/api/v1/inventories/${encodeURIComponent(id)}`),
    api("GET", `/api/v1/inventories/${encodeURIComponent(id)}/groups/${encodeURIComponent(name)}?page=${page}&page_size=${size}`),
    can("operator") ? api("GET", "/api/v1/templates") : Promise.resolve([]),
  ]);
  const ansibleTemplates = (templates || []).filter((item) => item.tool === "ansible");
  const link = (groupName) => `<a href="#/inventories/${encodeURIComponent(id)}/groups/${encodeURIComponent(groupName)}">${esc(groupName)}</a>`;
  const children = (group.children || []).map(link).join(", ") || `<span class="small muted">${h("no_child_groups")}</span>`;
  const parents = (group.parents || []).map(link).join(", ") || `<span class="small muted">${h("no_parent_groups")}</span>`;
  const vars = Object.entries(group.vars || {}).map(([key, value]) => ({ name: key, value, origin: group.name }));
  const totalPages = Math.max(1, Math.ceil(Number(group.total) / size));
  const base = `#/inventories/${encodeURIComponent(id)}/groups/${encodeURIComponent(name)}`;
  const pager = totalPages > 1
    ? `<div class="row" id="group-pager"><button class="btn" type="button" data-group-page="${page - 1}"${page <= 1 ? " disabled" : ""}>${h("previous_page")}</button><span class="small muted">${h("page_of_pages", { page, pages: totalPages })}</span><button class="btn" type="button" data-group-page="${page + 1}"${page >= totalPages ? " disabled" : ""}>${h("next_page")}</button></div>`
    : "";
  const hosts = group.hosts && group.hosts.length
    ? inventoryHostTable(group.hosts, { sort: "name" }, id)
    : `<p class="small muted">${h("no_hosts_yet")}</p>`;
  const launch = can("operator") ? inventoryLaunchCard("use_as_limit", ansibleTemplates, "use_as_limit", "group-launch", "group-template") : "";
  shell(group.name, `
    <p class="small muted"><a href="#/inventories/${encodeURIComponent(id)}">${h("back_to_inventory")}</a> · ${esc(item.name)}</p>
    <section class="card">
      <h2>${h("group")}</h2>
      <p><strong>${esc(group.name)}</strong></p>
      <p>${h("child_groups")}: ${children}</p>
      <p>${h("parent_groups")}: ${parents}</p>
    </section>
    <section class="card">
      <h2>${h("group_variables")}</h2>
      <p class="small muted">${h("secret_looking_variables_are_masked")}</p>
      ${variablesViewer(vars)}
    </section>
    <section class="card">
      <h2>${h("hosts")}</h2>
      <p class="small muted">${h("hosts_shown_of_total", { shown: (group.hosts || []).length, total: group.total })}</p>
      ${hosts}
      ${pager}
    </section>
    ${launch}`);
  bindVariablesViews();
  document.querySelectorAll("[data-group-page]").forEach((button) => {
    button.addEventListener("click", () => {
      if (button.disabled) return;
      go(Number(button.dataset.groupPage) > 1 ? `${base}?page=${button.dataset.groupPage}` : base);
    });
  });
  bindInventoryLaunch("group-launch", "group-template", ansibleTemplates, { inventoryId: id, limit: group.name });
}

registerRedirect("#/inventories/new", "#/inventories?new=1");
registerRoute("#/inventories/:id/hosts/:name", (params) => pageInventoryHost(params.id, decodeURIComponent(params.name)));
registerRoute("#/inventories/:id/groups/:name", (params) => pageInventoryGroup(params.id, decodeURIComponent(params.name)));
registerRoute("#/inventories/:id", (params) => pageInventory(params.id));
registerRoute("#/inventories", pageInventories);
