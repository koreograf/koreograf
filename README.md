# Koreograf

Koreograf is a prototype for team-scoped automation. One Rust binary serves the API, the browser UI, and the job runner. Ansible, OpenTofu, and shell run as jobs. Collections, roles, and OpenTofu providers stay in a cache. Workflows are directed graphs. Webhooks from Alertmanager or Grafana start those graphs.

This is an early prototype, not a replacement for AWX and not a rebuild of Semaphore. There are no surveys, no schedules, no Receptor, and no high-availability setup. The database today is one SQLite file with one writer. PostgreSQL is not connected yet.

The browser UI is German by default. DE / ENG in the sidebar and on the sign-in page switches the language.

## What the cache changes

AWX installs collections on every run because the execution environment is ephemeral. Koreograf keeps the Galaxy and role cache under `data/cache/<team>/` and mounts it into every run. A second run logs which roles are already cached. If `ansible-galaxy` fails, the job continues with the cache it already has.

OpenTofu uses `TF_PLUGIN_CACHE_DIR` on that same cache. `TF_DATA_DIR` stays in the job directory.

## Database and backup

Home labs and this prototype stay on SQLite (`data/koreograf.db`, WAL, one writer). An empty `KOREOGRAF_DATABASE_URL`, or a value of `sqlite:`, means that file. A `postgres://` URL stops startup with a clear message until a company database exists. The backup from the UI is the hand-off format for that later move: one SQLite file plus the key, not a raw copy of the data directory.

An administrator opens **Backup** in the sidebar. In the German UI the link is **Sicherung**.

- **Download** writes `koreograf-backup.zip`. Inside are `manifest.json` (`format` 1, `engine` sqlite), a consistent database (`koreograf.db`, copied through the SQLite backup API), and `secret.key`. Without the key, credentials cannot be read after a restore.
- **Left out** are the cache, git checkouts, and job directories. The next run fetches collections and roles again, and git projects are cloned again. Projects that use a local path need that same path on the target machine.
- **Restore** replaces the installation. Finish running jobs first. The request sends the zip body and the header `X-Restore-Confirm: ERSETZEN`. A new sign-in is required afterwards. The previous installation stays as `restore-previous.zip` in the data directory (mode `0600`) and can be restored the same way.
- The file is secret: it contains users, hooks, and the key for every credential. The upload limit is 256 MiB.

`GET /api/admin/backup` and `POST /api/admin/backup` are limited to the local administrator.

## Logs and Prometheus

Koreograf does not open a syslog socket. Every event is one line on stdout. journald, Docker, and Promtail read that stream. `KOREOGRAF_LOG_FORMAT=json` writes the same fields as JSON. Without the variable the format stays text.

The lines contain no credentials, no tokens, and no extra variables.

| Target | When | Fields |
| --- | --- | --- |
| `koreograf::job` | job started and finished | `job_id`, `team_id`, `tool`, `playbook`, `template`, and at the end `status` and `duration_ms` |
| `koreograf::cache` | when a job starts | collection and role counts, bytes for collections, roles, and OpenTofu |
| `koreograf::role` | Ansible role finished, and immediately on `failed` or `unreachable` | `playbook`, `role`, `result`, or the counters `ok`, `changed`, `failed`, `unreachable`, `skipped` |

For Ansible, `playbook` is the playbook path. For shell and OpenTofu it is the entry path. Koreograf reads roles from Ansible's default callback output (`TASK [role : task]`, `RUNNING HANDLER`, `ok:`, `changed:`, `fatal:`, `unreachable:`, `skipping:`). `KOREOGRAF Rolle aus Cache` means the role was already in the cache. A task name that contains ` : ` outside a role can look like a role.

`GET /metrics` is open, like `/healthz`, and returns Prometheus text. Counters live in memory and start at zero after a restart. Prometheus treats that jump as a restart. Cache numbers are read from disk on each scrape, even when no job is running. The time of a role is the log line. `koreograf_role_results_total` counts results and has no timestamp.

| Metric | Type | Labels |
| --- | --- | --- |
| `koreograf_jobs_started_total` | counter | `tool`, `playbook` |
| `koreograf_jobs_finished_total` | counter | `tool`, `playbook`, `status` (`successful`, `failed`, `canceled`) |
| `koreograf_role_results_total` | counter | `playbook`, `role`, `result` (`ok`, `changed`, `failed`, `unreachable`, `skipped`) |
| `koreograf_cache_collections` | gauge | `team_id`, `team` |
| `koreograf_cache_roles` | gauge | `team_id`, `team` |
| `koreograf_cache_bytes` | gauge | `team_id`, `team`, `kind` (`collections`, `roles`, `tofu`) |

