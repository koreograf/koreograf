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

Every push and every pull request starts GitHub Actions. The runs check formatting and Clippy, the Rust tests, the Rust docs, the syntax of `web/app.js`, Shellcheck, Hadolint, the pre-commit hooks, the Helm chart (`helm lint`, `helm template`, and kubeconform), `renovate.json`, and the pull request title. The `msrv` job runs `cargo +1.89.0 check`, because `rust-toolchain.toml` overrides the rustup default. That version is the minimum in `Cargo.toml`. Pull requests and pushes build the image with Rust 1.96, like the Dockerfile, and do not push it. A tag `v*` publishes `ghcr.io/koreograf/koreograf` for `linux/amd64`. Nothing is deployed.

## Releases

Do not edit versions by hand. release-please reads Conventional Commits on `main` and opens one release pull request. Merging that pull request tags the version and publishes the GitHub release. The workflow is `.github/workflows/release-please.yml`.

`feat:` is a feature. `fix:` and `perf:` are fixes. `feat!:` or a `BREAKING CHANGE:` footer is a breaking change. `docs:`, `chore:`, `ci:`, `test:`, `refactor:`, and `style:` do not release. Squash merges must use the pull request title as the commit. The title check is `.github/workflows/pr-title.yml`. It runs when a pull request is opened, edited, synchronized, or reopened, so an edited title does not keep a stale green check and does not re-run the rest of CI. Make `title` a required check. CI rejects a title that is not a Conventional Commit, that starts with an uppercase letter, that ends with a period, or that is longer than 72 characters.

The manifest starts at `0.1.0-alpha.1`. `versioning` is `prerelease` and `prerelease` is true, so the line stays `0.1.0-alpha.N`. The next release-worthy commit, including a `fix:`, becomes `0.1.0-alpha.2`, not `0.1.1`. On this alpha line the patch is 0, so `feat:` and `feat!:` also only increment N.

`bump-patch-for-minor-pre-major` is false, so after the alpha line a `feat:` bumps the minor (`0.1.0` to `0.2.0`). `bump-minor-pre-major` is true, so before 1.0.0 a `feat!:` bumps the minor, not the major. From 1.0.0 onward a `feat!:` bumps the major.

To leave the alpha line, edit `release-please-config.json` for one release: set `release-as` to `0.1.0`, set `prerelease` to false, and set `versioning` to `default`. Merge the release pull request release-please opens for `0.1.0`. Then delete `release-as`, or the following release would be forced to `0.1.0` again.

The Rust release type updates `Cargo.toml` and `Cargo.lock`. `extra-files` updates `Cargo.toml` again through the toml updater, `Cargo.lock` through the generic updater (the `x-release-please-version` comment on the package version), and both `version` and `appVersion` in `deploy/helm/koreograf/Chart.yaml` through the yaml updater. `cargo update` rewrites the lockfile and drops that comment; the Rust release type still finds the package by name. Put the comment back when you want the generic updater to see the line. New changelog sections are inserted above the existing `0.1.0-alpha.1` entry. The generic updater also rewrites lines marked `x-release-please-version` in `deploy/helm/koreograf/values.yaml` (`image.tag`), `docker-compose.yml` (the `image` line), `README.md`, and `docs/installation.md`. Renovate does not change that image tag. The tag is `v` plus the version, for example `v0.1.0-alpha.2`.

The release workflow uses the default `GITHUB_TOKEN`. A tag that token pushes does not start `.github/workflows/release.yml`. The release-please workflow therefore calls the image job itself when `release_created` is true. A tag you push yourself still starts `release.yml` on the `v*` trigger. The workflow token starts with no permissions. The release-please job grants `contents: write` and `pull-requests: write`. The image job grants `contents: read` and `packages: write` and does not receive other repository secrets. A concurrency group named `release-please` queues a second push instead of cancelling a release that is already opening the pull request or the tag. On the first run, confirm that the label `autorelease: pending` exists and is applied to the release pull request. release-please uses that label to find the merged release pull request. If the label is missing, add `issues: write` to the release-please job. In the repository settings, allow GitHub Actions to create pull requests, and set squash merges to Default to pull request title. Otherwise a one-commit pull request keeps the commit subject, and the title check never sees that subject.

