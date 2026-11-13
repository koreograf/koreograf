import { api } from "../core/api.js";
import { confirmDialog, openDialog } from "../core/dialog.js";
import { can, handle, markSaved, needTeam, route, shell } from "../core/app.js";
import { CREDENTIAL_KINDS, credentialKindLabel } from "../core/credentials.js";
import { addLink, emptyState, esc, h, paramRow, paramRowHtml, table } from "../core/dom.js";
import { editQuery, openEditDialog } from "../core/edit.js";
import { secretControl, secretInput } from "../core/forms.js";
import { t } from "../core/i18n.js";
import { registerRedirect, registerRoute } from "../core/router.js";

export function credentialUsageNames(item) {
  return (item.usages || []).map((usage) => usage.project_name || `${usage.inventory_name} / ${usage.source_name}`).join(", ");
}

export function credentialMatches(item, needle) {
  return !needle || `${item.name} ${item.kind} ${credentialKindLabel(item.kind)} ${credentialUsageNames(item)}`.toLowerCase().includes(needle);
}

function credentialUsageCard(item) {
  const names = credentialUsageNames(item);
  return `<section class="card"><h2>${h("used_by")}</h2>${names
    ? `<p>${esc(names)}</p>`
    : `<p class="small muted">${h("not_used")}</p>`}</section>`;
}


function credText(name, label, type, value) {
  const inputType = type || "text";
  const complete = inputType === "password" ? "new-password" : "off";
  const held = value ? ` value="${esc(value)}"` : "";
  return `<label class="field"><span>${h(label)}</span><input name="${name}" type="${inputType}" autocomplete="${complete}"${held}></label>`;
}

function credArea(name, label) {
  return `<label class="field"><span>${h(label)}</span><textarea name="${name}" autocomplete="off" spellcheck="false"></textarea></label>`;
}

function credCheck(name, label, checked) {
  return `<label class="field check"><span>${h(label)}</span><input name="${name}" type="checkbox"${checked ? " checked" : ""}></label>`;
}

function credHint(text) {
  return text ? `<p class="small muted">${h(text)}</p>` : "";
}

function becomeSelect(selected) {
  const current = selected || "";
  const methods = ["sudo", "su", "pbrun", "pfexec", "doas", "dzdo", "ksu", "runas"];
  const options = [`<option value="">${h("none")}</option>`].concat(
    methods.map((method) => `<option value="${method}"${method === current ? " selected" : ""}>${esc(method)}</option>`)
  );
  return `<label class="field"><span>${h("become_method")}</span><select class="fit" name="become_method">${options.join("")}</select></label>`;
}

const FIELD_LABELS = {
  username: "user",
  password: "password",
  private_key: "private_key",
  passphrase: "passphrase",
  become_method: "become_method",
  become_username: "become_user",
  become_password: "become_password",
  vault_id: "vault_id",
  authorize: "authorize",
  authorize_password: "authorize_password",
  access_key: "access_key",
  secret_key: "secret_key",
  security_token: "session_token",
  subscription_id: "subscription",
  client_id: "client",
  secret: "secret",
  tenant: "tenant",
  service_account: "service_account",
  host: "host",
  auth_url: "api_address",
  project: "project",
  domain: "domain",
  url: "api_address",
  token: "token",
  verify_ssl: "verify_certificate",
};

function fieldLabel(key) {
  return FIELD_LABELS[key] || key;
}

function secretArea(name, label, stored) {
  const filled = stored ? ` data-stored-mask="1"` : "";
  const body = stored ? "********" : "";
  return secretControl(
    name,
    label,
    `<textarea name="${name}" class="secret-mask" autocomplete="off" spellcheck="false"${filled}>${body}</textarea>`,
    stored
  );
}

