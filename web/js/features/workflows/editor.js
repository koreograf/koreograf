import * as appState from "../../core/state.js";
import { api } from "../../core/api.js";
import { confirmDialog, openDialog } from "../../core/dialog.js";
import { can, go, handle, route, shell } from "../../core/app.js";
import { bindChips, flushChips } from "../../core/chips.js";
import { credentialKindLabel } from "../../core/credentials.js";
import { esc, h, selectOptions, table } from "../../core/dom.js";
import { spanText, statusLabel, verbosityLabel, when } from "../../core/format.js";
import { jsonObject } from "../../core/forms.js";
import { t } from "../../core/i18n.js";
import { ansibleFields, credentialFields, extraVarsField, inventoryField, tofuFields } from "../../core/template-fields.js";
import { confirmTofuApply } from "../../core/tofu.js";
import { mountSchedules } from "../schedules.js";
import { chooseEdgeType, edgeWhenLabel, openCanvasMenu, openNodeMenu } from "./menus.js";
import { GRID_SIZE, layoutGraph, snapToGrid } from "./layout.js";
import { analyzeGraph, issuesForServerError, wouldCreateCycle } from "./validation.js";
import { autoPan, createViewport, minimapLayout, zoomAt } from "./viewport.js";

export function workflowTofuApply(graph, templates) {
  return ((graph && graph.nodes) || []).some((node) => {
    const template = templates.find((item) => item.id === node.template_id);
    if (!template || template.tool !== "opentofu") return false;
    const override = node.overrides && node.overrides.spec;
    const action = override && override.action !== undefined ? override.action : (template.spec || {}).action;
    return action === "apply";
  });
}

const PORT_OFFSETS = { success: 40, failure: 76, always: 58 };
const PORT_SYMBOLS = { success: "✓", failure: "✗", always: "↻" };
const TOOL_ICONS = {
  ansible: `<circle cx="8" cy="8" r="6.4" fill="none" stroke="currentColor" stroke-width="1.3"></circle><path d="M8 3.6 10.7 12 8 10.1 5.3 12Z" fill="currentColor"></path>`,
  opentofu: `<path d="M8 1.6 13.3 4.7v6.6L8 14.4 2.7 11.3V4.7Z" fill="none" stroke="currentColor" stroke-width="1.3"></path><path d="M8 5.2 10.7 6.8v3.2L8 11.6 5.3 10V6.8Z" fill="currentColor"></path>`,
  shell: `<path d="M2.6 4.4 6 8l-3.4 3.6M7.6 11.6H13" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"></path>`,
};

export function edgePoint(node, when) {
  const y = PORT_OFFSETS[when] || PORT_OFFSETS.success;
  return { x: node.x + 200, y: node.y + y };
}

export function workflowTab(hash = location.hash || "") {
  const params = new URLSearchParams((hash || "").split("?")[1] || "");
  const tab = params.get("tab");
  return tab === "schedules" || tab === "runs" || tab === "danger" ? tab : "editor";
}

export function workflowAddress(tab, id) {
  const params = new URLSearchParams();
  if (tab !== "editor") params.set("tab", tab);
  const query = params.toString();
  return `#/workflows/${encodeURIComponent(id)}${query ? `?${query}` : ""}`;
}

