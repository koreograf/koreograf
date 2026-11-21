"""Koreograf Ansible event callback.

Koreograf writes this file into the job work directory, adds that directory to
``ANSIBLE_CALLBACK_PLUGINS`` and enables ``koreograf_events`` in
``ANSIBLE_CALLBACKS_ENABLED``. The default stdout callback stays in place; this
plugin only writes one JSON object per line to ``ANSIBLE_KOREOGRAF_EVENTS``.

A result is never written when Ansible censored it for ``no_log``. Every text
field is bounded so a single huge diff cannot flood the sidecar. The runner
bounds the whole stream again before it reaches the database.
"""

from __future__ import annotations

import json
import os
import re
import time

from ansible.plugins.callback import CallbackBase

CALLBACK_VERSION = 2.0
CALLBACK_TYPE = "notification"
CALLBACK_NAME = "koreograf_events"
# Load automatically from the callback path. A whitelist entry would override a
# project's own callbacks_enabled value in ansible.cfg.
CALLBACK_NEEDS_ENABLED = False

MAX_FIELD = 4000
# A stats chunk stays well below the runner's 64 KiB line bound. The runner
# reserves room for every chunk after the per-task event budget is spent.
STATS_LINE = 24000
STATS_HOSTS = 250
STATS_FIELDS = ("ok", "changed", "unreachable", "failed", "skipped", "rescued", "ignored")
_ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
_OSC = re.compile(r"\x1b\][^\x1b\x07]*(?:\x07|\x1b\\)")
_SCRUB_VARS = (
    "KOREOGRAF_WORK",
    "KOREOGRAF_PROJECT",
    "KOREOGRAF_COLLECTIONS",
    "KOREOGRAF_ROLES",
    "KOREOGRAF_TOFU",
)


def _clip(text):
    if len(text) <= MAX_FIELD:
        return text
    return text[: MAX_FIELD - 1] + "\u2026"


def _scrub(text):
    for name in _SCRUB_VARS:
        value = os.environ.get(name, "")
        if value:
            text = text.replace(value, "[work]")
    return _clip(text)


def _text(value):
    if value is None:
        return ""
    if isinstance(value, str):
        text = value
    else:
        try:
            text = json.dumps(value, ensure_ascii=False, sort_keys=True)
        except (TypeError, ValueError):
            text = str(value)
    return _scrub(_OSC.sub("", _ANSI.sub("", text)))


def _name(getter, fallback=""):
    try:
        value = getter()
    except Exception:
        return fallback
    return str(value) if value else fallback


