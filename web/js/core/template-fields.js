import { chipBox } from "./chips.js";
import { CREDENTIAL_KINDS, credentialKindLabel } from "./credentials.js";
import { esc, h, selectOptions } from "./dom.js";
import { t } from "./i18n.js";
import { verbosityLabel } from "./format.js";

export const TOOL_CREDENTIAL_KINDS = {
  ansible: ["ssh", "vault"],
  opentofu: ["aws", "azure", "gce", "vmware", "openstack", "kubernetes", "env"],
  shell: ["ssh", "env"],
};

export function toolCredentialKinds(tool) {
  return new Set(TOOL_CREDENTIAL_KINDS[tool] || []);
}
export function selectedCredentialIds(item) {
  if (Array.isArray(item.credential_ids)) return item.credential_ids;
  return item.credential_id ? [item.credential_id] : [];
}

export function credentialRows(credentials, tool, selectedIds = []) {
  const offered = toolCredentialKinds(tool);
  const selected = new Set(selectedIds || []);
  return credentials.filter((row) => offered.has(row.kind) || selected.has(row.id));
}

export function credentialHintKey(tool, shownKinds = []) {
  const offered = tool ? toolCredentialKinds(tool) : null;
  if (!offered || offered.has("vault")) return "one_credential_of_each_kind_choose_vault_credentials";
  return shownKinds.length ? "one_credential_of_each_kind" : "";
}

export function refreshCredentialHint(form, tool) {
  const hint = form.querySelector("#template-credential-hint");
  if (!hint) return;
  const shownKinds = [...form.querySelectorAll("[data-credential-field]")]
    .filter((field) => !field.hidden)
    .map((field) => field.dataset.credentialField);
  const key = credentialHintKey(tool, shownKinds);
  hint.hidden = !key;
  hint.textContent = key ? t(key) : "";
}

function credentialLabel(row) { return `${row.name} (${credentialKindLabel(row.kind)})`; }
function control(ids, name) { return `${ids[name] ? ` id="${esc(ids[name])}"` : ""} name="${esc(name)}"`; }

export function inventoryField(inventories, selected, options = {}) {
  const label = options.labelId ? ` id="${esc(options.labelId)}"` : "";
  const hidden = options.hidden ? " hidden" : "";
  return `<label class="field"${label}${hidden}><span>${h("inventory")}</span><select${control({ inventory_id: options.id }, "inventory_id")}>${selectOptions(inventories, selected, true, (row) => row.name)}</select></label>`;
}

export function extraVarsField(value, id = "") {
  return `<label class="field"><span>${h("extra_variables_json")}</span><textarea${control({ extra_vars: id }, "extra_vars")}>${esc(JSON.stringify(value || {}, null, 2))}</textarea></label>`;
}

export function ansibleFields(spec, options = {}) {
  const ids = options.ids || {};
  const only = options.only ? new Set(options.only) : null;
  const wanted = (field) => !only || only.has(field);
  const verbosity = Number(spec.verbosity) || 0;
  const jobType = spec.job_type === "check" ? "check" : "run";
  const type = wanted("job_type")
    ? `<label class="field"><span>${h("type")}</span><select class="fit"${control(ids, "job_type")}><option value="run"${jobType === "check" ? "" : " selected"}>${h("run_2")}</option><option value="check"${jobType === "check" ? " selected" : ""}>${h("check_dry_run")}</option></select></label>`
    : "";
  const verbosityField = wanted("verbosity")
    ? `<label class="field"><span>${h("verbosity")}</span><select class="fit"${control(ids, "verbosity")}>${[0, 1, 2, 3, 4].map((level) => `<option value="${level}"${verbosity === level ? " selected" : ""}>${h(verbosityLabel(level))}</option>`).join("")}</select></label>`
    : "";
  const tags = wanted("tags") ? chipBox("tags", "tags", spec.tags, ids.tags) : "";
  const skipTags = wanted("skip_tags") ? chipBox("skip_tags", "skip_tags", spec.skip_tags, ids.skip_tags) : "";
  const limit = wanted("limit")
    ? `<div class="field-row"><label class="field"><span>${h("limit")}</span><input class="fit"${control(ids, "limit")} value="${esc(spec.limit || "")}" placeholder="web:&db"></label></div>`
    : "";
  if (only) {
    const typeRow = type || verbosityField ? `<div class="field-row">${type}${verbosityField}</div>` : "";
    const tagRow = tags || skipTags ? `<div class="field-row">${tags}${skipTags}</div>` : "";
    const tagHint = tags || skipTags ? `<p class="small muted">${h("a_comma_or_a_space_turns_the_typed")}</p>` : "";
    return `${typeRow}${tagRow}${tagHint}${limit}`;
  }
  const extras = options.extras === false ? "" : `
    <label class="field"><span>${h("forks")}</span><input class="tiny"${control(ids, "forks")} inputmode="numeric" value="${esc(spec.forks ?? "")}" placeholder="5"></label>
    <label class="field check"><span>${h("diff")}</span><input${control(ids, "diff")} type="checkbox"${spec.diff ? " checked" : ""}></label>`;
  return `<div class="field-row">
    ${type}
    ${verbosityField}${extras}
  </div>
  <div class="field-row">${tags}${skipTags}</div>
  <p class="small muted">${h("a_comma_or_a_space_turns_the_typed")}</p>
  ${limit}`;
}

