import * as appState from "./state.js";
import { api } from "./api.js";
import { configureDialogs, confirmDialog } from "./dialog.js";
import { esc, h } from "./dom.js";
import { FLAG_SVG, LANGUAGES, configureLanguage, currentLanguage, loadCatalog, setLang, t } from "./i18n.js";
import { matchRoute } from "./router.js";
import { ERROR_FIELD, fieldControl, showFieldError } from "./forms.js";

window.addEventListener("beforeunload", (event) => {
  if (!appState.leaveGuard || !appState.leaveGuard()) return;
  event.preventDefault();
  event.returnValue = "";
});

export function flagMarkup(country) {
  const svg = FLAG_SVG[country];
  if (!svg) return `<span class="flag flag-fallback" aria-hidden="true">${esc(String(country || "").slice(0, 2).toUpperCase())}</span>`;
  return `<img class="flag" alt="" src="data:image/svg+xml,${encodeURIComponent(svg)}">`;
}

export function langOption(item) {
  const selected = item.id === currentLanguage() ? "true" : "false";
  return `<li role="option" tabindex="-1" lang="${esc(item.id)}" data-lang-option="${esc(item.id)}" aria-selected="${selected}">${flagMarkup(item.country)}<span class="lang-code">${esc(item.code)}</span><span class="lang-name">${esc(item.name)}</span></li>`;
}

export function langSwitch() {
  const current = LANGUAGES.find((item) => item.id === currentLanguage()) || LANGUAGES[0];
  const options = LANGUAGES.map(langOption).join("");
  return `<div class="lang-switch" data-lang-select>
    <button class="lang-button" type="button" data-lang-button aria-haspopup="listbox" aria-expanded="false" aria-controls="lang-menu" aria-label="${h("language")}">${flagMarkup(current.country)}<span class="lang-code">${esc(current.code)}</span><span class="lang-caret" aria-hidden="true"></span></button>
    <ul class="lang-menu" id="lang-menu" role="listbox" hidden>${options}</ul>
  </div>`;
}

export function closeLangMenus() {
  document.querySelectorAll(".lang-menu").forEach((menu) => {
    menu.hidden = true;
    const button = menu.parentElement && menu.parentElement.querySelector("[data-lang-button]");
    if (button) button.setAttribute("aria-expanded", "false");
  });
}

export function bindLangSwitch() {
  document.querySelectorAll(".lang-switch").forEach((root) => {
    const button = root.querySelector("[data-lang-button]");
    const menu = root.querySelector(".lang-menu");
    if (!button || !menu) return;
    const options = () => [...menu.querySelectorAll("[data-lang-option]")];
    const openMenu = () => {
      closeLangMenus();
      menu.hidden = false;
      button.setAttribute("aria-expanded", "true");
      const current = options().find((item) => item.dataset.langOption === currentLanguage()) || options()[0];
      if (current) current.focus();
    };
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      if (menu.hidden) openMenu();
      else closeLangMenus();
    });
    button.addEventListener("keydown", (event) => {
      if (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        openMenu();
      }
    });
    options().forEach((option, index, list) => {
      option.addEventListener("click", () => {
        if (option.dataset.langOption !== currentLanguage()) setLang(option.dataset.langOption);
        else closeLangMenus();
      });
      option.addEventListener("keydown", (event) => {
        if (event.key === "ArrowDown") {
          event.preventDefault();
          list[(index + 1) % list.length].focus();
        } else if (event.key === "ArrowUp") {
          event.preventDefault();
          list[(index - 1 + list.length) % list.length].focus();
        } else if (event.key === "Home") {
          event.preventDefault();
          list[0].focus();
        } else if (event.key === "End") {
          event.preventDefault();
          list[list.length - 1].focus();
        } else if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          if (option.dataset.langOption !== currentLanguage()) setLang(option.dataset.langOption);
          else closeLangMenus();
        } else if (event.key === "Escape") {
          event.preventDefault();
          closeLangMenus();
          button.focus();
        }
      });
    });
  });
  if (!bindLangSwitch.outside) {
    document.addEventListener("click", (event) => {
      if (!event.target.closest(".lang-switch")) closeLangMenus();
    });
    bindLangSwitch.outside = true;
  }
}

