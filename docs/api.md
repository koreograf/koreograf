# API

## Versioning

`/api/v1` is the stable HTTP API path during the beta. Every route below uses that prefix. The unversioned `/api` prefix answers the same requests as an alias; it is deprecated and planned for removal in `1.0.0`. Webhook ingest `/hooks/{id}/{token}`, `/mcp`, `/metrics`, and `/healthz` stay outside the API prefix and are unchanged.

`GET /api/v1/version` does not need a session. It returns the version compiled from `Cargo.toml`. The browser UI reads this route for the build line on the sign-in page and in the sidebar. Session cookie `koreograf_session`, HttpOnly, SameSite=Lax, seven days, or `Authorization: Bearer <token>` with a personal API token. When both are present, the bearer token wins. `Secure` is added when `KOREOGRAF_COOKIE_SECURE` is `true`, and when it is `auto` and a trusted proxy sends `X-Forwarded-Proto: https`. The default is `auto` with that proxy off, so a plain HTTP sign-in still receives the cookie. Team routes need `X-Team-Id`. Job events use the team membership stored on the job, because `EventSource` cannot set custom headers. The stream is `GET /api/v1/jobs/{id}/events?after=`.

`POST /mcp` serves Streamable HTTP MCP on the same bind address as the API. It needs `Authorization: Bearer <personal API token>`; a missing or unknown token answers 401 with `sign_in_is_required`, and there is no anonymous session or cookie fallback. `KOREOGRAF_MCP=0` turns the route off with 404. The tools match the stdio subcommand and the HTTP API, including the `team_id` argument and `X-Team-Id`, and can launch what the token's role can already launch. `extra_vars` remain in the JSON and can contain secrets.

## Walkthrough

