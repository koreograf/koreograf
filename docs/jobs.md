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

A template updates its git project on every launch. Turn **Update the project on launch** off to run the checkout from the last sync, including when the remote cannot be reached. A revision pinned on the project stays as it is on disk until the next sync. In the German UI the checkbox is **Projekt vor dem Start aktualisieren**. The project still needs one successful sync. The Sync button on the project page is unchanged.

Inventory files are named `hosts` or `inventory`, or they live in `inventory/` and `inventories/`. A relative `inventory=` entry under `[defaults]` in `ansible.cfg` is included. `POST /api/inventories/import` stores the file text. INI and the usual YAML inventory show up in the host list. Ansible receives the same text.

A file with `plugin:` is not executed. `plugin: netbox.netbox.nb_inventory` (also `plugin: netbox`) has a NetBox source. The controller fetches devices and, when asked, virtual machines, groups them like the plugin (`sites_berlin`, `device_roles_web`), and stores INI. The token lives on the credential, not in the source. An inventory can have several sources, for example a file from the project and NetBox. Ansible receives each source as its own file. `POST /api/inventories/{id}/sync` refreshes every external source. Other inventory plugins are listed and cannot be executed here.

A job uses the inventory from the last sync. Launch does not call NetBox. Sync the inventory before the job when the host list should be current. With no `query_filters`, every device the API returns is imported, including a device that has no `primary_ip`. That host is listed and has no `ansible_host`. Virtual machines are included unless `virtual_machines` is false. A filter such as `status=active` leaves retired devices out:

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

An Ansible template stores the run in `spec`: `job_type` `run` or `check`, `tags`, `skip_tags`, `limit`, `diff`, `verbosity` from 0 to 4, and `forks` from 1 to 50. Check starts `ansible-playbook --check`.

`POST /api/jobs/{id}/cancel` sets the job to `canceled` and stops the controller process. A local job is its own process group: the group receives `SIGTERM`, then `SIGKILL` after two seconds if the job has not exited. That second signal also stops a child that ignored `SIGTERM`, including one that closed the job's output pipes. A container job is stopped with `docker kill` or `podman kill`, and the command's result is checked. A container that is already gone counts as stopped. An engine that cannot be reached does not. Output that arrives after the cancel is discarded. The status stays `canceled`. A task Ansible already started on a remote host, such as a long `sleep`, is not stopped from here.

Each job stores the parameters of that launch: template, tool, entry path, project, inventory, each attached credential as name and kind (not the secret), execution environment, spec, and the merged extra variables. The job page has an Output tab and a Parameters tab. A failed job can be started again from the job list and from the same row as the Output and Parameters tabs. `POST /api/jobs/{id}/relaunch` uses the stored parameters. `{"only_failed_hosts": true}` is Ansible-only and sets `--limit` to the hosts that failed or were unreachable. A play recap replaces earlier `fatal:` lines. When the recap never arrives, the fatal lines are kept. `GET /api/jobs/{id}/output` downloads the output as text.