export function roleOf() {
  if (!appState.me) return "viewer";
  if (appState.me.is_admin) return "owner";
  const team = (appState.me.teams || []).find((item) => item.id === appState.teamId);
  return team ? team.role : "viewer";
}

// Prefer the first local team over an LDAP-mapped team, which often has no content, then fall back to the first team.
export function preferredTeam() {
  const teams = appState.me.teams || [];
  const local = teams.find((team) => !team.ldap_group);
  return (local || teams[0] || {}).id || "";
}

export function rank(role) {
  return { viewer: 1, operator: 2, owner: 3 }[role] || 0;
}

export function can(min) {
  return rank(roleOf()) >= rank(min);
}

export function roleLabel(role) {
  return {
    viewer: t("viewer"),
    operator: t("operator"),
    owner: t("owner"),
  }[role] || role;
}

export function closeLive() {
  appState.setGeneration(appState.generation + 1);
  document.querySelector("#schedule-form")?.remove();
  document.querySelectorAll("[data-edit-overlay]").forEach((node) => node.remove());
  document.querySelectorAll(".node-menu-pop").forEach((node) => node.remove());
  if (appState.source) {
    appState.source.close();
    appState.setSource(null);
  }
  if (appState.pollTimer) {
    clearInterval(appState.pollTimer);
    appState.setPollTimer(null);
  }
  if (appState.navKey) {
    document.removeEventListener("keydown", appState.navKey);
    appState.setNavKey(null);
  }
}

export function markSaved() {
  appState.setFlashKey("saved");
}

export function markCreated() {
  appState.setFlashKey("created");
}

export function savedBanner() {
  if (!appState.flashKey) return "";
  const key = appState.flashKey;
  appState.setFlashKey("");
  if (key !== "created") return `<div class="banner good">${h(key)}</div>`;
  const timer = window.setTimeout(() => {
    const banner = document.getElementById("created-banner");
    if (banner) banner.remove();
  }, 4000);
  if (timer && typeof timer.unref === "function") timer.unref();
  return `<div class="banner good" id="created-banner" role="status">${h(key)}</div>`;
}

export function clearErrorBanners() {
  document.querySelectorAll(".banner.bad").forEach((node) => node.remove());
}

export function topmostCard() {
  const overlays = [...document.querySelectorAll(".overlay")];
  const overlay = overlays[overlays.length - 1];
  return overlay ? overlay.querySelector(".overlay-card") : null;
}

export function handle(error) {
  if (error && error.name === "AbortError") return;
  if (error && error.status === 401) {
    appState.setMe(null);
    renderLogin("session.expired");
    return;
  }
  if (!document.querySelector(".main")) shell(t("notice"), "");
  const main = document.querySelector(".main");
  const message = t(error.message || "error.generic", error.vars);
  clearErrorBanners();
  main.querySelectorAll(":scope > .banner.good").forEach((node) => node.remove());
  const card = topmostCard();
  const banner = `<div class="banner bad">${esc(message)}</div>`;
  if (card) (card.querySelector(".dialog-body") || card).insertAdjacentHTML("afterbegin", banner);
  else main.insertAdjacentHTML("afterbegin", banner);
  const fieldName = (error.vars && error.vars.field) || ERROR_FIELD[error.message] || "";
  const control = fieldControl(fieldName);
  if (!control) return;
  showFieldError(control, message);
  control.focus();
}

export function renderRouteError(error) {
  if (error && error.name === "AbortError") return;
  if (error && error.status === 401) {
    appState.setMe(null);
    renderLogin("session.expired");
    return;
  }
  const message = t(error.message || "error.generic", error.vars);
  shell(t("route_error_title"), `
    <div class="card">
      <div class="banner bad">${esc(message)}</div>
      <p class="muted">${h("route_error_explanation")}</p>
      <div class="row"><button class="btn primary" type="button" id="route-retry">${h("retry")}</button><a class="btn" href="#/">${h("back_to_overview")}</a></div>
    </div>`);
  const retry = document.querySelector("#route-retry");
  if (retry) retry.addEventListener("click", () => route());
}

