# Koreograf

[![ci](https://github.com/koreograf/koreograf/actions/workflows/ci.yml/badge.svg)](https://github.com/koreograf/koreograf/actions/workflows/ci.yml)

Koreograf is a prototype for team-scoped automation. One Rust binary serves the API, the browser UI, and the job runner. Ansible, OpenTofu, and shell run as jobs. Workflows are directed graphs. Webhooks from Alertmanager or Grafana start those graphs.

## Status

Alpha [0.1.0-alpha.1](CHANGELOG.md).

This is an early prototype, not a replacement for AWX and not a rebuild of Semaphore. There are no surveys, no schedules, no Receptor, and no high-availability setup. The database today is one SQLite file with one writer. PostgreSQL is not connected yet.

## Quick start

On a machine, with Rust 1.89 or newer:

```bash
cargo run
```

The UI is at `http://127.0.0.1:8080`. Sign in as `admin`. Without `KOREOGRAF_ADMIN_PASSWORD`, the first start writes the password to `data/initial-admin-password`.

Docker Compose:

```bash
docker compose up --build
```

Helm:

```bash
helm upgrade --install koreograf deploy/helm/koreograf
```

Requirements for each tool, the Helm password path, and the image details are in [Installation](docs/installation.md).

## Demo

Sign in, select the team `Plattform`, and press `Test alert`. The workflow runs the files in [examples/demo](examples/demo). The steps are in [Workflows and webhooks](docs/workflows-webhooks.md#demo).

## Documentation

- [Installation](docs/installation.md) — run on a machine, Docker Compose, and Helm.
- [Configuration](docs/configuration.md) — environment variables, runner mode, and languages.
- [Database and backup](docs/backup-restore.md) — the SQLite file, the key, and the backup zip.
- [Logs and Prometheus](docs/observability.md) — stdout fields and the metrics endpoint.
- [Organizations, teams, LDAP, and Active Directory](docs/authentication.md) — roles, directory sign-in, and users.
- [Jobs](docs/jobs.md) — tools, execution environments, playbooks, inventory sources, and the cache.
- [Workflows and webhooks](docs/workflows-webhooks.md) — graphs, Alertmanager and Grafana hooks, and the demo.
- [API](docs/api.md) — session cookie, team header, and routes.
- [Security](docs/security.md) — the trusted network, the key, and the backup zip.
- [Contributing](CONTRIBUTING.md) — local checks and what GitHub Actions runs.

## Security

The prototype is meant for a trusted network. Local jobs and arbitrary project paths are limited to administrators unless the environment opts in. See [Security](docs/security.md). Report vulnerabilities as described in [SECURITY.md](SECURITY.md).

## License

Koreograf is released under the [MIT license](LICENSE). Changes are listed in the [changelog](CHANGELOG.md).
