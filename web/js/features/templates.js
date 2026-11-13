import { api } from "../core/api.js";
import { confirmDialog, openDialog } from "../core/dialog.js";
import { addLink, emptyState, esc, h, selectOptions, table } from "../core/dom.js";
import { t } from "../core/i18n.js";
import { registerRedirect, registerRoute } from "../core/router.js";
import { can, go, handle, markSaved, needTeam, route, shell } from "../core/app.js";
import { bindChips, chipBox, flushChips } from "../core/chips.js";
import { CREDENTIAL_KINDS, credentialKindLabel } from "../core/credentials.js";
import { clearEditHash, editQuery, openEditDialog, rememberEditHash } from "../core/edit.js";
import { environmentToolLabel, scopeLabel, verbosityLabel } from "../core/format.js";
import { jsonObject, showFieldError } from "../core/forms.js";
import { confirmTofuApply } from "../core/tofu.js";
import { bindScheduleActions, openScheduleForm, scheduleTable } from "./schedules.js";

const NO_ENVIRONMENT = "__no_environment__";

export function templateSpec(form) {
  if (form.tool.value === "ansible") {
    const spec = { job_type: form.job_type.value === "check" ? "check" : "run" };
    const tags = form.tags.value.trim();
    const skip = form.skip_tags.value.trim();
    const limit = form.limit.value.trim();
    const forks = form.forks.value.trim();
    if (tags) spec.tags = tags;
    if (skip) spec.skip_tags = skip;
    if (limit) spec.limit = limit;
    if (form.diff.checked) spec.diff = true;
    const verbosity = Number(form.verbosity.value);
    if (verbosity) spec.verbosity = verbosity;
    if (forks) {
      const parsed = Number(forks);
      if (!Number.isInteger(parsed)) throw new Error("forks_is_a_whole_number");
      spec.forks = parsed;
    }
    return spec;
  }
  if (form.tool.value === "opentofu") {
    return { action: form.action.value === "apply" ? "apply" : "plan" };
  }
  const script = form.script.value;
  return script.trim() ? { script } : {};
}

export function filterTemplates(items, tool, query) {
  const needle = String(query || "").trim().toLowerCase();
  return items.filter((item) => {
    if (tool !== "all" && item.tool !== tool) return false;
    if (!needle) return true;
    const spec = item.spec || {};
    return [item.name, item.description, item.tool, item.entrypoint, spec.tags, spec.skip_tags, spec.limit]
      .join(" ")
      .toLowerCase()
      .includes(needle);
  });
}

export function templateEntrypointError(tool, entrypoint) {
  const entry = String(entrypoint || "").trim();
  if (tool === "ansible" && !entry) return "ansible_needs_a_playbook_as_the_entry";
  if (tool === "opentofu" && !entry) return "opentofu_needs_a_module_directory";
  return "";
}

export function templateOptionLine(item) {
  const spec = item.spec || {};
  if (item.tool === "ansible") {
    const bits = [spec.job_type === "check" ? t("check") : t("run_2")];
    if (spec.verbosity) bits.push(t(verbosityLabel(spec.verbosity)));
    if (spec.tags) bits.push(`${t("tags")} ${spec.tags}`);
    if (spec.skip_tags) bits.push(`${t("skip")} ${spec.skip_tags}`);
    if (spec.limit) bits.push(`${t("limit")} ${spec.limit}`);
    if (spec.diff) bits.push(t("diff"));
    return bits.join(" · ");
  }
  if (item.tool === "opentofu") return spec.action === "apply" ? "apply" : "plan";
  return item.entrypoint || t("script");
}

function tofuApply(item) {
  return Boolean(item) && item.tool === "opentofu" && (item.spec || {}).action === "apply";
}