function credentialFields(kind, current) {
  const row = CREDENTIAL_KINDS.find((item) => item[0] === kind);
  const hint = row ? row[2] : "";
  const fields = (current && current.fields) || {};
  const stored = new Set((current && current.set) || []);
  const text = (name, label, type) => credText(name, label, type, fields[name] == null ? "" : String(fields[name]));
  const has = (name) => stored.has(name);
  if (kind === "ssh") {
    return [
      credHint(hint),
      text("username", "user"),
      secretInput("password", "password", "password", has("password")),
      secretArea("private_key", "private_key", has("private_key")),
      secretInput("passphrase", "passphrase", "password", has("passphrase")),
      becomeSelect(fields.become_method),
      text("become_username", "become_user"),
      secretInput("become_password", "become_password", "password", has("become_password")),
    ].join("");
  }
  if (kind === "scm") {
    return [
      credHint(hint),
      credHint("source_control_username_hint"),
      text("username", "user"),
      secretInput("password", "password", "password", has("password")),
      secretArea("private_key", "private_key", has("private_key")),
      secretInput("passphrase", "passphrase", "password", has("passphrase")),
    ].join("");
  }
  if (kind === "vault") {
    return [
      secretInput("password", "vault_password", "password", has("password")),
      text("vault_id", "vault_id"),
    ].join("");
  }
  if (kind === "net") {
    return [
      text("username", "user"),
      secretInput("password", "password", "password", has("password")),
      secretArea("private_key", "private_key", has("private_key")),
      credCheck("authorize", "authorize", Boolean(fields.authorize)),
      secretInput("authorize_password", "authorize_password", "password", has("authorize_password")),
    ].join("");
  }
  if (kind === "aws") {
    return [
      text("access_key", "access_key"),
      secretInput("secret_key", "secret_key", "password", has("secret_key")),
      secretInput("security_token", "session_token", "password", has("security_token")),
    ].join("");
  }
  if (kind === "azure") {
    return [
      text("subscription_id", "subscription"),
      text("client_id", "client"),
      secretInput("secret", "secret", "password", has("secret")),
      text("tenant", "tenant"),
    ].join("");
  }
  if (kind === "gce") return secretArea("service_account", "service_account", has("service_account"));
  if (kind === "vmware") {
    return [
      text("host", "host"),
      text("username", "user"),
      secretInput("password", "password", "password", has("password")),
    ].join("");
  }
  if (kind === "openstack") {
    return [
      credHint(hint),
      text("auth_url", "api_address"),
      text("username", "user"),
      secretInput("password", "password", "password", has("password")),
      text("project", "project"),
      text("domain", "domain"),
    ].join("");
  }
  if (kind === "registry") {
    return [
      credHint(hint),
      text("host", "host"),
      text("username", "user"),
      secretInput("password", "password", "password", has("password")),
    ].join("");
  }
  if (kind === "github" || kind === "gitlab") return secretInput("token", "token", "password", has("token"));
  if (kind === "galaxy") {
    return [credHint(hint), text("url", "api_address"), secretInput("token", "token", "password", has("token"))].join("");
  }
  if (kind === "kubernetes") {
    return [
      text("host", "api_address"),
      secretInput("token", "token", "password", has("token")),
      credCheck("verify_ssl", "verify_certificate", current ? fields.verify_ssl !== false : true),
    ].join("");
  }
  if (kind === "insights") {
    return [text("username", "user"), secretInput("password", "password", "password", has("password"))].join("");
  }
  if (kind === "netbox") {
    return [credHint(hint), text("url", "api_address"), secretInput("token", "token", "password", has("token"))].join("");
  }
  const listed = [...stored].map((name) =>
    `<label class="field check"><input type="checkbox" data-clear="${esc(name)}"><span>${esc(name)} ${h("remove")}</span></label>`
  ).join("");
  const keep = stored.size ? credHint("leave_empty_to_keep_the_stored_value") : "";
  return `${keep}${listed}${credArea("env", "key_value_one_variable_per_line")}`;
}

function inputText(form, name) {
  const field = form.elements[name];
  return field && "value" in field ? field.value.trim() : "";
}

export function credentialSecret(form) {
  const kind = form.kind.value;
  if (kind === "env") {
    const secret = {};
    form.env.value.split("\n").forEach((line) => {
      const trimmed = line.trim();
      if (!trimmed) return;
      const eq = trimmed.indexOf("=");
      if (eq < 1) throw new Error("an_env_line_needs_key_value");
      secret[trimmed.slice(0, eq)] = trimmed.slice(eq + 1);
    });
    return secret;
  }
  const names = {
    ssh: ["username", "password", "private_key", "passphrase", "become_method", "become_username", "become_password"],
    vault: ["password", "vault_id"],
    scm: ["username", "password", "private_key", "passphrase"],
    net: ["username", "password", "private_key", "authorize_password"],
    aws: ["access_key", "secret_key", "security_token"],
    azure: ["subscription_id", "client_id", "secret", "tenant"],
    gce: ["service_account"],
    vmware: ["host", "username", "password"],
    openstack: ["auth_url", "username", "password", "project", "domain"],
    registry: ["host", "username", "password"],
    github: ["token"],
    gitlab: ["token"],
    galaxy: ["url", "token"],
    kubernetes: ["host", "token"],
    insights: ["username", "password"],
    netbox: ["token", "url"],
  };
  const secret = {};
  (names[kind] || []).forEach((name) => {
    const value = inputText(form, name);
    if (value) secret[name] = value;
  });
  if (kind === "net" && form.authorize && form.authorize.checked) secret.authorize = true;
  if (kind === "kubernetes") secret.verify_ssl = Boolean(form.verify_ssl && form.verify_ssl.checked);
  return secret;
}

function bindCredentialKind(form) {
  const box = document.querySelector("#credential-fields");
  const draw = () => {
    box.innerHTML = credentialFields(form.kind.value);
  };
  form.kind.addEventListener("change", draw);
  draw();
}

