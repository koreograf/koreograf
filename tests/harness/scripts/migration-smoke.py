#!/usr/bin/env python3
import http.cookiejar
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

BASE = os.environ["KOREOGRAF_SMOKE_URL"]
PASSWORD = os.environ["KOREOGRAF_SMOKE_PASSWORD"]
DATA = Path(os.environ["KOREOGRAF_SMOKE_DATA"])
STATE = os.environ.get("KOREOGRAF_SMOKE_STATE", "")
EXPECTED_SOURCE = os.environ.get("KOREOGRAF_SMOKE_EXPECT_SOURCE", "/home/user/projects/demo")
OLD_PROJECTS = os.environ.get("KOREOGRAF_SMOKE_OLD_PROJECTS", "/home/user/projects")


class Api:
    def __init__(self):
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar)
        )
        self.team = ""

    def call(self, method, path, payload=None, raw=False):
        body = None
        headers = {}
        if payload is not None:
            body = json.dumps(payload).encode()
            headers["content-type"] = "application/json"
        if self.team:
            headers["x-team-id"] = self.team
        request = urllib.request.Request(
            BASE + path, data=body, headers=headers, method=method
        )
        try:
            with self.opener.open(request, timeout=30) as response:
                data = response.read()
                if raw:
                    return data
                return json.loads(data) if data else None
        except urllib.error.HTTPError as error:
            detail = error.read().decode("utf-8", "replace")
            sys.exit(f"{method} {path} failed with {error.code}: {detail}")

    def wait_health(self):
        for _ in range(90):
            try:
                with urllib.request.urlopen(BASE + "/healthz", timeout=2) as response:
                    if response.status == 200:
                        return
            except OSError:
                time.sleep(1)
        sys.exit("/healthz did not answer")

    def login(self):
        profile = self.call(
            "POST", "/api/login", {"username": "admin", "password": PASSWORD}
        )
        if profile.get("username") != "admin":
            sys.exit("admin login did not return the admin profile")
        self.team = profile["teams"][0]["id"]


def state_path(data: Path) -> Path:
    matches = list(data.glob("cache/*/tofu-state/smoke-tofu/terraform.tfstate"))
    if len(matches) != 1:
        sys.exit(f"expected one restored OpenTofu state, found {matches}")
    return matches[0]


def empty_plan(state: Path):
    module = """
resource "terraform_data" "smoke" {
  input = "smoke"
}
"""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / "main.tf").write_text(module, encoding="utf-8")
        shutil.copy2(state, root / "terraform.tfstate")
        for command in (
            ["tofu", "init", "-input=false", "-no-color"],
            ["tofu", "plan", "-input=false", "-no-color", "-state=terraform.tfstate"],
        ):
            result = subprocess.run(
                command, cwd=root, text=True, capture_output=True, check=False
            )
            if result.returncode != 0:
                sys.exit(f"{' '.join(command)} failed:\n{result.stdout}\n{result.stderr}")
            if command[1] == "plan" and "No changes." not in result.stdout:
                sys.exit(f"OpenTofu plan was not empty:\n{result.stdout}")


def seed():
    api = Api()
    api.wait_health()
    api.login()
    credential = api.call(
        "POST",
        "/api/credentials",
        {
            "name": "Smoke env",
            "kind": "env",
            "secret": {"SMOKE_TOKEN": "restored-secret"},
        },
    )
    api.call(
        "POST",
        "/api/projects",
        {
            "name": "Smoke path",
            "source_type": "path",
            "source": f"{OLD_PROJECTS}/demo",
            "revision": "",
            "team_ids": [api.team],
        },
    )
    module = Path(tempfile.mkdtemp())
    try:
        (module / "main.tf").write_text(
            'resource "terraform_data" "smoke" {\n  input = "smoke"\n}\n',
            encoding="utf-8",
        )
        for command in (
            ["tofu", "init", "-input=false", "-no-color"],
            ["tofu", "apply", "-auto-approve", "-input=false", "-no-color"],
        ):
            result = subprocess.run(
                command, cwd=module, text=True, capture_output=True, check=False
            )
            if result.returncode != 0:
                sys.exit(f"{' '.join(command)} failed:\n{result.stdout}\n{result.stderr}")
        target = (
            DATA
            / "cache"
            / api.team
            / "tofu-state"
            / "smoke-tofu"
            / "terraform.tfstate"
        )
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(module / "terraform.tfstate", target)
    finally:
        shutil.rmtree(module, ignore_errors=True)
    archive = api.call("GET", "/api/admin/backup?tofu_state=true", raw=True)
    destination = Path(os.environ["KOREOGRAF_SMOKE_BACKUP"])
    destination.write_bytes(archive)
    import zipfile

    with zipfile.ZipFile(destination) as backup:
        names = backup.namelist()
        if not any(name.endswith("/tofu-state/smoke-tofu/terraform.tfstate") for name in names):
            sys.exit(f"backup does not contain the OpenTofu state: {names}")
    print(f"seeded old installation (credential {credential['id']})")


def verify():
    api = Api()
    api.wait_health()
    api.login()
    credentials = api.call("GET", "/api/credentials")
    matches = [item for item in credentials if item.get("name") == "Smoke env"]
    if len(matches) != 1:
        sys.exit(f"credential did not survive: {credentials}")
    api.call(
        "PUT",
        f"/api/credentials/{matches[0]['id']}",
        {"name": "Smoke env", "secret": {}},
    )
    projects = api.call("GET", "/api/projects")
    matches = [item for item in projects if item.get("name") == "Smoke path"]
    if len(matches) != 1 or matches[0].get("source") != EXPECTED_SOURCE:
        sys.exit(f"path project did not survive as expected: {projects}")
    empty_plan(Path(STATE) if STATE else state_path(DATA))
    print("login, credential decryption, path project, and empty OpenTofu plan ok")


if __name__ == "__main__":
    if len(sys.argv) != 2 or sys.argv[1] not in ("seed", "verify"):
        sys.exit("usage: migration-smoke.py seed|verify")
    if sys.argv[1] == "seed":
        seed()
    else:
        verify()