export async function openLaunchPrompt(item, launchWithOptions = false, preset = {}) {
  const previous = document.querySelector("#launch-prompt");
  if (previous) previous.remove();
  const spec = item.spec || {};
  const ansible = item.tool === "ansible";
  const [inventories, credentials] = await Promise.all([
    ansible ? api("GET", "/api/v1/inventories") : Promise.resolve([]),
    api("GET", "/api/v1/credentials"),
  ]);
  const verbosity = Number(spec.verbosity) || 0;
  const inventoryId = preset.inventoryId || item.inventory_id;
  const limit = preset.limit === undefined ? (spec.limit || "") : preset.limit;
  const inventory = ansible
    ? `<label class="field"><span>${h("inventory")}</span><select name="inventory_id">${selectOptions(inventories, inventoryId, true, (row) => row.name)}</select></label>`
    : "";
  const ansibleFields = ansible ? `
    <div class="field-row">
      <label class="field"><span>${h("type")}</span>
        <select class="fit" name="job_type">
          <option value="run"${spec.job_type === "check" ? "" : " selected"}>${h("run_2")}</option>
          <option value="check"${spec.job_type === "check" ? " selected" : ""}>${h("check_dry_run")}</option>
        </select>
      </label>
      <label class="field"><span>${h("verbosity")}</span>
        <select class="fit" name="verbosity">
          ${[0, 1, 2, 3, 4].map((level) => `<option value="${level}"${verbosity === level ? " selected" : ""}>${h(verbosityLabel(level))}</option>`).join("")}
        </select>
      </label>
    </div>
    <div class="field-row">
      ${chipBox("tags", "tags", spec.tags)}
      ${chipBox("skip_tags", "skip_tags", spec.skip_tags)}
    </div>
    <label class="field"><span>${h("limit")}</span><input class="fit" name="limit" value="${esc(limit)}"></label>` : "";
  let submitting = false;
  const dialog = openDialog({
    id: "launch-prompt",
    formId: "launch-prompt-form",
    title: t("launch"),
    body: `
      <p class="small muted">${h(launchWithOptions ? "launch_with_options_help" : "the_launch_dialog_starts_from_the_template")}</p>
      ${inventory}
      ${templateCredentialFields(credentials, selectedCredentialIds(item))}
      ${ansibleFields}
      <label class="field"><span>${h("extra_variables_json")}</span><textarea name="extra_vars">${esc(JSON.stringify(item.extra_vars || {}, null, 2))}</textarea></label>`,
    submit: t("launch"),
    onSubmit: async ({ form, close }) => {
      if (submitting || !flushChips(form)) return;
      submitting = true;
      if (item.tool === "opentofu" && spec.action === "apply") {
        const confirmed = await new Promise((resolve) => {
          let settled = false;
          const finish = (value) => {
            if (settled) return;
            settled = true;
            resolve(value);
          };
          openDialog({
            id: "tofu-apply-confirm",
            title: t("opentofu_apply_confirmation_title"),
            body: `<p>${h("opentofu_apply_confirmation_body", { name: item.name })}</p>`,
            submit: t("apply"),
            onSubmit: ({ close: closeConfirm }) => {
              finish(true);
              closeConfirm();
            },
            onClose: () => finish(false),
          });
        });
        if (!confirmed) {
          submitting = false;
          return;
        }
      }
      const submitButton = form.querySelector('button[type="submit"]');
      if (submitButton) {
        submitButton.disabled = true;
        submitButton.setAttribute("aria-busy", "true");
      }
      try {
        const body = {
          extra_vars: jsonObject(form.extra_vars.value, "extra_variables"),
          credential_ids: [
            ...[...form.querySelectorAll("[data-credential-kind]")].map((select) => select.value).filter(Boolean),
            ...[...form.querySelectorAll("#vault-pills [data-vault-id]")].map((node) => node.dataset.vaultId),
          ],
        };
        if (launchWithOptions) body.launch_with_options = true;
        if (ansible) {
          body.inventory_id = form.inventory_id.value;
          const level = Number(form.verbosity.value);
          body.spec = {
            job_type: form.job_type.value === "check" ? "check" : "run",
            tags: form.tags.value.trim(),
            skip_tags: form.skip_tags.value.trim(),
            limit: form.limit.value.trim(),
            verbosity: Number.isInteger(level) ? level : 0,
          };
        }
        const job = await api("POST", `/api/v1/templates/${encodeURIComponent(item.id)}/launch`, body);
        close();
        go(`#/jobs/${job.id}`);
      } catch (error) {
        submitting = false;
        if (submitButton) {
          submitButton.disabled = false;
          submitButton.removeAttribute("aria-busy");
        }
        throw error;
      }
    },
  });
  const form = dialog.form;
  bindChips(form);
  bindVaultPicker(form, credentials, selectedCredentialIds(item));
}

