#!/usr/bin/env python3
import http.cookiejar
import json
import os
import sys
import time
import urllib.error
import urllib.request
import uuid

BASE = os.environ.get("KOREOGRAF_IMAGE_SMOKE_URL", "http://127.0.0.1:8080")
PASSWORD = os.environ["KOREOGRAF_SMOKE_PASSWORD"]
ALPINE = "alpine:3.22@sha256:2e0b81d60597f449baa56dc25d355367869d2dca4cd57e866f1e2b52af3eb55f"


class Api:
    def __init__(self):
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar)
        )
        self.team = ""

    def call(self, method, path, payload=None, headers=None, raw=False, data=None):
        body = data
        request_headers = dict(headers or {})
        if payload is not None:
            body = json.dumps(payload).encode()
            request_headers["content-type"] = "application/json"
        if self.team:
            request_headers["x-team-id"] = self.team
        request = urllib.request.Request(
            BASE + path, data=body, headers=request_headers, method=method
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
        return profile

    def wait_job(self, job_id, timeout=180):
        for _ in range(timeout):
            job = self.call("GET", f"/api/jobs/{job_id}")
            if job.get("status") in ("successful", "failed", "canceled"):
                return job
            time.sleep(1)
        sys.exit(f"job {job_id} did not finish")

    def ensure(self, path, name, payload):
        rows = self.call("GET", path)
        for row in rows:
            if row.get("name") == name:
                return row
        return self.call("POST", path, payload)


def main():
    api = Api()
    api.wait_health()
    api.login()
    print("health and admin login ok")

    backup = api.call("GET", "/api/admin/backup", raw=True)
    if not backup.startswith(b"PK"):
        sys.exit("backup is not a zip")
    api.call(
        "POST",
        "/api/admin/backup",
        headers={"content-type": "application/zip", "x-restore-confirm": "ERSETZEN"},
        raw=True,
        data=backup,
    )
    api = Api()
    api.wait_health()
    api.login()
    print("backup restore ok")

    project = api.ensure(
        "/api/projects",
        "Image smoke git",
        {
            "name": "Image smoke git",
            "source_type": "git",
            "source": "https://github.com/koreograf/koreograf.git",
            "revision": "",
            "team_ids": [api.team],
        },
    )
    synced = api.call("POST", f"/api/projects/{project['id']}/sync")
    if not synced.get("path"):
        sys.exit("git project sync returned no path")
    print("git over https ok")

    environment = api.ensure(
        "/api/environments",
        "Image smoke shell",
        {
            "name": "Image smoke shell",
            "image": ALPINE,
            "tool": "shell",
            "scope": "global",
            "pull_policy": "if_not_present",
        },
    )
    template = api.ensure(
        "/api/templates",
        "Image smoke shell",
        {
            "name": "Image smoke shell",
            "tool": "shell",
            "environment_id": environment["id"],
            "entrypoint": "",
            "extra_vars": {},
            "spec": {"script": "echo image-smoke-ok"},
        },
    )
    marker = f"smoke-{uuid.uuid4()}"
    launched = api.call(
        "POST", f"/api/templates/{template['id']}/launch", {"extra_vars": {}}
    )
    job = api.wait_job(launched["id"])
    if job.get("status") != "successful":
        sys.exit(f"shell job ended as {job.get('status')}")
    output = api.call("GET", f"/api/jobs/{job['id']}/output", raw=True).decode(
        "utf-8", "replace"
    )
    if "image-smoke-ok" not in output:
        sys.exit(f"shell job output did not contain the marker\n{output}")
    print(f"shell job in an execution environment ok ({marker})")


if __name__ == "__main__":
    main()
