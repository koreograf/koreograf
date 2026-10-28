#!/usr/bin/env python3
"""Sign in against the OpenLDAP group-search lab.

cargo test does not run this file. ldap-groups.sh starts OpenLDAP and the
proxy, then this script runs Koreograf and checks both sign-in cases.
"""

from __future__ import annotations

import argparse
import json
import os
import pty
import select
import signal
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from http.cookiejar import CookieJar
from pathlib import Path

WARNING = "LDAP service re-bind failed, so the group search was skipped"


def read_secret(directory: Path, name: str) -> str:
    path = directory / name
    if not path.is_file():
        sys.exit(f"missing {path.name}")
    secret = path.read_text(encoding="utf-8").strip()
    if not secret:
        sys.exit(f"{path.name} is empty")
    return secret


class Api:
    def __init__(self, base: str, redact) -> None:
        self.base = base
        self.redact = redact
        jar = CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))

    def call(self, method: str, path: str, payload: dict | None = None, ok=(200,)):
        data = None
        headers = {"Accept": "application/json"}
        if payload is not None:
            data = json.dumps(payload).encode("utf-8")
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(
            self.base + path, data=data, headers=headers, method=method
        )
        try:
            with self.opener.open(request, timeout=60) as response:
                status = response.status
                text = response.read().decode("utf-8", errors="replace")
        except urllib.error.HTTPError as err:
            text = err.read().decode("utf-8", errors="replace")
            sys.exit(self.redact(f"{method} {path} returned {err.code}: {text[:500]}"))
        except urllib.error.URLError as err:
            sys.exit(self.redact(f"{method} {path} failed: {err.reason}"))
        if status not in ok:
            sys.exit(self.redact(f"{method} {path} returned {status}: {text[:500]}"))
        if not text:
            return {}
        return json.loads(text)


def wait_healthz(base: str, proc: subprocess.Popen) -> None:
    deadline = time.time() + 600
    while time.time() < deadline:
        if proc.poll() is not None:
            sys.exit(f"Koreograf exited ({proc.returncode}) before /healthz")
        try:
            with urllib.request.urlopen(base + "/healthz", timeout=5) as response:
                if response.status == 200:
                    print("healthz ok")
                    return
        except (urllib.error.URLError, TimeoutError):
            time.sleep(1)
    sys.exit("healthz did not answer")


def copy_pty(master: int, log_path: Path, stop: threading.Event) -> None:
    with log_path.open("w", encoding="utf-8", errors="replace") as handle:
        while not stop.is_set():
            ready, _, _ = select.select([master], [], [], 0.2)
            if not ready:
                continue
            try:
                data = os.read(master, 65536)
            except OSError:
                break
            if not data:
                break
            handle.write(data.decode("utf-8", "replace"))
            handle.flush()


def start_server(repo: Path, env: dict[str, str], log_path: Path, pid_path: Path) -> tuple:
    master, slave = pty.openpty()
    proc = subprocess.Popen(
        ["cargo", "run", "--bin", "koreograf"],
        cwd=repo,
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=slave,
        stderr=slave,
        start_new_session=True,
    )
    os.close(slave)
    pid_path.write_text(f"{proc.pid}\n", encoding="utf-8")
    stop = threading.Event()
    thread = threading.Thread(target=copy_pty, args=(master, log_path, stop), daemon=True)
    thread.start()
    return proc, master, thread, stop


def stop_server(proc, master: int, thread: threading.Thread, stop: threading.Event, pid_path: Path, guard: dict) -> None:
    if guard["done"]:
        return
    guard["done"] = True
    stop.set()
    if proc.poll() is None:
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except OSError:
            pass
        try:
            proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except OSError:
                pass
            proc.wait(timeout=5)
    try:
        os.close(master)
    except OSError:
        pass
    try:
        pid_path.unlink()
    except OSError:
        pass
    thread.join(timeout=2)


def team_named(items, name: str) -> dict:
    for item in items:
        if isinstance(item, dict) and item.get("name") == name:
            return item
    sys.exit(f"team {name} is missing")


def wait_for(path: Path, needle: str, previous: str) -> str:
    deadline = time.time() + 10
    while time.time() < deadline:
        text = path.read_text(encoding="utf-8", errors="replace")
        fresh = text[len(previous) :] if text.startswith(previous) else text
        if needle in fresh:
            return text
        time.sleep(0.2)
    return path.read_text(encoding="utf-8", errors="replace")


