"use strict";

const LANGUAGES = [
  { id: "de", code: "GER", name: "Deutsch", country: "de" },
  { id: "en", code: "ENG", name: "English", country: "gb" },
];

const FLAG_SVG = {
  de: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 5 3"><rect width="5" height="3" fill="#000"/><rect width="5" y="1" height="2" fill="#DE0000"/><rect width="5" y="2" height="1" fill="#FFCE00"/></svg>`,
  gb: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 60 30"><clipPath id="a"><path d="M0 0v30h60V0z"/></clipPath><clipPath id="b"><path d="M30 15h30v15zv15H0zH0V0zV0h30z"/></clipPath><g clip-path="url(#a)"><path d="M0 0v30h60V0z" fill="#012169"/><path d="M0 0l60 30m0-30L0 30" stroke="#fff" stroke-width="6"/><path d="M0 0l60 30m0-30L0 30" clip-path="url(#b)" stroke="#C8102E" stroke-width="4"/><path d="M30 0v30M0 15h60" stroke="#fff" stroke-width="10"/><path d="M30 0v30M0 15h60" stroke="#C8102E" stroke-width="6"/></g></svg>`,
};

function languageId(value) {
  return LANGUAGES.some((item) => item.id === value) ? value : "de";
}

const app = document.querySelector("#app");
let me = null;
let teamId = localStorage.getItem("koreograf.team") || "";
let lang = languageId(localStorage.getItem("koreograf.lang"));
let source = null;
let pollTimer = null;
let generation = 0;
let runKey = null;
document.documentElement.lang = lang;



function esc(value) {
  return String(value ?? "").replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }[ch]));
}

let messages = { de: {}, en: {} };

function lookup(language, key) {
  const table = messages[language] || {};
  if (Object.prototype.hasOwnProperty.call(table, key)) return table[key];
  if (language !== "de" && Object.prototype.hasOwnProperty.call(messages.de || {}, key)) {
    return messages.de[key];
  }
  return key;
}

function t(key, vars) {
  let out = lookup(lang, key);
  if (vars) {
    Object.entries(vars).forEach(([name, value]) => {
      const raw = String(value ?? "");
      const text = name === "field" || name === "label" ? lookup(lang, raw) : raw;
      out = out.replaceAll(`{${name}}`, text);
    });
  }
  return out;
}

function h(text, vars) {
  return esc(t(text, vars));
}

function setLang(next) {
  lang = languageId(next);
  localStorage.setItem("koreograf.lang", lang);
  document.documentElement.lang = lang;
  if (!me) renderLogin("");
  else route();
}

function flagMarkup(country) {
  const svg = FLAG_SVG[country];
  if (!svg) return `<span class="flag flag-fallback" aria-hidden="true">${esc(String(country || "").slice(0, 2).toUpperCase())}</span>`;
  return `<img class="flag" alt="" src="data:image/svg+xml,${encodeURIComponent(svg)}">`;
}

function langOption(item) {
  const selected = item.id === lang ? "true" : "false";
  return `<li role="option" tabindex="-1" lang="${esc(item.id)}" data-lang-option="${esc(item.id)}" aria-selected="${selected}">${flagMarkup(item.country)}<span class="lang-code">${esc(item.code)}</span><span class="lang-name">${esc(item.name)}</span></li>`;
}

function langSwitch() {
  const current = LANGUAGES.find((item) => item.id === lang) || LANGUAGES[0];
  const options = LANGUAGES.map(langOption).join("");
  return `<div class="lang-switch" data-lang-select>
    <button class="lang-button" type="button" data-lang-button aria-haspopup="listbox" aria-expanded="false" aria-controls="lang-menu" aria-label="${h("language")}">${flagMarkup(current.country)}<span class="lang-code">${esc(current.code)}</span><span class="lang-caret" aria-hidden="true"></span></button>
    <ul class="lang-menu" id="lang-menu" role="listbox" hidden>${options}</ul>
  </div>`;
}

function closeLangMenus() {
  document.querySelectorAll(".lang-menu").forEach((menu) => {
    menu.hidden = true;
    const button = menu.parentElement && menu.parentElement.querySelector("[data-lang-button]");
    if (button) button.setAttribute("aria-expanded", "false");
  });
}

function bindLangSwitch() {
  document.querySelectorAll(".lang-switch").forEach((root) => {
    const button = root.querySelector("[data-lang-button]");
    const menu = root.querySelector(".lang-menu");
    if (!button || !menu) return;
    const options = () => [...menu.querySelectorAll("[data-lang-option]")];
    const openMenu = () => {
      closeLangMenus();
      menu.hidden = false;
      button.setAttribute("aria-expanded", "true");
      const current = options().find((item) => item.dataset.langOption === lang) || options()[0];
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
        if (option.dataset.langOption !== lang) setLang(option.dataset.langOption);
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
          if (option.dataset.langOption !== lang) setLang(option.dataset.langOption);
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

function addLink(href, label) {
  return `<a class="btn pill primary" href="${href}">${h(label)}</a>`;
}

function deny(title, min) {
  const text = min === "admin"
    ? "you_need_an_administrator_account"
    : min === "owner"
      ? "you_need_the_owner_role"
      : "you_need_the_operator_role";
  shell(t(title), `<div class="card">${h(text)}</div>`);
}

function roleOf() {
  if (!me) return "viewer";
  if (me.is_admin) return "owner";
  const team = (me.teams || []).find((item) => item.id === teamId);
  return team ? team.role : "viewer";
}

function rank(role) {
  return { viewer: 1, operator: 2, owner: 3 }[role] || 0;
}

function can(min) {
  return rank(roleOf()) >= rank(min);
}

function roleLabel(role) {
  return {
    viewer: t("viewer"),
    operator: t("operator"),
    owner: t("owner"),
  }[role] || role;
}

function roleOptions(selected) {
  const labels = { viewer: "viewer", operator: "operator", owner: "owner" };
  return ["viewer", "operator", "owner"].map((role) =>
    `<option value="${role}"${role === selected ? " selected" : ""}>${h(labels[role])}</option>`
  ).join("");
}

function sourceLabel(source) {
  if (source === "ldap") return "LDAP";
  if (source === "manual") return t("manual");
  if (source === "local") return t("local");
  return source || "";
}

function authSourceLabel(source) {
  return sourceLabel(source === "local" ? "local" : source);
}

function roleLegend() {
  return `<ul class="role-legend">
    <li><strong>${h("viewer")}</strong> ${h("a_viewer_sees_projects_inventories_templates_jobs_and")}</li>
    <li><strong>${h("operator")}</strong> ${h("an_operator_launches_jobs_and_maintains_projects_inventories")}</li>
    <li><strong>${h("owner")}</strong> ${h("an_owner_also_manages_credentials_environments_and_members")}</li>
    <li><strong>${h("administrator")}</strong> ${h("an_administrator_manages_organizations_teams_users_settings_and")}</li>
  </ul>`;
}

function statusLabel(status) {
  return {
    pending: t("pending"),
    running: t("running"),
    successful: t("successful"),
    failed: t("failed"),
    canceled: t("canceled"),
    skipped: t("skipped"),
    accepted: t("accepted"),
    ignored: t("ignored"),
  }[status] || status;
}

function pill(status) {
  return `<span class="pill ${esc(status)}">${esc(statusLabel(status))}</span>`;
}

function clock(value) {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleTimeString(lang === "en" ? "en-GB" : "de-DE", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  });
}

function spanText(start, end, phase) {
  if (!start || phase === "pending" || phase === "skipped") return "—";
  const from = Date.parse(start);
  if (Number.isNaN(from)) return "—";
  const to = end ? Date.parse(end) : phase === "running" ? Date.now() : NaN;
  if (Number.isNaN(to)) return "—";
  const secs = Math.max(0, Math.round((to - from) / 1000));
  const mins = Math.floor(secs / 60);
  const rest = secs % 60;
  if (mins <= 0) return `${secs}s`;
  return `${mins}m ${String(rest).padStart(2, "0")}s`;
}

function bytes(value) {
  const n = Number(value) || 0;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / 1024 / 1024).toFixed(1)} MiB`;
}

function closeLive() {
  generation += 1;
  if (source) {
    source.close();
    source = null;
  }
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  if (runKey) {
    document.removeEventListener("keydown", runKey);
    runKey = null;
  }
}

async function api(method, path, body) {
  const headers = {};
  if (teamId) headers["X-Team-Id"] = teamId;
  const init = { method, headers, credentials: "same-origin" };
  if (body !== undefined) {
    headers["Content-Type"] = "application/json";
    init.body = JSON.stringify(body);
  }
  const response = await fetch(path, init);
  const text = await response.text();
  let data = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = { error: text };
    }
  }
  if (!response.ok) {
    const error = new Error((data && data.error) || "request.failed");
    error.status = response.status;
    error.vars = (data && data.vars) || null;
    throw error;
  }
  return data;
}

function handle(error) {
  if (error && error.status === 401) {
    me = null;
    renderLogin("session.expired");
    return;
  }
  if (!document.querySelector(".main")) shell(t("notice"), "");
  const main = document.querySelector(".main");
  main.insertAdjacentHTML("afterbegin", `<div class="banner bad">${h(error.message || "error.generic", error.vars)}</div>`);
}

function go(hash) {
  if ((location.hash || "#/") === hash) route();
  else location.hash = hash;
}

function navActive(here, href) {
  if (href === "#/") return here === "#/" || here === "";
  if (href === "#/runtime" && here.startsWith("#/environments")) return true;
  return here === href || here.startsWith(`${href}/`);
}

function shell(title, body, actions) {
  const here = (location.hash || "#/").split("?")[0];
  const groups = [
    [t("automation"), [
      ["#/", t("overview")],
      ["#/templates", t("templates")],
      ["#/jobs", t("jobs")],
      ["#/workflows", t("workflows")],
      ["#/schedules", t("schedules")],
      ["#/hooks", t("webhooks")],
    ]],
    [t("resources"), [
      ["#/projects", t("projects")],
      ["#/inventories", t("inventories")],
      ["#/credentials", t("credentials")],
      ["#/runtime", t("runtime")],
    ]],
    [t("nav.access"), [
      ["#/organizations", t("organizations")],
      ["#/team", t("team")],
      ...(me.is_admin ? [["#/settings", t("settings")], ["#/users", t("users")], ["#/backup", t("backup")]] : []),
    ]],
  ];
  const nav = groups.map(([label, links]) => `
    <div class="nav-group">
      <div class="nav-label">${label}</div>
      ${links.map(([href, text]) => `<a href="${href}" class="${navActive(here, href) ? "active" : ""}">${text}</a>`).join("")}
    </div>`).join("");
  const options = (me.teams || []).map((team) => {
    const label = team.organization_name ? `${team.organization_name} · ${team.name}` : team.name;
    return `<option value="${esc(team.id)}"${team.id === teamId ? " selected" : ""}>${esc(label)}</option>`;
  }).join("");
  app.innerHTML = `
    <div class="shell">
      <aside class="side">
        <div class="brand-lockup"><span class="mark" aria-hidden="true"></span><p class="word">Koreograf</p></div>
        <label class="field"><span class="small">${h("organization_team")}</span><select id="team-select">${options || `<option value="">${h("no_team")}</option>`}</select></label>
        <nav class="nav">${nav}</nav>
        <div class="side-foot">
          <a class="account-link" href="#/account">${esc(me.display_name)} · ${esc(roleLabel(roleOf()))}${me.auth_source === "ldap" ? " · LDAP" : ""}</a>
          ${langSwitch()}
          <button class="btn ghost" id="logout" type="button">${h("sign_out")}</button>
        </div>
      </aside>
      <main class="main">
        <div class="top"><div><h1>${esc(title)}</h1></div><div class="row">${actions || ""}</div></div>
        ${body}
      </main>
    </div>`;
  const select = document.querySelector("#team-select");
  select.addEventListener("change", () => {
    teamId = select.value;
    localStorage.setItem("koreograf.team", teamId);
    route();
  });
  bindLangSwitch();
  document.querySelector("#logout").addEventListener("click", async () => {
    try { await api("POST", "/api/logout"); } catch { /* session ended locally */ }
    me = null;
    go("#/login");
  });
}

function needTeam() {
  if (teamId) return true;
  shell(t("team"), `<div class="card"><p>${h("this_account_has_no_team_an_administrator_creates")}</p><p><a href="#/team">${h("open_team")}</a></p></div>`);
  return false;
}

function table(headers, rows) {
  const body = rows.length
    ? rows.join("")
    : `<tr><td class="muted" colspan="${headers.length}">${h("nothing_here")}</td></tr>`;
  return `<table class="table"><thead><tr>${headers.map((item) => `<th>${esc(item)}</th>`).join("")}</tr></thead><tbody>${body}</tbody></table>`;
}

function jsonObject(text, label) {
  let value;
  try {
    value = JSON.parse(text.trim() || "{}");
  } catch {
    const error = new Error("field.not_json");
    error.vars = { field: label };
    throw error;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    const error = new Error("field.not_object");
    error.vars = { field: label };
    throw error;
  }
  return value;
}

function renderLogin(message, tone, vars) {
  const kind = tone === "good" ? "good" : "bad";
  app.innerHTML = `
    <div class="login-wrap">
      <form class="login-card" id="login-form">
        <div class="spread">
          <div class="brand-lockup"><span class="mark" aria-hidden="true"></span><h1 class="word">Koreograf</h1></div>
          ${langSwitch()}
        </div>
        <p class="lede">${h("jobs_workflows_and_alerts_for_several_teams_local")}</p>
        ${message ? `<div class="banner ${kind}">${h(message, vars)}</div>` : ""}
        <div class="form">
          <label class="field"><span>${h("username")}</span><input name="username" autocomplete="username" required></label>
          <label class="field"><span>${h("password")}</span><input name="password" type="password" autocomplete="current-password" required></label>
          <button class="btn primary" type="submit">${h("sign_in")}</button>
        </div>
      </form>
    </div>`;
  bindLangSwitch();
  document.querySelector("#login-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const button = form.querySelector("button[type=submit]");
    button.disabled = true;
    try {
      me = await api("POST", "/api/login", {
        username: form.username.value,
        password: form.password.value,
      });
      const known = (me.teams || []).some((team) => team.id === teamId);
      if (!known) teamId = (me.teams[0] && me.teams[0].id) || "";
      localStorage.setItem("koreograf.team", teamId);
      go("#/");
    } catch (error) {
      renderLogin(error.message || "login.rejected", "bad", error.vars);
    }
  });
}

async function pageDashboard() {
  if (!needTeam()) return;
  const data = await api("GET", "/api/overview");
  const jobs = (data.jobs || []).map((job) =>
    `<tr><td><a href="#/jobs/${esc(job.id)}">${esc(job.name)}</a></td><td class="tool">${esc(job.tool)}</td><td>${pill(job.status)}</td><td class="small muted">${esc(job.created_at)}</td></tr>`
  );
  const deliveries = (data.deliveries || []).map((item) =>
    `<tr><td>${esc(item.hook_name)}</td><td>${pill(item.status)}</td><td>${esc(item.summary)}</td></tr>`
  );
  shell(t("overview"), `
    <div class="grid-3">
      <article class="card stat"><span class="small muted">${h("dependency_cache")}</span><strong>${esc(bytes(data.cache_bytes))}</strong><p class="small muted">${h("collections_roles_and_opentofu_providers_stay_between_runs")}</p></article>
      <article class="card stat"><span class="small muted">${h("workflow")}</span><strong>${data.workflow_id ? `<a href="#/workflows/${esc(data.workflow_id)}">${esc(data.workflow_name)}</a>` : "—"}</strong><p class="small muted">${h("success_edges_and_failure_edges_start_the_next")}</p></article>
      <article class="card stat"><span class="small muted">${h("webhook")}</span><strong>${data.hook_id ? esc(data.hook_name) : "—"}</strong><p class="small muted">${data.ldap ? h("ldap_is_configured") : h("local_accounts")} ${h("runner")}: ${esc(data.runner_mode)}${data.runtime ? `, ${esc(data.runtime)}` : ""}.</p></article>
    </div>
    <div class="stack" style="margin-top:14px">
      <section class="card">
        <div class="spread"><h2>${h("recent_jobs")}</h2><a href="#/jobs">${h("all")}</a></div>
        ${table([t("name"), t("tool"), t("status"), t("created")], jobs)}
      </section>
      <section class="card">
        <div class="spread">
          <h2>${h("webhook_deliveries")}</h2>
          ${data.hook_id && can("operator") ? `<button class="btn primary" id="test-alert" type="button">${h("test_alert")}</button>` : ""}
        </div>
        ${table([t("hook"), t("status"), t("summary")], deliveries)}
      </section>
    </div>`);
  const test = document.querySelector("#test-alert");
  if (test) {
    test.addEventListener("click", async () => {
      test.disabled = true;
      try {
        const result = await api("POST", `/api/hooks/${encodeURIComponent(data.hook_id)}/test`);
        if (result.workflow_job_id) go(`#/runs/${result.workflow_job_id}`);
        else if (result.job_id) go(`#/jobs/${result.job_id}`);
        else route();
      } catch (error) {
        test.disabled = false;
        handle(error);
      }
    });
  }
}

function selectOptions(items, selected, allowEmpty, label) {
  const empty = allowEmpty ? `<option value="">—</option>` : "";
  const options = items.map((item) =>
    `<option value="${esc(item.id)}"${item.id === selected ? " selected" : ""}>${esc(label(item))}</option>`
  ).join("");
  return empty + options;
}

function verbosityLabel(level) {
  return [
    "0_normal",
    "1_verbose",
    "2_more_verbose",
    "3_debug",
    "4_connection_debug",
  ][Number(level)] || "";
}

function parseTagList(value) {
  return String(value || "").split(",").map((part) => part.trim()).filter(Boolean);
}

function validTag(part) {
  return part.length > 0
    && part.length <= 80
    && !part.startsWith("-")
    && /^[A-Za-z0-9_.:-]+$/.test(part);
}

function splitDraft(raw, flush) {
  const parts = String(raw || "").split(/[,\s]+/);
  const trailing = /[,\s]$/.test(raw);
  if (flush || trailing) return { tokens: parts.filter(Boolean), rest: "" };
  return { tokens: parts.slice(0, -1).filter(Boolean), rest: parts.at(-1) || "" };
}

function readChips(box) {
  return [...box.querySelectorAll(".chip-text")].map((node) => node.textContent);
}

function renderChips(box, tokens) {
  const entry = box.querySelector(".chip-entry");
  box.querySelectorAll(".chip").forEach((node) => node.remove());
  const html = tokens.map((token) => `<span class="chip"><span class="chip-text">${esc(token)}</span><button class="chip-remove" type="button" aria-label="${h("remove_tag_name", { name: token })}">×</button></span>`).join("");
  entry.insertAdjacentHTML("beforebegin", html);
  entry.placeholder = tokens.length ? "" : t("enter_a_tag");
  const hidden = box.parentElement.querySelector(`input[type="hidden"][name="${CSS.escape(box.dataset.chips)}"]`);
  if (hidden) hidden.value = tokens.join(",");
  box.querySelectorAll(".chip-remove").forEach((button) => {
    button.addEventListener("mousedown", (event) => event.preventDefault());
    button.addEventListener("click", () => {
      const name = button.parentElement.querySelector(".chip-text").textContent;
      renderChips(box, readChips(box).filter((item) => item !== name));
      const error = box.parentElement.querySelector(".chip-error");
      if (error) error.textContent = "";
      entry.focus();
    });
  });
}

function addTokens(box, tokens) {
  const error = box.parentElement.querySelector(".chip-error");
  const chips = readChips(box);
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (chips.includes(token)) continue;
    const tooLong = [...chips, token].join(",").length > 300;
    if (!validTag(token) || tooLong) {
      if (error) {
        error.textContent = tooLong
          ? t("the_tags_are_too_long")
          : t("the_tag_part_is_invalid_letters_digits_and", { part: token });
      }
      renderChips(box, chips);
      return { ok: false, keep: tokens.slice(index) };
    }
    chips.push(token);
  }
  if (error) error.textContent = "";
  renderChips(box, chips);
  return { ok: true, keep: [] };
}

function takeDraft(box, flush) {
  const entry = box.querySelector(".chip-entry");
  const { tokens, rest } = splitDraft(entry.value, flush);
  if (!tokens.length) {
    entry.value = rest;
    return true;
  }
  const result = addTokens(box, tokens);
  const leftover = result.ok ? rest : [...result.keep, rest].filter(Boolean).join(" ");
  entry.value = leftover;
  return result.ok;
}

function bindChips(form) {
  form.querySelectorAll("[data-chips]").forEach((box) => {
    const hidden = box.parentElement.querySelector(`input[type="hidden"][name="${CSS.escape(box.dataset.chips)}"]`);
    renderChips(box, parseTagList(hidden ? hidden.value : ""));
    const entry = box.querySelector(".chip-entry");
    const error = box.parentElement.querySelector(".chip-error");
    entry.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === "," || event.key === " ") {
        event.preventDefault();
        takeDraft(box, true);
      } else if (event.key === "Backspace" && entry.value === "") {
        const tokens = readChips(box);
        if (!tokens.length) return;
        event.preventDefault();
        renderChips(box, tokens.slice(0, -1));
        if (error) error.textContent = "";
      }
    });
    entry.addEventListener("input", () => {
      if (error) error.textContent = "";
      if (/[,\s]/.test(entry.value)) takeDraft(box, false);
    });
    entry.addEventListener("blur", () => takeDraft(box, true));
    box.addEventListener("click", (event) => {
      if (event.target === box) entry.focus();
    });
  });
}

function flushChips(form) {
  let ok = true;
  form.querySelectorAll("[data-chips]").forEach((box) => {
    if (box.closest("[hidden]")) return;
    if (!takeDraft(box, true)) ok = false;
  });
  return ok;
}

function chipBox(name, label, value) {
  return `<label class="field chips-field"><span>${h(label)}</span>
    <div class="chips" data-chips="${esc(name)}">
      <input class="chip-entry" type="text" autocomplete="off" aria-label="${h(label)}">
    </div>
    <input type="hidden" name="${esc(name)}" value="${esc(parseTagList(value).join(","))}">
    <span class="chip-error" aria-live="polite"></span>
  </label>`;
}

function chipReadout(value) {
  const tokens = parseTagList(value);
  if (!tokens.length) return "";
  return `<span class="chips readonly">${tokens.map((token) => `<span class="chip"><span class="chip-text">${esc(token)}</span></span>`).join("")}</span>`;
}

