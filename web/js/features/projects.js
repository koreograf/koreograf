import * as appState from "../core/state.js";
import { api } from "../core/api.js";
import { confirmDialog } from "../core/dialog.js";
import { can, go, handle, markSaved, needTeam, route, shell } from "../core/app.js";
import { addLink, emptyState, esc, h, paramRow, selectOptions, table } from "../core/dom.js";
import { clearEditHash, editQuery, openEditDialog, rememberEditHash } from "../core/edit.js";
import { when } from "../core/format.js";
import { t } from "../core/i18n.js";
import { registerRedirect, registerRoute } from "../core/router.js";
import { openCredentialCreator } from "./credentials.js";

function grantLabel(item) {
  return item.team_ids && item.team_ids.length
    ? t("count_teams", { count: item.team_ids.length })
    : t("all_teams_in_the_organization");
}

export function projectMatches(item, needle) {
  return !needle || `${item.name} ${item.organization_name || ""} ${item.source_type} ${grantLabel(item)}`.toLowerCase().includes(needle);
}

function orgOptions(organizations, selected) {
  return organizations.map((item) =>
    `<option value="${esc(item.id)}"${item.id === selected ? " selected" : ""}>${esc(item.name)}</option>`
  ).join("");
}

function teamBoxes(teams, selected) {
  const chosen = new Set(selected || []);
  if (!teams.length) return `<p class="small muted">${h("this_organization_has_no_team_yet")}</p>`;
  return teams.map((team) =>
    `<label class="row"><input type="checkbox" name="team_ids" value="${esc(team.id)}"${chosen.has(team.id) ? " checked" : ""}> ${esc(team.name)}</label>`
  ).join("");
}

function projectCredentialField(credentials, selectedId) {
  const rows = credentials.filter((row) => row.kind === "scm");
  return `<label class="field"><span>${h("source_control_credential")}</span>
    <div class="row">
      <select class="fit" name="credential_id">${selectOptions(rows, selectedId || "", true, (row) => row.name)}</select>
      ${can("owner") ? `<button class="btn pill" type="button" data-new-scm-credential>${h("credential")}</button>` : ""}
    </div>
  </label>
  <p class="small muted">${h("source_control_project_hint")}</p>
  <p class="small muted" id="scm-key-hint" hidden>${h("source_control_key_hint")}</p>`;
}

function bindProjectCredential(form, credentials) {
  const select = form.elements.credential_id;
  const source = form.elements.source;
  const keyHint = form.querySelector("#scm-key-hint");
  const refreshHint = () => {
    const value = source.value.trim();
    if (keyHint) keyHint.hidden = !(value.startsWith("ssh://") || value.startsWith("git@"));
  };
  source.addEventListener("input", refreshHint);
  refreshHint();
  const add = form.querySelector("[data-new-scm-credential]");
  if (add && select) {
    add.addEventListener("click", () => {
      openCredentialCreator("scm", (created) => {
        credentials.push(created);
        const option = document.createElement("option");
        option.value = created.id;
        option.textContent = created.name;
        select.appendChild(option);
        select.value = created.id;
      });
    });
  }
}

async function pageProjects() {
  if (!needTeam()) return;
  const projects = await api("GET", "/api/v1/projects");
  shell(t("projects"), `
    <section class="card">
      <input class="search" id="project-search" placeholder="${h("search_projects")}" autocomplete="off">
      <div id="project-table"></div>
    </section>`,
    can("operator") ? addLink("#/projects?new=1", "project_2") : "");
  const drawProjects = () => {
    const needle = document.querySelector("#project-search").value.trim().toLowerCase();
    const shown = projects.filter((item) => projectMatches(item, needle));
    const host = document.querySelector("#project-table");
    if (!shown.length) {
      host.innerHTML = projects.length
        ? `<p class="small muted">${h("nothing_here")}</p>`
        : emptyState("empty_projects", can("operator") ? "#/projects?new=1" : "", "new_project");
      return;
    }
    const rows = shown.map((item) =>
      `<tr><td><a href="#/projects/${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(item.organization_name || "")}</td><td>${esc(item.source_type)}</td><td class="small muted">${esc(grantLabel(item))}</td><td>${can("operator") ? `<a class="btn" href="#/projects?edit=${esc(item.id)}">${h("edit")}</a>` : ""}</td></tr>`
    );
    host.innerHTML = table([t("name"), t("organization"), t("type"), t("visibility"), ""], rows);
  };
  document.querySelector("#project-search").addEventListener("input", drawProjects);
  drawProjects();
  const request = editQuery();
  if (request.create !== null) {
    if (can("operator")) await openProjectDialog(null);
    return;
  }
  if (!request.edit) return;
  const item = projects.find((row) => row.id === request.edit);
  if (!item) {
    clearEditHash();
    return;
  }
  if (can("operator")) await openProjectDialog(item, request.tab);
}

