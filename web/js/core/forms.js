import * as appState from "./state.js";
import { esc, h } from "./dom.js";
import { t } from "./i18n.js";

export const FIELD_ALIAS = {
  extra_variables: "extra_vars",
  user: "username",
  become_user: "become_username",
  session_token: "security_token",
  subscription: "subscription_id",
  client: "client_id",
  vault_password: "password",
};

export const ERROR_FIELD = {
  name_is_missing_or_too_long: "name",
  description_is_too_long: "description",
  limit_contains_an_invalid_character: "limit",
  forks_is_a_number_from_1_to_50: "forks",
  tool_is_ansible_opentofu_or_shell: "tool",
  project_is_missing: "project_id",
  execution_environment_is_missing: "environment_id",
  the_execution_environment_is_for_another_tool: "environment_id",
  the_execution_environment_is_not_visible_to_this: "environment_id",
  action_is_plan_or_apply: "action",
  ansible_needs_a_playbook_as_the_entry: "entrypoint",
  opentofu_needs_a_module_directory: "entrypoint",
  shell_needs_a_path_or_spec_script: "entrypoint",
  the_entry_path_must_be_relative_and_must: "entrypoint",
  extra_variables_must_be_a_json_object: "extra_vars",
  "template.vars_not_objects": "extra_vars",
  revision_may_contain_only_letters_digits_dot_slash: "revision",
  the_git_address_must_be_http_s_ssh: "source",
  the_git_address_contains_credentials: "credential_id",
  the_credential_is_not_a_source_control_credential: "credential_id",
  the_source_control_credential_is_disabled: "credential_id",
  source_is_path_or_git: "source_type",
  the_project_path_is_not_a_directory: "source",
  the_project_path_is_not_allowed: "source",
  path_projects_are_limited_to_administrators: "source",
  the_netbox_address_must_be_http_or_https: "credential_id",
  the_netbox_token_is_invalid: "credential_id",
  netbox_credential_has_the_wrong_kind: "credential_id",
  netbox_credential_is_required: "credential_id",
  netbox_endpoint_must_match_credential: "credential_id",
  netbox_token_needs_a_credential: "credential_id",
  path_is_missing: "path",
  the_file_is_not_an_inventory: "path",
  file_is_missing_from_the_project: "path",
  path_leaves_the_project: "path",
  path_is_not_a_file: "path",
  plugin_files_are_not_copied_as_text_add: "path",
  the_manual_source_is_empty: "body",
  inventory_ini_is_invalid: "body",
  schedule_date_is_too_far_in_the_past: "start_date",
  schedule_time_does_not_exist: "start_time",
  schedule_end_date_is_invalid: "end_date",
  schedule_count_is_invalid: "end_count",
  schedule_end_is_invalid: "end_kind",
  schedule_needs_a_weekday: "weekday",
};

export function fieldRoots() {
  const overlays = [...document.querySelectorAll(".overlay")].reverse();
  return overlays.length ? [...overlays, document] : [document];
}

export function fieldControl(name) {
  if (!name) return null;
  const candidates = [name, FIELD_ALIAS[name]].filter(Boolean);
  const roots = fieldRoots();
  for (const root of roots) {
    for (const candidate of candidates) {
      const found = root.querySelector(`.field [name="${CSS.escape(candidate)}"]`);
      if (found) return found;
    }
  }
  const label = t(name);
  for (const root of roots) {
    for (const field of root.querySelectorAll(".field")) {
      const span = [...field.querySelectorAll("span")].find((node) => !node.classList.contains("field-error") && !node.classList.contains("chip-error"));
      if (span && span.textContent.trim() === label) {
        const control = field.querySelector("input, select, textarea");
        if (control) return control;
      }
    }
  }
  return null;
}

export function showFieldError(control, message) {
  const field = control && control.closest ? control.closest(".field") : null;
  if (!field) return false;
  let note = field.querySelector(".field-error");
  if (!note) {
    note = document.createElement("span");
    note.className = "field-error";
    note.setAttribute("aria-live", "polite");
    field.appendChild(note);
  }
  note.textContent = message;
  control.setAttribute("aria-invalid", "true");
  return true;
}

export function clearControlError(control) {
  if (!control || !control.closest) return;
  const field = control.closest(".field");
  if (!field) return;
  const note = field.querySelector(".field-error");
  if (note && note.id !== "entrypoint-error") note.textContent = "";
  control.removeAttribute("aria-invalid");
}

document.addEventListener("submit", () => {
  appState.setNotedInvalid(false);
}, true);

document.addEventListener("invalid", (event) => {
  const control = event.target;
  if (!control || !control.closest || !control.closest(".field")) return;
  const existing = control.closest(".field").querySelector(".field-error");
  if (existing && existing.textContent.trim()) return;
  event.preventDefault();
  showFieldError(control, t("this_field_needs_a_valid_value"));
  if (!appState.notedInvalid) {
    appState.setNotedInvalid(true);
    control.focus();
  }
}, true);

document.addEventListener("input", (event) => {
  clearControlError(event.target);
}, true);

export function jsonObject(text, label) {
  let value;
  try {
    value = JSON.parse(text.trim() || "{}");
  } catch {
    const error = new Error("field.not_json");
    error.vars = { field: label };
    throw error;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    const error = new Error("field.not_object");
    error.vars = { field: label };
    throw error;
  }
  return value;
}

export function secretControl(name, label, widget, stored) {
  const clear = stored
    ? `<label class="field check"><input type="checkbox" data-clear="${esc(name)}"><span>${h("remove")}</span></label>`
    : "";
  return `<label class="field"><span>${h(label)}</span>${widget}</label>${clear}`;
}

export function secretInput(name, label, type, stored) {
  const inputType = type || "text";
  const complete = inputType === "password" ? "new-password" : "off";
  const filled = stored ? ` value="********" data-stored-mask="1"` : "";
  return secretControl(name, label, `<input name="${name}" type="${inputType}" autocomplete="${complete}"${filled}>`, stored);
}