function templateSpec(form) {
  if (form.tool.value === "ansible") {
    const spec = { job_type: form.job_type.value === "check" ? "check" : "run" };
    const tags = form.tags.value.trim();
    const skip = form.skip_tags.value.trim();
    const limit = form.limit.value.trim();
    const forks = form.forks.value.trim();
    if (tags) spec.tags = tags;
    if (skip) spec.skip_tags = skip;
    if (limit) spec.limit = limit;
    if (form.diff.checked) spec.diff = true;
    const verbosity = Number(form.verbosity.value);
    if (verbosity) spec.verbosity = verbosity;
    if (forks) {
      const parsed = Number(forks);
      if (!Number.isInteger(parsed)) throw new Error("forks_is_a_whole_number");
      spec.forks = parsed;
    }
    return spec;
  }
  if (form.tool.value === "opentofu") {
    return { action: form.action.value === "apply" ? "apply" : "plan" };
  }
  const script = form.script.value;
  return script.trim() ? { script } : {};
}

function templateOptionLine(item) {
  const spec = item.spec || {};
  if (item.tool === "ansible") {
    const bits = [spec.job_type === "check" ? t("check") : t("run_2")];
    if (spec.verbosity) bits.push(t(verbosityLabel(spec.verbosity)));
    if (spec.tags) bits.push(`${t("tags")} ${spec.tags}`);
    if (spec.skip_tags) bits.push(`${t("skip")} ${spec.skip_tags}`);
    if (spec.limit) bits.push(`${t("limit")} ${spec.limit}`);
    if (spec.diff) bits.push(t("diff"));
    return bits.join(" · ");
  }
  if (item.tool === "opentofu") return spec.action === "apply" ? "apply" : "plan";
  return item.entrypoint || t("script");
}

async function pageTemplates() {
  if (!needTeam()) return;
  const items = await api("GET", "/api/templates");
  shell(t("templates"), `
    <section class="card">
      <div class="spread">
        <input class="search" id="template-search" placeholder="${h("search_templates")}" autocomplete="off">
        <div class="choice-row" id="template-filters">
          <button class="btn pill primary" type="button" data-tool="all">${h("all")}</button>
          <button class="btn pill" type="button" data-tool="ansible">Ansible</button>
          <button class="btn pill" type="button" data-tool="opentofu">OpenTofu</button>
          <button class="btn pill" type="button" data-tool="shell">Shell</button>
        </div>
      </div>
      <div id="template-table"></div>
    </section>`,
    can("operator") ? addLink("#/templates/new", "template_2") : "");
  let tool = "all";
  let query = "";
  const draw = () => {
    const needle = query.trim().toLowerCase();
    const shown = items.filter((item) => {
      if (tool !== "all" && item.tool !== tool) return false;
      if (!needle) return true;
      const spec = item.spec || {};
      return [item.name, item.description, item.tool, item.entrypoint, spec.tags, spec.skip_tags, spec.limit]
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
    const rows = shown.map((item) => `
      <tr>
        <td><a href="#/templates/${esc(item.id)}">${esc(item.name)}</a><div class="small muted">${esc(item.description)}</div></td>
        <td class="tool">${esc(item.tool)}</td>
        <td class="mono">${esc(item.entrypoint || "—")}</td>
        <td class="small">${esc(templateOptionLine(item))}</td>
        <td>${can("operator") ? `<button class="btn primary" type="button" data-launch="${esc(item.id)}">${h("launch")}</button>` : ""}</td>
      </tr>`);
    document.querySelector("#template-table").innerHTML = table([t("name"), t("tool"), t("entry"), t("run_2"), ""], rows);
    document.querySelectorAll("[data-launch]").forEach((button) => {
      button.addEventListener("click", async () => {
        button.disabled = true;
        try {
          const job = await api("POST", `/api/templates/${encodeURIComponent(button.dataset.launch)}/launch`, { extra_vars: {} });
          go(`#/jobs/${job.id}`);
        } catch (error) {
          button.disabled = false;
          handle(error);
        }
      });
    });
  };
  document.querySelector("#template-search").addEventListener("input", (event) => {
    query = event.target.value;
    draw();
  });
  document.querySelectorAll("#template-filters button").forEach((button) => {
    button.addEventListener("click", () => {
      tool = button.dataset.tool;
      document.querySelectorAll("#template-filters button").forEach((item) => {
        item.classList.toggle("primary", item === button);
      });
      draw();
    });
  });
  draw();
}

async function pageTemplateForm(id) {
  if (!needTeam()) return;
  const [projects, inventories, credentials, environments, templates] = await Promise.all([
    api("GET", "/api/projects"),
    api("GET", "/api/inventories"),
    api("GET", "/api/credentials"),
    api("GET", "/api/environments"),
    api("GET", "/api/templates"),
  ]);
  const existing = id ? templates.find((item) => item.id === id) : null;
  if (id && !existing) {
    shell(t("template"), `<div class="card">${h("this_template_does_not_exist")}</div>`);
    return;
  }
  const spec = (existing && existing.spec) || {};
  const item = existing || {
    name: "", description: "", tool: "ansible", project_id: "", inventory_id: "",
    credential_id: "", credential_ids: [], environment_id: "", entrypoint: "", extra_vars: {}, spec: {},
  };
  const verbosity = Number(spec.verbosity) || 0;
  shell(id ? t("edit_template") : t("new_template"), `
    <form class="card form" id="template-form">
      <div class="form-section">
        <h2>${h("general")}</h2>
        <label class="field"><span>${h("name")}</span><input name="name" required value="${esc(item.name)}"></label>
        <label class="field"><span>${h("description")}</span><input name="description" value="${esc(item.description)}"></label>
        <div class="field-row">
          <label class="field"><span>${h("tool")}</span>
            <select class="fit" name="tool">
              <option value="ansible"${item.tool === "ansible" ? " selected" : ""}>Ansible</option>
              <option value="opentofu"${item.tool === "opentofu" ? " selected" : ""}>OpenTofu</option>
              <option value="shell"${item.tool === "shell" ? " selected" : ""}>Shell</option>
            </select>
          </label>
        </div>
      </div>
      <div class="form-section">
        <h2>${h("target")}</h2>
        <div class="field-grid">
          <label class="field"><span>${h("project")}</span><select name="project_id">${selectOptions(projects, item.project_id, true, (row) => row.name)}</select></label>
          <div id="update-on-launch" hidden>
            <label class="field check"><span>${h("update_the_project_on_launch")}</span><input name="update_on_launch" type="checkbox"${item.update_on_launch === false ? "" : " checked"}></label>
            <p class="small muted">${h("off_uses_the_checkout_from_the_last_sync")}</p>
          </div>
          <label class="field" id="inventory-field"${item.tool === "ansible" ? "" : " hidden"}><span>${h("inventory")}</span><select name="inventory_id">${selectOptions(inventories, item.inventory_id, true, (row) => row.name)}</select></label>
          <label class="field"><span>${h("execution_environment")}</span><select name="environment_id">${selectOptions(environments, item.environment_id, true, (row) => `${row.name} (${scopeLabel(row)})`)}</select></label>
        </div>
        ${templateCredentialFields(credentials, selectedCredentialIds(item))}
        <label class="field" id="playbook-field" hidden><span>${h("playbook_from_the_project")}</span><select name="playbook"><option value="">${h("other_path")}</option></select></label>
        <p class="small muted" id="entrypoint-note"></p>
        <label class="field"><span id="entrypoint-label">${h("entry_path")}</span><input name="entrypoint" value="${esc(item.entrypoint)}" placeholder="site.yml"></label>
        <p class="small muted">${h("without_an_execution_environment_the_template_runs_on")}</p>
      </div>
      <div class="form-section" id="ansible-options"${item.tool === "ansible" ? "" : " hidden"}>
        <h2>Ansible</h2>
        <div class="field-row">
          <label class="field"><span>${h("type")}</span>
            <select class="fit" name="job_type">
              <option value="run"${spec.job_type === "check" ? "" : " selected"}>${h("run_2")}</option>
              <option value="check"${spec.job_type === "check" ? " selected" : ""}>${h("check_dry_run")}</option>
            </select>
          </label>
          <label class="field"><span>${h("verbosity")}</span>
            <select class="fit" name="verbosity">
              ${[0, 1, 2, 3, 4].map((level) => `<option value="${level}"${verbosity === level ? " selected" : ""}>${h(verbosityLabel(level))}</option>`).join("")}
            </select>
          </label>
          <label class="field"><span>${h("forks")}</span><input class="tiny" name="forks" inputmode="numeric" value="${esc(spec.forks || "")}" placeholder="5"></label>
          <label class="field check"><span>${h("diff")}</span><input name="diff" type="checkbox"${spec.diff ? " checked" : ""}></label>
        </div>
        <div class="field-row">
          ${chipBox("tags", "tags", spec.tags)}
          ${chipBox("skip_tags", "skip_tags", spec.skip_tags)}
        </div>
        <p class="small muted">${h("a_comma_or_a_space_turns_the_typed")}</p>
        <div class="field-row">
          <label class="field"><span>${h("limit")}</span><input class="fit" name="limit" value="${esc(spec.limit || "")}" placeholder="web:&db"></label>
        </div>
        <p class="small muted">${h("check_starts_ansible_playbook_with_check_tags_skipped")}</p>
      </div>
      <div class="form-section" id="tofu-options"${item.tool === "opentofu" ? "" : " hidden"}>
        <h2>OpenTofu</h2>
        <div class="field-row">
          <label class="field"><span>${h("action")}</span>
            <select class="fit" name="action">
              <option value="plan"${spec.action === "apply" ? "" : " selected"}>plan</option>
              <option value="apply"${spec.action === "apply" ? " selected" : ""}>apply</option>
            </select>
          </label>
        </div>
      </div>
      <div class="form-section" id="shell-options"${item.tool === "shell" ? "" : " hidden"}>
        <h2>Shell</h2>
        <label class="field"><span>${h("script_when_no_entry_path_is_set")}</span><textarea name="script">${esc(spec.script || "")}</textarea></label>
      </div>
      <div class="form-section">
        <h2>${h("variables")}</h2>
        <label class="field"><span>${h("extra_variables_json")}</span><textarea name="extra_vars">${esc(JSON.stringify(item.extra_vars || {}, null, 2))}</textarea></label>
      </div>
      <div class="row">
        <button class="btn pill primary" type="submit">${id ? h("save") : h("create")}</button>
        ${id && can("operator") ? `<button class="btn danger" id="delete-template" type="button">${h("delete")}</button>` : ""}
      </div>
    </form>
    ${id ? `<section class="card" id="schedule-card" data-kind="template" data-target="${esc(id)}"></section>` : ""}`);
  const form = document.querySelector("#template-form");
  bindChips(form);
  bindVaultPicker(form, credentials, selectedCredentialIds(item));
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!flushChips(form)) return;
    try {
      const selectedProject = projects.find((item) => item.id === form.project_id.value);
      const body = {
        name: form.name.value,
        description: form.description.value,
        tool: form.tool.value,
        project_id: form.project_id.value || null,
        inventory_id: form.inventory_id.value || null,
        credential_ids: [
          ...[...form.querySelectorAll("[data-credential-kind]")].map((select) => select.value).filter(Boolean),
          ...[...form.querySelectorAll("#vault-pills [data-vault-id]")].map((node) => node.dataset.vaultId),
        ],
        environment_id: form.environment_id.value || null,
        update_on_launch: selectedProject && selectedProject.source_type === "git" ? form.update_on_launch.checked : true,
        entrypoint: form.tool.value === "ansible" && form.playbook.value ? form.playbook.value : form.entrypoint.value,
        extra_vars: jsonObject(form.extra_vars.value, "extra_variables"),
        spec: templateSpec(form),
      };
      if (id) await api("PUT", `/api/templates/${encodeURIComponent(id)}`, body);
      else await api("POST", "/api/templates", body);
      go("#/templates");
    } catch (error) {
      handle(error);
    }
  });
  const loadPlaybooks = async () => {
    const field = document.querySelector("#playbook-field");
    const note = document.querySelector("#entrypoint-note");
    const select = form.playbook;
    const ansible = form.tool.value === "ansible" && form.project_id.value;
    field.hidden = !ansible;
    note.textContent = "";
    if (!ansible) return;
    const current = form.entrypoint.value.trim();
    try {
      const contents = await api("GET", `/api/projects/${encodeURIComponent(form.project_id.value)}/contents`);
      const books = contents.playbooks || [];
      const known = books.map((path) => `<option value="${esc(path)}">${esc(path)}</option>`).join("");
      const extra = current && !books.includes(current)
        ? `<option value="${esc(current)}">${esc(current)}</option>`
        : "";
      select.innerHTML = `${known}${extra}<option value="">${h("other_path")}</option>`;
      select.value = current && (books.includes(current) || extra) ? current : "";
      if (!books.length) {
        note.textContent = t("no_playbooks_found_sync_the_git_project_first");
      }
    } catch (error) {
      select.innerHTML = `<option value="">${h("other_path")}</option>`;
      note.textContent = t(error.message || "playbooks_cannot_be_read", error.vars);
    }
  };
  const showToolSections = () => {
    const tool = form.tool.value;
    document.querySelector("#ansible-options").hidden = tool !== "ansible";
    document.querySelector("#tofu-options").hidden = tool !== "opentofu";
    document.querySelector("#shell-options").hidden = tool !== "shell";
    document.querySelector("#inventory-field").hidden = tool !== "ansible";
    const label = document.querySelector("#entrypoint-label");
    if (tool === "ansible") label.textContent = t("playbook_path");
    else if (tool === "opentofu") label.textContent = t("module_directory");
    else label.textContent = t("script_path");
  };
  form.tool.addEventListener("change", () => {
    showToolSections();
    loadPlaybooks();
  });
  showToolSections();
  const showProjectUpdate = () => {
    const row = projects.find((item) => item.id === form.project_id.value);
    const box = document.querySelector("#update-on-launch");
    if (box) box.hidden = !(row && row.source_type === "git");
  };
  form.project_id.addEventListener("change", () => {
    showProjectUpdate();
    loadPlaybooks();
  });
  showProjectUpdate();
  form.playbook.addEventListener("change", () => {
    if (form.playbook.value) form.entrypoint.value = form.playbook.value;
  });
  loadPlaybooks();
  const remove = document.querySelector("#delete-template");
  if (remove) {
    remove.addEventListener("click", async () => {
      if (!confirm(t("delete_this_template"))) return;
      try {
        await api("DELETE", `/api/templates/${encodeURIComponent(id)}`);
        go("#/templates");
      } catch (error) {
        handle(error);
      }
    });
  }
  if (id) mountSchedules(document.querySelector("#schedule-card"));
}

const SCHEDULE_ZONES = [
  "Europe/Berlin", "Europe/Vienna", "Europe/Zurich", "Europe/Paris", "Europe/Amsterdam",
  "Europe/London", "UTC", "America/New_York", "America/Chicago", "America/Los_Angeles",
  "Asia/Tokyo", "Asia/Singapore", "Australia/Sydney",
];
const SCHEDULE_DAYS = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

function browserZone() {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "Europe/Berlin";
  } catch (error) {
    return "Europe/Berlin";
  }
}

function zoneChoices(selected) {
  const zones = [...SCHEDULE_ZONES];
  const browser = browserZone();
  if (!zones.includes(browser)) zones.unshift(browser);
  if (selected && !zones.includes(selected)) zones.unshift(selected);
  return zones;
}

function localParts(zone) {
  const parts = new Intl.DateTimeFormat("en-GB", {
    timeZone: zone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
    weekday: "short",
  }).formatToParts(new Date());
  const get = (type) => parts.find((part) => part.type === type)?.value || "";
  const weekday = { Mon: "MO", Tue: "TU", Wed: "WE", Thu: "TH", Fri: "FR", Sat: "SA", Sun: "SU" }[get("weekday")] || "MO";
  let hour = get("hour");
  if (hour === "24") hour = "00";
  return {
    date: `${get("year")}-${get("month")}-${get("day")}`,
    time: `${hour.padStart(2, "0")}:${get("minute").padStart(2, "0")}`,
    weekday,
  };
}

function formatLocalDate(isoDate) {
  const [year, month, day] = String(isoDate || "").split("-");
  if (!year || !month || !day) return isoDate || "";
  return lang === "de" ? `${day}.${month}.${year}` : `${year}-${month}-${day}`;
}

function dayList(days) {
  return (days || []).map((day) => t(`schedule.day.${String(day).toLowerCase()}`)).join(", ");
}

function scheduleSentence(item) {
  const date = formatLocalDate(item.start_date);
  const time = item.start_time;
  const zone = item.timezone;
  const day = Number(String(item.start_date || "").slice(8, 10)) || "";
  let text = t("schedule.daily", { time, zone });
  if (item.frequency === "once") text = t("schedule.once", { date, time, zone });
  if (item.frequency === "hourly") text = t("schedule.hourly", { date, time, zone });
  if (item.frequency === "weekly") text = t("schedule.weekly", { days: dayList(item.weekdays), time, zone });
  if (item.frequency === "monthly") text = t("schedule.monthly", { day, time, zone });
  if (item.end_kind === "after" && item.end_count) text = `${text} ${t("schedule.ends_after", { count: item.end_count })}`;
  if (item.end_kind === "on_date" && item.end_date) text = `${text} ${t("schedule.ends_on", { date: formatLocalDate(item.end_date) })}`;
  return text;
}

function formatWhen(iso, zone) {
  if (!iso) return "";
  return new Intl.DateTimeFormat(lang === "de" ? "de-DE" : "en-GB", {
    dateStyle: "medium",
    timeStyle: "short",
    timeZone: zone || "UTC",
  }).format(new Date(iso));
}

function mountSchedules(card) {
  if (!card) return;
  const kind = card.dataset.kind;
  const target = card.dataset.target;
  const all = kind === "all";
  const path = all ? "/api/schedules" : `/api/${kind}s/${encodeURIComponent(target)}/schedules`;
  let items = [];
  let query = "";
  const empty = all
    ? h("schedule_empty_all")
    : h(kind === "workflow" ? "schedule_empty_workflow" : "schedule_empty_template");
  const render = () => {
    const needle = query.trim().toLowerCase();
    const shown = items
      .filter((item) => !needle || scheduleHaystack(item).includes(needle))
      .sort(compareSchedules);
    const host = card.querySelector(".schedule-table");
    const message = items.length ? h("nothing_here") : empty;
    host.innerHTML = scheduleTable(shown, all, message);
    bindScheduleActions(card, items, kind, target, draw);
  };
  const draw = async () => {
    items = await api("GET", path);
    render();
  };
  const search = all
    ? `<input class="search" id="schedule-search" placeholder="${h("search_schedules")}" autocomplete="off">`
    : "";
  const add = !all && can("operator") ? `<button class="btn" type="button" id="schedule-add">${h("schedule_add")}</button>` : "";
  const heading = all ? `<div class="spread">${search}</div>` : `<div class="row"><h2>${h("schedules")}</h2>${add}</div>`;
  card.innerHTML = `${heading}<div class="table-wrap schedule-table"></div>`;
  const searchBox = card.querySelector("#schedule-search");
  if (searchBox) {
    searchBox.addEventListener("input", () => {
      query = searchBox.value;
      render();
    });
  }
  const addButton = card.querySelector("#schedule-add");
  if (addButton) addButton.addEventListener("click", () => openScheduleForm(kind, target, null, draw));
  draw().catch(handle);
}

function bindScheduleActions(card, items, kind, target, draw) {
  card.querySelectorAll("[data-schedule-edit]").forEach((button) => {
    button.addEventListener("click", () => {
      const item = items.find((row) => row.id === button.dataset.scheduleEdit);
      if (item) openScheduleForm(item.target_kind || kind, item.target_id || target, item, draw);
    });
  });
  card.querySelectorAll("[data-schedule-delete]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (!confirm(t("delete_this_schedule"))) return;
      try {
        await api("DELETE", `/api/schedules/${encodeURIComponent(button.dataset.scheduleDelete)}`);
        await draw();
      } catch (error) {
        handle(error);
      }
    });
  });
  card.querySelectorAll("[data-schedule-enabled]").forEach((control) => {
    const apply = async () => {
      const item = items.find((row) => row.id === control.dataset.scheduleEnabled);
      if (!item) return;
      const enabled = control.type === "checkbox" ? control.checked : !item.enabled;
      try {
        await api("POST", `/api/schedules/${encodeURIComponent(item.id)}/enabled`, { enabled });
        await draw();
      } catch (error) {
        handle(error);
        if (control.isConnected && control.type === "checkbox") control.checked = item.enabled;
      }
    };
    if (control.type === "checkbox") control.addEventListener("change", apply);
    else control.addEventListener("click", apply);
  });
  card.querySelectorAll("[data-schedule-accept], [data-schedule-decline]").forEach((button) => {
    button.addEventListener("click", async () => {
      const id = button.dataset.scheduleAccept || button.dataset.scheduleDecline;
      try {
        await api("POST", `/api/schedules/${encodeURIComponent(id)}/missed`, { accept: Boolean(button.dataset.scheduleAccept) });
        await draw();
      } catch (error) {
        handle(error);
      }
    });
  });
}

function scheduleHaystack(item) {
  return [
    item.name,
    item.target_name,
    item.timezone,
    item.start_time,
    item.created_by,
    scheduleRepeat(item),
    scheduleEnd(item),
    scheduleSentence(item),
  ].join(" ").toLowerCase();
}

function compareSchedules(a, b) {
  const rank = (item) => {
    if (item.missed_at) return 0;
    if (item.enabled && item.next_run_at) return 1;
    if (item.enabled) return 2;
    return 3;
  };
  const byRank = rank(a) - rank(b);
  if (byRank) return byRank;
  const left = a.next_run_at || "9999";
  const right = b.next_run_at || "9999";
  if (left !== right) return left < right ? -1 : 1;
  return String(a.name).localeCompare(String(b.name));
}

function scheduleRepeat(item) {
  if (item.frequency === "weekly") {
    const days = (item.weekdays || []).map((day) => t(`schedule.short.${String(day).toLowerCase()}`));
    return days.length ? days.join(", ") : t("weekly");
  }
  if (item.frequency === "monthly") {
    const day = Number(String(item.start_date || "").slice(8, 10)) || "";
    return day ? t("schedule.day_of_month", { day }) : t("monthly");
  }
  return t(item.frequency || "daily");
}

function scheduleEnd(item) {
  if (item.end_kind === "after" && item.end_count) return t("schedule.after_count", { count: item.end_count });
  if (item.end_kind === "on_date" && item.end_date) return formatLocalDate(item.end_date);
  return t("never");
}

function scheduleNext(item) {
  const when = item.next_run_at
    ? esc(formatWhen(item.next_run_at, item.timezone))
    : `<span class="muted">${h("schedule_no_upcoming")}</span>`;
  const missed = item.missed_at ? `<div><span class="pill warn">${h("schedule_missed_state")}</span></div>` : "";
  return `${missed}${when}`;
}

