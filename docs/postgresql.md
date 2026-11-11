# PostgreSQL

SQLite stays the default. An empty `KOREOGRAF_DATABASE_URL`, or a value of `sqlite:`, keeps `data/koreograf.db`. A `postgres://` or `postgresql://` URL starts Koreograf on PostgreSQL instead. Both engines use the same schema, the same numbered migrations, the same backup archive, and the same API and browser UI.

The move from SQLite is one copy in one transaction. The SQLite file is opened read-only and never changed. To roll back, stop Koreograf, clear `KOREOGRAF_DATABASE_URL`, and start again from that file.

## Requirements

- PostgreSQL 13 or newer.
- A database and a login role that may create tables in the current schema. The role does not need `CREATEDB` or `SUPERUSER`.
- Network access from every Koreograf process to the server.
- The same `data/secret.key` as before. It stays in the data directory and is not written to PostgreSQL. Credentials copied from SQLite stay decryptable with that key.
- A backup before the move. See [Database and backup](backup-restore.md).

The data directory still holds `secret.key`, the cache, git checkouts, and job directories. Only database rows move.

## Create the database and user

Connect as a PostgreSQL administrator and create a role and an empty database. The examples use `koreograf` as both names; they may differ.

```sql
CREATE ROLE koreograf LOGIN PASSWORD 'replace-with-a-long-password';
CREATE DATABASE koreograf OWNER koreograf;
```

The owner can create tables in the `public` schema. If your server revokes that right from `PUBLIC`, grant it explicitly:

```sql
GRANT CREATE, USAGE ON SCHEMA public TO koreograf;
```

The target must be empty. The migration accepts a database that only holds the Koreograf schema with zero rows, and refuses a database with rows in any table.

## URL, TLS, and the password file

`KOREOGRAF_DATABASE_URL` carries the host, port, database, and user. An empty database path uses the user name. The default port is 5432.

```bash
KOREOGRAF_DATABASE_URL='postgresql://koreograf@db.example.com:5432/koreograf?sslmode=require'
```

`sslmode` is `disable`, `prefer`, `require` (the default), `verify-ca`, or `verify-full`. `require` and `verify-ca` behave like `verify-full`: Koreograf verifies the certificate chain and the host name. That is stricter than libpq.

For an internal CA, name the PEM file in the URL or in `KOREOGRAF_DATABASE_CA_FILE`:

```bash
KOREOGRAF_DATABASE_URL='postgresql://koreograf@db.example.com:5432/koreograf?sslmode=verify-full'
KOREOGRAF_DATABASE_CA_FILE=/etc/koreograf/postgres-ca.pem
```

Without a CA file, Koreograf uses the system roots. A local or Compose PostgreSQL without TLS needs `sslmode=disable`; the default `require` fails the handshake against it. `disable` sends the password and the data unencrypted, so use it only on localhost or a private container network.

Keep the password out of the URL. `KOREOGRAF_DATABASE_PASSWORD_FILE` names a file with the password; a trailing newline is ignored. The file wins when the URL also carries a password. The value is read once at startup and is never written to logs, `koreograf doctor` output, API responses, the audit log, or the browser UI.

```bash
KOREOGRAF_DATABASE_PASSWORD_FILE=/run/secrets/koreograf_database_password
```

A password that stays in the URL is supported. Quote the URL, and percent-encode characters that are special in a URL. A secret store that injects the URL is then the only place the password lives.

## Connection pool

Koreograf keeps a pool of PostgreSQL connections and serves requests in parallel. `KOREOGRAF_DATABASE_POOL_SIZE` sets the maximum number of open connections; the default is 8. The value must be a positive number. Connections open on demand and return to the pool after each query or transaction, so an idle process holds fewer than the maximum.

Raise the pool size when many requests wait on the database. Keep the total across every Koreograf process below the PostgreSQL connection limit, including maintenance and backup clients. A restore or database migration uses one connection for its whole transaction.

## Install types

### Docker Compose

Create the database on an existing PostgreSQL server, put the URL and the password file into `.env`, and mount the secret:

