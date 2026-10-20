# Configuration

## Languages

The browser UI is German by default. The language menu sits at the bottom of the sidebar and on the sign-in page. Each entry shows a flag and a short code, for example GER and ENG. Visible copy lives in `web/i18n/de.json` and `web/i18n/en.json`. The API returns a message key in `error`, and logs stay English.

Create-lists use the same pill button, for example `+ Projekt` (English: `+ Project`). The first start on a machine is also in [Installation](installation.md#run-on-a-machine).

## Environment variables

See `.env.example`. Comments in that file are still German.

| Variable | Default |
| --- | --- |
| `KOREOGRAF_BIND` | `0.0.0.0:8080` |
| `KOREOGRAF_DATA` | `./data` |
| `KOREOGRAF_WEB` | `./web` |
| `KOREOGRAF_EXAMPLES` | `./examples` |
| `KOREOGRAF_ADMIN_USER` | `admin` |
| `KOREOGRAF_ADMIN_PASSWORD` | empty, then generated once |
| `KOREOGRAF_RUNNER` | `auto` (`local` or `docker`) |
| `KOREOGRAF_LOCAL_RUNNER_FOR` | `admins` (`all` on a trusted network) |
| `KOREOGRAF_PROJECT_ROOTS` | empty. Comma-separated absolute directories |
| `KOREOGRAF_JOB_TIMEOUT_SECS` | `1800` (30–86400) |
| `KOREOGRAF_SECRET_KEY` | 64 hex characters, otherwise `data/secret.key` |
| `KOREOGRAF_LOG_FORMAT` | empty, text on stdout. `json` for Promtail |
| `KOREOGRAF_DATABASE_URL` | empty, SQLite. `postgres://` is rejected |
| `KOREOGRAF_LDAP_BIND_DN` | empty. Required when `KOREOGRAF_LDAP_URL` is set |
| `KOREOGRAF_LDAP_BIND_PASSWORD` | empty |
| `KOREOGRAF_LDAP_USER_BASE` | empty. Required when `KOREOGRAF_LDAP_URL` is set |
| `KOREOGRAF_LDAP_GROUP_BASE` | empty |

Directory sign-in, including these four variables, is described in [Organizations, teams, LDAP, and Active Directory](authentication.md).

`auto` looks for `docker` and `podman`. If the runtime is missing, jobs run locally. `local` ignores execution environments and writes that into the job log. `docker` fails a run that has an image set when no runtime is reachable. Templates without an environment always run locally.

`KOREOGRAF_LOCAL_RUNNER_FOR` is `admins` or `all`. The default is `admins`. An operator, an anonymous webhook, and a workflow started by either of those cannot start a job on the controller. An administrator can, and a workflow they start keeps that attribution. The Test button on a webhook runs as the signed-in user. `all` is for a trusted network where those jobs must run locally. The value comes from the environment, not from Settings. An install that already ran those jobs locally sets `KOREOGRAF_LOCAL_RUNNER_FOR=all`. An operator path project also needs that directory listed in `KOREOGRAF_PROJECT_ROOTS`. An unknown value is treated as `admins` and written to the log.

`KOREOGRAF_PROJECT_ROOTS` lists absolute directories separated by commas. Empty means only an administrator can add a path project or a `file://` Git URL. An operator can add one when the directory is under a listed root. `/`, `/etc`, `/proc`, `/sys`, `/dev`, `/run`, `/boot`, `/root`, `/var/run`, and the data directory are rejected for everyone. A `file://` URL is accepted for an empty host or `localhost`. Git's file protocol stays off for every other URL.

What the database file holds, and why a `postgres://` URL stops startup, is in [Database and backup](backup-restore.md). Log fields are in [Logs and Prometheus](observability.md). Directory, proxy, runner, and timeout values saved in the UI are in [Organizations, teams, LDAP, and Active Directory](authentication.md#saved-settings).

## Docker Compose

`docker-compose.yml` fixes four values inside the container. A different value in `.env` or in the shell does not change them.

| Variable | Value in the container |
| --- | --- |
| `KOREOGRAF_BIND` | `0.0.0.0:8080` |
| `KOREOGRAF_DATA` | `/var/lib/koreograf` |
| `KOREOGRAF_WEB` | `/opt/koreograf/web` |
| `KOREOGRAF_EXAMPLES` | `/opt/koreograf/examples` |

Compose forwards the other names from `.env` into the container. That includes `KOREOGRAF_LOG_FORMAT`, `KOREOGRAF_DATABASE_URL`, `HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`, and the lowercase `http_proxy`, `https_proxy`, and `no_proxy`. Log format and the database URL are passed through even when they are empty. A proxy name is passed only when `.env` or the shell sets it. An unset proxy is left out of the container. An empty `https_proxy` would turn off `HTTPS_PROXY` for git and for Python, so do not set a proxy name to an empty string.

Compose also reads the shell. A proxy exported in the shell is forwarded even when the line in `.env` is commented out. Leave those variables unset in the shell when the container should not use them. The container does not copy a proxy that is configured only on the Docker daemon.
