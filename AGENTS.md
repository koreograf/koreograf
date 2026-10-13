# Agents

Koreograf is a Rust prototype: one binary with an HTTP API, a browser UI, and a job runner. Read `README.md` before changing behavior. This file is for coding agents working in the repository.

## Language

- `README.md`, code identifiers, and commit messages are English.
- Commit messages follow Conventional Commits. The subject is imperative, starts with a lowercase letter, has no trailing period, and stays within 72 characters. commitlint enforces that on pull requests.
- The browser UI defaults to German. English is the `EN` map in `web/app.js`. The German string is the key. New visible copy goes through `t()` or `h()`, and the same German string gets an `EN` entry. Smoke tests look for those German literals in `/app.js`.
- A new UI language is one object in `LANGUAGES` (`id`, `code`, `name`, `country`) plus the `EN` strings when `id` is not `de`. Add that country's SVG to `FLAG_SVG`; a missing flag shows the country code. The sidebar and the sign-in page share the list. English uses the UK flag.
- API errors are German sentences in `{ "error": "..." }`. A static sentence that the UI shows should also be an `EN` key. Errors built with `format!` stay German.
- Comments in `.env.example` and the Helm chart are still German. Leave them unless asked to translate them.

## Build and test

- `rust-toolchain.toml` pins Rust 1.96.0 with rustfmt and Clippy. `Cargo.toml` `rust-version` is 1.89. CI checks 1.89 with `cargo check --locked` and runs the full suite on 1.96.
- For a behavior change, before committing:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
node --check web/app.js
```

- `#![forbid(unsafe_code)]` is set. Do not add `unsafe`. Do not use `unwrap` or `expect` on library paths. Do not enable `missing_docs`.
- The test suite must not require Ansible, Docker, or Helm. Smoke tests in `tests/smoke.rs` boot a temporary data directory and call the Axum router in process. They do not render or click the UI.
- A docs-only change does not need a new test. Say that in the commit or the review note.
- Do not weaken, skip, or delete a failing test to make the suite green.

## Data and secrets

- `data/` is gitignored. It holds `koreograf.db`, `secret.key`, `initial-admin-password`, the cache, git checkouts, and job directories.
- Do not commit or print the admin password, `secret.key`, credential material, tokens, or `extra_vars`.
- Do not log those values. Backup and restore already treat the zip as secret.

## Product boundaries

- SQLite is the only database. `postgres://` and `postgresql://` are rejected on purpose in `config::database_choice`. Do not add a PostgreSQL driver unless asked.
- Team roles stay `viewer`, `operator`, and `owner`. System admin is `users.is_admin`. Do not add per-object roles unless asked. `/api/users` is admin-only. `/api/me` is the signed-in account, including real memberships. LDAP memberships and the LDAP email stay directory-owned.
- This is not an AWX drop-in. Do not add surveys, schedules, Receptor, or high availability unless asked.
- Credential kinds are the built-in list in `credentials::KINDS`. One credential per template. Do not add external secret plugins or a custom type designer unless asked. An `scm` credential applies to the job, not to project sync.
- Demo playbooks must not delete arbitrary files on remote hosts. Cleanup stays under the job work directory and only for names the hook lists.
- A new CLI tool needs an entry in `runner::tools()` and a branch in `src/wrapper.sh`.

## Locks

- Do not hold the database mutex across `.await`.
- Backup and restore take `AppState.maintenance` before the database lock. Take the cipher lock only for the encrypt or decrypt call. Do not lock the cipher and the database in opposite orders.

## CI

`.github/workflows/ci.yml` runs formatting, Clippy, tests, rustdoc, `node --check`, Shellcheck, Hadolint, pre-commit, `helm template`, and `docker build`. It does not push an image and it does not deploy. Keep new checks in that shape unless asked to publish or roll out.