export function projectPayload(form, editing) {
  const payload = {
    name: form.name.value,
    source_type: form.source_type.value,
    source: form.source.value,
    revision: form.revision.value,
    credential_id: form.credential_id.value,
  };
  if (form.organization_id) payload.organization_id = form.organization_id.value;
  if (editing) {
    payload.team_ids = [...form.querySelectorAll("input[name=team_ids]:checked")].map((input) => input.value).sort();
  } else {
    payload.team_ids = [];
  }
  return payload;
}

async function openProjectDialog(item, initialTab = "") {
  const editing = Boolean(item);
  const [organizations, credentials] = await Promise.all([
    editing || appState.me.is_admin ? api("GET", "/api/v1/organizations") : Promise.resolve([]),
    api("GET", "/api/v1/credentials"),
  ]);
  const current = (appState.me.teams || []).find((team) => team.id === appState.teamId);
  const organization = editing ? await api("GET", `/api/v1/organizations/${encodeURIComponent(item.organization_id)}`) : null;
  const organizationField = editing || appState.me.is_admin
    ? `<label class="field"><span>${h("organization")}</span><select name="organization_id"${appState.me.is_admin ? "" : " disabled"}>${orgOptions(organizations, editing ? item.organization_id : (current && current.organization_id))}</select></label>`
    : "";
  const general = `<label class="field required"><span>${h("name")}</span><input name="name" required value="${editing ? esc(item.name) : ""}"></label>`;
  const source = `
    <div class="field-row">
      <label class="field"><span>${h("type")}</span><select class="fit" name="source_type">
        <option value="path"${editing && item.source_type === "git" ? "" : " selected"}>${h("path")}</option>
        <option value="git"${editing && item.source_type === "git" ? " selected" : ""}>Git</option>
      </select></label>
      <label class="field"><span>${h("revision")}</span><input class="fit" name="revision" value="${editing ? esc(item.revision) : ""}" placeholder="${h("branch_tag_or_commit")}"></label>
    </div>
    <label class="field required"><span>${h("source_2")}</span><input name="source" required value="${editing ? esc(item.source) : ""}" placeholder="${h("https_or_file")}"></label>
    ${projectCredentialField(credentials, editing ? item.credential_id : "")}
    ${editing && item.source_type === "git" ? `<p class="small muted" id="last-sync">${esc(projectSyncText(item))}</p>` : ""}`;
  const assignment = `
    <p class="small muted">${h("with_no_team_checked_every_team_in_the")}</p>
    ${organizationField}
    <div class="stack" id="team-boxes">${teamBoxes((organization && organization.teams) || [], editing ? item.team_ids : [])}</div>`;
  const dialog = openEditDialog({
    id: "project-dialog",
    title: editing ? t("edit_project") : t("new_project"),
    tabs: editing ? [
      { id: "general", label: t("general"), body: general },
      { id: "source", label: t("source_2"), body: source },
      { id: "assignment", label: t("assignment"), body: assignment },
    ] : null,
    body: `${general}${source}${organizationField}`,
    submit: editing ? t("save") : t("create"),
    initialTab,
    onSubmit: async ({ form, close }) => {
      const payload = projectPayload(form, editing);
      if (editing) {
        const saved = await api("PUT", `/api/v1/projects/${encodeURIComponent(item.id)}`, payload);
        markSaved();
        close();
        await route();
        if (saved.sync && !saved.sync.ok) {
          handle({ message: saved.sync.error || "sync_failed", vars: saved.sync.vars });
        }
      } else {
        const created = await api("POST", "/api/v1/projects", payload);
        markSaved();
        close();
        go(`#/projects/${created.id}`);
      }
    },
    onTab: editing ? (tab) => rememberEditHash({ edit: item.id, tab }, false) : undefined,
  });
  bindProjectCredential(dialog.form, credentials);
  if (editing && appState.me.is_admin) {
    dialog.form.organization_id.addEventListener("change", async () => {
      const box = dialog.overlay.querySelector("#team-boxes");
      box.innerHTML = `<p class="small muted">${h("loading_teams")}</p>`;
      try {
        const next = await api("GET", `/api/v1/organizations/${encodeURIComponent(dialog.form.organization_id.value)}`);
        box.innerHTML = teamBoxes(next.teams || [], []);
      } catch (error) { handle(error); }
    });
  }
}