async function pageTemplates() {
  if (!needTeam()) return;
  const items = await api("GET", "/api/v1/templates");
  shell(t("templates"), `
    <section class="card">
      <div class="spread">
        <input class="search" id="template-search" placeholder="${h("search_templates")}" autocomplete="off">
        <div class="choice-row" id="template-filters">
          <button class="btn pill primary" type="button" data-tool="all">${h("all")}</button>
          <button class="btn pill" type="button" data-tool="ansible">Ansible</button>
          <button class="btn pill" type="button" data-tool="opentofu">OpenTofu</button>
          <button class="btn pill" type="button" data-tool="shell">Shell</button>
        </div>
      </div>
      <div id="template-table"></div>
    </section>`,
    can("operator") ? addLink("#/templates?new=1", "template_2") : "");
  let tool = "all";
  let query = "";
  const draw = () => {
    const shown = filterTemplates(items, tool, query);
    const rows = shown.map((item) => `
      <tr>
        <td><a href="#/templates?edit=${esc(item.id)}">${esc(item.name)}</a><div class="small muted">${esc(item.description)}</div></td>
        <td class="tool narrow-hide">${esc(item.tool)}</td>
        <td class="mono narrow-hide">${esc(item.entrypoint || "—")}</td>
        <td class="small narrow-hide">${esc(templateOptionLine(item))}</td>
        <td>${can("operator") ? `<div class="row"><button class="btn primary" type="button" data-launch="${esc(item.id)}">${h("launch")}</button>${item.tool === "ansible" && !item.prompt_on_launch ? `<button class="btn" type="button" data-launch-options="${esc(item.id)}">${h("launch_with_options")}</button>` : ""}</div>` : ""}</td>
      </tr>`);
    const host = document.querySelector("#template-table");
    if (!shown.length) {
      host.innerHTML = items.length
        ? `<p class="small muted">${h("nothing_here")}</p>`
        : emptyState("empty_templates", can("operator") ? "#/templates?new=1" : "", "new_template");
      return;
    }
    host.innerHTML = table([t("name"), t("tool"), t("entry"), t("run_2"), ""], rows);
    document.querySelectorAll("[data-launch]").forEach((button) => {
      button.addEventListener("click", async () => {
        const item = items.find((row) => row.id === button.dataset.launch);
        if (item && item.prompt_on_launch) {
          try {
            await openLaunchPrompt(item);
          } catch (error) {
            handle(error);
          }
          return;
        }
        if (item && tofuApply(item) && !(await confirmTofuApply(item))) return;
        button.disabled = true;
        try {
          const job = await api("POST", `/api/v1/templates/${encodeURIComponent(button.dataset.launch)}/launch`, { extra_vars: {} });
          go(`#/jobs/${job.id}`);
        } catch (error) {
          button.disabled = false;
          handle(error);
        }
      });
    });
    document.querySelectorAll("[data-launch-options]").forEach((button) => {
      button.addEventListener("click", async () => {
        const item = items.find((row) => row.id === button.dataset.launchOptions);
        if (!item) return;
        try {
          await openLaunchPrompt(item, true);
        } catch (error) {
          handle(error);
        }
      });
    });
  };
  document.querySelector("#template-search").addEventListener("input", (event) => {
    query = event.target.value;
    draw();
  });
  document.querySelectorAll("#template-filters button").forEach((button) => {
    button.addEventListener("click", () => {
      tool = button.dataset.tool;
      document.querySelectorAll("#template-filters button").forEach((item) => {
        item.classList.toggle("primary", item === button);
      });
      draw();
    });
  });
  draw();
  const request = editQuery();
  if (request.create !== null) {
    if (can("operator")) await openTemplateDialog(null, request.tab);
    return;
  }
  if (!request.edit) return;
  const item = items.find((row) => row.id === request.edit);
  if (!item) {
    clearEditHash();
    return;
  }
  await openTemplateDialog(item, request.tab);
}

export function environmentRuntimeWarning(mode, runtime, environmentId) {
  if (!environmentId || runtime) return "";
  const selected = mode || "auto";
  if (selected === "auto") return "this_execution_environment_is_ignored_until_docker_or";
  if (selected === "docker") return "an_execution_environment_is_set_but_neither_docker";
  return "";
}

export function playbookChoice(books, entrypoint) {
  if (books.includes(entrypoint)) return entrypoint;
  if (!entrypoint && books.length) return books[0];
  return "";
}

export function environmentsFor(environments, tool) {
  const matching = environments.filter((row) => row.tool === tool);
  const legacy = environments.filter((row) => !row.tool || row.tool === "any");
  return [...matching, ...legacy];
}

