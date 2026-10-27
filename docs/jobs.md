# Jobs

## What the cache changes

Collections, roles, and OpenTofu providers stay in a cache.

AWX installs collections on every run because the execution environment is ephemeral. Koreograf keeps the Galaxy and role cache under `data/cache/<team>/` and mounts it into every run. A second run logs which roles are already cached. If `ansible-galaxy` fails, the job continues with the cache it already has.

OpenTofu uses `TF_PLUGIN_CACHE_DIR` on that same cache. `TF_DATA_DIR` stays in the job directory.

## Execution environments

An environment is an image plus a pull policy (`if_not_present`, `always`, `never`). It has one of three scopes. **Global** is visible to every team, and only an administrator can create or change it. **Organization** is visible to every team in that organization. An owner of a team in the organization, or an administrator, can manage it. The organization is the organization of the team selected in the UI. **Team** stays private to that one team, and an owner of that team manages it. A template may use any environment the selected team can see. The demo creates `quay.io/ansible/awx-ee:latest` on the demo team and attaches it to no template, so the first run works without an image pull.

The Runtime page lists environments first and shows who each one applies to. `GET` and `PUT /api/environments/{id}` read and change the name, image, pull policy, description, and scope. Omitting `scope` on create stores a team environment. Omitting it on update keeps the current scope. The image may be a tag or a digest reference (`name@sha256:…`). Collections, roles, and the OpenTofu cache stay under `data/cache/<team>/` for that team.

In the container the runner mounts the project read-only at `/runner/project`, the work directory at `/runner/work`, and the cache at `/cache`. `--env-file` points at a file on the host. The image gets a cleaned environment: `PATH` is local only, `HOME` is the work directory, `LANG=C.UTF-8`, and the `KOREOGRAF_*` variables. The server environment is not inherited. A configured HTTP proxy is added as `http_proxy` and `https_proxy`. That file is mode `0600` because the proxy password can sit in those values.

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

Allowed git addresses are `http(s)`, `ssh`, `git@`, and `file`. A `file://` URL is accepted for an empty host or `localhost` and then follows the same root rules as a path project. The project Sync button does not use an `scm` credential. That credential applies to git commands inside a job.

A template updates its git project on every launch. Turn **Update the project on launch** off to run the checkout from the last sync, including when the remote cannot be reached. A revision pinned on the project stays as it is on disk until the next sync. In the German UI the checkbox is **Projekt vor dem Start aktualisieren**. The project still needs one successful sync. The Sync button on the project page is unchanged.

Inventory files are named `hosts` or `inventory`, or they live in `inventory/` and `inventories/`. A relative `inventory=` entry under `[defaults]` in `ansible.cfg` is included. `POST /api/inventories/import` stores the file text. INI and the usual YAML inventory show up in the host list. Ansible receives the same text.

A file with `plugin:` is not executed. `plugin: netbox.netbox.nb_inventory` (also `plugin: netbox`) has a NetBox source. The controller fetches devices and, when asked, virtual machines, groups them like the plugin (`sites_berlin`, `device_roles_web`), and stores INI. The token lives on the credential, not in the source. `validate_certs: false` skips certificate checks, writes a warning on each fetch, and shows a badge in the inventory list. `ca_file` is an absolute PEM path on the controller. When it is set, checks stay on even if `validate_certs` is false. A missing or unreadable file fails the fetch. An inventory can have several sources, for example a file from the project and NetBox. Ansible receives each source as its own file. `POST /api/inventories/{id}/sync` refreshes every external source. Other inventory plugins are listed and cannot be executed here.

A job uses the inventory from the last sync. Launch does not call NetBox. Sync the inventory before the job when the host list should be current. With no `query_filters`, every device the API returns is imported. `ansible_host` is the host part of `primary_ip4`, then `primary_ip`, then `primary_ip6`. A null field or an address without a host is skipped, and a prefix length is removed. A device with none of those addresses is still listed and has no `ansible_host`. Virtual machines are included unless `virtual_machines` is false. A filter such as `status=active` leaves retired devices out:

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

An Ansible template stores the run in `spec`: `job_type` `run` or `check`, `tags`, `skip_tags`, `limit`, `diff`, `verbosity` from 0 to 4, and `forks` from 1 to 50. Check starts `ansible-playbook --check`.

`POST /api/jobs/{id}/cancel` sets the job to `canceled` and stops the controller process. A local job is its own process group: the group receives `SIGTERM`, then `SIGKILL` after two seconds if the job has not exited. That second signal also stops a child that ignored `SIGTERM`, including one that closed the job's output pipes. A container job is stopped with `docker kill` or `podman kill`, and the command's result is checked. A container that is already gone counts as stopped. An engine that cannot be reached does not. Output that arrives after the cancel is discarded. The status stays `canceled`. A task Ansible already started on a remote host, such as a long `sleep`, is not stopped from here.

Each job stores the parameters of that launch: template, tool, entry path, project, inventory, each attached credential as name and kind (not the secret), execution environment, spec, and the merged extra variables. The job page has an Output tab and a Parameters tab. A failed job can be started again from the job list and from the same row as the Output and Parameters tabs. `POST /api/jobs/{id}/relaunch` uses the stored parameters. When local runs are limited to administrators, that check uses the stored parameters rather than only the template as it looks now. A stored shell script, entry path, project, or inventory that is not on the current template runs only when the workflow that stored it was last saved by an administrator. Adding `--limit` for failed hosts does not by itself count as a different command. `{"only_failed_hosts": true}` is Ansible-only and sets `--limit` to the hosts that failed or were unreachable. A play recap replaces earlier `fatal:` lines. When the recap never arrives, the fatal lines are kept. `GET /api/jobs/{id}/output` downloads the output as text.

## Schedules

The Schedules entry in the menu is a table of every schedule of the selected team: what it runs, how it repeats, the clock time, the next run, and when it ends. Schedules that need an answer are listed first. A switch pauses or resumes one. Edit opens the same form. A template page and a workflow page use the same table for that resource. The form asks for a name, a date, a time, and a repeat in words: once, hourly, daily, weekly, or monthly. Weekly picks weekdays. Monthly uses the day of the month from the start date and skips a month that has no such day. The series ends never, after a count, or on a date. The stored time is a local clock time plus a time zone, so 06:00 stays 06:00 when daylight saving changes. A new schedule, a preview, and a changed start date reject a date more than one year before today. An edit that keeps the stored start date is allowed. A local time that does not exist on that day is skipped. A local time that occurs twice runs once, on the earlier offset.

The process checks about every 15 seconds. A time that passed while Koreograf was stopped is not started. The list names that last missed time and asks whether to run it now. Accept starts one run with the saved template or workflow settings and empty extra variables. Decline starts nothing. Either answer clears the question, and the same miss is not asked again. A later time stays where it is. If the previous run of that schedule is still going when the next time arrives, another run starts. The job trigger is `schedule`.
