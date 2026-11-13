"use strict";

const VOID = new Set(["input", "img", "br", "hr", "meta", "link", "source", "area", "base", "col", "embed", "param", "track", "wbr"]);

class Element {
  constructor(tag) {
    this.tagName = String(tag || "div").toUpperCase();
    this.attributes = {};
    this.dataset = new Proxy({}, {
      set: (target, name, value) => {
        target[name] = value;
        this.attributes[`data-${String(name).replace(/[A-Z]/g, (ch) => `-${ch.toLowerCase()}`)}`] = String(value);
        return true;
      },
      deleteProperty: (target, name) => {
        delete target[name];
        delete this.attributes[`data-${String(name).replace(/[A-Z]/g, (ch) => `-${ch.toLowerCase()}`)}`];
        return true;
      },
    });
    this.style = {};
    this.children = [];
    this.parentNode = null;
    this.listeners = {};
    this.classes = new Set();
    this.value = "";
    this.checked = false;
    this.disabled = false;
    this.required = false;
    this.hidden = false;
    this.textContent = "";
    this.scrollTop = 0;
    this.scrollHeight = 0;
    this.clientHeight = 0;
    this.clientWidth = 800;
    this.offsetWidth = 200;
    this.offsetHeight = 100;
    this.offsetParent = {};
    this.tabIndex = 0;
  }

  get id() {
    return this.attributes.id || "";
  }

  set id(value) {
    this.setAttribute("id", value);
  }

  get className() {
    return [...this.classes].join(" ");
  }

  set className(value) {
    this.classes = new Set(String(value || "").split(/\s+/).filter(Boolean));
  }

  get classList() {
    const classes = this.classes;
    return {
      add: (...names) => names.forEach((name) => classes.add(name)),
      remove: (...names) => names.forEach((name) => classes.delete(name)),
      toggle: (name, force) => {
        const next = force === undefined ? !classes.has(name) : Boolean(force);
        if (next) classes.add(name);
        else classes.delete(name);
        return next;
      },
      contains: (name) => classes.has(name),
    };
  }

  get innerHTML() {
    if (this.html !== undefined) return this.html;
    if (this.nodeType === 3) return this.textContent;
    return this.children.map(serializeNode).join("");
  }

  set innerHTML(value) {
    this.html = String(value);
    this.children = [];
    parse(String(value), this);
    bindFormNames(this);
  }

  get isConnected() {
    let node = this;
    while (node.parentNode) node = node.parentNode;
    return node === globalThis.document;
  }

  get parentElement() {
    return this.parentNode && this.parentNode.nodeType === 1 ? this.parentNode : null;
  }

  setAttribute(name, value) {
    const text = String(value);
    this.attributes[name] = text;
    if (name === "class") this.className = text;
    if (name === "value") this.value = text;
    if (name === "checked") this.checked = true;
    if (name === "disabled") this.disabled = true;
    if (name === "hidden") this.hidden = true;
    if (name.startsWith("data-")) this.dataset[name.slice(5).replace(/-([a-z])/g, (_, ch) => ch.toUpperCase())] = text;
  }

  getAttribute(name) {
    return Object.prototype.hasOwnProperty.call(this.attributes, name) ? this.attributes[name] : null;
  }

  removeAttribute(name) {
    delete this.attributes[name];
  }

  hasAttribute(name) {
    return Object.prototype.hasOwnProperty.call(this.attributes, name);
  }

  appendChild(child) {
    if (child.parentNode) child.remove();
    child.parentNode = this;
    this.children.push(child);
    return child;
  }

  insertBefore(child, before) {
    if (child.parentNode) child.remove();
    const index = this.children.indexOf(before);
    if (index < 0) this.children.push(child);
    else this.children.splice(index, 0, child);
    child.parentNode = this;
    return child;
  }

  insertAdjacentHTML(position, html) {
    const holder = new Element("div");
    holder.innerHTML = html;
    const nodes = holder.children.slice();
    if (position === "beforebegin" && this.parentNode) {
      nodes.forEach((node) => this.parentNode.insertBefore(node, this));
    } else if (position === "afterbegin") {
      nodes.reverse().forEach((node) => this.insertBefore(node, this.children[0]));
    } else {
      nodes.forEach((node) => this.appendChild(node));
    }
  }

  replaceChildren(...nodes) {
    this.children = [];
    nodes.forEach((node) => this.appendChild(node));
  }

  contains(node) {
    let current = node;
    while (current) {
      if (current === this) return true;
      current = current.parentNode;
    }
    return false;
  }

  remove() {
    if (!this.parentNode) return;
    const index = this.parentNode.children.indexOf(this);
    if (index >= 0) this.parentNode.children.splice(index, 1);
    this.parentNode = null;
  }

  focus() {
    if (globalThis.document) globalThis.document.activeElement = this;
  }

  blur() {
    if (globalThis.document && globalThis.document.activeElement === this) globalThis.document.activeElement = globalThis.document.body;
  }

  scrollIntoView() {}

  setPointerCapture() {}

  releasePointerCapture() {}

