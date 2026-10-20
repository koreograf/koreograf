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

What the database file holds, and why a `postgres://` URL stops startup, is in [Database and backup](backup-restore.md). Log fields are in [Logs and Prometheus](observability.md). Directory, proxy, runner, and timeout values saved in the UI are in [Organizations, teams, LDAP, and Active Directory](authentication.md#saved-settings).

## Docker Compose

`docker-compose.yml` fixes four values inside the container. A different value in `.env` or in the shell does not change them.

| Variable | Value in the container |
| --- | --- |
| `KOREOGRAF_BIND` | `0.0.0.0:8080` |
| `KOREOGRAF_DATA` | `/var/lib/koreograf` |
| `KOREOGRAF_WEB` | `/opt/koreograf/web` |
| `KOREOGRAF_EXAMPLES` | `/opt/koreograf/examples` |

Compose forwards the other names from `.env` into the container. That includes `KOREOGRAF_LOG_FORMAT`, `KOREOGRAF_DATABASE_URL`, `HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`, and the lowercase `http_proxy`, `https_proxy`, and `no_proxy`. An empty value is forwarded as empty. The process treats an empty proxy variable as unset, so a blank line does not enable a proxy.

Compose also reads the shell. A proxy exported in the shell is substituted even when the line in `.env` is commented out. Leave those variables unset in the shell when the container should not use them. The container does not copy a proxy that is configured only on the Docker daemon.