## Run on a machine

Requirements: Rust 1.89 or newer, plus `ansible-playbook` and `tofu` on `PATH` when those tools should run locally. The shell health check needs only `sh`.

```bash
cargo run
```

The UI is at `http://127.0.0.1:8080`. DE / ENG at the bottom of the sidebar and on the sign-in page switches the language. Create-lists share the same pill button, for example `+ Projekt`, `+ Zugang`, and `+ Umgebung` (English: `+ Project`, `+ Credential`, `+ Environment`). Without `KOREOGRAF_ADMIN_PASSWORD`, the first start writes the password to `data/initial-admin-password` (mode `0600`) and logs the path. The password is not in the log.

Environment variables: see `.env.example`. Comments in that file are still German.

| Variable | Default |
| --- | --- |
| `KOREOGRAF_BIND` | `0.0.0.0:8080` |
| `KOREOGRAF_DATA` | `./data` |
| `KOREOGRAF_WEB` | `./web` |
| `KOREOGRAF_EXAMPLES` | `./examples` |
| `KOREOGRAF_ADMIN_USER` | `admin` |
| `KOREOGRAF_ADMIN_PASSWORD` | empty, then generated once |
| `KOREOGRAF_RUNNER` | `auto` (`local` or `docker`) |
| `KOREOGRAF_JOB_TIMEOUT_SECS` | `1800` (30–86400) |
| `KOREOGRAF_SECRET_KEY` | 64 hex characters, otherwise `data/secret.key` |
| `KOREOGRAF_LOG_FORMAT` | empty, text on stdout. `json` for Promtail |
| `KOREOGRAF_DATABASE_URL` | empty, SQLite. `postgres://` is rejected |

`auto` looks for `docker` and `podman`. If the runtime is missing, jobs run locally. `local` ignores execution environments and writes that into the job log. `docker` fails a run that has an image set when no runtime is reachable. Templates without an environment always run locally.

## Docker Compose

```bash
docker compose up --build
```

The image contains Ansible from Debian, the Docker client, and OpenTofu 1.12.1 for `linux/amd64`. The process runs as root because a mounted Docker socket is root on the host.

The socket is commented out in `docker-compose.yml`. Execution environments need that line. Without the socket the runner stays local, and the demo health check still runs.

Data lives in the volume `koreograf-data`. Compose reads `.env` for variable substitution.

## Helm

```bash
helm upgrade --install koreograf deploy/helm/koreograf
```

`replicaCount` is 1. A second pod on the same SQLite file loses writes. The image must be reachable in the cluster (`image.repository`). The Docker socket is off by default (`runner.dockerSocket.enabled`).

Probes call `/healthz`. Prometheus can scrape `GET /metrics` on the same port. The counters belong to this one replica and live only in memory. If `admin.password` is empty, the first password is in the volume at `/var/lib/koreograf/initial-admin-password`.

## Checks on GitHub

Every push and every pull request starts GitHub Actions. The runs check formatting and Clippy, the Rust tests, the Rust docs, the syntax of `web/app.js`, Shellcheck, Hadolint, the pre-commit hooks, the Helm chart, and the image build. The image is not published, and nothing is deployed. `cargo check` also runs on Rust 1.89, the minimum version in `Cargo.toml`. The image build uses Rust 1.96, like the Dockerfile.

Commit messages are English and follow Conventional Commits. On pull requests, commitlint checks the messages. `data/` stays out of git: it holds the SQLite file, `secret.key`, and the first admin password.

## Organizations, teams, LDAP, and Active Directory

An organization owns the projects. Each team belongs to exactly one organization. A project is available to every team in that organization until teams are assigned. Once `team_ids` is set, only those teams see the project. Assigned teams must belong to the project's organization.

An operator cannot move a project to an organization other than the one of the team currently selected. An administrator can. A new team without `organization_id` lands in an organization of the same name, or creates that organization. Inventories and credentials stay on the team.

Roles are `viewer`, `operator`, and `owner`. The local administrator bypasses the role check so an LDAP outage cannot lock everyone out. Sign-in checks the local password first, then LDAP.

Memberships from LDAP have source `ldap` and role `operator`. When the group disappears, only that membership is removed. Manual memberships stay.

A team stores an LDAP group as a CN or a DN. The comparison ignores case.

