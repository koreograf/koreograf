# Configuration

## Languages

The browser UI is German by default. A language menu in the sidebar and on the sign-in page lists each language with a flag and a short code, for example GER and ENG. Visible copy lives in `web/i18n/de.json` and `web/i18n/en.json`. The API returns a message key in `error`, and logs stay English.

Where the menu sits in the sidebar, and the pill labels on create-lists, is described in [Installation](installation.md#run-on-a-machine).

## Environment variables

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

What the database file holds, and why a `postgres://` URL stops startup, is in [Backup and restore](backup-restore.md). Log fields are in [Observability](observability.md). Directory, proxy, runner, and timeout values saved in the UI are in [Authentication](authentication.md#saved-settings).