These commands assume a fresh Compose stack from the [quick start](../README.md#docker-compose), `curl`, and `jq`. Sign in first and leave the password file in place until this session is done. The playbook is the demo in this repository. The HTTP calls succeed even when that playbook later fails.

```bash
BASE=http://127.0.0.1:8080
PASS=$(docker compose exec -T koreograf cat /var/lib/koreograf/initial-admin-password | tr -d '\r\n')
JAR=/tmp/koreograf.cookies
rm -f "$JAR"

LOGIN=$(curl -fsS -c "$JAR" -H 'content-type: application/json' \
  --data "$(jq -n --arg username admin --arg password "$PASS" '{username:$username, password:$password}')" \
  "$BASE/api/v1/login")
TEAM=$(jq -r '.teams[] | select(.name=="Plattform") | .id' <<<"$LOGIN")

api() {
  curl -fsS -b "$JAR" -H "X-Team-Id: $TEAM" -H 'content-type: application/json' "$@"
}

PROJECT=$(api -X POST "$BASE/api/v1/projects" --data "$(jq -n '{
  name: "Koreograf",
  source_type: "git",
  source: "https://github.com/koreograf/koreograf.git",
  revision: "v0.1.0-alpha.4"
}')")
PROJECT_ID=$(jq -r .id <<<"$PROJECT")
api -X POST "$BASE/api/v1/projects/$PROJECT_ID/sync"

INVENTORY=$(api -X POST "$BASE/api/v1/inventories" --data "$(jq -n '{
  name: "Local",
  body: "localhost ansible_connection=local\n"
}')")
INVENTORY_ID=$(jq -r .id <<<"$INVENTORY")

CREDENTIAL=$(api -X POST "$BASE/api/v1/credentials" --data "$(jq -n '{
  name: "Deploy key",
  kind: "ssh",
  secret: {username: "deploy", private_key: "replace-with-a-private-key"}
}')")
CREDENTIAL_ID=$(jq -r .id <<<"$CREDENTIAL")

TEMPLATE=$(api -X POST "$BASE/api/v1/templates" --data "$(jq -n \
  --arg project_id "$PROJECT_ID" \
  --arg inventory_id "$INVENTORY_ID" \
  --arg credential_id "$CREDENTIAL_ID" '{
  name: "Demo site",
  tool: "ansible",
  project_id: $project_id,
  inventory_id: $inventory_id,
  credential_ids: [$credential_id],
  entrypoint: "examples/demo/site.yml",
  extra_vars: {},
  spec: {job_type: "run"},
  update_on_launch: false
}')")
TEMPLATE_ID=$(jq -r .id <<<"$TEMPLATE")

JOB=$(api -X POST "$BASE/api/v1/templates/$TEMPLATE_ID/launch" --data '{"extra_vars":{}}')
JOB_ID=$(jq -r .id <<<"$JOB")

for _ in $(seq 1 60); do
  STATUS=$(curl -fsS -b "$JAR" "$BASE/api/v1/jobs/$JOB_ID" | jq -r .status)
  case "$STATUS" in
    successful|failed|canceled) break ;;
  esac
  sleep 2
done

curl -fsS -N -b "$JAR" "$BASE/api/v1/jobs/$JOB_ID/events?after=0"
curl -fsS -b "$JAR" "$BASE/api/v1/jobs/$JOB_ID/output"
```

`/output` is the kept log: the first three quarters of the line limit and the last quarter, for stdout and for stderr. The default limit is 8000 lines. The download stops at 8 MiB. `GET /api/v1/jobs/{id}` includes `output_capped`, true when the download would stop at that cap, and a `gap` event with the omitted line count when the page leaves out the middle.

`GET /api/v1/jobs` lists the selected team's jobs, newest first. `limit` defaults to 25 and is capped at 100; `offset` skips rows. `q` searches the name, tool, status, and trigger. `status`, `tool`, `trigger`, and `template_id` filter exact values. `since` and `until` are RFC 3339 bounds on `created_at`. The response is `{"items": [...], "total": N}`, where `total` counts every match. The bare array this route returned before is gone; clients read `items`.

`GET /api/v1/me/tokens` lists the caller's personal API tokens without secrets. `POST /api/v1/me/tokens` takes `name` and returns the secret once. `DELETE /api/v1/me/tokens/{id}` revokes one token owned by the caller; another user's id is 404. The secret starts with `koro_` and the database stores only a hash. Names follow the same 120-character limit as other names.

`POST /api/v1/login` takes `username` and `password` and returns 200 with the session cookie. A rejected sign-in returns 401 and `invalid_username_or_password` for a wrong password, an unknown user, an empty password, and a directory failure. A request without a session still returns `sign_in_is_required`. Creating a credential needs an owner. Launch returns 202 and needs a JSON object. `extra_vars` is that object, or `{}`.

On a machine without Compose, read `data/initial-admin-password` instead of `docker compose exec`, and use the same JSON.

## Errors

API errors are `{"error": "<key>"}` with optional `vars`. A missing team role is 403 `you_are_not_allowed_to_do_this`. An id that is unknown or belongs to another team is 404 `not_found`; those responses do not say that an object belongs to another team, so a client cannot tell a foreign id from a missing one. A non-administrator who saves a path project gets 400 `the_project_path_is_not_allowed` for both a missing directory and a disallowed path. Administrators keep the clearer `the_project_path_is_not_a_directory` for a missing directory; sensitive and data paths remain `the_project_path_is_not_allowed`.

## Session responses

`GET /api/v1/me`, `PUT /api/v1/me`, and a successful `POST /api/v1/login` return one profile object with `id`, `username`, `display_name`, `auth_source`, `email`, `is_admin`, `teams`, and `memberships`. `email` is a string or `null`. Each team has `id`, `name`, `description`, `ldap_group` (string or `null`), `organization_id`, `organization_name`, and `role`. Each membership has `team_id`, `team_name`, `organization_id`, `organization_name`, `role`, and `source`.

`GET /api/v1/users` returns an array of account summaries with `id`, `username`, `display_name`, `auth_source`, `email`, `is_admin`, `disabled`, `created_at`, and `team_count`. `POST /api/v1/users` returns 201 with that summary. `GET /api/v1/users/{id}` and `PUT /api/v1/users/{id}` return the summary plus `memberships`; `team_count` matches the length of that list.

`GET /api/v1/me/tokens` returns an array with `id`, `name`, `prefix`, `created_at`, and `last_used_at`. `POST /api/v1/me/tokens` returns 201 with those fields plus the one-time `secret`; `last_used_at` is `null` until the token is used.

## Catalog responses

`GET /api/v1/teams` returns an array with `id`, `name`, `description`, `ldap_group` (string or `null`), `organization_id`, `organization_name`, `role`, and `created_at`. `POST /api/v1/teams` returns 201 and `PUT /api/v1/teams/{id}` returns 200 with the same object without `role`. `GET /api/v1/teams/{id}/members` returns an array with `user_id`, `username`, `display_name`, `role`, and `source`; `POST` adds one member and returns 201 with `user_id` and `role`.

`GET /api/v1/organizations` returns an array with `id`, `name`, `description`, and `created_at`. `GET /api/v1/organizations/{id}` adds `teams`, an array of the team objects above without `role`. `POST /api/v1/organizations` returns 201 and `PUT /api/v1/organizations/{id}` returns 200 with the object without `teams`.

`GET /api/v1/projects` returns an array and `GET /api/v1/projects/{id}` returns one project with `id`, `team_id`, `organization_id`, `organization_name`, `name`, `source_type`, `source`, `revision`, `credential_id` (string or `null`), `created_at`, `team_ids`, `last_sync_at`, `last_sync_commit`, and `last_sync_status`. `source` masks credentials in a git address. `credential_id` is `null` when the project belongs to another team. `POST /api/v1/projects` returns 201 with that object. When `PUT` changes `source_type`, `source`, or `revision`, the response adds `sync`; otherwise it has no `sync` key. `POST /api/v1/projects/{id}/sync` returns `{ "path": "..." }`.

`GET /api/v1/inventories` returns an array of summaries with `id`, `team_id`, `name`, `created_at`, `source_kind`, `source`, `credential_id` (string or `null`), `source_count`, `sources`, `host_count`, and `group_count`. Each entry in `sources` has `id`, `name`, `kind`, `source`, `credential_id` (string or `null`), and `host_count`, but no `body`. `GET /api/v1/inventories/{id}` returns one summary. `PUT /api/v1/inventories/{id}` and `POST /api/v1/inventories` return 200 or 201 with the summary plus `body` and `hosts`; each entry in `sources` then also has `body`. `POST /api/v1/inventories/import` and `POST /api/v1/inventories/netbox` return 201 with that detail.

`GET /api/v1/credentials` returns an array with `id`, `name`, `kind`, `created_at`, and `usages`. `GET /api/v1/credentials/{id}` and `PUT /api/v1/credentials/{id}` return `id`, `name`, `kind`, `created_at`, `fields`, `set`, and `usages`. `POST /api/v1/credentials` returns 201 with `id`, `name`, and `kind`. Every usage has `inventory_id`, `inventory_name`, `source_id`, `source_name`, `project_id`, and `project_name`; the inventory fields are `null` for a project usage and the project fields are `null` for a source usage.

`GET /api/v1/environments` returns an array and `GET /api/v1/environments/{id}` returns one environment with `id`, `team_id` (string or `null`), `organization_id` (string or `null`), `name`, `tool`, `image`, `pull_policy`, `network`, `description`, `created_at`, `scope`, `organization_name`, and `team_name`. `POST /api/v1/environments` returns 201 and `PUT /api/v1/environments/{id}` returns 200 with that object.

`GET /api/v1/overview` returns `runner_mode`, `runtime` (string or `null`), `preflight` (the preflight object for an administrator, otherwise `null`), `import` (object or `null`), `ldap`, `cache_bytes`, `jobs`, `deliveries`, and `workflow_id`, `workflow_name`, `hook_id`, and `hook_name`, each a string or `null`. `GET /api/v1/dashboard` returns `range_days`, `timezone`, `from`, `to`, `series`, and `cache`. Each series entry has `date`, `started`, `successful`, `failed`, and `canceled`; `cache` has `collections`, `roles`, and `tofu_bytes`. `GET /api/v1/cache` returns `collections_bytes`, `roles_bytes`, `tofu_bytes`, `collections`, `roles`, and `note`; each collection or role entry has `name` and `version`. `GET /api/v1/plugins` returns an array with `id` and `summary`.

## Work responses

`GET /api/v1/templates` returns an array. `POST /api/v1/templates` returns 201 and `PUT /api/v1/templates/{id}` returns 200 with one template: `id`, `team_id`, `name`, `description`, `tool`, `project_id` (string or `null`), `inventory_id` (string or `null`), `credential_id` (string or `null`), `credential_ids`, `environment_id` (string or `null`), `no_environment`, `entrypoint`, `extra_vars` (object), `spec` (object), `created_at`, `update_on_launch`, and `prompt_on_launch`. `credential_ids` is the list of attached credentials; without links it holds the single `credential_id`, or an empty array. `DELETE /api/v1/templates/{id}` answers 204.

`POST /api/v1/templates/{id}/launch` and `POST /api/v1/jobs/{id}/relaunch` answer 202 with a job. `POST /api/v1/jobs/{id}/cancel` answers 200 with the job. A job has `id`, `team_id`, `template_id` (string or `null`), `workflow_job_id` (string or `null`), `workflow_node_id` (string or `null`), `name`, `tool`, `status`, `extra_vars` (object), `launched_by`, `trigger`, `started_at` (string or `null`), `finished_at` (string or `null`), `created_at`, `parameters` (object), and `failed_hosts` (array of strings). `GET /api/v1/jobs/{id}` adds `events` and `output_capped` (boolean). Each event has `id` (number), `ts`, `stream`, and `line`. `GET /api/v1/jobs` returns `{"items": [...], "total": N}` with the job objects without `events` and `output_capped`; `total` is a number. `GET /api/v1/jobs/{id}/events` sends `log` events whose data has `id` (number), `ts`, `stream`, and `line`, and one `end` event whose data has `status` and `finished_at` (string or `null`).

`GET /api/v1/workflows` returns an array. `GET /api/v1/workflows/{id}`, `POST /api/v1/workflows`, and `PUT /api/v1/workflows/{id}` return one workflow: `id`, `team_id`, `name`, `description`, `graph` (object), and `created_at`. `DELETE /api/v1/workflows/{id}` answers 204. `POST /api/v1/workflows/{id}/launch` answers 202 and `GET /api/v1/workflow-jobs/{id}` answers 200 with a workflow job: `id`, `workflow_id` (string or `null`), `team_id`, `name`, `status`, `extra_vars` (object), `launched_by`, `trigger`, `created_at`, `finished_at` (string or `null`), `graph` (object), and `nodes`. Each node has `node_id`, `phase`, `job_id` (string or `null`), `job_name` (string or `null`), `tool` (string or `null`), `started_at` (string or `null`), `finished_at` (string or `null`), and `job_created_at` (string or `null`).

`GET /api/v1/hooks` returns an array. `POST /api/v1/hooks` returns 201 and `PUT /api/v1/hooks/{id}` returns 200 with one hook: `id`, `team_id`, `name`, `token`, `match_alertname`, `match_status`, `template_id` (string or `null`), `workflow_id` (string or `null`), `extra_vars` (object), `enabled`, and `created_at`. `token` is a non-empty string for a caller who may operate the team and an empty string for a viewer. `DELETE /api/v1/hooks/{id}` answers 204. `POST /api/v1/hooks/{id}/test` answers 202 and `POST /hooks/{id}/{token}` answers 200 with `status`, `summary`, `job_id` (string or `null`), and `workflow_job_id` (string or `null`); a template target sets `job_id` and a workflow target sets `workflow_job_id`. `GET /api/v1/hooks/{id}/deliveries` returns an array with `id`, `hook_id`, `hook_name`, `status`, `summary`, `job_id` (string or `null`), `workflow_job_id` (string or `null`), and `created_at`.

## Request bodies

Names are at most 120 characters. Empty optional strings are omitted.

### Projects

| Field | Required | Notes |
| --- | --- | --- |
| `name` | yes | |
| `source_type` | yes | `path` or `git` |
| `source` | yes | Directory inside the server, or a git address |
| `revision` | no | Branch, tag, or commit. Empty follows the default branch |
| `organization_id` | no | Defaults to the organization of `X-Team-Id` |
| `team_ids` | no | Empty gives the project to every team in the organization |
| `credential_id` | no | One `scm` credential of the same team or of an assigned team. Git projects use it for Sync and update on launch |

`PUT /api/v1/projects/{id}` accepts `name`, `source_type`, `source`, `revision`, `organization_id`, `team_ids`, and `credential_id`. An omitted field keeps its value; an empty `credential_id` clears it. The name, source type, source, and revision use the same validation as `POST`. Only an administrator can move a project to another organization. When `source_type`, `source`, or `revision` changes, the old checkout is discarded and a git project is synced again. That response is the project plus `sync`: `{"ok": true, "path": "..."}` or `{"ok": false, "error": "...", "vars": {...}}`. A failed sync keeps the new values and answers 200; the error is in `sync`. Path projects are described in [Jobs](jobs.md#path-projects).

### Inventories

| Field | Required | Notes |
| --- | --- | --- |
| `name` | yes | |
| `body` | no | Static INI or YAML text when `sources` is omitted |
| `sources` | no | Replaces `body`. Each item is one source |

A source has `kind` `ini`, `project`, or `netbox`.

| Kind | Fields |
| --- | --- |
| `ini` | `name`, `body` (the inventory text) |
| `project` | `name`, `project_id`, `path` relative to that project |
| `netbox` | `name`, required `credential_id` (a NetBox credential of the same team), and optionally `validate_certs` (default true), `group_by`, `group_names_raw` (default false), `load_config_context` (default false), `max_hosts` (default 2000, at most 10000), `query_filters`, `virtual_machines` (default true). The address always comes from the credential. A credential needs a `url`; one without it is rejected with 400 `netbox_needs_an_address`. `api_endpoint` is deprecated: it may be empty or must match the credential address, otherwise 400 `netbox_endpoint_must_match_credential`. `token` is deprecated and no longer creates a credential; a request with a token and no `credential_id` answers 400 `netbox_token_needs_a_credential`. With a credential, only an owner may change `validate_certs` |

`PUT /api/v1/inventories/{id}/sources/{source_id}` takes the fields of the stored kind. An omitted field keeps its value. For `ini` that is `name` and `body`. For `project` that is `name`, `project_id`, and `path`. For `netbox` that is `name`, `credential_id`, `validate_certs`, `group_by`, `group_names_raw`, `load_config_context`, `max_hosts`, `query_filters`, and `virtual_machines`. A stored `ca_file` from an older release is ignored. A non-empty `credential_id` switches the source to that credential. `api_endpoint` and `token` are still accepted for older clients but deprecated: a non-empty `api_endpoint` must match the credential address, and `token` never creates a credential. A used credential pins the source to its URL, so a mismatched `api_endpoint` answers 400 `netbox_endpoint_must_match_credential`; only an owner may change `validate_certs` then. The token is never returned. A field of another kind or an unknown field answers 400. The source keeps its id and position. A changed project or NetBox source is fetched again before the response; its `sync` carries `ok`, `before`, and `after` host counts (numbers) next to `collisions` and `dropped_vars` (arrays of strings). A failed fetch keeps the previous contents and answers with the error. `PUT /api/v1/inventories/{id}` changes the name and may replace the body of a single static source.

`POST /api/v1/inventories/{id}/sources` and `DELETE /api/v1/inventories/{id}/sources/{source_id}` return 200 with the inventory detail. `POST /api/v1/inventories/{id}/sources/{source_id}/sync` and `POST /api/v1/inventories/{id}/sync` return the detail plus `collisions` and `dropped_vars`. `GET /api/v1/inventories/{id}/sources/{source_id}` returns one source with `id`, `inventory_id`, `name`, `kind`, `source`, `credential_id` (string or `null`), `host_count` (number), and `body`.

`POST /api/v1/inventories/import` takes `project_id`, `path`, and an optional `name`. `POST /api/v1/inventories/netbox` takes the same NetBox fields as a source, with `name` required and `credential_id` required. `POST /api/v1/inventories/netbox/preview` and `POST /api/v1/inventories/netbox/test` take `credential_id` and use the credential address and token; the browser never sends a token. Test returns `version`. Preview returns `host_count` (number), `groups` with `name` and `host_count` (number), `sample_host` with `name` and `vars` (object) or `null`, and `collisions` and `dropped_vars` (arrays of strings). A `max_hosts` above 10000 or below 1 answers 400 `netbox_host_limit_is_out_of_range`. Reaching the limit during a fetch answers 400 `the_netbox_host_limit_of_was_reached_raise_max_hosts` with the limit in `vars.limit`.

Every inventory source that uses a NetBox credential uses the same credential row. The source does not store the address. Changing `url` or `token` in `PUT /api/v1/credentials/{id}` applies to every source that references it on the next sync, preview, and connection test. A source saved by an older release with its own `api_endpoint` keeps working: the stored value is ignored and the credential address is used. An older auto-created `<name> Token` credential stays a normal credential with its name; it can be renamed.

### Credentials

`POST /api/v1/credentials` takes `name`, `kind`, and `secret`. A NetBox credential needs a `token` and a `url`; the URL is `http` or `https`. A stored credential from an older release without a URL is disabled until an owner adds one. One credential of each kind can be attached to a template. Vault may repeat when each `vault_id` is unique, and only one vault credential may omit the id. `credential_ids` on a template is the list. `credential_id` is the single-id form.

| Kind | `secret` fields |
| --- | --- |
| `ssh` | `private_key` or `password`, one of them required. Optional `username`, `passphrase`, `become_method` (`sudo`, `su`, `pbrun`, `pfexec`, `doas`, `dzdo`, `ksu`, `runas`), `become_username`, `become_password` |
| `vault` | `password` required. Optional `vault_id` |
| `scm` | `private_key` or `password`. Optional `username`, `passphrase`. Used by a project's Sync, update on launch, and the Galaxy step of its Ansible jobs. Without a username, a token uses `oauth2` for GitLab and `x-access-token` for GitHub |
| `net` | `username` or `password`. Optional `private_key`, `authorize_password` |
| `aws` | `access_key`, `secret_key`. Optional `security_token` |
| `azure` | `subscription_id`, `client_id`, `secret`, `tenant` |
| `gce` | `service_account`, a JSON object encoded as a string |
| `vmware` | `host`, `username`, `password` |
| `openstack` | `auth_url` (`https`), `project`, `username`, `password`. Optional `domain` |
| `registry` | `host`, `username`, `password` |
| `github`, `gitlab` | `token` |
| `galaxy` | `token`. Optional `url` |
| `kubernetes` | `host` (`https`), `token`. Optional `verify_ssl` |
| `insights` | `username`, `password` |
| `env` | An object of variable names to values |
| `netbox` | `token`, `url` (`http` or `https`). The inventory source takes the address and token from this credential |

`PUT /api/v1/credentials/{id}` takes `name`, an optional `secret` object, and optional `clear` (names of secret fields to drop). Omitting a stored secret keeps it. `GET /api/v1/credentials` and `GET /api/v1/credentials/{id}` include `usages`, a list of objects with `inventory_id`, `inventory_name`, `source_id`, `source_name`, `project_id`, and `project_name`. Each key is present and is a string or `null`; a source usage has `null` project fields and a project usage has `null` inventory fields. `DELETE /api/v1/credentials/{id}` answers 409 `credential_is_in_use` with the names in `vars.names` while any source or project still uses it.

### Environments

| Field | Required | Notes |
| --- | --- | --- |
| `name` | yes | |
| `image` | yes | Must contain the binaries for `tool` |
| `tool` | on create | `ansible`, `opentofu`, or `shell`. An existing legacy `any` row may be saved unchanged |
| `pull_policy` | no | `if_not_present` (default), `always`, or `never` |
| `network` | no | Empty keeps the container default |
| `description` | no | |
| `scope` | no | `team` (default), `organization`, or `global` |

`GET /api/v1/environments` and `GET /api/v1/environments/{id}` include `tool`. A new environment without one of the three tools answers 400 `choose_a_tool_for_the_environment`. A legacy `any` environment keeps that tool only when the image is unchanged; a create or an image change needs one of the three. A template may only use an environment whose tool is `any` or matches the template's tool. The API rejects a mismatch with `the_execution_environment_is_for_another_tool`.

On the first start the database gets one global environment per tool and the matching defaults, unless `KOREOGRAF_SEED_ENVIRONMENTS` is off. The step is idempotent and records that it ran, so a renamed or deleted seeded environment is not created again. A launch that needs a container and finds no default answers 400 `no_default_execution_environment` with `vars.tool`.

### Templates

| Field | Required | Notes |
| --- | --- | --- |
| `name` | yes | |
| `description` | no | |
| `tool` | yes | `ansible`, `opentofu`, or `shell` |
| `project_id` | no | |
| `inventory_id` | no | |
| `credential_id` | no | One id. Ignored when `credential_ids` is present |
| `credential_ids` | no | |
| `environment_id` | no | Must match `tool`, or be a legacy `any` environment. Empty means the tool default |
| `no_environment` | no | Default false. True with an empty `environment_id` runs on the controller when this machine has the tool; otherwise the tool default applies |
| `entrypoint` | for ansible and opentofu | Playbook path, or the OpenTofu module directory |
| `extra_vars` | no | Object. Default `{}` |
| `spec` | no | Object. Default `{}` |
| `update_on_launch` | no | Default true. Git projects only |
| `prompt_on_launch` | no | Default false. The launch dialog asks for inventory, credentials, limit, tags, skip tags, job type, verbosity, and extra variables |

A `PUT` replaces the stored template, and `no_environment` falls back to false when the body omits it. A migrated template that runs on the controller therefore needs `"no_environment": true` sent explicitly; omitting the field switches that template to the tool default. In the container image `no_environment` is ignored and the tool default runs.

Ansible `spec`: `job_type` `run` or `check`, `tags`, `skip_tags`, `limit`, `diff` (boolean), `verbosity` from 0 to 4, `forks` from 1 to 50.

OpenTofu `spec`: `action` `plan` or `apply`.

Shell: `entrypoint` or `spec.script`. The script is written as `inline.sh`.

### Hooks

| Field | Required | Notes |
| --- | --- | --- |
| `name` | yes | |
| `match_alertname` | yes | |
| `match_status` | no | Default `firing` |
| `template_id` | no | One of template or workflow |
| `workflow_id` | no | |
| `extra_vars` | no | Object merged into the launch |

`PUT /api/v1/hooks/{id}` replaces those fields and keeps the token and the created time.

### Launch

`POST /api/v1/templates/{id}/launch` and `POST /api/v1/workflows/{id}/launch` take `{"extra_vars": {}}`. The value is an object. A template launch may also send `inventory_id`, `credential_ids`, and an Ansible `spec` with `job_type`, `tags`, `skip_tags`, `limit`, and `verbosity`. Omit a field to keep the template value. An empty `inventory_id` clears it. An empty string in those spec fields clears that field. Non-empty `extra_vars` and those three override fields are accepted only when the template has `prompt_on_launch` set, or when the body sets `launch_with_options: true`. The template row's Launch with options action sends that flag. Plain API launches do not set it, so a template without `prompt_on_launch` rejects those overrides with `prompt_on_launch_is_off`. An empty body or empty `extra_vars` keeps the template. When accepted, `extra_vars` is merged onto the template values. Hooks and other trigger payloads still merge their extra vars without that flag. The template row is not changed. Workflow launch ignores those three fields and uses the graph.

| Method | Path |
| --- | --- |
| POST | `/api/v1/login`, `/api/v1/logout` |
| GET, POST | `/api/v1/users` |
| GET, PUT, DELETE | `/api/v1/users/{id}` |
| GET, PUT | `/api/v1/me` |
| GET | `/api/v1/overview`, `/api/v1/dashboard`, `/api/v1/runner`, `/api/v1/runner/preflight`, `/api/v1/teams` |
| POST | `/api/v1/teams` |
| PUT | `/api/v1/teams/{id}` |
| GET, POST | `/api/v1/teams/{id}/members` |
| DELETE | `/api/v1/teams/{id}/members/{user_id}` |
| GET, POST | `/api/v1/organizations` |
| GET, PUT | `/api/v1/organizations/{id}` |
| GET, POST | `/api/v1/projects`, `/api/v1/inventories`, `/api/v1/credentials`, `/api/v1/environments` |
| GET | `/api/v1/projects/{id}`, `/api/v1/inventories/{id}`, `/api/v1/credentials/{id}`, `/api/v1/environments/{id}` |
| PUT | `/api/v1/projects/{id}`, `/api/v1/inventories/{id}`, `/api/v1/credentials/{id}`, `/api/v1/environments/{id}` |
| DELETE | `/api/v1/projects/{id}`, `/api/v1/inventories/{id}`, `/api/v1/credentials/{id}`, `/api/v1/environments/{id}` |
| GET | `/api/v1/projects/{id}/contents` |
| POST | `/api/v1/projects/{id}/sync` |
| POST | `/api/v1/inventories/import`, `/api/v1/inventories/netbox`, `/api/v1/inventories/netbox/preview` |
| POST | `/api/v1/inventories/{id}/sync` |
| GET | `/api/v1/inventories/{id}/hosts` |
| GET | `/api/v1/inventories/{id}/hosts/{name}` |
| GET | `/api/v1/inventories/{id}/groups` |
| GET | `/api/v1/inventories/{id}/groups/{name}` |
| POST | `/api/v1/inventories/{id}/sources` |
| GET, PUT, DELETE | `/api/v1/inventories/{id}/sources/{source_id}` |
| POST | `/api/v1/inventories/{id}/sources/{source_id}/sync` |
| GET | `/api/v1/cache`, `/api/v1/plugins` |
| GET, POST | `/api/v1/templates`, `/api/v1/workflows`, `/api/v1/hooks`, `/api/v1/jobs` |
| PUT, DELETE | `/api/v1/templates/{id}`, `/api/v1/workflows/{id}`, `/api/v1/hooks/{id}` |
| POST | `/api/v1/templates/{id}/launch`, `/api/v1/workflows/{id}/launch` |
| GET, POST | `/api/v1/templates/{id}/schedules`, `/api/v1/workflows/{id}/schedules` |
| GET | `/api/v1/schedules` |
| POST | `/api/v1/schedules/preview` |
| PUT, DELETE | `/api/v1/schedules/{id}` |
| POST | `/api/v1/schedules/{id}/enabled`, `/api/v1/schedules/{id}/missed` |
| GET | `/api/v1/jobs/{id}`, `/api/v1/jobs/{id}/output`, `/api/v1/workflows/{id}`, `/api/v1/workflow-jobs/{id}` |
| POST | `/api/v1/jobs/{id}/cancel`, `/api/v1/jobs/{id}/relaunch`, `/api/v1/hooks/{id}/test` |
| GET | `/api/v1/hooks/{id}/deliveries` |
| POST | `/hooks/{id}/{token}` |
| GET | `/healthz`, `/metrics` |
| GET, POST | `/api/v1/admin/backup` |
| GET | `/api/v1/maintenance` |
| GET | `/api/v1/admin/database/status` |
| POST | `/api/v1/admin/database/check`, `/api/v1/admin/database/migrate` |
| GET, PUT | `/api/v1/settings` |

`GET /api/v1/maintenance` needs no session and returns `active` and `since`. While a database migration runs, every other mutating request answers 503 `maintenance_mode_is_active`; `GET`, `HEAD`, `OPTIONS`, `/api/v1/login`, and `/api/v1/logout` stay available.

`GET /api/v1/admin/database/status` is administrator-only. It returns `engine`, `supported` (true while the process runs on SQLite), `maintenance`, and `migration` with `running`, `done`, `failed`, `started_at`, `finished_at`, `target_host`, `url`, `rows`, `error`, `tables` (`table`, `rows`, `done`), and `audit` (`user`, `at`, `host`). `url` never carries the password. `POST /api/v1/admin/database/check` takes `host`, `port`, `database`, `user`, `password`, `sslmode`, and an optional `ca_file`, runs the same checks as `koreograf db migrate`, and returns `all_passed`, `id`, and `checks` (`id`, `ok`, `key`). It stores the target only in memory. `POST /api/v1/admin/database/migrate` takes that `id`, refuses while a check failed or a job runs, enters maintenance mode, copies in the background, and answers 202 `{"started": true}`; progress comes from the status route. The password is never in a response, a log line, or the audit entry.

`GET /api/v1/admin/backup?tofu_state=true` adds `cache/<team>/tofu-state/` to the zip. Without the query parameter the state stays out. `POST /api/v1/admin/backup` still needs `X-Restore-Confirm: ERSETZEN`. `GET /api/v1/overview` includes `import` with `source`, `path_map`, `missing_paths`, and the copied state count for an administrator after an import, and `null` otherwise.

`GET /api/v1/inventories` returns one summary per inventory with `id`, `team_id`, `name`, `created_at`, `source_kind`, `source`, `credential_id`, `source_count`, `host_count`, `group_count`, and `sources` metadata. A summary has no `hosts` array and no source `body`. This is a breaking change for alpha clients that read `hosts` from the list.

`GET /api/v1/inventories/{id}` returns the same summary. It has no `hosts` array and no source `body`. `GET /api/v1/inventories/{id}/hosts` returns a page of parsed hosts with `id`, `total`, `page`, `page_size`, `groups`, and `hosts`. Each host has `name`, `groups`, and `vars`; `vars` is an object of names to values. `page` starts at 1. `page_size` defaults to 50 and is capped at 200; `limit` is an alias and `offset` starts at zero. `search` matches the host name, group names, variable names, and variable values. `group` keeps hosts in that exact group. `sort` is `name` (default), `address` (the `ansible_host` value), or `groups`; a leading `-` reverses the order.

`GET /api/v1/inventories/{id}/hosts/{name}` returns one host with its `groups` (including parents from `:children`), its `source` (`id` is a string for a source row and `null` for a static body, plus `name` and `kind`), and `vars` as an array of `{name, value, origin}`. The value and origin follow Ansible precedence: `all` is lowest, a child group overrides its parent groups, unrelated groups merge, and host variables win. The origin is `host`, `all`, or the group name. `netbox_id` and `netbox_url` are always present and are a string or `null`; a NetBox source with a numeric id points `netbox_url` at `/dcim/devices/{id}/` on its `api_endpoint`. An unknown host answers 404.

`GET /api/v1/inventories/{id}/groups` returns `id`, `total`, and `groups`. Each group has `name`, `host_count` (including child groups), `children`, and `vars`. `GET /api/v1/inventories/{id}/groups/{name}` returns `id`, `name`, `children`, `parents`, `vars`, `host_count`, `total`, `page`, `page_size`, and a page of `hosts` with the paging and sort of the hosts endpoint. `search` matches the host name, groups, and variables. An unknown group answers 404. The browser UI masks variables whose names look secret and offers no reveal.

`GET /api/v1/inventories/{id}/sources/{source_id}` returns one source including its raw `body`. Groups with `:vars` or `:children` are not hosts. A git sync that fails answers with the last lines from git. `contents` answers with `playbooks` (array of strings) and `inventories`, where each entry has `path`, `format`, `plugin` (string or `null`), `api_endpoint` (string or `null`), `group_by`, `query_filters`, `validate_certs` (boolean or `null`), and `virtual_machines` (boolean or `null`). The NetBox fetch happens before the save.

Launches answer 202 and need a JSON object, at least `{}`. An array is rejected. `extra_vars` is an object. A request without a JSON content type is `json_body_required`. An empty body, broken JSON, a non-object, or a field of the wrong type is `json_body_invalid`. An unknown field is `json_field_is_not_supported` with the field name in `vars.field`; every endpoint that reads a JSON request body rejects a field its request type does not use. A body over 2 MiB is `request_too_large` with status 413. `POST /hooks/{id}/{token}` is the only endpoint that accepts any JSON value, including arbitrary fields.

Every mutating `/api/v1/` request whose `Origin` or `Referer` does not match the request origin is refused with 403 `cross_origin_request_is_not_allowed`. The request origin is `KOREOGRAF_PUBLIC_URL` when set, otherwise the `Host` header with an assumed `http` scheme (plus `X-Forwarded-Host`/`X-Forwarded-Proto` when `KOREOGRAF_TRUSTED_PROXY` is on); a TLS-terminating proxy therefore needs the public URL or the trusted-proxy setting. Scheme, host and port are compared; a header that is present but unparsable, or `null`, is refused. A request without either header is allowed, so scripts and token clients keep working. `POST /hooks/{id}/{token}` is outside `/api` and is not covered.

`PUT /api/v1/settings` takes a complete body. It ignores the read-only fields that `GET /api/v1/settings` returns (`source`, `ldap.bind_password_set`, `proxy.password_set`, `proxy.unsupported_env`), so the GET body can be sent back unchanged. `ldap`, `proxy`, `runner`, and `job_timeout_secs` are required, and a body that leaves one of them out answers 400 with `the_settings_body_is_incomplete` and the missing names in `fields`. `job_output_lines` is optional. `default_environment_ansible`, `default_environment_opentofu`, and `default_environment_shell` are optional environment ids; empty means no default. A non-empty id must be global and match that tool, or be legacy `any`; otherwise the API answers `the_default_environment_is_missing`, `the_default_environment_must_be_global`, or `the_default_environment_is_for_another_tool`. `GET /api/v1/settings` includes the three ids, and deleting the referenced environment clears the setting. An edit that changes that environment's tool or moves it out of the global scope clears the setting too. `GET /api/v1/runner/preflight` is administrator-only. It reruns the runner check, refreshes the cached report, and returns `mode`, `container_image`, `ok`, `tools`, `runtime`, `missing`, and English `messages`. Each tool has `id`, `kind`, `template_tool`, `minimum`, `required`, `found`, `path`, `version`, `meets_minimum`, and `reason`. A `kind` of `project` marks `git`; the other tools are template tools, and `ssh` belongs to Ansible. A runtime has `binary`, `found`, `responding`, `path`, `version`, and `reason`. The response never includes an environment variable or the value of `PATH`. `GET /api/v1/overview` includes the cached report as `preflight` for an administrator and `null` for everyone else; startup and this endpoint refresh that cache.

`GET /api/v1/runner` includes the same three ids for the template overlay, plus `container_image`, `runtime`, and `local_tools` (`ansible`, `opentofu`, `shell`). The overlay hides **No environment** when `container_image` is true or the matching `local_tools` value is false. `{ "reset": true }` needs no other fields and clears the defaults.

A schedule object has `id`, `name`, `target_kind` (`template` or `workflow`), `target_id`, `timezone`, `start_date`, `start_time`, `frequency`, `weekdays` (array of `MO` through `SU`), `end_kind`, `end_count` (number or `null`), `end_date` (string or `null`), `enabled`, `next_run_at` (string or `null`), `missed_at` (string or `null`), `created_by`, `updated_by`, and `launch_block` (string or `null`). The lists for one template or workflow return an array of these objects. `GET /api/v1/schedules` lists every schedule of the selected team and adds `target_name`. Creating one answers 201, changing one answers 200, and deleting one answers 204 with no body. `POST /api/v1/schedules/preview` returns `{"occurrences": [...]}` with up to five upcoming UTC instants. A schedule belongs to one template or one workflow. Creating one needs `name`, an IANA `timezone`, `start_date` (`YYYY-MM-DD`), `start_time` (`HH:MM`), and `frequency` (`once`, `hourly`, `daily`, `weekly`, `monthly`). Weekly also needs `weekdays` (`MO` through `SU`). `end_kind` is `never`, `after` with `end_count` from 1 to 999, or `on_date`. The clock time stays in that zone across daylight saving. The preview does not save anything. A viewer can list. An operator can create, change, delete, and pause. `POST /api/v1/schedules/{id}/enabled` takes `{ "enabled": true }`. `POST /api/v1/schedules/{id}/missed` takes `{ "accept": true }` to start the last missed time once, or `{ "accept": false }` to clear it. A failed start leaves that missed time in place. Creating a schedule sets `created_by` and `updated_by` to the signed-in user. Changing one sets `updated_by` to the editing user and leaves `created_by` as it was. Automatic runs start as `updated_by` only when that user may still start jobs for the team. Otherwise no job starts, the time stays missed, and `launch_block` is `schedule_needs_a_new_owner`. Times that passed while the process was stopped are not replayed.
