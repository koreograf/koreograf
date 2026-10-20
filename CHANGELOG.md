# Changelog

## Unreleased

Local jobs on the controller now default to administrators. An install that previously let operators, webhooks, or workflow nodes start those jobs sets `KOREOGRAF_LOCAL_RUNNER_FOR=all`. An operator path project also needs its directory in `KOREOGRAF_PROJECT_ROOTS`. See [Configuration](docs/configuration.md).

## 0.1.0-alpha.1

First public alpha. One Rust binary serves the API, the German UI, and the job runner. English is available from the language menu.

This release can run Ansible, OpenTofu, and shell jobs for teams in organizations. It stores credentials, inventories (including a project file and NetBox), and projects. Execution environments are global, per organization, or private to one team. The Galaxy, role, and OpenTofu cache stays under `data/cache/<team>/`. Workflows are directed graphs. Alertmanager and Grafana webhooks can start a job or a workflow. An administrator can set the directory, the HTTP proxy, the runner, and the job timeout, and can download or restore a backup of the SQLite file and `secret.key`. `GET /metrics` exposes Prometheus text.

Limits of this alpha: one SQLite file and one writer, so the Helm chart stays at one replica. There are no surveys, no schedules, no Receptor, and no high-availability setup. A `postgres://` URL stops startup. The container image is `linux/amd64`. The prototype expects a trusted network. The Docker socket is off unless you turn it on, and then it is root on the host.