export function openCredentialCreator(kind, onCreated) {
  document.querySelector("#quick-credential")?.remove();
  openDialog({
    id: "quick-credential",
    formId: "quick-credential-form",
    title: t("new_credential"),
    body: `
      <input type="hidden" name="kind" value="${esc(kind)}">
      <label class="field required"><span>${h("name")}</span><input name="name" required></label>
      <label class="field"><span>${h("type")}</span><input value="${esc(credentialKindLabel(kind))}" disabled></label>
      <div>${credentialFields(kind)}</div>`,
    submit: t("create"),
    onSubmit: async ({ form, close }) => {
      const created = await api("POST", "/api/v1/credentials", {
        name: form.name.value,
        kind: form.kind.value,
        secret: credentialSecret(form),
      });
      close();
      onCreated(created);
    },
  });
}

async function pageCredentials() {
  if (!needTeam()) return;
  const credentials = await api("GET", "/api/v1/credentials");
  shell(t("credentials"), `
    <section class="card">
      <p class="small muted">${h("secrets_are_stored_encrypted_and_are_not_shown")}</p>
      <input class="search" id="credential-search" placeholder="${h("search_credentials")}" aria-label="${h("search_credentials")}" autocomplete="off">
      <div id="credential-table"></div>
    </section>`,
    can("owner") ? addLink("#/credentials?new=1", "credential") : "");
  const drawCredentials = () => {
    const needle = document.querySelector("#credential-search").value.trim().toLowerCase();
    const shown = credentials.filter((item) => credentialMatches(item, needle));
    const host = document.querySelector("#credential-table");
    if (!shown.length) {
      host.innerHTML = credentials.length
        ? `<p class="small muted">${h("nothing_here")}</p>`
        : emptyState("empty_credentials", can("owner") ? "#/credentials?new=1" : "", "new_credential");
      return;
    }
    const rows = shown.map((item) =>
      `<tr><td><a href="#/credentials?edit=${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(credentialKindLabel(item.kind))}</td><td>${esc(credentialUsageNames(item) || t("not_used"))}</td><td>${can("owner") ? `<button class="btn danger" type="button" data-credential="${esc(item.id)}" data-credential-name="${esc(item.name)}">${h("delete")}</button>` : ""}</td></tr>`
    );
    host.innerHTML = table([t("name"), t("type"), t("used_by"), ""], rows);
    document.querySelectorAll("[data-credential]").forEach((button) => {
      button.addEventListener("click", async () => {
        if (!(await confirmDialog({ title: t("delete"), body: t("delete_this_credential", { name: button.dataset.credentialName }), confirmLabel: t("delete"), danger: true }))) return;
        try {
          await api("DELETE", `/api/v1/credentials/${encodeURIComponent(button.dataset.credential)}`);
          route();
        } catch (error) { handle(error); }
      });
    });
  };
  document.querySelector("#credential-search").addEventListener("input", drawCredentials);
  drawCredentials();
  const request = editQuery();
  if (request.create !== null) {
    if (can("owner")) openCredentialDialog(null);
    return;
  }
  if (!request.edit) return;
  try {
    const item = await api("GET", `/api/v1/credentials/${encodeURIComponent(request.edit)}`);
    openCredentialDialog(item, !can("owner"), request.tab);
  } catch (error) {
    handle(error);
  }
}

const SECRET_FIELD_NAMES = {
  ssh: ["password", "private_key", "passphrase", "become_password"],
  scm: ["password", "private_key", "passphrase"],
  vault: ["password"],
  net: ["password", "private_key", "authorize_password"],
  aws: ["secret_key", "security_token"],
  azure: ["secret"],
  gce: ["service_account"],
  vmware: ["password"],
  openstack: ["password"],
  registry: ["password"],
  github: ["token"],
  gitlab: ["token"],
  galaxy: ["token"],
  kubernetes: ["token"],
  insights: ["password"],
  netbox: ["token"],
};

