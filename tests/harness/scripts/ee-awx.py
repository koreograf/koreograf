#!/usr/bin/env python3
"""One stock awx-ee job through the socket harness."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from http.cookiejar import CookieJar
from pathlib import Path

BASE = "http://127.0.0.1:18084"
ROOT = Path(__file__).resolve().parents[1]
COMPOSE = ROOT / "ee-socket-compose.yml"
PROJECT = "koreograf-ee-awx"
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


def ensure_environment(api: Api) -> str:
    rows = api.call("GET", "/api/environments")
    for row in rows:
        if row.get("name") == "Stock AWX EE":
            return row["id"]
    created = api.call(
        "POST",
        "/api/environments",
        {
            "name": "Stock AWX EE",
            "image": "quay.io/ansible/awx-ee:latest",
            "pull_policy": "if_not_present",
            "description": "Unmodified awx-ee image.",
        },
        ok=(200, 201),
    )
    return created["id"]


def ensure_template(api: Api, environment_id: str) -> str:
    rows = api.call("GET", "/api/templates")
    for row in rows:
        if row.get("name") == "AWX echo":
            return row["id"]
    created = api.call(
        "POST",
        "/api/templates",
        {
            "name": "AWX echo",
            "description": "awx-ee ownership check.",
            "tool": "shell",
            "environment_id": environment_id,
            "entrypoint": "",
            "extra_vars": {},
            "spec": {
                "script": (
                    "set -eu\n"
                    "mode=$(stat -c '%a %u' /runner/work/container.env)\n"
                    "printf 'secret-mode %s\\n' \"$mode\"\n"
                    "test \"$mode\" = '600 0'\n"
                    "echo ee-awx-ok\n"
                )
            },
            "update_on_launch": False,
            "prompt_on_launch": False,
        },
        ok=(200, 201),
    )
    return created["id"]


def wait_job(api: Api, job_id: str) -> dict:
    deadline = time.time() + 180
    while time.time() < deadline:
        job = api.call("GET", f"/api/jobs/{job_id}")
        if job.get("status") in TERMINAL:
            return job
        time.sleep(1)
    sys.exit(f"job {job_id} did not finish")


def compose_container() -> str:
    result = subprocess.run(
        [
            "docker",
            "compose",
            "-p",
            PROJECT,
            "-f",
            str(COMPOSE),
            "ps",
            "-q",
            "koreograf",
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    cid = result.stdout.strip()
    if result.returncode != 0 or not cid:
        sys.exit(result.stderr.strip() or "koreograf container is missing")
    return cid.splitlines()[0]


def stat_text(cid: str, path: str) -> str:
    result = subprocess.run(
        ["docker", "exec", cid, "stat", "-c", "%u %a", path],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        sys.exit(result.stderr.strip() or f"stat failed for {path}")
    return result.stdout.strip()


def main() -> None:
    secret = os.environ.get("EE_SOCKET_PASSWORD", "").strip()
    if not secret:
        sys.exit("EE_SOCKET_PASSWORD is empty")
    wait_health()
    api = login(secret)
    environment_id = ensure_environment(api)
    template_id = ensure_template(api, environment_id)
    launched = api.call(
        "POST",
        f"/api/templates/{template_id}/launch",
        {"extra_vars": {}},
        ok=(200, 202),
    )
    job_id = launched["id"]
    job = wait_job(api, job_id)
    output = api.text(f"/api/jobs/{job_id}/output")
    if job.get("status") != "successful" or "ee-awx-ok" not in output:
        sys.exit(f"awx job {job.get('status')}:\n{output[-2000:]}")
    if "The job container runs as 0:0." not in output:
        sys.exit(f"job did not run as root:\n{output[-2000:]}")
    if "secret-mode 600 0" not in output:
        sys.exit(f"secret file was not mode 600:\n{output[-2000:]}")
    cid = compose_container()
    work = stat_text(cid, f"/var/lib/koreograf/jobs/{job_id}")
    if not work.startswith("0 "):
        sys.exit(f"job directory owner is {work}")
    leftover = subprocess.run(
        [
            "docker",
            "exec",
            cid,
            "test",
            "!",
            "-e",
            f"/var/lib/koreograf/jobs/{job_id}/container.env",
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if leftover.returncode != 0:
        sys.exit("container.env was still present after the job")
    print("awx-ee echo successful")
    print(f"job directory {work}")


if __name__ == "__main__":
    main()
