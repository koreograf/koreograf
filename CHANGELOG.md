# Changelog

## [0.1.0-alpha.2](https://github.com/koreograf/koreograf/compare/v0.1.0-alpha.1...v0.1.0-alpha.2) (2026-10-22)


### ⚠ BREAKING CHANGES

* error responses use a message key in error, plus optional vars, instead of a German sentence.

### Features

* let a template skip the git update on launch ([b22c75a](https://github.com/koreograf/koreograf/commit/b22c75ad7286128e4ac9529a4b1e1f0162c7438f))
* move visible copy into message catalogs ([3b2a6d0](https://github.com/koreograf/koreograf/commit/3b2a6d0f7fbbf78746c4c16a3a045c255e1a91a3))


### Bug Fixes

* forward log, database, and proxy settings in Compose ([#41](https://github.com/koreograf/koreograf/issues/41)) ([2995677](https://github.com/koreograf/koreograf/commit/29956778a99e06ecbea2e0c9e654ef68cadd6f47))
* keep demo role defaults when alert vars are missing ([#45](https://github.com/koreograf/koreograf/issues/45)) ([3c0fc3d](https://github.com/koreograf/koreograf/commit/3c0fc3df8876d176cf23159aac096c624efface8))
* return a catalog error when the JSON body is missing ([5e54e45](https://github.com/koreograf/koreograf/commit/5e54e45b09d03323916320fd2a2847721fb365c4)), closes [#30](https://github.com/koreograf/koreograf/issues/30)
* run local jobs from a relative data directory ([d229084](https://github.com/koreograf/koreograf/commit/d229084c0d2000fae085995efd78102d36918404)), closes [#36](https://github.com/koreograf/koreograf/issues/36) [#37](https://github.com/koreograf/koreograf/issues/37) [#17](https://github.com/koreograf/koreograf/issues/17)
* search LDAP groups with the service account ([#40](https://github.com/koreograf/koreograf/issues/40)) ([0d61ea1](https://github.com/koreograf/koreograf/commit/0d61ea19da5d72189980ef48c931028c563d4815)), closes [#20](https://github.com/koreograf/koreograf/issues/20) [#28](https://github.com/koreograf/koreograf/issues/28) [#51](https://github.com/koreograf/koreograf/issues/51)
* stop a canceled job by its process group ([#39](https://github.com/koreograf/koreograf/issues/39)) ([9197054](https://github.com/koreograf/koreograf/commit/9197054e385462454e66e9a6f068b42412cddd1f))

## 0.1.0-alpha.1

First public alpha. One Rust binary serves the API, the German UI, and the job runner. English is available from the language menu.

This release can run Ansible, OpenTofu, and shell jobs for teams in organizations. It stores credentials, inventories (including a project file and NetBox), and projects. Execution environments are global, per organization, or private to one team. The Galaxy, role, and OpenTofu cache stays under `data/cache/<team>/`. Workflows are directed graphs. Alertmanager and Grafana webhooks can start a job or a workflow. An administrator can set the directory, the HTTP proxy, the runner, and the job timeout, and can download or restore a backup of the SQLite file and `secret.key`. `GET /metrics` exposes Prometheus text.

Limits of this alpha: one SQLite file and one writer, so the Helm chart stays at one replica. There are no surveys, no schedules, no Receptor, and no high-availability setup. A `postgres://` URL stops startup. The container image is `linux/amd64`. The prototype expects a trusted network. The Docker socket is off unless you turn it on, and then it is root on the host.