function scheduleTable(items, showTarget, emptyMessage) {
  const columns = [
    ["name", ""],
    ...(showTarget ? [["schedule_target", ""]] : []),
    ["repeat", ""],
    ["start_time", ""],
    ["next_run", ""],
    ["ends", ""],
    ["schedule_active", "tight"],
    ["actions", "actions"],
  ];
  const body = items.length
    ? items.map((item) => scheduleTableRows(item, showTarget, columns.length)).join("")
    : `<tr><td class="muted" colspan="${columns.length}">${emptyMessage}</td></tr>`;
  const head = columns.map(([key, kind]) => `<th class="${kind}">${esc(t(key))}</th>`).join("");
  return `<table class="table"><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table>`;
}

function scheduleTableRows(item, showTarget, span) {
  const target = showTarget ? `<td>${scheduleTarget(item)}</td>` : "";
  const edit = can("operator") ? `
    <button class="btn compact" type="button" data-schedule-edit="${esc(item.id)}">${h("schedule_edit")}</button>
    <button class="btn compact danger" type="button" data-schedule-delete="${esc(item.id)}">${h("delete")}</button>` : "";
  const by = item.created_by ? `<div class="small muted">${h("schedule.by", { name: item.created_by })}</div>` : "";
  const alert = item.missed_at ? `
    <tr class="schedule-alert">
      <td colspan="${span}">
        <div class="schedule-miss">
          <span>${h("schedule.missed", { when: formatWhen(item.missed_at, item.timezone) })}</span>
          ${can("operator") ? `<button class="btn compact primary" type="button" data-schedule-accept="${esc(item.id)}">${h("run_missed_now")}</button><button class="btn compact" type="button" data-schedule-decline="${esc(item.id)}">${h("skip_missed")}</button>` : ""}
        </div>
      </td>
    </tr>` : "";
  return `<tr class="${item.missed_at ? "schedule-due" : ""}">
    <td><strong>${esc(item.name)}</strong>${by}</td>
    ${target}
    <td>${esc(scheduleRepeat(item))}</td>
    <td class="num">${esc(item.start_time || "")}<div class="small muted">${esc(item.timezone || "")}</div></td>
    <td class="num">${scheduleNext(item)}</td>
    <td>${esc(scheduleEnd(item))}</td>
    <td class="tight">${scheduleSwitch(item)}</td>
    <td class="actions">${edit}</td>
  </tr>${alert}`;
}

function scheduleSwitch(item) {
  const checked = item.enabled ? " checked" : "";
  const disabled = can("operator") ? "" : " disabled";
  return `<label class="switch"><input type="checkbox" data-schedule-enabled="${esc(item.id)}" aria-label="${h(item.enabled ? "pause_schedule" : "resume_schedule")}"${checked}${disabled}><span></span></label>`;
}

function scheduleTarget(item) {
  if (!item.target_id) return "";
  const workflow = item.target_kind === "workflow";
  const href = workflow ? `#/workflows/${encodeURIComponent(item.target_id)}` : `#/templates/${encodeURIComponent(item.target_id)}`;
  const label = workflow ? t("workflow") : t("template");
  const name = item.target_name || label;
  return `<a href="${href}">${esc(name)}</a><div class="small muted">${esc(label)}</div>`;
}

function openScheduleForm(kind, target, existing, redraw) {
  document.querySelector("#schedule-form")?.remove();
  const zone = existing?.timezone || browserZone();
  const local = localParts(zone);
  const item = existing || {
    name: "",
    timezone: zoneChoices(zone).includes(zone) ? zone : "Europe/Berlin",
    start_date: local.date,
    start_time: "08:00",
    frequency: "daily",
    weekdays: [local.weekday],
    end_kind: "never",
    end_count: 5,
    end_date: local.date,
  };
  const zones = zoneChoices(item.timezone).map((name) => `<option value="${esc(name)}"${name === item.timezone ? " selected" : ""}>${esc(name)}</option>`).join("");
  const days = SCHEDULE_DAYS.map((day) => `
    <label><input type="checkbox" name="weekday" value="${day}"${(item.weekdays || []).includes(day) ? " checked" : ""}> ${h(`schedule.day.${day.toLowerCase()}`)}</label>`).join("");
  const host = document.createElement("div");
  host.innerHTML = `
    <div class="overlay" id="schedule-form">
      <div class="overlay-card schedule-form" role="dialog" aria-modal="true" aria-labelledby="schedule-title">
        <div class="row">
          <h2 id="schedule-title">${existing ? h("edit_schedule") : h("new_schedule")}</h2>
          <button class="btn" type="button" id="schedule-close">${h("close")}</button>
        </div>
        <form id="schedule-fields" class="form">
          <label class="field"><span>${h("name")}</span><input name="name" required value="${esc(item.name)}"></label>
          <div class="field-row">
            <label class="field"><span>${h("start_date")}</span><input name="start_date" type="date" required value="${esc(item.start_date)}"></label>
            <label class="field"><span>${h("start_time")}</span><input name="start_time" type="time" required value="${esc(item.start_time)}"></label>
            <label class="field"><span>${h("time_zone")}</span><select class="fit" name="timezone">${zones}</select></label>
          </div>
          <label class="field"><span>${h("repeat")}</span>
            <select class="fit" name="frequency">
              ${["once", "hourly", "daily", "weekly", "monthly"].map((value) => `<option value="${value}"${item.frequency === value ? " selected" : ""}>${h(value)}</option>`).join("")}
            </select>
          </label>
          <div class="field" id="schedule-weekdays">
            <span>${h("weekdays")}</span>
            <div class="weekday-row">${days}</div>
          </div>
          <div class="field-row" id="schedule-end">
            <label class="field"><span>${h("ends")}</span>
              <select class="fit" name="end_kind">
                <option value="never"${item.end_kind === "never" ? " selected" : ""}>${h("never")}</option>
                <option value="after"${item.end_kind === "after" ? " selected" : ""}>${h("after_n_occurrences")}</option>
                <option value="on_date"${item.end_kind === "on_date" ? " selected" : ""}>${h("on_a_date")}</option>
              </select>
            </label>
            <label class="field" id="schedule-count"><span>${h("times")}</span><input class="tiny" name="end_count" type="number" min="1" max="999" value="${esc(item.end_count || 5)}"></label>
            <label class="field" id="schedule-until"><span>${h("end_date")}</span><input name="end_date" type="date" value="${esc(item.end_date || local.date)}"></label>
          </div>
          <p id="schedule-sentence"></p>
          <div>
            <p class="small muted">${h("schedule_preview")}</p>
            <ul id="schedule-next"></ul>
          </div>
          <div class="row">
            <button class="btn pill primary" type="submit">${h("save")}</button>
          </div>
        </form>
      </div>
    </div>`;
  document.body.appendChild(host.firstElementChild);
  const overlay = document.querySelector("#schedule-form");
  const form = document.querySelector("#schedule-fields");
  const close = () => overlay.remove();
  overlay.querySelector("#schedule-close").addEventListener("click", close);
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) close();
  });
  overlay.addEventListener("keydown", (event) => {
    if (event.key === "Escape") close();
  });
  const showParts = () => {
    const weekly = form.frequency.value === "weekly";
    const once = form.frequency.value === "once";
    document.querySelector("#schedule-weekdays").hidden = !weekly;
    document.querySelector("#schedule-end").hidden = once;
    document.querySelector("#schedule-count").hidden = form.end_kind.value !== "after";
    document.querySelector("#schedule-until").hidden = form.end_kind.value !== "on_date";
  };
  const payload = () => ({
    timezone: form.timezone.value,
    start_date: form.start_date.value,
    start_time: form.start_time.value,
    frequency: form.frequency.value,
    weekdays: [...form.querySelectorAll("input[name=weekday]:checked")].map((box) => box.value),
    end_kind: form.frequency.value === "once" ? "never" : form.end_kind.value,
    end_count: form.end_kind.value === "after" ? Number(form.end_count.value) : null,
    end_date: form.end_kind.value === "on_date" ? form.end_date.value : null,
  });
  let previewTimer = 0;
  const preview = () => {
    window.clearTimeout(previewTimer);
    previewTimer = window.setTimeout(async () => {
      const body = payload();
      document.querySelector("#schedule-sentence").textContent = scheduleSentence({ ...body, name: form.name.value });
      try {
        const result = await api("POST", "/api/schedules/preview", body);
        const lines = (result.occurrences || []).map((at) => `<li>${esc(formatWhen(at, body.timezone))}</li>`).join("");
        document.querySelector("#schedule-next").innerHTML = lines || `<li>${h("schedule_no_upcoming")}</li>`;
      } catch (error) {
        document.querySelector("#schedule-next").innerHTML = `<li>${esc(t(error.message || "error.internal", error.vars))}</li>`;
      }
    }, 200);
  };
  form.addEventListener("input", () => {
    showParts();
    preview();
  });
  form.addEventListener("change", () => {
    showParts();
    preview();
  });
  showParts();
  preview();
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const body = { ...payload(), name: form.name.value };
    try {
      if (existing) await api("PUT", `/api/schedules/${encodeURIComponent(existing.id)}`, body);
      else await api("POST", `/api/${kind}s/${encodeURIComponent(target)}/schedules`, body);
      close();
      await redraw();
    } catch (error) {
      handle(error);
    }
  });
  form.name.focus();
}

async function pageSchedules() {
  if (!needTeam()) return;
  shell(t("schedules"), `<section class="card" id="schedule-card" data-kind="all"></section>`);
  mountSchedules(document.querySelector("#schedule-card"));
}

async function pageJobs() {
  if (!needTeam()) return;
  const jobs = await api("GET", "/api/jobs");
  shell(t("jobs"), `
    <section class="card">
      <input class="search" id="job-search" placeholder="${h("search_jobs")}" autocomplete="off">
      <div id="job-table"></div>
    </section>`);
  const drawJobs = () => {
    const needle = document.querySelector("#job-search").value.trim().toLowerCase();
    const shown = jobs.filter((job) => !needle || [job.name, job.tool, job.status, job.trigger, statusLabel(job.status)].join(" ").toLowerCase().includes(needle));
    const filtered = shown.map((job) =>
      `<tr><td><a href="#/jobs/${esc(job.id)}">${esc(job.name)}</a></td><td class="tool">${esc(job.tool)}</td><td>${pill(job.status)}</td><td class="small">${esc(job.trigger)}</td><td class="small muted">${esc(job.created_at)}</td><td>${jobListActions(job)}</td></tr>`
    );
    document.querySelector("#job-table").innerHTML = table([t("name"), t("tool"), t("status"), t("trigger"), t("created"), ""], filtered);
    document.querySelectorAll("[data-relaunch]").forEach((button) => {
      button.addEventListener("click", async () => {
        button.disabled = true;
        try {
          const copy = await api("POST", `/api/jobs/${encodeURIComponent(button.dataset.relaunch)}/relaunch`, {
            only_failed_hosts: button.dataset.failed === "1",
          });
          go(`#/jobs/${copy.id}`);
        } catch (error) {
          button.disabled = false;
          handle(error);
        }
      });
    });
  };
  document.querySelector("#job-search").addEventListener("input", drawJobs);
  drawJobs();
}

function appendLog(row) {
  const log = document.querySelector("#log");
  if (!log) return;
  const line = document.createElement("div");
  if (row.stream === "system" || row.stream === "stderr") line.className = row.stream;
  line.textContent = row.line;
  log.appendChild(line);
  log.scrollTop = log.scrollHeight;
}

function paramRow(label, value) {
  if (value === undefined || value === null || value === "") return "";
  return `<dt>${h(label)}</dt><dd>${esc(value)}</dd>`;
}

function paramRowHtml(label, html) {
  if (!html) return "";
  return `<dt>${h(label)}</dt><dd>${html}</dd>`;
}

function credentialSummary(params) {
  if (Array.isArray(params.credentials)) {
    return params.credentials.map((item) => {
      const name = item && item.name;
      if (!name) return "";
      const kind = item.kind ? credentialKindLabel(item.kind) : "";
      return [name, kind].filter(Boolean).join(" · ");
    }).filter(Boolean).join(", ");
  }
  const kind = params.credential_kind ? credentialKindLabel(params.credential_kind) : "";
  return [params.credential_name, kind].filter(Boolean).join(" · ");
}

function jobParameters(job) {
  const params = job.parameters && job.parameters.tool ? job.parameters : null;
  const hosts = Array.isArray(job.failed_hosts) ? job.failed_hosts : [];
  if (!params) {
    const extra = job.extra_vars && Object.keys(job.extra_vars).length
      ? `<pre class="vars">${esc(JSON.stringify(job.extra_vars, null, 2))}</pre>`
      : "";
    return `<p class="muted">${h("no_parameters_were_stored_for_this_job")}</p>${extra}`;
  }
  const spec = params.spec || {};
  const templateId = params.template_id || job.template_id;
  const templateName = params.template_name || job.name;
  const template = templateId
    ? `<a href="#/templates/${esc(templateId)}">${esc(templateName)}</a>`
    : esc(templateName);
  const credential = credentialSummary(params);
  const environment = [params.environment_name, params.environment_image].filter(Boolean).join(" · ");
  const kind = spec.job_type === "check" ? t("check") : spec.job_type === "run" ? t("run") : "";
  const diff = spec.diff === true ? t("yes") : spec.diff === false ? t("no") : "";
  const hostLine = hosts.length
    ? `<dt>${h("failed_hosts")}</dt><dd>${hosts.map((host) => esc(host)).join(", ")}</dd>`
    : "";
  return `<dl class="kv">
    <dt>${h("template")}</dt><dd>${template}</dd>
    ${paramRow("tool", params.tool)}
    ${paramRow("entry", params.entrypoint)}
    ${paramRow("project", params.project_name)}
    ${typeof params.update_on_launch === "boolean" && params.project_name ? paramRow("project_update", t(params.update_on_launch ? "on_launch" : "use_the_last_sync")) : ""}
    ${paramRow("inventory", params.inventory_name)}
    ${paramRow("nav.access", credential)}
    ${paramRow("execution_environment", environment)}
    ${paramRow("type", kind)}
    ${paramRowHtml("tags", chipReadout(spec.tags))}
    ${paramRowHtml("skip_tags", chipReadout(spec.skip_tags))}
    ${paramRow("limit", spec.limit)}
    ${paramRow("diff", diff)}
    ${paramRow("verbosity", spec.verbosity === undefined || spec.verbosity === null || spec.verbosity === "" ? "" : t(verbosityLabel(spec.verbosity)))}
    ${paramRow("forks", spec.forks)}
    ${paramRow("action", spec.action)}
    ${hostLine}
  </dl>
  ${spec.script ? `<h3>${h("script")}</h3><pre class="vars">${esc(spec.script)}</pre>` : ""}
  <h3>${h("extra_variables")}</h3>
  <pre class="vars">${esc(JSON.stringify(params.extra_vars || {}, null, 2))}</pre>`;
}

function jobListActions(job) {
  if (!can("operator") || job.status !== "failed") return "";
  const hosts = Array.isArray(job.failed_hosts) ? job.failed_hosts : [];
  const failedHosts = job.tool === "ansible" && hosts.length
    ? `<button class="btn" type="button" data-relaunch="${esc(job.id)}" data-failed="1">${h("failed_hosts_only")}</button>`
    : "";
  return `<div class="row"><button class="btn" type="button" data-relaunch="${esc(job.id)}" data-failed="0">${h("relaunch")}</button>${failedHosts}</div>`;
}

function jobActions(job) {
  const running = job.status === "pending" || job.status === "running";
  const hosts = Array.isArray(job.failed_hosts) ? job.failed_hosts : [];
  if (!can("operator")) return "";
  if (running) return `<button class="btn danger" type="button" data-action="cancel">${h("cancel")}</button>`;
  if (job.status !== "failed") return "";
  const failedHosts = job.tool === "ansible" && hosts.length
    ? `<button class="btn" type="button" data-action="relaunch-failed">${h("failed_hosts_only")}</button>`
    : "";
  return `<button class="btn primary" type="button" data-action="relaunch">${h("relaunch")}</button>${failedHosts}`;
}

async function pageJob(id) {
  const live = generation;
  const job = await api("GET", `/api/jobs/${encodeURIComponent(id)}`);
  const running = job.status === "pending" || job.status === "running";
  const actions = jobActions(job);
  shell(job.name, `
    <div class="spread" style="margin-bottom:12px">
      <div class="row">${pill(job.status)} <span class="tool">${esc(job.tool)}</span> <span class="small muted">${esc(job.trigger)} · ${esc(job.launched_by)}</span></div>
    </div>
    <div class="spread tab-bar">
      <div class="tabs">
        <button class="btn pill primary" type="button" data-job-tab="output">${h("output")}</button>
        <button class="btn pill" type="button" data-job-tab="parameters">${h("parameters")}</button>
      </div>
      <div class="row">
        ${actions}
        <button class="btn" type="button" id="download-output">${h("download_output")}</button>
      </div>
    </div>
    <div id="panel-output"><div class="log" id="log"></div></div>
    <div id="panel-parameters" hidden>${jobParameters(job)}</div>`);
  const badge = document.querySelector(".pill");
  (job.events || []).forEach(appendLog);
  const relaunchButtons = () => document.querySelectorAll("[data-action=relaunch], [data-action=relaunch-failed]");
  const startAgain = async (onlyFailed) => {
    relaunchButtons().forEach((button) => {
      button.disabled = true;
    });
    try {
      const copy = await api("POST", `/api/jobs/${encodeURIComponent(id)}/relaunch`, { only_failed_hosts: onlyFailed });
      go(`#/jobs/${copy.id}`);
    } catch (error) {
      relaunchButtons().forEach((button) => {
        button.disabled = false;
      });
      handle(error);
    }
  };
  document.querySelectorAll("[data-action=cancel]").forEach((cancel) => {
    cancel.addEventListener("click", async () => {
      try {
        await api("POST", `/api/jobs/${encodeURIComponent(id)}/cancel`, {});
        if (badge) {
          badge.className = "pill canceled";
          badge.textContent = statusLabel("canceled");
        }
        document.querySelectorAll("[data-action=cancel]").forEach((button) => button.remove());
      } catch (error) {
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-action=relaunch]").forEach((button) => {
    button.addEventListener("click", () => startAgain(false));
  });
  document.querySelectorAll("[data-action=relaunch-failed]").forEach((button) => {
    button.addEventListener("click", () => startAgain(true));
  });
  document.querySelectorAll("[data-job-tab]").forEach((button) => {
    button.addEventListener("click", () => {
      const tab = button.dataset.jobTab;
      document.querySelectorAll("[data-job-tab]").forEach((item) => {
        item.classList.toggle("primary", item === button);
      });
      document.querySelector("#panel-output").hidden = tab !== "output";
      document.querySelector("#panel-parameters").hidden = tab !== "parameters";
    });
  });
  document.querySelector("#download-output").addEventListener("click", async () => {
    const button = document.querySelector("#download-output");
    button.disabled = true;
    try {
      const headers = {};
      if (teamId) headers["X-Team-Id"] = teamId;
      const response = await fetch(`/api/jobs/${encodeURIComponent(id)}/output`, { headers, credentials: "same-origin" });
      const blob = await response.blob();
      if (!response.ok) {
        const text = await blob.text();
        let message = "request.failed";
        let vars = null;
        try {
          const data = JSON.parse(text);
          message = data.error || message;
          vars = data.vars || null;
        } catch { /* keep */ }
        const error = new Error(message);
        error.status = response.status;
        error.vars = vars;
        throw error;
      }
      const match = /filename="([^"]+)"/.exec(response.headers.get("content-disposition") || "");
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = match ? match[1] : "job.txt";
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
    } catch (error) {
      handle(error);
    } finally {
      button.disabled = false;
    }
  });
  if (!running || live !== generation) return;
  const last = (job.events || []).reduce((max, event) => Math.max(max, event.id), 0);
  source = new EventSource(`/api/jobs/${encodeURIComponent(id)}/events?after=${last}`);
  source.addEventListener("log", (event) => {
    if (live !== generation) return;
    appendLog(JSON.parse(event.data));
  });
  source.addEventListener("end", (event) => {
    if (live !== generation) return;
    const payload = JSON.parse(event.data);
    if (badge) {
      badge.className = `pill ${payload.status}`;
      badge.textContent = statusLabel(payload.status);
    }
    document.querySelectorAll("[data-action=cancel]").forEach((button) => button.remove());
    source.close();
    source = null;
    if (payload.status === "failed") pageJob(id);
  });
}

