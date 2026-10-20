# Workflows and webhooks

## Workflows

The editor draws nodes and edges. Add node stays in the side panel after the first node. A run lists each node's status, start, end, and duration under the canvas. Clicking a node opens that job's output on top of the workflow. The run links back to the workflow so it can be edited again. A node can override the template's inventory, credential, extra variables, and the Ansible or OpenTofu options. Another credential replaces only the same kind, so a different SSH login keeps the vault passwords. Clearing the credential removes every credential for that node. Untouched fields keep the template value, so one template can patch a test inventory and then the production inventory. A selected node chooses one target for success, one for failure, and one for always. The same links can be drawn by clicking success or failure on the card and then the target node. An edge applies on `success`, `failure`, or `always`. A node starts on `any` as soon as one matching edge is ready, or on `all` when every matching incoming edge is satisfied. A failure counts as handled when a `failure` or `always` edge leaves that node. The API rejects cycles. An empty graph can be saved and cannot be started.

Cancel sets the job to `canceled` and stops the controller process. The workflow treats that like a failure so a failure edge can continue. A task already started on a remote host is not stopped; see [Jobs](jobs.md).

## Webhooks

`POST /hooks/{id}/{token}` is public. The token is the authentication. Alertmanager reads `alerts[].labels.alertname` and `status`. Grafana reads the older format with `state` and `ruleName`. Otherwise an `alertname` field is enough. `alerting` becomes `firing`, and `ok` becomes `resolved`.

A match on alert name and status (`firing`, `resolved`, `any`) starts the template or the workflow. At most 10 starts per call. A valid token answers 200 even when no alert matches. A wrong token is 401. An unknown id is 404.

Variables are built in this order: template extra variables, then the hook's variables, then the event (`alertname`, `status`, `instance`, `summary`, `event`). A hook `heal_action` does not override a label of the same name.

Alertmanager:

```yaml
receivers:
  - name: koreograf
    webhook_configs:
      - url: http://koreograf.example.com/hooks/HOOK_ID/TOKEN
        send_resolved: true
```

Grafana, contact point Webhook, same URL. In the UI an operator creates the hook and sees the finished URL. `Test` (German: `Test-Alert`) sends an Alertmanager body with the stored alert name.

## Demo

The playbooks, the role, and the scripts are in [examples/demo](../examples/demo).

1. Sign in as `admin`. The password comes from the environment or from `data/initial-admin-password`.
2. Select the team `Plattform`.
3. On the overview, press `Test alert` (German: `Test-Alert`).
4. The workflow `Self-Heal Ablauf` starts. The health check is shell. On success the Ansible role `incident_report` runs on the controller and writes a report into the job directory. On failure, `escalate.sh` runs.
5. The role deletes only file names that the hook lists in `incident_report_cleanup_files`, and only under the job work directory. The demo hook sets `heal_action` to `cleanup-tmp` and the file `stale.tmp`.

A second Ansible run shows the role coming from the cache in the log.