export function go(hash) {
  if ((location.hash || "#/") === hash) route();
  else location.hash = hash;
}

export function assistantNavItem(enabled) {
  return enabled ? [["#/assistant", t("assistant")]] : [];
}

export function navActive(here, href) {
  if (href === "#/") return here === "#/" || here === "";
  if (href === "#/runtime" && here.startsWith("#/environments")) return true;
  return here === href || here.startsWith(`${href}/`);
}

export function versionSlot() {
  return `<p class="small version-line" data-version hidden><a href="https://github.com/koreograf/koreograf" target="_blank" rel="noopener noreferrer"></a></p>`;
}

export function fillVersion() {
  if (!appState.buildVersion) return;
  const version = [appState.buildVersion, appState.buildCommit.slice(0, 7)].filter(Boolean).join(" ");
  document.querySelectorAll("[data-version]").forEach((line) => {
    const link = line.querySelector("a");
    if (link) {
      link.textContent = t("build_version", { version });
      link.title = appState.buildCommit;
    }
    line.hidden = false;
  });
}

export async function loadVersion() {
  if (!appState.versionRequested) {
    appState.setVersionRequested(true);
    try {
      const response = await fetch("/api/v1/version", { credentials: "same-origin" });
      if (response.ok) {
        const data = await response.json();
        const version = String(data.version || "");
        const commit = data.commit && data.commit !== "unknown" ? String(data.commit) : "";
        if (version) appState.setBuildVersion(version);
        appState.setBuildCommit(commit);
      }
    } catch {
      appState.setBuildVersion("");
      appState.setBuildCommit("");
    }
  }
  fillVersion();
}

export function maintenanceBanner() {
  return `<div class="banner warn" id="maintenance-banner"${appState.maintenanceActive ? "" : " hidden"}><p>${h("maintenance_mode_banner")}</p><p class="small">${h("no_new_jobs_during_maintenance")}${appState.maintenanceSince ? ` <span class="mono">${esc(appState.maintenanceSince)}</span>` : ""}</p></div>`;
}

export async function loadMaintenance() {
  try {
    const response = await fetch("/api/v1/maintenance", { credentials: "same-origin" });
    if (response.ok) {
      const data = await response.json();
      appState.setMaintenanceActive(Boolean(data && data.active));
      appState.setMaintenanceSince(data && data.since ? String(data.since) : "");
    }
  } catch {
    return;
  }
  const node = document.querySelector("#maintenance-banner");
  if (node) {
    node.hidden = !appState.maintenanceActive;
    const detail = node.querySelector(".small");
    if (detail) detail.innerHTML = `${h("no_new_jobs_during_maintenance")}${appState.maintenanceSince ? ` <span class="mono">${esc(appState.maintenanceSince)}</span>` : ""}`;
  }
  if (!appState.maintenanceTimer) {
    appState.setMaintenanceTimer(window.setInterval(() => { loadMaintenance(); }, 15000));
  }
}