async function pageWorkflows() {
  if (!needTeam()) return;
  const items = await api("GET", "/api/workflows");
  shell(t("workflows"), `
    <section class="card">
      <input class="search" id="workflow-search" placeholder="${h("search_workflows")}" autocomplete="off">
      <div id="workflow-table"></div>
    </section>`,
    can("operator") ? addLink("#/workflows/new", "workflow_2") : "");
  const drawWorkflows = () => {
    const needle = document.querySelector("#workflow-search").value.trim().toLowerCase();
    const shown = items.filter((item) => !needle || `${item.name} ${item.description}`.toLowerCase().includes(needle));
    const filtered = shown.map((item) => `
      <tr>
        <td><a href="#/workflows/${esc(item.id)}">${esc(item.name)}</a><div class="small muted">${esc(item.description)}</div></td>
        <td>${h("count_nodes", { count: (item.graph.nodes || []).length })}</td>
        <td>${can("operator") ? `<button class="btn primary" type="button" data-launch="${esc(item.id)}">${h("launch")}</button>` : ""}</td>
      </tr>`);
    document.querySelector("#workflow-table").innerHTML = table([t("name"), t("nodes"), ""], filtered);
    document.querySelectorAll("[data-launch]").forEach((button) => {
      button.addEventListener("click", async () => {
        button.disabled = true;
        try {
          const run = await api("POST", `/api/workflows/${encodeURIComponent(button.dataset.launch)}/launch`, { extra_vars: {} });
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
}

async function pageWorkflowNew() {
  if (!needTeam()) return;
  if (!can("operator")) {
    deny("workflow", "operator");
    return;
  }
  shell(t("new_workflow"), `
    <form class="card form" id="workflow-new">
      <label class="field"><span>${h("name")}</span><input name="name" required></label>
      <label class="field"><span>${h("description")}</span><input name="description"></label>
      <button class="btn pill primary" type="submit">${h("create_and_open")}</button>
    </form>`);
  document.querySelector("#workflow-new").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const created = await api("POST", "/api/workflows", {
        name: form.name.value,
        description: form.description.value,
        graph: { nodes: [], edges: [] },
      });
      go(`#/workflows/${created.id}`);
    } catch (error) {
      handle(error);
    }
  });
}

function edgePoint(node, when) {
  const y = when === "failure" ? 54 : when === "always" ? 39 : 24;
  return { x: node.x + 200, y: node.y + y };
}

async function pageWorkflow(id) {
  if (!needTeam()) return;
  const [workflow, templates, inventories, credentials] = await Promise.all([
    api("GET", `/api/workflows/${encodeURIComponent(id)}`),
    api("GET", "/api/templates"),
    api("GET", "/api/inventories"),
    api("GET", "/api/credentials"),
  ]);
  mountEditor(workflow, templates, false, {}, { inventories, credentials });
}

function mountEditor(workflow, templates, readonly, phases, catalogs, hooks) {
  const onNode = hooks && hooks.onNode;
  const lists = catalogs || { inventories: [], credentials: [] };
  const editable = !readonly && can("operator");
  const graph = {
    nodes: (workflow.graph.nodes || []).map((node) => ({ ...node })),
    edges: (workflow.graph.edges || []).map((edge) => ({ ...edge })),
  };
  let selected = null;
  let selectedEdge = null;
  let link = null;
  let drag = null;
  let flushNode = () => true;
  const back = readonly && workflow.workflow_id
    ? `<a class="btn" href="#/workflows/${esc(workflow.workflow_id)}">${h("back_to_the_workflow")}</a>`
    : "";
  shell(readonly ? workflow.name : t("workflow"), `
    <div class="editor-stack">
    <div class="editor-body">
      <div>
        ${readonly ? "" : `
          <div class="row" style="margin-bottom:10px">
            <input id="wf-name" value="${esc(workflow.name)}" ${can("operator") ? "" : "disabled"}>
            <input id="wf-description" value="${esc(workflow.description)}" placeholder="${h("description")}" ${can("operator") ? "" : "disabled"}>
          </div>`}
        <p class="small muted" id="wf-hint">${readonly ? h("colors_show_the_run_a_click_shows_the") : h("drag_nodes_add_more_nodes_on_the_right")}</p>
        <div class="canvas" id="canvas"><div id="board" style="position:relative;min-width:100%;min-height:560px"><svg class="edges" id="edges"></svg><div id="nodes"></div></div></div>
      </div>
      <aside class="card inspector" id="inspector"></aside>
    </div>
    ${readonly ? `<section class="card run-log" id="run-log" aria-live="polite"></section>` : `<section class="card" id="schedule-card" data-kind="workflow" data-target="${esc(workflow.id)}"></section>`}
    </div>`, readonly ? back : `
      ${can("operator") ? `<button class="btn" id="wf-save" type="button">${h("save")}</button><button class="btn primary" id="wf-launch" type="button">${h("launch")}</button>` : ""}
      ${can("operator") ? `<button class="btn danger" id="wf-delete" type="button">${h("delete")}</button>` : ""}`);

  const board = document.querySelector("#board");
  const svg = document.querySelector("#edges");
  const nodes = document.querySelector("#nodes");

  function localPoint(event) {
    const rect = board.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
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
    const join = node.convergence === "all" ? t("all_predecessors_2") : t("one_predecessor");
    const inventoryId = node.overrides && Object.prototype.hasOwnProperty.call(node.overrides, "inventory_id")
      ? node.overrides.inventory_id
      : undefined;
    if (inventoryId === undefined) return join;
    if (!inventoryId) return `${join} · ${t("no_inventory")}`;
    const inventory = lists.inventories.find((item) => item.id === inventoryId);
    return inventory ? `${join} · ${inventory.name}` : join;
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
        </label>`;
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
    const width = Math.max(board.parentElement.clientWidth, ...graph.nodes.map((node) => node.x + 260), 640);
    const height = Math.max(560, ...graph.nodes.map((node) => node.y + 140));
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
      return `<path class="${klass}${selectedEdge === index ? " selected" : ""}" data-edge="${index}" d="M ${start.x} ${start.y} C ${start.x + dx} ${start.y}, ${end.x - dx} ${end.y}, ${end.x} ${end.y}"></path>`;
    }).join("");
  }

  function renderNodes() {
    nodes.innerHTML = graph.nodes.map((node) => {
      const phase = phases[node.id];
      const phaseClass = phase ? ` phase-${esc(phase.phase)}` : "";
      const selectedClass = selected === node.id ? " selected" : "";
      const runClass = readonly ? " run-node" : "";
      const armed = link && link.from === node.id ? link.when : "";
      const ports = editable ? `<div class="ports"><button type="button" class="port success${armed === "success" ? " armed" : ""}" data-port="success">${h("success")}</button><button type="button" class="port failure${armed === "failure" ? " armed" : ""}" data-port="failure">${h("error.generic")}</button></div>` : "";
      return `<div class="wf-node${selectedClass}${phaseClass}${runClass}" data-id="${esc(node.id)}" style="left:${Number(node.x) || 0}px;top:${Number(node.y) || 0}px"><strong>${esc(node.name)}</strong><span class="small muted">${esc(nodeCaption(node))}</span>${ports}</div>`;
    }).join("");
    nodes.querySelectorAll(".wf-node").forEach((element) => {
      const node = graph.nodes.find((item) => item.id === element.dataset.id);
      element.addEventListener("pointerdown", (event) => {
        if (event.target.closest(".port")) return;
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
        if (link && link.from !== node.id) {
          if (!flushNode()) return;
          const fromId = link.from;
          const exists = graph.edges.some((edge) => edge.from === fromId && edge.to === node.id && edge.when === link.when);
          if (!exists) graph.edges.push({ from: fromId, to: node.id, when: link.when });
          link = null;
          selected = fromId;
          selectedEdge = null;
          document.querySelector("#wf-hint").textContent = t("edge_added");
          renderNodes();
          renderInspector();
          return;
        }
        if (!flushNode()) return;
        selected = node.id;
        selectedEdge = null;
        const point = localPoint(event);
        drag = { id: node.id, ox: point.x - node.x, oy: point.y - node.y };
        element.setPointerCapture(event.pointerId);
        renderInspector();
        nodes.querySelectorAll(".wf-node").forEach((item) => item.classList.toggle("selected", item.dataset.id === selected));
      });
      element.addEventListener("pointermove", (event) => {
        if (!drag || drag.id !== node.id) return;
        const point = localPoint(event);
        node.x = Math.max(0, Math.round(point.x - drag.ox));
        node.y = Math.max(0, Math.round(point.y - drag.oy));
        element.style.left = `${node.x}px`;
        element.style.top = `${node.y}px`;
        drawEdges();
      });
      element.addEventListener("pointerup", () => { drag = null; });
      element.querySelectorAll(".port").forEach((port) => {
        port.addEventListener("pointerdown", (event) => event.stopPropagation());
        port.addEventListener("click", (event) => {
          event.stopPropagation();
          link = { from: node.id, when: port.dataset.port };
          nodes.querySelectorAll(".port").forEach((item) => item.classList.toggle("armed", item === port));
          document.querySelector("#wf-hint").textContent = port.dataset.port === "failure"
            ? t("failure_edge_click_the_target_node")
            : t("success_edge_click_the_target_node");
        });
      });
    });
    drawEdges();
  }

  function outgoingTarget(fromId, when) {
    const edge = graph.edges.find((item) => item.from === fromId && item.when === when);
    return edge ? edge.to : "";
  }

  function setOutgoing(fromId, when, toId) {
    graph.edges = graph.edges.filter((edge) => !(edge.from === fromId && edge.when === when));
    if (toId && toId !== fromId && graph.nodes.some((item) => item.id === toId)) {
      graph.edges.push({ from: fromId, to: toId, when });
    }
    link = null;
    drawEdges();
  }

  function targetOptions(fromId, when) {
    const current = outgoingTarget(fromId, when);
    const options = [`<option value="">${h("no_target")}</option>`];
    graph.nodes.forEach((item) => {
      if (item.id === fromId) return;
      options.push(`<option value="${esc(item.id)}"${item.id === current ? " selected" : ""}>${esc(item.name)}</option>`);
    });
    return options.join("");
  }

  function addNodeControls() {
    return `<div class="inspector-block">
      <h2>${h("nodes")}</h2>
      <label class="field"><span>${h("template")}</span><select class="fit" id="add-template">${selectOptions(templates, "", false, (row) => `${row.name} · ${row.tool}`)}</select></label>
      <button class="btn pill" id="add-node" type="button">${h("add_node")}</button>
    </div>`;
  }

  function bindAddNode(host) {
    host.querySelector("#add-node").addEventListener("click", () => {
      if (!flushNode()) return;
      const templateId = host.querySelector("#add-template").value;
      const template = templates.find((item) => item.id === templateId);
      if (!template) {
        const hint = document.querySelector("#wf-hint");
        if (hint) hint.textContent = t("create_a_template_or_a_workflow_first");
        return;
      }
      const index = graph.nodes.length;
      const id = `n${Math.random().toString(16).slice(2, 10)}`;
      graph.nodes.push({
        id,
        template_id: template.id,
        name: template.name,
        x: 40 + (index % 3) * 240,
        y: 40 + Math.floor(index / 3) * 140,
        convergence: "any",
      });
      selected = id;
      selectedEdge = null;
      link = null;
      renderNodes();
      renderInspector();
    });
  }

  function fieldValue(id) {
    const element = document.querySelector(id);
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

  function readOverrides(node) {
    const template = templates.find((item) => item.id === node.template_id);
    const extraField = document.querySelector("#node-extra");
    if (!template || !extraField) return true;
    const over = {};
    const templateSpec = template.spec || {};
    if (template.tool === "ansible") {
      const inventoryId = fieldValue("#node-inventory");
      if (inventoryId !== (template.inventory_id || "")) over.inventory_id = inventoryId;
      const spec = {};
      const jobType = fieldValue("#node-job-type") === "check" ? "check" : "run";
      const templateJob = templateSpec.job_type === "check" ? "check" : "run";
      if (jobType !== templateJob) spec.job_type = jobType;
      const verbosity = Number(fieldValue("#node-verbosity")) || 0;
      const templateVerbosity = Number(templateSpec.verbosity) || 0;
      if (verbosity !== templateVerbosity) spec.verbosity = verbosity;
      const forksRaw = fieldValue("#node-forks").trim();
      const templateForks = templateSpec.forks;
      const templateHasForks = templateForks !== undefined && templateForks !== null && templateForks !== "";
      if (forksRaw === "") {
        if (templateHasForks) spec.forks = null;
      } else {
        const forks = Number(forksRaw);
        if (!Number.isInteger(forks)) {
          note(t("forks_is_a_whole_number"));
          return false;
        }
        if (forks < 1 || forks > 50) {
          note(t("forks_is_a_number_from_1_to_50"));
          return false;
        }
        if (forks !== templateForks) spec.forks = forks;
      }
      const diff = Boolean(document.querySelector("#node-diff") && document.querySelector("#node-diff").checked);
      if (diff !== Boolean(templateSpec.diff)) spec.diff = diff;
      const tags = fieldValue("#node-tags");
      if (tags !== (templateSpec.tags || "")) spec.tags = tags;
      const skip = fieldValue("#node-skip-tags");
      if (skip !== (templateSpec.skip_tags || "")) spec.skip_tags = skip;
      const limit = fieldValue("#node-limit");
      if (limit !== (templateSpec.limit || "")) spec.limit = limit;
      if (Object.keys(spec).length) over.spec = spec;
    } else if (template.tool === "opentofu") {
      const action = fieldValue("#node-action") === "apply" ? "apply" : "plan";
      const templateAction = templateSpec.action === "apply" ? "apply" : "plan";
      if (action !== templateAction) over.spec = { action };
    }
    const credentialId = fieldValue("#node-credential");
    if (credentialId !== (template.credential_id || "")) over.credential_id = credentialId;
    let extra;
    try {
      extra = jsonObject(extraField.value, "extra_variables");
    } catch (error) {
      note(t(error.message, error.vars));
      return false;
    }
    if (!sameJson(extra, template.extra_vars || {})) over.extra_vars = extra;
    if (Object.keys(over).length) node.overrides = over;
    else delete node.overrides;
    paintCaption(node);
    const hint = document.querySelector("#wf-hint");
    if (hint && (hint.textContent === t("forks_is_a_whole_number") || hint.textContent === t("forks_is_a_number_from_1_to_50") || hint.textContent === t("extra_variables_are_not_valid_json") || hint.textContent === t("extra_variables_must_be_a_json_object"))) {
      hint.textContent = "";
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
      host.innerHTML = `${addNodeControls()}
        <div class="inspector-block">
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
      bindAddNode(host);
      host.querySelector("#edge-when").addEventListener("change", (event) => {
        edge.when = event.target.value;
        drawEdges();
      });
      host.querySelector("#edge-delete").addEventListener("click", () => {
        graph.edges.splice(selectedEdge, 1);
        selectedEdge = null;
        renderInspector();
        drawEdges();
      });
      return;
    }
    if (!node) {
      host.innerHTML = `${addNodeControls()}
        <p class="small muted">${h("an_empty_graph_can_be_saved_but_not")}</p>
        <p class="small muted">${h("select_a_node_to_connect_it")}</p>`;
      bindAddNode(host);
      return;
    }
    host.innerHTML = `${addNodeControls()}
      <div class="inspector-block">
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
        <label class="field"><span>${h("on_success_go_to")}</span><select class="fit" id="link-success">${targetOptions(node.id, "success")}</select></label>
        <label class="field"><span>${h("on_failure_go_to")}</span><select class="fit" id="link-failure">${targetOptions(node.id, "failure")}</select></label>
        <label class="field"><span>${h("always_go_to")}</span><select class="fit" id="link-always">${targetOptions(node.id, "always")}</select></label>
        <p class="small muted">${h("the_edge_starts_at_this_node")}</p>
        <button class="btn danger" id="node-delete" type="button">${h("delete_node")}</button>
      </div>`;
    bindAddNode(host);
    host.querySelector("#node-name").addEventListener("input", (event) => {
      node.name = event.target.value;
      const label = nodes.querySelector(`[data-id="${CSS.escape(node.id)}"] strong`);
      if (label) label.textContent = node.name;
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
      renderInspector();
    });
    host.querySelector("#node-join").addEventListener("change", (event) => {
      node.convergence = event.target.value;
      paintCaption(node);
    });
    flushNode = () => readOverrides(node);
    ["#node-inventory", "#node-credential", "#node-job-type", "#node-verbosity", "#node-diff", "#node-action", "#node-forks", "#node-tags", "#node-skip-tags", "#node-limit", "#node-extra"].forEach((id) => {
      const element = host.querySelector(id);
      if (!element) return;
      element.addEventListener("change", () => { flushNode(); });
    });
    ["success", "failure", "always"].forEach((when) => {
      host.querySelector(`#link-${when}`).addEventListener("change", (event) => {
        setOutgoing(node.id, when, event.target.value);
        renderNodes();
      });
    });
    host.querySelector("#node-delete").addEventListener("click", () => {
      flushNode = () => true;
      graph.nodes = graph.nodes.filter((item) => item.id !== node.id);
      graph.edges = graph.edges.filter((edge) => edge.from !== node.id && edge.to !== node.id);
      selected = null;
      link = null;
      renderNodes();
      renderInspector();
    });
  }

  svg.addEventListener("click", (event) => {
    const path = event.target.closest("path");
    if (!path || readonly) return;
    if (!flushNode()) return;
    selectedEdge = Number(path.dataset.edge);
    selected = null;
    link = null;
    drawEdges();
    renderInspector();
  });
  board.addEventListener("pointerdown", (event) => {
    if (readonly || !editable) return;
    if (event.target.closest(".wf-node, path")) return;
    if (!flushNode()) return;
    selected = null;
    selectedEdge = null;
    link = null;
    renderNodes();
    renderInspector();
  });

  renderNodes();
  renderInspector();
  mountSchedules(document.querySelector("#schedule-card"));

  const save = document.querySelector("#wf-save");
  if (save) {
    save.addEventListener("click", async () => {
      if (!flushNode()) return;
      try {
        await api("PUT", `/api/workflows/${encodeURIComponent(workflow.id)}`, {
          name: document.querySelector("#wf-name").value,
          description: document.querySelector("#wf-description").value,
          graph,
        });
        document.querySelector("#wf-hint").textContent = t("saved");
      } catch (error) {
        handle(error);
      }
    });
  }
  const launch = document.querySelector("#wf-launch");
  if (launch) {
    launch.addEventListener("click", async () => {
      if (!flushNode()) return;
      try {
        await api("PUT", `/api/workflows/${encodeURIComponent(workflow.id)}`, {
          name: document.querySelector("#wf-name").value,
          description: document.querySelector("#wf-description").value,
          graph,
        });
        const run = await api("POST", `/api/workflows/${encodeURIComponent(workflow.id)}/launch`, { extra_vars: {} });
        go(`#/runs/${run.id}`);
      } catch (error) {
        handle(error);
      }
    });
  }
  const remove = document.querySelector("#wf-delete");
  if (remove) {
    remove.addEventListener("click", async () => {
      if (!confirm(t("delete_this_workflow"))) return;
      try {
        await api("DELETE", `/api/workflows/${encodeURIComponent(workflow.id)}`);
        go("#/workflows");
      } catch (error) {
        handle(error);
      }
    });
  }

  function applyPhases() {
    nodes.querySelectorAll(".wf-node").forEach((element) => {
      const phase = phases[element.dataset.id];
      const phaseClass = phase ? ` phase-${phase.phase}` : "";
      element.className = `wf-node run-node${phaseClass}`;
    });
  }

  return { applyPhases };
}

async function pageRun(id) {
  const live = generation;
  const phases = {};
  let editor = null;
  let openNodeId = null;
  let shownJob = "";
  let outputToken = 0;

  function adopt(run) {
    Object.keys(phases).forEach((key) => delete phases[key]);
    (run.nodes || []).forEach((node) => { phases[node.node_id] = node; });
    return {
      id: run.id,
      workflow_id: run.workflow_id || "",
      name: run.name,
      description: "",
      status: run.status,
      graph: run.graph || { nodes: [], edges: [] },
    };
  }

  function nodeStart(phase) {
    if (!phase) return "";
    return phase.started_at || (phase.phase === "running" ? phase.job_created_at : "") || "";
  }

  function renderLog(run) {
    const host = document.querySelector("#run-log");
    if (!host) return;
    const graphNodes = (run.graph && run.graph.nodes) || [];
    const ranked = graphNodes.map((node, index) => ({ node, index, phase: phases[node.id] }));
    ranked.sort((left, right) => {
      const a = nodeStart(left.phase);
      const b = nodeStart(right.phase);
      if (a && b && a !== b) return a < b ? -1 : 1;
      if (a && !b) return -1;
      if (!a && b) return 1;
      return left.index - right.index;
    });
    const rows = ranked.map(({ node, phase }) => {
      const status = phase ? phase.phase : "pending";
      const start = nodeStart(phase);
      const end = phase && phase.finished_at ? phase.finished_at : "";
      return `<button class="run-line ${esc(status)}" type="button" data-run-node="${esc(node.id)}">
        <span class="mono">${esc(clock(start))}</span>
        <span>${esc(node.name)}</span>
        <span>${esc(statusLabel(status))}</span>
        <span class="mono">${esc(clock(end))}</span>
        <span class="mono">${esc(spanText(start, end, status))}</span>
      </button>`;
    }).join("");
    host.innerHTML = `
      <h2>${h("status")}</h2>
      <div class="run-lines">
        <div class="run-line head">
          <span>${h("start")}</span>
          <span>${h("name")}</span>
          <span>${h("status")}</span>
          <span>${h("end")}</span>
          <span>${h("duration")}</span>
        </div>
        ${rows}
      </div>`;
    host.querySelectorAll("[data-run-node]").forEach((row) => {
      row.addEventListener("click", () => {
        const node = graphNodes.find((item) => item.id === row.dataset.runNode);
        if (node) openOutput(node, phases[node.id] || null);
      });
    });
  }

  function overlayMeta(phase) {
    const status = phase ? phase.phase : "pending";
    const start = nodeStart(phase);
    const end = phase && phase.finished_at ? phase.finished_at : "";
    const tool = phase && phase.tool ? `<span class="tool">${esc(phase.tool)}</span>` : "";
    const link = phase && phase.job_id
      ? `<a href="#/jobs/${esc(phase.job_id)}">${h("open_the_job")}</a>`
      : "";
    return `${pill(status)} ${tool} <span class="small muted">${esc(clock(start))} – ${esc(clock(end))} · ${esc(spanText(start, end, status))}</span> ${link}`;
  }

  function closeOutput() {
    outputToken += 1;
    openNodeId = null;
    shownJob = "";
    const overlay = document.querySelector("#node-output");
    if (overlay) overlay.remove();
    if (source) {
      source.close();
      source = null;
    }
  }

  async function openOutput(node, phase) {
    if (live !== generation) return;
    const token = ++outputToken;
    openNodeId = node.id;
    const jobId = phase && phase.job_id ? phase.job_id : "";
    shownJob = jobId;
    if (source) {
      source.close();
      source = null;
    }
    let overlay = document.querySelector("#node-output");
    if (!overlay) {
      document.querySelector(".main").insertAdjacentHTML("beforeend", `
        <div class="overlay" id="node-output">
          <div class="overlay-card" role="dialog" aria-modal="true" aria-labelledby="overlay-title">
            <div class="spread">
              <h2 id="overlay-title"></h2>
              <button class="btn" type="button" id="overlay-close">${h("close")}</button>
            </div>
            <p class="small row" id="overlay-meta"></p>
            <div class="log" id="log"></div>
          </div>
        </div>`);
      overlay = document.querySelector("#node-output");
      overlay.addEventListener("click", (event) => {
        if (event.target === overlay) closeOutput();
      });
      document.querySelector("#overlay-close").addEventListener("click", closeOutput);
    }
    document.querySelector("#overlay-title").textContent = node.name;
    document.querySelector("#overlay-meta").innerHTML = overlayMeta(phase);
    const log = document.querySelector("#log");
    log.replaceChildren();
    const note = (text) => {
      const line = document.createElement("div");
      line.className = "system";
      line.textContent = text;
      log.appendChild(line);
    };
    if (!jobId) {
      note(phase && phase.phase === "skipped"
        ? t("this_node_was_skipped")
        : t("this_node_has_no_output_yet"));
      return;
    }
    try {
      const job = await api("GET", `/api/jobs/${encodeURIComponent(jobId)}`);
      if (token !== outputToken || live !== generation) return;
      (job.events || []).forEach(appendLog);
      if (!log.childNodes.length) note(t("this_node_has_no_output_yet"));
      const running = job.status === "pending" || job.status === "running";
      if (!running) return;
      const last = (job.events || []).reduce((max, event) => Math.max(max, event.id), 0);
      source = new EventSource(`/api/jobs/${encodeURIComponent(jobId)}/events?after=${last}`);
      source.addEventListener("log", (event) => {
        if (token !== outputToken || live !== generation) return;
        appendLog(JSON.parse(event.data));
      });
      source.addEventListener("end", (event) => {
        if (token !== outputToken || live !== generation) return;
        const payload = JSON.parse(event.data);
        const badge = document.querySelector("#node-output .pill");
        if (badge) {
          badge.className = `pill ${payload.status}`;
          badge.textContent = statusLabel(payload.status);
        }
        source.close();
        source = null;
      });
    } catch (error) {
      if (token === outputToken && live === generation) handle(error);
    }
  }

  function paint(run) {
    const view = adopt(run);
    if (!editor) {
      editor = mountEditor(view, [], true, phases, null, { onNode: openOutput });
    } else if (editor) {
      editor.applyPhases();
      const badge = document.querySelector("#run-state");
      if (badge) {
        badge.className = `pill ${run.status || "pending"}`;
        badge.textContent = statusLabel(run.status || "pending");
      }
    }
    renderLog(run);
    if (!openNodeId || !document.querySelector("#node-output")) return;
    const node = ((run.graph && run.graph.nodes) || []).find((item) => item.id === openNodeId);
    const phase = phases[openNodeId] || null;
    if (!node) return;
    const jobId = phase && phase.job_id ? phase.job_id : "";
    if (jobId && jobId !== shownJob) openOutput(node, phase);
    else {
      const meta = document.querySelector("#overlay-meta");
      if (meta) meta.innerHTML = overlayMeta(phase);
    }
  }

  runKey = (event) => {
    if (event.key === "Escape") closeOutput();
  };
  document.addEventListener("keydown", runKey);

  const run = await api("GET", `/api/workflow-jobs/${encodeURIComponent(id)}`);
  if (live !== generation) return;
  paint(run);
  if (["successful", "failed", "canceled"].includes(run.status)) return;
  pollTimer = setInterval(async () => {
    if (live !== generation) return;
    try {
      const next = await api("GET", `/api/workflow-jobs/${encodeURIComponent(id)}`);
      if (live !== generation) return;
      paint(next);
      if (["successful", "failed", "canceled"].includes(next.status)) {
        clearInterval(pollTimer);
        pollTimer = null;
      }
    } catch (error) {
      if (live === generation) handle(error);
    }
  }, 1000);
}

async function pageHooks() {
  if (!needTeam()) return;
  const hooks = await api("GET", "/api/hooks");
  const deliveryRows = hooks.length
    ? await api("GET", `/api/hooks/${encodeURIComponent(hooks[0].id)}/deliveries`)
    : [];
  const origin = location.origin;
  const incoming = deliveryRows.map((item) =>
    `<tr><td>${esc(item.hook_name)}</td><td>${pill(item.status)}</td><td>${esc(item.summary)}</td><td class="small muted">${esc(item.created_at)}</td></tr>`
  );
  shell(t("webhooks"), `
    <section class="card" style="margin-bottom:14px">
      <input class="search" id="hook-search" placeholder="${h("search_webhooks")}" autocomplete="off">
      <div id="hook-table"></div>
    </section>
    <section class="card"><h2>${h("deliveries")}</h2>${table([t("hook"), t("status"), t("summary"), t("time")], incoming)}</section>`,
    can("operator") ? addLink("#/hooks/new", "webhook_2") : "");
  const drawHooks = () => {
    const needle = document.querySelector("#hook-search").value.trim().toLowerCase();
    const rows = hooks.filter((hook) => !needle || `${hook.name} ${hook.match_alertname} ${hook.match_status}`.toLowerCase().includes(needle)).map((hook) => {
      const url = hook.token ? `${origin}/hooks/${hook.id}/${hook.token}` : "";
      return `<tr>
        <td>${esc(hook.name)}<div class="small muted">${esc(hook.match_alertname)} · ${esc(hook.match_status)}</div></td>
        <td class="mono">${url ? esc(url) : h("token_is_visible_to_operators")}</td>
        <td>${can("operator") ? `<button class="btn" type="button" data-test="${esc(hook.id)}">${h("test")}</button> <button class="btn danger" type="button" data-delete="${esc(hook.id)}">${h("delete")}</button>` : ""}</td>
      </tr>`;
    });
    document.querySelector("#hook-table").innerHTML = table([t("hook"), t("address"), ""], rows);
    document.querySelectorAll("[data-test]").forEach((button) => {
      button.addEventListener("click", async () => {
        try {
          const result = await api("POST", `/api/hooks/${encodeURIComponent(button.dataset.test)}/test`);
          if (result.workflow_job_id) go(`#/runs/${result.workflow_job_id}`);
          else if (result.job_id) go(`#/jobs/${result.job_id}`);
          else route();
        } catch (error) {
          handle(error);
        }
      });
    });
    document.querySelectorAll("[data-delete]").forEach((button) => {
      button.addEventListener("click", async () => {
        if (!confirm(t("delete_this_webhook"))) return;
        try {
          await api("DELETE", `/api/hooks/${encodeURIComponent(button.dataset.delete)}`);
          route();
        } catch (error) {
          handle(error);
        }
      });
    });
  };
  document.querySelector("#hook-search").addEventListener("input", drawHooks);
  drawHooks();
}

async function pageHookNew() {
  if (!needTeam()) return;
  if (!can("operator")) {
    deny("webhook", "operator");
    return;
  }
  const [templates, workflows] = await Promise.all([
    api("GET", "/api/templates"),
    api("GET", "/api/workflows"),
  ]);
  shell(t("new_webhook"), `
    <form class="card form" id="hook-form">
      <label class="field"><span>${h("name")}</span><input name="name" required></label>
      <div class="field-row">
        <label class="field"><span>${h("alert_name")}</span><input class="fit" name="match_alertname" required placeholder="DiskSpaceLow"></label>
        <label class="field"><span>${h("status")}</span>
          <select class="fit" name="match_status"><option value="firing">firing</option><option value="resolved">resolved</option><option value="any">any</option></select>
        </label>
      </div>
      <label class="field"><span>${h("target")}</span>
        <select name="target">
          <optgroup label="${h("workflow")}">${workflows.map((item) => `<option value="workflow:${esc(item.id)}">${esc(item.name)}</option>`).join("")}</optgroup>
          <optgroup label="${h("template")}">${templates.map((item) => `<option value="template:${esc(item.id)}">${esc(item.name)}</option>`).join("")}</optgroup>
        </select>
      </label>
      <label class="field"><span>${h("extra_variables_json")}</span><textarea name="extra_vars">{}</textarea></label>
      <button class="btn pill primary" type="submit">${h("create")}</button>
    </form>`);
  document.querySelector("#hook-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const rawTarget = form.target.value;
      const splitAt = rawTarget.indexOf(":");
      if (splitAt < 1) throw new Error("create_a_template_or_a_workflow_first");
      const kind = rawTarget.slice(0, splitAt);
      const targetId = rawTarget.slice(splitAt + 1);
      await api("POST", "/api/hooks", {
        name: form.name.value,
        match_alertname: form.match_alertname.value,
        match_status: form.match_status.value,
        extra_vars: jsonObject(form.extra_vars.value, "extra_variables"),
        template_id: kind === "template" ? targetId : null,
        workflow_id: kind === "workflow" ? targetId : null,
      });
      go("#/hooks");
    } catch (error) {
      handle(error);
    }
  });
}

