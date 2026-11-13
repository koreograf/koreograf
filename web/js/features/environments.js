import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { can, go, handle, markSaved, needTeam, route, shell } from "../core/app.js";
import { addLink, esc, h, paramRow, table } from "../core/dom.js";
import { clearEditHash, editQuery, openEditDialog } from "../core/edit.js";
import { bytes, environmentToolLabel, scopeLabel } from "../core/format.js";
import { t } from "../core/i18n.js";
import { registerRedirect, registerRoute } from "../core/router.js";

export function environmentMatches(item, needle) {
  return !needle || `${item.name} ${item.tool || ""} ${item.image} ${item.pull_policy} ${item.scope || ""} ${scopeLabel(item)}`.toLowerCase().includes(needle);
}

export function environmentImagePlaceholder(tool) {
  if (tool === "ansible") return "quay.io/ansible/awx-ee:latest";
  if (tool === "opentofu") return "ghcr.io/opentofu/opentofu:latest";
  if (tool === "shell") return "docker.io/library/alpine:latest";
  return "registry.example.com/execution-environment:latest";
}

function canManageEnvironment(item) {
  if (!can("owner")) return false;
  if (item && item.scope === "global") return !!appState.me.is_admin;
  return true;
}

function environmentScopeOptions(selected) {
  const choices = [
    ["team", "this_team"],
    ["organization", "this_organization"],
  ];
  if (appState.me.is_admin) choices.push(["global", "global"]);
  return choices.map(([value, label]) =>
    `<option value="${value}"${value === selected ? " selected" : ""}>${h(label)}</option>`
  ).join("");
}

export function environmentPayload(form) {
  return {
    name: form.name.value,
    image: form.image.value,
    tool: form.tool.value,
    pull_policy: form.pull_policy.value,
    network: form.network.value,
    description: form.description.value,
    scope: form.scope.value,
  };
}

async function pageRuntime() {
  if (!needTeam()) return;
  const [cache, plugins, environments, runner] = await Promise.all([
    api("GET", "/api/v1/cache"),
    api("GET", "/api/v1/plugins"),
    api("GET", "/api/v1/environments"),
    api("GET", "/api/v1/runner"),
  ]);
  const collectionRows = (cache.collections || []).map((item) => `<tr><td>${esc(item.name)}</td><td>${esc(item.version)}</td></tr>`);
  const roleRows = (cache.roles || []).map((item) => `<tr><td>${esc(item.name)}</td><td>${esc(item.version)}</td></tr>`);
  const pluginRows = plugins.map((item) => `<tr><td class="tool">${esc(item.id)}</td><td>${esc(t(item.summary))}</td></tr>`);
  shell(t("execution_environments"), `
    <section class="card" style="margin-bottom:14px">
      <h2>${h("execution_environments")}</h2>
      <input class="search" id="environment-search" placeholder="${h("search_environments")}" autocomplete="off">
      <div id="environment-table"></div>
    </section>
    <div class="grid-3" style="margin-bottom:14px">
      <article class="card stat"><span class="small muted">${h("collections")}</span><strong>${esc(bytes(cache.collections_bytes))}</strong></article>
      <article class="card stat"><span class="small muted">${h("roles")}</span><strong>${esc(bytes(cache.roles_bytes))}</strong></article>
      <article class="card stat"><span class="small muted">${h("opentofu")}</span><strong>${esc(bytes(cache.tofu_bytes))}</strong></article>
      <article class="card stat"><span class="small muted">${h("local_jobs")}</span><strong>${esc(runner.local_jobs)}</strong></article>
    </div>
    <p class="small muted" style="margin-top:0">${esc(t(cache.note || ""))}</p>
    <section class="card" style="margin-bottom:14px"><h2>${h("tools")}</h2>${table([t("id"), t("description")], pluginRows)}</section>
    <section class="card" style="margin-bottom:14px"><h2>${h("cached_collections")}</h2>${table([t("name"), t("version")], collectionRows)}</section>
    <section class="card" style="margin-bottom:14px"><h2>${h("cached_roles")}</h2>${table([t("name"), t("version")], roleRows)}</section>`,
    can("owner") ? addLink("#/runtime?new=1", "environment") : "");
  const drawEnvironments = () => {
    const needle = document.querySelector("#environment-search").value.trim().toLowerCase();
    const rows = environments.filter((item) => environmentMatches(item, needle)).map((item) =>
      `<tr><td><a href="#/environments/${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(scopeLabel(item))}</td><td>${esc(environmentToolLabel(item.tool))}</td><td class="mono">${esc(item.image)}</td><td>${esc(item.pull_policy)}</td><td>${canManageEnvironment(item) ? `<a class="btn" href="#/runtime?edit=${esc(item.id)}">${h("edit")}</a> <button class="btn danger" type="button" data-env="${esc(item.id)}">${h("delete")}</button>` : ""}</td></tr>`
    );
    document.querySelector("#environment-table").innerHTML = table([t("name"), t("applies_to"), t("tool"), t("image"), t("pull"), ""], rows);
    document.querySelectorAll("[data-env]").forEach((button) => {
      button.addEventListener("click", async () => {
        try {
          await api("DELETE", `/api/v1/environments/${encodeURIComponent(button.dataset.env)}`);
          route();
        } catch (error) {
          handle(error);
        }
      });
    });
  };
  document.querySelector("#environment-search").addEventListener("input", drawEnvironments);
  drawEnvironments();
  const request = editQuery();
  if (request.create !== null) {
    if (can("owner")) openEnvironmentDialog(null);
    return;
  }
  if (!request.edit) return;
  const item = environments.find((row) => row.id === request.edit);
  if (!item) {
    clearEditHash();
    return;
  }
  if (canManageEnvironment(item)) openEnvironmentDialog(item);
}

