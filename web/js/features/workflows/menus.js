import { esc, h } from "../../core/dom.js";
import { openDialog } from "../../core/dialog.js";
import { t } from "../../core/i18n.js";

let nodeMenu = null;

function closeNodeMenu(restoreFocus) {
  if (!nodeMenu) return;
  const opener = nodeMenu.opener;
  nodeMenu.element.remove();
  document.removeEventListener("pointerdown", nodeMenu.outside, true);
  document.removeEventListener("keydown", nodeMenu.keys, true);
  nodeMenu = null;
  if (opener && opener.matches && opener.matches("[data-node-menu]")) opener.setAttribute("aria-expanded", "false");
  if (restoreFocus && opener && opener.isConnected) opener.focus();
}

export function openNodeMenu(node, x, y, opener, actions) {
  closeNodeMenu();
  const element = document.createElement("div");
  element.className = "node-menu-pop";
  element.setAttribute("role", "menu");
  element.innerHTML = `<button class="btn" type="button" role="menuitem" data-edit-node>${h("edit")}</button>
    <button class="btn danger" type="button" role="menuitem" data-delete-node>${h("delete")}</button>
    <button class="btn" type="button" role="menuitem" data-connect-node="${esc(node.id)}">${h("connect")}</button>`;
  document.body.appendChild(element);
  const rect = element.getBoundingClientRect();
  element.style.left = `${Math.max(8, Math.min(x, window.innerWidth - rect.width - 8))}px`;
  element.style.top = `${Math.max(8, Math.min(y, window.innerHeight - rect.height - 8))}px`;
  const outside = (event) => {
    if (!element.contains(event.target)) closeNodeMenu();
  };
  const keys = (event) => {
    const items = [...element.querySelectorAll('[role="menuitem"]')];
    const index = items.indexOf(document.activeElement);
    if (event.key === "Escape") {
      event.preventDefault();
      closeNodeMenu(true);
      return;
    }
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      let next = 0;
      if (event.key === "ArrowDown") next = (index + 1) % items.length;
      else if (event.key === "ArrowUp") next = (index - 1 + items.length) % items.length;
      else if (event.key === "End") next = items.length - 1;
      items[next].focus();
    }
  };
  document.addEventListener("pointerdown", outside, true);
  document.addEventListener("keydown", keys, true);
  nodeMenu = { element, opener, outside, keys };
  if (opener && opener.matches && opener.matches("[data-node-menu]")) opener.setAttribute("aria-expanded", "true");
  element.querySelector("[data-edit-node]").addEventListener("click", () => {
    closeNodeMenu(true);
    actions.edit(node);
  });
  element.querySelector("[data-delete-node]").addEventListener("click", () => {
    closeNodeMenu(true);
    actions.remove(node);
  });
  element.querySelector("[data-connect-node]").addEventListener("click", () => {
    closeNodeMenu(true);
    actions.connect(node);
  });
  element.querySelector('[role="menuitem"]').focus();
}

export function openCanvasMenu(point, x, y, onAdd) {
  closeNodeMenu();
  const element = document.createElement("div");
  element.className = "node-menu-pop";
  element.setAttribute("role", "menu");
  element.innerHTML = `<button class="btn" type="button" role="menuitem" data-add-node>${h("add_node")}</button>`;
  document.body.appendChild(element);
  const rect = element.getBoundingClientRect();
  element.style.left = `${Math.max(8, Math.min(x, window.innerWidth - rect.width - 8))}px`;
  element.style.top = `${Math.max(8, Math.min(y, window.innerHeight - rect.height - 8))}px`;
  const outside = (event) => {
    if (!element.contains(event.target)) closeNodeMenu();
  };
  const keys = (event) => {
    const items = [...element.querySelectorAll('[role="menuitem"]')];
    const index = items.indexOf(document.activeElement);
    if (event.key === "Escape") {
      event.preventDefault();
      closeNodeMenu();
      return;
    }
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      let next = 0;
      if (event.key === "ArrowDown") next = (index + 1) % items.length;
      else if (event.key === "ArrowUp") next = (index - 1 + items.length) % items.length;
      else if (event.key === "End") next = items.length - 1;
      items[next].focus();
    }
  };
  document.addEventListener("pointerdown", outside, true);
  document.addEventListener("keydown", keys, true);
  nodeMenu = { element, opener: null, outside, keys };
  const item = element.querySelector("[data-add-node]");
  item.addEventListener("click", () => {
    closeNodeMenu();
    onAdd(point);
  });
  item.focus();
}

export function edgeWhenLabel(when) {
  return when === "failure" ? "error.generic" : when;
}

export function chooseEdgeType(fromNode, toNode) {
  return new Promise((resolve) => {
    let decided = false;
    const decide = (value) => {
      if (decided) return;
      decided = true;
      resolve(value);
    };
    const dialog = openDialog({
      id: "edge-type-dialog",
      title: t("connect_nodes", { from: fromNode.name, to: toNode.name }),
      body: `<p class="small muted">${h("choose_edge_type")}</p><div class="choice-row">${["success", "failure", "always"].map((when) => `<button class="btn pill${when === "success" ? " primary" : ""}" type="button" data-edge-when="${when}">${h(edgeWhenLabel(when))}</button>`).join("")}<button class="btn" type="button" data-edge-cancel>${h("cancel")}</button></div>`,
      form: false,
      actions: false,
      onClose: () => decide(null),
    });
    dialog.overlay.querySelectorAll("[data-edge-when]").forEach((button) => {
      button.addEventListener("click", () => {
        decide(button.dataset.edgeWhen);
        dialog.close();
      });
    });
    dialog.overlay.querySelector("[data-edge-cancel]").addEventListener("click", () => dialog.close());
  });
}
