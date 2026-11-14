import * as appState from "../../core/state.js";
import { api } from "../../core/api.js";
import { confirmDialog, openDialog } from "../../core/dialog.js";
import { can, go, handle, shell } from "../../core/app.js";
import { credentialKindLabel } from "../../core/credentials.js";
import { esc, h, selectOptions } from "../../core/dom.js";
import { statusLabel, verbosityLabel } from "../../core/format.js";
import { jsonObject } from "../../core/forms.js";
import { t } from "../../core/i18n.js";
import { confirmTofuApply } from "../../core/tofu.js";
import { mountSchedules } from "../schedules.js";
import { chooseEdgeType, openCanvasMenu, openNodeMenu } from "./menus.js";
import { autoPan, createViewport, minimapLayout } from "./viewport.js";

export function workflowTofuApply(graph, templates) {
  return ((graph && graph.nodes) || []).some((node) => {
    const template = templates.find((item) => item.id === node.template_id);
    if (!template || template.tool !== "opentofu") return false;
    const override = node.overrides && node.overrides.spec;
    const action = override && override.action !== undefined ? override.action : (template.spec || {}).action;
    return action === "apply";
  });
}

export function edgePoint(node, when) {
  const y = when === "failure" ? 54 : when === "always" ? 39 : 24;
  return { x: node.x + 200, y: node.y + y };
}

export function workflowTab(hash = location.hash || "") {
  const params = new URLSearchParams((hash || "").split("?")[1] || "");
  const tab = params.get("tab");
  return tab === "schedules" || tab === "danger" ? tab : "editor";
}

export function workflowAddress(tab, id) {
  const params = new URLSearchParams();
  if (tab !== "editor") params.set("tab", tab);
  const query = params.toString();
  return `#/workflows/${encodeURIComponent(id)}${query ? `?${query}` : ""}`;
}

