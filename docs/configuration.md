# Configuration

## Languages

The browser UI is German by default. The language menu sits at the bottom of the sidebar and on the sign-in page. Each entry shows a flag and a short code, for example GER and ENG. Visible copy lives in `web/i18n/de.json` and `web/i18n/en.json`. The API returns a message key in `error`, and logs stay English.

Create-lists use the same pill button, for example `+ Projekt` (English: `+ Project`). The first start on a machine is also in [Installation](installation.md#run-on-a-machine).

## Environment variables

See `.env.example`. Comments in that file are English. Copy it to `.env` before `docker compose up` when you want to override a forwarded variable.

| Variable | Default |
| --- | --- |
| `KOREOGRAF_BIND` | `0.0.0.0:8080` |
| `KOREOGRAF_DATA` | `./data` |
| `KOREOGRAF_DATA_HOST` | empty. Detected path of `KOREOGRAF_DATA` for the container daemon |
| `KOREOGRAF_WEB` | `./web` |
| `KOREOGRAF_EXAMPLES` | `./examples` |
| `KOREOGRAF_ADMIN_USER` | `admin` |
| `KOREOGRAF_ADMIN_PASSWORD` | empty, then generated once |
| `KOREOGRAF_RUNNER` | `auto` (`local` or `docker`; the image has no `local`) |
| `KOREOGRAF_CONTAINER_IMAGE` | off (the image build sets `1`) |
| `KOREOGRAF_LOCAL_RUNNER_FOR` | `admins` (`all` on a trusted network) |
| `KOREOGRAF_PROJECT_ROOTS` | empty, plus the examples directory. Comma-separated absolute directories |
| `KOREOGRAF_JOB_TIMEOUT_SECS` | `1800` (30–86400) |
| `KOREOGRAF_JOB_OUTPUT_LINES` | `8000` (100 to 100000) |
| `KOREOGRAF_SEED_ENVIRONMENTS` | on. `0`, `false`, `no`, or `off` skips the curated default environments |
| `KOREOGRAF_SECRET_KEY` | empty, then `data/secret.key`. When set, must be 64 hex characters or startup fails |
| `KOREOGRAF_LOG_FORMAT` | empty, text on stdout. `json` for Promtail |
| `KOREOGRAF_HSTS` | false. `true` sends `Strict-Transport-Security` |
| `KOREOGRAF_COOKIE_SECURE` | `auto`. `true` sets `Secure` on the session cookie |
| `KOREOGRAF_TRUSTED_PROXY` | false. When true, `auto` reads `X-Forwarded-Proto` |
| `KOREOGRAF_PUBLIC_URL` | Empty. Public origin(s) for browser writes behind a proxy, for example `https://koreograf.example.com`. Comma- or space-separated. Empty falls back to the request `Host` and port |
| `KOREOGRAF_DATABASE_URL` | empty, SQLite in `data/koreograf.db`. `postgres://` or `postgresql://` selects PostgreSQL |
| `KOREOGRAF_DATABASE_PASSWORD_FILE` | empty. File with the PostgreSQL password; it overrides a password in the URL |
| `KOREOGRAF_DATABASE_CA_FILE` | empty. PEM file with the CA for PostgreSQL TLS |
| `KOREOGRAF_RESTORE_FROM` | empty. A mounted zip restored before the port opens when the data directory is empty |
| `KOREOGRAF_IMPORT_FROM` | empty. An old data directory copied on first start when the data directory is empty |
| `KOREOGRAF_PATH_MAP` | empty. Comma-separated `OLD=NEW` absolute prefixes applied once during an import |
| `KOREOGRAF_IMPORT_CACHE` | off. With an import, copy the rest of the cache as well |
| `KOREOGRAF_LDAP_BIND_DN` | empty. Required when `KOREOGRAF_LDAP_URL` is set |
| `KOREOGRAF_LDAP_BIND_PASSWORD` | empty. Required when `KOREOGRAF_LDAP_URL` is set |
| `KOREOGRAF_LDAP_USER_BASE` | empty. Required when `KOREOGRAF_LDAP_URL` is set |
| `KOREOGRAF_LDAP_GROUP_BASE` | empty |
| `KOREOGRAF_LDAP_TLS_INSECURE` | false. Skips certificate checks and writes a warning |

## PostgreSQL

SQLite stays the default. A `postgres://` or `postgresql://` URL selects the experimental PostgreSQL backend. This slice bootstraps the schema and serves sign-in, sessions, `/healthz`, `/api/version`, `/api/me` with API tokens, users, credentials, organizations, teams, projects, inventories with their sources, templates, execution environments, schedules, hooks, workflows, hook delivery records, project sync status, job execution with template and workflow launch, scheduled runs and matching hook dispatch, the job list with events and output, and the overview and dashboard. Backup and restore through the UI and the API work on both engines; the archive stays one SQLite file plus the key, so an archive from either engine restores into either engine.

```bash
KOREOGRAF_DATABASE_URL='postgresql://koreograf@db.example:5432/koreograf?sslmode=require'
```

A local or Compose PostgreSQL without TLS needs `sslmode=disable`; the default `require` fails the handshake against it:

```bash
KOREOGRAF_DATABASE_URL='postgresql://koreograf@127.0.0.1:5432/koreograf?sslmode=disable'
```

`sslmode=disable` sends the password and the data unencrypted. Use it only on localhost or a private Compose network.

The URL carries the host, port, database, and user. An empty database path uses the user name. `sslmode` is `disable`, `prefer`, `require` (the default), `verify-ca`, or `verify-full`. `require` and `verify-ca` behave like `verify-full`, which verifies the certificate chain and the host name; that is stricter than libpq. For an internal CA, add `sslrootcert=/etc/koreograf/postgres-ca.pem` or set `KOREOGRAF_DATABASE_CA_FILE`. The CA file must be PEM. Without a CA file, Koreograf uses the system roots.

Keep the password out of the URL with `KOREOGRAF_DATABASE_PASSWORD_FILE`, for example a Docker or Kubernetes secret. The file is read once at startup and a trailing newline is ignored. The password is never written to logs, API responses, `koreograf doctor` output, or error messages. When both are present, the file wins.

The data directory still holds `secret.key`, the cache, git checkouts, and job directories. To move an existing SQLite installation into this backend, stop the server and run `koreograf db migrate --to 'postgresql://koreograf@db.example:5432/koreograf?sslmode=require'`. The command needs an empty target, keeps IDs, resets sequences, and checks every row count; it prints the URL to set and never prints the password. It refuses while a server still holds the data directory. Backups stay portable, so the same archive restores into SQLite or PostgreSQL. See [Database and backup](backup-restore.md#copy-sqlite-data-to-postgresql).

Directory sign-in is described in [Organizations, teams, LDAP, and Active Directory](authentication.md). Company CAs are mounted at `/etc/koreograf/ca`; see [Installation](installation.md#company-certificate-authorities).

Startup runs the runner preflight, logs one block, and keeps the report for the overview. `koreograf doctor` prints the same block and exits with code 2 when the configured mode lacks a tool or runtime. The check covers `ansible-playbook` 2.14 or newer, `tofu` 1.6 or newer, `sh`, `git`, `ssh`, and a Docker or Podman CLI whose daemon answers. The tools get two seconds; the container CLI gets ten. `GET /api/runner/preflight` is administrator-only, runs the check fresh, refreshes the cached report, and returns binary paths and versions but never an environment variable or the value of `PATH`. The **Overview** page and `GET /api/overview` use the cached report, so **Check again** under **Settings → Jobs** refreshes it after a change to the machine or runner mode. `KOREOGRAF_CONTAINER_IMAGE=1` reduces the check to the container CLI and socket. The image build sets it.

`auto` looks for `docker` and `podman`. That check stops after ten seconds and then treats the runtime as unreachable. Without a runtime, a job still runs on the controller when its tool exists locally; the log says to mount the Docker socket or to set the runner mode to `docker` so the job fails instead. Before any work, a job in `auto` fails when its own tool is missing locally and no runtime answers. The failure line is the same text as the overview notice. `local` starts the job and the missing tool fails the run in the job log; it ignores execution environments and writes that into the job log. `docker` fails a run that has an image set when no runtime is reachable. Templates without an environment always run locally. The overview and `GET /api/runner` name the runtime that answered, or that none did. The template overlay reads `/api/runner` and warns while an environment is set and no runtime is reachable.

The container image has no local tool set and no local runner. There `auto` uses the container runtime, a saved `local` mode becomes `docker`, and a job without a reachable `/var/run/docker.sock` fails at start with a line that names the socket and links to [Installation](installation.md).

`KOREOGRAF_DATA_HOST` is the absolute path the container daemon uses for `KOREOGRAF_DATA`. Leave it empty. Koreograf reads the mounts of its own container and uses that path. Set it only when the lookup is wrong. A relative path is ignored. A path the daemon cannot see fails the job before the container starts, and the daemon does not create `jobs/` or `cache/` on its host.

`KOREOGRAF_COOKIE_SECURE` is `true`, `false`, or `auto`. The process speaks HTTP, so `auto` leaves `Secure` off unless `KOREOGRAF_TRUSTED_PROXY` is true and the proxy sends `X-Forwarded-Proto: https`. Set `true` when TLS ends in front of the process. `false` never sets the flag. An unknown value is treated as `auto` and written to the log. The same flag is applied when the cookie is cleared.

`KOREOGRAF_LOCAL_RUNNER_FOR` is `admins` or `all`. The default is `admins`. An operator, an anonymous webhook, and a workflow started by either of those cannot start a job on the controller. An administrator can, and a workflow they start keeps that attribution. The workflow and the inventory still have to have been saved by an administrator. A workflow or inventory an operator edited does not start a local job until an administrator saves it again. The Test button on a webhook runs as the signed-in user. `all` is for a trusted network where those jobs must run locally. The value comes from the environment, not from Settings. An install that already ran those jobs locally sets `KOREOGRAF_LOCAL_RUNNER_FOR=all`. An operator path project also needs that directory listed in `KOREOGRAF_PROJECT_ROOTS`. An unknown value is treated as `admins` and written to the log.

`KOREOGRAF_PROJECT_ROOTS` lists absolute directories separated by commas. The directory in `KOREOGRAF_EXAMPLES` is always an allowed root, so the seeded Demo project runs on a default install. An operator can add a path project or a `file://` Git URL when the directory is under one of these roots. An administrator can save a path outside them. At run time the canonical directory must still be under an allowed root, including a path that was accepted when it was saved, so that saved path does not run until its directory is listed. `/`, `/etc`, `/proc`, `/sys`, `/dev`, `/run`, `/boot`, `/root`, `/var/run`, and the data directory are rejected for everyone. A `file://` URL is accepted for an empty host or `localhost`. Git's file protocol stays off for every other URL.

`KOREOGRAF_JOB_OUTPUT_LINES` is the number of lines kept for each of stdout and stderr. The process clamps a value outside 100 to 100000. The job keeps the first three quarters and the last quarter. Download and the job page are described in the [README](../README.md#job-output).

What the database file holds, and which engine serves the backup, is in [Database and backup](backup-restore.md). Log fields are in [Logs and Prometheus](observability.md). Directory, proxy, runner, timeout, and output-line values saved in the UI are in [Organizations, teams, LDAP, and Active Directory](authentication.md#saved-settings).

## Docker Compose

`docker-compose.yml` fixes four values inside the container. A different value in `.env` or in the shell does not change them.

| Variable | Value in the container |
| --- | --- |
| `KOREOGRAF_BIND` | `0.0.0.0:8080` |
| `KOREOGRAF_DATA` | `/var/lib/koreograf` |
| `KOREOGRAF_WEB` | `/opt/koreograf/web` |
| `KOREOGRAF_EXAMPLES` | `/opt/koreograf/examples` |

Compose forwards the other names from `.env` into the container. That includes `KOREOGRAF_DATA_HOST`, `KOREOGRAF_LOG_FORMAT`, `KOREOGRAF_HSTS`, `KOREOGRAF_COOKIE_SECURE`, `KOREOGRAF_TRUSTED_PROXY`, `KOREOGRAF_DATABASE_URL`, `KOREOGRAF_RESTORE_FROM`, `KOREOGRAF_IMPORT_FROM`, `KOREOGRAF_PATH_MAP`, `KOREOGRAF_IMPORT_CACHE`, `KOREOGRAF_SEED_ENVIRONMENTS`, `HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`, and the lowercase `http_proxy`, `https_proxy`, and `no_proxy`. Log format and the database URL are passed through even when they are empty. A proxy name is passed only when `.env` or the shell sets it. An unset proxy is left out of the container. An empty `https_proxy` would turn off `HTTPS_PROXY` for git and for Python, so do not set a proxy name to an empty string.

Compose also reads the shell. A proxy exported in the shell is forwarded even when the line in `.env` is commented out. Leave those variables unset in the shell when the container should not use them. The container does not copy a proxy that is configured only on the Docker daemon.