function projectSyncText(project) {
  const status = { success: t("success"), failed: t("failed") }[project.last_sync_status] || t("never");
  return t("last_synced_value", {
    time: project.last_sync_at ? when(project.last_sync_at) : t("never"),
    commit: project.last_sync_commit || "—",
    status,
  });
}

async function pageProject(id) {
  if (!needTeam()) return;
  const project = await api("GET", `/api/v1/projects/${encodeURIComponent(id)}`);
  shell(project.name, `
    <section class="card">
      <dl class="kv">
        ${paramRow("name", project.name)}
        ${paramRow("source_2", `${project.source_type} · ${project.source}`)}
        ${paramRow("revision", project.revision)}
        ${paramRow("organization", project.organization_name)}
        ${paramRow("visibility", grantLabel(project))}
      </dl>
      ${project.source_type === "git" ? `<p class="small muted" id="last-sync">${esc(projectSyncText(project))}</p>` : ""}
      ${can("operator") && project.source_type === "git" ? `<div class="row"><button class="btn" id="sync" type="button">${h("sync")}</button></div>` : ""}
    </section>
    ${can("operator") ? `<section class="card danger-zone" style="margin-top:14px">
      <h2>${h("danger_zone")}</h2>
      <p class="small muted">${h("deleting_cannot_be_undone")}</p>
      <button class="btn danger" id="delete-project" type="button">${h("delete")}</button>
    </section>` : ""}`,
    can("operator") ? `<a class="btn pill" href="#/projects?edit=${esc(project.id)}">${h("edit")}</a>` : "");
  const sync = document.querySelector("#sync");
  if (sync) {
    sync.addEventListener("click", async () => {
      sync.disabled = true;
      try {
        await api("POST", `/api/v1/projects/${encodeURIComponent(id)}/sync`, {});
        const fresh = await api("GET", `/api/v1/projects/${encodeURIComponent(id)}`);
        const note = document.querySelector("#last-sync");
        if (note) note.textContent = projectSyncText(fresh);
        sync.textContent = t("synced");
      } catch (error) {
        const fresh = await api("GET", `/api/v1/projects/${encodeURIComponent(id)}`).catch(() => null);
        const note = document.querySelector("#last-sync");
        if (note && fresh) note.textContent = projectSyncText(fresh);
        sync.disabled = false;
        handle(error);
      }
    });
  }
  const remove = document.querySelector("#delete-project");
  if (remove) {
    remove.addEventListener("click", async () => {
      if (!(await confirmDialog({ title: t("delete"), body: t("delete_this_project", { name: project.name }), confirmLabel: t("delete"), danger: true }))) return;
      try {
        await api("DELETE", `/api/v1/projects/${encodeURIComponent(id)}`);
        appState.setLeaveGuard(null);
        go("#/projects");
      } catch (error) { handle(error); }
    });
  }
}

registerRedirect("#/projects/new", "#/projects?new=1");
registerRoute("#/projects/:id", (params) => pageProject(params.id));
registerRoute("#/projects", pageProjects);
