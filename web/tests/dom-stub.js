"use strict";

function node() {
  return {
    classList: { add() {}, remove() {}, toggle() {}, contains() { return false; } },
    dataset: {},
    style: {},
    hidden: false,
    querySelector() { return null; },
    querySelectorAll() { return []; },
    addEventListener() {},
    removeEventListener() {},
    setAttribute() {},
    removeAttribute() {},
    insertAdjacentHTML() {},
    appendChild() {},
    replaceChildren() {},
    remove() {},
    focus() {},
  };
}

globalThis.document = {
  querySelector() { return node(); },
  querySelectorAll() { return []; },
  getElementById() { return null; },
  createElement() { return node(); },
  addEventListener() {},
  removeEventListener() {},
  documentElement: { lang: "de" },
  body: node(),
  contains() { return false; },
  activeElement: null,
};
globalThis.window = {
  addEventListener() {},
  removeEventListener() {},
  location: { hash: "#/" },
  setTimeout,
  clearTimeout,
  setInterval,
  clearInterval,
};
globalThis.location = { hash: "#/" };
globalThis.localStorage = { getItem() { return null; }, setItem() {}, removeItem() {} };
globalThis.EventSource = class {};
