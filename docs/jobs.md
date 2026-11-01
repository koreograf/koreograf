# Jobs

## What the cache changes

Collections, roles, and OpenTofu providers stay in a cache.

AWX installs collections on every run because the execution environment is ephemeral. Koreograf keeps the Galaxy and role cache under `data/cache/<team>/` and mounts it into every run. A second run logs which roles are already cached. If `ansible-galaxy` fails, the job continues with the cache it already has.

OpenTofu uses `TF_PLUGIN_CACHE_DIR` on that same cache (`data/cache/<team>/tofu`). `TF_DATA_DIR` stays in the job directory. The state file does not. A module with no backend block gets a local backend whose path is `data/cache/<team>/tofu-state/<template>/terraform.tfstate`. The next `plan` or `apply` of that template sees the resources from the last run. Another team has its own directory, so the state does not cross `team_id`. The directory is mode `0700` and the file is mode `0600`. An execution environment mounts that one template directory, and only when the job is OpenTofu. It is part of the cache, so the backup zip leaves it out. A remote backend is the setup to use when the state must outlive this data directory or be used from more than one machine. `apply` writes a warning when the module has no remote backend. A module that already sets `backend "local"` keeps that path, and a relative path is still removed with the job directory. A module that sets any other backend keeps its own state, and this controller does not copy it.

## Job log

Each of stdout and stderr keeps the first three quarters of the line limit and the last quarter. The tail lines wait in memory and stay within 8 MiB per stream. The default is 8000 lines: the first 6000 and the last 2000. A system line between them records how many lines were left out. `KOREOGRAF_JOB_OUTPUT_LINES` sets the limit, from 100 to 100000. Settings can replace that value for the next job. `GET /api/jobs/{id}/output` downloads the kept log and stops at 8 MiB. The job page shows the start and the end when the stored log is longer than 2000 lines. Play recap lines and `fatal:` lines are read from the whole stream, including lines the log does not keep.

On the job page, `fatal:`, `UNREACHABLE!`, `ERROR!`, and a recap line with `failed` or `unreachable` above zero are marked as the failure. The first of those lines stays pinned at the top of the log. Output from `ansible-galaxy`, including its `ERROR!` lines, stays a warning: the run continues with the cache. A successful job does not take the failure color from that Galaxy line.

## Prompt on launch

A template can ask before it starts. The launch button then opens a dialog filled with the template's inventory, credentials, limit, tags, skip tags, job type, verbosity, and extra variables. Cancel starts nothing. Confirm runs that one job with the dialog values and leaves the template row unchanged. An Ansible template without prompt on launch also offers Launch with options for operators. That action opens the same dialog and sends `launch_with_options: true` with the run overrides. `POST /api/templates/{id}/launch` with only `extra_vars`, or with none of the prompt fields, still uses the template. The same applies to hooks and scripts. Inventory, credentials, and the Ansible fields above are accepted only when prompt on launch is on, or when that explicit flag asks for the options run.

## Execution environments

An environment is an image plus a pull policy (`if_not_present`, `always`, `never`) and an optional `network`. It has one of three scopes. **Global** is visible to every team, and only an administrator can create or change it. **Organization** is visible to every team in that organization. An owner of a team in the organization, or an administrator, can manage it. The organization is the organization of the team selected in the UI. **Team** stays private to that one team, and an owner of that team manages it. A template may use any environment the selected team can see. The demo creates `quay.io/ansible/awx-ee:latest` on the demo team and attaches it to no template, so the first run works without an image pull.

The Runtime page lists environments first and shows who each one applies to. `GET` and `PUT /api/environments/{id}` read and change the name, image, pull policy, network, description, and scope. Omitting `scope` on create stores a team environment. Omitting it on update keeps the current scope. The image may be a tag or a digest reference (`name@sha256:…`). Collections, roles, and the OpenTofu cache stay under `data/cache/<team>/` for that team.

Without a `network` the job container gets the daemon's default bridge. A target that only lives in a Compose network or another user-defined network is then unreachable. Its service name resolves through the upstream DNS and times out, and an address in that subnet times out too. Set `network` to a network the daemon knows. The Compose default network is the project name plus `_default`, for example `koreograf_default`, and that is the recommended value. It reaches the targets by service name and does not tie the job to this container. `container:<name>` shares the network stack of another running container; it reaches everything that container reaches, including its published ports, but the job inherits that container's addresses and fails when the container is recreated under another name or is not running. Only a plain name (letters, digits, `.`, `_`, `-`) or `container:<name>` is accepted. An empty value keeps the default. The value is one argument to `docker run` or `podman run`, so it cannot add another option. The mount probe still runs with `--network none`.