export function credentialUpdateBody(form, kind) {
  const clear = [...form.querySelectorAll("[data-clear]:checked")].map((node) => node.getAttribute("data-clear"));
  if (kind === "env") {
    const secret = {};
    form.env.value.split("\n").forEach((line) => {
      const trimmed = line.trim();
      if (!trimmed) return;
      const eq = trimmed.indexOf("=");
      if (eq < 1) throw new Error("an_env_line_needs_key_value");
      secret[trimmed.slice(0, eq)] = trimmed.slice(eq + 1);
    });
    return { secret, clear };
  }
  const secret = {};
  const hidden = new Set(SECRET_FIELD_NAMES[kind] || []);
  const names = {
    ssh: ["username", "password", "private_key", "passphrase", "become_method", "become_username", "become_password"],
    vault: ["password", "vault_id"],
    scm: ["username", "password", "private_key", "passphrase"],
    net: ["username", "password", "private_key", "authorize_password"],
    aws: ["access_key", "secret_key", "security_token"],
    azure: ["subscription_id", "client_id", "secret", "tenant"],
    gce: ["service_account"],
    vmware: ["host", "username", "password"],
    openstack: ["auth_url", "username", "password", "project", "domain"],
    registry: ["host", "username", "password"],
    github: ["token"],
    gitlab: ["token"],
    galaxy: ["url", "token"],
    kubernetes: ["host", "token"],
    insights: ["username", "password"],
    netbox: ["url", "token"],
  };
  (names[kind] || []).forEach((name) => {
    const field = form.elements[name];
    if (!field || !("value" in field)) return;
    const value = field.value.trim();
    if (hidden.has(name)) {
      if (field.dataset.storedMask !== "1" && value) secret[name] = value;
      return;
    }
    secret[name] = value;
  });
  if (kind === "net" && form.authorize) secret.authorize = Boolean(form.authorize.checked);
  if (kind === "kubernetes" && form.verify_ssl) secret.verify_ssl = Boolean(form.verify_ssl.checked);
  return { secret, clear };
}

function openCredentialDialog(item, readOnly = false, initialTab = "") {
  const editing = Boolean(item);
  if (readOnly) {
    const visible = Object.entries(item.fields || {}).map(([key, value]) => {
      const shown = typeof value === "boolean" ? t(value ? "yes" : "no") : String(value);
      return paramRow(fieldLabel(key), shown);
    }).join("");
    const stored = (item.set || []).map((key) => paramRowHtml(fieldLabel(key), `<input type="password" value="********" disabled tabindex="-1">`)).join("");
    openEditDialog({
      id: "credential-dialog",
      title: item.name,
      body: `<dl class="kv">${paramRow("name", item.name)}${paramRow("type", credentialKindLabel(item.kind))}${visible}${stored}</dl>${credentialUsageCard(item)}`,
      hint: "",
      actions: false,
    });
    return;
  }
  const kindOptions = CREDENTIAL_KINDS.map(([id, label]) => `<option value="${esc(id)}">${h(label)}</option>`).join("");
  const dialog = openEditDialog({
    id: "credential-dialog",
    title: editing ? item.name : t("new_credential"),
    body: `
      <label class="field required"><span>${h("name")}</span><input name="name" required value="${editing ? esc(item.name) : ""}"></label>
      ${editing
        ? `<label class="field"><span>${h("type")}</span><input value="${esc(credentialKindLabel(item.kind))}" disabled></label><p class="small muted">${h("the_type_cannot_be_changed")}</p>`
        : `<label class="field"><span>${h("type")}</span><select name="kind">${kindOptions}</select></label>`}
      <div id="credential-fields">${editing ? credentialFields(item.kind, item) : ""}</div>
      ${editing ? credentialUsageCard(item) : ""}`,
    submit: editing ? t("save") : t("create"),
    hint: "fields_marked_with_a_star_are_required",
    danger: editing ? `<button class="btn danger" type="button" data-credential-delete>${h("delete")}</button>` : "",
    onSubmit: async ({ form, close }) => {
      if (editing) {
        const body = credentialUpdateBody(form, item.kind);
        await api("PUT", `/api/v1/credentials/${encodeURIComponent(item.id)}`, {
          name: form.name.value,
          secret: body.secret,
          clear: body.clear,
        });
      } else {
        await api("POST", "/api/v1/credentials", {
          name: form.name.value,
          kind: form.kind.value,
          secret: credentialSecret(form),
        });
      }
      markSaved();
      close();
      await route();
    },
    initialTab,
  });
  const form = dialog.form;
  if (editing) {
    form.querySelectorAll("[data-stored-mask]").forEach((field) => {
      field.addEventListener("focus", () => {
        if (field.dataset.storedMask === "1") field.select();
      });
      field.addEventListener("input", () => {
        delete field.dataset.storedMask;
      });
    });
    const remove = dialog.overlay.querySelector("[data-credential-delete]");
    remove.addEventListener("click", async () => {
      if (!(await confirmDialog({ title: t("delete"), body: t("delete_this_credential", { name: item.name }), confirmLabel: t("delete"), danger: true }))) return;
      try {
        await api("DELETE", `/api/v1/credentials/${encodeURIComponent(item.id)}`);
        markSaved();
        dialog.close();
        await route();
      } catch (error) { handle(error); }
    });
  } else {
    bindCredentialKind(form);
  }
}

registerRedirect("#/credentials/new", "#/credentials?new=1");
registerRedirect("#/credentials/:id", ({ id }) => `#/credentials?edit=${encodeURIComponent(id)}`);
registerRoute("#/credentials", pageCredentials);