export function tofuFields(spec, options = {}) {
  const action = spec.action === "apply" ? "apply" : "plan";
  return `<div class="field-row">
    <label class="field"><span>${h("action")}</span><select class="fit"${control(options.ids || {}, "action")}><option value="plan"${action === "apply" ? "" : " selected"}>plan</option><option value="apply"${action === "apply" ? " selected" : ""}>apply</option></select></label>
  </div>
  <p class="small muted">${h("opentofu_plan_reads_state_apply_keeps_it")}</p>`;
}

export function credentialFields(credentials, selectedIds, tool = "", options = {}) {
  if (options.singleId) {
    const rows = credentialRows(credentials, tool, selectedIds);
    return `<label class="field"><span>${h("credentials")}</span><select class="fit" id="${esc(options.singleId)}">${selectOptions(rows, (selectedIds || [])[0] || "", true, credentialLabel)}</select></label>`;
  }
  const readOnly = Boolean(options.readOnly);
  const selected = new Set(selectedIds || []);
  const present = new Set(credentials.map((row) => row.kind));
  const offered = tool ? toolCredentialKinds(tool) : null;
  const toolKinds = new Set(Object.values(TOOL_CREDENTIAL_KINDS).flat());
  const selectKinds = CREDENTIAL_KINDS
    .map((row) => row[0])
    .filter((kind) => kind !== "vault" && present.has(kind))
    .filter((kind) => !offered || toolKinds.has(kind));
  const shownKinds = selectKinds.filter((kind) => !offered || offered.has(kind));
  const selects = selectKinds
    .map((kind) => {
      const rows = credentials.filter((row) => row.kind === kind);
      const current = rows.find((row) => selected.has(row.id));
      const hidden = offered && !offered.has(kind) ? " hidden" : "";
      return `<div data-credential-field="${esc(kind)}"${hidden}><label class="field"><span>${esc(credentialKindLabel(kind))}</span><select class="fit" data-credential-kind="${esc(kind)}">${selectOptions(rows, current ? current.id : "", true, (row) => row.name)}</select></label></div>`;
    })
    .join("");
  const grid = selects ? `<div class="field-grid">${selects}</div>` : "";
  const empty = credentials.length ? "" : `<p class="small muted">${h("no_credentials_in_this_team")}</p>`;
  const vaults = credentials.filter((row) => row.kind === "vault");
  const vaultHidden = offered && !offered.has("vault") ? " hidden" : "";
  const vaultPicker = vaults.length && !readOnly
    ? `<div class="vault-picker">
      <input class="search" id="vault-search" type="search" autocomplete="off" role="combobox" aria-autocomplete="list" aria-expanded="false" aria-controls="vault-options" placeholder="${h("search_vaults")}" aria-label="${h("search_vaults")}">
      <div class="vault-options" id="vault-options" role="listbox" hidden></div>
    </div>`
    : "";
  const vaultEmpty = vaults.length ? "" : `<p class="small muted">${h("no_vault_credentials_in_this_team")}</p>`;
  const vault = `<div class="field" id="template-vault-field"${vaultHidden}>
    <span>${esc(credentialKindLabel("vault"))}</span>
    <div class="vault-row">
      <div class="chips" id="vault-pills"></div>
      ${vaultPicker}
    </div>
    ${vaultEmpty}
  </div>`;
  const legacy = tool
    ? `<div class="field" id="template-legacy-credentials" hidden><span>${h("other_credentials")}</span><div class="chips" id="legacy-pills"></div></div>`
    : "";
  const hintKey = credentialHintKey(tool, shownKinds);
  const hint = `<p class="small muted" id="template-credential-hint"${hintKey ? "" : " hidden"}>${hintKey ? h(hintKey) : ""}</p>`;
  return `${empty}${hint}${grid}${vault}${legacy}`;
}

export function readCredentialIds(form, tool) {
  const offered = toolCredentialKinds(tool);
  const ids = new Set();
  form.querySelectorAll("[data-credential-field]").forEach((field) => {
    if (field.hidden) return;
    const select = field.querySelector("select[data-credential-kind]");
    if (select && select.value) ids.add(select.value);
  });
  if (offered.has("vault")) {
    form.querySelectorAll("#vault-pills [data-vault-id]").forEach((node) => ids.add(node.dataset.vaultId));
  }
  form.querySelectorAll("[data-legacy-credential-id]").forEach((node) => ids.add(node.dataset.legacyCredentialId));
  return [...ids];
}

function unfitCredentials(credentials, selectedIds, tool) {
  const offered = toolCredentialKinds(tool);
  const selected = new Set(selectedIds || []);
  return credentials.filter((row) => selected.has(row.id) && !offered.has(row.kind));
}

