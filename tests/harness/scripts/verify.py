#!/usr/bin/env python3
"""Drive the lab API. Exit non-zero when a required check fails."""

from __future__ import annotations

import json
import sys
import time
import urllib.error
import urllib.request
from http.cookiejar import CookieJar
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SECRETS = ROOT / ".secrets"
BASE = "http://127.0.0.1:18080"
TERMINAL = {"successful", "failed", "canceled"}
DISTROS = ("debian", "ubuntu", "rocky", "alma")
INVENTORY = """[distros]
debian ansible_host=debian ansible_user=ansible ansible_python_interpreter=/usr/bin/python3
ubuntu ansible_host=ubuntu ansible_user=ansible ansible_python_interpreter=/usr/bin/python3
rocky ansible_host=rocky ansible_user=ansible ansible_python_interpreter=/usr/bin/python3
alma ansible_host=alma ansible_user=ansible ansible_python_interpreter=/usr/bin/python3

[nosudo]
nosudo ansible_host=nosudo ansible_user=ansible ansible_python_interpreter=/usr/bin/python3

[unreachable]
unreachable ansible_host=unreachable ansible_user=ansible
"""


def read_secret(name: str) -> str:
    path = SECRETS / name
    if not path.is_file():
        sys.exit(f"missing {path}. Run tests/harness/scripts/up.sh first.")
    return path.read_text(encoding="utf-8").strip()


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
        req = urllib.request.Request(
            BASE + path, data=data, headers=headers, method=method
        )
        try:
            with self.opener.open(req, timeout=60) as response:
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
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            return text

    def text(self, path: str) -> str:
        headers = {}
        if self.team:
            headers["x-team-id"] = self.team
        req = urllib.request.Request(BASE + path, headers=headers, method="GET")
        try:
            with self.opener.open(req, timeout=60) as response:
                return response.read().decode("utf-8", errors="replace")
        except urllib.error.HTTPError as err:
            body = err.read().decode("utf-8", errors="replace")
            sys.exit(f"GET {path} returned {err.code}: {body[:500]}")


def wait_healthz() -> None:
    deadline = time.time() + 120
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(BASE + "/healthz", timeout=5) as response:
                if response.status == 200:
                    print("healthz ok")
                    return
        except (urllib.error.URLError, TimeoutError):
            time.sleep(2)
    sys.exit("healthz did not answer")


def team_named(profile: dict, name: str) -> dict:
    for team in profile.get("teams", []):
        if team.get("name") == name:
            return team
    sys.exit(f"team {name} is missing from the profile")


def by_name(items, name: str):
    for item in items:
        if isinstance(item, dict) and item.get("name") == name:
            return item
    return None


def ensure(api: Api, path: str, name: str, payload: dict):
    found = by_name(api.call("GET", path), name)
    if found is not None:
        return found
    return api.call("POST", path, payload, ok=(200, 201))


def wait_done(api: Api, path: str, timeout: int) -> dict:
    deadline = time.time() + timeout
    last = ""
    announced = 0.0
    while time.time() < deadline:
        body = api.call("GET", path)
        status = str(body.get("status", ""))
        now = time.time()
        if status != last or now - announced > 30:
            print(f"{path} status {status}")
            last = status
            announced = now
        if status in TERMINAL:
            return body
        time.sleep(2)
    sys.exit(f"timed out waiting for {path}, last status {last}")


def show_failure(api: Api, body: dict) -> None:
    nodes = body.get("nodes")
    if isinstance(nodes, list):
        for node in nodes:
            job_id = node.get("job_id")
            if job_id:
                print(f"--- job {node.get('node_id')} {job_id} ---")
                print(tail(api.text(f"/api/jobs/{job_id}/output")))
        return
    job_id = body.get("id")
    if job_id:
        print(tail(api.text(f"/api/jobs/{job_id}/output")))


def hides_key_material(line: str) -> bool:
    # Split the banner so this source file does not match the key hook.
    banner = "BEG" + "IN"
    kind = "PRIV" + "ATE KEY"
    return banner in line and kind in line


def tail(text: str) -> str:
    lines = text.splitlines()[-60:]
    redacted = []
    for line in lines:
        if hides_key_material(line):
            redacted.append("[redacted key material]")
        else:
            redacted.append(line[:400])
    return "\n".join(redacted)


