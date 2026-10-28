# API

`GET /api/version` does not need a session. It returns the version compiled from `Cargo.toml`. Session cookie `koreograf_session`, HttpOnly, SameSite=Lax, seven days. `Secure` is added when `KOREOGRAF_COOKIE_SECURE` is `true`, and when it is `auto` and a trusted proxy sends `X-Forwarded-Proto: https`. The default is `auto` with that proxy off, so a plain HTTP sign-in still receives the cookie. Team routes need `X-Team-Id`. Job events use the team membership stored on the job, because `EventSource` cannot set custom headers. The stream is `GET /api/jobs/{id}/events?after=`.

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

`/output` is the kept log: the first three quarters of the line limit and the last quarter, for stdout and for stderr. The default limit is 8000 lines. The download stops at 8 MiB.

`POST /api/login` takes `username` and `password` and returns 200 with the session cookie. A rejected sign-in returns 401 and `invalid_username_or_password` for a wrong password, an unknown user, an empty password, and a directory failure. A request without a session still returns `sign_in_is_required`. Creating a credential needs an owner. Launch returns 202 and needs a JSON object. `extra_vars` is that object, or `{}`.

On a machine without Compose, read `data/initial-admin-password` instead of `docker compose exec`, and use the same JSON.

## Request bodies

Names are at most 120 bytes. Empty optional strings are omitted.

### Projects

| Field | Required | Notes |
| --- | --- | --- |
| `name` | yes | |
| `source_type` | yes | `path` or `git` |
| `source` | yes | Directory inside the server, or a git address |
| `revision` | no | Branch, tag, or commit. Empty follows the default branch |
| `organization_id` | no | Defaults to the organization of `X-Team-Id` |
| `team_ids` | no | Empty gives the project to every team in the organization |

