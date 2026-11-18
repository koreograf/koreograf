# alpha.12 upgrade fixtures

Both fixtures come from `v0.1.0-alpha.12` (`5272a0d`, release `0.1.0-alpha.12`).
They hold the same kind of representative data:

- users `admin` and `fixture-user`
- teams `Plattform` and `Fixture Team`
- template `Fixture check` (shell) plus the demo templates
- jobs `Fixture check` from a manual launch and from the workflow
- workflows `Self-Heal Ablauf` and `Fixture workflow`
- the seeded demo project, inventory, environment, and hook

No credentials were created. The fixture directories contain no `secret.key`
and no plaintext password; only password hashes are stored.

## SQLite

Build the released binary from a worktree at the tag:

```bash
git worktree add /tmp/opencode/alpha12 v0.1.0-alpha.12
cd /tmp/opencode/alpha12
cargo build --locked
```

Boot it with a temporary data directory, then sign in as `admin` and create
the rows through the HTTP API:

```bash
KOREOGRAF_DATA=/tmp/opencode/alpha12-sqlite \
KOREOGRAF_WEB=./web \
KOREOGRAF_EXAMPLES=./examples \
KOREOGRAF_BIND=127.0.0.1:18080 \
KOREOGRAF_ADMIN_PASSWORD=fixture-admin-pass \
KOREOGRAF_LOCAL_RUNNER_FOR=all \
./target/debug/koreograf
```

`POST /api/login` with `admin`, then:

- `POST /api/users` for `fixture-user`
- `POST /api/teams` for `Fixture Team` and `POST /api/teams/{id}/members`
  for `fixture-user` as `operator`
- `POST /api/templates` for `Fixture check` on the demo project
- `POST /api/workflows` for `Fixture workflow` with one `Fixture check` node
- `POST /api/templates/{id}/launch` and `POST /api/workflows/{id}/launch`

Stop the server, checkpoint the WAL, and copy the database:

```bash
sqlite3 koreograf.db 'PRAGMA wal_checkpoint(TRUNCATE);'
sqlite3 koreograf.db ".backup '<repo>/tests/fixtures/alpha12/koreograf.db'"
```

## PostgreSQL

Start PostgreSQL 17 with a matching user and database:

```bash
docker run --rm -d --name koreograf-alpha12-pg \
  -e POSTGRES_USER=fixture -e POSTGRES_PASSWORD=fixture \
  -e POSTGRES_DB=fixture -p 55432:5432 postgres:17
```

Boot the same alpha.12 binary with the URL and repeat the API calls above:

```bash
KOREOGRAF_DATABASE_URL='postgres://fixture:fixture@127.0.0.1:55432/fixture?sslmode=disable' \
KOREOGRAF_DATA=/tmp/opencode/alpha12-postgres \
KOREOGRAF_WEB=./web \
KOREOGRAF_EXAMPLES=./examples \
KOREOGRAF_BIND=127.0.0.1:18082 \
KOREOGRAF_ADMIN_PASSWORD=fixture-admin-pass \
KOREOGRAF_LOCAL_RUNNER_FOR=all \
./target/debug/koreograf
```

Then dump it without owners and privileges:

```bash
docker exec koreograf-alpha12-pg pg_dump --no-owner --no-privileges --inserts \
  -U fixture -d fixture \
  | sed -e '/^\\restrict /d' -e '/^\\unrestrict /d' \
      -e '/^SET transaction_timeout = 0;$/d' \
  > <repo>/tests/fixtures/alpha12/postgres.sql
```

`--inserts` keeps the test loader free of `COPY` blocks. The `sed` drops
psql-only `\restrict` lines and the PostgreSQL 17 `transaction_timeout`
setting so the dump runs on every supported server version.