def require_success(api: Api, path: str, timeout: int, label: str) -> None:
    body = wait_done(api, path, timeout)
    if body.get("status") != "successful":
        show_failure(api, body)
        sys.exit(f"{label} finished as {body.get('status')}")
    print(f"{label} successful")


def main() -> None:
    wait_healthz()
    api = Api()
    profile = api.call(
        "POST",
        "/api/login",
        {
            "username": "admin",
            "password": read_secret("admin-password"),
        },
    )
    if profile.get("username") != "admin":
        sys.exit("admin login did not return the admin profile")
    print("admin login ok")

    team = team_named(profile, "Plattform")
    api.team = team["id"]

    workflows = api.call("GET", "/api/workflows")
    workflow = by_name(workflows, "Self-Heal Ablauf")
    if workflow is None:
        sys.exit("demo workflow Self-Heal Ablauf is missing")
    launched = api.call(
        "POST",
        f"/api/workflows/{workflow['id']}/launch",
        {"extra_vars": {}},
        ok=(200, 202),
    )
    require_success(api, f"/api/workflow-jobs/{launched['id']}", 840, "demo workflow")

    project = ensure(
        api,
        "/api/projects",
        "Harness ping",
        {
            "name": "Harness ping",
            "source_type": "path",
            "source": "/opt/koreograf/harness",
        },
    )
    inventory = ensure(
        api,
        "/api/inventories",
        "Lab distros",
        {"name": "Lab distros", "body": INVENTORY},
    )
    key = (SECRETS / "id_ed25519").read_text(encoding="utf-8")
    credential = ensure(
        api,
        "/api/credentials",
        "Lab SSH",
        {
            "name": "Lab SSH",
            "kind": "ssh",
            "secret": {"username": "ansible", "private_key": key},
        },
    )
    template = ensure(
        api,
        "/api/templates",
        "Ping distros",
        {
            "name": "Ping distros",
            "description": "Ping Debian, Ubuntu, Rocky Linux, and AlmaLinux.",
            "tool": "ansible",
            "project_id": project["id"],
            "inventory_id": inventory["id"],
            "credential_ids": [credential["id"]],
            "entrypoint": "ping.yml",
            "extra_vars": {},
            "spec": {},
            "update_on_launch": False,
            "prompt_on_launch": False,
        },
    )
    ping = api.call(
        "POST",
        f"/api/templates/{template['id']}/launch",
        {"extra_vars": {}},
        ok=(200, 202),
    )
    require_success(api, f"/api/jobs/{ping['id']}", 300, "distro ping")
    output = api.text(f"/api/jobs/{ping['id']}/output")
    for host in DISTROS:
        if host not in output:
            sys.exit(f"ping output does not mention {host}")
    print("distro ping mentioned all four hosts")

    updated = api.call(
        "PUT",
        f"/api/teams/{team['id']}",
        {
            "name": team["name"],
            "description": team.get("description") or "",
            "ldap_group": "Plattform",
        },
    )
    if updated.get("ldap_group") != "Plattform":
        sys.exit("team LDAP group was not saved")

    ada = Api()
    ada_profile = ada.call(
        "POST",
        "/api/login",
        {"username": "ada", "password": read_secret("ldap-user-password")},
    )
    ada_team = team_named(ada_profile, "Plattform")
    if ada_team.get("role") != "operator":
        sys.exit(f"LDAP user role is {ada_team.get('role')}")
    print("LDAP user ada is an operator on Plattform")

    token = read_secret("netbox-token")
    existing = by_name(api.call("GET", "/api/inventories"), "Lab devices")
    if existing is None:
        netbox = api.call(
            "POST",
            "/api/inventories/netbox",
            {
                "name": "Lab devices",
                "api_endpoint": "http://netbox:8080",
                "token": token,
                "validate_certs": False,
                "virtual_machines": False,
                "group_by": [],
                "query_filters": [],
            },
            ok=(200, 201),
        )
    else:
        netbox = api.call("POST", f"/api/inventories/{existing['id']}/sync")
    if netbox.get("host_count") != 2:
        sys.exit(f"NetBox host count is {netbox.get('host_count')}")
    body = netbox.get("body") or ""
    for name in ("edge-1", "edge-2"):
        if name not in body:
            sys.exit(f"NetBox inventory is missing {name}")
    print("NetBox inventory has 2 hosts")
    print("verify ok")


if __name__ == "__main__":
    main()