`PUT /api/projects/{id}` sets `organization_id` and `team_ids`. Path projects are described in [Jobs](jobs.md#path-projects).

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
| `netbox` | `name`, `api_endpoint`, `token`, and optionally `validate_certs` (default true), `ca_file` (absolute PEM path, empty uses the system CAs), `group_by`, `query_filters`, `virtual_machines` (default true) |

`POST /api/inventories/import` takes `project_id`, `path`, and an optional `name`. `POST /api/inventories/netbox` takes the same NetBox fields as a source, with `name` required.

### Credentials

`POST /api/credentials` takes `name`, `kind`, and `secret`. One credential of each kind can be attached to a template. Vault may repeat when each `vault_id` is unique, and only one vault credential may omit the id. `credential_ids` on a template is the list. `credential_id` is the single-id form.

| Kind | `secret` fields |
| --- | --- |
| `ssh` | `private_key` or `password`, one of them required. Optional `username`, `passphrase`, `become_method` (`sudo`, `su`, `pbrun`, `pfexec`, `doas`, `dzdo`, `ksu`, `runas`), `become_username`, `become_password` |
| `vault` | `password` required. Optional `vault_id` |
| `scm` | `private_key` or `password`. Optional `username`, `passphrase`. Not used by project Sync |
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
| `netbox` | `token`. Optional `url` |

`PUT /api/credentials/{id}` takes `name`, an optional `secret` object, and optional `clear` (names of secret fields to drop). Omitting a stored secret keeps it.

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
| `environment_id` | no | |
| `entrypoint` | for ansible and opentofu | Playbook path, or the OpenTofu module directory |
| `extra_vars` | no | Object. Default `{}` |
| `spec` | no | Object. Default `{}` |
| `update_on_launch` | no | Default true. Git projects only |
| `prompt_on_launch` | no | Default false. The launch dialog asks for inventory, credentials, limit, tags, skip tags, job type, verbosity, and extra variables |

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

### Launch

`POST /api/templates/{id}/launch` and `POST /api/workflows/{id}/launch` take `{"extra_vars": {}}`. The value is an object. A template launch may also send `inventory_id`, `credential_ids`, and an Ansible `spec` with `job_type`, `tags`, `skip_tags`, `limit`, and `verbosity`. Omit a field to keep the template value. An empty `inventory_id` clears it. An empty string in those spec fields clears that field. `extra_vars` is still merged. Those three fields are accepted only when the template has `prompt_on_launch` set. Otherwise the launch is rejected. A body that sends none of them keeps the template, including for hooks and scripts. The template row is not changed. Workflow launch ignores those three fields and uses the graph.

| Method | Path |
| --- | --- |
| POST | `/api/login`, `/api/logout` |
| GET, POST | `/api/users` |
| GET, PUT, DELETE | `/api/users/{id}` |
| GET, PUT | `/api/me` |
| GET | `/api/overview`, `/api/teams` |
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
| POST | `/api/inventories/import`, `/api/inventories/netbox` |
| POST | `/api/inventories/{id}/sync` |
| POST | `/api/inventories/{id}/sources` |
| PUT, DELETE | `/api/inventories/{id}/sources/{source_id}` |
| POST | `/api/inventories/{id}/sources/{source_id}/sync` |
| GET | `/api/cache`, `/api/plugins` |
| GET, POST | `/api/templates`, `/api/workflows`, `/api/hooks`, `/api/jobs` |
| PUT, DELETE | `/api/templates/{id}`, `/api/workflows/{id}` |
| POST | `/api/templates/{id}/launch`, `/api/workflows/{id}/launch` |
| GET, POST | `/api/templates/{id}/schedules`, `/api/workflows/{id}/schedules` |
| GET | `/api/schedules` |
| POST | `/api/schedules/preview` |
| PUT, DELETE | `/api/schedules/{id}` |
| POST | `/api/schedules/{id}/enabled`, `/api/schedules/{id}/missed` |
| GET | `/api/jobs/{id}`, `/api/jobs/{id}/output`, `/api/workflows/{id}`, `/api/workflow-jobs/{id}` |
| POST | `/api/jobs/{id}/cancel`, `/api/jobs/{id}/relaunch`, `/api/hooks/{id}/test` |
| GET | `/api/hooks/{id}/deliveries` |
| DELETE | `/api/hooks/{id}` |
| POST | `/hooks/{id}/{token}` |
| GET | `/healthz`, `/metrics` |
| GET, POST | `/api/admin/backup` |
| GET, PUT | `/api/settings` |

`GET /api/inventories/{id}` returns `hosts`, `host_count`, `source_kind`, and `source`. Groups with `:vars` or `:children` are not hosts. A git sync that fails answers with the last lines from git. `contents` answers with `playbooks` and `inventories`. The NetBox fetch happens before the save.

Launches answer 202 and need a JSON object, at least `{}`. An array is rejected. `extra_vars` is an object. A request without a JSON content type is `json_body_required`. An empty body, broken JSON, a non-object, or a field of the wrong type is `json_body_invalid`. A body over 2 MiB is `request_too_large` with status 413. `POST /hooks/{id}/{token}` accepts any JSON value.

`GET /api/schedules` lists every schedule of the selected team and includes `target_name`. A schedule belongs to one template or one workflow. Creating one needs `name`, an IANA `timezone`, `start_date` (`YYYY-MM-DD`), `start_time` (`HH:MM`), and `frequency` (`once`, `hourly`, `daily`, `weekly`, `monthly`). Weekly also needs `weekdays` (`MO` through `SU`). `end_kind` is `never`, `after` with `end_count` from 1 to 999, or `on_date`. The clock time stays in that zone across daylight saving. `POST /api/schedules/preview` returns up to five upcoming UTC instants and does not save anything. A viewer can list. An operator can create, change, delete, and pause. `POST /api/schedules/{id}/enabled` takes `{ "enabled": true }`. `POST /api/schedules/{id}/missed` takes `{ "accept": true }` to start the last missed time once, or `{ "accept": false }` to clear it. A failed start leaves that missed time in place. Creating a schedule sets `created_by` and `updated_by` to the signed-in user. Changing one sets `updated_by` to the editing user and leaves `created_by` as it was. Automatic runs start as `updated_by` only when that user may still start jobs for the team. Otherwise no job starts, the time stays missed, and `launch_block` is `schedule_needs_a_new_owner`. Times that passed while the process was stopped are not replayed.
