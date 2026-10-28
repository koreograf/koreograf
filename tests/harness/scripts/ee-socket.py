#!/usr/bin/env python3
"""API checks for the socket harness. The shell script starts Compose."""

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

BASE = "http://127.0.0.1:18083"
WRONG = "/var/lib/koreograf-ee-socket-wrong"
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


def wait_job(api: Api, job_id: str, timeout: int) -> dict:
    deadline = time.time() + timeout
    while time.time() < deadline:
        job = api.call("GET", f"/api/jobs/{job_id}")
        if job.get("status") in TERMINAL:
            return job
        time.sleep(1)
    sys.exit(f"job {job_id} did not finish")


def container_ids(name: str) -> list:
    result = subprocess.run(
        ["docker", "ps", "-aq", "--filter", f"name=^{name}$"],
        check=False,
        capture_output=True,
        text=True,
    )
    return [line for line in result.stdout.splitlines() if line.strip()]


def wait_container(name: str, present: bool, timeout: int) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        found = bool(container_ids(name))
        if found == present:
            return
        time.sleep(1)
    state = "appear" if present else "go away"
    sys.exit(f"container {name} did not {state}")


def ensure_environment(api: Api) -> str:
    rows = api.call("GET", "/api/environments")
    for row in rows:
        if row.get("name") == "Socket image":
            return row["id"]
    created = api.call(
        "POST",
        "/api/environments",
        {
            "name": "Socket image",
            "image": "koreograf:ee-socket",
            "pull_policy": "never",
            "description": "Image built for the socket check.",
        },
        ok=(200, 201),
    )
    return created["id"]


def ensure_template(api: Api, name: str, environment_id: str, script: str) -> str:
    rows = api.call("GET", "/api/templates")
    for row in rows:
        if row.get("name") == name:
            return row["id"]
    created = api.call(
        "POST",
        "/api/templates",
        {
            "name": name,
            "description": "Socket harness.",
            "tool": "shell",
            "environment_id": environment_id,
            "entrypoint": "",
            "extra_vars": {},
            "spec": {"script": script},
            "update_on_launch": False,
            "prompt_on_launch": False,
        },
        ok=(200, 201),
    )
    return created["id"]


def launch(api: Api, template_id: str) -> str:
    job = api.call(
        "POST",
        f"/api/templates/{template_id}/launch",
        {"extra_vars": {}},
        ok=(200, 202),
    )
    return job["id"]


def read_password() -> str:
    value = os.environ.get("EE_SOCKET_PASSWORD", "").strip()
    if not value:
        sys.exit("EE_SOCKET_PASSWORD is empty")
    return value


def run_success(password: str) -> None:
    wait_health()
    api = login(password)
    environment_id = ensure_environment(api)
    echo_id = ensure_template(
        api, "Socket echo", environment_id, "echo ee-socket-ok"
    )
    echo_job = launch(api, echo_id)
    echo = wait_job(api, echo_job, 180)
    output = api.text(f"/api/jobs/{echo_job}/output")
    if echo.get("status") != "successful" or "ee-socket-ok" not in output:
        sys.exit(f"echo job {echo.get('status')}:\n{output[-2000:]}")
    print("socket echo successful")

    sleep_id = ensure_template(api, "Socket sleep", environment_id, "sleep 60")
    sleep_job = launch(api, sleep_id)
    short = sleep_job[:8]
    name = f"koreograf-{short}"
    wait_container(name, True, 90)
    api.call("POST", f"/api/jobs/{sleep_job}/cancel", ok=(200,))
    wait_container(name, False, 30)
    canceled = wait_job(api, sleep_job, 60)
    if canceled.get("status") != "canceled":
        sys.exit(f"sleep job finished as {canceled.get('status')}")
    print("canceled container was removed")


def run_wrong(password: str) -> None:
    wait_health()
    api = login(password)
    rows = api.call("GET", "/api/templates")
    template_id = ""
    for row in rows:
        if row.get("name") == "Socket echo":
            template_id = row["id"]
    if not template_id:
        sys.exit("socket echo template is missing")
    job_id = launch(api, template_id)
    job = wait_job(api, job_id, 120)
    output = api.text(f"/api/jobs/{job_id}/output")
    if job.get("status") != "failed":
        sys.exit(f"wrong path finished as {job.get('status')}:\n{output[-2000:]}")
    if "KOREOGRAF_DATA_HOST" not in output or "was not started" not in output:
        sys.exit(f"wrong path did not explain itself:\n{output[-2000:]}")
    if Path(WRONG).exists():
        sys.exit(f"{WRONG} was created")
    print("wrong data path failed before the container started")


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit("usage: ee-socket.py success|wrong")
    mode = sys.argv[1]
    secret = read_password()
    if mode == "success":
        run_success(secret)
    elif mode == "wrong":
        run_wrong(secret)
    else:
        sys.exit(f"unknown mode {mode}")


if __name__ == "__main__":
    main()
