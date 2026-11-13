import { esc, h } from "./dom.js";
import { currentLanguage, t } from "./i18n.js";

export function statusLabel(status) {
  return {
    pending: t("pending"),
    running: t("running"),
    successful: t("successful"),
    failed: t("failed"),
    canceled: t("canceled"),
    skipped: t("skipped"),
    accepted: t("accepted"),
    ignored: t("ignored"),
    rejected: t("rejected"),
  }[status] || status;
}

export function pill(status) {
  return `<span class="pill ${esc(status)}">${esc(statusLabel(status))}</span>`;
}

export function when(value) {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleString(currentLanguage() === "en" ? "en-GB" : "de-DE", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
    timeZoneName: "short",
  });
}

export function relativeTime(value) {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const secs = Math.max(0, Math.round((Date.now() - date.getTime()) / 1000));
  if (secs < 60) return t("just_now");
  const mins = Math.floor(secs / 60);
  if (mins < 60) return mins === 1 ? t("minute_ago") : t("minutes_ago", { count: mins });
  const hours = Math.floor(mins / 60);
  if (hours < 24) return hours === 1 ? t("hour_ago") : t("hours_ago", { count: hours });
  const days = Math.floor(hours / 24);
  return days === 1 ? t("day_ago") : t("days_ago", { count: days });
}

export function spanText(start, end, phase) {
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

export function environmentToolLabel(tool) {
  return !tool || tool === "any" ? t("any_tool_legacy") : t(tool);
}

export function verbosityLabel(level) {
  return [
    "0_normal",
    "1_verbose",
    "2_more_verbose",
    "3_debug",
    "4_connection_debug",
  ][Number(level)] || "";
}

export function sourceLabel(source) {
  if (source === "ldap") return "LDAP";
  if (source === "manual") return t("manual");
  if (source === "local") return t("local");
  return source || "";
}

export function authSourceLabel(source) {
  return sourceLabel(source === "local" ? "local" : source);
}

export function emailLine(email) {
  return email ? esc(email) : `<span class="muted">${h("no_email_is_stored")}</span>`;
}

export function bytes(value) {
  const n = Number(value) || 0;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / 1024 / 1024).toFixed(1)} MiB`;
}

export function preflightNotices(report) {
  if (!report || report.ok) return [];
  const missing = report.missing || [];
  const runtimeMissing = missing.some((item) => item.kind === "runtime");
  return missing.map((item) => {
    if (item.kind === "runtime") {
      const reason = item.reason === "timeout" || item.reason === "not_answering" ? item.reason : "missing";
      return { key: `preflight_runtime_${reason}` };
    }
    if (item.kind === "project") {
      return {
        key: item.reason === "timeout" || item.reason === "failed" ? "preflight_project_unusable" : "preflight_project_missing",
        tool: item.tool,
      };
    }
    if (item.reason === "version") {
      return {
        key: "preflight_tool_too_old",
        tool: item.tool,
        template_tool: item.template_tool,
        version: item.version || "",
        minimum: item.minimum || "",
      };
    }
    if (item.reason === "timeout" || item.reason === "failed") {
      return { key: "preflight_tool_unusable", tool: item.tool, template_tool: item.template_tool };
    }
    return {
      key: runtimeMissing ? "preflight_no_runtime_and_tool_missing" : "preflight_tool_missing",
      tool: item.tool,
      template_tool: item.template_tool,
    };
  });
}

export function preflightNoticeHtml(report) {
  const notices = preflightNotices(report);
  if (!notices.length) return "";
  const lines = notices.map((item) => {
    const template = item.template_tool || "";
    const vars = {
      ...item,
      templates: template ? t(template) : "",
      install: item.tool === "sh"
        ? t("preflight_install_shell")
        : item.tool === "ssh"
          ? t("preflight_install_ssh")
          : item.kind === "project"
            ? t("preflight_install_git")
            : (template ? t(template) : item.tool),
    };
    return `<p>${h(item.key, vars)}</p>`;
  }).join("");
  return `<div class="banner warn">${lines}</div>`;
}

export function preflightReportHtml(report) {
  return preflightNoticeHtml(report) || `<div class="banner good">${h("preflight_ready")}</div>`;
}

export function scopeLabel(item) {
  if (!item || item.scope === "global") return t("global");
  if (item.scope === "organization") return t("organization_name", { name: item.organization_name || "" });
  return t("team_name", { name: item.team_name || "" });
}
