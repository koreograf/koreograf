export const LANGUAGES = [
  { id: "de", code: "GER", name: "Deutsch", country: "de" },
  { id: "en", code: "ENG", name: "English", country: "gb" },
];

export const FLAG_SVG = {
  de: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 5 3"><rect width="5" height="3" fill="#000"/><rect width="5" y="1" height="2" fill="#DE0000"/><rect width="5" y="2" height="1" fill="#FFCE00"/></svg>`,
  gb: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 60 30"><clipPath id="a"><path d="M0 0v30h60V0z"/></clipPath><clipPath id="b"><path d="M30 15h30v15zv15H0zH0V0zV0h30z"/></clipPath><g clip-path="url(#a)"><path d="M0 0v30h60V0z" fill="#012169"/><path d="M0 0l60 30m0-30L0 30" stroke="#fff" stroke-width="6"/><path d="M0 0l60 30m0-30L0 30" clip-path="url(#b)" stroke="#C8102E" stroke-width="4"/><path d="M30 0v30M0 15h60" stroke="#fff" stroke-width="10"/><path d="M30 0v30M0 15h60" stroke="#C8102E" stroke-width="6"/></g></svg>`,
};

export function languageId(value) {
  return LANGUAGES.some((item) => item.id === value) ? value : "de";
}

let messages = {};
let lang = "de";
let hooks = {
  ask: async () => true,
  leaveGuard: () => null,
  clearLeaveGuard: () => {},
  signedIn: () => false,
  renderLogin: () => {},
  route: async () => {},
};

export function configureLanguage(overrides) {
  hooks = { ...hooks, ...overrides };
}

export function currentLanguage() {
  return lang;
}

export function initLanguage() {
  lang = languageId(localStorage.getItem("koreograf.lang"));
  document.documentElement.lang = lang;
}

export function setCatalog(id, table) {
  messages[id] = table;
}

export async function loadCatalog(id) {
  if (messages[id]) return;
  try {
    messages[id] = await fetch(`/i18n/${id}.json`).then((response) => response.json());
  } catch {
    messages[id] = {};
  }
}

export function lookup(language, key) {
  const table = messages[language] || {};
  if (Object.prototype.hasOwnProperty.call(table, key)) return table[key];
  if (language !== "de" && Object.prototype.hasOwnProperty.call(messages.de || {}, key)) {
    return messages.de[key];
  }
  return key;
}

export function translate(language, key, vars) {
  let out = lookup(language, key);
  if (vars) {
    Object.entries(vars).forEach(([name, value]) => {
      const raw = String(value ?? "");
      const text = name === "field" || name === "label" ? lookup(language, raw) : raw;
      out = out.replaceAll(`{${name}}`, () => text);
    });
  }
  return out;
}

export function t(key, vars) {
  return translate(lang, key, vars);
}

export async function setLang(next) {
  const guard = hooks.leaveGuard();
  if (guard && guard() && !(await hooks.ask({ title: t("discard_unsaved_changes"), confirmLabel: t("ok") }))) return;
  hooks.clearLeaveGuard();
  lang = languageId(next);
  await loadCatalog(lang);
  localStorage.setItem("koreograf.lang", lang);
  document.documentElement.lang = lang;
  if (!hooks.signedIn()) hooks.renderLogin("");
  else await hooks.route();
  const button = document.querySelector("[data-lang-button]");
  if (button) button.focus();
}