async function pageRuntime() {
  if (!needTeam()) return;
  const [cache, plugins, environments] = await Promise.all([
    api("GET", "/api/cache"),
    api("GET", "/api/plugins"),
    api("GET", "/api/environments"),
  ]);
  const collectionRows = (cache.collections || []).map((item) => `<tr><td>${esc(item.name)}</td><td>${esc(item.version)}</td></tr>`);
  const roleRows = (cache.roles || []).map((item) => `<tr><td>${esc(item.name)}</td><td>${esc(item.version)}</td></tr>`);
  const pluginRows = plugins.map((item) => `<tr><td class="tool">${esc(item.id)}</td><td>${esc(t(item.summary))}</td></tr>`);
  shell(t("runtime"), `
    <section class="card" style="margin-bottom:14px">
      <h2>${h("execution_environments")}</h2>
      <input class="search" id="environment-search" placeholder="${h("search_environments")}" autocomplete="off">
      <div id="environment-table"></div>
    </section>
    <div class="grid-3" style="margin-bottom:14px">
      <article class="card stat"><span class="small muted">${h("collections")}</span><strong>${esc(bytes(cache.collections_bytes))}</strong></article>
      <article class="card stat"><span class="small muted">${h("roles")}</span><strong>${esc(bytes(cache.roles_bytes))}</strong></article>
      <article class="card stat"><span class="small muted">${h("opentofu")}</span><strong>${esc(bytes(cache.tofu_bytes))}</strong></article>
    </div>
    <p class="small muted" style="margin-top:0">${esc(t(cache.note || ""))}</p>
    <section class="card" style="margin-bottom:14px"><h2>${h("tools")}</h2>${table([t("id"), t("description")], pluginRows)}</section>
    <section class="card" style="margin-bottom:14px"><h2>${h("cached_collections")}</h2>${table([t("name"), t("version")], collectionRows)}</section>
    <section class="card" style="margin-bottom:14px"><h2>${h("cached_roles")}</h2>${table([t("name"), t("version")], roleRows)}</section>`,
    can("owner") ? addLink("#/environments/new", "environment") : "");
  const drawEnvironments = () => {
    const needle = document.querySelector("#environment-search").value.trim().toLowerCase();
    const rows = environments.filter((item) => !needle || `${item.name} ${item.image} ${item.pull_policy} ${item.scope || ""} ${scopeLabel(item)}`.toLowerCase().includes(needle)).map((item) =>
      `<tr><td><a href="#/environments/${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(scopeLabel(item))}</td><td class="mono">${esc(item.image)}</td><td>${esc(item.pull_policy)}</td><td>${canManageEnvironment(item) ? `<button class="btn danger" type="button" data-env="${esc(item.id)}">${h("delete")}</button>` : ""}</td></tr>`
    );
    document.querySelector("#environment-table").innerHTML = table([t("name"), t("applies_to"), t("image"), t("pull"), ""], rows);
    document.querySelectorAll("[data-env]").forEach((button) => {
      button.addEventListener("click", async () => {
        try {
          await api("DELETE", `/api/environments/${encodeURIComponent(button.dataset.env)}`);
          route();
        } catch (error) {
          handle(error);
        }
      });
    });
  };
  document.querySelector("#environment-search").addEventListener("input", drawEnvironments);
  drawEnvironments();
}

function canManageEnvironment(item) {
  if (!can("owner")) return false;
  if (item && item.scope === "global") return !!me.is_admin;
  return true;
}

function scopeLabel(item) {
  if (!item || item.scope === "global") return t("global");
  if (item.scope === "organization") return t("organization_name", { name: item.organization_name || "" });
  return t("team_name", { name: item.team_name || "" });
}

function environmentScopeOptions(selected) {
  const choices = [
    ["team", "this_team"],
    ["organization", "this_organization"],
  ];
  if (me.is_admin) choices.push(["global", "global"]);
  return choices.map(([value, label]) =>
    `<option value="${value}"${value === selected ? " selected" : ""}>${h(label)}</option>`
  ).join("");
}

function environmentForm(item) {
  const policy = item ? item.pull_policy : "if_not_present";
  const scope = item && item.scope ? item.scope : "team";
  const option = (value) => `<option value="${value}"${policy === value ? " selected" : ""}>${value}</option>`;
  return `<form class="card form" id="env-form">
    <label class="field"><span>${h("name")}</span><input name="name" required value="${item ? esc(item.name) : ""}"></label>
    <label class="field"><span>${h("image")}</span><input name="image" required value="${item ? esc(item.image) : ""}" placeholder="quay.io/ansible/awx-ee:latest"></label>
    <div class="field-row">
      <label class="field"><span>${h("pull")}</span><select class="fit" name="pull_policy">${option("if_not_present")}${option("always")}${option("never")}</select></label>
      <label class="field"><span>${h("applies_to")}</span><select class="fit" name="scope">${environmentScopeOptions(scope)}</select></label>
    </div>
    <p class="small muted">${h("global_applies_to_every_team_the_organization_applies")}</p>
    <label class="field"><span>${h("description")}</span><input name="description" value="${item ? esc(item.description) : ""}"></label>
    <button class="btn pill primary" type="submit">${item ? h("save") : h("create")}</button>
  </form>`;
}

function bindEnvironmentForm(id) {
  document.querySelector("#env-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const payload = {
      name: form.name.value,
      image: form.image.value,
      pull_policy: form.pull_policy.value,
      description: form.description.value,
      scope: form.scope.value,
    };
    try {
      if (id) await api("PUT", `/api/environments/${encodeURIComponent(id)}`, payload);
      else await api("POST", "/api/environments", payload);
      go("#/runtime");
    } catch (error) {
      handle(error);
    }
  });
}

async function pageEnvironmentNew() {
  if (!needTeam()) return;
  if (!can("owner")) {
    deny("execution_environment", "owner");
    return;
  }
  shell(t("new_environment"), environmentForm(null));
  bindEnvironmentForm(null);
}

async function pageEnvironment(id) {
  if (!needTeam()) return;
  const item = await api("GET", `/api/environments/${encodeURIComponent(id)}`);
  if (!canManageEnvironment(item)) {
    shell(item.name, `<section class="card"><dl class="kv">
      ${paramRow("name", item.name)}
      ${paramRow("applies_to", scopeLabel(item))}
      ${paramRow("image", item.image)}
      ${paramRow("pull", item.pull_policy)}
      ${paramRow("description", item.description)}
    </dl></section>`);
    return;
  }
  shell(item.name, environmentForm(item), `<button class="btn danger" id="delete-env" type="button">${h("delete")}</button>`);
  bindEnvironmentForm(id);
  document.querySelector("#delete-env").addEventListener("click", async () => {
    try {
      await api("DELETE", `/api/environments/${encodeURIComponent(id)}`);
      go("#/runtime");
    } catch (error) {
      handle(error);
    }
  });
}

function grantLabel(item) {
  return item.team_ids && item.team_ids.length
    ? t("count_teams", { count: item.team_ids.length })
    : t("all_teams_in_the_organization");
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

async function pageProjects() {
  if (!needTeam()) return;
  const projects = await api("GET", "/api/projects");
  shell(t("projects"), `
    <section class="card">
      <input class="search" id="project-search" placeholder="${h("search_projects")}" autocomplete="off">
      <div id="project-table"></div>
    </section>`,
    can("operator") ? addLink("#/projects/new", "project_2") : "");
  const drawProjects = () => {
    const needle = document.querySelector("#project-search").value.trim().toLowerCase();
    const rows = projects.filter((item) => !needle || `${item.name} ${item.organization_name || ""} ${item.source_type} ${grantLabel(item)}`.toLowerCase().includes(needle)).map((item) =>
      `<tr><td><a href="#/projects/${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(item.organization_name || "")}</td><td>${esc(item.source_type)}</td><td class="small muted">${esc(grantLabel(item))}</td></tr>`
    );
    document.querySelector("#project-table").innerHTML = table([t("name"), t("organization"), t("type"), t("visibility")], rows);
  };
  document.querySelector("#project-search").addEventListener("input", drawProjects);
  drawProjects();
}

async function pageProjectNew() {
  if (!needTeam()) return;
  if (!can("operator")) {
    deny("project", "operator");
    return;
  }
  const organizations = me.is_admin ? await api("GET", "/api/organizations") : [];
  const current = (me.teams || []).find((team) => team.id === teamId);
  shell(t("new_project"), `
    <form class="card form" id="project-form">
      <label class="field"><span>${h("name")}</span><input name="name" required></label>
      <div class="field-row">
        <label class="field"><span>${h("type")}</span><select class="fit" name="source_type"><option value="path">${h("path")}</option><option value="git">Git</option></select></label>
        <label class="field"><span>${h("revision")}</span><input class="fit" name="revision" placeholder="${h("branch_tag_or_commit")}"></label>
      </div>
      <label class="field"><span>${h("source_2")}</span><input name="source" required placeholder="${h("https_or_file")}"></label>
      ${me.is_admin ? `<label class="field"><span>${h("organization")}</span><select name="organization_id">${orgOptions(organizations, current && current.organization_id)}</select></label>` : ""}
      <button class="btn pill primary" type="submit">${h("create")}</button>
    </form>`);
  document.querySelector("#project-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const payload = {
        name: form.name.value,
        source_type: form.source_type.value,
        source: form.source.value,
        revision: form.revision.value,
        team_ids: [],
      };
      if (form.organization_id) payload.organization_id = form.organization_id.value;
      const created = await api("POST", "/api/projects", payload);
      go(`#/projects/${created.id}`);
    } catch (error) { handle(error); }
  });
}

async function pageProject(id) {
  if (!needTeam()) return;
  const project = await api("GET", `/api/projects/${encodeURIComponent(id)}`);
  const organizations = await api("GET", "/api/organizations");
  const org = await api("GET", `/api/organizations/${encodeURIComponent(project.organization_id)}`);
  shell(project.name, `
    <section class="card" style="margin-bottom:14px">
      <p class="muted">${h("source_source", { source: `${project.source_type} · ${project.source}` })}</p>
      ${project.revision ? `<p class="small">${h("revision_revision", { revision: project.revision })}</p>` : ""}
      ${can("operator") && project.source_type === "git" ? `<div class="row"><button class="btn" id="sync" type="button">${h("sync")}</button></div><p class="small" id="sync-note"></p>` : ""}
    </section>
    <section class="card">
      <h2>${h("assignment")}</h2>
      <p class="small muted">${h("with_no_team_checked_every_team_in_the")}</p>
      ${can("operator") ? `<form class="form" id="assign-form">
        <label class="field"><span>${h("organization")}</span><select name="organization_id"${me.is_admin ? "" : " disabled"}>${orgOptions(organizations, project.organization_id)}</select></label>
        <div class="stack" id="team-boxes">${teamBoxes(org.teams || [], project.team_ids)}</div>
        <button class="btn pill primary" type="submit">${h("save_assignment")}</button>
      </form>` : `<p>${h("organization_name_visibility_grant", { name: project.organization_name || "", grant: grantLabel(project) })}</p>`}
    </section>`,
    can("operator") ? `<button class="btn danger" id="delete-project" type="button">${h("delete")}</button>` : "");
  const sync = document.querySelector("#sync");
  if (sync) {
    sync.addEventListener("click", async () => {
      sync.disabled = true;
      const note = document.querySelector("#sync-note");
      try {
        const result = await api("POST", `/api/projects/${encodeURIComponent(id)}/sync`, {});
        note.textContent = t("checkout_path", { path: result.path });
        sync.textContent = t("synced");
      } catch (error) {
        sync.disabled = false;
        if (note) note.textContent = t(error.message || "sync_failed", error.vars);
        handle(error);
      }
    });
  }
  const form = document.querySelector("#assign-form");
  if (form) {
    form.organization_id.addEventListener("change", async () => {
      const box = document.querySelector("#team-boxes");
      box.innerHTML = `<p class="small muted">${h("loading_teams")}</p>`;
      try {
        const next = await api("GET", `/api/organizations/${encodeURIComponent(form.organization_id.value)}`);
        box.innerHTML = teamBoxes(next.teams || [], []);
      } catch (error) { handle(error); }
    });
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        const teamIds = [...form.querySelectorAll("input[name=team_ids]:checked")].map((input) => input.value);
        await api("PUT", `/api/projects/${encodeURIComponent(id)}`, {
          organization_id: form.organization_id.value,
          team_ids: teamIds,
        });
        route();
      } catch (error) { handle(error); }
    });
  }
  const remove = document.querySelector("#delete-project");
  if (remove) {
    remove.addEventListener("click", async () => {
      if (!confirm(t("delete_this_project"))) return;
      try {
        await api("DELETE", `/api/projects/${encodeURIComponent(id)}`);
        go("#/projects");
      } catch (error) { handle(error); }
    });
  }
}

function sourceKindLabel(kind) {
  return { ini: "INI", project: t("project"), netbox: t("netbox"), mixed: t("several") }[kind] || kind || "INI";
}

function sourceSummary(item) {
  const data = item.source || {};
  if (item.kind === "netbox") return data.api_endpoint || t("netbox");
  if (item.kind === "project") return data.path || t("project");
  return t("text");
}

function certificateCheckOff(source) {
  const data = (source && source.source) || {};
  return source && source.kind === "netbox" && data.validate_certs === false && !String(data.ca_file || "").trim();
}

function certificateBadge(on) {
  return on ? ` <span class="pill warn">${h("certificate_check_off")}</span>` : "";
}

function mountSourceDraft(container, projects, kind, preset) {
  const data = preset || {};
  const card = document.createElement("section");
  card.className = "card";
  card.dataset.kind = kind;
  card.style.marginTop = "10px";
  const projectOptions = projects.map((item) => `<option value="${esc(item.id)}">${esc(item.name)}</option>`).join("");
  const title = kind === "ini" ? t("manual") : kind === "project" ? t("project") : "netbox";
  let fields = "";
  if (kind === "ini") {
    fields = `<label class="field"><span>${h("ini_or_yaml")}</span><textarea name="body" required>${esc(data.body || "[all]\nlocalhost ansible_connection=local")}</textarea></label>`;
  } else if (kind === "project") {
    fields = `<label class="field"><span>${h("project")}</span><select name="project_id"><option value="">—</option>${projectOptions}</select></label>
      <label class="field"><span>${h("inventory_file")}</span><select name="path"><option value="">—</option></select></label>
      <p class="small muted" data-plugin-note></p>`;
  } else {
    const groupBy = Array.isArray(data.group_by) ? data.group_by.join(", ") : (data.group_by || "");
    const filters = Array.isArray(data.query_filters) ? data.query_filters.join("\n") : (data.query || "");
    fields = `<label class="field"><span>${h("api_address")}</span><input name="api_endpoint" required value="${esc(data.api_endpoint || "")}" placeholder="https://netbox.example.com"></label>
      <label class="field"><span>${h("token")}</span><input name="token" type="password" required autocomplete="off"></label>
      <label class="field"><span>${h("group_by")}</span><input name="group_by" value="${esc(groupBy)}" placeholder="sites, device_roles"></label>
      <label class="field"><span>${h("filters_one_key_value_per_line")}</span><textarea name="query">${esc(filters)}</textarea></label>
      <label class="field"><span>${h("verify_certificate")}</span><input name="validate_certs" type="checkbox"${data.validate_certs === false ? "" : " checked"}></label>
      <label class="field"><span>${h("private_ca_file")}</span><input name="ca_file" value="${esc(data.ca_file || "")}" placeholder="/etc/ssl/certs/internal-ca.pem" autocomplete="off" spellcheck="false"></label>
      <p class="small muted">${h("an_absolute_pem_path_empty_uses_the_system_cas")}</p>
      <label class="field"><span>${h("virtual_machines")}</span><input name="virtual_machines" type="checkbox"${data.virtual_machines === false ? "" : " checked"}></label>`;
  }
  card.innerHTML = `<div class="spread"><h2>${esc(title)}</h2><button class="btn ghost" type="button" data-remove-draft>${h("remove")}</button></div>
    <div class="form">${fields}
      <label class="field"><span>${h("source_name")}</span><input name="source_name" value="${esc(data.name || "")}" placeholder="${esc(title)}"></label>
    </div>`;
  container.appendChild(card);
  card.querySelector("[data-remove-draft]").addEventListener("click", () => card.remove());
  if (kind === "project") {
    const select = card.querySelector("[name=project_id]");
    const files = card.querySelector("[name=path]");
    const note = card.querySelector("[data-plugin-note]");
    select.addEventListener("change", async () => {
      files.innerHTML = `<option value="">—</option>`;
      note.textContent = "";
      if (!select.value) return;
      try {
        const contents = await api("GET", `/api/projects/${encodeURIComponent(select.value)}/contents`);
        const inventories = contents.inventories || [];
        const staticFiles = inventories.filter((file) => file.format !== "plugin");
        files.innerHTML = `<option value="">—</option>${staticFiles.map((file) => `<option value="${esc(file.path)}">${esc(file.path)}</option>`).join("")}`;
        const plugins = inventories.filter((file) => file.format === "plugin");
        if (plugins.length) {
          note.textContent = t("files_is_not_executed_here_add_netbox_as", {
            files: plugins.map((file) => `${file.path} (${file.plugin || "Plugin"})`).join(", "),
          });
        } else if (!staticFiles.length) {
          note.textContent = t("no_inventory_files_sync_the_git_project_first");
        }
      } catch (error) {
        note.textContent = t(error.message || "the_project_cannot_be_read", error.vars);
      }
    });
  }
  return card;
}