| Variable | LDAP | Active Directory |
| --- | --- | --- |
| `KOREOGRAF_LDAP_URL` | `ldap://ldap.example.com` | `ldap://dc.example.com` |
| `KOREOGRAF_LDAP_MODE` | `ldap` | `ad` |
| `KOREOGRAF_LDAP_USER_FILTER` | `(uid={username})` | `(sAMAccountName={username})` |
| `KOREOGRAF_LDAP_USERNAME_ATTR` | `uid` | `sAMAccountName` |
| `KOREOGRAF_LDAP_GROUP_ATTR` | `memberOf` | `memberOf` |

Empty filters and attributes use the default for the selected mode. `KOREOGRAF_LDAP_STARTTLS=true` turns on StartTLS. `KOREOGRAF_LDAP_TLS_INSECURE=true` skips certificate checks. The service binds with `KOREOGRAF_LDAP_BIND_DN`, searches for the user, then binds with that user's password. `{username}` in the filter is escaped.

## Execution environments

An environment is an image plus a pull policy (`if_not_present`, `always`, `never`). The demo creates `quay.io/ansible/awx-ee:latest` and attaches it to no template, so the first run works without an image pull.

In the container the runner mounts the project read-only at `/runner/project`, the work directory at `/runner/work`, and the cache at `/cache`. `--env-file` points at a file on the host. The image gets a cleaned environment: `PATH` is local only, `HOME` is the work directory, `LANG=C.UTF-8`, and the `KOREOGRAF_*` variables. The server environment is not inherited.

## Workflows

The editor draws nodes and edges. An edge applies on `success`, `failure`, or `always`. A node starts on `any` as soon as one matching edge is ready, or on `all` when every matching incoming edge is satisfied. A failure counts as handled when a `failure` or `always` edge leaves that node. The API rejects cycles. An empty graph can be saved and cannot be started.

Cancel sets the job to `canceled`. The workflow treats that like a failure so a failure edge can continue.

## Webhooks

`POST /hooks/{id}/{token}` is public. The token is the authentication. Alertmanager reads `alerts[].labels.alertname` and `status`. Grafana reads the older format with `state` and `ruleName`. Otherwise an `alertname` field is enough. `alerting` becomes `firing`, and `ok` becomes `resolved`.

A match on alert name and status (`firing`, `resolved`, `any`) starts the template or the workflow. At most 10 starts per call. A valid token answers 200 even when no alert matches. A wrong token is 401. An unknown id is 404.

Variables are built in this order: template extra variables, then the hook's variables, then the event (`alertname`, `status`, `instance`, `summary`, `event`). A hook `heal_action` does not override a label of the same name.

Alertmanager:

```yaml
receivers:
  - name: koreograf
    webhook_configs:
      - url: http://koreograf.example.com/hooks/HOOK_ID/TOKEN
        send_resolved: true
```

Grafana, contact point Webhook, same URL. In the UI an operator creates the hook and sees the finished URL. `Test` (German: `Test-Alert`) sends an Alertmanager body with the stored alert name.

## Tools

The registry lives in `runner::tools()`. Another CLI tool needs an entry there and a branch in `src/wrapper.sh`. The three built-in tools:

| Id | Run |
| --- | --- |
| `ansible` | `ansible-playbook`, Galaxy into the team cache |
| `opentofu` | `tofu init`, then `plan` or `apply` |
| `shell` | a script from the project, or `spec.script` written as `inline.sh` |

Credential kinds are `ssh`, `vault`, `env`, and `netbox`. They are stored with AES-GCM and the API never returns them. `ssh` and `vault` land in the job directory with mode `0600`. A NetBox token stays on the credential.

## Playbooks and inventory sources

`GET /api/projects/{id}/contents` reads an Ansible project and lists playbooks. A file counts when it ends in `.yml` or `.yaml`, the first content line is a list, and `hosts:` appears somewhere. `requirements.yml`, `galaxy.yml`, and role directories such as `tasks`, `handlers`, and `group_vars` stay out. A git project has to be synced first. The template offers the files that were found. Another path can still be typed by hand.

Inventory files are named `hosts` or `inventory`, or they live in `inventory/` and `inventories/`. A relative `inventory=` entry under `[defaults]` in `ansible.cfg` is included. `POST /api/inventories/import` stores the file text. INI and the usual YAML inventory show up in the host list. Ansible receives the same text.

A file with `plugin:` is not executed. `plugin: netbox.netbox.nb_inventory` (also `plugin: netbox`) has a NetBox source. The controller fetches devices and, when asked, virtual machines, groups them like the plugin (`sites_berlin`, `device_roles_web`), and stores INI. The token lives on the credential, not in the source. An inventory can have several sources, for example a file from the project and NetBox. Ansible receives each source as its own file. `POST /api/inventories/{id}/sync` refreshes every external source. Other inventory plugins are listed and cannot be executed here.

