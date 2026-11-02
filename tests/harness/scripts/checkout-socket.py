#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from http.cookiejar import CookieJar

BASE = "http://127.0.0.1:18085"
CONTAINER = os.environ.get("CHECKOUT_CONTAINER", "koreograf-checkout-socket")
SOCKET_GID = os.environ.get("CHECKOUT_SOCKET_GID", "")
PROJECT = "Socket checkout"


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


def ensure_project(api: Api) -> str:
    for row in api.call("GET", "/api/projects"):
        if row.get("name") == PROJECT:
            return row["id"]
    created = api.call(
        "POST",
        "/api/projects",
        {
            "name": PROJECT,
            "source_type": "git",
            "source": "file:///srv/source",
            "revision": "",
        },
        ok=(200, 201),
    )
    return created["id"]


def process_status() -> tuple[str, list[str]]:
    script = (
        'for p in /proc/[0-9]*; do '
        'name=$(sed -n "s/^Name:[[:space:]]*//p" "$p/status" 2>/dev/null); '
        'if [ "$name" = koreograf ]; then '
        'sed -n "s/^Uid:[[:space:]]*//p" "$p/status"; '
        'sed -n "s/^Groups:[[:space:]]*//p" "$p/status"; '
        "break; fi; done"
    )
    lines = subprocess.check_output(
        ["docker", "exec", CONTAINER, "sh", "-c", script], text=True
    ).splitlines()
    if len(lines) < 2:
        sys.exit("the koreograf process is missing")
    return lines[0].split()[0], lines[1].split()


def check_checkout_owner() -> None:
    paths = subprocess.check_output(
        [
            "docker",
            "exec",
            CONTAINER,
            "find",
            "/var/lib/koreograf/checkouts",
            "-mindepth",
            "1",
            "-maxdepth",
            "1",
            "-type",
            "d",
        ],
        text=True,
    ).split()
    if not paths:
        sys.exit("no checkout was created")
    for path in paths:
        owner = subprocess.check_output(
            ["docker", "exec", CONTAINER, "stat", "-c", "%u", path], text=True
        ).strip()
        if owner != "10001":
            sys.exit(f"{path} is owned by uid {owner}")


def main() -> None:
    if len(sys.argv) not in (2, 3):
        sys.exit("usage: checkout-socket.py sync [socket]")
    if sys.argv[1] != "sync":
        sys.exit(f"unknown mode {sys.argv[1]}")
    with_socket = len(sys.argv) == 3 and sys.argv[2] == "socket"
    password = os.environ.get("CHECKOUT_PASSWORD", "")
    if not password:
        sys.exit("CHECKOUT_PASSWORD is empty")
    wait_health()
    uid, groups = process_status()
    if uid != "10001":
        sys.exit(f"the server runs as uid {uid}")
    if with_socket and SOCKET_GID not in groups:
        sys.exit(f"group {SOCKET_GID} is missing from {groups}")
    api = login(password)
    project_id = ensure_project(api)
    api.call("POST", f"/api/projects/{project_id}/sync", {})
    check_checkout_owner()
    print(f"checkout synced as uid {uid} with groups {' '.join(groups)}")


if __name__ == "__main__":
    main()
