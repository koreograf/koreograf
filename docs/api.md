# API

Session cookie `koreograf_session`, HttpOnly, SameSite=Lax, seven days. Team routes need `X-Team-Id`. Job events use the team membership stored on the job, because `EventSource` cannot set custom headers. The stream is `GET /api/jobs/{id}/events?after=`.

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
| GET | `/api/jobs/{id}`, `/api/jobs/{id}/output`, `/api/workflows/{id}`, `/api/workflow-jobs/{id}` |
| POST | `/api/jobs/{id}/cancel`, `/api/jobs/{id}/relaunch`, `/api/hooks/{id}/test` |
| GET | `/api/hooks/{id}/deliveries` |
| DELETE | `/api/hooks/{id}` |
| POST | `/hooks/{id}/{token}` |
| GET | `/healthz`, `/metrics` |
| GET, POST | `/api/admin/backup` |
| GET, PUT | `/api/settings` |

`PUT /api/projects/{id}` sets `organization_id` and `team_ids`. An empty list gives the project to every team in the organization. `GET /api/inventories/{id}` returns `hosts`, `host_count`, `source_kind`, and `source`. Groups with `:vars` or `:children` are not hosts. A git sync that fails answers with the last lines from git. Allowed addresses are `http(s)`, `ssh`, `git@`, and `file`. `contents` answers with `playbooks` and `inventories`. `import` takes a static file. `netbox` expects `name`, `api_endpoint`, `token`, and optionally `group_by`, `query_filters`, `validate_certs`, and `virtual_machines`. The fetch happens before the save.

Launches answer 202 and need a JSON body, at least `{}`. `extra_vars` is an object.