export function shell(title, body, actions) {
  const here = (location.hash || "#/").split("?")[0];
  const groups = [
    [t("nav.operations"), [
      ["#/", t("overview")],
      ["#/jobs", t("jobs")],
      ["#/schedules", t("schedules")],
      ...assistantNavItem(appState.assistantEnabled),
    ]],
    [t("automation"), [
      ["#/templates", t("templates")],
      ["#/workflows", t("workflows")],
      ["#/hooks", t("webhooks")],
    ]],
    [t("resources"), [
      ["#/projects", t("projects")],
      ["#/inventories", t("inventories")],
      ["#/credentials", t("credentials")],
      ["#/runtime", t("execution_environments")],
    ]],
    [t("nav.team"), [
      ["#/team", t("team")],
      ["#/organizations", t("organizations")],
    ]],
    ...(appState.me.is_admin ? [[t("nav.administration"), [
      ["#/settings", t("settings")],
      ["#/users", t("users")],
      ["#/backup", t("backup")],
    ]]] : []),
  ];
  const nav = groups.map(([label, links]) => `
    <div class="nav-group">
      <div class="nav-label">${label}</div>
      ${links.map(([href, text]) => `<a href="${href}" class="${navActive(here, href) ? "active" : ""}">${text}</a>`).join("")}
    </div>`).join("");
  const options = (appState.me.teams || []).map((team) => {
    const label = team.organization_name ? `${team.organization_name} · ${team.name}` : team.name;
    return `<option value="${esc(team.id)}"${team.id === appState.teamId ? " selected" : ""}>${esc(label)}</option>`;
  }).join("");
  appState.setNavOpen(false);
  appState.app.innerHTML = `
    <div class="shell">
      <a class="skip-link" href="#main">${h("skip_to_content")}</a>
      <div class="scrim" id="nav-scrim"></div>
      <aside class="side">
        <div class="brand-lockup"><span class="mark" aria-hidden="true"></span><p class="word">Koreograf</p></div>
        <label class="field"><span class="small">${h("organization_team")}</span><select id="team-select">${options || `<option value="">${h("no_team")}</option>`}</select></label>
        <nav class="nav">${nav}</nav>
        <div class="side-foot">
          <a class="account-link" href="#/account">${esc(appState.me.display_name)} · ${esc(roleLabel(roleOf()))}${appState.me.auth_source === "ldap" ? " · LDAP" : ""}</a>
          ${versionSlot()}
          ${langSwitch()}
          <button class="btn ghost" id="logout" type="button">${h("sign_out")}</button>
        </div>
      </aside>
      <main class="main" id="main" tabindex="-1">
        <div class="top"><div><button class="btn ghost nav-toggle" id="nav-toggle" type="button" aria-label="${h("menu")}" aria-expanded="false"><svg width="20" height="20" viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="M3 5.5h14M3 10h14M3 14.5h14" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg></button><h1 tabindex="-1">${esc(title)}</h1></div><div class="row">${actions || ""}</div></div>
        ${savedBanner()}
        ${maintenanceBanner()}
        ${body}
      </main>
    </div>`;
  const select = document.querySelector("#team-select");
  select.addEventListener("change", async () => {
    if (appState.leaveGuard && appState.leaveGuard() && !(await confirmDialog({ title: t("discard_unsaved_changes"), confirmLabel: t("ok") }))) {
      select.value = appState.teamId;
      return;
    }
    appState.setLeaveGuard(null);
    appState.setTeamId(select.value);
    localStorage.setItem("koreograf.team", appState.teamId);
    route();
  });
  bindLangSwitch();
  document.querySelector(".skip-link").addEventListener("click", (event) => {
    event.preventDefault();
    document.getElementById("main").focus();
  });
  document.querySelector("#logout").addEventListener("click", async () => {
    try { await api("POST", "/api/v1/logout"); } catch { /* session ended locally */ }
    appState.setMe(null);
    go("#/login");
  });
  const navToggle = document.querySelector("#nav-toggle");
  const sidePanel = document.querySelector(".side");
  const navScrim = document.querySelector("#nav-scrim");
  const setNavOpen = (open) => {
    appState.setNavOpen(open);
    navToggle.setAttribute("aria-expanded", String(open));
    sidePanel.classList.toggle("open", open);
    navScrim.classList.toggle("open", open);
  };
  navToggle.addEventListener("click", () => setNavOpen(!appState.navOpen));
  navScrim.addEventListener("click", () => setNavOpen(false));
  appState.setNavKey((event) => { if (event.key === "Escape") setNavOpen(false); });
  document.addEventListener("keydown", appState.navKey);
  loadVersion();
  loadMaintenance();
}