function readSourceDraft(card) {
  const kind = card.dataset.kind;
  const name = card.querySelector("[name=source_name]").value.trim();
  if (kind === "ini") return { kind, name, body: card.querySelector("[name=body]").value };
  if (kind === "project") {
    return {
      kind,
      name,
      project_id: card.querySelector("[name=project_id]").value,
      path: card.querySelector("[name=path]").value,
    };
  }
  return {
    kind,
    name,
    api_endpoint: card.querySelector("[name=api_endpoint]").value,
    token: card.querySelector("[name=token]").value,
    group_by: card.querySelector("[name=group_by]").value.split(",").map((item) => item.trim()).filter(Boolean),
    query_filters: card.querySelector("[name=query]").value.split("\n").map((item) => item.trim()).filter(Boolean),
    validate_certs: card.querySelector("[name=validate_certs]").checked,
    ca_file: card.querySelector("[name=ca_file]").value.trim(),
    virtual_machines: card.querySelector("[name=virtual_machines]").checked,
  };
}

function bindSourcePicks(projects) {
  document.querySelectorAll("[data-add-kind]").forEach((button) => {
    button.addEventListener("click", () => {
      mountSourceDraft(document.querySelector("#source-drafts"), projects, button.dataset.addKind);
    });
  });
}

async function pageInventories() {
  if (!needTeam()) return;
  const items = await api("GET", "/api/inventories");
  shell(t("inventories"), `
    <section class="card">
      <input class="search" id="inventory-search" placeholder="${h("search_inventories")}" autocomplete="off">
      <div id="inventory-table"></div>
    </section>`,
    can("operator") ? addLink("#/inventories/new", "inventory_2") : "");
  const draw = () => {
    const needle = document.querySelector("#inventory-search").value.trim().toLowerCase();
    const shown = items.filter((item) => {
      if (!needle) return true;
      const kinds = (item.sources || []).map((source) => `${source.name} ${sourceKindLabel(source.kind)} ${sourceSummary(source)}`).join(" ");
      return `${item.name} ${kinds}`.toLowerCase().includes(needle);
    });
    const rows = shown.map((item) => {
      const kinds = (item.sources || []).map((source) => sourceKindLabel(source.kind));
      const label = kinds.length ? [...new Set(kinds)].join(", ") : sourceKindLabel(item.source_kind);
      const count = item.source_count != null ? item.source_count : kinds.length;
      const insecure = (item.sources || []).some(certificateCheckOff) || certificateCheckOff({ kind: item.source_kind, source: item.source });
      return `<tr><td><a href="#/inventories/${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(count)}</td><td>${esc(label)}${certificateBadge(insecure)}</td><td>${esc(item.host_count)}</td></tr>`;
    });
    document.querySelector("#inventory-table").innerHTML = table([t("name"), t("sources"), t("type"), t("hosts")], rows);
  };
  document.querySelector("#inventory-search").addEventListener("input", draw);
  draw();
}

async function pageInventoryNew() {
  if (!needTeam()) return;
  if (!can("operator")) {
    deny("inventory", "operator");
    return;
  }
  const projects = await api("GET", "/api/projects");
  shell(t("new_inventory"), `
    <form class="card form" id="inventory-create">
      <label class="field"><span>${h("name")}</span><input name="name" required></label>
      <div>
        <h2>${h("sources")}</h2>
        <p class="small muted">${h("an_inventory_can_have_several_sources_they_are")}</p>
        <div class="choice-row">
          <button class="btn pill" type="button" data-add-kind="ini">${h("manual_2")}</button>
          <button class="btn pill" type="button" data-add-kind="project">${h("project_2")}</button>
          <button class="btn pill" type="button" data-add-kind="netbox">${h("netbox_2")}</button>
        </div>
        <div id="source-drafts"></div>
      </div>
      <button class="btn pill primary" type="submit">${h("create")}</button>
    </form>`);
  bindSourcePicks(projects);
  document.querySelector("#inventory-create").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const sources = [...document.querySelectorAll("#source-drafts [data-kind]")].map(readSourceDraft);
      const created = await api("POST", "/api/inventories", { name: form.name.value, sources });
      go(`#/inventories/${created.id}`);
    } catch (error) { handle(error); }
  });
}

async function pageInventory(id) {
  if (!needTeam()) return;
  const [item, projects] = await Promise.all([
    api("GET", `/api/inventories/${encodeURIComponent(id)}`),
    can("operator") ? api("GET", "/api/projects") : Promise.resolve([]),
  ]);
  const rows = (item.hosts || []).map((host) => {
    const vars = Object.entries(host.vars || {}).map(([key, value]) => `${key}=${value}`).join(" ");
    return `<tr><td class="mono">${esc(host.name)}</td><td>${esc((host.groups || []).join(", "))}</td><td class="mono">${esc(vars)}</td></tr>`;
  });
  const external = (item.sources || []).some((source) => source.kind === "project" || source.kind === "netbox");
  const sourceCards = (item.sources || []).map((source) => `
    <section class="card" style="margin-top:10px">
      <div class="spread">
        <div><strong>${esc(source.name)}</strong>${certificateBadge(certificateCheckOff(source))} <span class="small muted">${esc(sourceKindLabel(source.kind))} · ${esc(sourceSummary(source))}</span></div>
        <div class="row">
          ${can("operator") && source.kind !== "ini" ? `<button class="btn" type="button" data-sync-source="${esc(source.id)}">${h("refresh")}</button>` : ""}
          ${can("operator") ? `<button class="btn danger" type="button" data-drop-source="${esc(source.id)}">${h("remove")}</button>` : ""}
        </div>
      </div>
      <p class="small muted">${h("count_hosts", { count: source.host_count })}</p>
      ${can("operator") && source.kind === "ini" ? `<form class="form" data-ini-source="${esc(source.id)}">
        <label class="field"><span>${h("text")}</span><textarea name="body" required>${esc(source.body || "")}</textarea></label>
        <button class="btn" type="submit">${h("save_source")}</button>
      </form>` : `<pre class="mono">${esc(source.body || "")}</pre>`}
    </section>`).join("");
  shell(item.name, `
    <section class="card" style="margin-bottom:14px">
      <div class="spread">
        <h2>${h("hosts")}</h2>
        ${external && can("operator") ? `<button class="btn" id="sync-inventory" type="button">${h("refresh_all_sources")}</button>` : ""}
      </div>
      ${table([t("name"), t("groups"), t("variables")], rows)}
    </section>
    <section>
      <div class="spread">
        <h2>${h("sources")}</h2>
        ${can("operator") ? `<button class="btn danger" id="delete-inventory" type="button">${h("delete_inventory")}</button>` : ""}
      </div>
      ${sourceCards || `<p class="small muted">${h("no_source_yet")}</p>`}
      ${can("operator") ? `<div class="card" style="margin-top:14px">
        <h2>${h("add_source")}</h2>
        <div class="choice-row">
          <button class="btn pill" type="button" data-add-kind="ini">${h("manual_2")}</button>
          <button class="btn pill" type="button" data-add-kind="project">${h("project_2")}</button>
          <button class="btn pill" type="button" data-add-kind="netbox">${h("netbox_2")}</button>
        </div>
        <div id="source-drafts"></div>
        <button class="btn pill primary" id="add-sources" type="button" style="margin-top:12px">${h("add")}</button>
      </div>` : ""}
    </section>`,
    can("operator") ? `<a class="btn pill" href="#/inventories/new">${h("inventory_2")}</a>` : "");
  const sync = document.querySelector("#sync-inventory");
  if (sync) {
    sync.addEventListener("click", async () => {
      sync.disabled = true;
      try {
        await api("POST", `/api/inventories/${encodeURIComponent(id)}/sync`);
        route();
      } catch (error) {
        sync.disabled = false;
        handle(error);
      }
    });
  }
  document.querySelectorAll("[data-sync-source]").forEach((button) => {
    button.addEventListener("click", async () => {
      button.disabled = true;
      try {
        await api("POST", `/api/inventories/${encodeURIComponent(id)}/sources/${encodeURIComponent(button.dataset.syncSource)}/sync`);
        route();
      } catch (error) {
        button.disabled = false;
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-drop-source]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (!confirm(t("remove_this_source"))) return;
      try {
        await api("DELETE", `/api/inventories/${encodeURIComponent(id)}/sources/${encodeURIComponent(button.dataset.dropSource)}`);
        route();
      } catch (error) { handle(error); }
    });
  });
  document.querySelectorAll("[data-ini-source]").forEach((form) => {
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        await api("PUT", `/api/inventories/${encodeURIComponent(id)}/sources/${encodeURIComponent(form.dataset.iniSource)}`, {
          body: form.body.value,
        });
        route();
      } catch (error) { handle(error); }
    });
  });
  if (can("operator")) {
    bindSourcePicks(projects);
    document.querySelector("#add-sources").addEventListener("click", async () => {
      const drafts = [...document.querySelectorAll("#source-drafts [data-kind]")].map(readSourceDraft);
      if (!drafts.length) return;
      try {
        for (const source of drafts) {
          await api("POST", `/api/inventories/${encodeURIComponent(id)}/sources`, source);
        }
        route();
      } catch (error) { handle(error); }
    });
    document.querySelector("#delete-inventory").addEventListener("click", async () => {
      if (!confirm(t("delete_this_inventory"))) return;
      try {
        await api("DELETE", `/api/inventories/${encodeURIComponent(id)}`);
        go("#/inventories");
      } catch (error) { handle(error); }
    });
  }
}
const CREDENTIAL_KINDS = [
  ["ssh", "machine_ssh", "a_key_or_a_password_or_both"],
  ["vault", "ansible_vault", ""],
  ["scm", "source_control", "applies_to_the_job_project_sync_does_not"],
  ["net", "network", ""],
  ["aws", "amazon_web_services", ""],
  ["azure", "microsoft_azure", ""],
  ["gce", "google_compute_engine", ""],
  ["vmware", "vmware_vcenter", ""],
  ["openstack", "openstack", "empty_sets_the_domain_to_default"],
  ["registry", "container_registry", "the_job_sees_the_host_username_and_password"],
  ["github", "github_token", ""],
  ["gitlab", "gitlab_token", ""],
  ["galaxy", "ansible_galaxy", "empty_keeps_https_galaxy_ansible_com"],
  ["kubernetes", "kubernetes", ""],
  ["insights", "red_hat_insights", ""],
  ["env", "environment_variables", ""],
  ["netbox", "netbox", ""],
];

function credentialKindLabel(kind) {
  const row = CREDENTIAL_KINDS.find((item) => item[0] === kind);
  return row ? t(row[1]) : kind;
}

function selectedCredentialIds(item) {
  if (Array.isArray(item.credential_ids)) return item.credential_ids;
  return item.credential_id ? [item.credential_id] : [];
}

function templateCredentialFields(credentials, selectedIds) {
  const selected = new Set(selectedIds || []);
  const present = new Set(credentials.map((row) => row.kind));
  const selects = CREDENTIAL_KINDS
    .map((row) => row[0])
    .filter((kind) => kind !== "vault" && present.has(kind))
    .map((kind) => {
      const rows = credentials.filter((row) => row.kind === kind);
      const current = rows.find((row) => selected.has(row.id));
      return `<label class="field"><span>${esc(credentialKindLabel(kind))}</span><select class="fit" data-credential-kind="${esc(kind)}">${selectOptions(rows, current ? current.id : "", true, (row) => row.name)}</select></label>`;
    })
    .join("");
  const grid = selects ? `<div class="field-grid">${selects}</div>` : "";
  const empty = credentials.length ? "" : `<p class="small muted">${h("no_credentials_in_this_team")}</p>`;
  return `${empty}<p class="small muted">${h("one_credential_of_each_kind_choose_vault_credentials")}</p>${grid}
    <div class="field">
      <span>${esc(credentialKindLabel("vault"))}</span>
      <div class="vault-row">
        <div class="chips" id="vault-pills"></div>
        <button class="btn pill" id="vault-add" type="button">${h("select")}</button>
      </div>
    </div>`;
}

function renderVaultPills(box, credentials, ids) {
  const byId = new Map(credentials.map((row) => [row.id, row]));
  box.innerHTML = ids.map((id) => {
    const row = byId.get(id);
    if (!row) return "";
    return `<span class="chip" data-vault-id="${esc(id)}"><span class="chip-text">${esc(row.name)}</span><button class="chip-remove" type="button" aria-label="${h("remove_vault_credential_name", { name: row.name })}">×</button></span>`;
  }).join("");
  box.querySelectorAll(".chip-remove").forEach((button) => {
    button.addEventListener("click", () => {
      const chip = button.closest(".chip");
      const id = chip ? chip.dataset.vaultId : "";
      const next = [...box.querySelectorAll("[data-vault-id]")]
        .map((node) => node.dataset.vaultId)
        .filter((item) => item !== id);
      renderVaultPills(box, credentials, next);
    });
  });
}

function closeVaultPicker() {
  const overlay = document.querySelector("#vault-picker");
  if (overlay) overlay.remove();
  if (closeVaultPicker.onKey) {
    document.removeEventListener("keydown", closeVaultPicker.onKey);
    closeVaultPicker.onKey = null;
  }
}

function openVaultPicker(credentials, selected, onApply) {
  closeVaultPicker();
  const chosen = new Set(selected);
  const vaults = credentials
    .filter((row) => row.kind === "vault")
    .slice()
    .sort((left, right) => left.name.localeCompare(right.name, "de"));
  const rows = vaults.map((row) => `
    <label class="vault-choice${chosen.has(row.id) ? " selected" : ""}">
      <input type="checkbox" value="${esc(row.id)}"${chosen.has(row.id) ? " checked" : ""}>
      <span>${esc(row.name)}</span>
    </label>`).join("");
  const body = vaults.length
    ? `<input class="search" id="vault-search" placeholder="${h("search_vaults")}" autocomplete="off">
       <div class="vault-choices" id="vault-choices">${rows}</div>
       <div class="row">
         <button class="btn pill primary" type="button" id="vault-apply">${h("apply")}</button>
         <button class="btn" type="button" id="vault-cancel">${h("cancel")}</button>
       </div>`
    : `<p class="muted">${h("no_vault_credentials_in_this_team")}</p>`;
  document.querySelector(".main").insertAdjacentHTML("beforeend", `
    <div class="overlay" id="vault-picker">
      <div class="overlay-card vault-picker" role="dialog" aria-modal="true" aria-labelledby="vault-picker-title">
        <div class="spread">
          <h2 id="vault-picker-title">${esc(credentialKindLabel("vault"))}</h2>
          <button class="btn" type="button" id="vault-picker-close">${h("close")}</button>
        </div>
        ${body}
      </div>
    </div>`);
  const overlay = document.querySelector("#vault-picker");
  const dismiss = () => closeVaultPicker();
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) dismiss();
  });
  document.querySelector("#vault-picker-close").addEventListener("click", dismiss);
  const cancel = document.querySelector("#vault-cancel");
  if (cancel) cancel.addEventListener("click", dismiss);
  closeVaultPicker.onKey = (event) => {
    if (event.key === "Escape") dismiss();
  };
  document.addEventListener("keydown", closeVaultPicker.onKey);
  document.querySelectorAll(".vault-choice").forEach((label) => {
    const input = label.querySelector("input");
    input.addEventListener("change", () => label.classList.toggle("selected", input.checked));
  });
  const search = document.querySelector("#vault-search");
  if (search) {
    search.addEventListener("input", () => {
      const query = search.value.trim().toLowerCase();
      document.querySelectorAll(".vault-choice").forEach((label) => {
        const name = label.querySelector("span").textContent.toLowerCase();
        label.hidden = query !== "" && !name.includes(query);
      });
    });
    if (typeof search.focus === "function") search.focus();
  }
  const apply = document.querySelector("#vault-apply");
  if (!apply) return;
  apply.addEventListener("click", () => {
    const checked = [...document.querySelectorAll("#vault-choices input")]
      .filter((input) => input.checked)
      .map((input) => input.value);
    const kept = selected.filter((id) => checked.includes(id));
    const added = checked.filter((id) => !selected.includes(id));
    onApply([...kept, ...added]);
    closeVaultPicker();
  });
}

function bindVaultPicker(form, credentials, selectedIds) {
  const box = form.querySelector("#vault-pills");
  const open = form.querySelector("#vault-add");
  if (!box || !open) return;
  const vaultIds = new Set(credentials.filter((row) => row.kind === "vault").map((row) => row.id));
  renderVaultPills(box, credentials, (selectedIds || []).filter((id) => vaultIds.has(id)));
  open.addEventListener("click", () => {
    const current = [...box.querySelectorAll("[data-vault-id]")].map((node) => node.dataset.vaultId);
    openVaultPicker(credentials, current, (ids) => renderVaultPills(box, credentials, ids));
  });
}

function credText(name, label, type, value) {
  const inputType = type || "text";
  const complete = inputType === "password" ? "new-password" : "off";
  const held = value ? ` value="${esc(value)}"` : "";
  return `<label class="field"><span>${h(label)}</span><input name="${name}" type="${inputType}" autocomplete="${complete}"${held}></label>`;
}

function credArea(name, label) {
  return `<label class="field"><span>${h(label)}</span><textarea name="${name}" autocomplete="off" spellcheck="false"></textarea></label>`;
}

function credCheck(name, label, checked) {
  return `<label class="field check"><span>${h(label)}</span><input name="${name}" type="checkbox"${checked ? " checked" : ""}></label>`;
}

function credHint(text) {
  return text ? `<p class="small muted">${h(text)}</p>` : "";
}

function becomeSelect(selected) {
  const current = selected || "";
  const methods = ["sudo", "su", "pbrun", "pfexec", "doas", "dzdo", "ksu", "runas"];
  const options = [`<option value="">${h("none")}</option>`].concat(
    methods.map((method) => `<option value="${method}"${method === current ? " selected" : ""}>${esc(method)}</option>`)
  );
  return `<label class="field"><span>${h("become_method")}</span><select class="fit" name="become_method">${options.join("")}</select></label>`;
}

const FIELD_LABELS = {
  username: "user",
  password: "password",
  private_key: "private_key",
  passphrase: "passphrase",
  become_method: "become_method",
  become_username: "become_user",
  become_password: "become_password",
  vault_id: "vault_id",
  authorize: "authorize",
  authorize_password: "authorize_password",
  access_key: "access_key",
  secret_key: "secret_key",
  security_token: "session_token",
  subscription_id: "subscription",
  client_id: "client",
  secret: "secret",
  tenant: "tenant",
  service_account: "service_account",
  host: "host",
  auth_url: "api_address",
  project: "project",
  domain: "domain",
  url: "api_address",
  token: "token",
  verify_ssl: "verify_certificate",
};

function fieldLabel(key) {
  return FIELD_LABELS[key] || key;
}

function secretControl(name, label, widget, stored) {
  const clear = stored
    ? `<label class="field check"><input type="checkbox" data-clear="${esc(name)}"><span>${h("remove")}</span></label>`
    : "";
  return `<label class="field"><span>${h(label)}</span>${widget}</label>${clear}`;
}

function secretInput(name, label, type, stored) {
  const inputType = type || "text";
  const complete = inputType === "password" ? "new-password" : "off";
  const filled = stored ? ` value="********" data-stored-mask="1"` : "";
  return secretControl(name, label, `<input name="${name}" type="${inputType}" autocomplete="${complete}"${filled}>`, stored);
}

function secretArea(name, label, stored) {
  const filled = stored ? ` data-stored-mask="1"` : "";
  const body = stored ? "********" : "";
  return secretControl(
    name,
    label,
    `<textarea name="${name}" class="secret-mask" autocomplete="off" spellcheck="false"${filled}>${body}</textarea>`,
    stored
  );
}

