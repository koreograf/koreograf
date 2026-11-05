# API

`GET /api/version` does not need a session. It returns the version compiled from `Cargo.toml`. The browser UI reads this route for the build line on the sign-in page and in the sidebar. Session cookie `koreograf_session`, HttpOnly, SameSite=Lax, seven days, or `Authorization: Bearer <token>` with a personal API token. When both are present, the bearer token wins. `Secure` is added when `KOREOGRAF_COOKIE_SECURE` is `true`, and when it is `auto` and a trusted proxy sends `X-Forwarded-Proto: https`. The default is `auto` with that proxy off, so a plain HTTP sign-in still receives the cookie. Team routes need `X-Team-Id`. Job events use the team membership stored on the job, because `EventSource` cannot set custom headers. The stream is `GET /api/jobs/{id}/events?after=`.

## Walkthrough

These commands assume a fresh Compose stack from the [quick start](../README.md#docker-compose), `curl`, and `jq`. Sign in first and leave the password file in place until this session is done. The playbook is the demo in this repository. The HTTP calls succeed even when that playbook later fails.

```bash
BASE=http://127.0.0.1:8080
PASS=$(docker compose exec -T koreograf cat /var/lib/koreograf/initial-admin-password | tr -d '\r\n')
JAR=/tmp/koreograf.cookies
rm -f "$JAR"

LOGIN=$(curl -fsS -c "$JAR" -H 'content-type: application/json' \
  --data "$(jq -n --arg username admin --arg password "$PASS" '{username:$username, password:$password}')" \
  "$BASE/api/login")
TEAM=$(jq -r '.teams[] | select(.name=="Plattform") | .id' <<<"$LOGIN")

api() {
  curl -fsS -b "$JAR" -H "X-Team-Id: $TEAM" -H 'content-type: application/json' "$@"
}

PROJECT=$(api -X POST "$BASE/api/projects" --data "$(jq -n '{
  name: "Koreograf",
  source_type: "git",
  source: "https://github.com/koreograf/koreograf.git",
  revision: "v0.1.0-alpha.4"
}')")
PROJECT_ID=$(jq -r .id <<<"$PROJECT")
api -X POST "$BASE/api/projects/$PROJECT_ID/sync"

INVENTORY=$(api -X POST "$BASE/api/inventories" --data "$(jq -n '{
  name: "Local",
  body: "localhost ansible_connection=local\n"
}')")
INVENTORY_ID=$(jq -r .id <<<"$INVENTORY")

CREDENTIAL=$(api -X POST "$BASE/api/credentials" --data "$(jq -n '{
  name: "Deploy key",
  kind: "ssh",
  secret: {username: "deploy", private_key: "replace-with-a-private-key"}
}')")
CREDENTIAL_ID=$(jq -r .id <<<"$CREDENTIAL")

TEMPLATE=$(api -X POST "$BASE/api/templates" --data "$(jq -n \
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

JOB=$(api -X POST "$BASE/api/templates/$TEMPLATE_ID/launch" --data '{"extra_vars":{}}')
JOB_ID=$(jq -r .id <<<"$JOB")

for _ in $(seq 1 60); do
  STATUS=$(curl -fsS -b "$JAR" "$BASE/api/jobs/$JOB_ID" | jq -r .status)
  case "$STATUS" in
    successful|failed|canceled) break ;;
  esac
  sleep 2
done

curl -fsS -N -b "$JAR" "$BASE/api/jobs/$JOB_ID/events?after=0"
curl -fsS -b "$JAR" "$BASE/api/jobs/$JOB_ID/output"
```

`/output` is the kept log: the first three quarters of the line limit and the last quarter, for stdout and for stderr. The default limit is 8000 lines. The download stops at 8 MiB. `GET /api/jobs/{id}` includes `output_capped`, true when the download would stop at that cap, and a `gap` event with the omitted line count when the page leaves out the middle.

`GET /api/jobs` lists the selected team's jobs, newest first. `limit` defaults to 25 and is capped at 100; `offset` skips rows. `q` searches the name, tool, status, and trigger. `status`, `tool`, `trigger`, and `template_id` filter exact values. `since` and `until` are RFC 3339 bounds on `created_at`. The response is `{"items": [...], "total": N}`, where `total` counts every match. The bare array this route returned before is gone; clients read `items`.

`GET /api/me/tokens` lists the caller's personal API tokens without secrets. `POST /api/me/tokens` takes `name` and returns the secret once. `DELETE /api/me/tokens/{id}` revokes one token owned by the caller; another user's id is 404. The secret starts with `koro_` and the database stores only a hash. Names follow the same 120-character limit as other names.

`POST /api/login` takes `username` and `password` and returns 200 with the session cookie. A rejected sign-in returns 401 and `invalid_username_or_password` for a wrong password, an unknown user, an empty password, and a directory failure. A request without a session still returns `sign_in_is_required`. Creating a credential needs an owner. Launch returns 202 and needs a JSON object. `extra_vars` is that object, or `{}`.

On a machine without Compose, read `data/initial-admin-password` instead of `docker compose exec`, and use the same JSON.

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

`PUT /api/projects/{id}` accepts `name`, `source_type`, `source`, `revision`, `organization_id`, `team_ids`, and `credential_id`. An omitted field keeps its value; an empty `credential_id` clears it. The name, source type, source, and revision use the same validation as `POST`. Only an administrator can move a project to another organization. When `source_type`, `source`, or `revision` changes, the old checkout is discarded and a git project is synced again. That response is the project plus `sync`: `{"ok": true, "path": "..."}` or `{"ok": false, "error": "...", "vars": {...}}`. A failed sync keeps the new values and answers 200; the error is in `sync`. Path projects are described in [Jobs](jobs.md#path-projects).

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

`PUT /api/inventories/{id}/sources/{source_id}` takes the fields of the stored kind. An omitted field keeps its value. For `ini` that is `name` and `body`. For `project` that is `name`, `project_id`, and `path`. For `netbox` that is `name`, `credential_id`, `validate_certs`, `group_by`, `group_names_raw`, `load_config_context`, `max_hosts`, `query_filters`, and `virtual_machines`. A stored `ca_file` from an older release is ignored. A non-empty `credential_id` switches the source to that credential. `api_endpoint` and `token` are still accepted for older clients but deprecated: a non-empty `api_endpoint` must match the credential address, and `token` never creates a credential. A used credential pins the source to its URL, so a mismatched `api_endpoint` answers 400 `netbox_endpoint_must_match_credential`; only an owner may change `validate_certs` then. The token is never returned. A field of another kind or an unknown field answers 400. The source keeps its id and position. A changed project or NetBox source is fetched again before the response; its `sync` carries `ok`, `before`, and `after` host counts. A failed fetch keeps the previous contents and answers with the error. `PUT /api/inventories/{id}` changes the name and may replace the body of a single static source.

`POST /api/inventories/import` takes `project_id`, `path`, and an optional `name`. `POST /api/inventories/netbox` takes the same NetBox fields as a source, with `name` required and `credential_id` required. `POST /api/inventories/netbox/preview` and `POST /api/inventories/netbox/test` take `credential_id` and use the credential address and token; the browser never sends a token. Preview returns `host_count`, `groups` with `name` and `host_count`, `sample_host` with its `vars`, and `collisions`. A `max_hosts` above 10000 or below 1 answers 400 `netbox_host_limit_is_out_of_range`. Reaching the limit during a fetch answers 400 `the_netbox_host_limit_of_was_reached_raise_max_hosts` with the limit in `vars.limit`.

Every inventory source that uses a NetBox credential uses the same credential row. The source does not store the address. Changing `url` or `token` in `PUT /api/credentials/{id}` applies to every source that references it on the next sync, preview, and connection test. A source saved by an older release with its own `api_endpoint` keeps working: the stored value is ignored and the credential address is used. An older auto-created `<name> Token` credential stays a normal credential with its name; it can be renamed.

### Credentials

`POST /api/credentials` takes `name`, `kind`, and `secret`. A NetBox credential needs a `token` and a `url`; the URL is `http` or `https`. A stored credential from an older release without a URL is disabled until an owner adds one. One credential of each kind can be attached to a template. Vault may repeat when each `vault_id` is unique, and only one vault credential may omit the id. `credential_ids` on a template is the list. `credential_id` is the single-id form.

| Kind | `secret` fields |
| --- | --- |
| `ssh` | `private_key` or `password`, one of them required. Optional `username`, `passphrase`, `become_method` (`sudo`, `su`, `pbrun`, `pfexec`, `doas`, `dzdo`, `ksu`, `runas`), `become_username`, `become_password` |
| `vault` | `password` required. Optional `vault_id` |
| `scm` | `private_key` or `password`. Optional `username`, `passphrase`. Used by a project that selects it. Without a username, a token uses `oauth2` for GitLab and `x-access-token` for GitHub |
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

`PUT /api/credentials/{id}` takes `name`, an optional `secret` object, and optional `clear` (names of secret fields to drop). Omitting a stored secret keeps it. `GET /api/credentials` and `GET /api/credentials/{id}` include `usages`, a list of `{inventory_id, inventory_name, source_id, source_name}` for inventory sources and `{project_id, project_name}` for projects that reference the credential. `DELETE /api/credentials/{id}` answers 409 `credential_is_in_use` with the names in `vars.names` while any source or project still uses it.

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

`GET /api/environments` and `GET /api/environments/{id}` include `tool`. A new environment without one of the three tools answers 400 `choose_a_tool_for_the_environment`. A legacy `any` environment keeps that tool only when the image is unchanged; a create or an image change needs one of the three. A template may only use an environment whose tool is `any` or matches the template's tool. The API rejects a mismatch with `the_execution_environment_is_for_another_tool`.

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

`PUT /api/hooks/{id}` replaces those fields and keeps the token and the created time.

### Launch

`POST /api/templates/{id}/launch` and `POST /api/workflows/{id}/launch` take `{"extra_vars": {}}`. The value is an object. A template launch may also send `inventory_id`, `credential_ids`, and an Ansible `spec` with `job_type`, `tags`, `skip_tags`, `limit`, and `verbosity`. Omit a field to keep the template value. An empty `inventory_id` clears it. An empty string in those spec fields clears that field. `extra_vars` is still merged. Those three fields are accepted only when the template has `prompt_on_launch` set, or when the body sets `launch_with_options: true`. The template row's Launch with options action sends that flag. Plain API launches and hooks do not set it, so a template without `prompt_on_launch` still rejects those overrides. Otherwise the launch is rejected. A body that sends none of them keeps the template, including for hooks and scripts. The template row is not changed. Workflow launch ignores those three fields and uses the graph.

| Method | Path |
| --- | --- |
| POST | `/api/login`, `/api/logout` |
| GET, POST | `/api/users` |
| GET, PUT, DELETE | `/api/users/{id}` |
| GET, PUT | `/api/me` |
| GET | `/api/overview`, `/api/dashboard`, `/api/runner`, `/api/runner/preflight`, `/api/teams` |
| POST | `/api/teams` |
| PUT | `/api/teams/{id}` |
| GET, POST | `/api/teams/{id}/members` |
| DELETE | `/api/teams/{id}/members/{user_id}` |
| GET, POST | `/api/organizations` |
| GET, PUT | `/api/organizations/{id}` |
| GET, POST | `/api/projects`, `/api/inventories`, `/api/credentials`, `/api/environments` |
| GET | `/api/projects/{id}`, `/api/inventories/{id}`, `/api/credentials/{id}`, `/api/environments/{id}` |
| PUT | `/api/projects/{id}`, `/api/inventories/{id}`, `/api/credentials/{id}`, `/api/environments/{id}` |
| DELETE | `/api/projects/{id}`, `/api/inventories/{id}`, `/api/credentials/{id}`, `/api/environments/{id}` |
| GET | `/api/projects/{id}/contents` |
| POST | `/api/projects/{id}/sync` |
| POST | `/api/inventories/import`, `/api/inventories/netbox`, `/api/inventories/netbox/preview` |
| POST | `/api/inventories/{id}/sync` |
| GET | `/api/inventories/{id}/hosts` |
| GET | `/api/inventories/{id}/hosts/{name}` |
| GET | `/api/inventories/{id}/groups` |
| GET | `/api/inventories/{id}/groups/{name}` |
| POST | `/api/inventories/{id}/sources` |
| GET, PUT, DELETE | `/api/inventories/{id}/sources/{source_id}` |
| POST | `/api/inventories/{id}/sources/{source_id}/sync` |
| GET | `/api/cache`, `/api/plugins` |
| GET, POST | `/api/templates`, `/api/workflows`, `/api/hooks`, `/api/jobs` |
| PUT, DELETE | `/api/templates/{id}`, `/api/workflows/{id}`, `/api/hooks/{id}` |
| POST | `/api/templates/{id}/launch`, `/api/workflows/{id}/launch` |
| GET, POST | `/api/templates/{id}/schedules`, `/api/workflows/{id}/schedules` |
| GET | `/api/schedules` |
| POST | `/api/schedules/preview` |
| PUT, DELETE | `/api/schedules/{id}` |
| POST | `/api/schedules/{id}/enabled`, `/api/schedules/{id}/missed` |
| GET | `/api/jobs/{id}`, `/api/jobs/{id}/output`, `/api/workflows/{id}`, `/api/workflow-jobs/{id}` |
| POST | `/api/jobs/{id}/cancel`, `/api/jobs/{id}/relaunch`, `/api/hooks/{id}/test` |
| GET | `/api/hooks/{id}/deliveries` |
| POST | `/hooks/{id}/{token}` |
| GET | `/healthz`, `/metrics` |
| GET, POST | `/api/admin/backup` |
| GET, PUT | `/api/settings` |

`GET /api/admin/backup?tofu_state=true` adds `cache/<team>/tofu-state/` to the zip. Without the query parameter the state stays out. `POST /api/admin/backup` still needs `X-Restore-Confirm: ERSETZEN`. `GET /api/overview` includes `import` with `source`, `path_map`, `missing_paths`, and the copied state count for an administrator after an import, and `null` otherwise.

`GET /api/inventories` returns one summary per inventory with `id`, `name`, `created_at`, `source_kind`, `source`, `source_count`, `host_count`, `group_count`, and `sources` metadata. A summary has no `hosts` array and no source `body`. This is a breaking change for alpha clients that read `hosts` from the list.

`GET /api/inventories/{id}` returns the same summary. It has no `hosts` array and no source `body`. `GET /api/inventories/{id}/hosts` returns a page of parsed hosts with `id`, `total`, `page`, `page_size`, `groups`, and `hosts`. `page` starts at 1. `page_size` defaults to 50 and is capped at 200; `limit` is an alias and `offset` starts at zero. `search` matches the host name, group names, variable names, and variable values. `group` keeps hosts in that exact group. `sort` is `name` (default), `address` (the `ansible_host` value), or `groups`; a leading `-` reverses the order.

`GET /api/inventories/{id}/hosts/{name}` returns one host with its `groups` (including parents from `:children`), its `source` (`id`, `name`, and `kind`, or the inventory itself for a static body without a source row), and `vars` as an array of `{name, value, origin}`. The value and origin follow Ansible precedence: `all` is lowest, a child group overrides its parent groups, unrelated groups merge, and host variables win. The origin is `host`, `all`, or the group name. A NetBox source with a numeric `netbox_id` also returns `netbox_id` and `netbox_url`, which points at `/dcim/devices/{id}/` on its `api_endpoint`. An unknown host answers 404.

`GET /api/inventories/{id}/groups` returns `id`, `total`, and `groups`. Each group has `name`, `host_count` (including child groups), `children`, and `vars`. `GET /api/inventories/{id}/groups/{name}` returns the same fields plus `parents`, and a page of `hosts` with the paging and sort of the hosts endpoint. `search` matches the host name, groups, and variables. An unknown group answers 404. The browser UI masks variables whose names look secret and offers no reveal.

`GET /api/inventories/{id}/sources/{source_id}` returns one source including its raw `body`. Groups with `:vars` or `:children` are not hosts. A git sync that fails answers with the last lines from git. `contents` answers with `playbooks` and `inventories`. The NetBox fetch happens before the save.

Launches answer 202 and need a JSON object, at least `{}`. An array is rejected. `extra_vars` is an object. A request without a JSON content type is `json_body_required`. An empty body, broken JSON, a non-object, or a field of the wrong type is `json_body_invalid`. An unknown field is `json_field_is_not_supported` with the field name in `vars.field`; every endpoint that reads a JSON request body rejects a field its request type does not use. A body over 2 MiB is `request_too_large` with status 413. `POST /hooks/{id}/{token}` is the only endpoint that accepts any JSON value, including arbitrary fields.

`PUT /api/settings` takes a complete body. `ldap`, `proxy`, `runner`, and `job_timeout_secs` are required, and a body that leaves one of them out answers 400 with `the_settings_body_is_incomplete` and the missing names in `fields`. `job_output_lines` is optional. `default_environment_ansible`, `default_environment_opentofu`, and `default_environment_shell` are optional environment ids; empty means no default. A non-empty id must be global and match that tool, or be legacy `any`; otherwise the API answers `the_default_environment_is_missing`, `the_default_environment_must_be_global`, or `the_default_environment_is_for_another_tool`. `GET /api/settings` includes the three ids, and deleting the referenced environment clears the setting. An edit that changes that environment's tool or moves it out of the global scope clears the setting too. `GET /api/runner/preflight` is administrator-only. It reruns the runner check, refreshes the cached report, and returns `mode`, `container_image`, `ok`, `tools`, `runtime`, `missing`, and English `messages`. Each tool has `id`, `kind`, `template_tool`, `minimum`, `required`, `found`, `path`, `version`, `meets_minimum`, and `reason`. A `kind` of `project` marks `git`; the other tools are template tools, and `ssh` belongs to Ansible. A runtime has `binary`, `found`, `responding`, `path`, `version`, and `reason`. The response never includes an environment variable or the value of `PATH`. `GET /api/overview` includes the cached report as `preflight` for an administrator and `null` for everyone else; startup and this endpoint refresh that cache.

`GET /api/runner` includes the same three ids for the template overlay, plus `container_image`, `runtime`, and `local_tools` (`ansible`, `opentofu`, `shell`). The overlay hides **No environment** when `container_image` is true or the matching `local_tools` value is false. `{ "reset": true }` needs no other fields and clears the defaults.

`GET /api/schedules` lists every schedule of the selected team and includes `target_name`. A schedule belongs to one template or one workflow. Creating one needs `name`, an IANA `timezone`, `start_date` (`YYYY-MM-DD`), `start_time` (`HH:MM`), and `frequency` (`once`, `hourly`, `daily`, `weekly`, `monthly`). Weekly also needs `weekdays` (`MO` through `SU`). `end_kind` is `never`, `after` with `end_count` from 1 to 999, or `on_date`. The clock time stays in that zone across daylight saving. `POST /api/schedules/preview` returns up to five upcoming UTC instants and does not save anything. A viewer can list. An operator can create, change, delete, and pause. `POST /api/schedules/{id}/enabled` takes `{ "enabled": true }`. `POST /api/schedules/{id}/missed` takes `{ "accept": true }` to start the last missed time once, or `{ "accept": false }` to clear it. A failed start leaves that missed time in place. Creating a schedule sets `created_by` and `updated_by` to the signed-in user. Changing one sets `updated_by` to the editing user and leaves `created_by` as it was. Automatic runs start as `updated_by` only when that user may still start jobs for the team. Otherwise no job starts, the time stays missed, and `launch_block` is `schedule_needs_a_new_owner`. Times that passed while the process was stopped are not replayed.