## Dependency updates

Renovate opens the dependency pull requests. The config is `renovate.json`. CI runs `renovate-config-validator` from Renovate 44.172.0. A custom manager keeps that pin current.

The repo owner installs the Renovate GitHub App for `koreograf/koreograf` only. The ruleset `main-protect` requires `review/reviewer`, `title`, `commits`, `rust`, `msrv`, `pre-commit`, `web`, `shell`, `chart`, `image`, and `renovate`. `commits` and `title` run only on pull requests. GitHub auto-merge is on. Renovate enables it for `patch` and `pin` updates of GitHub Actions, npm dev dependencies, pre-commit hooks, the demo Galaxy collection, and demo OpenTofu providers. The merge method is squash, because `main` requires a linear history. A grouped pull request automerges only when every update in it is allowed to, so a minor in the Actions group waits. Digest updates of Actions and pre-commit hooks wait for a person: a digest change on a pinned tag means the tag moved. That includes `dtolnay/rust-toolchain` at `v1`. The tag moves on purpose, and the new commit still needs a person, because `release.yml` runs those actions with `packages: write`. Runtime updates wait too: Cargo dependencies, Docker base images, OpenTofu in the image, and the Rust toolchain group. Shellcheck, Hadolint, the Renovate validator pin, and every major wait as well. Vulnerability alerts run only when Dependabot alerts are enabled for the repository. Renovate labels them `security` and does not wait for Monday. npm, Cargo, and the validator pin wait three days after a release is published. Every Renovate pull request is labeled `dependencies`.

Renovate runs before 6am on Monday, Europe/Berlin, and keeps at most five update pull requests open.

GitHub Actions share one pull request. Pre-commit hooks share one. The pre-commit manager is enabled here, because Renovate leaves it off by default. npm dev dependencies share one. Cargo patch and minor updates are grouped, and each Cargo major is its own pull request.

Runtime updates use `fix(deps)`: Cargo dependencies, the Docker base images, and the OpenTofu binary in the image. The Rust toolchain group is `fix(deps)` as well, so `rust-toolchain.toml`, `FROM rust` in the Dockerfile, and the 1.96.0 `toolchain` input share one title. A weekly digest pin of `debian:bookworm-slim` is also `fix(deps)`, so release-please proposes a release for that pin. Dev and CI updates use `chore(deps)`: Actions, npm, pre-commit, the demo Galaxy collection, and demo OpenTofu providers. A major update is always `chore(deps)`. It is never titled `feat!:` and it is never merged by the bot.

Actions are pinned to a commit SHA with a version comment. `dtolnay/rust-toolchain` is pinned to the `v1` commit of the action, and the Rust version is the `toolchain` input. Pinning the old `@1.96.0` ref to a SHA would install the wrong toolchain, because that action treats the git ref as the Rust version. Renovate bumps `rust-toolchain.toml`, `FROM rust` in the Dockerfile, and the 1.96.0 `toolchain` input in one pull request. The 1.89.0 input stays put. `rust-version` in `Cargo.toml` stays `1.89`.

`ignorePaths` is only `**/node_modules/**`. That replaces the recommended preset, which would skip `examples/`. `ARG TOFU_VERSION` tracks OpenTofu GitHub releases. `TOFU_SHA256_AMD64` and `TOFU_SHA256_ARM64` do not. Copy both lines from that release's `SHA256SUMS`, or the image build rejects the zip. Shellcheck, Hadolint, and the Renovate validator pin in CI are custom managers. Helm's version is the `azure/setup-helm` input, which Renovate already understands. The `required_version` constraint in `examples/demo/tofu/main.tf` is disabled, because Renovate would read it as HashiCorp Terraform. Provider requirements under the terraform manager still update. The product image tag in the Helm values and in `docker-compose.yml` is ignored, so Renovate does not race a release. The Dependency Dashboard lists every manager, including the OpenTofu one.
