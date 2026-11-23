# alpha.15 upgrade fixtures

Both fixtures come from the published image
`ghcr.io/koreograf/koreograf:0.1.0-alpha.15`
(`sha256:c1542c009eb14289992ed4f66f88c347723239290c2b600af959c24c738ca72e`,
built from `d25a236`, release `0.1.0-alpha.15`). The image wrote schema version
`10` (`PRAGMA user_version` on SQLite, `schema_meta.schema_version` on
PostgreSQL) and every numbered migration through `0010_inventory_vars`.

The fixtures hold the same representative data:

- users `admin` and `fixture-user` (local accounts, password hashes only)
- teams `Plattform` (seeded) and `Fixture Team` with `fixture-user` as `operator`
- inventory `Fixture inventory` with two hosts and `vars_json` rows for
  `group_vars/all.yml` and `group_vars/web.yml`
- template `Fixture check` (shell, inline script, no execution environment)
- three jobs named `Fixture check`: triggers `manual`, `hook`, and `workflow`
- host results for `web01.example.test` and `web02.example.test`, including one
  successful, one changed, and one failed result
- workflow `Fixture workflow` at version `3` with one `Fixture check` node and
  one failed workflow run with its node row
- activity entries from the API calls plus fixed `success` and `failed` rows
- API tokens `fixture-read-token` (read scope, 30-day expiry),
  `fixture-write-token` (write scope, no expiry), and
  `fixture-expired-token` (read scope, expired on `2026-01-01`)
- sessions `fixture-session-admin` and `fixture-session-user` with fixed
  fixture-only tokens and complete `id`, `created_at`, and `last_seen_at`
- hook `Fixture hook` with an accepted test delivery and an ignored delivery

No credentials were created. The fixture directories contain no `secret.key`
and no plaintext password. The stored session tokens and API token hashes are
obviously fake fixture values, and the API token secrets start with
`koro_fixture`.

## SQLite

Boot the released image against an empty, writable directory:

```bash
mkdir -p /tmp/opencode/alpha15-sqlite
docker run --rm -d --name alpha15-sqlite \
  -p 18090:8080 \
  -v /tmp/opencode/alpha15-sqlite:/var/lib/koreograf \
  -e KOREOGRAF_ADMIN_PASSWORD=fixture-admin-pass \
  ghcr.io/koreograf/koreograf:0.1.0-alpha.15
```

Sign in with `admin` / `fixture-admin-pass` on `POST /api/login`, then create
the rows through the HTTP API. Every team route needs `X-Team-Id`; the
same-origin `Origin: http://127.0.0.1:18090` header satisfies the CSRF check.
The calls were:

- `POST /api/teams` for `Fixture Team` in the seeded `Plattform` organization,
  then `POST /api/teams/{id}/members` for `fixture-user` as `operator`
- `POST /api/inventories` for `Fixture inventory`
- `POST /api/templates` for `Fixture check` with
  `{"tool":"shell","no_environment":true,"spec":{"script":"#!/bin/sh\necho fixture\n"}}`
- `POST /api/workflows` for `Fixture workflow` with one template node, then two
  `PUT /api/workflows/{id}` calls to reach version `3`
- `POST /api/hooks` for `Fixture hook` and `POST /api/hooks/{id}/test`
- `POST /api/templates/{id}/launch` for the manual job and
  `POST /api/workflows/{id}/launch` for the workflow run
- `POST /api/me/tokens` for `fixture-read-token` with `scope: "read"` and
  `expires_in_days: 30`
- a second login as `fixture-user` for the second session

The jobs fail in this image because no container runtime is mounted; the job,
workflow-node, delivery, and activity rows still exist. Stop the container and
checkpoint the WAL before copying:

```bash
docker stop alpha15-sqlite
sqlite3 /tmp/opencode/alpha15-sqlite/koreograf.db \
  'PRAGMA wal_checkpoint(TRUNCATE);'
sqlite3 /tmp/opencode/alpha15-sqlite/koreograf.db \
  ".backup '<repo>/tests/fixtures/alpha15/koreograf.db'"
```

The copy was then opened with `sqlite3` for the rows the API cannot produce.
These inserts set the fixed host results, the two extra API tokens
(`fixture-write-token` and `fixture-expired-token` with the hashes of the
`koro_fixture…` dummies), the two fixed sessions, the ignored hook delivery,
the fixed activity rows, and both `vars_json` values. The fixture was backed up
again with `sqlite3 … ".backup …"` after those inserts.

## PostgreSQL

Start PostgreSQL 17 on a Docker network so the image can reach it:

```bash
docker network create alpha15net
docker run --rm -d --name alpha15-pg --network alpha15net \
  -e POSTGRES_USER=fixture -e POSTGRES_PASSWORD=fixture \
  -e POSTGRES_DB=fixture postgres:17
```

Boot the same alpha.15 image against that database:

```bash
mkdir -p /tmp/opencode/alpha15-postgres
docker run --rm -d --name alpha15-postgres --network alpha15net \
  -p 18091:8080 \
  -v /tmp/opencode/alpha15-postgres:/var/lib/koreograf \
  -e KOREOGRAF_ADMIN_PASSWORD=fixture-admin-pass \
  -e KOREOGRAF_DATABASE_URL='postgres://fixture:fixture@alpha15-pg:5432/fixture?sslmode=disable' \
  ghcr.io/koreograf/koreograf:0.1.0-alpha.15
```

Repeat the SQLite API calls on port `18091` with the matching origin header.
Then apply the same supplementary rows with `psql` (the IDs come from the API
responses above) and dump without owners and privileges:

```bash
docker exec alpha15-pg pg_dump --no-owner --no-privileges --inserts \
  -U fixture -d fixture \
  | sed -e '/^\\restrict /d' -e '/^\\unrestrict /d' \
      -e '/^SET transaction_timeout = 0;$/d' \
  > <repo>/tests/fixtures/alpha15/postgres.sql
```

`--inserts` keeps the test loader free of `COPY` blocks. The `sed` drops
psql-only `\restrict` lines and the PostgreSQL 17 `transaction_timeout` setting
so the dump runs on every supported server version.