export function bindLegacyCredentials(form, credentials, selectedIds, tool, readOnly = false) {
  const field = form.querySelector("#template-legacy-credentials");
  const box = form.querySelector("#legacy-pills");
  if (!field || !box) return;
  const rows = unfitCredentials(credentials, selectedIds, tool);
  box.innerHTML = rows.map((row) => `<span class="chip" data-legacy-credential-id="${esc(row.id)}" data-legacy-kind="${esc(row.kind)}"><span class="chip-text">${esc(`${credentialKindLabel(row.kind)} · ${row.name}`)}</span><button class="chip-remove" type="button"${readOnly ? " disabled" : ""} aria-label="${h("remove_credential_name", { name: row.name })}">×</button></span>`).join("");
  field.hidden = rows.length === 0;
  box.querySelectorAll(".chip-remove").forEach((button) => {
    button.addEventListener("click", () => {
      if (readOnly) return;
      const chip = button.closest(".chip");
      if (chip) chip.remove();
      field.hidden = box.querySelectorAll("[data-legacy-credential-id]").length === 0;
    });
  });
}

function vaultSuggestions(credentials, chosen, query) {
  const needle = String(query || "").trim().toLowerCase();
  return credentials
    .filter((row) => row.kind === "vault" && !chosen.includes(row.id))
    .filter((row) => !needle || row.name.toLowerCase().includes(needle))
    .slice()
    .sort((left, right) => left.name.localeCompare(right.name, "de"));
}

function renderVaultPills(box, credentials, ids, readOnly = false) {
  const byId = new Map(credentials.map((row) => [row.id, row]));
  box.innerHTML = ids.map((id) => {
    const row = byId.get(id);
    if (!row) return "";
    const remove = readOnly
      ? ""
      : `<button class="chip-remove" type="button" aria-label="${h("remove_vault_credential_name", { name: row.name })}">×</button>`;
    return `<span class="chip" data-vault-id="${esc(id)}"><span class="chip-text">${esc(row.name)}</span>${remove}</span>`;
  }).join("");
}

export function bindVaultPicker(form, credentials, selectedIds, readOnly = false) {
  const box = form.querySelector("#vault-pills");
  const search = form.querySelector("#vault-search");
  const options = form.querySelector("#vault-options");
  const vaultIds = new Set(credentials.filter((row) => row.kind === "vault").map((row) => row.id));
  let chosen = (selectedIds || []).filter((id) => vaultIds.has(id));
  let open = false;
  let active = 0;
  const drawPills = () => {
    if (box) renderVaultPills(box, credentials, chosen, readOnly);
  };
  const drawOptions = () => {
    if (!search || !options) return;
    const rows = open ? vaultSuggestions(credentials, chosen, search.value) : [];
    if (active >= rows.length) active = Math.max(rows.length - 1, 0);
    options.innerHTML = rows.map((row, index) =>
      `<button class="vault-option${index === active ? " selected" : ""}" type="button" role="option" aria-selected="${index === active}" data-vault-pick="${esc(row.id)}">${esc(row.name)}</button>`
    ).join("");
    options.hidden = rows.length === 0;
    search.setAttribute("aria-expanded", rows.length ? "true" : "false");
  };
  const pick = (id) => {
    if (!vaultIds.has(id) || chosen.includes(id)) return;
    chosen = [...chosen, id];
    if (search) search.value = "";
    open = false;
    active = 0;
    drawPills();
    drawOptions();
  };
  const remove = (id) => {
    chosen = chosen.filter((item) => item !== id);
    drawPills();
    drawOptions();
  };
  drawPills();
  drawOptions();
  if (!box || !search || !options || readOnly) {
    return { ids: () => [...chosen], remove, suggest: (query) => vaultSuggestions(credentials, chosen, query) };
  }
  box.addEventListener("click", (event) => {
    const button = event.target && event.target.closest ? event.target.closest(".chip-remove") : null;
    if (!button) return;
    const chip = button.closest(".chip");
    if (!chip) return;
    remove(chip.dataset.vaultId);
    search.focus();
  });
  options.addEventListener("mousedown", (event) => event.preventDefault());
  options.addEventListener("click", (event) => {
    const button = event.target && event.target.closest ? event.target.closest("[data-vault-pick]") : null;
    if (!button) return;
    pick(button.dataset.vaultPick);
    search.focus();
  });
  search.addEventListener("focus", () => {
    open = true;
    active = 0;
    drawOptions();
  });
  search.addEventListener("blur", () => {
    open = false;
    drawOptions();
  });
  search.addEventListener("input", () => {
    open = true;
    active = 0;
    drawOptions();
  });
  search.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      const rows = vaultSuggestions(credentials, chosen, search.value);
      if (rows[active]) pick(rows[active].id);
      return;
    }
    if (event.key === "Escape") {
      open = false;
      drawOptions();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      const rows = vaultSuggestions(credentials, chosen, search.value);
      if (!rows.length) return;
      event.preventDefault();
      active = (active + (event.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length;
      drawOptions();
    }
  });
  return { ids: () => [...chosen], remove, suggest: (query) => vaultSuggestions(credentials, chosen, query) };
}