async function openTemplateDialog(existing, initialTab = "") {
  if (!needTeam()) return;
  const [projects, inventories, credentials, environments, runner] = await Promise.all([
    api("GET", "/api/v1/projects"),
    api("GET", "/api/v1/inventories"),
    api("GET", "/api/v1/credentials"),
    api("GET", "/api/v1/environments"),
    api("GET", "/api/v1/runner"),
  ]);
  const editing = Boolean(existing);
  const readOnly = !can("operator");
  const spec = (existing && existing.spec) || {};
  const item = existing || {
    name: "", description: "", tool: "ansible", project_id: "", inventory_id: "",
    credential_id: "", credential_ids: [], environment_id: "", entrypoint: "", extra_vars: {}, spec: {},
  };
  const environmentsForTool = (tool) => environmentsFor(environments, tool);
  const environmentDefault = (tool) => runner[`default_environment_${tool}`] || "";
  const noEnvironmentAvailable = (tool) => !runner.container_image
    && (runner.runner_mode || "auto") !== "docker"
    && Boolean(runner.local_tools && runner.local_tools[tool]);
  const environmentOptions = (tool, selected) => {
    const defaultId = environmentDefault(tool);
    const defaultName = environments.find((row) => row.id === defaultId)?.name;
    const label = defaultName ? t("default_environment_named", { name: defaultName }) : t("default_environment_none");
    const noEnvironment = noEnvironmentAvailable(tool);
    const chosen = noEnvironment ? selected : selected === NO_ENVIRONMENT ? "" : selected;
    const noEnvironmentOption = noEnvironment
      ? `<option value="${NO_ENVIRONMENT}"${chosen === NO_ENVIRONMENT ? " selected" : ""}>${h("no_environment_run_on_the_controller")}</option>`
      : "";
    const control = `<option value="">${esc(label)}</option>${noEnvironmentOption}`;
    const rows = environmentsForTool(tool).map((row) =>
      `<option value="${esc(row.id)}"${row.id === chosen ? " selected" : ""}>${esc(`${row.name} (${scopeLabel(row)}, ${environmentToolLabel(row.tool)})`)}</option>`
    ).join("");
    return control + rows;
  };
  const verbosity = Number(spec.verbosity) || 0;
  const general = `
    <label class="field required"><span>${h("name")}</span><input name="name" required value="${esc(item.name)}"></label>
    <label class="field"><span>${h("description")}</span><input name="description" maxlength="1000" value="${esc(item.description)}"></label>
    <label class="field check"><span>${h("prompt_on_launch")}</span><input name="prompt_on_launch" type="checkbox"${item.prompt_on_launch ? " checked" : ""}></label>
    <p class="small muted">${h("the_launch_dialog_starts_from_the_template")}</p>
    <div class="field-row">
      <label class="field"><span>${h("tool")}</span>
        <select class="fit" name="tool">
          <option value="ansible"${item.tool === "ansible" ? " selected" : ""}>Ansible</option>
          <option value="opentofu"${item.tool === "opentofu" ? " selected" : ""}>OpenTofu</option>
          <option value="shell"${item.tool === "shell" ? " selected" : ""}>Shell</option>
        </select>
      </label>
    </div>`;
  const target = `
    <div class="field-grid">
      <label class="field"><span>${h("project")}</span><select name="project_id">${selectOptions(projects, item.project_id, true, (row) => row.name)}</select></label>
      <div id="update-on-launch" hidden>
        <label class="field check"><span>${h("update_the_project_on_launch")}</span><input name="update_on_launch" type="checkbox"${item.update_on_launch === false ? "" : " checked"}></label>
        <p class="small muted">${h("off_uses_the_checkout_from_the_last_sync")}</p>
      </div>
      <label class="field" id="inventory-field"${item.tool === "ansible" ? "" : " hidden"}><span>${h("inventory")}</span><select name="inventory_id">${selectOptions(inventories, item.inventory_id, true, (row) => row.name)}</select></label>
      <label class="field"><span>${h("execution_environment")}</span><select name="environment_id">${environmentOptions(item.tool, item.no_environment ? NO_ENVIRONMENT : item.environment_id)}</select></label>
    </div>
    <p class="banner warn" id="environment-runtime-warning" hidden></p>
    ${templateCredentialFields(credentials, selectedCredentialIds(item), item.tool, readOnly)}
    <label class="field" id="playbook-field" hidden><span>${h("playbook_from_the_project")}</span><select name="playbook"><option value="">${h("other_path")}</option></select></label>
    <p class="small muted" id="entrypoint-note"></p>
    <label class="field" id="entrypoint-field"><span id="entrypoint-label">${h("entry_path")}</span><input name="entrypoint" value="${esc(item.entrypoint)}" placeholder="${h("for_example_site_yml")}"><span class="field-error" id="entrypoint-error" aria-live="polite"></span></label>
    <p class="small muted">${h("without_an_execution_environment_the_template_runs_on")}</p>`;
  const options = `
    <div class="form-section" id="ansible-options"${item.tool === "ansible" ? "" : " hidden"}>
      <h2>Ansible</h2>
      <div class="field-row">
        <label class="field"><span>${h("type")}</span>
          <select class="fit" name="job_type">
            <option value="run"${spec.job_type === "check" ? "" : " selected"}>${h("run_2")}</option>
            <option value="check"${spec.job_type === "check" ? " selected" : ""}>${h("check_dry_run")}</option>
          </select>
        </label>
        <label class="field"><span>${h("verbosity")}</span>
          <select class="fit" name="verbosity">
            ${[0, 1, 2, 3, 4].map((level) => `<option value="${level}"${verbosity === level ? " selected" : ""}>${h(verbosityLabel(level))}</option>`).join("")}
          </select>
        </label>
        <label class="field"><span>${h("forks")}</span><input class="tiny" name="forks" inputmode="numeric" value="${esc(spec.forks || "")}" placeholder="5"></label>
        <label class="field check"><span>${h("diff")}</span><input name="diff" type="checkbox"${spec.diff ? " checked" : ""}></label>
      </div>
      <div class="field-row">
        ${chipBox("tags", "tags", spec.tags)}
        ${chipBox("skip_tags", "skip_tags", spec.skip_tags)}
      </div>
      <p class="small muted">${h("a_comma_or_a_space_turns_the_typed")}</p>
      <div class="field-row">
        <label class="field"><span>${h("limit")}</span><input class="fit" name="limit" value="${esc(spec.limit || "")}" placeholder="web:&db"></label>
      </div>
      <p class="small muted">${h("check_starts_ansible_playbook_with_check_tags_skipped")}</p>
    </div>
    <div class="form-section" id="tofu-options"${item.tool === "opentofu" ? "" : " hidden"}>
      <h2>OpenTofu</h2>
      <div class="field-row">
        <label class="field"><span>${h("action")}</span>
          <select class="fit" name="action">
            <option value="plan"${spec.action === "apply" ? "" : " selected"}>plan</option>
            <option value="apply"${spec.action === "apply" ? " selected" : ""}>apply</option>
          </select>
        </label>
      </div>
      <p class="small muted">${h("opentofu_plan_reads_state_apply_keeps_it")}</p>
    </div>
    <div class="form-section" id="shell-options"${item.tool === "shell" ? "" : " hidden"}>
      <h2>Shell</h2>
      <label class="field"><span>${h("script_when_no_entry_path_is_set")}</span><textarea name="script">${esc(spec.script || "")}</textarea></label>
    </div>`;
  const variables = `<label class="field"><span>${h("extra_variables_json")}</span><textarea name="extra_vars">${esc(JSON.stringify(item.extra_vars || {}, null, 2))}</textarea></label>`;
  const templateSchedules = `
    <p class="small muted">${h("template_schedules_hint")}</p>
    <p class="small muted" id="template-schedules-status" role="status">${h("loading")}</p>
    <div class="table-wrap schedule-table" id="template-schedules"></div>
    ${can("operator") ? `<div class="row"><button class="btn pill primary" type="button" id="template-schedule-add">${h("schedule_add")}</button></div>` : ""}`;
  let vaultPicker = null;
  const dialog = openEditDialog({
    id: "template-dialog",
    title: readOnly ? t("read_only_template", { name: item.name }) : editing ? t("edit_template") : t("new_template"),
    hint: readOnly ? "read_only_template_hint" : undefined,
    tabs: [
      { id: "general", label: t("general"), body: general },
      { id: "target", label: t("target"), body: target },
      { id: "options", label: t("options"), body: options },
      { id: "variables", label: t("variables"), body: variables },
      ...(editing ? [{ id: "schedules", label: t("schedules"), body: templateSchedules }] : []),
    ],
    initialTab,
    wide: true,
    form: true,
    submit: readOnly ? undefined : editing ? t("save") : t("create"),
    danger: !readOnly && editing ? `<button class="btn danger" id="delete-template" type="button">${h("delete")}</button>` : "",
    onSubmit: readOnly ? undefined : async ({ form, close }) => {
      if (!flushChips(form)) return;
      const missing = templateEntrypointError(form.tool.value, chosenEntrypoint());
      if (missing) {
        showEntrypointError(missing);
        form.entrypoint.reportValidity();
        return;
      }
      clearEntrypointError();
      try {
        const selectedProject = projects.find((item) => item.id === form.project_id.value);
        const offeredKinds = new Set(TOOL_CREDENTIAL_KINDS[form.tool.value] || []);
        const legacyBox = form.querySelector("#template-legacy-credentials");
        const credentialIds = [
          ...[...form.querySelectorAll("[data-credential-field]")]
            .filter((field) => !field.hidden)
            .map((field) => field.querySelector("select[data-credential-kind]").value)
            .filter(Boolean),
          ...(offeredKinds.has("vault") && vaultPicker ? vaultPicker.ids() : []),
          ...(legacyBox && !legacyBox.hidden
            ? [...legacyBox.querySelectorAll("[data-legacy-credential-id]")].map((node) => node.dataset.legacyCredentialId)
            : []),
        ];
        const body = {
          name: form.name.value,
          description: form.description.value,
          tool: form.tool.value,
          project_id: form.project_id.value || null,
          inventory_id: form.inventory_id.value || null,
          credential_ids: credentialIds,
          environment_id: form.environment_id.value && form.environment_id.value !== NO_ENVIRONMENT ? form.environment_id.value : null,
          no_environment: form.environment_id.value === NO_ENVIRONMENT,
          update_on_launch: selectedProject && selectedProject.source_type === "git" ? form.update_on_launch.checked : true,
          entrypoint: chosenEntrypoint(),
          extra_vars: jsonObject(form.extra_vars.value, "extra_variables"),
          spec: templateSpec(form),
          prompt_on_launch: form.prompt_on_launch.checked,
        };
        if (editing) await api("PUT", `/api/v1/templates/${encodeURIComponent(item.id)}`, body);
        else await api("POST", "/api/v1/templates", body);
        markSaved();
        close();
        await route();
      } catch (error) {
        if (error && entrypointErrors.has(error.message)) {
          showEntrypointError(error.message, error.vars);
        }
        throw error;
      }
    },
    onTab: (tab) => rememberEditHash(editing ? { edit: item.id, tab } : { create: "1", tab }, false),
  });
  const form = dialog.form;
  const initialCredentialIds = new Set(selectedCredentialIds(item));
  const toolKinds = new Set(Object.values(TOOL_CREDENTIAL_KINDS).flat());
  const legacyCredentialIds = new Set([...initialCredentialIds].filter((id) => {
    const row = credentials.find((credential) => credential.id === id);
    return row && !toolKinds.has(row.kind);
  }));
  const drawLegacyCredentials = () => {
    const fieldset = form.querySelector("#template-legacy-credentials");
    const box = form.querySelector("#legacy-pills");
    if (!fieldset || !box) return;
    if (form.tool.value !== item.tool) {
      box.innerHTML = "";
      fieldset.hidden = true;
      return;
    }
    const offered = new Set(TOOL_CREDENTIAL_KINDS[form.tool.value] || []);
    const ids = new Set(legacyCredentialIds);
    form.querySelectorAll("[data-credential-field]").forEach((field) => {
      if (offered.has(field.dataset.credentialField)) return;
      const select = field.querySelector("select[data-credential-kind]");
      if (select && initialCredentialIds.has(select.value)) ids.add(select.value);
    });
    if (!offered.has("vault") && vaultPicker) {
      vaultPicker.ids().forEach((id) => ids.add(id));
    }
    const rows = [...ids].map((id) => credentials.find((row) => row.id === id)).filter(Boolean);
    box.innerHTML = rows.map((row) => `<span class="chip" data-legacy-credential-id="${esc(row.id)}" data-legacy-kind="${esc(row.kind)}"><span class="chip-text">${esc(`${credentialKindLabel(row.kind)} · ${row.name}`)}</span><button class="chip-remove" type="button"${readOnly ? " disabled" : ""} aria-label="${h("remove_credential_name", { name: row.name })}">×</button></span>`).join("");
    fieldset.hidden = rows.length === 0;
    box.querySelectorAll(".chip-remove").forEach((button) => {
      button.addEventListener("click", () => {
        if (readOnly) return;
        const chip = button.closest(".chip");
        if (!chip) return;
        const id = chip.dataset.legacyCredentialId;
        const select = form.querySelector(`select[data-credential-kind="${chip.dataset.legacyKind}"]`);
        legacyCredentialIds.delete(id);
        if (select && select.value === id) select.value = "";
        if (vaultPicker) vaultPicker.remove(id);
        drawLegacyCredentials();
      });
    });
  };
  const showCredentialKinds = () => {
    const offered = new Set(TOOL_CREDENTIAL_KINDS[form.tool.value] || []);
    form.querySelectorAll("[data-credential-field]").forEach((field) => {
      field.hidden = !offered.has(field.dataset.credentialField);
    });
    const vaultField = form.querySelector("#template-vault-field");
    if (vaultField) vaultField.hidden = !offered.has("vault");
    drawLegacyCredentials();
  };
  if (readOnly) {
    form.querySelectorAll("input, select, textarea").forEach((control) => {
      control.disabled = true;
    });
    form.querySelectorAll("button").forEach((button) => {
      if (button.dataset.dialogTab !== undefined || button.hasAttribute("data-dialog-cancel") || button.classList.contains("dialog-close")) return;
      button.disabled = true;
    });
  }
  const chosenEnvironment = () => {
    const value = form.environment_id.value;
    if (value === NO_ENVIRONMENT) return "";
    return value || environmentDefault(form.tool.value);
  };
  const showEnvironmentWarning = () => {
    const warning = dialog.overlay.querySelector("#environment-runtime-warning");
    if (!warning) return;
    const key = environmentRuntimeWarning(
      runner.runner_mode,
      runner.runtime,
      chosenEnvironment(),
    );
    warning.hidden = !key;
    if (key) warning.textContent = t(key);
  };
  form.environment_id.addEventListener("change", showEnvironmentWarning);
  showEnvironmentWarning();
  bindChips(form);
  vaultPicker = bindVaultPicker(form, credentials, selectedCredentialIds(item), readOnly);
  const entrypointErrors = new Set([
    "ansible_needs_a_playbook_as_the_entry",
    "opentofu_needs_a_module_directory",
    "shell_needs_a_path_or_spec_script",
    "the_entry_path_must_be_relative_and_must",
  ]);
  const chosenEntrypoint = () => {
    return form.entrypoint.value.trim();
  };
  const showEntrypointError = (key, vars) => {
    const node = dialog.overlay.querySelector("#entrypoint-error");
    const message = t(key, vars);
    if (node) node.textContent = message;
    if (key === "shell_needs_a_path_or_spec_script") {
      form.entrypoint.setCustomValidity("");
      return;
    }
    form.entrypoint.setCustomValidity(message);
  };
  const clearEntrypointError = () => {
    const node = dialog.overlay.querySelector("#entrypoint-error");
    if (node) node.textContent = "";
    form.entrypoint.setCustomValidity("");
  };
  const loadPlaybooks = async () => {
    const field = dialog.overlay.querySelector("#playbook-field");
    const note = dialog.overlay.querySelector("#entrypoint-note");
    const select = form.playbook;
    const ansible = form.tool.value === "ansible" && form.project_id.value;
    field.hidden = !ansible;
    note.textContent = "";
    if (!ansible) return;
    const current = form.entrypoint.value.trim();
    try {
      const contents = await api("GET", `/api/v1/projects/${encodeURIComponent(form.project_id.value)}/contents`);
      const books = contents.playbooks || [];
      const known = books.map((path) => `<option value="${esc(path)}">${esc(path)}</option>`).join("");
      select.innerHTML = `${known}<option value="">${h("other_path")}</option>`;
      if (!current && books.length) {
        form.entrypoint.value = books[0];
        clearEntrypointError();
      }
      select.value = playbookChoice(books, current);
      if (!books.length) {
        note.textContent = t("no_playbooks_found_sync_the_git_project_first");
      }
    } catch (error) {
      select.innerHTML = `<option value="">${h("other_path")}</option>`;
      note.textContent = t(error.message || "playbooks_cannot_be_read", error.vars);
    }
  };
  const showToolSections = () => {
    const tool = form.tool.value;
    const available = environmentsForTool(tool);
    const selected = form.environment_id.value;
    form.environment_id.innerHTML = environmentOptions(tool, selected);
    const incompatible = selected && selected !== NO_ENVIRONMENT && !available.some((row) => row.id === selected);
    if (incompatible) {
      form.environment_id.value = "";
      const warning = dialog.overlay.querySelector("#environment-runtime-warning");
      if (warning) {
        warning.hidden = false;
        warning.textContent = t("the_execution_environment_is_for_another_tool");
      }
    }
    dialog.overlay.querySelector("#ansible-options").hidden = tool !== "ansible";
    dialog.overlay.querySelector("#tofu-options").hidden = tool !== "opentofu";
    dialog.overlay.querySelector("#shell-options").hidden = tool !== "shell";
    dialog.overlay.querySelector("#inventory-field").hidden = tool !== "ansible";
    showCredentialKinds();
    const label = dialog.overlay.querySelector("#entrypoint-label");
    if (tool === "ansible") {
      label.textContent = t("playbook_path");
      form.entrypoint.placeholder = t("for_example_site_yml");
    } else if (tool === "opentofu") {
      label.textContent = t("module_directory");
      form.entrypoint.placeholder = t("for_example_current_directory");
    } else {
      label.textContent = t("script_path");
      form.entrypoint.placeholder = t("for_example_scripts_run_sh");
    }
    form.entrypoint.setAttribute("aria-required", tool === "shell" ? "false" : "true");
    dialog.overlay.querySelector("#entrypoint-field").classList.toggle("required", tool !== "shell");
    if (!incompatible) showEnvironmentWarning();
  };
  form.tool.addEventListener("change", () => {
    clearEntrypointError();
    showToolSections();
    loadPlaybooks();
  });
  showToolSections();
  const showProjectUpdate = () => {
    const row = projects.find((item) => item.id === form.project_id.value);
    const box = dialog.overlay.querySelector("#update-on-launch");
    if (box) box.hidden = !(row && row.source_type === "git");
  };
  form.project_id.addEventListener("change", () => {
    showProjectUpdate();
    loadPlaybooks();
  });
  showProjectUpdate();
  form.playbook.addEventListener("change", () => {
    if (form.playbook.value) {
      form.entrypoint.value = form.playbook.value;
      clearEntrypointError();
    }
  });
  form.entrypoint.addEventListener("input", () => {
    clearEntrypointError();
    const value = form.entrypoint.value.trim();
    if (form.tool.value === "ansible" && form.project_id.value) {
      form.playbook.value = [...form.playbook.options].some((option) => option.value === value) ? value : "";
    }
  });
  form.script.addEventListener("input", () => {
    if (form.tool.value === "shell") clearEntrypointError();
  });
  loadPlaybooks();
  if (editing) {
    const scheduleHost = dialog.overlay.querySelector("#template-schedules");
    const scheduleStatus = dialog.overlay.querySelector("#template-schedules-status");
    let scheduleRows = [];
    let scheduleLoaded = false;
    const drawSchedules = () => {
      scheduleHost.innerHTML = scheduleTable(scheduleRows, false, h("schedule_empty_template"));
      bindScheduleActions(scheduleHost, scheduleRows, "template", item.id, loadSchedules, { hash: false });
    };
    const loadSchedules = async () => {
      scheduleLoaded = true;
      try {
        scheduleRows = await api("GET", `/api/v1/templates/${encodeURIComponent(item.id)}/schedules`);
        scheduleStatus.hidden = true;
        drawSchedules();
      } catch (error) {
        scheduleStatus.hidden = false;
        scheduleStatus.textContent = t(error.message || "error.generic", error.vars);
      }
    };
    const schedulesTab = dialog.overlay.querySelector('[data-dialog-tab="schedules"]');
    if (schedulesTab) {
      schedulesTab.addEventListener("click", () => {
        if (!scheduleLoaded) loadSchedules();
      });
    }
    if (initialTab === "schedules") loadSchedules();
    const addSchedule = dialog.overlay.querySelector("#template-schedule-add");
    if (addSchedule) {
      addSchedule.addEventListener("click", () => {
        openScheduleForm("template", item.id, null, loadSchedules, { hash: false });
      });
    }
  }
  const remove = dialog.overlay.querySelector("#delete-template");
  if (remove) {
    remove.addEventListener("click", () => deleteTemplate(item, dialog));
  }
}