def assert_clean(text: str, secrets: list[str], label: str) -> None:
    for secret in secrets:
        if secret and secret in text:
            sys.exit(f"{label} contains a lab password")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="http://127.0.0.1:18082")
    parser.add_argument("--ldap-url", default="ldap://127.0.0.1:1389")
    parser.add_argument("--proxy-url", default="ldap://127.0.0.1:1390")
    parser.add_argument("--secrets", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--pid-file", type=Path, required=True)
    parser.add_argument("--proxy-log", type=Path, required=True)
    parser.add_argument("--reader-dn", required=True)
    parser.add_argument("--user-base", required=True)
    parser.add_argument("--group-base", required=True)
    parser.add_argument("--group-dn", required=True)
    args = parser.parse_args()

    admin_password = read_secret(args.secrets, "admin-password")
    user_password = read_secret(args.secrets, "ldap-user-password")
    reader_password = read_secret(args.secrets, "ldap-reader-password")
    secrets = [admin_password, user_password, reader_password]

    def redact(text: str) -> str:
        for secret in secrets:
            text = text.replace(secret, "[redacted]")
        return text

    parsed = urllib.parse.urlparse(args.base)
    if not parsed.hostname or not parsed.port:
        sys.exit("--base needs a host and a port")
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith("KOREOGRAF_")
    }
    env.update(
        {
            "KOREOGRAF_BIND": f"{parsed.hostname}:{parsed.port}",
            "KOREOGRAF_DATA": str(args.data),
            "KOREOGRAF_ADMIN_PASSWORD": admin_password,
            "KOREOGRAF_RUNNER": "local",
            "KOREOGRAF_JOB_TIMEOUT_SECS": "1800",
            "KOREOGRAF_JOB_OUTPUT_LINES": "8000",
            "KOREOGRAF_LDAP_URL": args.ldap_url,
            "KOREOGRAF_LDAP_BIND_DN": args.reader_dn,
            "KOREOGRAF_LDAP_BIND_PASSWORD": reader_password,
            "KOREOGRAF_LDAP_USER_BASE": args.user_base,
            "KOREOGRAF_LDAP_USER_FILTER": "(uid={username})",
            "KOREOGRAF_LDAP_USERNAME_ATTR": "uid",
            "KOREOGRAF_LDAP_GROUP_ATTR": "memberOf",
            "KOREOGRAF_LDAP_GROUP_BASE": args.group_base,
            "KOREOGRAF_LDAP_STARTTLS": "false",
            "KOREOGRAF_LDAP_TLS_INSECURE": "false",
            "RUST_LOG": "koreograf=info,tower_http=info",
            "CARGO_TERM_COLOR": "never",
        }
    )
    repo = Path(__file__).resolve().parents[3]
    args.data.mkdir(parents=True, exist_ok=True)
    guard = {"done": False}
    proc, master, thread, stop = start_server(repo, env, args.log, args.pid_file)

    def shutdown(signum, _frame) -> None:
        stop_server(proc, master, thread, stop, args.pid_file, guard)
        sys.exit(128 + signum)

    signal.signal(signal.SIGTERM, shutdown)
    signal.signal(signal.SIGINT, shutdown)
    try:
        wait_healthz(args.base, proc)
        check_sign_in(args, admin_password, user_password, secrets, redact)
    finally:
        stop_server(proc, master, thread, stop, args.pid_file, guard)


def check_sign_in(args, admin_password: str, user_password: str, secrets: list[str], redact) -> None:
    admin = Api(args.base, redact)
    admin.call("POST", "/api/login", {"username": "admin", "password": admin_password})
    teams = admin.call("GET", "/api/teams")
    team = team_named(teams, "Plattform")
    updated = admin.call(
        "PUT",
        f"/api/teams/{team['id']}",
        {
            "name": team["name"],
            "description": team.get("description") or "",
            "ldap_group": args.group_dn,
        },
    )
    if updated.get("ldap_group") != args.group_dn:
        sys.exit("team LDAP group was not saved as the directory DN")

    ada = Api(args.base, redact)
    profile = ada.call(
        "POST",
        "/api/login",
        {"username": "ada", "password": user_password},
    )
    if profile.get("username") != "ada":
        sys.exit(f"directory account is {profile.get('username')}")
    matched = team_named(profile.get("teams", []), "Plattform")
    if matched.get("role") != "operator":
        sys.exit(f"LDAP user role is {matched.get('role')}")
    print("group DN sign-in returned the team")

    before = args.log.read_text(encoding="utf-8", errors="replace") if args.log.is_file() else ""
    saved = admin.call(
        "PUT",
        "/api/settings",
        {
            "ldap": {
                "url": args.proxy_url,
                "mode": "ldap",
                "bind_dn": args.reader_dn,
                "bind_password": "",
                "user_base": args.user_base,
                "user_filter": "(uid={username})",
                "username_attr": "uid",
                "group_attr": "memberOf",
                "group_base": args.group_base,
                "starttls": False,
                "tls_insecure": False,
                "ca_file": "",
            },
            "proxy": {"url": "", "username": "", "password": "", "no_proxy": ""},
            "runner": "local",
            "job_timeout_secs": 1800,
            "job_output_lines": 8000,
        },
    )
    ldap = saved.get("ldap") or {}
    if ldap.get("url") != args.proxy_url:
        sys.exit("saved LDAP URL is not the proxy")
    if not ldap.get("bind_password_set"):
        sys.exit("saved LDAP settings lost the service password")

    again = Api(args.base, redact)
    profile = again.call(
        "POST",
        "/api/login",
        {"username": "ada", "password": user_password},
    )
    if profile.get("username") != "ada":
        sys.exit("re-bind failure rejected the sign-in")
    names = [item.get("name") for item in profile.get("teams", []) if isinstance(item, dict)]
    if "Plattform" in names:
        sys.exit("sign-in still listed the team after the service re-bind failed")
    print("re-bind failure still signed the user in")

    log_text = wait_for(args.log, WARNING, before)
    fresh = log_text[len(before) :] if log_text.startswith(before) else log_text
    if WARNING not in fresh:
        sys.exit("re-bind warning is missing")
    assert_clean(log_text, secrets, "Koreograf log")
    proxy_log = args.proxy_log.read_text(encoding="utf-8", errors="replace")
    assert_clean(proxy_log, secrets, "proxy log")
    if proxy_log.count("forwarded a service bind") != 1:
        sys.exit("proxy did not forward the first service bind")
    if proxy_log.count("dropped the second service bind") != 1:
        sys.exit("proxy did not drop exactly one service re-bind")
    if proxy_log.count("accepted a connection") != 1:
        sys.exit("sign-in did not use exactly one proxied connection")
    print("re-bind warning does not contain the password")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