```dotenv
KOREOGRAF_DATABASE_URL='postgresql://koreograf@db.example.com:5432/koreograf?sslmode=require'
KOREOGRAF_DATABASE_PASSWORD_FILE=/run/secrets/koreograf_database_password
```

```yaml
services:
  koreograf:
    environment:
      KOREOGRAF_DATABASE_URL: ${KOREOGRAF_DATABASE_URL:-}
      KOREOGRAF_DATABASE_PASSWORD_FILE: /run/secrets/koreograf_database_password
    volumes:
      - ./postgres-password:/run/secrets/koreograf_database_password:ro
```

`docker-compose.yml` already forwards `KOREOGRAF_DATABASE_URL` from `.env`. Add the password-file variable and the mount, or place the password in the URL. For a stand-alone Compose PostgreSQL, add a service and use a private network:

```yaml
services:
  db:
    image: postgres:17
    environment:
      POSTGRES_DB: koreograf
      POSTGRES_USER: koreograf
      POSTGRES_PASSWORD_FILE: /run/secrets/postgres_password
    volumes:
      - koreograf-db:/var/lib/postgresql/data
      - ./postgres-password:/run/secrets/postgres_password:ro
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U koreograf -d koreograf"]
      interval: 5s
      timeout: 3s
      retries: 20

  koreograf:
    depends_on:
      db:
        condition: service_healthy
    environment:
      KOREOGRAF_DATABASE_URL: postgresql://koreograf@db:5432/koreograf?sslmode=disable
      KOREOGRAF_DATABASE_PASSWORD_FILE: /run/secrets/postgres_password

volumes:
  koreograf-db:
```

The `postgres:17` image and the healthcheck are examples; use the version your company supports. Keep `sslmode=disable` inside a private Compose network only.

### Binary

```bash
KOREOGRAF_DATABASE_URL='postgresql://koreograf@db.example.com:5432/koreograf?sslmode=require' \
KOREOGRAF_DATABASE_PASSWORD_FILE=/etc/koreograf/postgres-password \
./koreograf
```

Put the same names in the environment file of your service manager. A `systemd` unit can add `After=network-online.target` and, for a local server, `After=postgresql.service`.

### OS package

Edit `/etc/koreograf/koreograf.env` as root, then restart the service:

```bash
sudoedit /etc/koreograf/koreograf.env
```

```dotenv
KOREOGRAF_DATABASE_URL='postgresql://koreograf@db.example.com:5432/koreograf?sslmode=require'
KOREOGRAF_DATABASE_PASSWORD_FILE=/etc/koreograf/postgres-password
```

```bash
sudo chown root:koreograf /etc/koreograf/postgres-password
sudo chmod 0640 /etc/koreograf/postgres-password
sudo systemctl restart koreograf
```

The unit already reads `/etc/koreograf/koreograf.env`. The password file must be readable by the `koreograf` service user.

### Helm

The current chart does not render database values. Add the variables to the running Deployment from a Secret, or patch the Deployment in your own values overlay. A short working example:

```bash
kubectl create secret generic koreograf-database \
  --from-literal=KOREOGRAF_DATABASE_URL='postgresql://koreograf@db.example.com:5432/koreograf?sslmode=require' \
  --from-literal=KOREOGRAF_DATABASE_PASSWORD='replace-with-a-long-password'
```

The password in that Secret is inside the URL, so it reaches the process only through the environment. When your chart or overlay mounts a password file, prefer `KOREOGRAF_DATABASE_PASSWORD_FILE`. Set the variables with an environment overlay that keeps them across Helm upgrades; a plain `kubectl set env` is removed by the next `helm upgrade`. Keep `replicaCount` at one and one writer, as the chart comment says.

## Move an existing SQLite installation

Take a backup first, then use the CLI or the admin Settings page. Both run the same checks and the same copy. The CLI needs the server stopped; the UI does it while the server is in maintenance mode.

### 1. Take a backup

Sign in as an administrator, open **Backup**, enable **Include OpenTofu state** when jobs use OpenTofu, and download `koreograf-backup.zip`. The archive contains the database and `secret.key`. Keep it somewhere only administrators can read: it can decrypt every credential. A UI backup is also the fallback if the migration or the switch goes wrong.

### 2. Run the checks

