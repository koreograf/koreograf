# Workflows and webhooks

## Workflows

The editor draws nodes and edges. The list creates a workflow in an overlay for the name and description, and saving opens the editor. The editor page keeps the canvas and inspector in the **Editor** tab, the run history in the **Runs** tab, the schedules in the **Schedules** tab, and the delete action in the **Danger Zone** tab for operators; `?tab=runs`, `?tab=schedules`, and `?tab=danger` open a tab directly. The run view has no schedules or Danger Zone tab. `#/workflows?new=1` opens the same overlay after a reload, and Back closes it. The canvas fits the whole graph when it opens. On a narrow screen the page keeps its width and only the graph pans and zooms; the node panel opens as a bottom sheet and collapses to give the canvas more room. The `+`, `−`, **Fit**, and **100 %** controls, Ctrl+wheel, and the `+`, `−`, `0`, and `F` keys zoom, fit, and reset; drag empty canvas to pan, and a minimap appears for more than twelve nodes. An empty canvas offers **Add first step** in the middle; it opens the same full node fields in an overlay. Right-click empty canvas to add a node; the full node fields open in an overlay, and the node lands near the click. With the canvas focused, the context menu key or Shift+F10 opens the same menu. Each node shows success, failure, and always output handles; the plus on a handle opens the same overlay and inserts the new node already connected with that type. Each card also shows the tool icon, the template name, a badge for a validation problem, and a chip for each active override: limit, tags, inventory, or an OpenTofu apply. Dropping a dragged connection on empty canvas opens the overlay too and connects the new node on success. Right-click a node or use its menu to edit, delete, or connect it; editing opens the same full node fields in an edit overlay with Save and Cancel, and Enter or a double-click on a focused node opens it too. A left click only selects the node and shows a read-only summary in the side panel. Selecting an edge shows its details there, and Edit edge opens an overlay to change its type or delete it. A run lists each node's status, start, end, and duration under the canvas. Each node also shows a status icon, its status text, and its duration, and the legend includes skipped. Taken edges stay bright and untaken edges dim; a successful run with a handled failure says so in the run summary. Clicking a node opens that job's output on top of the workflow; the first error starts above the collapsed setup output, and Escape closes the overlay and returns focus to the node. The run links back to the workflow once, and a running run can be canceled there while a finished run can be relaunched as a whole. The workflow list shows the last run's status and time. A node can override the template's inventory, credential, extra variables, and the Ansible or OpenTofu options. Another credential replaces only the same kind, so a different SSH login keeps the vault passwords. Clearing the credential removes every credential for that node. Untouched fields keep the template value, so one template can patch a test inventory and then the production inventory. The Connections section lists each outgoing edge in its own row; change its target there or remove it, and add another edge with a type, a target, and **Add edge**. The primary way to wire an edge is to right-click a node (or open its menu button) and choose **Connect**, then click the target node or drag from the source to it. A dialog then asks whether the edge starts on success, failure, or always. Self-links are refused, and the same source, target, and `when` is rejected. An edge applies on `success`, `failure`, or `always`. Each edge carries an arrow, a type chip, and a stroke style that also works without colour. Click near the line to select it. The inspector names the source and target, and either name selects that node; it can also change the type or delete the edge. Tab to a node or edge; Enter or Space selects it, and Delete or Backspace removes the focused edge or node without a dialog and offers **Undo** in a toast. Ctrl/Cmd+Z undoes graph changes, Ctrl/Cmd+Shift+Z or Ctrl+Y redoes them, the Undo and Redo toolbar buttons do the same, and `?` lists the shortcuts. In connect mode, Enter or Space on the target finishes the connection and Escape cancels. The browser tab names the workflow, and a run names itself there too. A node with two or more predecessors shows the convergence control: it starts on `any` as soon as one matching edge is ready, or on `all` when every predecessor satisfies its edge condition, where a skipped branch counts as not met. The node carries an `ALL` or `ANY` badge on its input side when the control applies. A failure counts as handled when a `failure` or `always` edge leaves that node. The API rejects cycles. An empty graph can be saved and cannot be started.

Cancel sets the job to `canceled` and stops the controller process. The workflow treats that like a failure so a failure edge can continue. That edge can start during the two seconds between `SIGTERM` and `SIGKILL`. A task already started on a remote host is not stopped; see [Jobs](jobs.md).

A workflow can also start from a schedule in the workflow page's **Schedules** tab, which links as `#/workflows/<id>?tab=schedules`. The delete action is in the **Danger Zone** tab for operators. The form, the missed-run question, and the check interval are in [Schedules](jobs.md#schedules).

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

Grafana, contact point Webhook, same URL. In the UI an operator creates the hook and sees the finished URL. `Test` (German: `Test-Alert`) sends an Alertmanager body with the stored alert name. That button runs as the signed-in user. A public call stays anonymous, so it does not start a local job while local runs are limited to administrators. Event values reach Ansible as unsafe strings.

## Demo

The playbooks, the role, and the scripts are in [examples/demo](../examples/demo).

1. Sign in as `admin`. The password comes from the environment or from `data/initial-admin-password`.
2. Select the team `Plattform`.
3. On the overview, press `Test alert` (German: `Test-Alert`).
4. The workflow `Self-Heal Ablauf` starts. The health check is shell. On success the Ansible role `incident_report` runs on the controller and writes a report into the job directory. On failure, `escalate.sh` runs.
5. The role deletes only file names that the hook lists in `incident_report_cleanup_files`, and only under the job work directory. The demo hook sets `heal_action` to `cleanup-tmp` and the file `stale.tmp`.

A second Ansible run shows the role coming from the cache in the log.
