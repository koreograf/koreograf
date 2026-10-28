# Logs and Prometheus

Koreograf does not open a syslog socket. Every event is one line on stdout. journald, Docker, and Promtail read that stream. `KOREOGRAF_LOG_FORMAT=json` writes the same fields as JSON. Without the variable the format stays text.

The lines contain no credentials, no tokens, and no extra variables.

| Target | When | Fields |
| --- | --- | --- |
| `koreograf::job` | job started and finished | `job_id`, `team_id`, `tool`, `playbook`, `template`, and at the end `status` and `duration_ms` |
| `koreograf::cache` | when a job starts | collection and role counts, bytes for collections, roles, and OpenTofu |
| `koreograf::role` | Ansible role finished, and immediately on `failed` or `unreachable` | `playbook`, `role`, `result`, or the counters `ok`, `changed`, `failed`, `unreachable`, `skipped` |

For Ansible, `playbook` is the playbook path. For shell and OpenTofu it is the entry path. Koreograf reads roles from Ansible's default callback output (`TASK [role : task]`, `RUNNING HANDLER`, `ok:`, `changed:`, `fatal:`, `unreachable:`, `skipping:`). `KOREOGRAF Role from cache` means the role was already in the cache. A task name that contains ` : ` outside a role can look like a role.

`GET /metrics` is open, like `/healthz`, and returns Prometheus text. Counters live in memory and start at zero after a restart. Prometheus treats that jump as a restart. Cache numbers are read from disk on each scrape, even when no job is running. The time of a role is the log line. `koreograf_role_results_total` counts results and has no timestamp.

| Metric | Type | Labels |
| --- | --- | --- |
| `koreograf_jobs_started_total` | counter | `tool`, `playbook` |
| `koreograf_jobs_finished_total` | counter | `tool`, `playbook`, `status` (`successful`, `failed`, `canceled`) |
| `koreograf_role_results_total` | counter | `playbook`, `role`, `result` (`ok`, `changed`, `failed`, `unreachable`, `skipped`) |
| `koreograf_cache_collections` | gauge | `team_id`, `team` |
| `koreograf_cache_roles` | gauge | `team_id`, `team` |
| `koreograf_cache_bytes` | gauge | `team_id`, `team`, `kind` (`collections`, `roles`, `tofu`) |

`koreograf_jobs_started_total` increases when execution begins, after preparation. `koreograf_jobs_finished_total` increases once for every job that reaches `successful`, `failed`, or `canceled`. That includes a job canceled while it is still pending and a job that fails during preparation, so finished can be larger than started. The `playbook` label is the template entry path. An empty path is `-`.