The checks are the same in the UI and in the CLI. They do not write to the target.

| Check | Passes when |
| --- | --- |
| Source schema | `koreograf.db` uses the schema of this build |
| Target reachable | The server answers and accepts the login |
| TLS | The negotiated connection matches the requested `sslmode` |
| Server version | The server is PostgreSQL 13 or newer |
| User rights | The user may create tables in the current schema |
| Empty target | Every table in the target schema has no rows |

Run the current build once against SQLite if the source schema is older. The CLI reports that too.

### 3a. Migrate with the CLI

Stop Koreograf and run:

```bash
koreograf db migrate --to 'postgresql://koreograf@db.example.com:5432/koreograf?sslmode=require'
```

The command holds the data-directory lock and refuses while a server still uses that directory. It reads `KOREOGRAF_DATABASE_PASSWORD_FILE` and `KOREOGRAF_DATABASE_CA_FILE`, opens `koreograf.db` read-only, and copies table by table in one transaction. Primary keys, encrypted secrets, and the schema version are copied; identity sequences are reset; row counts are checked per table. On failure the transaction rolls back, the command says the target can be reused, and SQLite is untouched.

On success it prints the `KOREOGRAF_DATABASE_URL` value without the password. Continue with step 4.

### 3b. Migrate from the admin settings

Sign in as an administrator and open **Settings → Database**.

1. Enter host, port, database, user, password, TLS mode, and an optional CA file. The password is sent once for the checks and the migration. It is never stored in SQLite.
2. Choose **Run checks**. Every result is listed with passed or failed and a reason. The start button stays disabled while one check fails.
3. Choose **Start migration** after all checks pass. Koreograf enters maintenance mode: no new job starts and every user sees a banner. Running jobs must finish or be canceled before the copy starts; the UI says so. Progress and the row count per table are shown. A failure keeps SQLite and leaves maintenance mode.
4. On success the page shows the exact `.env` line for Docker Compose and the line for `/etc/koreograf/koreograf.env`. Koreograf does not rewrite either file.

The action is written to the audit log with the user, the time, and the target host. The password is not part of that entry.

### 4. Switch the URL and restart

Set `KOREOGRAF_DATABASE_URL` in the place your install reads at start: `.env` for Docker Compose, `/etc/koreograf/koreograf.env` for the OS package, the unit or shell environment for the binary, and the Deployment environment for Helm. Keep the password in `KOREOGRAF_DATABASE_PASSWORD_FILE` when you use one.

Restart Koreograf. Startup logs `engine=postgresql` and the target; it never logs the password. The migration leaves maintenance mode only after a restart on SQLite, so the old process cannot write more rows after the copy. Stop the old process before starting the new one.

### 5. Verify

- Sign in and check users, teams, projects, inventories, templates, workflows, schedules, credentials, and job history.
- Open **Backup** and download an archive once. It works on both engines.
- `curl -fsS http://127.0.0.1:8080/healthz` answers `ok (postgresql)`.
- `koreograf doctor` still prints the runner report. It does not print a database password.
- Check `data/koreograf.db` is still present and unchanged. The migration never deletes it.

### 6. Roll back

Stop Koreograf and clear `KOREOGRAF_DATABASE_URL` (or set it to `sqlite:`), then start again. Koreograf opens the untouched `data/koreograf.db`. Remove or archive the PostgreSQL database afterwards; it is no longer used. A backup from step 1 restores into either engine if the SQLite file was lost.

## Backups and restore

A UI or API backup is always one SQLite file plus `secret.key`, whichever engine is running. It restores into SQLite or PostgreSQL. There is no separate PostgreSQL dump format. Backups created before the move remain valid after it, and a restore while running on PostgreSQL writes into PostgreSQL. See [Database and backup](backup-restore.md).

## Limits

- The copy is a one-time move. There is no replication, no two-writer mode, and no automatic switch back.
- SQLite stays the default. A fresh install uses SQLite until `KOREOGRAF_DATABASE_URL` says otherwise; the Settings wizard only moves an existing SQLite installation.
- The wizard is available while the process runs on SQLite. An installation already on PostgreSQL shows only an explanation, not a second migration.