function credentialFields(kind, current) {
  const row = CREDENTIAL_KINDS.find((item) => item[0] === kind);
  const hint = row ? row[2] : "";
  const fields = (current && current.fields) || {};
  const stored = new Set((current && current.set) || []);
  const text = (name, label, type) => credText(name, label, type, fields[name] == null ? "" : String(fields[name]));
  const has = (name) => stored.has(name);
  if (kind === "ssh") {
    return [
      credHint(hint),
      text("username", "user"),
      secretInput("password", "password", "password", has("password")),
      secretArea("private_key", "private_key", has("private_key")),
      secretInput("passphrase", "passphrase", "password", has("passphrase")),
      becomeSelect(fields.become_method),
      text("become_username", "become_user"),
      secretInput("become_password", "become_password", "password", has("become_password")),
    ].join("");
  }
  if (kind === "scm") {
    return [
      credHint(hint),
      text("username", "user"),
      secretInput("password", "password", "password", has("password")),
      secretArea("private_key", "private_key", has("private_key")),
      secretInput("passphrase", "passphrase", "password", has("passphrase")),
    ].join("");
  }
  if (kind === "vault") {
    return [
      secretInput("password", "vault_password", "password", has("password")),
      text("vault_id", "vault_id"),
    ].join("");
  }
  if (kind === "net") {
    return [
      text("username", "user"),
      secretInput("password", "password", "password", has("password")),
      secretArea("private_key", "private_key", has("private_key")),
      credCheck("authorize", "authorize", Boolean(fields.authorize)),
      secretInput("authorize_password", "authorize_password", "password", has("authorize_password")),
    ].join("");
  }
  if (kind === "aws") {
    return [
      text("access_key", "access_key"),
      secretInput("secret_key", "secret_key", "password", has("secret_key")),
      secretInput("security_token", "session_token", "password", has("security_token")),
    ].join("");
  }
  if (kind === "azure") {
    return [
      text("subscription_id", "subscription"),
      text("client_id", "client"),
      secretInput("secret", "secret", "password", has("secret")),
      text("tenant", "tenant"),
    ].join("");
  }
  if (kind === "gce") return secretArea("service_account", "service_account", has("service_account"));
  if (kind === "vmware") {
    return [
      text("host", "host"),
      text("username", "user"),
      secretInput("password", "password", "password", has("password")),
    ].join("");
  }
  if (kind === "openstack") {
    return [
      credHint(hint),
      text("auth_url", "api_address"),
      text("username", "user"),
      secretInput("password", "password", "password", has("password")),
      text("project", "project"),
      text("domain", "domain"),
    ].join("");
  }
  if (kind === "registry") {
    return [
      credHint(hint),
      text("host", "host"),
      text("username", "user"),
      secretInput("password", "password", "password", has("password")),
    ].join("");
  }
  if (kind === "github" || kind === "gitlab") return secretInput("token", "token", "password", has("token"));
  if (kind === "galaxy") {
    return [credHint(hint), text("url", "api_address"), secretInput("token", "token", "password", has("token"))].join("");
  }
  if (kind === "kubernetes") {
    return [
      text("host", "api_address"),
      secretInput("token", "token", "password", has("token")),
      credCheck("verify_ssl", "verify_certificate", current ? fields.verify_ssl !== false : true),
    ].join("");
  }
  if (kind === "insights") {
    return [text("username", "user"), secretInput("password", "password", "password", has("password"))].join("");
  }
  if (kind === "netbox") {
    return [text("url", "api_address"), secretInput("token", "token", "password", has("token"))].join("");
  }
  const listed = [...stored].map((name) =>
    `<label class="field check"><input type="checkbox" data-clear="${esc(name)}"><span>${esc(name)} ${h("remove")}</span></label>`
  ).join("");
  const keep = stored.size ? credHint("leave_empty_to_keep_the_stored_value") : "";
  return `${keep}${listed}${credArea("env", "key_value_one_variable_per_line")}`;
}

function inputText(form, name) {
  const field = form.elements[name];
  return field && "value" in field ? field.value.trim() : "";
}

function credentialSecret(form) {
  const kind = form.kind.value;
  if (kind === "env") {
    const secret = {};
    form.env.value.split("\n").forEach((line) => {
      const trimmed = line.trim();
      if (!trimmed) return;
      const eq = trimmed.indexOf("=");
      if (eq < 1) throw new Error("an_env_line_needs_key_value");
      secret[trimmed.slice(0, eq)] = trimmed.slice(eq + 1);
    });
    return secret;
  }
  const names = {
    ssh: ["username", "password", "private_key", "passphrase", "become_method", "become_username", "become_password"],
    vault: ["password", "vault_id"],
    scm: ["username", "password", "private_key", "passphrase"],
    net: ["username", "password", "private_key", "authorize_password"],
    aws: ["access_key", "secret_key", "security_token"],
    azure: ["subscription_id", "client_id", "secret", "tenant"],
    gce: ["service_account"],
    vmware: ["host", "username", "password"],
    openstack: ["auth_url", "username", "password", "project", "domain"],
    registry: ["host", "username", "password"],
    github: ["token"],
    gitlab: ["token"],
    galaxy: ["url", "token"],
    kubernetes: ["host", "token"],
    insights: ["username", "password"],
    netbox: ["token", "url"],
  };
  const secret = {};
  (names[kind] || []).forEach((name) => {
    const value = inputText(form, name);
    if (value) secret[name] = value;
  });
  if (kind === "net" && form.authorize && form.authorize.checked) secret.authorize = true;
  if (kind === "kubernetes") secret.verify_ssl = Boolean(form.verify_ssl && form.verify_ssl.checked);
  return secret;
}

function bindCredentialKind(form) {
  const box = document.querySelector("#credential-fields");
  const draw = () => {
    box.innerHTML = credentialFields(form.kind.value);
  };
  form.kind.addEventListener("change", draw);
  draw();
}

async function pageCredentials() {
  if (!needTeam()) return;
  const credentials = await api("GET", "/api/credentials");
  shell(t("credentials"), `
    <section class="card">
      <p class="small muted">${h("secrets_are_stored_encrypted_and_are_not_shown")}</p>
      <input class="search" id="credential-search" placeholder="${h("search_credentials")}" autocomplete="off">
      <div id="credential-table"></div>
    </section>`,
    can("owner") ? addLink("#/credentials/new", "credential") : "");
  const drawCredentials = () => {
    const needle = document.querySelector("#credential-search").value.trim().toLowerCase();
    const rows = credentials.filter((item) => !needle || `${item.name} ${item.kind} ${credentialKindLabel(item.kind)}`.toLowerCase().includes(needle)).map((item) =>
      `<tr><td><a href="#/credentials/${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(credentialKindLabel(item.kind))}</td><td>${can("owner") ? `<button class="btn danger" type="button" data-credential="${esc(item.id)}">${h("delete")}</button>` : ""}</td></tr>`
    );
    document.querySelector("#credential-table").innerHTML = table([t("name"), t("type"), ""], rows);
    document.querySelectorAll("[data-credential]").forEach((button) => {
      button.addEventListener("click", async () => {
        try {
          await api("DELETE", `/api/credentials/${encodeURIComponent(button.dataset.credential)}`);
          route();
        } catch (error) { handle(error); }
      });
    });
  };
  document.querySelector("#credential-search").addEventListener("input", drawCredentials);
  drawCredentials();
}

const SECRET_FIELD_NAMES = {
  ssh: ["password", "private_key", "passphrase", "become_password"],
  scm: ["password", "private_key", "passphrase"],
  vault: ["password"],
  net: ["password", "private_key", "authorize_password"],
  aws: ["secret_key", "security_token"],
  azure: ["secret"],
  gce: ["service_account"],
  vmware: ["password"],
  openstack: ["password"],
  registry: ["password"],
  github: ["token"],
  gitlab: ["token"],
  galaxy: ["token"],
  kubernetes: ["token"],
  insights: ["password"],
  netbox: ["token"],
};

function credentialUpdateBody(form, kind) {
  const clear = [...form.querySelectorAll("[data-clear]:checked")].map((node) => node.getAttribute("data-clear"));
  if (kind === "env") {
    const secret = {};
    form.env.value.split("\n").forEach((line) => {
      const trimmed = line.trim();
      if (!trimmed) return;
      const eq = trimmed.indexOf("=");
      if (eq < 1) throw new Error("an_env_line_needs_key_value");
      secret[trimmed.slice(0, eq)] = trimmed.slice(eq + 1);
    });
    return { secret, clear };
  }
  const secret = {};
  const hidden = new Set(SECRET_FIELD_NAMES[kind] || []);
  const names = {
    ssh: ["username", "password", "private_key", "passphrase", "become_method", "become_username", "become_password"],
    vault: ["password", "vault_id"],
    scm: ["username", "password", "private_key", "passphrase"],
    net: ["username", "password", "private_key", "authorize_password"],
    aws: ["access_key", "secret_key", "security_token"],
    azure: ["subscription_id", "client_id", "secret", "tenant"],
    gce: ["service_account"],
    vmware: ["host", "username", "password"],
    openstack: ["auth_url", "username", "password", "project", "domain"],
    registry: ["host", "username", "password"],
    github: ["token"],
    gitlab: ["token"],
    galaxy: ["url", "token"],
    kubernetes: ["host", "token"],
    insights: ["username", "password"],
    netbox: ["url", "token"],
  };
  (names[kind] || []).forEach((name) => {
    const field = form.elements[name];
    if (!field || !("value" in field)) return;
    const value = field.value.trim();
    if (hidden.has(name)) {
      if (field.dataset.storedMask !== "1" && value) secret[name] = value;
      return;
    }
    secret[name] = value;
  });
  if (kind === "net" && form.authorize) secret.authorize = Boolean(form.authorize.checked);
  if (kind === "kubernetes" && form.verify_ssl) secret.verify_ssl = Boolean(form.verify_ssl.checked);
  return { secret, clear };
}

function credentialEditForm(item) {
  return `<form class="card form" id="credential-form" data-kind="${esc(item.kind)}">
    <label class="field"><span>${h("name")}</span><input name="name" required value="${esc(item.name)}"></label>
    <label class="field"><span>${h("type")}</span><input value="${esc(credentialKindLabel(item.kind))}" disabled></label>
    <p class="small muted">${h("the_type_cannot_be_changed")}</p>
    ${credentialFields(item.kind, item)}
    <button class="btn pill primary" type="submit">${h("save")}</button>
  </form>`;
}

async function pageCredential(id) {
  if (!needTeam()) return;
  const item = await api("GET", `/api/credentials/${encodeURIComponent(id)}`);
  if (!can("owner")) {
    const visible = Object.entries(item.fields || {}).map(([key, value]) => {
      const shown = typeof value === "boolean" ? t(value ? "yes" : "no") : String(value);
      return paramRow(fieldLabel(key), shown);
    }).join("");
    const stored = (item.set || []).map((key) => paramRowHtml(fieldLabel(key), `<input type="password" value="********" disabled tabindex="-1">`)).join("");
    shell(item.name, `<section class="card"><dl class="kv">${paramRow("name", item.name)}${paramRow("type", credentialKindLabel(item.kind))}${visible}${stored}</dl></section>`);
    return;
  }
  shell(item.name, credentialEditForm(item), `<button class="btn danger" id="delete-credential" type="button">${h("delete")}</button>`);
  const form = document.querySelector("#credential-form");
  form.querySelectorAll("[data-stored-mask]").forEach((field) => {
    field.addEventListener("focus", () => {
      if (field.dataset.storedMask === "1") field.select();
    });
    field.addEventListener("input", () => {
      delete field.dataset.storedMask;
    });
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    try {
      const body = credentialUpdateBody(form, item.kind);
      await api("PUT", `/api/credentials/${encodeURIComponent(id)}`, {
        name: form.name.value,
        secret: body.secret,
        clear: body.clear,
      });
      go("#/credentials");
    } catch (error) { handle(error); }
  });
  document.querySelector("#delete-credential").addEventListener("click", async () => {
    try {
      await api("DELETE", `/api/credentials/${encodeURIComponent(id)}`);
      go("#/credentials");
    } catch (error) { handle(error); }
  });
}

async function pageCredentialNew() {
  if (!needTeam()) return;
  if (!can("owner")) {
    deny("nav.access", "owner");
    return;
  }
  const kindOptions = CREDENTIAL_KINDS.map(([id, label]) => `<option value="${esc(id)}">${h(label)}</option>`).join("");
  shell(t("new_credential"), `
    <form class="card form" id="credential-form">
      <label class="field"><span>${h("name")}</span><input name="name" required></label>
      <div class="field-row">
        <label class="field"><span>${h("type")}</span><select class="fit" name="kind">${kindOptions}</select></label>
      </div>
      <div id="credential-fields"></div>
      <button class="btn pill primary" type="submit">${h("create")}</button>
    </form>`);
  const form = document.querySelector("#credential-form");
  bindCredentialKind(form);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    try {
      await api("POST", "/api/credentials", {
        name: form.name.value,
        kind: form.kind.value,
        secret: credentialSecret(form),
      });
      go("#/credentials");
    } catch (error) { handle(error); }
  });
}

async function pageOrganizations() {
  const items = await api("GET", "/api/organizations");
  shell(t("organizations"), `
    <section class="card">
      <input class="search" id="organization-search" placeholder="${h("search_organizations")}" autocomplete="off">
      <div id="organization-table"></div>
    </section>`,
    me.is_admin ? addLink("#/organizations/new", "organization_2") : "");
  const drawOrganizations = () => {
    const needle = document.querySelector("#organization-search").value.trim().toLowerCase();
    const rows = items.filter((item) => !needle || `${item.name} ${item.description || ""}`.toLowerCase().includes(needle)).map((item) =>
      `<tr><td><a href="#/organizations/${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(item.description || "")}</td></tr>`
    );
    document.querySelector("#organization-table").innerHTML = table([t("name"), t("description")], rows);
  };
  document.querySelector("#organization-search").addEventListener("input", drawOrganizations);
  drawOrganizations();
}

async function pageOrganizationNew() {
  if (!me.is_admin) {
    deny("organization", "admin");
    return;
  }
  shell(t("new_organization"), `
    <form class="card form" id="org-form">
      <label class="field"><span>${h("name")}</span><input name="name" required></label>
      <label class="field"><span>${h("description")}</span><input name="description"></label>
      <button class="btn pill primary" type="submit">${h("create")}</button>
    </form>`);
  document.querySelector("#org-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      await api("POST", "/api/organizations", { name: form.name.value, description: form.description.value });
      go("#/organizations");
    } catch (error) { handle(error); }
  });
}

async function pageOrganization(id) {
  const item = await api("GET", `/api/organizations/${encodeURIComponent(id)}`);
  const rows = (item.teams || []).map((team) =>
    `<tr><td>${esc(team.name)}</td><td>${esc(team.description || "")}</td><td class="small muted">${esc(team.ldap_group || "")}</td></tr>`
  );
  shell(item.name, `
    <section class="card" style="margin-bottom:14px">
      ${me.is_admin ? `<form class="form" id="org-edit">
        <label class="field"><span>${h("name")}</span><input name="name" value="${esc(item.name)}" required></label>
        <label class="field"><span>${h("description")}</span><input name="description" value="${esc(item.description || "")}"></label>
        <button class="btn pill primary" type="submit">${h("save")}</button>
      </form>` : `<p class="muted">${esc(item.description || "")}</p>`}
    </section>
    <section class="card">
      <h2>${h("teams")}</h2>
      ${table([t("name"), t("description"), t("ldap_group")], rows)}
    </section>`);
  const form = document.querySelector("#org-edit");
  if (form) {
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        await api("PUT", `/api/organizations/${encodeURIComponent(id)}`, {
          name: form.name.value,
          description: form.description.value,
        });
        me = await api("GET", "/api/me");
        route();
      } catch (error) { handle(error); }
    });
  }
}

async function pageTeam() {
  const teams = me.teams || [];
  const current = teams.find((team) => team.id === teamId);
  const organizations = me.is_admin ? await api("GET", "/api/organizations") : [];
  let members = [];
  if (teamId) members = await api("GET", `/api/teams/${encodeURIComponent(teamId)}/members`);
  const memberRows = members.map((member) => {
    const name = me.is_admin
      ? `<a href="#/users/${esc(member.user_id)}">${esc(member.display_name)}</a>`
      : esc(member.display_name);
    const roleCell = member.source === "manual" && can("owner")
      ? `<select data-member-role="${esc(member.username)}" data-current="${esc(member.role)}">${roleOptions(member.role)}</select>`
      : esc(roleLabel(member.role));
    const sourceCell = member.source === "ldap"
      ? `${esc(sourceLabel(member.source))} <span class="small muted">${h("comes_from_the_directory")}</span>`
      : esc(sourceLabel(member.source));
    const remove = can("owner")
      ? `<button class="btn danger" type="button" data-member="${esc(member.user_id)}">${h("remove")}</button>`
      : "";
    return `<tr><td>${name} <span class="small muted">${esc(member.username)}</span></td><td>${roleCell}</td><td>${sourceCell}</td><td>${remove}</td></tr>`;
  });
  const actions = [
    teamId && can("owner") ? addLink("#/team/members/new", "member") : "",
    me.is_admin ? addLink("#/team/new", "team_2") : "",
    me.is_admin ? addLink("#/users/new", "user_2") : "",
  ].join("");
  shell(t("team"), `
    ${current ? `<section class="card" style="margin-bottom:14px">
      <h2>${esc(current.name)}</h2>
      <p class="muted">${esc(current.description || "")}</p>
      <p class="small">${h("organization_name_2", { name: current.organization_name || "—" })}</p>
      ${can("owner") ? `<form class="form" id="team-form">
        <label class="field"><span>${h("name")}</span><input name="name" value="${esc(current.name)}" required></label>
        <label class="field"><span>${h("description")}</span><input name="description" value="${esc(current.description || "")}"></label>
        <label class="field"><span>${h("ldap_or_ad_group")}</span><input name="ldap_group" value="${esc(current.ldap_group || "")}" placeholder="${h("cn_or_dn_for_example_platform")}"></label>
        ${me.is_admin ? `<label class="field"><span>${h("move_organization")}</span><select name="organization_id">${orgOptions(organizations, current.organization_id)}</select></label>` : ""}
        <button class="btn pill primary" type="submit">${h("save_team")}</button>
      </form>` : `<p class="small">${h("ldap_group_group", { group: current.ldap_group || "—" })}</p>`}
    </section>
    <section class="card">
      <h2>${h("members")}</h2>
      ${roleLegend()}
      ${table([t("name"), t("role"), t("source"), ""], memberRows)}
    </section>` : `<div class="card">${h("no_team_selected")}</div>`}`, actions);
  const teamForm = document.querySelector("#team-form");
  if (teamForm) {
    teamForm.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        const payload = {
          name: teamForm.name.value,
          description: teamForm.description.value,
          ldap_group: teamForm.ldap_group.value,
        };
        if (teamForm.organization_id) payload.organization_id = teamForm.organization_id.value;
        await api("PUT", `/api/teams/${encodeURIComponent(teamId)}`, payload);
        me = await api("GET", "/api/me");
        route();
      } catch (error) { handle(error); }
    });
  }
  document.querySelectorAll("[data-member-role]").forEach((select) => {
    select.addEventListener("change", async () => {
      const next = select.value;
      try {
        await api("POST", `/api/teams/${encodeURIComponent(teamId)}/members`, {
          username: select.dataset.memberRole,
          role: next,
        });
        select.dataset.current = next;
      } catch (error) {
        select.value = select.dataset.current;
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-member]").forEach((button) => {
    button.addEventListener("click", async () => {
      try {
        await api("DELETE", `/api/teams/${encodeURIComponent(teamId)}/members/${encodeURIComponent(button.dataset.member)}`);
        route();
      } catch (error) { handle(error); }
    });
  });
}

async function pageMemberNew() {
  if (!teamId) {
    shell(t("team"), `<div class="card">${h("no_team_selected")}</div>`);
    return;
  }
  if (!can("owner")) {
    deny("members", "owner");
    return;
  }
  shell(t("add_member"), `
    <form class="card form" id="member-form">
      <label class="field"><span>${h("username")}</span><input name="username" required></label>
      <div class="field-row">
        <label class="field"><span>${h("role")}</span>
          <select class="fit" name="role"><option value="viewer">${h("viewer")}</option><option value="operator" selected>${h("operator")}</option><option value="owner">${h("owner")}</option></select>
        </label>
      </div>
      <button class="btn pill primary" type="submit">${h("add")}</button>
    </form>`);
  document.querySelector("#member-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      await api("POST", `/api/teams/${encodeURIComponent(teamId)}/members`, {
        username: form.username.value,
        role: form.role.value,
      });
      go("#/team");
    } catch (error) { handle(error); }
  });
}

async function pageTeamNew() {
  if (!me.is_admin) {
    deny("team", "admin");
    return;
  }
  const organizations = await api("GET", "/api/organizations");
  const current = (me.teams || []).find((team) => team.id === teamId);
  shell(t("new_team_2"), `
    <form class="card form" id="create-team">
      <label class="field"><span>${h("name")}</span><input name="name" required></label>
      <label class="field"><span>${h("description")}</span><input name="description"></label>
      <label class="field"><span>${h("organization")}</span><select name="organization_id"><option value="">${h("new_organization_named_after_the_team")}</option>${orgOptions(organizations, current && current.organization_id)}</select></label>
      <label class="field"><span>${h("ldap_group")}</span><input name="ldap_group"></label>
      <button class="btn pill primary" type="submit">${h("create")}</button>
    </form>`);
  document.querySelector("#create-team").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const payload = {
        name: form.name.value,
        description: form.description.value,
        ldap_group: form.ldap_group.value,
      };
      if (form.organization_id && form.organization_id.value) payload.organization_id = form.organization_id.value;
      const created = await api("POST", "/api/teams", payload);
      me = await api("GET", "/api/me");
      teamId = created.id;
      localStorage.setItem("koreograf.team", teamId);
      go("#/team");
    } catch (error) { handle(error); }
  });
}

function emailLine(email) {
  return email ? esc(email) : `<span class="muted">${h("no_email_is_stored")}</span>`;
}

async function pageAccount() {
  me = await api("GET", "/api/me");
  const rows = (me.memberships || []).map((row) => {
    const org = row.organization_name || "—";
    return `<tr><td>${esc(org)}</td><td>${esc(row.team_name)}</td><td>${esc(roleLabel(row.role))}</td><td>${esc(sourceLabel(row.source))}</td></tr>`;
  });
  const local = me.auth_source === "local";
  const identity = local ? `
    <form class="card form" id="account-form">
      <h2>${h("account")}</h2>
      <p class="small muted">${h("username")}: ${esc(me.username)} · ${esc(authSourceLabel(me.auth_source))}${me.is_admin ? ` · ${h("administrator")}` : ""}</p>
      <label class="field"><span>${h("display_name")}</span><input name="display_name" value="${esc(me.display_name)}" required></label>
      <label class="field"><span>${h("email")}</span><input name="email" type="email" value="${esc(me.email || "")}" autocomplete="email"></label>
      <label class="field"><span>${h("new_password")}</span><input name="password" type="password" minlength="8" autocomplete="new-password" placeholder="${h("leave_empty_to_keep_the_password")}"></label>
      <button class="btn pill primary" type="submit">${h("save")}</button>
    </form>` : `
    <section class="card">
      <h2>${h("account")}</h2>
      <p class="small muted">${h("username")}: ${esc(me.username)} · ${esc(authSourceLabel(me.auth_source))}${me.is_admin ? ` · ${h("administrator")}` : ""}</p>
      <p>${h("display_name")}: ${esc(me.display_name)}</p>
      <p>${h("email")}: ${emailLine(me.email)}</p>
      <p class="muted">${h("name_and_email_come_from_the_directory")}</p>
    </section>`;
  shell(t("my_account"), `
    <div class="stack">
      ${identity}
      <section class="card">
        <h2>${h("memberships")}</h2>
        ${table([t("organization"), t("team"), t("role"), t("source")], rows)}
      </section>
    </div>`);
  const form = document.querySelector("#account-form");
  if (form) {
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        const payload = {
          display_name: form.display_name.value,
          email: form.email.value,
        };
        if (form.password.value) payload.password = form.password.value;
        me = await api("PUT", "/api/me", payload);
        route();
      } catch (error) { handle(error); }
    });
  }
}