  matches(selector) {
    return selector.split(",").some((part) => matchCompound(this, part.trim()));
  }

  closest(selector) {
    let node = this;
    while (node && node !== globalThis.document) {
      if (node.matches && node.matches(selector)) return node;
      node = node.parentNode;
    }
    return null;
  }

  querySelector(selector) {
    return query(this, selector, false)[0] || null;
  }

  querySelectorAll(selector) {
    return query(this, selector, true).filter((node) => node !== this);
  }

  addEventListener(type, listener) {
    (this.listeners[type] = this.listeners[type] || []).push(listener);
  }

  removeEventListener(type, listener) {
    if (!this.listeners[type]) return;
    this.listeners[type] = this.listeners[type].filter((item) => item !== listener);
  }

  dispatchEvent(event) {
    event.target = event.target || this;
    event.currentTarget = this;
    event.preventDefault = event.preventDefault || (() => {});
    event.stopPropagation = event.stopPropagation || (() => {});
    (this.listeners[event.type] || []).slice().forEach((listener) => listener.call(this, event));
    return true;
  }

  getBoundingClientRect() {
    return { left: 0, top: 0, width: 200, height: 120, right: 200, bottom: 120 };
  }
}

Element.prototype.nodeType = 1;

class Text extends Element {
  constructor(text) {
    super("#text");
    this.nodeType = 3;
    this.textContent = String(text);
  }
}

function decode(text) {
  return String(text)
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}

