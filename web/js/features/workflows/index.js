import { api } from "../../core/api.js";
import { can, go, handle, markSaved, needTeam, shell } from "../../core/app.js";
import { addLink, emptyState, esc, h, table } from "../../core/dom.js";
import { editQuery, openEditDialog } from "../../core/edit.js";
import { t } from "../../core/i18n.js";
import { registerRedirect, registerRoute } from "../../core/router.js";
import { confirmTofuApply } from "../../core/tofu.js";
import { mountEditor, workflowTofuApply } from "./editor.js";
import { pageRun } from "./run.js";

export function filterWorkflows(items, needle) {
  const query = (needle || "").trim().toLowerCase();
  return items.filter((item) => !query || `${item.name} ${item.description}`.toLowerCase().includes(query));
}

async function pageWorkflows() {
  if (!needTeam()) return;
  const [items, templates] = await Promise.all([
    api("GET", "/api/v1/workflows"),
    api("GET", "/api/v1/templates"),
  ]);
  shell(t("workflows"), `
    <section class="card">
      <input class="search" id="workflow-search" placeholder="${h("search_workflows")}" autocomplete="off">
      <div id="workflow-table"></div>
    </section>`,
    can("operator") ? addLink("#/workflows?new=1", "workflow_2") : "");
  const drawWorkflows = () => {
    const needle = document.querySelector("#workflow-search").value.trim().toLowerCase();
    const shown = filterWorkflows(items, needle);
    const host = document.querySelector("#workflow-table");
    if (!shown.length) {
      host.innerHTML = items.length
        ? `<p class="small muted">${h("nothing_here")}</p>`
        : emptyState("empty_workflows", can("operator") ? "#/workflows?new=1" : "", "new_workflow");
      return;
    }
    const filtered = shown.map((item) => `
      <tr>
        <td><a href="#/workflows/${esc(item.id)}">${esc(item.name)}</a><div class="small muted">${esc(item.description)}</div></td>
        <td>${h("count_nodes", { count: (item.graph.nodes || []).length })}</td>
        <td>${can("operator") ? `<button class="btn primary" type="button" data-launch="${esc(item.id)}">${h("launch")}</button>` : ""}</td>
      </tr>`);
    host.innerHTML = table([t("name"), t("nodes"), ""], filtered);
    document.querySelectorAll("[data-launch]").forEach((button) => {
      button.addEventListener("click", async () => {
        const item = items.find((row) => row.id === button.dataset.launch);
        if (item && workflowTofuApply(item.graph, templates) && !(await confirmTofuApply(item))) return;
        button.disabled = true;
        try {
          const run = await api("POST", `/api/v1/workflows/${encodeURIComponent(button.dataset.launch)}/launch`, { extra_vars: {} });
          go(`#/runs/${run.id}`);
        } catch (error) {
          button.disabled = false;
          handle(error);
        }
      });
    });
  };
  document.querySelector("#workflow-search").addEventListener("input", drawWorkflows);
  drawWorkflows();
  const request = editQuery();
  if (request.create !== null) {
    if (can("operator")) openWorkflowDialog();
    return;
  }
}

function openWorkflowDialog() {
  openEditDialog({
    id: "workflow-dialog",
    title: t("new_workflow"),
    body: `
      <label class="field required"><span>${h("name")}</span><input name="name" required></label>
      <label class="field"><span>${h("description")}</span><input name="description" maxlength="1000"></label>`,
    submit: t("create_and_open"),
    onSubmit: async ({ form, close }) => {
      const created = await api("POST", "/api/v1/workflows", {
        name: form.name.value,
        description: form.description.value,
        graph: { nodes: [], edges: [] },
      });
      markSaved();
      close();
      go(`#/workflows/${created.id}`);
    },
  });
}

async function pageWorkflow(id) {
  if (!needTeam()) return;
  const [workflow, templates, inventories, credentials] = await Promise.all([
    api("GET", `/api/v1/workflows/${encodeURIComponent(id)}`),
    api("GET", "/api/v1/templates"),
    api("GET", "/api/v1/inventories"),
    api("GET", "/api/v1/credentials"),
  ]);
  mountEditor(workflow, templates, false, {}, { inventories, credentials });
}

registerRedirect("#/workflows/new", "#/workflows?new=1");
registerRoute("#/workflows/:id", (params) => pageWorkflow(params.id));
registerRoute("#/workflows", pageWorkflows);
registerRoute("#/runs/:id", (params) => pageRun(params.id));
