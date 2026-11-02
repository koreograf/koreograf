#!/usr/bin/env python3
"""One job per tool on a fresh install with the seeded execution environments."""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.request
from http.cookiejar import CookieJar

BASE = "http://127.0.0.1:18085"
TERMINAL = {"successful", "failed", "canceled"}


class Api:
    def __init__(self) -> None:
        self.jar = CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar)
        )
        self.team = ""

    def call(self, method: str, path: str, payload: dict | None = None, ok=(200,)):
        data = None
        headers = {"Accept": "application/json"}
        if self.team:
            headers["x-team-id"] = self.team
        if payload is not None:
            data = json.dumps(payload).encode("utf-8")
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(
            BASE + path, data=data, headers=headers, method=method
        )
        try:
            with self.opener.open(request, timeout=60) as response:
                raw = response.read()
                status = response.status
                text = raw.decode("utf-8", errors="replace")
        except urllib.error.HTTPError as err:
            text = err.read().decode("utf-8", errors="replace")
            sys.exit(f"{method} {path} returned {err.code}: {text[:500]}")
        if status not in ok:
            sys.exit(f"{method} {path} returned {status}: {text[:500]}")
        if not text:
            return {}
        return json.loads(text)

    def text(self, path: str) -> str:
        headers = {}
        if self.team:
            headers["x-team-id"] = self.team
        request = urllib.request.Request(BASE + path, headers=headers, method="GET")
        with self.opener.open(request, timeout=60) as response:
            return response.read().decode("utf-8", errors="replace")


def wait_health() -> None:
    deadline = time.time() + 180
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(BASE + "/healthz", timeout=5) as response:
                if response.status == 200:
                    return
        except (urllib.error.URLError, OSError):
            time.sleep(1)
    sys.exit("healthz did not answer")


def login(password: str) -> Api:
    api = Api()
    profile = api.call(
        "POST",
        "/api/login",
        {"username": "admin", "password": password},
    )
    teams = profile.get("teams") or []
    if not teams:
        sys.exit("admin has no team")
    api.team = teams[0]["id"]
    return api


def wait_job(api: Api, job_id: str) -> dict:
    deadline = time.time() + 300
    while time.time() < deadline:
        job = api.call("GET", f"/api/jobs/{job_id}")
        if job.get("status") in TERMINAL:
            return job
        time.sleep(1)
    sys.exit(f"job {job_id} did not finish")


def seeded_defaults(api: Api) -> dict:
    environments = api.call("GET", "/api/environments")
    settings = api.call("GET", "/api/settings")
    expected = {"ansible": "Ansible (curated)", "opentofu": "OpenTofu (curated)", "shell": "Shell (curated)"}
    found = {}
    for tool, name in expected.items():
        default = settings.get(f"default_environment_{tool}") or ""
        row = next(
            (
                item
                for item in environments
                if item.get("id") == default and item.get("name") == name
            ),
            None,
        )
        if row is None:
            sys.exit(f"default environment for {tool} is not {name}: {settings}")
        found[tool] = row["id"]
    return found


def template(api: Api, name: str) -> str:
    rows = api.call("GET", "/api/templates")
    for row in rows:
        if row.get("name") == name:
            return row["id"]
    sys.exit(f"demo template {name} is missing")


def run(api: Api, name: str, marker: str) -> None:
    template_id = template(api, name)
    launched = api.call(
        "POST",
        f"/api/templates/{template_id}/launch",
        {"extra_vars": {}},
        ok=(200, 202),
    )
    job_id = launched["id"]
    job = wait_job(api, job_id)
    output = api.text(f"/api/jobs/{job_id}/output")
    if job.get("status") != "successful" or marker not in output:
        sys.exit(f"{name} job {job.get('status')}:\n{output[-3000:]}")
    default = job.get("parameters", {}).get("environment_name") or ""
    expected = {"Healthcheck": "Shell (curated)", "Infrastruktur-Plan": "OpenTofu (curated)", "Self-Heal": "Ansible (curated)"}[name]
    if default != expected:
        sys.exit(f"{name} ran with {default}, expected {expected}")
    print(f"{name} ran on {expected}")


def main() -> None:
    secret = os.environ.get("EE_SOCKET_PASSWORD", "").strip()
    if not secret:
        sys.exit("EE_SOCKET_PASSWORD is empty")
    wait_health()
    api = login(secret)
    seeded_defaults(api)
    run(api, "Healthcheck", "healthcheck ok")
    run(api, "Infrastruktur-Plan", "message")
    run(api, "Self-Heal", "PLAY RECAP")


if __name__ == "__main__":
    main()
