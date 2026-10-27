#!/usr/bin/env python3
"""Create two NetBox devices when they are missing. The token stays in .secrets/."""

from __future__ import annotations

import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SECRETS = ROOT / ".secrets"
BASE = "http://127.0.0.1:18081"
DEVICES = ("edge-1", "edge-2")


def read_secret(name: str) -> str:
    path = SECRETS / name
    if not path.is_file():
        sys.exit(f"missing {path}. Run tests/harness/scripts/up.sh first.")
    return path.read_text(encoding="utf-8").strip()


TOKEN = read_secret("netbox-token")


class ApiError(Exception):
    pass


def request(method: str, path: str, payload: dict | None = None, query: dict | None = None):
    url = BASE + path
    if query:
        url += "?" + urllib.parse.urlencode(query)
    data = None
    headers = {
        "Authorization": f"Token {TOKEN}",
        "Accept": "application/json",
    }
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=30) as response:
            raw = response.read().decode("utf-8")
            return response.status, json.loads(raw) if raw else {}
    except urllib.error.HTTPError as err:
        body = err.read().decode("utf-8", errors="replace")
        raise ApiError(f"NetBox {method} {path} returned {err.code}: {body[:500]}") from err
    except urllib.error.URLError as err:
        raise ApiError(f"NetBox {method} {path} failed: {err.reason}") from err


def wait_ready() -> None:
    deadline = time.time() + 180
    last = "NetBox API did not become ready"
    while time.time() < deadline:
        try:
            status, _ = request("GET", "/api/dcim/sites/", query={"limit": "1"})
            if status == 200:
                return
        except ApiError as err:
            last = str(err)
            time.sleep(3)
            continue
        time.sleep(3)
    sys.exit(last)


def first_named(path: str, name: str, field: str = "name"):
    _, body = request("GET", path, query={field: name, "limit": "50"})
    for item in body.get("results", []):
        if item.get(field) == name:
            return item
    return None


def ensure(path: str, name: str, payload: dict, field: str = "name"):
    found = first_named(path, name, field)
    if found is not None:
        return found
    _, created = request("POST", path, payload)
    return created


def main() -> None:
    try:
        seed()
    except ApiError as err:
        sys.exit(str(err))


def seed() -> None:
    wait_ready()
    # This NetBox release still requires slug on these models.
    site = ensure(
        "/api/dcim/sites/",
        "Lab",
        {"name": "Lab", "slug": "lab", "status": "active"},
    )
    manufacturer = ensure(
        "/api/dcim/manufacturers/",
        "Generic",
        {"name": "Generic", "slug": "generic"},
    )
    device_type = ensure(
        "/api/dcim/device-types/",
        "generic",
        {
            "manufacturer": manufacturer["id"],
            "model": "generic",
            "slug": "generic",
        },
        field="model",
    )
    role = ensure(
        "/api/dcim/device-roles/",
        "Switch",
        {"name": "Switch", "slug": "switch", "color": "607d8b"},
    )
    for name in DEVICES:
        ensure(
            "/api/dcim/devices/",
            name,
            {
                "name": name,
                "device_type": device_type["id"],
                "role": role["id"],
                "site": site["id"],
                "status": "active",
            },
        )
    print("NetBox devices are present.")


if __name__ == "__main__":
    main()