Only an administrator may set `host` or `container:<name>`. The form and the API refuse it for every other account, including a team owner. `host` puts the job in the host network namespace, so the job reaches every service the host reaches and loses the isolation of the bridge. `container:<name>` gives the job the addresses and published ports of another container and follows that container's lifecycle. Both depend on the host and are therefore administrator-only. A non-administrator keeps plain named networks plus `bridge` and `none`. Under `KOREOGRAF_LOCAL_RUNNER_FOR=admins` a stored environment with a privileged network is also refused at launch when a non-administrator starts the job, even though that account only attached an existing environment.

In the container the runner mounts the project read-only at `/runner/project`, the work directory at `/runner/work`, and the cache at `/cache`. The mount source has to be a path the container daemon can see. On the host that is the data directory. When Koreograf itself runs in a container, a path inside that container is not a daemon path. The source is then the daemon path of the data volume. Koreograf reads that path from its own container. `KOREOGRAF_DATA_HOST` replaces it when the lookup is wrong. `--env-file` is not one of those mounts. This process reads the file and passes the values, so that path stays inside the Koreograf container.

The work directory contains the decrypted credential files for that job, including `container.env`. They are mode `0600` and owned by the user the container runs as, so the job can read them. They are removed when the job ends. The job container does not receive the Docker socket.

The container runs as the numeric owner of the work directory, so an image whose default user is not root can still create `/runner/work/tmp` when this process is root. A rootless Podman or Docker daemon maps that host user to container uid 0, so the job then uses `0:0`. Secret files stay mode `0600` and are not given to another user. The container is started with `--security-opt no-new-privileges` and `--cap-drop ALL`. Without `CAP_DAC_OVERRIDE` and `CAP_CHOWN`, a job running as `0:0` cannot write image paths owned by another uid, such as `/runner` in `awx-ee`. The image gets a cleaned environment: `PATH` is local only, `HOME` is the work directory, `LANG=C.UTF-8`, and the `KOREOGRAF_*` variables. The server environment is not inherited. A configured HTTP proxy is added as `http_proxy` and `https_proxy`. That file is mode `0600` because the proxy password can sit in those values.