export function mountEditor(workflow, templates, readonly, phases, catalogs, hooks) {
  const onNode = hooks && hooks.onNode;
  const lists = catalogs || { inventories: [], credentials: [] };
  const editable = !readonly && can("operator");
  const graph = {
    nodes: (workflow.graph.nodes || []).map((node) => ({ ...node })),
    edges: (workflow.graph.edges || []).map((edge) => ({ ...edge })),
  };
  let selected = null;
  let selectedEdge = null;
  let connecting = null;
  let connectDrag = null;
  let drag = null;
  let flushNode = () => true;
  let dirty = false;
  const leaveGuard = () => dirty;
  const back = readonly && workflow.workflow_id
    ? `<a class="btn" href="#/workflows/${esc(workflow.workflow_id)}">${h("back_to_the_workflow")}</a>`
    : "";
  const tabbed = !readonly;
  const canDelete = can("operator");
  const tabItems = tabbed
    ? [["editor", "editor"], ["schedules", "schedules"], ...(canDelete ? [["danger", "danger_zone"]] : [])]
    : [];
  let activeTab = tabbed ? workflowTab() : "editor";
  if (activeTab === "danger" && !canDelete) activeTab = "editor";
  const tabBar = tabbed
    ? `<div class="spread tab-bar"><div class="tabs" role="tablist" aria-label="${h("workflow")}">${tabItems.map(([id, label]) => `<button class="btn pill${id === activeTab ? " primary" : ""}" type="button" role="tab" id="wf-tab-${id}" aria-selected="${id === activeTab}" aria-controls="wf-panel-${id}" tabindex="${id === activeTab ? "0" : "-1"}" data-workflow-tab="${id}">${h(label)}</button>`).join("")}</div></div>`
    : "";
  shell(readonly ? workflow.name : t("workflow"), `
    <div class="editor-stack">
    ${tabBar}
    <div id="wf-panel-editor"${tabbed ? ` role="tabpanel" aria-labelledby="wf-tab-editor"${activeTab === "editor" ? "" : " hidden"}` : ""}>
      <div class="editor-body">
        <div class="canvas-column">
          ${readonly ? "" : `
            <div class="field-row" style="margin-bottom:10px">
              <label class="field"><span>${h("name")}</span><input class="fit" id="wf-name" value="${esc(workflow.name)}" ${can("operator") ? "" : "disabled"}></label>
              <label class="field"><span>${h("description")}</span><input class="fit" id="wf-description" maxlength="1000" value="${esc(workflow.description)}" ${can("operator") ? "" : "disabled"}></label>
            </div>`}
          <p class="small muted" id="wf-hint" role="status" aria-live="polite">${readonly ? h("colors_show_the_run_a_click_shows_the") : h("drag_nodes_and_use_the_canvas_menus")}</p>
          <p class="small muted" id="wf-keyboard-help">${h(readonly ? "run_keyboard_help" : "workflow_keyboard_help")}</p>
          <div class="canvas" id="canvas" tabindex="0" role="application" aria-label="${h("workflow_graph")}" aria-describedby="wf-keyboard-help">
            <div class="canvas-controls" role="group" aria-label="${h("canvas_controls")}">
              <button class="btn compact ghost" type="button" data-canvas-zoom-in aria-label="${h("zoom_in")}">+</button>
              <button class="btn compact ghost" type="button" data-canvas-zoom-out aria-label="${h("zoom_out")}">−</button>
              <button class="btn compact ghost" type="button" data-canvas-fit>${h("fit_view")}</button>
              <button class="btn compact ghost" type="button" data-canvas-reset>${h("zoom_reset")}</button>
            </div>
            <div class="minimap" id="minimap" role="img" aria-label="${h("minimap")}" hidden></div>
            <div id="board" style="position:absolute;left:0;top:0;transform-origin:0 0"><svg class="edges" id="edges"></svg><div id="nodes"></div></div>
          </div>
        </div>
        <aside class="card inspector" id="inspector"></aside>
      </div>
      ${readonly ? `<section class="card run-log" id="run-log" aria-live="polite"></section>` : ""}
    </div>
    ${readonly ? "" : `<div id="wf-panel-schedules" role="tabpanel" aria-labelledby="wf-tab-schedules"${activeTab === "schedules" ? "" : " hidden"}>
      <section class="card" id="schedule-card" data-kind="workflow" data-target="${esc(workflow.id)}"></section>
    </div>`}
    ${!tabbed || !canDelete ? "" : `<div id="wf-panel-danger" role="tabpanel" aria-labelledby="wf-tab-danger"${activeTab === "danger" ? "" : " hidden"}>
      <section class="card danger-zone">
        <h2>${h("danger_zone")}</h2>
        <p class="small muted">${h("deleting_cannot_be_undone")}</p>
        <button class="btn danger" id="wf-delete" type="button">${h("delete")}</button>
      </section>
    </div>`}
    </div>`, readonly ? back : `
      ${can("operator") ? `<span class="small muted" id="wf-status" role="status" aria-live="polite">${h("all_changes_saved")}</span><button class="btn" id="wf-save" type="button">${h("save")}</button><button class="btn primary" id="wf-launch" type="button">${h("launch")}</button>` : ""}`);

  document.title = readonly
    ? `${t("run_2")} ${workflow.name} – Koreograf`
    : `${workflow.name} – ${t("workflow")} – Koreograf`;

  const canvas = document.querySelector("#canvas");
  const board = document.querySelector("#board");
  const svg = document.querySelector("#edges");
  const nodes = document.querySelector("#nodes");
  const minimap = document.querySelector("#minimap");
  let fitted = false;
  let pan = null;
  let autoPanTimer = null;

  const viewport = createViewport({
    canvas,
    board,
    onChange: (view, size) => drawMinimap(view, size),
  });

  function canvasPoint(event) {
    const rect = canvas.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  function localPoint(event) {
    return viewport.screenToGraph(canvasPoint(event));
  }

  function centerPoint() {
    const size = viewport.size();
    return { x: size.width / 2, y: size.height / 2 };
  }

  function zoomBy(factor) {
    viewport.zoom(factor, centerPoint());
  }

  function fitGraph() {
    viewport.fit(graph.nodes);
    fitted = true;
  }

  function fitOnOpen() {
    if (canvas.offsetParent === null) return;
    const size = viewport.size();
    if (size.width < 240 || size.height < 240) return;
    fitGraph();
  }

  function paintMinimap() {
    drawMinimap(viewport.view, viewport.size());
  }

  function drawMinimap(view, size) {
    if (!minimap) return;
    const visible = graph.nodes.length > 12;
    minimap.hidden = !visible;
    if (!visible) {
      minimap.innerHTML = "";
      return;
    }
    const layout = minimapLayout(graph.nodes, view, size);
    const rects = layout.nodes.map((rect) => `<rect class="minimap-node" x="${rect.x}" y="${rect.y}" width="${rect.width}" height="${rect.height}"></rect>`).join("");
    const frame = `<rect class="minimap-view" x="${layout.viewport.x}" y="${layout.viewport.y}" width="${layout.viewport.width}" height="${layout.viewport.height}"></rect>`;
    minimap.innerHTML = `<svg viewBox="0 0 ${layout.width} ${layout.height}" aria-hidden="true">${rects}${frame}</svg>`;
  }

  function stopAutoPan() {
    if (autoPanTimer) window.clearInterval(autoPanTimer);
    autoPanTimer = null;
  }

  function autoPanStep() {
    if (!drag || !drag.screen) {
      stopAutoPan();
      return;
    }
    const result = autoPan(viewport.view, drag.screen, viewport.size());
    if (!result.dx && !result.dy) {
      stopAutoPan();
      return;
    }
    const node = graph.nodes.find((item) => item.id === drag.id);
    if (node) {
      const scale = viewport.view.scale;
      node.x = Math.max(0, node.x - result.dx / scale);
      node.y = Math.max(0, node.y - result.dy / scale);
      const element = nodes.querySelector(`[data-id="${CSS.escape(node.id)}"]`);
      if (element) {
        element.style.left = `${node.x}px`;
        element.style.top = `${node.y}px`;
      }
    }
    viewport.set(result.view);
    drawEdges();
    markDirty();
  }

  function scheduleAutoPan() {
    if (!drag || !drag.screen) return;
    const result = autoPan(viewport.view, drag.screen, viewport.size());
    if (!result.dx && !result.dy) {
      stopAutoPan();
      return;
    }
    autoPanStep();
    if (!autoPanTimer) autoPanTimer = window.setInterval(autoPanStep, 60);
  }

  function sameJson(left, right) {
    const norm = (value) => {
      if (Array.isArray(value)) return value.map(norm);
      if (value && typeof value === "object") {
        return Object.keys(value).sort().reduce((acc, key) => {
          acc[key] = norm(value[key]);
          return acc;
        }, {});
      }
      return value;
    };
    return JSON.stringify(norm(left)) === JSON.stringify(norm(right));
  }

  function nodeCaption(node) {
    const count = new Set(graph.edges.filter((edge) => edge.to === node.id).map((edge) => edge.from)).size;
    let join = "";
    if (count >= 2) {
      join = node.convergence === "all"
        ? t("all_of_n_predecessors", { count })
        : t("any_of_n_predecessors", { count });
    }
    const inventoryId = node.overrides && Object.prototype.hasOwnProperty.call(node.overrides, "inventory_id")
      ? node.overrides.inventory_id
      : undefined;
    if (inventoryId === undefined) return join;
    if (!inventoryId) return [join, t("no_inventory")].filter(Boolean).join(" · ");
    const inventory = lists.inventories.find((item) => item.id === inventoryId);
    return inventory ? [join, inventory.name].filter(Boolean).join(" · ") : join;
  }

  function paintCaption(node) {
    const label = nodes.querySelector(`[data-id="${CSS.escape(node.id)}"] .muted`);
    if (label) label.textContent = nodeCaption(node);
  }

  function nodeParams(node, template) {
    if (!template) return "";
    const over = node.overrides || {};
    const spec = { ...(template.spec || {}), ...(over.spec || {}) };
    const inventoryId = over.inventory_id !== undefined ? over.inventory_id : (template.inventory_id || "");
    const credentialId = over.credential_id !== undefined ? over.credential_id : (template.credential_id || "");
    const extra = over.extra_vars || template.extra_vars || {};
    const verbosity = Number(spec.verbosity) || 0;
    const jobType = spec.job_type === "check" ? "check" : "run";
    let toolFields = "";
    if (template.tool === "ansible") {
      toolFields = `
        <label class="field"><span>${h("inventory")}</span><select class="fit" id="node-inventory">${selectOptions(lists.inventories, inventoryId, true, (row) => row.name)}</select></label>
        <label class="field"><span>${h("type")}</span>
          <select class="fit" id="node-job-type">
            <option value="run"${jobType === "check" ? "" : " selected"}>${h("run_2")}</option>
            <option value="check"${jobType === "check" ? " selected" : ""}>${h("check_dry_run")}</option>
          </select>
        </label>
        <label class="field"><span>${h("verbosity")}</span>
          <select class="fit" id="node-verbosity">
            ${[0, 1, 2, 3, 4].map((level) => `<option value="${level}"${verbosity === level ? " selected" : ""}>${h(verbosityLabel(level))}</option>`).join("")}
          </select>
        </label>
        <label class="field"><span>${h("forks")}</span><input class="tiny" id="node-forks" inputmode="numeric" value="${esc(spec.forks ?? "")}"></label>
        <label class="field check"><span>${h("diff")}</span><input id="node-diff" type="checkbox"${spec.diff ? " checked" : ""}></label>
        <label class="field"><span>${h("tags")}</span><input id="node-tags" value="${esc(spec.tags || "")}"></label>
        <label class="field"><span>${h("skip_tags")}</span><input id="node-skip-tags" value="${esc(spec.skip_tags || "")}"></label>
        <label class="field"><span>${h("limit")}</span><input id="node-limit" value="${esc(spec.limit || "")}"></label>`;
    } else if (template.tool === "opentofu") {
      const action = spec.action === "apply" ? "apply" : "plan";
      toolFields = `
        <label class="field"><span>${h("action")}</span>
          <select class="fit" id="node-action">
            <option value="plan"${action === "apply" ? "" : " selected"}>plan</option>
            <option value="apply"${action === "apply" ? " selected" : ""}>apply</option>
          </select>
        </label>
        <p class="small muted">${h("opentofu_plan_reads_state_apply_keeps_it")}</p>`;
    }
    return `
      <details class="node-params" open>
        <summary>${h("parameters")}</summary>
        <p class="small muted">${h("applies_only_to_this_node_the_template_stays")}</p>
        ${toolFields}
        <label class="field"><span>${h("nav.access")}</span><select class="fit" id="node-credential">${selectOptions(lists.credentials, credentialId, true, (row) => `${row.name} (${credentialKindLabel(row.kind)})`)}</select></label>
        <label class="field"><span>${h("extra_variables_json")}</span><textarea id="node-extra">${esc(JSON.stringify(extra, null, 2))}</textarea></label>
      </details>`;
  }

  function drawEdges() {
    const width = Math.max(...graph.nodes.map((node) => (Number(node.x) || 0) + 260), 640);
    const height = Math.max(...graph.nodes.map((node) => (Number(node.y) || 0) + 140), 400);
    board.style.width = `${width}px`;
    board.style.height = `${height}px`;
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    svg.innerHTML = graph.edges.map((edge, index) => {
      const from = graph.nodes.find((node) => node.id === edge.from);
      const to = graph.nodes.find((node) => node.id === edge.to);
      if (!from || !to) return "";
      const start = edgePoint(from, edge.when);
      const end = { x: to.x, y: to.y + 36 };
      const dx = Math.max(48, Math.abs(end.x - start.x) / 2);
      const klass = ["success", "failure", "always"].includes(edge.when) ? edge.when : "always";
      const label = `${edgeWhenText(klass)}: ${from.name} → ${to.name}`;
      const role = editable ? ` role="button" tabindex="0"` : ` role="img"`;
      return `<path class="${klass}${selectedEdge === index ? " selected" : ""}" data-edge="${index}"${role} aria-label="${esc(label)}" d="M ${start.x} ${start.y} C ${start.x + dx} ${start.y}, ${end.x - dx} ${end.y}, ${end.x} ${end.y}"></path>`;
    }).join("");
    paintMinimap();
  }

  function focusEdge(index) {
    const path = svg.querySelector(`path[data-edge="${index}"]`);
    if (path) path.focus();
  }

  function selectEdge(index, focus) {
    if (!flushNode()) return;
    selectedEdge = index;
    selected = null;
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    drawEdges();
    renderInspector();
    if (focus) focusEdge(index);
  }

  function removeEdgeAt(index) {
    const edge = graph.edges[index];
    if (!edge) return false;
    graph.edges.splice(index, 1);
    if (selectedEdge === index) selectedEdge = null;
    else if (selectedEdge !== null && selectedEdge > index) selectedEdge -= 1;
    markDirty();
    setHint("edge_removed");
    drawEdges();
    renderNodes();
    renderInspector();
    return true;
  }

  function edgeExists(fromId, toId, when) {
    return graph.edges.some((edge) => edge.from === fromId && edge.to === toId && edge.when === when);
  }

  function setHint(key, vars) {
    const hint = document.querySelector("#wf-hint");
    if (hint) hint.textContent = t(key, vars);
  }

  function paintDirty() {
    const saveButton = document.querySelector("#wf-save");
    const launchButton = document.querySelector("#wf-launch");
    const status = document.querySelector("#wf-status");
    if (saveButton) saveButton.classList.toggle("primary", dirty);
    if (launchButton) launchButton.classList.toggle("primary", !dirty);
    if (status) status.textContent = t(dirty ? "unsaved_changes" : "all_changes_saved");
  }

  function setDirty(next) {
    dirty = Boolean(next);
    paintDirty();
    appState.setLeaveGuard(dirty ? leaveGuard : null);
  }

  function markDirty() {
    if (!editable) return;
    setDirty(true);
  }

  function clearConnectPreview() {
    const preview = svg.querySelector("#connect-preview");
    if (preview) preview.remove();
  }

  function drawConnectPreview(from, point) {
    let preview = svg.querySelector("#connect-preview");
    if (!preview) {
      preview = document.createElementNS("http://www.w3.org/2000/svg", "path");
      preview.id = "connect-preview";
      preview.setAttribute("class", "connecting");
      svg.appendChild(preview);
    }
    const start = edgePoint(from, "always");
    const dx = Math.max(48, Math.abs(point.x - start.x) / 2);
    preview.setAttribute("d", `M ${start.x} ${start.y} C ${start.x + dx} ${start.y}, ${point.x - dx} ${point.y}, ${point.x} ${point.y}`);
  }

  function openAddNode(point) {
    if (!editable) return;
    if (!templates.length) {
      setHint("create_a_template_or_a_workflow_first");
      return;
    }
    const template = templates[0];
    openNodeEditor({
      id: `n${Math.random().toString(16).slice(2, 10)}`,
      template_id: template.id,
      name: template.name,
      convergence: "any",
    }, point);
  }

  function addNode(draft, point) {
    const spot = point
      ? { x: Math.max(0, Math.round(point.x - 20)), y: Math.max(0, Math.round(point.y - 20)) }
      : freeNodePosition();
    const node = { ...draft, x: spot.x, y: spot.y };
    graph.nodes.push(node);
    markDirty();
    selected = node.id;
    selectedEdge = null;
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    renderNodes();
    renderInspector();
    const created = nodes.querySelector(`[data-id="${CSS.escape(node.id)}"]`);
    if (created) created.focus();
  }

  async function removeNode(node) {
    if (!(await confirmDialog({ title: t("delete_node"), body: t("delete_this_node", { name: node.name }), confirmLabel: t("delete"), danger: true }))) return;
    flushNode = () => true;
    graph.nodes = graph.nodes.filter((item) => item.id !== node.id);
    graph.edges = graph.edges.filter((edge) => edge.from !== node.id && edge.to !== node.id);
    markDirty();
    selected = null;
    selectedEdge = null;
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    renderNodes();
    renderInspector();
  }

  function nodeEditorFields(node) {
    const template = templates.find((item) => item.id === node.template_id);
    return `<label class="field required"><span>${h("name")}</span><input id="node-name" value="${esc(node.name)}" required></label>
      <label class="field"><span>${h("template")}</span><select class="fit" id="node-template">${selectOptions(templates, node.template_id, false, (row) => row.name)}</select></label>
      <label class="field"><span>${h("convergence")}</span>
        <select class="fit" id="node-join">
          <option value="any"${node.convergence !== "all" ? " selected" : ""}>${h("any_predecessor")}</option>
          <option value="all"${node.convergence === "all" ? " selected" : ""}>${h("all_predecessors")}</option>
        </select>
      </label>
      ${nodeParams(node, template)}`;
  }

  function openNodeEditor(node, point) {
    if (!editable || !flushNode()) return;
    const adding = Boolean(point);
    const draftFor = (templateId, name, convergence) => {
      const draft = { ...node, name, convergence, template_id: templateId };
      if (!node.overrides) return draft;
      draft.overrides = JSON.parse(JSON.stringify(node.overrides));
      if (templateId !== node.template_id) {
        delete draft.overrides.spec;
        if (draft.overrides.inventory_id === undefined && draft.overrides.credential_id === undefined && draft.overrides.extra_vars === undefined) {
          delete draft.overrides;
        }
      }
      return draft;
    };
    const dialog = openDialog({
      id: adding ? "add-node-dialog" : "node-edit-dialog",
      title: adding ? t("add_node") : t("edit_node"),
      submit: adding ? t("create") : t("save"),
      body: nodeEditorFields(node),
      onSubmit: ({ form, close }) => {
        const name = form.querySelector("#node-name").value.trim();
        if (!name) throw new Error("name_is_missing_or_too_long");
        const templateId = form.querySelector("#node-template").value;
        if (!templates.some((item) => item.id === templateId)) throw new Error("create_a_template_or_a_workflow_first");
        const convergence = form.querySelector("#node-join").value === "all" ? "all" : "any";
        const draft = draftFor(templateId, name, convergence);
        if (!readOverrides(draft, form)) return;
        if (adding) {
          addNode(draft, point);
        } else {
          Object.assign(node, draft);
          flushNode = () => true;
          markDirty();
          renderNodes();
          renderInspector();
        }
        close();
      },
    });
    const bindTemplate = () => {
      const select = dialog.form.querySelector("#node-template");
      if (!select) return;
      select.addEventListener("change", (event) => {
        const next = templates.find((item) => item.id === event.target.value);
        if (!next) return;
        const name = dialog.form.querySelector("#node-name").value;
        const convergence = dialog.form.querySelector("#node-join").value;
        const values = {};
        dialog.form.querySelectorAll("input[id], select[id], textarea[id]").forEach((control) => {
          values[control.id] = control.type === "checkbox" ? control.checked : control.value;
        });
        const draft = draftFor(next.id, name, convergence);
        dialog.form.querySelector(".dialog-body").innerHTML = nodeEditorFields(draft);
        Object.entries(values).forEach(([id, value]) => {
          if (id === "node-template") return;
          const control = dialog.form.querySelector(`#${CSS.escape(id)}`);
          if (!control) return;
          if (control.type === "checkbox") control.checked = Boolean(value);
          else if (control.tagName === "SELECT" && [...control.options].some((option) => option.value === value)) control.value = value;
          else if (control.tagName !== "SELECT") control.value = value;
        });
        bindTemplate();
      });
    };
    bindTemplate();
  }

  function startConnect(node) {
    if (!editable || !flushNode()) return;
    connecting = { from: node.id };
    connectDrag = null;
    selected = node.id;
    selectedEdge = null;
    setHint("connect_click_or_drag_target", { name: node.name });
    renderNodes();
    renderInspector();
    const source = nodes.querySelector(`[data-id="${CSS.escape(node.id)}"]`);
    if (source) source.focus();
  }

  function cancelConnect() {
    if (!connecting) return;
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    setHint("drag_nodes_and_use_the_canvas_menus");
    renderNodes();
    renderInspector();
  }

  async function finishConnect(toId) {
    if (!connecting) return;
    const fromNode = graph.nodes.find((node) => node.id === connecting.from);
    const toNode = graph.nodes.find((node) => node.id === toId);
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    if (!fromNode || !toNode) return;
    if (fromNode.id === toNode.id) {
      setHint("an_edge_cannot_point_at_the_same_node");
      renderNodes();
      renderInspector();
      return;
    }
    const when = await chooseEdgeType(fromNode, toNode);
    if (!when) {
      setHint("connect_cancelled");
      renderNodes();
      renderInspector();
      return;
    }
    if (edgeExists(fromNode.id, toNode.id, when)) {
      setHint("the_same_edge_is_present_twice");
      renderNodes();
      renderInspector();
      return;
    }
    graph.edges.push({ from: fromNode.id, to: toNode.id, when });
    markDirty();
    selected = fromNode.id;
    selectedEdge = null;
    setHint("edge_added", { when: edgeWhenText(when), from: fromNode.name, to: toNode.name });
    renderNodes();
    renderInspector();
  }

  const nodeMenuActions = {
    edit: openNodeEditor,
    remove: removeNode,
    connect: startConnect,
  };

  function nodeLabel(node) {
    const template = templates.find((item) => item.id === node.template_id);
    const parts = [node.name];
    if (template) parts.push(template.name);
    if (readonly) parts.push(statusLabel((phases[node.id] && phases[node.id].phase) || "pending"));
    return parts.filter(Boolean).join(" · ");
  }

  function renderNodes() {
    nodes.innerHTML = graph.nodes.map((node) => {
      const phase = phases[node.id];
      const phaseClass = phase ? ` phase-${esc(phase.phase)}` : "";
      const selectedClass = selected === node.id ? " selected" : "";
      const runClass = readonly ? " run-node" : "";
      const connectClass = connecting && connecting.from === node.id ? " connecting" : "";
      const menu = editable ? `<button class="btn ghost compact node-menu-button" type="button" data-node-menu aria-haspopup="menu" aria-expanded="false" aria-label="${h("node_actions", { name: node.name })}">⋯</button>` : "";
      return `<div class="wf-node${selectedClass}${connectClass}${phaseClass}${runClass}" data-id="${esc(node.id)}" style="left:${Number(node.x) || 0}px;top:${Number(node.y) || 0}px" tabindex="0" role="group" aria-label="${esc(nodeLabel(node))}">${menu}<strong>${esc(node.name)}</strong><span class="small muted">${esc(nodeCaption(node))}</span></div>`;
    }).join("");
    nodes.querySelectorAll(".wf-node").forEach((element) => {
      const node = graph.nodes.find((item) => item.id === element.dataset.id);
      const menuButton = element.querySelector("[data-node-menu]");
      element.addEventListener("pointerdown", (event) => {
        if (event.target.closest(".node-menu-button")) return;
        if (readonly) {
          if (onNode) onNode(node, phases[node.id] || null);
          return;
        }
        if (!editable) {
          selected = node.id;
          selectedEdge = null;
          renderInspector();
          nodes.querySelectorAll(".wf-node").forEach((item) => item.classList.toggle("selected", item.dataset.id === selected));
          return;
        }
        if (connecting) {
          if (node.id === connecting.from) {
            event.preventDefault();
            if (!flushNode()) return;
            connectDrag = { id: node.id, pointerId: event.pointerId };
            element.setPointerCapture(event.pointerId);
            drawConnectPreview(node, localPoint(event));
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          finishConnect(node.id);
          return;
        }
        if (!flushNode()) return;
        selected = node.id;
        selectedEdge = null;
        const point = localPoint(event);
        drag = { id: node.id, ox: point.x - node.x, oy: point.y - node.y, screen: canvasPoint(event) };
        element.setPointerCapture(event.pointerId);
        renderInspector();
        nodes.querySelectorAll(".wf-node").forEach((item) => item.classList.toggle("selected", item.dataset.id === selected));
      });
      element.addEventListener("pointermove", (event) => {
        if (connectDrag && connectDrag.id === node.id) {
          drawConnectPreview(node, localPoint(event));
          return;
        }
        if (!drag || drag.id !== node.id) return;
        const point = localPoint(event);
        node.x = Math.max(0, Math.round(point.x - drag.ox));
        node.y = Math.max(0, Math.round(point.y - drag.oy));
        element.style.left = `${node.x}px`;
        element.style.top = `${node.y}px`;
        drag.screen = canvasPoint(event);
        drawEdges();
        markDirty();
        scheduleAutoPan();
      });
      element.addEventListener("pointerup", (event) => {
        if (connectDrag && connectDrag.id === node.id) {
          connectDrag = null;
          clearConnectPreview();
          const target = document.elementFromPoint(event.clientX, event.clientY);
          const hit = target && target.closest(".wf-node");
          if (hit && hit.dataset.id !== node.id) finishConnect(hit.dataset.id);
          else setHint("connect_click_or_drag_target", { name: node.name });
          return;
        }
        stopAutoPan();
        drag = null;
      });
      element.addEventListener("contextmenu", (event) => {
        if (!editable || !flushNode()) return;
        event.preventDefault();
        event.stopPropagation();
        openNodeMenu(node, event.clientX, event.clientY, element, nodeMenuActions);
      });
      if (menuButton) {
        menuButton.addEventListener("pointerdown", (event) => event.stopPropagation());
        menuButton.addEventListener("click", (event) => {
          event.stopPropagation();
          const rect = menuButton.getBoundingClientRect();
          openNodeMenu(node, rect.left, rect.bottom + 4, menuButton, nodeMenuActions);
        });
      }
      element.addEventListener("keydown", (event) => {
        if (event.target !== element) return;
        if (event.key === "Escape" && connecting) {
          event.preventDefault();
          cancelConnect();
          return;
        }
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          if (readonly) {
            if (onNode) onNode(node, phases[node.id] || null);
            return;
          }
          if (connecting) {
            if (node.id !== connecting.from) finishConnect(node.id);
            return;
          }
          if (!flushNode()) return;
          selected = node.id;
          selectedEdge = null;
          if (event.key === "Enter") {
            openNodeEditor(node);
            return;
          }
          renderInspector();
          nodes.querySelectorAll(".wf-node").forEach((item) => item.classList.toggle("selected", item.dataset.id === selected));
          return;
        }
        if (!editable) return;
        if (event.key === "Delete" || event.key === "Backspace") {
          event.preventDefault();
          removeNode(node);
          return;
        }
        const step = event.shiftKey ? 40 : 10;
        const delta = {
          ArrowLeft: [-step, 0],
          ArrowRight: [step, 0],
          ArrowUp: [0, -step],
          ArrowDown: [0, step],
        }[event.key];
        if (!delta) return;
        event.preventDefault();
        node.x = Math.max(0, (Number(node.x) || 0) + delta[0]);
        node.y = Math.max(0, (Number(node.y) || 0) + delta[1]);
        element.style.left = `${node.x}px`;
        element.style.top = `${node.y}px`;
        drawEdges();
        markDirty();
      });
    });
    drawEdges();
  }

  function edgeWhenLabel(when) {
    if (when === "failure") return h("error.generic");
    if (when === "always") return h("always");
    return h("success");
  }

  function edgeWhenText(when) {
    if (when === "failure") return t("error.generic");
    if (when === "always") return t("always");
    return t("success");
  }

  function targetOptions(fromId, current, placeholder) {
    const options = placeholder ? [`<option value="">${h("choose_target")}</option>`] : [];
    graph.nodes.forEach((item) => {
      if (item.id === fromId) return;
      options.push(`<option value="${esc(item.id)}"${item.id === current ? " selected" : ""}>${esc(item.name)}</option>`);
    });
    return options.join("");
  }

  function outgoingRows(node) {
    return graph.edges
      .map((edge, index) => ({ edge, index }))
      .filter(({ edge }) => edge.from === node.id)
      .map(({ edge, index }) => {
        const when = ["success", "failure", "always"].includes(edge.when) ? edge.when : "always";
        const target = graph.nodes.find((item) => item.id === edge.to);
        const targetName = target ? target.name : edge.to;
        return `<div class="edge-row" data-edge-index="${index}" data-edge-when="${when}">
          <span class="pill ${when}">${edgeWhenLabel(when)}</span>
          <select class="fit edge-target" data-edge-index="${index}" aria-label="${h("target")}: ${esc(targetName)}">${targetOptions(node.id, edge.to, false)}</select>
          <button class="btn compact ghost" type="button" data-remove-edge="${index}" aria-label="${h("delete_edge")}: ${esc(targetName)}">${h("remove")}</button>
        </div>`;
      })
      .join("");
  }

  function changeOutgoing(node, index, toId) {
    const edge = graph.edges[index];
    if (!edge || edge.from !== node.id) return false;
    if (!toId || toId === node.id || !graph.nodes.some((item) => item.id === toId)) return false;
    if (graph.edges.some((item, position) => position !== index && item.from === edge.from && item.to === toId && item.when === edge.when)) {
      setHint("the_same_edge_is_present_twice");
      return false;
    }
    edge.to = toId;
    markDirty();
    setHint("edge_changed");
    drawEdges();
    renderNodes();
    return true;
  }

  function addOutgoing(node) {
    const host = document.querySelector("#inspector");
    const when = host.querySelector("#link-when").value;
    const toId = host.querySelector("#link-target").value;
    if (!toId || toId === node.id || !graph.nodes.some((item) => item.id === toId)) {
      setHint("choose_target");
      return;
    }
    if (edgeExists(node.id, toId, when)) {
      setHint("the_same_edge_is_present_twice");
      return;
    }
    graph.edges.push({ from: node.id, to: toId, when });
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    markDirty();
    setHint("edge_added", { when: edgeWhenText(when), from: node.name, to: graph.nodes.find((item) => item.id === toId).name });
    drawEdges();
    renderNodes();
    renderInspector();
  }

  function removeOutgoing(node, index) {
    const edge = graph.edges[index];
    if (!edge || edge.from !== node.id) return;
    graph.edges.splice(index, 1);
    markDirty();
    setHint("edge_removed");
    drawEdges();
    renderNodes();
    renderInspector();
  }

  function freeNodePosition() {
    const width = 200;
    const height = 150;
    const box = (node) => {
      const element = nodes.querySelector(`[data-id="${CSS.escape(node.id)}"]`);
      return {
        x: Number(node.x) || 0,
        y: Number(node.y) || 0,
        width: element ? element.offsetWidth : width,
        height: element ? element.offsetHeight : height,
      };
    };
    const overlaps = (x, y) => graph.nodes.some((node) => {
      const item = box(node);
      return x < item.x + item.width && x + width > item.x && y < item.y + item.height && y + height > item.y;
    });
    const anchor = graph.nodes.find((node) => node.id === selected);
    let x = 40;
    let y = 40;
    if (anchor) {
      const item = box(anchor);
      x = item.x;
      y = item.y + item.height + 20;
    } else if (graph.nodes.length) {
      x = Math.max(...graph.nodes.map((node) => (Number(node.x) || 0) + width)) + 40;
    }
    while (overlaps(x, y)) y += height;
    return { x, y };
  }

  function fieldValue(root, id) {
    const element = root.querySelector(id);
    if (!element) return "";
    if (element.tagName === "SELECT") {
      const index = element.selectedIndex;
      if (typeof index === "number" && index >= 0 && element.options && element.options[index]) {
        return element.options[index].value;
      }
      const chosen = element.querySelector("option[selected]");
      if (chosen) return chosen.value;
    }
    return element.value == null ? "" : String(element.value);
  }

  function note(message) {
    const hint = document.querySelector("#wf-hint");
    if (hint) hint.textContent = message;
  }

  function readOverrides(node, root = document) {
    const template = templates.find((item) => item.id === node.template_id);
    const extraField = root.querySelector("#node-extra");
    if (!template || !extraField) return true;
    const fromDialog = root !== document;
    const fail = (key, vars) => {
      if (fromDialog) {
        const error = new Error(key);
        if (vars) error.vars = vars;
        throw error;
      }
      note(t(key, vars));
      return false;
    };
    const over = {};
    const templateSpec = template.spec || {};
    if (template.tool === "ansible") {
      const inventoryId = fieldValue(root, "#node-inventory");
      if (inventoryId !== (template.inventory_id || "")) over.inventory_id = inventoryId;
      const spec = {};
      const jobType = fieldValue(root, "#node-job-type") === "check" ? "check" : "run";
      const templateJob = templateSpec.job_type === "check" ? "check" : "run";
      if (jobType !== templateJob) spec.job_type = jobType;
      const verbosity = Number(fieldValue(root, "#node-verbosity")) || 0;
      const templateVerbosity = Number(templateSpec.verbosity) || 0;
      if (verbosity !== templateVerbosity) spec.verbosity = verbosity;
      const forksRaw = fieldValue(root, "#node-forks").trim();
      const templateForks = templateSpec.forks;
      const templateHasForks = templateForks !== undefined && templateForks !== null && templateForks !== "";
      if (forksRaw === "") {
        if (templateHasForks) spec.forks = null;
      } else {
        const forks = Number(forksRaw);
        if (!Number.isInteger(forks)) return fail("forks_is_a_whole_number");
        if (forks < 1 || forks > 50) return fail("forks_is_a_number_from_1_to_50");
        if (forks !== templateForks) spec.forks = forks;
      }
      const diff = Boolean(root.querySelector("#node-diff") && root.querySelector("#node-diff").checked);
      if (diff !== Boolean(templateSpec.diff)) spec.diff = diff;
      const tags = fieldValue(root, "#node-tags");
      if (tags !== (templateSpec.tags || "")) spec.tags = tags;
      const skip = fieldValue(root, "#node-skip-tags");
      if (skip !== (templateSpec.skip_tags || "")) spec.skip_tags = skip;
      const limit = fieldValue(root, "#node-limit");
      if (limit !== (templateSpec.limit || "")) spec.limit = limit;
      if (Object.keys(spec).length) over.spec = spec;
    } else if (template.tool === "opentofu") {
      const action = fieldValue(root, "#node-action") === "apply" ? "apply" : "plan";
      const templateAction = templateSpec.action === "apply" ? "apply" : "plan";
      if (action !== templateAction) over.spec = { action };
    }
    const credentialId = fieldValue(root, "#node-credential");
    if (credentialId !== (template.credential_id || "")) over.credential_id = credentialId;
    let extra;
    try {
      extra = jsonObject(extraField.value, "extra_variables");
    } catch (error) {
      if (fromDialog) throw error;
      note(t(error.message, error.vars));
      return false;
    }
    if (!sameJson(extra, template.extra_vars || {})) over.extra_vars = extra;
    if (Object.keys(over).length) node.overrides = over;
    else delete node.overrides;
    paintCaption(node);
    if (!fromDialog) {
      const hint = document.querySelector("#wf-hint");
      if (hint && (hint.textContent === t("forks_is_a_whole_number") || hint.textContent === t("forks_is_a_number_from_1_to_50") || hint.textContent === t("extra_variables_are_not_valid_json") || hint.textContent === t("extra_variables_must_be_a_json_object"))) {
        hint.textContent = "";
      }
    }
    return true;
  }

  function renderInspector() {
    if (!flushNode()) return;
    flushNode = () => true;
    const host = document.querySelector("#inspector");
    if (readonly) {
      host.innerHTML = `<h2>${h("run_2")}</h2><p class="small"><span id="run-state" class="pill ${esc(workflow.status || "pending")}">${esc(statusLabel(workflow.status || "pending"))}</span></p>${back}<p class="small muted">${h("green_is_successful_red_is_failed_blue_is")}</p>`;
      return;
    }
    if (!editable) {
      host.innerHTML = `<h2>${h("view")}</h2><p class="small muted">${h("changes_are_reserved_for_operators")}</p>`;
      return;
    }
    const node = graph.nodes.find((item) => item.id === selected);
    if (selectedEdge !== null && graph.edges[selectedEdge]) {
      const edge = graph.edges[selectedEdge];
      host.innerHTML = `<div class="inspector-block">
          <h2>${h("edge")}</h2>
          <label class="field"><span>${h("when")}</span>
            <select class="fit" id="edge-when">
              <option value="success"${edge.when === "success" ? " selected" : ""}>${h("success")}</option>
              <option value="failure"${edge.when === "failure" ? " selected" : ""}>${h("error.generic")}</option>
              <option value="always"${edge.when === "always" ? " selected" : ""}>${h("always")}</option>
            </select>
          </label>
          <button class="btn danger" id="edge-delete" type="button">${h("delete_edge")}</button>
        </div>`;
      host.querySelector("#edge-when").addEventListener("change", (event) => {
        const next = event.target.value;
        if (next !== edge.when && graph.edges.some((item, index) => index !== selectedEdge && item.from === edge.from && item.to === edge.to && item.when === next)) {
          setHint("the_same_edge_is_present_twice");
          event.target.value = edge.when;
          return;
        }
        edge.when = next;
        drawEdges();
        markDirty();
        setHint("edge_changed");
      });
      host.querySelector("#edge-delete").addEventListener("click", () => {
        if (removeEdgeAt(selectedEdge)) canvas.focus();
      });
      return;
    }
    if (!node) {
      host.innerHTML = `<p class="small muted">${h("right_click_empty_canvas_to_add_a_node")}</p>
        <p class="small muted">${h("an_empty_graph_can_be_saved_but_not")}</p>
        <p class="small muted">${h("select_a_node_to_connect_it")}</p>`;
      return;
    }
    host.innerHTML = `<div class="inspector-block">
        <h2>${h("this_node")}</h2>
        <label class="field"><span>${h("name")}</span><input id="node-name" value="${esc(node.name)}"></label>
        <label class="field"><span>${h("template")}</span><select class="fit" id="node-template">${selectOptions(templates, node.template_id, false, (row) => row.name)}</select></label>
        <label class="field"><span>${h("convergence")}</span>
          <select class="fit" id="node-join">
            <option value="any"${node.convergence !== "all" ? " selected" : ""}>${h("any_predecessor")}</option>
            <option value="all"${node.convergence === "all" ? " selected" : ""}>${h("all_predecessors")}</option>
          </select>
        </label>
        ${nodeParams(node, templates.find((item) => item.id === node.template_id))}
        <h2>${h("connect")}</h2>
        <button class="btn pill primary" id="node-connect" type="button">${h("connect")}</button>
        <p class="small muted">${h("connect_help")}</p>
        <div class="edge-list" id="outgoing-edges">${outgoingRows(node)}</div>
        <div class="edge-add">
          <label class="field"><span>${h("when")}</span>
            <select class="fit" id="link-when">
              <option value="success">${h("success")}</option>
              <option value="failure">${h("error.generic")}</option>
              <option value="always">${h("always")}</option>
            </select>
          </label>
          <label class="field"><span>${h("target")}</span><select class="fit" id="link-target">${targetOptions(node.id, "", true)}</select></label>
          <button class="btn" id="link-add" type="button">${h("add_edge")}</button>
        </div>
        <button class="btn danger" id="node-delete" type="button">${h("delete_node")}</button>
      </div>`;
    host.querySelector("#node-connect").addEventListener("click", () => startConnect(node));
    host.querySelector("#node-name").addEventListener("input", (event) => {
      node.name = event.target.value;
      const label = nodes.querySelector(`[data-id="${CSS.escape(node.id)}"] strong`);
      if (label) label.textContent = node.name;
      markDirty();
    });
    host.querySelector("#node-template").addEventListener("change", (event) => {
      const next = event.target.value;
      if (!readOverrides(node)) {
        Array.from(event.target.options).forEach((option) => {
          if (option.value === node.template_id) option.setAttribute("selected", "");
          else option.removeAttribute("selected");
        });
        return;
      }
      if (node.overrides) delete node.overrides.spec;
      if (node.overrides && node.overrides.inventory_id === undefined && node.overrides.credential_id === undefined && node.overrides.extra_vars === undefined) {
        delete node.overrides;
      }
      node.template_id = next;
      flushNode = () => true;
      markDirty();
      renderInspector();
    });
    host.querySelector("#node-join").addEventListener("change", (event) => {
      node.convergence = event.target.value;
      paintCaption(node);
      markDirty();
    });
    flushNode = () => readOverrides(node);
    ["#node-inventory", "#node-credential", "#node-job-type", "#node-verbosity", "#node-diff", "#node-action", "#node-forks", "#node-tags", "#node-skip-tags", "#node-limit", "#node-extra"].forEach((id) => {
      const element = host.querySelector(id);
      if (!element) return;
      element.addEventListener("input", markDirty);
      element.addEventListener("change", () => { if (flushNode()) markDirty(); });
    });
    host.querySelector("#link-add").addEventListener("click", () => addOutgoing(node));
    host.querySelectorAll(".edge-target").forEach((select) => {
      select.addEventListener("change", (event) => {
        const index = Number(event.target.dataset.edgeIndex);
        const edge = graph.edges[index];
        if (!edge) return;
        if (!changeOutgoing(node, index, event.target.value)) event.target.value = edge.to;
      });
    });
    host.querySelectorAll("[data-remove-edge]").forEach((button) => {
      button.addEventListener("click", () => removeOutgoing(node, Number(button.dataset.removeEdge)));
    });
    host.querySelector("#node-delete").addEventListener("click", () => removeNode(node));
  }

  svg.addEventListener("click", (event) => {
    const path = event.target.closest("path");
    if (!path || readonly) return;
    selectEdge(Number(path.dataset.edge), true);
  });
  svg.addEventListener("keydown", (event) => {
    const path = event.target.closest("path");
    if (!path || !editable) return;
    const index = Number(path.dataset.edge);
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      selectEdge(index, true);
      return;
    }
    if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      if (removeEdgeAt(index)) canvas.focus();
    }
  });
  function canvasPointerDown(event) {
    if (event.target.closest(".wf-node, path, .canvas-controls, .minimap, button")) return;
    if (connecting) {
      cancelConnect();
      return;
    }
    if (!flushNode()) return;
    if (editable) {
      selected = null;
      selectedEdge = null;
      renderNodes();
      renderInspector();
    }
    pan = { pointerId: event.pointerId, x: event.clientX, y: event.clientY, viewX: viewport.view.x, viewY: viewport.view.y };
    if (canvas.setPointerCapture) canvas.setPointerCapture(event.pointerId);
    canvas.classList.add("panning");
  }

  function canvasPointerMove(event) {
    if (!pan || pan.pointerId !== event.pointerId) return;
    viewport.set({
      x: pan.viewX + (event.clientX - pan.x),
      y: pan.viewY + (event.clientY - pan.y),
      scale: viewport.view.scale,
    });
  }

  function endPan(event) {
    if (!pan || (event.pointerId !== undefined && pan.pointerId !== event.pointerId)) return;
    pan = null;
    canvas.classList.remove("panning");
  }

  function canvasContextMenu(event) {
    if (!editable || !flushNode()) return;
    if (event.target.closest(".wf-node, .canvas-controls, .minimap")) return;
    event.preventDefault();
    openCanvasMenu(localPoint(event), event.clientX, event.clientY, openAddNode);
  }

  canvas.addEventListener("pointerdown", canvasPointerDown);
  canvas.addEventListener("pointermove", canvasPointerMove);
  canvas.addEventListener("pointerup", endPan);
  canvas.addEventListener("pointercancel", endPan);
  canvas.addEventListener("contextmenu", canvasContextMenu);
  board.addEventListener("pointerdown", (event) => {
    canvasPointerDown(event);
    event.stopPropagation();
  });
  board.addEventListener("contextmenu", (event) => {
    canvasContextMenu(event);
    event.stopPropagation();
  });
  canvas.addEventListener("wheel", (event) => {
    if (!(event.ctrlKey || event.metaKey)) return;
    event.preventDefault();
    const rect = canvas.getBoundingClientRect();
    viewport.zoom(event.deltaY < 0 ? 1.15 : 1 / 1.15, { x: event.clientX - rect.left, y: event.clientY - rect.top });
  }, { passive: false });
  const zoomInButton = document.querySelector("[data-canvas-zoom-in]");
  if (zoomInButton) zoomInButton.addEventListener("click", () => zoomBy(1.2));
  const zoomOutButton = document.querySelector("[data-canvas-zoom-out]");
  if (zoomOutButton) zoomOutButton.addEventListener("click", () => zoomBy(1 / 1.2));
  const fitButton = document.querySelector("[data-canvas-fit]");
  if (fitButton) fitButton.addEventListener("click", fitGraph);
  const resetButton = document.querySelector("[data-canvas-reset]");
  if (resetButton) resetButton.addEventListener("click", () => viewport.reset());

  canvas.addEventListener("keydown", (event) => {
    if (!editable || event.target !== canvas) return;
    if (event.key === "Escape" && connecting) {
      event.preventDefault();
      cancelConnect();
      return;
    }
    const key = String(event.key);
    if (key === "+" || key === "=") {
      event.preventDefault();
      zoomBy(1.2);
      return;
    }
    if (key === "-" || key === "_") {
      event.preventDefault();
      zoomBy(1 / 1.2);
      return;
    }
    if (key === "0") {
      event.preventDefault();
      viewport.reset();
      return;
    }
    if (key.toLowerCase() === "f") {
      event.preventDefault();
      fitGraph();
      return;
    }
    if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
    event.preventDefault();
    if (!flushNode()) return;
    const point = freeNodePosition();
    const screen = viewport.graphToScreen(point);
    const rect = canvas.getBoundingClientRect();
    openCanvasMenu(point, rect.left + screen.x, rect.top + screen.y, openAddNode);
  });

  function bindWorkflowTabs() {
    const buttons = [...document.querySelectorAll("[data-workflow-tab]")];
    if (!buttons.length) return;
    const panels = {
      editor: document.querySelector("#wf-panel-editor"),
      schedules: document.querySelector("#wf-panel-schedules"),
      danger: document.querySelector("#wf-panel-danger"),
    };
    const choose = (next, focus) => {
      buttons.forEach((button) => {
        const active = button.dataset.workflowTab === next;
        button.classList.toggle("primary", active);
        button.setAttribute("aria-selected", String(active));
        button.tabIndex = active ? 0 : -1;
      });
      Object.entries(panels).forEach(([id, panel]) => {
        if (panel) panel.hidden = id !== next;
      });
      history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", workflowAddress(next, workflow.id));
      appState.setShownHash(location.hash);
      if (next === "editor") {
        if (!fitted) fitOnOpen();
        drawEdges();
      }
      if (focus) {
        const active = buttons.find((button) => button.dataset.workflowTab === next);
        if (active) active.focus();
      }
    };
    buttons.forEach((button, index) => {
      button.addEventListener("click", () => {
        if (button.dataset.workflowTab !== activeTab) {
          activeTab = button.dataset.workflowTab;
          choose(activeTab, false);
        }
      });
      button.addEventListener("keydown", (event) => {
        let target = null;
        if (event.key === "ArrowRight") target = buttons[(index + 1) % buttons.length];
        else if (event.key === "ArrowLeft") target = buttons[(index - 1 + buttons.length) % buttons.length];
        else if (event.key === "Home") target = buttons[0];
        else if (event.key === "End") target = buttons[buttons.length - 1];
        if (!target) return;
        event.preventDefault();
        activeTab = target.dataset.workflowTab;
        choose(activeTab, true);
      });
    });
  }

  renderNodes();
  renderInspector();
  fitOnOpen();
  bindWorkflowTabs();
  mountSchedules(document.querySelector("#schedule-card"));
  paintDirty();
  if (editable) appState.setLeaveGuard(leaveGuard);

  const nameField = document.querySelector("#wf-name");
  if (nameField) {
    nameField.addEventListener("input", () => {
      markDirty();
      document.title = `${nameField.value} – ${t("workflow")} – Koreograf`;
    });
  }
  const descriptionField = document.querySelector("#wf-description");
  if (descriptionField) descriptionField.addEventListener("input", markDirty);

  async function saveGraph() {
    const saveButton = document.querySelector("#wf-save");
    if (saveButton) saveButton.disabled = true;
    try {
      const name = document.querySelector("#wf-name").value;
      const description = document.querySelector("#wf-description").value;
      await api("PUT", `/api/v1/workflows/${encodeURIComponent(workflow.id)}`, { name, description, graph });
      workflow.name = name;
      workflow.description = description;
      workflow.graph = JSON.parse(JSON.stringify(graph));
      setDirty(false);
      return true;
    } catch (error) {
      handle(error);
      return false;
    } finally {
      if (saveButton) saveButton.disabled = false;
    }
  }

  async function saveWorkflow() {
    if (!editable || !flushNode()) return false;
    return saveGraph();
  }

  const save = document.querySelector("#wf-save");
  if (save) {
    save.addEventListener("click", () => {
      saveWorkflow();
    });
  }

  function chooseLaunchMode() {
    return new Promise((resolve) => {
      let settled = false;
      const decide = (value) => {
        if (settled) return;
        settled = true;
        resolve(value);
      };
      const dialog = openDialog({
        id: "workflow-launch-dialog",
        title: t("unsaved_changes"),
        body: `<p class="small muted">${h("launch_unsaved_changes_hint")}</p><div class="choice-row"><button class="btn pill primary" type="button" data-launch-save>${h("save_and_launch")}</button><button class="btn" type="button" data-launch-saved>${h("launch_last_saved_version")}</button><button class="btn" type="button" data-launch-cancel>${h("cancel")}</button></div>`,
        form: false,
        actions: false,
        onClose: () => decide("cancel"),
      });
      dialog.overlay.querySelector("[data-launch-save]").addEventListener("click", () => {
        decide("save");
        dialog.close();
      });
      dialog.overlay.querySelector("[data-launch-saved]").addEventListener("click", () => {
        decide("saved");
        dialog.close();
      });
      dialog.overlay.querySelector("[data-launch-cancel]").addEventListener("click", () => dialog.close());
    });
  }

  const launch = document.querySelector("#wf-launch");
  if (launch) {
    launch.addEventListener("click", async () => {
      if (!flushNode()) return;
      let mode = "current";
      if (dirty) {
        mode = await chooseLaunchMode();
        if (mode === "cancel") return;
      }
      const name = document.querySelector("#wf-name").value;
      const graphToLaunch = mode === "saved" ? workflow.graph : graph;
      const launchName = mode === "saved" ? workflow.name : name;
      if (workflowTofuApply(graphToLaunch, templates) && !(await confirmTofuApply({ name: launchName }))) return;
      try {
        if (mode === "save" && !(await saveGraph())) return;
        const run = await api("POST", `/api/v1/workflows/${encodeURIComponent(workflow.id)}/launch`, { extra_vars: {} });
        appState.setLeaveGuard(null);
        go(`#/runs/${run.id}`);
      } catch (error) {
        handle(error);
      }
    });
  }

  function saveShortcut(event) {
    if (!(event.ctrlKey || event.metaKey) || String(event.key).toLowerCase() !== "s") return;
    if (event.defaultPrevented || !editable || document.querySelector(".overlay")) return;
    event.preventDefault();
    if (dirty) saveWorkflow();
  }

  const editorStack = document.querySelector(".editor-stack");
  if (editorStack) editorStack.addEventListener("keydown", saveShortcut);
  canvas.addEventListener("keydown", saveShortcut);

  const remove = document.querySelector("#wf-delete");
  if (remove) {
    remove.addEventListener("click", async () => {
      if (!(await confirmDialog({ title: t("delete"), body: t("delete_this_workflow", { name: workflow.name }), confirmLabel: t("delete"), danger: true }))) return;
      try {
        await api("DELETE", `/api/v1/workflows/${encodeURIComponent(workflow.id)}`);
        go("#/workflows");
      } catch (error) {
        handle(error);
      }
    });
  }

  function applyPhases() {
    nodes.querySelectorAll(".wf-node").forEach((element) => {
      const node = graph.nodes.find((item) => item.id === element.dataset.id);
      const phase = phases[element.dataset.id];
      const phaseClass = phase ? ` phase-${phase.phase}` : "";
      element.className = `wf-node run-node${phaseClass}`;
      if (node) element.setAttribute("aria-label", nodeLabel(node));
    });
  }

  return { applyPhases, viewport };
}
