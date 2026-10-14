# Contributing

Koreograf is an early prototype. Issues and pull requests are welcome. Read `SECURITY.md` before sending anything that might be a vulnerability.

Visible copy lives in `web/i18n/de.json` and `web/i18n/en.json`. The API returns a message key. Logs stay English.

## Commits

Write commit messages in English. Use Conventional Commits. Keep the subject and each body line within 72 characters.

Work on a branch (`feat/…`, `fix/…`, `docs/…`, `chore/…`, lowercase, hyphens). Do not commit directly to `main`. Merge with a fast-forward when the change is ready.

Do not use `git add -A`. Do not commit `data/`, secrets, or build output. `data/` holds the SQLite file, `secret.key`, the first admin password, the cache, git checkouts, and job directories.

## Checks

From a Rust 1.96 toolchain, with 1.89 as the minimum in `Cargo.toml`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
node --check web/app.js
```

Tests must not need Ansible, Docker, or Helm. The admin password inside a test is `test-admin-pass` and nowhere else.

GitHub Actions runs these checks, plus the Rust docs, Shellcheck, Hadolint, pre-commit, and `helm template`. A push to `main` also builds the container image. A tag `v*` publishes it. Nothing is deployed.