async function deleteTemplate(item, dialog) {
  if (!(await confirmDialog({ title: t("delete"), body: t("delete_this_template", { name: item.name }), confirmLabel: t("delete"), danger: true }))) return;
  try {
    await api("DELETE", `/api/v1/templates/${encodeURIComponent(item.id)}`);
    markSaved();
    dialog.close();
    await route();
  } catch (error) {
    handle(error);
  }
}

export const TOOL_CREDENTIAL_KINDS = {
  ansible: ["ssh", "vault"],
  opentofu: ["aws", "azure", "gce", "vmware", "openstack", "kubernetes", "env"],
  shell: ["ssh", "env"],
};

function selectedCredentialIds(item) {
  if (Array.isArray(item.credential_ids)) return item.credential_ids;
  return item.credential_id ? [item.credential_id] : [];
}

function templateCredentialFields(credentials, selectedIds, tool = "", readOnly = false) {
  const selected = new Set(selectedIds || []);
  const present = new Set(credentials.map((row) => row.kind));
  const offered = tool ? new Set(TOOL_CREDENTIAL_KINDS[tool] || []) : null;
  const toolKinds = new Set(Object.values(TOOL_CREDENTIAL_KINDS).flat());
  const selects = CREDENTIAL_KINDS
    .map((row) => row[0])
    .filter((kind) => kind !== "vault" && present.has(kind))
    .filter((kind) => !offered || toolKinds.has(kind))
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
  return `${empty}<p class="small muted">${h("one_credential_of_each_kind_choose_vault_credentials")}</p>${grid}${vault}${legacy}`;
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

registerRedirect("#/templates/new", "#/templates?new=1");
registerRedirect("#/templates/:id", ({ id }) => `#/templates?edit=${encodeURIComponent(id)}`);
registerRoute("#/templates", pageTemplates);
