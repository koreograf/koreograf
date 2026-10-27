# Koreograf

[![ci](https://github.com/koreograf/koreograf/actions/workflows/ci.yml/badge.svg)](https://github.com/koreograf/koreograf/actions/workflows/ci.yml)

Koreograf is a prototype for team-scoped automation. One Rust binary serves the API, the browser UI, and the job runner. Ansible, OpenTofu, and shell run as jobs. Workflows are directed graphs. Webhooks from Alertmanager or Grafana start those graphs.

## Status

Alpha [0.1.0-alpha.5](CHANGELOG.md). <!-- x-release-please-version -->
`Cargo.toml` uses the same pre-release version.

This is an early prototype, not a replacement for AWX and not a rebuild of Semaphore. There are no surveys, no Receptor, and no high-availability setup. A schedule can start a job or a workflow at a local clock time. The database today is one SQLite file with one writer. PostgreSQL is not connected yet.

## Quick start

### Docker Compose

You need Docker Engine 20.10 or newer and Docker Compose v2, the `docker compose` command. The published image is `linux/amd64`.

```bash
git clone https://github.com/koreograf/koreograf.git
cd koreograf
cp .env.example .env
docker compose up -d
```

Compose pulls `ghcr.io/koreograf/koreograf:0.1.0-alpha.5`. <!-- x-release-please-version -->
It does not compile. Build this checkout without replacing that published tag:

```bash
docker compose -f docker-compose.yml -f docker-compose.build.yml up --build -d
```

The local image is tagged `koreograf:local`. buildx is only for publishing a multi-architecture image.

The port mapping is `127.0.0.1:8080:8080`, so the UI is only on loopback. Change it to `8080:8080` to publish every interface, and put TLS on a reverse proxy when other machines should reach the UI.

The UI is at `http://127.0.0.1:8080`. Sign in as `admin`. The generated password is one line in the container. The file is mode `0600`, and the password is not written to the log.

```bash
docker compose exec koreograf cat /var/lib/koreograf/initial-admin-password
```

Koreograf does not delete that file. After the first sign-in, remove it and set a new password of at least 8 characters on the account page (the name in the sidebar):

```bash
docker compose exec koreograf rm /var/lib/koreograf/initial-admin-password
```

The volume is `<project>_koreograf-data`. Compose takes the project name from the directory name, folded to lowercase, so a checkout named `Koreograf` uses `koreograf_koreograf-data`. `docker volume ls` shows it. `COMPOSE_PROJECT_NAME` overrides the project name.

Compose ignores `KOREOGRAF_BIND`, `KOREOGRAF_DATA`, `KOREOGRAF_WEB`, and `KOREOGRAF_EXAMPLES` from `.env`. It fixes those four inside the container. The other names in `.env.example` are forwarded. Comments in that file are English.

### On a machine

Rust 1.89 is the minimum in `Cargo.toml`. This repository pins 1.96.0 in `rust-toolchain.toml`, and rustup installs that toolchain when you build here.

```bash
cargo run
```

The UI is at `http://127.0.0.1:8080`. Without `KOREOGRAF_ADMIN_PASSWORD`, the first start writes the password to `data/initial-admin-password` and logs that path. Delete the file after reading it. The process does not remove it.

### Helm

```bash
helm upgrade --install koreograf deploy/helm/koreograf
```

If `admin.password` is empty, the first password is in the volume at `/var/lib/koreograf/initial-admin-password`. Delete that file after the first sign-in. Set `existingSecret` when the admin password, the secret key, and the LDAP bind password already live in a Secret. `existingSecretKeys` selects the key names. The chart then skips its own Secret. Tool requirements, the image tag, the non-root defaults, and path projects are in [Installation](docs/installation.md). A copy-paste API session is in [API](docs/api.md#walkthrough).

## Demo

Sign in, select the team `Plattform`, and press `Test alert`. The workflow runs the files in [examples/demo](examples/demo). The steps are in [Workflows and webhooks](docs/workflows-webhooks.md#demo).

## OpenTofu state

`plan` reads the saved state and does not change resources. `apply` keeps that state for one team and one template at `data/cache/<team>/tofu-state/<template>/terraform.tfstate`. The next apply of the same template sees the resources from the last run. The plugin cache stays at `data/cache/<team>/tofu`, and `TF_DATA_DIR` stays in the job directory. Another team has a different directory. The state file is local to this controller and is not in the backup zip. Use a remote backend when the state has to survive a new data directory or be used from more than one machine. `apply` writes a warning when the module has no remote backend. Details are in [Jobs](docs/jobs.md#what-the-cache-changes).

## Job output

Each of stdout and stderr keeps the first three quarters of a line limit and the last quarter. The default limit is 8000 lines, so a long job keeps the first 6000 and the last 2000. A system line between those parts says how many lines were left out. Lines after a cancel are still discarded.

`KOREOGRAF_JOB_OUTPUT_LINES` sets the limit. Values outside 100 to 100000 are clamped. An administrator can change the same limit under Settings. The next job uses the new value. The tail lines wait in memory until the stream ends, and that buffer stays within 8 MiB per stream.

`GET /api/jobs/{id}/output` downloads the kept log. The file stops at 8 MiB and then ends with `… output truncated.` The job page shows that log. When more than 2000 lines are stored, the page shows the start and the end and says that the middle of the view was left out.

Play recap lines and `fatal:` lines are read from the whole stream, including lines that are not kept, so failed hosts still follow the real end of the play.

## Documentation

- [Installation](docs/installation.md) — run on a machine, Docker Compose, and Helm.
- [Configuration](docs/configuration.md) — environment variables, runner mode, and languages.
- [Database and backup](docs/backup-restore.md) — the SQLite file, the key, and the backup zip.
- [Logs and Prometheus](docs/observability.md) — stdout fields and the metrics endpoint.
- [Organizations, teams, LDAP, and Active Directory](docs/authentication.md) — roles, directory sign-in, and users.
- [Jobs](docs/jobs.md) — tools, execution environments, playbooks, inventory sources, and the cache.
- [Workflows and webhooks](docs/workflows-webhooks.md) — graphs, Alertmanager and Grafana hooks, and the demo.
- [API](docs/api.md) — session cookie (`Secure` with `KOREOGRAF_COOKIE_SECURE`), team header, and routes.
- [Security](docs/security.md) — the trusted network, the key, and the backup zip.
- [Contributing](CONTRIBUTING.md) — local checks and what GitHub Actions runs.

## Security

The prototype is meant for a trusted network. Local jobs and arbitrary project paths are limited to administrators unless the environment opts in. See [Security](docs/security.md). Report vulnerabilities as described in [SECURITY.md](SECURITY.md).

## License

Koreograf is released under the [MIT license](LICENSE). Changes are listed in the [changelog](CHANGELOG.md).