export function deny(title, min) {
  const text = min === "admin"
    ? "you_need_an_administrator_account"
    : min === "owner"
      ? "you_need_the_owner_role"
      : "you_need_the_operator_role";
  shell(t(title), `<div class="card">${h(text)}</div>`);
}

export function needTeam() {
  if (appState.teamId) return true;
  shell(t("team"), `<div class="card"><p>${h("this_account_has_no_team_an_administrator_creates")}</p><p><a href="#/team">${h("open_team")}</a></p></div>`);
  return false;
}

export function renderLogin(message, tone, vars, username) {
  const kind = tone === "good" ? "good" : "bad";
  appState.app.innerHTML = `
    <div class="login-wrap">
      <div class="login-stack">
        <form class="login-card" id="login-form">
          <div class="spread">
            <div class="brand-lockup"><span class="mark" aria-hidden="true"></span><h1 class="word">Koreograf</h1></div>
          </div>
          <p class="lede">${h("jobs_workflows_and_alerts_for_several_teams_local")}</p>
          ${message ? `<div class="banner ${kind}">${h(message, vars)}</div>` : ""}
          ${maintenanceBanner()}
          <div class="form">
            <label class="field"><span>${h("username")}</span><input name="username" autocomplete="username" required value="${esc(username || "")}"></label>
            <label class="field"><span>${h("password")}</span><input name="password" type="password" autocomplete="current-password" required></label>
            <button class="btn primary" type="submit">${h("sign_in")}</button>
          </div>
          ${langSwitch()}
        </form>
        ${versionSlot()}
      </div>
    </div>`;
  bindLangSwitch();
  loadVersion();
  loadMaintenance();
  const initial = username
    ? document.querySelector("#login-form [name=password]")
    : document.querySelector("#login-form [name=username]");
  if (initial) initial.focus();
  document.querySelector("#login-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const button = form.querySelector("button[type=submit]");
    button.disabled = true;
    const entered = form.username.value;
    try {
      appState.setMe(await api("POST", "/api/v1/login", {
        username: entered,
        password: form.password.value,
      }));
      const known = (appState.me.teams || []).some((team) => team.id === appState.teamId);
      if (!known) appState.setTeamId(preferredTeam());
      localStorage.setItem("koreograf.team", appState.teamId);
      go("#/");
    } catch (error) {
      renderLogin(error.message || "invalid_username_or_password", "bad", error.vars, entered);
      const password = document.querySelector("#login-form [name=password]");
      if (password) password.focus();
    }
  });
}

export const ROUTE_TITLES = {
  "": "overview",
  templates: "templates",
  jobs: "jobs",
  schedules: "schedules",
  workflows: "workflows",
  hooks: "webhooks",
  environments: "execution_environments",
  runtime: "execution_environments",
  projects: "projects",
  inventories: "inventories",
  credentials: "credentials",
  organizations: "organizations",
  account: "account",
  users: "users",
  settings: "settings",
  backup: "backup",
  assistant: "assistant",
  team: "team",
};

export function routeTitle(parts) {
  const key = parts.length ? ROUTE_TITLES[parts[0]] : "overview";
  return key ? t(key) : t("loading");
}

export function skeletonBody() {
  return `<div class="loading-skeleton" role="status" aria-live="polite">
    <span class="small muted">${h("loading")}</span>
    <div class="skeleton-card"><span class="skeleton-line short"></span><span class="skeleton-line"></span><span class="skeleton-line medium"></span></div>
    <div class="skeleton-card"><span class="skeleton-line short"></span><span class="skeleton-line"></span><span class="skeleton-line medium"></span></div>
  </div>`;
}

export function showLoading(title) {
  const main = document.querySelector(".main");
  if (!main) {
    shell(title, skeletonBody());
    return;
  }
  main.setAttribute("aria-busy", "true");
  main.innerHTML = `<div class="top"><div><h1 tabindex="-1">${esc(title)}</h1></div><div class="row"></div></div>${skeletonBody()}`;
}

