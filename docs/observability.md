# Logs and Prometheus

Koreograf does not open a syslog socket. Every event is one line on stdout. journald, Docker, and Promtail read that stream. `KOREOGRAF_LOG_FORMAT=json` writes the same fields as JSON. Without the variable the format stays text.

The lines contain no credentials, no tokens, and no extra variables.

| Target | When | Fields |
| --- | --- | --- |
| `koreograf::job` | job started and finished | `job_id`, `team_id`, `tool`, `playbook`, `template`, and at the end `status` and `duration_ms` |
| `koreograf::cache` | when a job starts | collection and role counts, bytes for collections, roles, and OpenTofu |
| `koreograf::role` | Ansible role finished, and immediately on `failed` or `unreachable` | `playbook`, `role`, `result`, or the counters `ok`, `changed`, `failed`, `unreachable`, `skipped` |

For Ansible, `playbook` is the playbook path. For shell and OpenTofu it is the entry path. Koreograf reads roles from Ansible's default callback output (`TASK [role : task]`, `RUNNING HANDLER`, `ok:`, `changed:`, `fatal:`, `unreachable:`, `skipping:`). `KOREOGRAF Role from cache` means the role was already in the cache. A task name that contains ` : ` outside a role can look like a role.

The dashboard reads the stored jobs in SQLite for its daily series; it does not scrape `/metrics`.

`GET /metrics` returns Prometheus text. While `KOREOGRAF_METRICS_TOKEN` is empty or unset, only a client whose TCP peer address is loopback may scrape it. That keeps an in-process scrape and a sidecar or Prometheus in the same network namespace working on `127.0.0.1` or `::1`. A request from another address gets `401` with a keyed JSON error, even when it sends `X-Forwarded-For: 127.0.0.1`; Koreograf decides from the peer address, not from a forwarded header. Set `KOREOGRAF_METRICS_TOKEN` for a Prometheus server on another host. A scrape must then send `Authorization: Bearer <token>` or use an administrator session cookie; otherwise Koreograf answers `401` with a keyed JSON error. The token applies to loopback too. A regular administrator API token from the user menu does not satisfy this check; it gets the same `401` unless it matches the metrics token. `/healthz` stays open. The token never appears in the metrics text.

```bash
curl -H "Authorization: Bearer $KOREOGRAF_METRICS_TOKEN" http://127.0.0.1:8080/metrics
```

If the UI port is reachable from a wider network, an internal-only `KOREOGRAF_BIND` behind a reverse proxy is the alternative to a token.

Counters live in memory and start at zero after a restart. Prometheus treats that jump as a restart. Cache numbers are read from disk on each scrape, even when no job is running. The time of a role is the log line. `koreograf_role_results_total` counts results and has no timestamp.

A team owner can clear the cache from the dashboard or with `DELETE /api/v1/cache`. That removes `data/cache/<team>/collections`, `roles`, and `tofu`, so the three cache gauges fall to zero on the next scrape. `data/cache/<team>/tofu-state` and unrecognised entries stay, and a pending or running job of that team blocks the clear with `409 a_job_is_running`. Maintenance mode also blocks it. The action appends a `delete` entry on `cache` to the activity log; a refusal appends nothing. The activity log is separate from this scrape output.

| Metric | Type | Labels |
| --- | --- | --- |
| `koreograf_jobs_started_total` | counter | `tool`, `playbook` |
| `koreograf_jobs_finished_total` | counter | `tool`, `playbook`, `status` (`successful`, `failed`, `canceled`) |
| `koreograf_job_duration_seconds` | histogram | `tool`, `playbook`, `status`; bucket `le` |
| `koreograf_jobs_pending` | gauge | none |
| `koreograf_jobs_running` | gauge | none |
| `koreograf_role_results_total` | counter | `playbook`, `role`, `result` (`ok`, `changed`, `failed`, `unreachable`, `skipped`) |
| `koreograf_host_last_run_timestamp_seconds` | gauge | `host`, `playbook` |
| `koreograf_host_last_run_success` | gauge | `host`, `playbook` |
| `koreograf_cache_collections` | gauge | `team_id`, `team` |
| `koreograf_cache_roles` | gauge | `team_id`, `team` |
| `koreograf_cache_bytes` | gauge | `team_id`, `team`, `kind` (`collections`, `roles`, `tofu`) |

`koreograf_jobs_started_total` increases when execution begins, after preparation. `koreograf_jobs_finished_total` increases once for every job that reaches `successful`, `failed`, or `canceled`. That includes a job canceled while it is still pending and a job that fails during preparation, so finished can be larger than started. The `playbook` label is the template entry path. An empty path is `-`.

`koreograf_job_duration_seconds` measures the time from the start of execution to the terminal status, in seconds. Its buckets are `1`, `5`, `15`, `30`, `60`, `120`, `300`, `600`, `1800`, `3600`, `7200`, `21600`, and `+Inf`. A job that is canceled while pending or fails during preparation has no duration, so it only appears in `koreograf_jobs_finished_total`.

`koreograf_jobs_pending` is read from the database on each scrape and counts jobs with status `pending`, across every team and including jobs launched by a workflow. `koreograf_jobs_running` counts the jobs this process has begun and not yet finished; both gauges are unlabelled and read zero after a restart until work arrives.

`koreograf_host_last_run_timestamp_seconds` is the finish time of the latest Ansible job that reported a result for that host and playbook. `koreograf_host_last_run_success` is `1` when that job had no `failed` or `unreachable` result on the host, otherwise `0`. A host that appears in several plays of one job is one series. The host series carry no role or task label, so a host that runs many roles still produces one timestamp and one status per playbook.

Both host gauges live in memory and start empty after a restart, like the role counter. The process keeps the 1000 most recently updated host and playbook pairs and drops the least recently updated pair when a new one arrives, so deleting an inventory or a host removes its series once the bound evicts it. A team with more than 1000 host and playbook pairs sees the least recently updated pairs disappear from `/metrics`; split playbooks rather than adding per-host role labels.

The per-host history that backs the UI is stored in SQLite and PostgreSQL in `host_results`. `GET /api/v1/inventories/{id}/hosts/{name}/history?page=1&page_size=25` returns the latest runs for one host in the signed-in team, including playbook, role, result, counters, time, and job id and name. It uses the same team check as the other inventory routes, so a viewer of another team gets `404`. Rows stay after an inventory or a job is deleted, because the table has no foreign key and no retention job; the host endpoint is unreachable once its inventory is gone.

Example alerts:

```yaml
groups:
  - name: koreograf-hosts
    rules:
      - alert: HostNotPatched
        expr: time() - koreograf_host_last_run_timestamp_seconds{playbook="patch.yml"} > 7 * 24 * 3600
        for: 1h
        labels:
          severity: warning
        annotations:
          summary: "{{ $labels.host }} has not run {{ $labels.playbook }} for 7 days"
      - alert: HostLastRunFailed
        expr: koreograf_host_last_run_success == 0
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "The last {{ $labels.playbook }} run on {{ $labels.host }} failed"
```

`HostNotPatched` only fires for a host that has run the playbook at least once, because a missing series has no value. Use `absent()` or a separate inventory export when a host must be covered before its first run. `HostLastRunFailed` stays firing until the next run on that host and playbook succeeds.