class CallbackModule(CallbackBase):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._path = os.environ.get("ANSIBLE_KOREOGRAF_EVENTS", "")
        self._play = ""
        self._play_index = 0
        self._task_index = 0
        self._task_key = ""
        self._tasks = {}
        self._starts = {}

    def _emit(self, event):
        if not self._path:
            return
        try:
            line = json.dumps(event, ensure_ascii=False, separators=(",", ":"))
            with open(self._path, "a", encoding="utf-8") as handle:
                handle.write(line)
                handle.write("\n")
        except (OSError, TypeError, ValueError):
            # A broken sidecar must never stop the playbook or the stdout log.
            pass

    def v2_playbook_on_play_start(self, play):
        self._play_index += 1
        self._play = _name(play.get_name)
        self._emit(
            {
                "type": "play_start",
                "play": self._play,
                "index": self._play_index,
            }
        )

    def _task_start(self, task, handler=False):
        self._task_index += 1
        key = _name(lambda: task._uuid, f"{self._play_index}:{self._task_index}")
        role = ""
        try:
            role = _name(task._role.get_name)
        except (AttributeError, TypeError):
            role = ""
        name = _name(task.get_name, key)
        entry = {
            "type": "task_start",
            "play": self._play,
            "play_index": self._play_index,
            "index": self._task_index,
            "task": name,
            "role": role,
            "handler": handler,
        }
        self._tasks[key] = entry
        self._task_key = key
        self._emit(entry)

    def v2_playbook_on_task_start(self, task, is_conditional):
        self._task_start(task, False)

    def v2_playbook_on_handler_task_start(self, task):
        self._task_start(task, True)

    def v2_runner_on_start(self, host, task):
        key = _name(lambda: task._uuid, self._task_key)
        name = _name(host.get_name)
        self._starts[(key, name)] = time.monotonic()

    def _result(self, result, status, ignored=False):
        task = getattr(result, "_task", None)
        host = getattr(result, "_host", None)
        key = _name(lambda: task._uuid, self._task_key)
        host_name = _name(host.get_name) if host is not None else ""
        entry = self._tasks.get(key, {})
        data = getattr(result, "_result", None) or {}
        if not isinstance(data, dict):
            data = {}
        if status == "ok" and data.get("changed") is True:
            status = "changed"
        started = self._starts.pop((key, host_name), None)
        event = {
            "type": "host_result",
            "play": entry.get("play", self._play),
            "play_index": entry.get("play_index", self._play_index),
            "index": entry.get("index", self._task_index),
            "task": entry.get("task", ""),
            "role": entry.get("role", ""),
            "host": host_name,
            "status": status,
        }
        if started is not None:
            event["duration"] = round(max(0.0, time.monotonic() - started), 3)
        if ignored:
            event["ignored"] = True
        if data.get("_ansible_no_log") or "censored" in data:
            event["censored"] = True
            self._emit(event)
            return
        changed = data.get("changed")
        if isinstance(changed, bool):
            event["changed"] = changed
        for field in ("msg", "stdout", "stderr"):
            value = data.get(field)
            if isinstance(value, str) and value:
                event[field] = _text(value)
        if data.get("diff"):
            event["diff"] = _text(data["diff"])
        rc = data.get("rc")
        if isinstance(rc, int) and not isinstance(rc, bool):
            event["rc"] = rc
        self._emit(event)

    def v2_runner_on_ok(self, result):
        self._result(result, "ok")

    def v2_runner_on_failed(self, result, ignore_errors=False):
        self._result(result, "failed", bool(ignore_errors))

    def v2_runner_on_skipped(self, result):
        self._result(result, "skipped")

    def v2_runner_on_unreachable(self, result):
        self._result(result, "unreachable")

    def _emit_stats(self, rows):
        """One total plus compact per-host recap chunks.

        The runner caps the per-task stream at 2000 events, so the final recap
        would otherwise lose hosts that the play only reached late. Chunks are
        bounded by serialized size so a long host name cannot push one line
        over the runner's line limit.
        """
        chunks = []
        current = []
        for row in rows:
            current.append(row)
            try:
                size = len(json.dumps(current, ensure_ascii=False, separators=(",", ":")))
            except (TypeError, ValueError):
                size = STATS_LINE + 1
            if len(current) > 1 and (size > STATS_LINE or len(current) >= STATS_HOSTS):
                chunks.append(current[:-1])
                current = [row]
        chunks.append(current)
        total = len(rows)
        parts = len(chunks)
        for number, chunk in enumerate(chunks, 1):
            self._emit(
                {
                    "type": "stats",
                    "total": total,
                    "part": number,
                    "parts": parts,
                    "hosts": chunk,
                }
            )

    def v2_playbook_on_stats(self, stats):
        try:
            rows = []
            for host in sorted(stats.processed):
                summary = stats.summarize(host)
                row = {"host": str(host)}
                for field in STATS_FIELDS:
                    key = "failures" if field == "failed" else field
                    value = summary.get(key, 0)
                    row[field] = max(0, int(value)) if isinstance(value, int) else 0
                rows.append(row)
            self._emit_stats(rows)
        except Exception:
            # A recap failure must never stop the playbook or the raw log.
            pass
        self._emit({"type": "playbook_end"})