An Ansible template stores the run in `spec`: `job_type` `run` or `check`, `tags`, `skip_tags`, `limit`, `diff`, `verbosity` from 0 to 4, and `forks` from 1 to 50. Check starts `ansible-playbook --check`.

## API

Session cookie `koreograf_session`, HttpOnly, SameSite=Lax, seven days. Team routes need `X-Team-Id`. Job events use the team membership stored on the job, because `EventSource` cannot set custom headers. The stream is `GET /api/jobs/{id}/events?after=`.

| Method | Path |
| --- | --- |
| POST | `/api/login`, `/api/logout`, `/api/users` |
| GET | `/api/me`, `/api/overview`, `/api/teams` |
| POST | `/api/teams` |
| PUT | `/api/teams/{id}` |
| GET, POST | `/api/teams/{id}/members` |
| DELETE | `/api/teams/{id}/members/{user_id}` |
| GET, POST | `/api/organizations` |
| GET, PUT | `/api/organizations/{id}` |
| GET, POST | `/api/projects`, `/api/inventories`, `/api/credentials`, `/api/environments` |
| GET | `/api/projects/{id}`, `/api/inventories/{id}` |
| PUT | `/api/projects/{id}`, `/api/inventories/{id}` |
| DELETE | `/api/projects/{id}`, `/api/inventories/{id}`, `/api/credentials/{id}`, `/api/environments/{id}` |
| GET | `/api/projects/{id}/contents` |
| POST | `/api/projects/{id}/sync` |
| POST | `/api/inventories/import`, `/api/inventories/netbox` |
| POST | `/api/inventories/{id}/sync` |
| POST | `/api/inventories/{id}/sources` |
| PUT, DELETE | `/api/inventories/{id}/sources/{source_id}` |
| POST | `/api/inventories/{id}/sources/{source_id}/sync` |
| GET | `/api/cache`, `/api/plugins` |
| GET, POST | `/api/templates`, `/api/workflows`, `/api/hooks`, `/api/jobs` |
| PUT, DELETE | `/api/templates/{id}`, `/api/workflows/{id}` |
| POST | `/api/templates/{id}/launch`, `/api/workflows/{id}/launch` |
| GET | `/api/jobs/{id}`, `/api/workflows/{id}`, `/api/workflow-jobs/{id}` |
| POST | `/api/jobs/{id}/cancel`, `/api/hooks/{id}/test` |
| GET | `/api/hooks/{id}/deliveries` |
| DELETE | `/api/hooks/{id}` |
| POST | `/hooks/{id}/{token}` |
| GET | `/healthz`, `/metrics` |
| GET, POST | `/api/admin/backup` |

`PUT /api/projects/{id}` sets `organization_id` and `team_ids`. An empty list gives the project to every team in the organization. `GET /api/inventories/{id}` returns `hosts`, `host_count`, `source_kind`, and `source`. Groups with `:vars` or `:children` are not hosts. A git sync that fails answers with the last lines from git. Allowed addresses are `http(s)`, `ssh`, `git@`, and `file`. `contents` answers with `playbooks` and `inventories`. `import` takes a static file. `netbox` expects `name`, `api_endpoint`, `token`, and optionally `group_by`, `query_filters`, `validate_certs`, and `virtual_machines`. The fetch happens before the save.

Launches answer 202 and need a JSON body, at least `{}`. `extra_vars` is an object.

## Security

The prototype is meant for a trusted network. The Docker socket is root on the host. `data/secret.key` decrypts every credential. Copying that file, the SQLite database, or `koreograf-backup.zip` exposes the credentials. `KOREOGRAF_LDAP_TLS_INSECURE` and an open port without TLS do not belong on the internet.

## Demo

1. Sign in as `admin`. The password comes from the environment or from `data/initial-admin-password`.
2. Select the team `Plattform`.
3. On the overview, press `Test alert` (German: `Test-Alert`).
4. The workflow `Self-Heal Ablauf` starts. The health check is shell. On success the Ansible role `incident_report` runs on the controller and writes a report into the job directory. On failure, `escalate.sh` runs.
5. The role deletes only file names that the hook lists in `incident_report_cleanup_files`, and only under the job work directory. The demo hook sets `heal_action` to `cleanup-tmp` and the file `stale.tmp`.

A second Ansible run shows the role coming from the cache in the log.
