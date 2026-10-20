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

Every push and every pull request starts GitHub Actions. The runs check formatting and Clippy, the Rust tests, the Rust docs, the syntax of `web/app.js`, Shellcheck, Hadolint, the pre-commit hooks, the Helm chart (`helm template`), and `renovate.json`. The `msrv` job runs `cargo +1.89.0 check`, because `rust-toolchain.toml` overrides the rustup default. That version is the minimum in `Cargo.toml`. A push to `main` builds the image with Rust 1.96, like the Dockerfile. A tag `v*` publishes `ghcr.io/koreograf/koreograf` for `linux/amd64`. Nothing is deployed.

## Dependency updates

Renovate opens the dependency pull requests. The config is `renovate.json`. CI runs `renovate-config-validator` from Renovate 44.172.0. A custom manager keeps that pin current.

The repo owner installs the Renovate GitHub App for `koreograf/koreograf` only. The ruleset `main-protect` requires `review/reviewer`, `title`, `commits`, `rust`, `msrv`, `pre-commit`, `web`, `shell`, `chart`, `image`, and `renovate`. `commits` and `title` run only on pull requests. GitHub auto-merge is on. Renovate enables it for `patch` and `pin` updates of GitHub Actions, npm dev dependencies, pre-commit hooks, the demo Galaxy collection, and demo OpenTofu providers. The merge method is squash, because `main` requires a linear history. A grouped pull request automerges only when every update in it is allowed to, so a minor in the Actions group waits. Digest updates of Actions and pre-commit hooks wait for a person: a digest change on a pinned tag means the tag moved. That includes `dtolnay/rust-toolchain` at `v1`. The tag moves on purpose, and the new commit still needs a person, because `release.yml` runs those actions with `packages: write`. Runtime updates wait too: Cargo dependencies, Docker base images, OpenTofu in the image, and the Rust toolchain group. Shellcheck, Hadolint, the Renovate validator pin, and every major wait as well. Vulnerability alerts run only when Dependabot alerts are enabled for the repository. Renovate labels them `security` and does not wait for Monday. npm, Cargo, and the validator pin wait three days after a release is published. Every Renovate pull request is labeled `dependencies`.

Renovate runs before 6am on Monday, Europe/Berlin, and keeps at most five update pull requests open.

GitHub Actions share one pull request. Pre-commit hooks share one. The pre-commit manager is enabled here, because Renovate leaves it off by default. npm dev dependencies share one. Cargo patch and minor updates are grouped, and each Cargo major is its own pull request.

Runtime updates use `fix(deps)`: Cargo dependencies, the Docker base images, and the OpenTofu binary in the image. The Rust toolchain group is `fix(deps)` as well, so `rust-toolchain.toml`, `FROM rust` in the Dockerfile, and the 1.96.0 `toolchain` input share one title. A weekly digest pin of `debian:bookworm-slim` is also `fix(deps)`. Once release-please is in use, that pin proposes a release. Dev and CI updates use `chore(deps)`: Actions, npm, pre-commit, the demo Galaxy collection, and demo OpenTofu providers. A major update is always `chore(deps)`. It is never titled `feat!:` and it is never merged by the bot.

Actions are pinned to a commit SHA with a version comment. `dtolnay/rust-toolchain` is pinned to the `v1` commit of the action, and the Rust version is the `toolchain` input. Pinning the old `@1.96.0` ref to a SHA would install the wrong toolchain, because that action treats the git ref as the Rust version. Renovate bumps `rust-toolchain.toml`, `FROM rust` in the Dockerfile, and the 1.96.0 `toolchain` input in one pull request. The 1.89.0 input stays put. `rust-version` in `Cargo.toml` stays `1.89`.

`ignorePaths` is only `**/node_modules/**`. That replaces the recommended preset, which would skip `examples/`. `ARG TOFU_VERSION` tracks OpenTofu GitHub releases. Shellcheck, Hadolint, and the Renovate validator pin in CI are custom managers. Helm's version is the `azure/setup-helm` input, which Renovate already understands. The `required_version` constraint in `examples/demo/tofu/main.tf` is disabled, because Renovate would read it as HashiCorp Terraform. Provider requirements under the terraform manager still update. The product image tag in the Helm values is ignored, so Renovate does not race a release. The Dependency Dashboard lists every manager, including the OpenTofu one.
