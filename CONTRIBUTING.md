# Contributing

Koreograf is an early prototype. Issues and pull requests are welcome. Read `SECURITY.md` before sending anything that might be a vulnerability.

Visible copy lives in `web/i18n/de.json` and `web/i18n/en.json`. The API returns a message key. Logs stay English.

## Commits

Write commit messages in English. Use Conventional Commits. Keep the subject and each body line within 72 characters. On pull requests, commitlint checks the messages.

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

Every push and every pull request starts GitHub Actions. The runs check formatting and Clippy, the Rust tests, the Rust docs, the syntax of `web/app.js`, Shellcheck, Hadolint, the pre-commit hooks, the Helm chart (`helm template`), and `renovate.json`. `cargo check` also runs on Rust 1.89, the minimum version in `Cargo.toml`. A push to `main` builds the image with Rust 1.96, like the Dockerfile. A tag `v*` publishes `ghcr.io/koreograf/koreograf` for `linux/amd64`. Nothing is deployed.

## Dependency updates

Renovate opens the dependency pull requests. The repo owner installs the Renovate GitHub App for `koreograf/koreograf` only. The config is `renovate.json`. CI runs `renovate-config-validator` from Renovate 44.

Renovate runs before 6am on Monday, Europe/Berlin, and keeps at most five update pull requests open. Security alerts skip that schedule and are labeled `security`. npm and Cargo updates wait three days after a release is published. Every Renovate pull request is labeled `dependencies`.

GitHub Actions share one pull request. Pre-commit hooks share one. npm dev dependencies share one. Cargo patch and minor updates are grouped, and each Cargo major is its own pull request.

Runtime updates use `fix(deps)`: Cargo dependencies, the Docker base images, and the OpenTofu binary in the image. Dev and CI updates use `chore(deps)`: Actions, npm, pre-commit, the demo Galaxy collection, and demo OpenTofu providers. A major update is always `chore(deps)`. It is never titled `feat!:` and it is never merged by the bot.

Patch updates and digest pins of dev and CI dependencies use GitHub auto-merge (`platformAutomerge`). Auto-merge waits until the required checks are green and a review is approved, so `main` needs branch protection that requires the CI workflow and one approving review. Other updates wait for a person.

Actions are pinned to a commit SHA with a version comment. `dtolnay/rust-toolchain` is pinned to the `v1` commit of the action, and the Rust version is the `toolchain` input. Pinning the old `@1.96.0` ref to a SHA would install the wrong toolchain, because that action treats the git ref as the Rust version. Renovate bumps `rust-toolchain.toml`, `FROM rust` in the Dockerfile, and the 1.96.0 `toolchain` input in one pull request. The 1.89.0 input is the MSRV check and stays put. `rust-version` in `Cargo.toml` stays `1.89`.

`ARG TOFU_VERSION` tracks OpenTofu GitHub releases. Shellcheck and Hadolint in CI are custom managers. Helm's version is the `azure/setup-helm` input, which Renovate already understands. The product image tag in the Helm values is ignored, so Renovate does not race a release. The Dependency Dashboard lists every manager, including the OpenTofu one.
