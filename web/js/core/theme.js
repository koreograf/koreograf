import { h } from "./dom.js";

export const THEME_STORAGE_KEY = "koreograf.theme";
export const THEMES = ["system", "light", "dark"];
const DARK_QUERY = "(prefers-color-scheme: dark)";

let choice = "system";
let media = null;

export function themeId(value) {
  return THEMES.includes(value) ? value : "system";
}

export function resolveTheme(value, prefersDark) {
  const id = themeId(value);
  return id === "system" ? (prefersDark ? "dark" : "light") : id;
}

export function themeChoice() {
  return choice;
}

export function systemPrefersDark() {
  return typeof window !== "undefined"
    && typeof window.matchMedia === "function"
    && window.matchMedia(DARK_QUERY).matches;
}

export function applyTheme() {
  document.documentElement.dataset.theme = resolveTheme(choice, systemPrefersDark());
}

export function initTheme() {
  choice = themeId(localStorage.getItem(THEME_STORAGE_KEY));
  applyTheme();
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
  if (media) return;
  media = window.matchMedia(DARK_QUERY);
  const onChange = () => {
    if (choice === "system") applyTheme();
  };
  if (typeof media.addEventListener === "function") media.addEventListener("change", onChange);
  else if (typeof media.addListener === "function") media.addListener(onChange);
}

export function setTheme(next) {
  choice = themeId(next);
  localStorage.setItem(THEME_STORAGE_KEY, choice);
  applyTheme();
}

export function themeSwitch() {
  const option = (value, key) => `<option value="${value}"${choice === value ? " selected" : ""}>${h(key)}</option>`;
  return `<label class="field theme-switch"><span class="small">${h("theme")}</span><select class="theme-select" data-theme-select>${option("system", "theme_system")}${option("light", "theme_light")}${option("dark", "theme_dark")}</select></label>`;
}

export function bindThemeSwitch() {
  document.querySelectorAll("[data-theme-select]").forEach((select) => {
    select.value = choice;
    select.addEventListener("change", () => {
      setTheme(select.value);
      document.querySelectorAll("[data-theme-select]").forEach((node) => {
        node.value = themeChoice();
      });
    });
  });
}