export function settleRoute(back) {
  const hash = location.hash || "#/";
  if (back && appState.routeScroll.has(hash)) window.scrollTo(0, appState.routeScroll.get(hash));
  else window.scrollTo(0, 0);
  if (back) return;
  const main = document.querySelector(".main");
  const heading = main && main.querySelector("h1");
  if (!main || !heading || main.contains(document.activeElement)) return;
  heading.focus({ preventScroll: true });
}

export function hasHistoryIndex(state) {
  return Boolean(state && Number.isInteger(state.idx));
}

export function routeDirection(state) {
  if (hasHistoryIndex(state)) {
    const back = state.idx <= appState.historyIndex;
    appState.setHistoryIndex(state.idx);
    return back;
  }
  appState.setHistoryIndex(appState.historyIndex + 1);
  return false;
}

export async function route() {
  const nextHash = location.hash || "#/";
  if (appState.leaveGuard && appState.shownHash !== null && nextHash !== appState.shownHash && appState.leaveGuard()) {
    return confirmDialog({ title: t("discard_unsaved_changes"), confirmLabel: t("ok") }).then((leave) => {
      if (!leave) {
        history.replaceState({ idx: (history.state && history.state.idx) || 0 }, "", appState.shownHash);
        return;
      }
      return load();
    });
  }
  async function load() {
    appState.setLeaveGuard(null);
    const previousHash = appState.shownHash;
    appState.setShownHash(nextHash);
    closeLive();
    if (appState.navToken) appState.navToken.abort();
    const hash = location.hash || "#/";
    const hashPath = hash.split("?")[0];
    const token = new AbortController();
    appState.setNavToken(token);
    if (!appState.me) {
      if (hashPath !== "#/login") location.hash = "#/login";
      renderLogin("");
      return;
    }
    if (hashPath === "#/login") {
      location.hash = "#/";
      return;
    }
    const parts = hashPath.slice(1).split("/").filter(Boolean);
    const historyState = history.state;
    const back = routeDirection(historyState);
    if (!hasHistoryIndex(historyState)) history.replaceState({ idx: appState.historyIndex }, "", location.hash || "#/");
    if (previousHash !== null && previousHash !== nextHash) appState.routeScroll.set(previousHash, window.scrollY);
    showLoading(routeTitle(parts));
    if (!back) window.scrollTo(0, 0);
    try {
      const matched = matchRoute(hashPath);
      if (!matched) shell(t("page"), `<div class="card">${h("this_page_does_not_exist")}</div>`);
      else {
        const result = await matched.handler(matched.params);
        if (result && result.redirect) {
          location.replace(result.redirect);
          return;
        }
      }
      settleRoute(back);
    } catch (error) {
      if (token !== appState.navToken) return;
      renderRouteError(error);
      settleRoute(false);
    }
  }
  return load();
}

export async function boot() {
  const catalogs = currentLanguage() === "de" ? ["de"] : ["de", currentLanguage()];
  await Promise.all(catalogs.map(loadCatalog));
  try {
    appState.setMe(await api("GET", "/api/v1/me"));
    const known = (appState.me.teams || []).some((team) => team.id === appState.teamId);
    if (!known) {
      appState.setTeamId(preferredTeam());
      localStorage.setItem("koreograf.team", appState.teamId);
    }
    try {
      const assistant = await api("GET", "/api/v1/assistant");
      appState.setAssistantEnabled(Boolean(assistant && assistant.enabled));
      appState.setAssistantModel(assistant && assistant.model ? String(assistant.model) : "");
    } catch {
      appState.setAssistantEnabled(false);
      appState.setAssistantModel("");
    }
  } catch {
    appState.setMe(null);
  }
  window.addEventListener("hashchange", route);
  route();
}

configureLanguage({
  ask: (options) => confirmDialog(options),
  leaveGuard: () => appState.leaveGuard,
  clearLeaveGuard: () => appState.setLeaveGuard(null),
  signedIn: () => Boolean(appState.me),
  renderLogin: (message) => renderLogin(message),
  route: () => route(),
});
configureDialogs((error) => handle(error));