Runner modes are in [Configuration](configuration.md#environment-variables).

## Tools

The registry lives in `runner::tools()`. Another CLI tool needs an entry there and a branch in `src/wrapper.sh`. The three built-in tools:

| Id | Run |
| --- | --- |
| `ansible` | `ansible-playbook`, Galaxy into the team cache |
| `opentofu` | `tofu init`, then `plan` or `apply` |
| `shell` | a script from the project, or `spec.script` written as `inline.sh` |

A template stores one credential of each kind. Ansible Vault is the exception: several vault credentials can be attached, each with its own vault id, and only one of them may omit the id. The kinds follow the built-in types in AWX and the two useful Semaphore key-store types (SSH, and login with password). Semaphore's empty "None" type is left out. There is no custom credential-type designer, and no external lookup such as CyberArk, HashiCorp Vault, or a cloud secret manager.

| Kind | What the job receives |
| --- | --- |
| `ssh` | Machine login. A private key, a password, or both. Optional username, key passphrase, and become (`sudo`, `su`, `pbrun`, `pfexec`, `doas`, `dzdo`, `ksu`, `runas`). The password is a file with mode `0600`, not a command argument. |
| `vault` | Ansible Vault password, and an optional vault id. |
| `scm` | Git username and password, or an SSH key, for commands inside the job. The project Sync button does not use this credential. |
| `net` | Network username, password, optional key, and an enable password. |
| `aws`, `azure`, `gce`, `vmware`, `openstack` | Cloud credentials as environment variables. Google is a service-account file. |
| `registry` | Host, username, and password visible to the job. Koreograf does not run `docker login`. |
| `github`, `gitlab`, `galaxy`, `kubernetes`, `insights` | A token, or a username and password, for that service. |
| `env` | Extra environment variables as `KEY=value` lines. |
| `netbox` | A token, and an optional API address. The token stays on the credential, not in the inventory source. |

Secrets are stored with AES-GCM. The list API returns the name and the kind, never the secret. Opening one credential returns public fields such as the username and the names of stored secrets, still without the secret values. An owner can change the name and the secret. A stored password, private key, passphrase, or become password is shown with the usual password dots. Replacing those dots stores the new value. Leaving them, or leaving the field empty, keeps the stored value. `clear` removes named secret fields. The kind stays as it was created. Files written for the job, including the container environment file, use mode `0600`.

## Playbooks and inventory sources

`GET /api/projects/{id}/contents` reads an Ansible project and lists playbooks. A file counts when it ends in `.yml` or `.yaml`, the first content line is a list, and `hosts:` appears somewhere. `requirements.yml`, `galaxy.yml`, and role directories such as `tasks`, `handlers`, and `group_vars` stay out. A git project has to be synced first. The template offers the files that were found. Another path can still be typed by hand.

### Path projects

`source_type` is `path` or `git`. A path project is a directory on the machine that runs Koreograf. In Docker or Helm that directory is inside the container. A bind mount on the host is not visible until it is mounted into the container.

With Compose, mount the directory read-only and list its parent in `KOREOGRAF_PROJECT_ROOTS`. The stock `docker-compose.yml` does not forward that variable, so both lines have to be added. The [Installation](installation.md#docker-compose) page shows the fragment. Create the project with `source` set to the container path, for example `/srv/projects/my-project`.

The process runs as uid 10001 since #101. A bind-mounted directory, and any `file://` git repository inside it, must be owned by uid 10001 or readable for it; otherwise git aborts with `dubious ownership`. Own it with `chown -R 10001:10001 /srv/projects/my-project`. `git config --global --add safe.directory /srv/projects/my-project` is the alternative if the maintainers prefer it.

The Helm chart has no extra-volume value. Add a volume, a volume mount, and `KOREOGRAF_PROJECT_ROOTS` on the deployment. The examples directory in the image is always an allowed root, which is why the seeded Demo project runs without an extra mount.

An administrator can save a path outside the allowed roots. At run time the canonical directory still has to sit under one of those roots, so that saved path does not start until the directory is listed. `/`, `/etc`, `/proc`, `/sys`, `/dev`, `/run`, `/boot`, `/root`, `/var/run`, and the data directory are rejected for everyone. An operator can save a path only when it is already under an allowed root.

A public git project needs no mount. `revision` is optional. An empty revision follows the default branch. A non-empty revision is a branch, tag, or commit: letters, digits, `.`, `/`, `_`, and `-`, at most 200 characters, and it must not start with `-`.

```json
{
  "name": "Koreograf",
  "source_type": "git",
  "source": "https://github.com/koreograf/koreograf.git",
  "revision": "v0.1.0-alpha.4"
}
```

Allowed git addresses are `http(s)`, `ssh`, `git@`, and `file`. A `file://` URL is accepted for an empty host or `localhost` and then follows the same root rules as a path project. The repository must be owned by or readable for uid 10001 as well, or git reports `dubious ownership`; see [Path projects](#path-projects). The project Sync button does not use an `scm` credential. That credential applies to git commands inside a job.

A template updates its git project on every launch. Turn **Update the project on launch** off to run the checkout from the last sync, including when the remote cannot be reached. A revision pinned on the project stays as it is on disk until the next sync. In the German UI the checkbox is **Projekt vor dem Start aktualisieren**. The project still needs one successful sync. Changing the type, source, or revision on the project page discards the old checkout and syncs again. A failed sync keeps the new settings and shows the error. The page also shows the time, commit, and status of the last sync. The Sync button stays available.

Inventory files are named `hosts` or `inventory`, or they live in `inventory/` and `inventories/`. A relative `inventory=` entry under `[defaults]` in `ansible.cfg` is included. `POST /api/inventories/import` stores the file text. INI and the usual YAML inventory show up in the host list. Ansible receives the same text.

A file with `plugin:` is not executed. `plugin: netbox.netbox.nb_inventory` (also `plugin: netbox`) has a NetBox source. The controller fetches devices and, when asked, virtual machines, groups them like the plugin (`sites_berlin`, `device_roles_web`), and stores INI. The token lives on the credential, not in the source. `validate_certs: false` skips certificate checks, writes a warning on each fetch, and shows a badge in the inventory list. `ca_file` is an absolute PEM path on the controller. When it is set, checks stay on even if `validate_certs` is false. A missing or unreadable file fails the fetch. An inventory can have several sources, for example a file from the project and NetBox. Ansible receives each source as its own file. `POST /api/inventories/{id}/sync` refreshes every external source. Other inventory plugins are listed and cannot be executed here.

A job uses the inventory from the last sync. Launch does not call NetBox. Sync the inventory before the job when the host list should be current. With no `query_filters`, every device the API returns is imported. `ansible_host` is the host part of `primary_ip4`, then `primary_ip`, then `primary_ip6`. A null field or an address without a host is skipped, and a prefix length is removed. The remaining text is kept only when it parses as an IP address, and the inventory gets that normalized address. Anything else leaves `ansible_host` unset and logs a warning with the device name. A device with none of those addresses is still listed and has no `ansible_host`. Virtual machines are included unless `virtual_machines` is false. A filter such as `status=active` leaves retired devices out:

```yaml
plugin: netbox.netbox.nb_inventory
api_endpoint: https://netbox.example.com
query_filters:
  - status: active
group_by:
  - sites
  - is_virtual
```

`netbox_id` is the numeric id from the endpoint that returned the host. A device and a virtual machine can share the same number. `group_by` with `is_virtual` adds the groups `is_virtual_true` and `is_virtual_false`. It does not change `netbox_id`.

A group name contains only letters, digits, and underscores. A `-` or `.` in a NetBox slug becomes `_`. Repeated underscores collapse into one, and underscores at either end are removed. The tag `prod-fake` is the group `tags_prod_fake`, and the site `lab.west` is the group `sites_lab_west`. The host name keeps `-` and `.`. An inventory synced before this change receives the new group names on the next sync.

An Ansible template stores the run in `spec`: `job_type` `run` or `check`, `tags`, `skip_tags`, `limit`, `diff`, `verbosity` from 0 to 4, and `forks` from 1 to 50. Check starts `ansible-playbook --check`. When `tags` or `skip_tags` is set and the play reaches hosts without a task other than Gathering Facts, the job stays `successful`. Ansible exits 0. The job page shows a warning and the log repeats it. A tag that runs a task, including a playbook that is only facts plus tasks for that tag, does not warn. A limit that matches no hosts does not use this warning.

`POST /api/jobs/{id}/cancel` sets the job to `canceled` and stops the controller process. A local job is its own process group: the group receives `SIGTERM`, then `SIGKILL` after two seconds if the job has not exited. That second signal also stops a child that ignored `SIGTERM`, including one that closed the job's output pipes. A container job is stopped with `docker kill` or `podman kill`, and the command's result is checked. A container that is already gone counts as stopped. An engine that cannot be reached does not. Output that arrives after the cancel is discarded. The status stays `canceled`. A task Ansible already started on a remote host, such as a long `sleep`, is not stopped from here.

Each job stores the parameters of that launch: template, tool, entry path, project, inventory, each attached credential as name and kind (not the secret), execution environment, spec, and the merged extra variables. The job page has an Output tab and a Parameters tab. A failed or canceled job can be started again from the job list and from the same row as the Output and Parameters tabs. `POST /api/jobs/{id}/relaunch` uses the stored parameters. When local runs are limited to administrators, that check uses the stored parameters rather than only the template as it looks now. A stored shell script, entry path, project, or inventory that is not on the current template runs only when the workflow that stored it was last saved by an administrator. Adding `--limit` for failed hosts does not by itself count as a different command. `{"only_failed_hosts": true}` is Ansible-only and sets `--limit` to the hosts that failed or were unreachable. A job that stored no such hosts, including a cancel before any host failed, rejects that flag. A play recap replaces earlier `fatal:` lines. When the recap never arrives, the fatal lines are kept. `GET /api/jobs/{id}/output` downloads the output as text.

## Schedules

The Schedules entry in the menu is a table of every schedule of the selected team: what it runs, how it repeats, the clock time, the next run, and when it ends. Schedules that need an answer are listed first. A switch pauses or resumes one. Edit opens the same form. A template page and a workflow page use the same table for that resource. The form asks for a name, a date, a time, and a repeat in words: once, hourly, daily, weekly, or monthly. Weekly picks weekdays. Monthly uses the day of the month from the start date and skips a month that has no such day. The series ends never, after a count, or on a date. The stored time is a local clock time plus a time zone, so 06:00 stays 06:00 when daylight saving changes. A new schedule, a preview, and a changed start date reject a date more than one year before today. An edit that keeps the stored start date is allowed. A local time that falls into a daylight-saving gap runs shifted forward by the width of the gap, so 02:30 becomes 03:30 when the clocks jump from 02:00 to 03:00. A local time that occurs twice runs once, on the earlier offset.

The process checks about every 15 seconds. A time that passed while Koreograf was stopped is not started. The list names that last missed time and asks whether to run it now. Accept starts one run with the saved template or workflow settings and empty extra variables. Decline starts nothing and clears the question. A failed start leaves the question in place. A successful answer is not asked again. A later time stays where it is. If the previous run of that schedule is still going when the next time arrives, another run starts. The job trigger is `schedule`. A saved edit stores the editing user. Later automatic runs start as that user and use that user's permissions at start time. The list shows that name. Pausing, resuming, and answering a missed run leave it unchanged, and answering still starts as the person who clicks. An automatic run also checks that this user may still start jobs in the team. An administrator may. A missing, disabled, non-member, or viewer account does not: no job starts, that time stays missed, and the list asks for a new editor. Saving the schedule clears that notice. A blank editor is stored as the name `schedule` and is blocked the same way, including rows saved before this check.