function parseAttributes(source, node) {
  const pattern = /([^\s=/>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g;
  let match;
  while ((match = pattern.exec(source))) {
    const name = match[1];
    const value = match[2] ?? match[3] ?? match[4] ?? "";
    node.setAttribute(name, decode(value));
  }
  if (node.getAttribute("class")) node.className = node.getAttribute("class");
}

function serializeNode(node) {
  if (node.nodeType === 3 || node.tagName === "#TEXT") return node.textContent;
  const attributes = Object.entries(node.attributes).map(([name, value]) => ` ${name}="${value}"`).join("");
  return `<${node.tagName.toLowerCase()}${attributes}>${node.innerHTML}</${node.tagName.toLowerCase()}>`;
}

function bindFormNames(node) {  if (node.tagName === "SELECT") {
    const options = descendants(node, []).filter((child) => child.tagName === "OPTION");
    const selected = options.find((option) => option.hasAttribute("selected")) || options[0];
    node.value = selected ? (selected.getAttribute("value") ?? selected.textContent) : "";
  }
  if (node.tagName === "FORM") {
    const elements = {};
    descendants(node, []).forEach((child) => {
      const name = child.getAttribute("name");
      if (!name) return;
      elements[name] = child;
      if (!Object.prototype.hasOwnProperty.call(node, name)) node[name] = child;
    });
    node.elements = elements;
  }
  node.children.forEach(bindFormNames);
}

function parse(html, parent) {  const pattern = /<!--[\s\S]*?-->|<(\/?)([a-zA-Z][\w-]*)((?:\s+[^>]*?)?)(\/?)>/g;
  let last = 0;
  let match;
  let current = parent;
  const stack = [];
  const appendText = (text) => {
    if (!text) return;
    const node = new Text(decode(text));
    node.parentNode = current;
    current.children.push(node);
  };
  while ((match = pattern.exec(html))) {
    appendText(html.slice(last, match.index));
    last = pattern.lastIndex;
    if (match[1] === "/") {
      const index = stack.map((node) => node.tagName).lastIndexOf(match[2].toUpperCase());
      if (index >= 0) {
        stack.length = index;
        current = stack.length ? stack[stack.length - 1] : parent;
      }
      continue;
    }
    const node = new Element(match[2]);
    parseAttributes(match[3] || "", node);
    node.parentNode = current;
    current.children.push(node);
    if (!match[4] && !VOID.has(match[2].toLowerCase())) {
      stack.push(node);
      current = node;
    }
  }
  appendText(html.slice(last));
}

function parseSelector(selector) {
  const parts = [];
  let combinator = " ";
  let token = "";
  let depth = 0;
  for (const ch of selector.trim()) {
    if (ch === "[" || ch === "(") depth += 1;
    if (ch === "]" || ch === ")") depth -= 1;
    if (depth === 0 && /\s/.test(ch)) {
      if (token) {
        parts.push({ combinator, token });
        token = "";
        combinator = " ";
      }
    } else if (depth === 0 && ch === ">") {
      if (token) {
        parts.push({ combinator, token });
        token = "";
      }
      combinator = ">";
    } else {
      token += ch;
    }
  }
  if (token) parts.push({ combinator, token });
  return parts;
}

function matchCompound(node, compound) {
  if (!node || node.nodeType !== 1) return false;
  if (compound === "*") return true;
  let rest = compound;
  const tag = /^([a-zA-Z][\w-]*)/.exec(rest);
  if (tag) {
    if (node.tagName !== tag[1].toUpperCase()) return false;
    rest = rest.slice(tag[1].length);
  }
  const extras = /([.#][\w-]+|\[[^\]]+\]|:[\w-]+(?:\([^)]*\))?)/g;
  let match;
  while ((match = extras.exec(rest))) {
    const part = match[1];
    if (part.startsWith(".")) {
      if (!node.classes.has(part.slice(1))) return false;
    } else if (part.startsWith("#")) {
      if (node.id !== part.slice(1)) return false;
    } else if (part.startsWith("[")) {
      const body = part.slice(1, -1);
      const split = /^([^=~|^$*]+)(?:([~|^$*]?=)(?:"([^"]*)"|'([^']*)'|([^\]]*)))?$/.exec(body);
      if (!split) return false;
      const name = split[1].trim();
      if (!node.hasAttribute(name)) return false;
      if (split[2]) {
        const expected = split[3] ?? split[4] ?? split[5] ?? "";
        const actual = node.getAttribute(name);
        if (split[2] === "=" && actual !== expected) return false;
        if (split[2] === "^=" && !actual.startsWith(expected)) return false;
        if (split[2] === "$=" && !actual.endsWith(expected)) return false;
        if (split[2] === "*=" && !actual.includes(expected)) return false;
      }
    } else if (part.startsWith(":not(")) {
      if (matchCompound(node, part.slice(5, -1))) return false;
    } else if (part === ":checked") {
      if (!node.checked) return false;
    } else if (part === ":disabled") {
      if (!node.disabled) return false;
    } else if (part === ":scope") {
      continue;
    } else {
      return false;
    }
  }
  return true;
}

function descendants(node, out) {
  node.children.forEach((child) => {
    if (child.nodeType === 1) out.push(child);
    descendants(child, out);
  });
  return out;
}

function query(root, selector, many) {
  const groups = selector.split(",").map((part) => parseSelector(part));
  const found = [];
  for (const parts of groups) {
    const first = parts[0];
    if (!first) continue;
    let current;
    if (first.combinator === ">") current = root.children.filter((node) => node.nodeType === 1 && matchCompound(node, first.token));
    else current = descendants(root, []).filter((node) => matchCompound(node, first.token));
    for (const part of parts.slice(1)) {
      const next = [];
      const pool = part.combinator === ">" ? current.flatMap((node) => node.children.filter((child) => child.nodeType === 1)) : descendants({ children: current }, []);
      pool.forEach((node) => {
        if (matchCompound(node, part.token) && !next.includes(node)) next.push(node);
      });
      current = next;
    }
    current.forEach((node) => {
      if (!found.includes(node)) found.push(node);
    });
  }
  return found;
}

class Document extends Element {
  constructor() {
    super("#document");
    this.nodeType = 9;
    this.documentElement = new Element("html");
    this.body = new Element("body");
    this.documentElement.appendChild(this.body);
    this.appendChild(this.documentElement);
    this.activeElement = this.body;
    this.app = new Element("div");
    this.app.id = "app";
    this.body.appendChild(this.app);
  }

  createElement(tag) {
    return new Element(tag);
  }

  createElementNS(_namespace, tag) {
    return new Element(tag);
  }

  elementFromPoint() {
    return null;
  }

  createTextNode(text) {
    return new Text(text);
  }

  getElementById(id) {
    return this.querySelector(`#${id}`);
  }

  contains(node) {
    let current = node;
    while (current) {
      if (current === this) return true;
      current = current.parentNode;
    }
    return false;
  }
}

function stateObject() {
  let value = null;
  let index = 0;
  return {
    get state() {
      return value;
    },
    replaceState(next, _title, hash) {
      value = next;
      index += 1;
      if (hash) globalThis.location.hash = hash;
    },
    pushState(next, _title, hash) {
      value = next;
      index += 1;
      if (hash) globalThis.location.hash = hash;
    },
  };
}

function install() {
  const document = new Document();
  globalThis.Element = Element;
  globalThis.document = document;
  globalThis.location = { hash: "#/" };
  globalThis.history = stateObject();
  const listeners = {};
  globalThis.window = {
    addEventListener(type, listener) {
      (listeners[type] = listeners[type] || []).push(listener);
    },
    removeEventListener(type, listener) {
      listeners[type] = (listeners[type] || []).filter((item) => item !== listener);
    },
    dispatchEvent(event) {
      (listeners[event.type] || []).slice().forEach((listener) => listener(event));
    },
    location: globalThis.location,
    innerWidth: 1024,
    innerHeight: 768,
    scrollY: 0,
    scrollTo() {},
    setTimeout,
    clearTimeout,
    setInterval: (fn, ms) => {
      const timer = setInterval(fn, ms);
      if (timer.unref) timer.unref();
      return timer;
    },
    clearInterval,
  };
  globalThis.localStorage = { getItem() { return null; }, setItem() {}, removeItem() {} };
  globalThis.EventSource = class {
    addEventListener() {}
    close() {}
  };
  globalThis.CSS = { escape: (value) => String(value).replace(/[^a-zA-Z0-9_-]/g, (ch) => `\\${ch}`) };
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => "", json: async () => ({}) });
  return { document, window: globalThis.window, history: globalThis.history, Element, Text };
}

module.exports = { install, Element, Text };