function openEnvironmentDialog(item) {
  const editing = Boolean(item);
  const policy = item ? item.pull_policy : "if_not_present";
  const scope = item && item.scope ? item.scope : "team";
  const current = item && item.tool ? item.tool : "";
  const tool = ["ansible", "opentofu", "shell"].includes(current) ? current : "ansible";
  const option = (value) => `<option value="${value}"${policy === value ? " selected" : ""}>${value}</option>`;
  const toolOption = (value) => `<option value="${value}"${tool === value ? " selected" : ""}>${h(value)}</option>`;
  const dialog = openEditDialog({
    id: "environment-dialog",
    title: editing ? t("edit_environment") : t("new_environment"),
    body: `
      <label class="field required"><span>${h("name")}</span><input name="name" required value="${editing ? esc(item.name) : ""}"></label>
      <label class="field"><span>${h("tool")}</span><select class="fit" name="tool">${toolOption("ansible")}${toolOption("opentofu")}${toolOption("shell")}</select></label>
      <label class="field required"><span>${h("image")}</span><input name="image" required value="${editing ? esc(item.image) : ""}" placeholder="${esc(environmentImagePlaceholder(tool))}"></label>
      <label class="field"><span>${h("network")}</span><input name="network" value="${editing ? esc(item.network || "") : ""}" placeholder="koreograf_default"></label>
      <p class="small muted">${h("the_network_is_passed_to_the_container_run_an_empty")}</p>
      <div class="field-row">
        <label class="field"><span>${h("pull")}</span><select class="fit" name="pull_policy">${option("if_not_present")}${option("always")}${option("never")}</select></label>
        <label class="field"><span>${h("applies_to")}</span><select class="fit" name="scope">${environmentScopeOptions(scope)}</select></label>
      </div>
      <p class="small muted">${h("global_applies_to_every_team_the_organization_applies")}</p>
      <label class="field"><span>${h("description")}</span><input name="description" maxlength="1000" value="${editing ? esc(item.description) : ""}"></label>`,
    submit: editing ? t("save") : t("create"),
    onSubmit: async ({ form, close }) => {
      const payload = environmentPayload(form);
      if (editing) await api("PUT", `/api/v1/environments/${encodeURIComponent(item.id)}`, payload);
      else await api("POST", "/api/v1/environments", payload);
      markSaved();
      close();
      await route();
    },
  });
  const refreshImagePlaceholder = () => {
    dialog.form.image.placeholder = environmentImagePlaceholder(dialog.form.tool.value);
  };
  dialog.form.tool.addEventListener("change", refreshImagePlaceholder);
  refreshImagePlaceholder();
}

async function pageEnvironment(id) {
  if (!needTeam()) return;
  const item = await api("GET", `/api/v1/environments/${encodeURIComponent(id)}`);
  const manage = canManageEnvironment(item);
  shell(item.name, `<section class="card"><dl class="kv">
      ${paramRow("name", item.name)}
      ${paramRow("applies_to", scopeLabel(item))}
      ${paramRow("tool", environmentToolLabel(item.tool))}
      ${paramRow("image", item.image)}
      ${paramRow("pull", item.pull_policy)}
      ${paramRow("description", item.description)}
    </dl></section>`, manage
    ? `<a class="btn pill" href="#/runtime?edit=${esc(item.id)}">${h("edit")}</a><button class="btn danger" id="delete-env" type="button">${h("delete")}</button>`
    : "");
  const remove = document.querySelector("#delete-env");
  if (!remove) return;
  remove.addEventListener("click", async () => {
    try {
      await api("DELETE", `/api/v1/environments/${encodeURIComponent(id)}`);
      go("#/runtime");
    } catch (error) {
      handle(error);
    }
  });
}

registerRedirect("#/environments/new", "#/runtime?new=1");
registerRoute("#/environments/:id", (params) => pageEnvironment(params.id));
registerRoute("#/runtime", pageRuntime);