async function pageUsers() {
  if (!me.is_admin) {
    deny("users", "admin");
    return;
  }
  const items = await api("GET", "/api/users");
  shell(t("users"), `
    <section class="card">
      <input class="search" id="user-search" placeholder="${h("search_users")}" autocomplete="off">
      <div id="user-table"></div>
    </section>`,
    addLink("#/users/new", "user_2"));
  const draw = () => {
    const needle = document.querySelector("#user-search").value.trim().toLowerCase();
    const shown = items.filter((item) => {
      if (!needle) return true;
      return `${item.display_name} ${item.username} ${item.email || ""} ${item.auth_source}`.toLowerCase().includes(needle);
    });
    const rows = shown.map((item) => {
      const flags = [
        item.is_admin ? `<span class="pill">${h("administrator")}</span>` : "",
        item.disabled ? `<span class="pill failed">${h("disabled")}</span>` : "",
      ].filter(Boolean).join(" ");
      const when = String(item.created_at || "").slice(0, 10);
      return `<tr>
        <td><a href="#/users/${esc(item.id)}">${esc(item.display_name)}</a> ${flags}</td>
        <td>${esc(item.username)}</td>
        <td>${esc(authSourceLabel(item.auth_source))}</td>
        <td>${esc(item.team_count)}</td>
        <td class="small muted">${esc(when)}</td>
      </tr>`;
    });
    document.querySelector("#user-table").innerHTML = table(
      [t("name"), t("username"), t("source"), t("teams"), t("created")],
      rows
    );
  };
  document.querySelector("#user-search").addEventListener("input", draw);
  draw();
}

async function pageUser(id) {
  if (!me.is_admin) {
    deny("users", "admin");
    return;
  }
  const item = await api("GET", `/api/users/${encodeURIComponent(id)}`);
  const teams = await api("GET", "/api/teams");
  const memberIds = new Set((item.memberships || []).map((row) => row.team_id));
  const openTeams = teams.filter((team) => !memberIds.has(team.id));
  const self = item.id === me.id;
  const rows = (item.memberships || []).map((row) => {
    const org = row.organization_name ? `${row.organization_name} · ` : "";
    const roleCell = row.source === "manual"
      ? `<select data-role="${esc(row.team_id)}" data-current="${esc(row.role)}">${roleOptions(row.role)}</select>`
      : `${esc(roleLabel(row.role))} <span class="small muted">${h("comes_from_the_directory")}</span>`;
    const remove = row.source === "manual"
      ? `<button class="btn danger" type="button" data-leave="${esc(row.team_id)}">${h("remove")}</button>`
      : "";
    return `<tr><td>${esc(org + row.team_name)}</td><td>${roleCell}</td><td>${esc(sourceLabel(row.source))}</td><td>${remove}</td></tr>`;
  });
  const passwordField = item.auth_source === "local"
    ? `<label class="field"><span>${h("new_password")}</span><input name="password" type="password" minlength="8" autocomplete="new-password" placeholder="${h("leave_empty_to_keep_the_password")}"></label>`
    : `<p class="small muted">${h("the_password_comes_from_ldap_or_active_directory")}</p>`;
  const teamChoices = openTeams.map((team) => {
    const label = team.organization_name ? `${team.organization_name} · ${team.name}` : team.name;
    return `<option value="${esc(team.id)}">${esc(label)}</option>`;
  }).join("");
  shell(item.display_name, `
    <div class="stack">
      <form class="card form" id="user-form">
        <h2>${h("account")}</h2>
        <p class="small muted">${h("username")}: ${esc(item.username)} · ${esc(authSourceLabel(item.auth_source))}</p>
        <p class="small">${h("email")}: ${emailLine(item.email)}</p>
        <label class="field"><span>${h("display_name")}</span><input name="display_name" value="${esc(item.display_name)}" required></label>
        <label class="row"><input name="is_admin" type="checkbox"${item.is_admin ? " checked" : ""}> ${h("administrator")}</label>
        <label class="row"><input name="account_disabled" type="checkbox"${item.disabled ? " checked" : ""}${self ? " disabled" : ""}> ${h("disable_account")}</label>
        ${passwordField}
        <div class="row">
          <button class="btn pill primary" type="submit">${h("save")}</button>
          ${self ? "" : `<button class="btn danger" id="delete-user" type="button">${h("delete")}</button>`}
        </div>
      </form>
      <section class="card">
        <h2>${h("permissions")}</h2>
        ${roleLegend()}
      </section>
      <section class="card">
        <h2>${h("memberships")}</h2>
        ${table([t("team"), t("role"), t("source"), ""], rows)}
        ${openTeams.length ? `<form class="form" id="grant-form">
          <h2>${h("add_to_a_team")}</h2>
          <label class="field"><span>${h("team")}</span><select name="team_id">${teamChoices}</select></label>
          <div class="field-row">
            <label class="field"><span>${h("role")}</span><select class="fit" name="role">${roleOptions("operator")}</select></label>
          </div>
          <button class="btn pill primary" type="submit">${h("add")}</button>
        </form>` : `<p class="muted">${h("the_user_is_in_every_team")}</p>`}
      </section>
    </div>`);
  document.querySelector("#user-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const payload = {
        display_name: form.display_name.value,
        is_admin: form.is_admin.checked,
        disabled: form.account_disabled.checked,
      };
      if (form.password && form.password.value) payload.password = form.password.value;
      await api("PUT", `/api/users/${encodeURIComponent(id)}`, payload);
      if (self) me = await api("GET", "/api/me");
      route();
    } catch (error) { handle(error); }
  });
  const deleteButton = document.querySelector("#delete-user");
  if (deleteButton) {
    deleteButton.addEventListener("click", async () => {
      if (!window.confirm(t("delete_this_account"))) return;
      try {
        await api("DELETE", `/api/users/${encodeURIComponent(id)}`);
        go("#/users");
      } catch (error) { handle(error); }
    });
  }
  document.querySelectorAll("[data-role]").forEach((select) => {
    select.addEventListener("change", async () => {
      const next = select.value;
      try {
        await api("POST", `/api/teams/${encodeURIComponent(select.dataset.role)}/members`, {
          username: item.username,
          role: next,
        });
        select.dataset.current = next;
      } catch (error) {
        select.value = select.dataset.current;
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-leave]").forEach((button) => {
    button.addEventListener("click", async () => {
      try {
        await api("DELETE", `/api/teams/${encodeURIComponent(button.dataset.leave)}/members/${encodeURIComponent(item.id)}`);
        route();
      } catch (error) { handle(error); }
    });
  });
  const grant = document.querySelector("#grant-form");
  if (grant) {
    grant.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        await api("POST", `/api/teams/${encodeURIComponent(grant.team_id.value)}/members`, {
          username: item.username,
          role: grant.role.value,
        });
        route();
      } catch (error) { handle(error); }
    });
  }
}

async function pageUserNew() {
  if (!me.is_admin) {
    deny("user", "admin");
    return;
  }
  shell(t("new_user"), `
    <form class="card form" id="create-user">
      <label class="field"><span>${h("username")}</span><input name="username" required autocomplete="off"></label>
      <p class="small muted">${h("username_2_64_characters_a_z_0_9")}</p>
      <label class="field"><span>${h("display_name")}</span><input name="display_name" required></label>
      <label class="field"><span>${h("password")}</span><input name="password" type="password" required minlength="8" autocomplete="new-password"></label>
      <label class="row"><input name="is_admin" type="checkbox"> ${h("administrator")}</label>
      <button class="btn pill primary" type="submit">${h("create")}</button>
    </form>`);
  document.querySelector("#create-user").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const created = await api("POST", "/api/users", {
        username: form.username.value,
        display_name: form.display_name.value,
        password: form.password.value,
        is_admin: form.is_admin.checked,
      });
      go(`#/users/${created.id}`);
    } catch (error) { handle(error); }
  });
}

function optionSelected(current, value) {
  return current === value ? " selected" : "";
}

function settingsBody(form) {
  const secret = (name) => {
    const field = form.elements[name];
    if (!field || field.dataset.storedMask === "1") return "";
    return field.value;
  };
  const clear = [...form.querySelectorAll("[data-clear]:checked")].map((node) => node.getAttribute("data-clear"));
  return {
    ldap: {
      url: form.ldap_url.value.trim(),
      mode: form.ldap_mode.value,
      bind_dn: form.ldap_bind_dn.value.trim(),
      bind_password: secret("ldap_bind_password"),
      user_base: form.ldap_user_base.value.trim(),
      user_filter: form.ldap_user_filter.value.trim(),
      username_attr: form.ldap_username_attr.value.trim(),
      group_attr: form.ldap_group_attr.value.trim(),
      group_base: form.ldap_group_base.value.trim(),
      starttls: form.ldap_starttls.checked,
      tls_insecure: form.ldap_tls_insecure.checked,
      ca_file: form.ldap_ca_file.value.trim(),
    },
    proxy: {
      url: form.proxy_url.value.trim(),
      username: form.proxy_username.value.trim(),
      password: secret("proxy_password"),
      no_proxy: form.proxy_no_proxy.value.trim(),
    },
    runner: form.runner.value,
    job_timeout_secs: Number(form.job_timeout_secs.value),
    clear,
  };
}

function bindStoredMasks(form) {
  form.querySelectorAll("[data-stored-mask]").forEach((field) => {
    field.addEventListener("focus", () => {
      if (field.dataset.storedMask === "1") field.select();
    });
    field.addEventListener("input", () => {
      delete field.dataset.storedMask;
    });
  });
}

async function pageSettings(notice) {
  if (!me.is_admin) {
    deny("settings", "admin");
    return;
  }
  const item = await api("GET", "/api/settings");
  const ldap = item.ldap || {};
  const proxy = item.proxy || {};
  const mode = ldap.mode === "ad" ? "ad" : "ldap";
  const runner = ["auto", "docker", "local"].includes(item.runner) ? item.runner : "auto";
  const source = item.source === "saved"
    ? "these_values_are_saved_and_apply_immediately"
    : "nothing_is_saved_yet_the_environment_variables_from";
  const unsupported = proxy.unsupported_env
    ? `<p class="banner warn">${h("the_environment_uses_a_proxy_that_is_neither")}</p>`
    : "";
  const saved = notice ? `<p class="banner good">${h(notice)}</p>` : "";
  shell(t("settings"), `<form class="card form" id="settings-form">
    ${saved}
    <p class="small muted">${h(source)}</p>
    <p class="small muted">${h("the_bind_address_data_directory_and_first_admin")}</p>
    <section class="form-section">
      <h2>${h("directory")}</h2>
      <p class="small muted">${h("ldap_or_active_directory_an_empty_address_turns")}</p>
      <div class="field-row">
        <label class="field"><span>${h("mode")}</span>
          <select class="fit" name="ldap_mode">
            <option value="ldap"${optionSelected(mode, "ldap")}>LDAP</option>
            <option value="ad"${optionSelected(mode, "ad")}>${h("active_directory")}</option>
          </select>
        </label>
      </div>
      <label class="field"><span>${h("address")}</span><input name="ldap_url" value="${esc(ldap.url || "")}" placeholder="ldap://ldap.example.com" autocomplete="off" spellcheck="false"></label>
      <label class="field"><span>${h("bind_dn")}</span><input name="ldap_bind_dn" value="${esc(ldap.bind_dn || "")}" placeholder="cn=svc,dc=example,dc=com" autocomplete="off" spellcheck="false"></label>
      ${secretInput("ldap_bind_password", "bind_password", "password", Boolean(ldap.bind_password_set))}
      <label class="field"><span>${h("user_base")}</span><input name="ldap_user_base" value="${esc(ldap.user_base || "")}" placeholder="ou=users,dc=example,dc=com" autocomplete="off" spellcheck="false"></label>
      <div class="field-grid">
        <label class="field"><span>${h("user_filter")}</span><input name="ldap_user_filter" value="${esc(ldap.user_filter || "")}" spellcheck="false"></label>
        <label class="field"><span>${h("username_attribute")}</span><input name="ldap_username_attr" value="${esc(ldap.username_attr || "")}" spellcheck="false"></label>
        <label class="field"><span>${h("group_attribute")}</span><input name="ldap_group_attr" value="${esc(ldap.group_attr || "")}" spellcheck="false"></label>
        <label class="field"><span>${h("group_base")}</span><input name="ldap_group_base" value="${esc(ldap.group_base || "")}" autocomplete="off" spellcheck="false"></label>
      </div>
      <label class="row"><input name="ldap_starttls" type="checkbox"${ldap.starttls ? " checked" : ""}> ${h("starttls")}</label>
      <label class="row"><input name="ldap_tls_insecure" type="checkbox"${ldap.tls_insecure ? " checked" : ""}> ${h("skip_certificate_checks_only_on_a_network_you")}</label>
      <label class="field"><span>${h("private_ca_file")}</span><input name="ldap_ca_file" value="${esc(ldap.ca_file || "")}" placeholder="/etc/ssl/certs/internal-ca.pem" autocomplete="off" spellcheck="false"></label>
      <p class="small muted">${h("an_absolute_pem_path_empty_uses_the_system_cas")}</p>
    </section>
    <section class="form-section">
      <h2>${h("proxy")}</h2>
      <p class="small muted">${h("one_address_is_used_for_http_and_https")}</p>
      <p class="small muted">${h("an_empty_address_turns_the_proxy_off")}</p>
      ${unsupported}
      <label class="field"><span>${h("proxy_address")}</span><input name="proxy_url" value="${esc(proxy.url || "")}" placeholder="http://proxy.example:3128" autocomplete="off" spellcheck="false"></label>
      <div class="field-row">
        <label class="field"><span>${h("proxy_user")}</span><input class="fit" name="proxy_username" value="${esc(proxy.username || "")}" autocomplete="off" spellcheck="false"></label>
        ${secretInput("proxy_password", "proxy_password", "password", Boolean(proxy.password_set))}
      </div>
      <label class="field"><span>${h("exceptions")}</span><input name="proxy_no_proxy" value="${esc(proxy.no_proxy || "")}" placeholder="localhost,127.0.0.1,.internal" autocomplete="off" spellcheck="false"></label>
    </section>
    <section class="form-section">
      <h2>${h("jobs")}</h2>
      <p class="small muted">${h("the_mode_and_the_time_limit_apply_to")}</p>
      <div class="field-row">
        <label class="field"><span>${h("runner")}</span>
          <select class="fit" name="runner">
            <option value="auto"${optionSelected(runner, "auto")}>${h("automatic")}</option>
            <option value="docker"${optionSelected(runner, "docker")}>${h("container")}</option>
            <option value="local"${optionSelected(runner, "local")}>${h("local")}</option>
          </select>
        </label>
        <label class="field"><span>${h("time_limit_in_seconds")}</span><input class="tiny" name="job_timeout_secs" type="number" min="30" max="86400" required value="${esc(String(item.job_timeout_secs || 1800))}"></label>
      </div>
      <p class="small muted">${h("automatic_uses_an_execution_environment_when_docker_or")}</p>
    </section>
    <p class="small muted">${h("saving_replaces_the_environment_variables_an_empty_password")}</p>
    <div class="choice-row">
      <button class="btn pill primary" type="submit">${h("save")}</button>
      <button class="btn pill" id="settings-reset" type="button">${h("use_environment_variables")}</button>
    </div>
  </form>`);
  const form = document.querySelector("#settings-form");
  bindStoredMasks(form);
  form.ldap_mode.addEventListener("change", () => {
    const ad = form.ldap_mode.value === "ad";
    const next = ad
      ? { filter: "(sAMAccountName={username})", attr: "sAMAccountName" }
      : { filter: "(uid={username})", attr: "uid" };
    const previous = ad
      ? { filter: "(uid={username})", attr: "uid" }
      : { filter: "(sAMAccountName={username})", attr: "sAMAccountName" };
    if (!form.ldap_user_filter.value.trim() || form.ldap_user_filter.value.trim() === previous.filter) {
      form.ldap_user_filter.value = next.filter;
    }
    if (!form.ldap_username_attr.value.trim() || form.ldap_username_attr.value.trim() === previous.attr) {
      form.ldap_username_attr.value = next.attr;
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    try {
      await api("PUT", "/api/settings", settingsBody(form));
      await pageSettings("settings_saved");
    } catch (error) {
      handle(error);
    }
  });
  document.querySelector("#settings-reset").addEventListener("click", async () => {
    if (!window.confirm(t("discard_the_saved_settings_and_use_the_environment"))) return;
    try {
      await api("PUT", "/api/settings", { reset: true });
      await pageSettings();
    } catch (error) {
      handle(error);
    }
  });
}

async function pageBackup() {
  if (!me.is_admin) {
    deny("backup", "admin");
    return;
  }
  shell(t("backup"), `<div class="stack">
    <div class="card">
      <p>${h("koreograf_stores_data_in_sqlite_the_backup_contains")}</p>
      <p class="muted">${h("postgresql_is_planned_for_a_later_company_install")}</p>
      <div><button class="btn pill primary" id="backup-download" type="button">${h("download_backup")}</button></div>
    </div>
    <form class="card form" id="backup-restore">
      <h2>${h("restore_backup")}</h2>
      <p>${h("this_replaces_organizations_teams_users_projects_inventories_credentials")}</p>
      <label class="field"><span>${h("file")}</span><input name="file" type="file" accept=".zip,application/zip" required></label>
      <label class="row"><input name="confirm" type="checkbox" required> ${h("the_current_installation_will_be_replaced")}</label>
      <button class="btn pill primary" type="submit">${h("restore_backup")}</button>
    </form>
  </div>`);
  document.querySelector("#backup-download").addEventListener("click", async () => {
    const button = document.querySelector("#backup-download");
    button.disabled = true;
    try {
      const headers = {};
      if (teamId) headers["X-Team-Id"] = teamId;
      const response = await fetch("/api/admin/backup", { headers, credentials: "same-origin" });
      const blob = await response.blob();
      if (!response.ok) {
        const text = await blob.text();
        let message = "request.failed";
        let vars = null;
        try {
          const data = JSON.parse(text);
          message = data.error || message;
          vars = data.vars || null;
        } catch { /* keep */ }
        const error = new Error(message);
        error.vars = vars;
        throw error;
      }
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = "koreograf-backup.zip";
      link.click();
      URL.revokeObjectURL(url);
    } catch (error) {
      handle(error);
    } finally {
      button.disabled = false;
    }
  });
  document.querySelector("#backup-restore").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const file = form.file.files[0];
    if (!file || !form.confirm.checked) return;
    const button = form.querySelector("button[type=submit]");
    button.disabled = true;
    try {
      const headers = { "X-Restore-Confirm": "ERSETZEN" };
      if (teamId) headers["X-Team-Id"] = teamId;
      const response = await fetch("/api/admin/backup", {
        method: "POST",
        headers,
        body: file,
        credentials: "same-origin",
      });
      const data = await response.json().catch(() => ({}));
      if (!response.ok) {
        const error = new Error(data.error || "request.failed");
        error.vars = data.vars || null;
        throw error;
      }
      me = null;
      renderLogin("backup_restored_the_previous_installation_is_restore_previous", "good");
    } catch (error) {
      handle(error);
      button.disabled = false;
    }
  });
}

async function route() {
  closeLive();
  const hash = location.hash || "#/";
  if (!me) {
    if (hash !== "#/login") location.hash = "#/login";
    renderLogin("");
    return;
  }
  if (hash === "#/login") {
    location.hash = "#/";
    return;
  }
  const parts = hash.slice(1).split("/").filter(Boolean);
  try {
    if (parts.length === 0) await pageDashboard();
    else if (parts[0] === "templates" && parts[1] === "new") await pageTemplateForm(null);
    else if (parts[0] === "templates" && parts[1]) await pageTemplateForm(parts[1]);
    else if (parts[0] === "templates") await pageTemplates();
    else if (parts[0] === "jobs" && parts[1]) await pageJob(parts[1]);
    else if (parts[0] === "jobs") await pageJobs();
    else if (parts[0] === "workflows" && parts[1] === "new") await pageWorkflowNew();
    else if (parts[0] === "workflows" && parts[1]) await pageWorkflow(parts[1]);
    else if (parts[0] === "workflows") await pageWorkflows();
    else if (parts[0] === "schedules") await pageSchedules();
    else if (parts[0] === "runs" && parts[1]) await pageRun(parts[1]);
    else if (parts[0] === "hooks" && parts[1] === "new") await pageHookNew();
    else if (parts[0] === "hooks") await pageHooks();
    else if (parts[0] === "environments" && parts[1] === "new") await pageEnvironmentNew();
    else if (parts[0] === "environments" && parts[1]) await pageEnvironment(parts[1]);
    else if (parts[0] === "runtime") await pageRuntime();
    else if (parts[0] === "projects" && parts[1] === "new") await pageProjectNew();
    else if (parts[0] === "projects" && parts[1]) await pageProject(parts[1]);
    else if (parts[0] === "projects") await pageProjects();
    else if (parts[0] === "inventories" && parts[1] === "new") await pageInventoryNew();
    else if (parts[0] === "inventories" && parts[1]) await pageInventory(parts[1]);
    else if (parts[0] === "inventories") await pageInventories();
    else if (parts[0] === "credentials" && parts[1] === "new") await pageCredentialNew();
    else if (parts[0] === "credentials" && parts[1]) await pageCredential(parts[1]);
    else if (parts[0] === "credentials") await pageCredentials();
    else if (parts[0] === "organizations" && parts[1] === "new") await pageOrganizationNew();
    else if (parts[0] === "organizations" && parts[1]) await pageOrganization(parts[1]);
    else if (parts[0] === "organizations") await pageOrganizations();
    else if (parts[0] === "catalog") location.hash = "#/projects";
    else if (parts[0] === "team" && parts[1] === "members" && parts[2] === "new") await pageMemberNew();
    else if (parts[0] === "team" && parts[1] === "new") await pageTeamNew();
    else if (parts[0] === "account") await pageAccount();
    else if (parts[0] === "users" && parts[1] === "new") await pageUserNew();
    else if (parts[0] === "users" && parts[1]) await pageUser(parts[1]);
    else if (parts[0] === "users") await pageUsers();
    else if (parts[0] === "settings") await pageSettings();
    else if (parts[0] === "backup") await pageBackup();
    else if (parts[0] === "team") await pageTeam();
    else shell(t("page"), `<div class="card">${h("this_page_does_not_exist")}</div>`);
  } catch (error) {
    handle(error);
  }
}

async function boot() {
  try {
    const [german, english] = await Promise.all([
      fetch("/i18n/de.json").then((response) => response.json()),
      fetch("/i18n/en.json").then((response) => response.json()),
    ]);
    messages = { de: german, en: english };
  } catch {
    messages = { de: {}, en: {} };
  }
  try {
    me = await api("GET", "/api/me");
    const known = (me.teams || []).some((team) => team.id === teamId);
    if (!known) {
      teamId = (me.teams[0] && me.teams[0].id) || "";
      localStorage.setItem("koreograf.team", teamId);
    }
  } catch {
    me = null;
  }
  window.addEventListener("hashchange", route);
  route();
}

boot();