export function mountEditor(workflow, templates, readonly, phases, catalogs, hooks) {
  const onNode = hooks && hooks.onNode;
  const runActions = hooks && hooks.runActions ? hooks.runActions : "";
  const lists = catalogs || { inventories: [], credentials: [] };
  const runs = lists.runs || [];
  const compactQuery = typeof window.matchMedia === "function"
    ? window.matchMedia("(max-width: 900px), (hover: none) and (pointer: coarse)")
    : null;
  const compactEditor = () => (compactQuery ? compactQuery.matches : Number(window.innerWidth) <= 900);
  let editable = !readonly && can("operator") && !compactEditor();
  const phoneViewer = !readonly && !editable && compactEditor();
  const team = ((appState.me && appState.me.teams) || []).find((item) => item.id === appState.teamId);
  const teamName = team ? team.name : "";
  const readOnlyHint = () => (teamName ? t("read_only_hint", { team: teamName }) : t("read_only_hint_no_team"));
  const graph = {
    nodes: (workflow.graph.nodes || []).map((node) => ({ ...node })),
    edges: (workflow.graph.edges || []).map((edge) => ({ ...edge })),
  };
  let validation = analyzeGraph(graph, templates);
  let serverIssues = [];
  let selected = null;
  let selectedEdge = null;
  let connecting = null;
  let connectDrag = null;
  let drag = null;
  let dirty = false;
  let baseline = null;
  let historyStack = [snapshotGraph()];
  let historyIndex = 0;
  let toastTimer = null;
  const leaveGuard = () => dirty;
  const back = readonly && workflow.workflow_id
    ? `<a class="btn" href="#/workflows/${esc(workflow.workflow_id)}">${h("back_to_the_workflow")}</a>`
    : "";
  const tabbed = !readonly;
  const canDelete = can("operator") && !compactEditor();
  const desktopNote = phoneViewer ? `<p class="banner warn" id="wf-desktop-note" role="status">${h("workflow_desktop_only")}</p>` : "";
  const tabItems = tabbed
    ? [["editor", "editor"], ["schedules", "schedules"], ["runs", "runs"], ...(canDelete ? [["danger", "danger_zone"]] : [])]
    : [];
  let activeTab = tabbed ? workflowTab() : "editor";
  if (activeTab === "danger" && !canDelete) activeTab = "editor";
  const tabBar = tabbed
    ? `<div class="spread tab-bar"><div class="tabs" role="tablist" aria-label="${h("workflow")}">${tabItems.map(([id, label]) => `<button class="btn pill${id === activeTab ? " primary" : ""}" type="button" role="tab" id="wf-tab-${id}" aria-selected="${id === activeTab}" aria-controls="wf-panel-${id}" tabindex="${id === activeTab ? "0" : "-1"}" data-workflow-tab="${id}">${h(label)}</button>`).join("")}</div></div>`
    : "";
  const runRows = runs.map((run) => `
    <tr>
      <td><a href="#/runs/${esc(run.id)}">${esc(run.name)}</a></td>
      <td><span class="pill ${esc(run.status)}">${esc(statusLabel(run.status))}</span></td>
      <td><span class="mono">${esc(when(run.created_at))}</span></td>
      <td class="mono">${esc(spanText(run.created_at, run.finished_at, run.status))}</td>
      <td>${esc(run.trigger)}</td>
      <td>${esc(run.launched_by)}</td>
    </tr>`);
  shell(readonly ? workflow.name : t("workflow"), `
    <div class="editor-stack">
    ${tabBar}
    <div id="wf-panel-editor"${tabbed ? ` role="tabpanel" aria-labelledby="wf-tab-editor"${activeTab === "editor" ? "" : " hidden"}` : ""}>
      ${desktopNote}
      <div class="editor-body">
        <div class="canvas-column">
          ${readonly ? `<div class="run-status-bar" id="run-status-bar" role="status"><span class="small muted">${h("status")}</span><span class="pill ${esc(workflow.status || "pending")}" id="run-state-top">${esc(statusLabel(workflow.status || "pending"))}</span></div>` : ""}
          ${readonly ? "" : `
            <div class="field-row wf-meta-row" style="margin-bottom:10px">
              <label class="field wf-name-field"><span>${h("name")}</span><input class="fit" id="wf-name" value="${esc(workflow.name)}" ${editable ? "" : "disabled"}></label>
              <label class="field wf-description-field"><span>${h("description")}</span><input class="fit" id="wf-description" maxlength="1000" value="${esc(workflow.description)}" title="${esc(workflow.description)}" ${editable ? "" : "disabled"}></label>
            </div>`}
          <p class="small muted" id="wf-hint" role="status" aria-live="polite">${readonly ? h("colors_show_the_run_a_click_shows_the") : editable ? h("drag_nodes_and_use_the_canvas_menus") : phoneViewer ? h("workflow_desktop_only") : esc(readOnlyHint())}</p>
          <p class="small muted" id="wf-keyboard-help">${h(readonly ? "run_keyboard_help" : editable ? "workflow_keyboard_help" : "viewer_keyboard_help")}</p>
          <div class="canvas" id="canvas" tabindex="0" role="application" aria-label="${h("workflow_graph")}" aria-describedby="wf-keyboard-help">
            <div class="canvas-controls" role="group" aria-label="${h("canvas_controls")}">
              <button class="btn compact ghost" type="button" data-canvas-zoom-in aria-label="${h("zoom_in")}">+</button>
              <button class="btn compact ghost" type="button" data-canvas-zoom-out aria-label="${h("zoom_out")}">−</button>
              <button class="btn compact ghost" type="button" data-canvas-fit>${h("fit_view")}</button>
              <button class="btn compact ghost" type="button" data-canvas-reset>${h("zoom_reset")}</button>
              ${editable ? `<button class="btn compact ghost" type="button" id="wf-auto-layout" aria-keyshortcuts="Shift+Alt+T" title="${h("auto_layout")}" aria-label="${h("auto_layout")}">${h("auto_layout")}</button>` : ""}
              <button class="btn compact ghost" type="button" id="wf-shortcuts" aria-haspopup="dialog" aria-label="${h("keyboard_shortcuts")}" title="${h("keyboard_shortcuts")}">?</button>
            </div>
            <div class="minimap" id="minimap" role="img" aria-label="${h("minimap")}" hidden></div>
            <div id="board" style="position:absolute;left:0;top:0;transform-origin:0 0"><svg class="edges" id="edges"></svg><div id="nodes"></div></div>
            ${editable ? `<div class="canvas-empty" id="wf-empty" hidden>
              <p class="small muted">${h("empty_canvas_hint")}</p>
              <button class="btn pill primary" type="button" id="wf-add-first">${h("add_first_step")}</button>
            </div>` : ""}
          </div>
        </div>
        <div class="inspector-panel" id="inspector-panel">
          <div class="inspector-bar">
            <button class="btn compact ghost inspector-toggle" type="button" id="inspector-toggle" aria-expanded="true" aria-controls="inspector" aria-label="${h("collapse_node_panel")}" title="${h("collapse_node_panel")}">−</button>
          </div>
          <aside class="card inspector" id="inspector"></aside>
        </div>
      </div>
      ${readonly ? `<section class="card run-log" id="run-log" aria-live="polite"></section>` : ""}
    </div>
    ${readonly ? "" : `<div id="wf-panel-schedules" role="tabpanel" aria-labelledby="wf-tab-schedules"${activeTab === "schedules" ? "" : " hidden"}>
      <section class="card" id="schedule-card" data-kind="workflow" data-target="${esc(workflow.id)}"></section>
    </div>`}
    ${readonly ? "" : `<div id="wf-panel-runs" role="tabpanel" aria-labelledby="wf-tab-runs"${activeTab === "runs" ? "" : " hidden"}>
      <section class="card">
        <h2>${h("runs")}</h2>
        ${runs.length
          ? table([t("name"), t("status"), t("start"), t("duration"), t("trigger"), t("who")], runRows)
          : `<p class="small muted">${h("no_runs_yet")}</p>`}
      </section>
    </div>`}
    ${!tabbed || !canDelete ? "" : `<div id="wf-panel-danger" role="tabpanel" aria-labelledby="wf-tab-danger"${activeTab === "danger" ? "" : " hidden"}>
      <section class="card danger-zone">
        <h2>${h("danger_zone")}</h2>
        <p class="small muted">${h("deleting_cannot_be_undone")}</p>
        <button class="btn danger" id="wf-delete" type="button">${h("delete")}</button>
      </section>
    </div>`}
    ${editable ? `<div class="toast" id="wf-toast" role="status" aria-live="polite" hidden></div>` : ""}
    </div>`, readonly ? `${back}${runActions}` : editable ? `
      <span class="row" role="group" aria-label="${h("history")}"><button class="btn" id="wf-undo" type="button" aria-keyshortcuts="Control+Z Meta+Z" disabled>${h("undo")}</button><button class="btn" id="wf-redo" type="button" aria-keyshortcuts="Control+Shift+Z Control+Y Meta+Shift+Z" disabled>${h("redo")}</button></span><span class="small muted" id="wf-status" role="status" aria-live="polite">${h("all_changes_saved")}</span><span class="small wf-validation" id="wf-validation" role="status" aria-live="polite"></span><button class="btn" id="wf-save" type="button">${h("save")}</button><button class="btn primary" id="wf-launch" type="button" aria-describedby="wf-validation">${h("launch")}</button>` : `<span class="pill read-only">${h("read_only")}</span>`);

  document.title = readonly
    ? `${t("run_2")} ${workflow.name} – Koreograf`
    : `${workflow.name} – ${t("workflow")} – Koreograf`;

  const canvas = document.querySelector("#canvas");
  canvas.classList.toggle("editable", editable);
  canvas.classList.toggle("inspectable", !readonly);
  const board = document.querySelector("#board");
  const svg = document.querySelector("#edges");
  const nodes = document.querySelector("#nodes");
  const minimap = document.querySelector("#minimap");
  const editorBody = document.querySelector(".editor-body");
  const inspector = document.querySelector("#inspector");
  const inspectorToggle = document.querySelector("#inspector-toggle");
  let inspectorCollapsed = false;
  let fitted = false;
  let pan = null;
  let pinch = null;
  let autoPanTimer = null;
  const touchPointers = new Map();

  const narrowQuery = typeof window.matchMedia === "function" ? window.matchMedia("(max-width: 900px)") : null;

  const editorRoot = document.querySelector(".editor-stack");
  function watchCompactMode() {
    if (readonly || !compactQuery || typeof compactQuery.addEventListener !== "function") return;
    const onChange = () => {
      if (!editorRoot || !editorRoot.isConnected) {
        compactQuery.removeEventListener("change", onChange);
        return;
      }
      const nextEditable = !readonly && can("operator") && !compactEditor();
      if (nextEditable === editable) return;
      compactQuery.removeEventListener("change", onChange);
      route();
    };
    compactQuery.addEventListener("change", onChange);
  }
  watchCompactMode();

  function narrowScreen() {
    if (narrowQuery) return narrowQuery.matches;
    return Number(window.innerWidth) <= 900;
  }

  function setInspectorCollapsed(collapsed, focus) {
    inspectorCollapsed = Boolean(collapsed);
    if (editorBody) editorBody.classList.toggle("inspector-collapsed", inspectorCollapsed);
    if (inspector) inspector.hidden = inspectorCollapsed;
    if (inspectorToggle) {
      const key = inspectorCollapsed ? "expand_node_panel" : "collapse_node_panel";
      inspectorToggle.setAttribute("aria-expanded", String(!inspectorCollapsed));
      inspectorToggle.setAttribute("aria-label", t(key));
      inspectorToggle.title = t(key);
      inspectorToggle.textContent = inspectorCollapsed ? "+" : "−";
    }
    if (focus && inspectorToggle) inspectorToggle.focus();
  }

  function revealInspector() {
    if (!narrowScreen() || !inspectorCollapsed) return;
    setInspectorCollapsed(false, false);
  }

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
    let nodeWidth = 0;
    let nodeHeight = 0;
    graph.nodes.forEach((node) => {
      const element = nodes.querySelector(`[data-id="${CSS.escape(node.id)}"]`);
      if (!element) return;
      nodeWidth = Math.max(nodeWidth, element.offsetWidth || 0);
      nodeHeight = Math.max(nodeHeight, element.offsetHeight || 0);
    });
    const options = {};
    if (nodeWidth) options.nodeWidth = nodeWidth;
    if (nodeHeight) options.nodeHeight = nodeHeight;
    viewport.fit(graph.nodes, options);
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

  function snapshotGraph() {
    return JSON.parse(JSON.stringify({ nodes: graph.nodes, edges: graph.edges }));
  }

  function currentState() {
    const nameField = document.querySelector("#wf-name");
    const descriptionField = document.querySelector("#wf-description");
    return {
      graph: snapshotGraph(),
      name: nameField ? nameField.value : workflow.name,
      description: descriptionField ? descriptionField.value : workflow.description,
    };
  }

  function updateHistoryButtons() {
    const undoButton = document.querySelector("#wf-undo");
    const redoButton = document.querySelector("#wf-redo");
    if (undoButton) undoButton.disabled = !editable || historyIndex <= 0;
    if (redoButton) redoButton.disabled = !editable || historyIndex >= historyStack.length - 1;
  }

  function recordHistory() {
    const snapshot = snapshotGraph();
    if (sameJson(snapshot, historyStack[historyIndex])) {
      updateHistoryButtons();
      return;
    }
    historyStack = historyStack.slice(0, historyIndex + 1);
    historyStack.push(snapshot);
    historyIndex = historyStack.length - 1;
    updateHistoryButtons();
  }

  function refreshDirty() {
    setDirty(!sameJson(currentState(), baseline));
  }

  function hideToast() {
    const toast = document.querySelector("#wf-toast");
    if (toastTimer) window.clearTimeout(toastTimer);
    toastTimer = null;
    if (!toast) return;
    toast.hidden = true;
    toast.innerHTML = "";
  }

  function showToast(key) {
    const toast = document.querySelector("#wf-toast");
    if (!toast) return;
    if (toastTimer) window.clearTimeout(toastTimer);
    toast.innerHTML = `<span>${h(key)}</span><button class="btn compact" type="button" data-toast-undo>${h("undo")}</button>`;
    toast.hidden = false;
    const button = toast.querySelector("[data-toast-undo]");
    if (button) button.addEventListener("click", () => {
      hideToast();
      undo();
    });
    toastTimer = window.setTimeout(hideToast, 8000);
    if (toastTimer && typeof toastTimer.unref === "function") toastTimer.unref();
  }

  function restoreGraph(snapshot) {
    graph.nodes = JSON.parse(JSON.stringify(snapshot.nodes));
    graph.edges = JSON.parse(JSON.stringify(snapshot.edges));
    if (selected && !graph.nodes.some((node) => node.id === selected)) selected = null;
    if (selectedEdge !== null && !graph.edges[selectedEdge]) selectedEdge = null;
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    serverIssues = [];
    recomputeValidation();
    renderNodes();
    renderInspector();
    refreshDirty();
    updateHistoryButtons();
  }

  function undo() {
    if (!editable) return;
    if (historyIndex <= 0) return;
    historyIndex -= 1;
    hideToast();
    restoreGraph(historyStack[historyIndex]);
  }

  function redo() {
    if (!editable) return;
    if (historyIndex >= historyStack.length - 1) return;
    historyIndex += 1;
    hideToast();
    restoreGraph(historyStack[historyIndex]);
  }

  function autoLayoutGraph() {
    if (!editable) return;
    if (!graph.nodes.length) return;
    const before = snapshotGraph();
    const positions = new Map(layoutGraph(graph).map((position) => [String(position.id), position]));
    graph.nodes.forEach((node) => {
      const position = positions.get(String(node.id));
      if (!position) return;
      node.x = position.x;
      node.y = position.y;
    });
    if (!sameJson(before, snapshotGraph())) {
      recordHistory();
      markDirty();
      renderNodes();
      renderInspector();
    }
    fitGraph();
    setHint("auto_layout_done");
    canvas.focus();
  }

  function layoutShortcut(event) {
    if (event.defaultPrevented || !editable || document.querySelector(".overlay")) return;
    if (!event.shiftKey || !event.altKey) return;
    if (String(event.key).toLowerCase() !== "t") return;
    if (isFormField(event.target) || Boolean(event.target && event.target.isContentEditable)) return;
    event.preventDefault();
    autoLayoutGraph();
  }

  function openShortcuts() {
    const rows = editable
      ? [
        ["Ctrl/Cmd+Z", "shortcut_undo"],
        ["Ctrl/Cmd+Shift+Z · Ctrl+Y", "shortcut_redo"],
        ["Delete / Backspace", "shortcut_delete"],
        ["Enter / Space", "shortcut_connect"],
        ["Ctrl/Cmd+S", "shortcut_save"],
        ["Shift+Alt+T", "shortcut_layout"],
        ["Shift+F10", "shortcut_canvas"],
        ["?", "shortcut_help"],
      ]
      : [
        ["Tab · Enter · Space", readonly ? "run_keyboard_help" : "viewer_keyboard_help"],
        ["+ − 0 F · Ctrl+wheel", "shortcut_view"],
        ["?", "shortcut_help"],
      ];
    openDialog({
      id: "shortcuts-dialog",
      title: t("keyboard_shortcuts"),
      body: `<ul class="shortcut-list small">${rows.map(([keys, key]) => `<li><span class="mono">${keys}</span><span>${h(key)}</span></li>`).join("")}</ul>`,
      form: false,
      actions: false,
    });
  }

  function isFormField(target) {
    return Boolean(target && target.closest && target.closest("input, textarea, select"));
  }

  function historyShortcut(event) {
    if (event.defaultPrevented || document.querySelector(".overlay")) return;
    const key = String(event.key).toLowerCase();
    const inTextField = isFormField(event.target) || Boolean(event.target && event.target.isContentEditable);
    if ((event.ctrlKey || event.metaKey) && key === "z") {
      if (!editable || inTextField) return;
      event.preventDefault();
      if (event.shiftKey) redo();
      else undo();
      return;
    }
    if ((event.ctrlKey || event.metaKey) && key === "y") {
      if (!editable || inTextField) return;
      event.preventDefault();
      redo();
      return;
    }
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.key !== "?" || isFormField(event.target)) return;
    event.preventDefault();
    openShortcuts();
  }

  function predecessorCount(node) {
    return new Set(graph.edges.filter((edge) => edge.to === node.id).map((edge) => edge.from)).size;
  }

  function toolKey(tool) {
    return tool === "ansible" || tool === "opentofu" || tool === "shell" ? tool : "";
  }

  function nodeMeta(template) {
    if (!template) return "";
    const key = toolKey(template.tool);
    const tool = key
      ? `<span class="wf-node-tool" role="img" aria-label="${h(key)}" title="${h(key)}" data-tool="${key}"><svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" focusable="false">${TOOL_ICONS[key]}</svg></span>`
      : "";
    return `${tool}<span class="wf-node-template" title="${esc(template.name)}">${esc(template.name)}</span>`;
  }

  function chip(kind, key, value) {
    const label = t(key, { value });
    return `<span class="wf-node-chip" data-chip="${kind}" title="${esc(label)}">${esc(label)}</span>`;
  }

  function nodeChips(node, template) {
    const over = node.overrides || {};
    const spec = over.spec || {};
    const chips = [];
    if (over.inventory_id !== undefined) {
      const inventory = over.inventory_id ? lists.inventories.find((item) => item.id === over.inventory_id) : null;
      const name = over.inventory_id ? (inventory ? inventory.name : over.inventory_id) : t("no_inventory");
      chips.push(chip("inventory", "node_chip_inventory", name));
    }
    if (spec.limit !== undefined) chips.push(chip("limit", "node_chip_limit", spec.limit || t("none")));
    if (spec.tags !== undefined) chips.push(chip("tags", "node_chip_tags", spec.tags || t("none")));
    if (template && template.tool === "opentofu") {
      const action = spec.action !== undefined ? spec.action : (template.spec || {}).action;
      if (action === "apply") chips.push(`<span class="wf-node-chip apply" data-chip="apply" title="${h("node_chip_apply")}">${h("node_chip_apply")}</span>`);
    }
    return chips.join("");
  }

  function joinBadge(node) {
    const count = predecessorCount(node);
    if (count < 2) return "";
    const all = node.convergence === "all";
    const label = t(all ? "all_of_n_predecessors" : "any_of_n_predecessors", { count });
    return `<span class="wf-join-badge ${all ? "all" : "any"}" title="${esc(label)}" aria-label="${esc(label)}">${h(all ? "join_badge_all" : "join_badge_any")}</span>`;
  }

  function nodeParamFields(node, template) {
    if (!template) return "";
    const over = node.overrides || {};
    const spec = { ...(template.spec || {}), ...(over.spec || {}) };
    const inventoryId = over.inventory_id !== undefined ? over.inventory_id : (template.inventory_id || "");
    const credentialId = over.credential_id !== undefined ? over.credential_id : (template.credential_id || "");
    const extra = over.extra_vars || template.extra_vars || {};
    let toolFields = "";
    if (template.tool === "ansible") {
      toolFields = `${inventoryField(lists.inventories, inventoryId, { id: "node-inventory" })}${ansibleFields(spec, {
        ids: {
          job_type: "node-job-type",
          verbosity: "node-verbosity",
          forks: "node-forks",
          diff: "node-diff",
          tags: "node-tags",
          skip_tags: "node-skip-tags",
          limit: "node-limit",
        },
      })}`;
    } else if (template.tool === "opentofu") {
      toolFields = tofuFields(spec, { ids: { action: "node-action" } });
    }
    return `${toolFields}
      ${credentialFields(lists.credentials, [credentialId], template.tool, { singleId: "node-credential" })}
      ${extraVarsField(extra, "node-extra")}`;
  }

  function nodeParams(node, template) {
    if (!template) return "";
    return `<details class="node-params" open>
      <summary>${h("parameters")}</summary>
      <p class="small muted">${h("applies_only_to_this_node_the_template_stays")}</p>
      ${nodeParamFields(node, template)}
    </details>`;
  }

  function edgeOffsets() {
    const groups = new Map();
    graph.edges.forEach((edge) => {
      const key = [edge.from, edge.to].sort().join("|");
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(edge);
    });
    const offsets = new Map();
    groups.forEach((list) => {
      list.forEach((edge, index) => {
        offsets.set(edge, (index - (list.length - 1) / 2) * 28);
      });
    });
    return offsets;
  }

  function edgeGeometry(from, to, when, offset) {
    const start = edgePoint(from, when);
    const end = { x: to.x - 2, y: to.y + 36 };
    if (end.x < start.x + 24) {
      const lane = Math.max(from.y + 96, to.y + 96) + 80 + offset;
      return { start, end, c1: { x: start.x + 80, y: lane }, c2: { x: end.x - 80, y: lane } };
    }
    const dx = Math.max(48, Math.abs(end.x - start.x) / 2);
    return { start, end, c1: { x: start.x + dx, y: start.y + offset }, c2: { x: end.x - dx, y: end.y + offset } };
  }

  function edgePath(geometry) {
    const { start, c1, c2, end } = geometry;
    return `M ${start.x} ${start.y} C ${c1.x} ${c1.y}, ${c2.x} ${c2.y}, ${end.x} ${end.y}`;
  }

  function edgeChip(geometry, klass, label) {
    const x = (geometry.start.x + 3 * geometry.c1.x + 3 * geometry.c2.x + geometry.end.x) / 8;
    const y = (geometry.start.y + 3 * geometry.c1.y + 3 * geometry.c2.y + geometry.end.y) / 8;
    const width = Math.max(56, label.length * 7 + 18);
    return `<g class="edge-chip" aria-hidden="true" pointer-events="none"><rect class="edge-chip-bg" x="${x - width / 2}" y="${y - 10}" width="${width}" height="20" rx="10" pointer-events="none"></rect><text class="edge-chip-text ${klass}" x="${x}" y="${y}" text-anchor="middle" dominant-baseline="central" pointer-events="none">${esc(label)}</text></g>`;
  }

  function paintEdgeChips() {
    svg.querySelectorAll(".edge-chip").forEach((chip) => {
      const text = chip.querySelector(".edge-chip-text");
      const rect = chip.querySelector(".edge-chip-bg");
      if (!text || !rect || typeof text.getComputedTextLength !== "function") return;
      const width = Math.max(56, text.getComputedTextLength() + 18);
      const x = Number(text.getAttribute("x"));
      rect.setAttribute("width", String(width));
      rect.setAttribute("x", String(x - width / 2));
    });
  }

  function drawEdges() {
    const offsets = edgeOffsets();
    const edges = graph.edges.map((edge, index) => {
      const from = graph.nodes.find((node) => node.id === edge.from);
      const to = graph.nodes.find((node) => node.id === edge.to);
      if (!from || !to) return null;
      const klass = ["success", "failure", "always"].includes(edge.when) ? edge.when : "always";
      const geometry = edgeGeometry(from, to, klass, offsets.get(edge) || 0);
      const issues = readonly ? [] : combinedEdgeIssues(index);
      const invalid = issues.some((issue) => issue.severity === "error");
      const warning = issues.some((issue) => issue.severity === "warning");
      const server = issues.some((issue) => issue.server);
      const reason = issues.map(issueReason).join(" · ");
      return { edge, index, from, to, klass, geometry, d: edgePath(geometry), invalid, warning, server, reason };
    }).filter(Boolean);
    const width = Math.max(640, ...graph.nodes.map((node) => (Number(node.x) || 0) + 260), ...edges.map(({ geometry }) => Math.max(geometry.start.x, geometry.end.x, geometry.c1.x, geometry.c2.x) + 20));
    const height = Math.max(400, ...graph.nodes.map((node) => (Number(node.y) || 0) + 140), ...edges.map(({ geometry }) => Math.max(geometry.start.y, geometry.end.y, geometry.c1.y, geometry.c2.y) + 20));
    board.style.width = `${width}px`;
    board.style.height = `${height}px`;
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    const defs = `<defs>${["success", "failure", "always"].map((when) => `<marker id="edge-arrow-${when}" viewBox="0 0 12 12" refX="10" refY="6" markerWidth="12" markerHeight="12" markerUnits="userSpaceOnUse" orient="auto"><polygon class="edge-arrow ${when}" points="0,0 12,6 0,12"></polygon></marker>`).join("")}</defs>`;
    svg.innerHTML = `${defs}${edges.map(({ edge, index, from, to, klass, geometry, d, invalid, warning, server, reason }) => {
      const label = `${edgeWhenText(klass)}: ${from.name} → ${to.name}`;
      const chip = `${PORT_SYMBOLS[klass]} ${edgeWhenText(klass)}`;
      const role = readonly ? ` role="img"` : ` role="button" tabindex="0"`;
      const runClass = readonly ? (edgeTaken(edge) ? " taken" : " untaken") : "";
      const classes = `edge-line ${klass}${runClass}${selectedEdge === index ? " selected" : ""}${invalid ? " invalid" : ""}${warning ? " warning" : ""}${server ? " server-invalid" : ""}`;
      const groupClass = `edge-group ${klass}${runClass}${invalid ? " invalid" : ""}`;
      const described = reason ? `${label} – ${reason}` : label;
      const invalidAttr = invalid ? ` aria-invalid="true"` : "";
      return `<g class="${groupClass}" data-edge-group="${index}">
        <path class="${classes}" data-edge="${index}"${role} aria-label="${esc(described)}"${invalidAttr} d="${d}" marker-end="url(#edge-arrow-${klass})"></path>
        <path class="edge-hit" data-edge="${index}" d="${d}"></path>
        ${edgeChip(geometry, klass, chip)}
      </g>`;
    }).join("")}`;
    paintEdgeChips();
    paintMinimap();
  }

  function focusEdge(index) {
    const path = svg.querySelector(`path[data-edge="${index}"]`);
    if (path) path.focus();
  }

  function selectEdge(index, focus) {
    selectedEdge = index;
    selected = null;
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    drawEdges();
    revealInspector();
    renderInspector();
    if (focus) focusEdge(index);
  }

  function removeEdgeAt(index) {
    const edge = graph.edges[index];
    if (!edge) return false;
    graph.edges.splice(index, 1);
    if (selectedEdge === index) selectedEdge = null;
    else if (selectedEdge !== null && selectedEdge > index) selectedEdge -= 1;
    recordHistory();
    markDirty();
    setHint("edge_removed");
    drawEdges();
    renderNodes();
    renderInspector();
    showToast("edge_deleted");
    return true;
  }

  function edgeExists(fromId, toId, when) {
    return graph.edges.some((edge) => edge.from === fromId && edge.to === toId && edge.when === when);
  }

  function setHint(key, vars) {
    const hint = document.querySelector("#wf-hint");
    if (hint) hint.textContent = t(key, vars);
  }

  function combinedNodeIssues(id) {
    return [
      ...(validation.nodeIssues[id] || []),
      ...serverIssues.filter((issue) => issue.nodes.includes(id)),
    ];
  }

  function combinedEdgeIssues(index) {
    return [
      ...(validation.edgeIssues[index] || []),
      ...serverIssues.filter((issue) => issue.edges.includes(index)),
    ];
  }

  function combinedIssues() {
    return [...validation.issues, ...serverIssues];
  }

  function issueReason(issue) {
    return t(issue.key, issue.vars);
  }

  function badgeLabel(issue) {
    if (issue.key === "template.missing_named" || issue.key === "workflow.node_template") return t("node_template_missing_badge");
    if (issue.key === "workflow.node_name") return t("node_name_missing_badge");
    if (issue.key === "workflow.node_duplicate") return t("node_duplicate_badge");
    if (issue.key === "node_is_unconnected") return t("node_unconnected_badge");
    if (issue.key === "node_is_unreachable") return t("node_unreachable_badge");
    return t("node_issue_badge");
  }

  function nodeBadge(node) {
    if (readonly) return "";
    const issues = combinedNodeIssues(node.id);
    if (!issues.length) return "";
    const primary = issues.find((issue) => issue.severity === "error") || issues[0];
    const reason = issues.map(issueReason).join(" · ");
    return `<span class="wf-node-badge ${primary.severity}" title="${esc(reason)}" aria-label="${esc(reason)}">${esc(badgeLabel(primary))}</span>`;
  }

  function summaryText(errors, warnings, issues) {
    if (!issues.length) return t("graph_is_valid");
    if (issues.length === 1) return issueReason(issues[0]);
    const parts = [];
    if (errors.length) parts.push(t("graph_error_count", { count: errors.length }));
    if (warnings.length) parts.push(t("graph_warning_count", { count: warnings.length }));
    return parts.join(" · ");
  }

  function paintValidation() {
    const issues = combinedIssues();
    const errors = issues.filter((issue) => issue.severity === "error");
    const warnings = issues.filter((issue) => issue.severity === "warning");
    const summary = document.querySelector("#wf-validation");
    const launchButton = document.querySelector("#wf-launch");
    if (summary) {
      summary.textContent = summaryText(errors, warnings, issues);
      summary.classList.toggle("error", errors.length > 0);
      summary.classList.toggle("warn", errors.length === 0 && warnings.length > 0);
      if (issues.length) summary.setAttribute("title", issues.map(issueReason).join(" · "));
      else summary.removeAttribute("title");
    }
    if (launchButton) {
      launchButton.disabled = errors.length > 0;
      if (errors.length) launchButton.setAttribute("title", issueReason(errors[0]));
      else launchButton.removeAttribute("title");
    }
    paintDirty();
  }

  function recomputeValidation() {
    validation = analyzeGraph(graph, templates);
    paintValidation();
  }

  function paintDirty() {
    const saveButton = document.querySelector("#wf-save");
    const launchButton = document.querySelector("#wf-launch");
    const status = document.querySelector("#wf-status");
    if (saveButton) saveButton.classList.toggle("primary", dirty);
    if (launchButton) launchButton.classList.toggle("primary", !dirty && !launchButton.disabled);
    if (status) status.textContent = t(dirty ? "unsaved_changes" : "all_changes_saved");
  }

  function setDirty(next) {
    dirty = Boolean(next);
    paintDirty();
    appState.setLeaveGuard(dirty ? leaveGuard : null);
  }

  function markDirty() {
    if (!editable) return;
    serverIssues = [];
    recomputeValidation();
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

  function openAddNode(point, link) {
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
    }, point, link);
  }

  function addNode(draft, point, link) {
    const spot = point
      ? { x: Math.max(0, Math.round(point.x - 20)), y: Math.max(0, Math.round(point.y - 20)) }
      : freeNodePosition();
    const node = { ...draft, x: spot.x, y: spot.y };
    graph.nodes.push(node);
    if (link && link.from && link.when) {
      const from = graph.nodes.find((item) => item.id === link.from);
      if (from && from.id !== node.id && !edgeExists(from.id, node.id, link.when)) {
        graph.edges.push({ from: from.id, to: node.id, when: link.when });
        setHint("edge_added", { when: edgeWhenText(link.when), from: from.name, to: node.name });
      }
    }
    recordHistory();
    markDirty();
    selected = node.id;
    selectedEdge = null;
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    renderNodes();
    revealInspector();
    renderInspector();
    const created = nodes.querySelector(`[data-id="${CSS.escape(node.id)}"]`);
    if (created) created.focus();
  }

  function removeNode(node) {
    if (!editable) return;
    if (!graph.nodes.some((item) => item.id === node.id)) return;
    graph.nodes = graph.nodes.filter((item) => item.id !== node.id);
    graph.edges = graph.edges.filter((edge) => edge.from !== node.id && edge.to !== node.id);
    recordHistory();
    markDirty();
    selected = null;
    selectedEdge = null;
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    renderNodes();
    renderInspector();
    showToast("node_deleted");
    canvas.focus();
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

  function applyNodeEdit(node, draft) {
    const before = snapshotGraph();
    node.name = draft.name;
    node.template_id = draft.template_id;
    node.convergence = draft.convergence;
    if (draft.overrides === undefined) delete node.overrides;
    else node.overrides = draft.overrides;
    if (!sameJson(before, snapshotGraph())) {
      recordHistory();
      markDirty();
    }
    renderNodes();
    renderInspector();
  }

  function openNodeEditor(node, point, link) {
    if (!editable) return;
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
      title: t(adding ? "add_node" : "edit_node"),
      submit: t(adding ? "create" : "save"),
      body: nodeEditorFields(node),
      onSubmit: ({ form, close }) => {
        if (!flushChips(form)) return;
        const name = form.querySelector("#node-name").value.trim();
        if (!name) throw new Error("name_is_missing_or_too_long");
        const templateId = form.querySelector("#node-template").value;
        if (!templates.some((item) => item.id === templateId)) throw new Error("create_a_template_or_a_workflow_first");
        const convergence = form.querySelector("#node-join").value === "all" ? "all" : "any";
        const draft = draftFor(templateId, name, convergence);
        readOverrides(draft, form);
        if (adding) addNode(draft, point, link);
        else applyNodeEdit(node, draft);
        close();
      },
    });
    let currentTemplateId = node.template_id;
    const bindTemplate = () => {
      bindChips(dialog.form);
      const select = dialog.form.querySelector("#node-template");
      if (!select) return;
      select.addEventListener("change", (event) => {
        const next = templates.find((item) => item.id === event.target.value);
        if (!next) return;
        if (!flushChips(dialog.form)) {
          event.target.value = currentTemplateId;
          return;
        }
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
        currentTemplateId = next.id;
        bindTemplate();
      });
    };
    bindTemplate();
  }

  function startConnect(node) {
    if (!editable) return;
    connecting = { from: node.id };
    connectDrag = null;
    selected = node.id;
    selectedEdge = null;
    setHint("connect_click_or_drag_target", { name: node.name });
    renderNodes();
    revealInspector();
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
    if (wouldCreateCycle(graph, fromNode.id, toNode.id)) {
      setHint("connection_would_create_a_cycle");
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
    recordHistory();
    markDirty();
    selected = fromNode.id;
    selectedEdge = null;
    setHint("edge_added", { when: edgeWhenText(when), from: fromNode.name, to: toNode.name });
    renderNodes();
    revealInspector();
    renderInspector();
  }

  function connectToNew(node, point) {
    if (!editable) return;
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    renderNodes();
    setHint("drag_nodes_and_use_the_canvas_menus");
    openAddNode(point, { from: node.id, when: "success" });
  }

  function editNode(node) {
    if (!editable) return;
    selected = node.id;
    selectedEdge = null;
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    renderNodes();
    revealInspector();
    renderInspector();
    openNodeEditor(node);
  }

  const nodeMenuActions = {
    edit: editNode,
    remove: removeNode,
    connect: startConnect,
  };

  function nodeLabel(node) {
    const template = templates.find((item) => item.id === node.template_id);
    const parts = [node.name];
    if (template) {
      parts.push(template.name);
      const key = toolKey(template.tool);
      if (key) parts.push(t(key));
    }
    if (readonly) {
      const phase = phases[node.id];
      const status = phase ? phase.phase : "pending";
      parts.push(statusLabel(status));
      const duration = spanText(phaseStart(phase), phase && phase.finished_at, status);
      if (duration !== "—") parts.push(duration);
    }
    return parts.filter(Boolean).join(" · ");
  }

  function phaseStart(phase) {
    if (!phase) return "";
    return phase.started_at || (phase.phase === "running" ? phase.job_created_at : "") || "";
  }

  function statusMark(status) {
    return {
      successful: "✓",
      failed: "✕",
      skipped: "⊘",
      running: "",
      pending: "○",
    }[status] || "○";
  }

  function nodeStatusInner(phase) {
    const status = phase ? phase.phase : "pending";
    const icon = status === "running"
      ? `<span class="wf-node-spinner" aria-hidden="true"></span>`
      : `<span class="wf-node-state-icon" aria-hidden="true">${statusMark(status)}</span>`;
    const duration = spanText(phaseStart(phase), phase && phase.finished_at, status);
    return `${icon}<span class="wf-node-status-text">${esc(statusLabel(status))}</span><span class="mono wf-node-duration">${esc(duration)}</span>`;
  }

  function nodeStatus(phase) {
    const status = phase ? phase.phase : "pending";
    return `<span class="wf-node-status ${esc(status)}" data-status="${esc(status)}">${nodeStatusInner(phase)}</span>`;
  }

  function paintNodeStatus(element) {
    const phase = phases[element.dataset.id];
    const status = phase ? phase.phase : "pending";
    const host = element.querySelector(".wf-node-status");
    if (!host) return;
    host.className = `wf-node-status ${status}`;
    host.dataset.status = status;
    host.innerHTML = nodeStatusInner(phase);
  }

  function edgeTaken(edge) {
    const phase = phases[edge.from];
    if (!phase) return false;
    if (edge.when === "success") return phase.phase === "successful";
    if (edge.when === "failure") return phase.phase === "failed";
    if (edge.when === "always") return phase.phase === "successful" || phase.phase === "failed";
    return false;
  }

  function handledFailureCount() {
    return graph.nodes.filter((node) => {
      const phase = phases[node.id];
      return phase && phase.phase === "failed" && graph.edges.some((edge) => edge.from === node.id && (edge.when === "failure" || edge.when === "always"));
    }).length;
  }

  function runSummary() {
    if (workflow.status !== "successful") return "";
    const handled = handledFailureCount();
    if (!handled) return "";
    return `<p class="small run-summary" id="run-summary">${h(handled === 1 ? "successful_one_failure_handled" : "successful_failures_handled", { count: handled })}</p>`;
  }

  function runLegend() {
    return `<ul class="run-legend">
      <li><span class="run-legend-icon successful" aria-hidden="true">✓</span>${h("successful")}</li>
      <li><span class="run-legend-icon failed" aria-hidden="true">✕</span>${h("failed")}</li>
      <li><span class="run-legend-icon running" aria-hidden="true"><span class="wf-node-spinner"></span></span>${h("running")}</li>
      <li><span class="run-legend-icon skipped" aria-hidden="true">⊘</span>${h("skipped")}</li>
      <li><span class="run-legend-edge taken" aria-hidden="true"></span>${h("taken_path")}</li>
      <li><span class="run-legend-edge untaken" aria-hidden="true"></span>${h("not_taken_path")}</li>
    </ul>`;
  }

  function renderNodes() {
    if (!readonly) recomputeValidation();
    const empty = document.querySelector("#wf-empty");
    if (empty) empty.hidden = graph.nodes.length > 0;
    nodes.innerHTML = graph.nodes.map((node) => {
      const phase = phases[node.id];
      const phaseClass = phase ? ` phase-${esc(phase.phase)}` : "";
      const selectedClass = selected === node.id ? " selected" : "";
      const runClass = readonly ? " run-node" : "";
      const connectClass = connecting && connecting.from === node.id ? " connecting" : "";
      const nodeIssues = readonly ? [] : combinedNodeIssues(node.id);
      const issueClass = nodeIssues.some((issue) => issue.severity === "error")
        ? " invalid"
        : nodeIssues.some((issue) => issue.severity === "warning") ? " warning" : "";
      const menu = editable ? `<button class="btn ghost compact node-menu-button" type="button" data-node-menu aria-haspopup="menu" aria-expanded="false" aria-label="${h("node_actions", { name: node.name })}">⋯</button>` : "";
      const template = templates.find((item) => item.id === node.template_id);
      const ports = `<div class="wf-ports">${["success", "failure", "always"].map((when) => {
        const add = editable ? `<button class="wf-port-add" type="button" data-add-port="${when}" aria-label="${h("add_step_after", { name: node.name, when: edgeWhenText(when) })}" title="${h("add_step_after", { name: node.name, when: edgeWhenText(when) })}">+</button>` : "";
        return `<div class="wf-port ${when}"><span class="wf-port-symbol" aria-hidden="true">${PORT_SYMBOLS[when]}</span>${add}</div>`;
      }).join("")}</div>`;
      return `<div class="wf-node${selectedClass}${connectClass}${phaseClass}${runClass}${issueClass}" data-id="${esc(node.id)}" style="left:${Number(node.x) || 0}px;top:${Number(node.y) || 0}px" tabindex="0" role="group" aria-label="${esc(nodeLabel(node))}">${joinBadge(node)}${menu}<strong>${esc(node.name)}</strong><span class="wf-node-meta">${nodeMeta(template)}</span>${readonly ? nodeStatus(phase) : ""}${nodeBadge(node)}<span class="wf-node-chips">${nodeChips(node, template)}</span>${ports}</div>`;
    }).join("");
    nodes.querySelectorAll(".wf-node").forEach((element) => {
      const node = graph.nodes.find((item) => item.id === element.dataset.id);
      const menuButton = element.querySelector("[data-node-menu]");
      element.addEventListener("pointerdown", (event) => {
        if (event.target.closest(".node-menu-button, .wf-port")) return;
        if (readonly) {
          if (onNode) onNode(node, phases[node.id] || null, element);
          return;
        }
        if (!editable) {
          selected = node.id;
          selectedEdge = null;
          revealInspector();
          renderInspector();
          nodes.querySelectorAll(".wf-node").forEach((item) => item.classList.toggle("selected", item.dataset.id === selected));
          return;
        }
        if (connecting) {
          if (node.id === connecting.from) {
            event.preventDefault();
            connectDrag = { id: node.id, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, moved: false };
            element.setPointerCapture(event.pointerId);
            drawConnectPreview(node, localPoint(event));
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          finishConnect(node.id);
          return;
        }
        selected = node.id;
        selectedEdge = null;
        const point = localPoint(event);
        drag = { id: node.id, ox: point.x - node.x, oy: point.y - node.y, screen: canvasPoint(event) };
        element.setPointerCapture(event.pointerId);
        revealInspector();
        renderInspector();
        nodes.querySelectorAll(".wf-node").forEach((item) => item.classList.toggle("selected", item.dataset.id === selected));
      });
      element.addEventListener("pointermove", (event) => {
        if (connectDrag && connectDrag.id === node.id) {
          if (Math.hypot(event.clientX - connectDrag.startX, event.clientY - connectDrag.startY) > 4) connectDrag.moved = true;
          drawConnectPreview(node, localPoint(event));
          return;
        }
        if (!drag || drag.id !== node.id) return;
        const point = localPoint(event);
        node.x = Math.max(0, snapToGrid(point.x - drag.ox));
        node.y = Math.max(0, snapToGrid(point.y - drag.oy));
        element.style.left = `${node.x}px`;
        element.style.top = `${node.y}px`;
        drag.screen = canvasPoint(event);
        drawEdges();
        markDirty();
        scheduleAutoPan();
      });
      element.addEventListener("pointerup", (event) => {
        if (connectDrag && connectDrag.id === node.id) {
          const moved = connectDrag.moved;
          connectDrag = null;
          clearConnectPreview();
          const target = document.elementFromPoint(event.clientX, event.clientY);
          const hit = target && target.closest(".wf-node");
          if (hit && (hit.dataset.id !== node.id || moved)) finishConnect(hit.dataset.id);
          else if (!hit && (!target || (target.closest("#canvas") && !target.closest(".canvas-controls, .minimap")))) connectToNew(node, localPoint(event));
          else setHint("connect_click_or_drag_target", { name: node.name });
          return;
        }
        stopAutoPan();
        if (drag && drag.id === node.id) {
          node.x = Math.max(0, snapToGrid(node.x));
          node.y = Math.max(0, snapToGrid(node.y));
          element.style.left = `${node.x}px`;
          element.style.top = `${node.y}px`;
          drawEdges();
        }
        recordHistory();
        drag = null;
      });
      element.addEventListener("pointercancel", () => {
        connectDrag = null;
        clearConnectPreview();
        stopAutoPan();
        if (drag) recordHistory();
        drag = null;
      });
      element.addEventListener("contextmenu", (event) => {
        if (!editable) return;
        event.preventDefault();
        event.stopPropagation();
        openNodeMenu(node, event.clientX, event.clientY, element, nodeMenuActions);
      });
      element.addEventListener("dblclick", (event) => {
        if (!editable || event.target.closest(".node-menu-button, .wf-port")) return;
        event.preventDefault();
        event.stopPropagation();
        editNode(node);
      });
      if (menuButton) {
        menuButton.addEventListener("pointerdown", (event) => event.stopPropagation());
        menuButton.addEventListener("click", (event) => {
          event.stopPropagation();
          const rect = menuButton.getBoundingClientRect();
          openNodeMenu(node, rect.left, rect.bottom + 4, menuButton, nodeMenuActions);
        });
      }
      element.querySelectorAll("[data-add-port]").forEach((button) => {
        button.addEventListener("pointerdown", (event) => event.stopPropagation());
        button.addEventListener("click", (event) => {
          event.stopPropagation();
          const when = button.dataset.addPort;
          const offset = PORT_OFFSETS[when] || PORT_OFFSETS.success;
          openAddNode({ x: node.x + 260, y: node.y + offset }, { from: node.id, when });
        });
      });
      element.addEventListener("keydown", (event) => {
        if (event.target !== element) return;
        historyShortcut(event);
        if (event.defaultPrevented) return;
        layoutShortcut(event);
        if (event.defaultPrevented) return;
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
          if (event.key === "Enter") {
            editNode(node);
            return;
          }
          selected = node.id;
          selectedEdge = null;
          revealInspector();
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
        const step = event.shiftKey ? GRID_SIZE * 4 : GRID_SIZE;
        const delta = {
          ArrowLeft: [-step, 0],
          ArrowRight: [step, 0],
          ArrowUp: [0, -step],
          ArrowDown: [0, step],
        }[event.key];
        if (!delta) return;
        event.preventDefault();
        node.x = Math.max(0, snapToGrid((Number(node.x) || 0) + delta[0]));
        node.y = Math.max(0, snapToGrid((Number(node.y) || 0) + delta[1]));
        element.style.left = `${node.x}px`;
        element.style.top = `${node.y}px`;
        drawEdges();
        recordHistory();
        markDirty();
      });
    });
    drawEdges();
  }

  function edgeWhenText(when) {
    return t(edgeWhenLabel(when));
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

  function readOverrides(node, root) {
    const template = templates.find((item) => item.id === node.template_id);
    const extraField = root.querySelector("#node-extra");
    if (!template || !extraField) return;
    const fail = (key, vars) => {
      const error = new Error(key);
      if (vars) error.vars = vars;
      throw error;
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
        if (!Number.isInteger(forks)) fail("forks_is_a_whole_number");
        if (forks < 1 || forks > 50) fail("forks_is_a_number_from_1_to_50");
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
    const extra = jsonObject(extraField.value, "extra_variables");
    if (!sameJson(extra, template.extra_vars || {})) over.extra_vars = extra;
    if (Object.keys(over).length) node.overrides = over;
    else delete node.overrides;
  }

  function detailRow(label, value) {
    return `<dt>${esc(label)}</dt><dd>${value}</dd>`;
  }

  function detailList(rows) {
    return `<dl class="kv readonly-kv">${rows.join("")}</dl>`;
  }

  function nodeNameById(id) {
    const node = graph.nodes.find((item) => item.id === id);
    return node ? node.name : id;
  }

  function inventoryLabel(id) {
    if (!id) return t("no_inventory");
    const inventory = lists.inventories.find((item) => item.id === id);
    return inventory ? inventory.name : id;
  }

  function credentialLabel(id) {
    if (!id) return t("none");
    const credential = lists.credentials.find((item) => item.id === id);
    return credential ? `${credential.name} (${credentialKindLabel(credential.kind)})` : id;
  }

  function viewerParamRows(node, template) {
    if (!template) return `<p class="small muted">${h("node_template_missing_badge")}</p>`;
    const over = node.overrides || {};
    const spec = { ...(template.spec || {}), ...(over.spec || {}) };
    const inventoryId = over.inventory_id !== undefined ? over.inventory_id : (template.inventory_id || "");
    const credentialId = over.credential_id !== undefined ? over.credential_id : (template.credential_id || "");
    const extra = over.extra_vars || template.extra_vars || {};
    const rows = [];
    if (template.tool === "ansible") {
      rows.push(detailRow(t("inventory"), esc(inventoryLabel(inventoryId))));
      rows.push(detailRow(t("type"), esc(spec.job_type === "check" ? t("check_dry_run") : t("run_2"))));
      rows.push(detailRow(t("verbosity"), esc(t(verbosityLabel(Number(spec.verbosity) || 0)))));
      rows.push(detailRow(t("forks"), esc(spec.forks ?? t("none"))));
      rows.push(detailRow(t("diff"), esc(spec.diff ? t("yes") : t("no"))));
      rows.push(detailRow(t("tags"), esc(spec.tags || t("none"))));
      rows.push(detailRow(t("skip_tags"), esc(spec.skip_tags || t("none"))));
      rows.push(detailRow(t("limit"), esc(spec.limit || t("none"))));
    } else if (template.tool === "opentofu") {
      rows.push(detailRow(t("action"), esc(spec.action === "apply" ? t("apply") : "plan")));
    }
    rows.push(detailRow(t("nav.access"), esc(credentialLabel(credentialId))));
    rows.push(`<dt>${esc(t("extra_variables_json"))}</dt><dd><pre class="vars">${esc(JSON.stringify(extra, null, 2))}</pre></dd>`);
    return detailList(rows);
  }

  function viewerConnections(node) {
    const edgeRow = (when, other) => {
      const kind = ["success", "failure", "always"].includes(when) ? when : "always";
      return `<div class="edge-row readonly-edge"><span class="pill ${kind}">${h(edgeWhenLabel(kind))}</span><span class="readonly-target">${esc(other)}</span></div>`;
    };
    const outgoing = graph.edges.filter((edge) => edge.from === node.id).map((edge) => edgeRow(edge.when, nodeNameById(edge.to)));
    const incoming = graph.edges.filter((edge) => edge.to === node.id).map((edge) => edgeRow(edge.when, nodeNameById(edge.from)));
    const group = (label, rows, direction) => `<p class="small muted">${h(label)}</p>${rows.length ? `<div class="edge-list" data-direction="${direction}">${rows.join("")}</div>` : `<p class="small muted">${h("none")}</p>`}`;
    return `${group("outgoing", outgoing, "outgoing")}${group("incoming", incoming, "incoming")}`;
  }

  function openEdgeEditor(index) {
    const edge = graph.edges[index];
    if (!editable || !edge) return;
    const dialog = openDialog({
      id: "edge-edit-dialog",
      title: t("edit_edge"),
      body: `<p class="edge-ends"><span class="mono">${esc(nodeNameById(edge.from))}</span><span aria-hidden="true">→</span><span class="mono">${esc(nodeNameById(edge.to))}</span></p>
        <label class="field"><span>${h("when")}</span>
          <select class="fit" id="edge-when">
            ${["success", "failure", "always"].map((when) => `<option value="${when}"${edge.when === when ? " selected" : ""}>${h(edgeWhenLabel(when))}</option>`).join("")}
          </select>
        </label>`,
      danger: `<button class="btn danger" type="button" data-edge-delete>${h("delete_edge")}</button>`,
      onSubmit: ({ form, close }) => {
        const when = form.querySelector("#edge-when").value;
        if (when !== edge.when && graph.edges.some((item, position) => position !== index && item.from === edge.from && item.to === edge.to && item.when === when)) {
          throw new Error("the_same_edge_is_present_twice");
        }
        if (when !== edge.when) {
          edge.when = when;
          recordHistory();
          markDirty();
          setHint("edge_changed");
          drawEdges();
          renderInspector();
        }
        close();
      },
    });
    dialog.overlay.querySelector("[data-edge-delete]").addEventListener("click", () => {
      dialog.clean();
      dialog.close();
      if (removeEdgeAt(index)) canvas.focus();
    });
  }

  function renderDetailsInspector(host) {
    const badge = editable ? "" : `<span class="pill read-only">${h("read_only")}</span>`;
    const viewerHint = `<p class="small muted">${esc(readOnlyHint())}</p>`;
    if (selectedEdge !== null && graph.edges[selectedEdge]) {
      const edge = graph.edges[selectedEdge];
      const when = ["success", "failure", "always"].includes(edge.when) ? edge.when : "always";
      const hint = editable ? `<p class="small muted">${h("edge_details_hint")}</p>` : viewerHint;
      const edit = editable ? `<button class="btn" id="edge-edit" type="button">${h("edit_edge")}</button>` : "";
      host.innerHTML = `<div class="inspector-block">
        <div class="spread"><h2>${h("edge")}</h2>${badge}</div>
        <p class="edge-ends"><span class="mono">${esc(nodeNameById(edge.from))}</span><span aria-hidden="true">→</span><span class="mono">${esc(nodeNameById(edge.to))}</span></p>
        ${detailList([detailRow(t("when"), `<span class="pill ${when}">${h(edgeWhenLabel(when))}</span>`)])}
        ${edit}
        ${hint}
      </div>`;
      const editButton = host.querySelector("#edge-edit");
      if (editButton) editButton.addEventListener("click", () => openEdgeEditor(selectedEdge));
      return;
    }
    const node = graph.nodes.find((item) => item.id === selected);
    if (!node) {
      host.innerHTML = `<div class="inspector-block">
        <div class="spread"><h2>${h("view")}</h2>${badge}</div>
        <p class="small muted">${h("select_a_node_or_an_edge_to_see_its_details")}</p>
        ${editable ? "" : viewerHint}
      </div>`;
      return;
    }
    const template = templates.find((item) => item.id === node.template_id);
    const tool = template ? toolKey(template.tool) : "";
    const hint = editable ? `<p class="small muted">${h("node_details_hint")}</p>` : viewerHint;
    host.innerHTML = `<div class="node-panel">
      <div class="node-panel-head">
        <div class="spread"><h2>${esc(node.name)}</h2>${badge}</div>
        ${hint}
      </div>
      <details class="node-section" open>
        <summary>${h("general")}</summary>
        <div class="node-section-body">
          ${detailList([
            detailRow(t("name"), esc(node.name)),
            detailRow(t("template"), esc(template ? template.name : t("node_template_missing_badge"))),
            detailRow(t("tool"), esc(tool ? t(tool) : t("none"))),
            detailRow(t("convergence"), esc(t(node.convergence === "all" ? "all_predecessors" : "any_predecessor"))),
          ])}
          ${predecessorCount(node) < 2 ? `<p class="small muted">${h("convergence_needs_two_predecessors")}</p>` : ""}
        </div>
      </details>
      <details class="node-section" open>
        <summary>${h("parameters")}</summary>
        <div class="node-section-body">${viewerParamRows(node, template)}</div>
      </details>
      <details class="node-section" open>
        <summary>${h("connections")}</summary>
        <div class="node-section-body">${viewerConnections(node)}</div>
      </details>
    </div>`;
  }

  function renderInspector() {
    const host = document.querySelector("#inspector");
    if (readonly) {
      host.innerHTML = `<h2>${h("run_2")}</h2><p class="small"><span id="run-state" class="pill ${esc(workflow.status || "pending")}">${esc(statusLabel(workflow.status || "pending"))}</span></p>${runSummary()}${runLegend()}`;
      return;
    }
    renderDetailsInspector(host);
  }

  svg.addEventListener("click", (event) => {
    const path = event.target.closest("path");
    if (!path || readonly) return;
    selectEdge(Number(path.dataset.edge), true);
  });
  svg.addEventListener("keydown", (event) => {
    const path = event.target.closest("path");
    if (!path || readonly) return;
    historyShortcut(event);
    if (event.defaultPrevented) return;
    const index = Number(path.dataset.edge);
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      selectEdge(index, true);
      return;
    }
    if (!editable) return;
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
    if (editable) {
      selected = null;
      selectedEdge = null;
      renderNodes();
      renderInspector();
    }
    if (event.pointerType === "touch") {
      touchPointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
      if (touchPointers.size >= 2) {
        pan = null;
        canvas.classList.remove("panning");
        const [first, second] = [...touchPointers.values()];
        pinch = {
          distance: Math.hypot(first.x - second.x, first.y - second.y),
          view: viewport.view,
        };
        return;
      }
    }
    pan = { pointerId: event.pointerId, x: event.clientX, y: event.clientY, viewX: viewport.view.x, viewY: viewport.view.y };
    if (canvas.setPointerCapture) canvas.setPointerCapture(event.pointerId);
    canvas.classList.add("panning");
  }

  function canvasPointerMove(event) {
    if (event.pointerType === "touch" && touchPointers.has(event.pointerId)) {
      touchPointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
      if (pinch && touchPointers.size >= 2) {
        const [first, second] = [...touchPointers.values()];
        const distance = Math.hypot(first.x - second.x, first.y - second.y);
        if (pinch.distance > 0) {
          const rect = canvas.getBoundingClientRect();
          const center = { x: (first.x + second.x) / 2 - rect.left, y: (first.y + second.y) / 2 - rect.top };
          viewport.set(zoomAt(pinch.view, distance / pinch.distance, center));
        }
        return;
      }
    }
    if (!pan || pan.pointerId !== event.pointerId) return;
    viewport.set({
      x: pan.viewX + (event.clientX - pan.x),
      y: pan.viewY + (event.clientY - pan.y),
      scale: viewport.view.scale,
    });
  }

  function endPan(event) {
    if (event.pointerType === "touch") touchPointers.delete(event.pointerId);
    if (touchPointers.size < 2) pinch = null;
    if (!pan || (event.pointerId !== undefined && pan.pointerId !== event.pointerId)) return;
    pan = null;
    canvas.classList.remove("panning");
  }

  function canvasContextMenu(event) {
    if (!editable) return;
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
  const autoLayoutButton = document.querySelector("#wf-auto-layout");
  if (autoLayoutButton) autoLayoutButton.addEventListener("click", autoLayoutGraph);
  const shortcutsButton = document.querySelector("#wf-shortcuts");
  if (shortcutsButton) shortcutsButton.addEventListener("click", openShortcuts);
  const undoButton = document.querySelector("#wf-undo");
  if (undoButton) undoButton.addEventListener("click", undo);
  const redoButton = document.querySelector("#wf-redo");
  if (redoButton) redoButton.addEventListener("click", redo);

  if (inspectorToggle) {
    inspectorToggle.addEventListener("click", () => setInspectorCollapsed(!inspectorCollapsed, false));
  }

  canvas.addEventListener("keydown", (event) => {
    if (event.target !== canvas) return;
    const key = String(event.key);
    if (!event.ctrlKey && !event.metaKey && !event.altKey) {
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
    }
    if (!editable) return;
    if (event.key === "Escape" && connecting) {
      event.preventDefault();
      cancelConnect();
      return;
    }
    if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
    event.preventDefault();
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
      runs: document.querySelector("#wf-panel-runs"),
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

  baseline = currentState();
  renderNodes();
  renderInspector();
  if (narrowScreen()) setInspectorCollapsed(true, false);
  if (narrowQuery) {
    narrowQuery.addEventListener("change", (event) => {
      if (!editorBody || !editorBody.isConnected) return;
      setInspectorCollapsed(event.matches, false);
    });
  }
  fitOnOpen();
  bindWorkflowTabs();
  mountSchedules(document.querySelector("#schedule-card"));
  paintDirty();
  updateHistoryButtons();
  if (editable) appState.setLeaveGuard(leaveGuard);

  const addFirst = document.querySelector("#wf-add-first");
  if (addFirst) {
    addFirst.addEventListener("click", () => openAddNode(viewport.screenToGraph(centerPoint())));
  }

  const nameField = document.querySelector("#wf-name");
  if (nameField) {
    nameField.addEventListener("input", () => {
      markDirty();
      document.title = `${nameField.value} – ${t("workflow")} – Koreograf`;
    });
  }
  const descriptionField = document.querySelector("#wf-description");
  if (descriptionField) {
    descriptionField.addEventListener("input", () => {
      descriptionField.setAttribute("title", descriptionField.value);
      markDirty();
    });
  }

  function applyWorkflow(current) {
    const nextGraph = current.graph || { nodes: [], edges: [] };
    workflow.name = current.name;
    workflow.description = current.description;
    workflow.graph = JSON.parse(JSON.stringify(nextGraph));
    if (typeof current.version === "number") workflow.version = current.version;
    graph.nodes = (nextGraph.nodes || []).map((node) => ({ ...node }));
    graph.edges = (nextGraph.edges || []).map((edge) => ({ ...edge }));
    const nameField = document.querySelector("#wf-name");
    if (nameField) nameField.value = current.name;
    const descriptionField = document.querySelector("#wf-description");
    if (descriptionField) {
      descriptionField.value = current.description;
      descriptionField.setAttribute("title", current.description);
    }
    selected = null;
    selectedEdge = null;
    connecting = null;
    connectDrag = null;
    clearConnectPreview();
    serverIssues = [];
    historyStack = [snapshotGraph()];
    historyIndex = 0;
    baseline = currentState();
    recomputeValidation();
    renderNodes();
    renderInspector();
    drawEdges();
    setDirty(false);
    updateHistoryButtons();
  }

  async function reloadWorkflow() {
    try {
      const current = await api("GET", `/api/v1/workflows/${encodeURIComponent(workflow.id)}`);
      applyWorkflow(current);
      return true;
    } catch (error) {
      handle(error);
      return false;
    }
  }

  function chooseConflictAction() {
    return new Promise((resolve) => {
      let settled = false;
      const decide = (value) => {
        if (settled) return;
        settled = true;
        resolve(value);
      };
      const dialog = openDialog({
        id: "workflow-conflict-dialog",
        title: t("workflow_changed_title"),
        body: `<p class="small muted">${h("workflow_changed_hint")}</p><div class="choice-row"><button class="btn pill primary" type="button" data-conflict-reload>${h("reload")}</button><button class="btn" type="button" data-conflict-overwrite>${h("overwrite")}</button><button class="btn" type="button" data-conflict-cancel>${h("cancel")}</button></div>`,
        form: false,
        actions: false,
        onClose: () => decide("cancel"),
      });
      dialog.overlay.querySelector("[data-conflict-reload]").addEventListener("click", () => {
        decide("reload");
        dialog.close();
      });
      dialog.overlay.querySelector("[data-conflict-overwrite]").addEventListener("click", () => {
        decide("overwrite");
        dialog.close();
      });
      dialog.overlay.querySelector("[data-conflict-cancel]").addEventListener("click", () => dialog.close());
    });
  }

  async function saveGraph(overwrite = false) {
    const saveButton = document.querySelector("#wf-save");
    if (saveButton) saveButton.disabled = true;
    try {
      const name = document.querySelector("#wf-name").value;
      const description = document.querySelector("#wf-description").value;
      if (overwrite) {
        const current = await api("GET", `/api/v1/workflows/${encodeURIComponent(workflow.id)}`);
        if (typeof current.version === "number") workflow.version = current.version;
      }
      const body = { name, description, graph };
      if (typeof workflow.version === "number") body.version = workflow.version;
      const saved = await api("PUT", `/api/v1/workflows/${encodeURIComponent(workflow.id)}`, body);
      if (saved && typeof saved.version === "number") workflow.version = saved.version;
      workflow.name = name;
      workflow.description = description;
      workflow.graph = JSON.parse(JSON.stringify(graph));
      baseline = currentState();
      serverIssues = [];
      recomputeValidation();
      setDirty(false);
      return true;
    } catch (error) {
      if (error && (error.status === 409 || error.status === 412)) {
        const choice = await chooseConflictAction();
        if (choice === "reload") {
          await reloadWorkflow();
          return false;
        }
        if (choice === "overwrite") return saveGraph(true);
        return false;
      }
      const issues = issuesForServerError(graph, error && error.message, error && error.vars);
      serverIssues = issues;
      if (issues.length) {
        renderNodes();
        renderInspector();
        const primary = issues.find((issue) => issue.severity === "error") || issues[0];
        setHint(primary.key, primary.vars);
      } else {
        recomputeValidation();
      }
      handle(error);
      return false;
    } finally {
      if (saveButton) saveButton.disabled = false;
    }
  }

  async function saveWorkflow() {
    if (!editable) return false;
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
      if (launch.disabled) return;
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
  if (editorStack) {
    editorStack.addEventListener("keydown", saveShortcut);
    editorStack.addEventListener("keydown", historyShortcut);
    editorStack.addEventListener("keydown", layoutShortcut);
  }
  canvas.addEventListener("keydown", saveShortcut);
  canvas.addEventListener("keydown", historyShortcut);
  canvas.addEventListener("keydown", layoutShortcut);

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

  function applyPhases(status) {
    if (status) workflow.status = status;
    const currentStatus = status || workflow.status || "pending";
    document.querySelectorAll("#run-state, #run-state-top").forEach((badge) => {
      badge.className = `pill ${currentStatus}`;
      badge.textContent = statusLabel(currentStatus);
    });
    nodes.querySelectorAll(".wf-node").forEach((element) => {
      const node = graph.nodes.find((item) => item.id === element.dataset.id);
      const phase = phases[element.dataset.id];
      const phaseClass = phase ? ` phase-${phase.phase}` : "";
      element.className = `wf-node run-node${phaseClass}`;
      if (node) element.setAttribute("aria-label", nodeLabel(node));
      paintNodeStatus(element);
    });
    drawEdges();
    renderInspector();
  }

  return { applyPhases, viewport };
}
