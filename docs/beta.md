# Beta contract

`0.1.0-beta.1` is the first beta. From that tag onward this page states what an installation may rely on, and what the beta does not promise. The alpha line had no such contract, so an alpha build may have changed any of it.

## What stays stable

- **HTTP API.** `/api/v1` is the stable path. A route there keeps its request and response shape for the rest of the `0.1.0` line. `/api` answers the same requests as an alias, is deprecated, and is removed in `1.0.0`. Webhook ingest `/hooks/{id}/{token}`, `/mcp`, `/metrics`, and `/healthz` stay outside the API prefix. See [API](api.md#versioning).
- **Database schema.** Upgrades are forward-only. A beta or later build migrates a database or backup from the last alpha release and from every earlier beta. A database or backup whose schema is newer than the running build is refused before anything is replaced, so an older binary cannot damage newer data. A downgrade is not supported; restore a backup instead. See [Database and backup](backup-restore.md#schema-versions-and-upgrades).
- **Environment variables.** Every name in [Configuration](configuration.md#environment-variables) keeps its meaning and its default. A name that has to change is listed under [Breaking changes](#breaking-changes) before the release that drops it.
- **Helm values.** Every value in `deploy/helm/koreograf/values.yaml` keeps its meaning. A renamed or removed value is listed under [Breaking changes](#breaking-changes) in the same way.

A change that breaks one of those points carries a `feat!:` commit or a `BREAKING CHANGE:` footer, so release-please marks the release and the changelog names it.

## Out of scope

These stay out of the beta contract. Changing one of them needs a new statement on this page, not only a changelog entry.

- **Trusted network only.** There is no hardening for the public internet and no access control beyond a team and its roles. Put TLS and an access control layer in front of the UI when other machines reach it. See [Security](security.md).
- **No high availability.** There is no leader election, no shared queue, and no failover. One process runs the jobs, and the counters in `/metrics` live in that process.
- **One replica.** `replicaCount` stays `1` in the Helm chart. A second pod on the same SQLite file loses writes. PostgreSQL is a second backend, not a way to run more than one pod.

## Breaking changes

None yet. A beta release that changes a point above is listed here with the release, what changed, and what an operator has to do.
