# Installation

The shortest commands are also in the [README](../README.md#quick-start).

## Run on a machine

Rust 1.89 is the minimum (`rust-version` in `Cargo.toml`). This repository pins 1.96.0 in `rust-toolchain.toml`, and rustup installs that toolchain in this checkout. You also need `ansible-playbook` and `tofu` on `PATH` when those tools should run locally. Git projects need `git`, and Ansible jobs on remote hosts need `ssh`. The shell health check needs only `sh`.

```bash
cargo run
```

`koreograf --version` prints that same version and exits before the server starts. `koreograf doctor` prints the runner preflight in the same block as the startup log and exits with code 2 when the configured mode lacks something. It reads the saved runner mode from `KOREOGRAF_DATA` when that database exists and otherwise uses `KOREOGRAF_RUNNER`. Koreograf never installs a tool and never runs `sudo`.

The preflight checks `ansible-playbook` 2.14 or newer, `tofu` 1.6 or newer, a POSIX `sh`, `git`, `ssh`, and a Docker or Podman CLI whose daemon answers. The tools get two seconds; the container CLI gets ten. The startup log prints the result once and caches it for the **Overview** page. Administrators can refresh it with **Check again** under **Settings → Jobs** or with `GET /api/runner/preflight`. Missing `git` blocks project sync; missing `ssh` is reported for Ansible.

| Tool | Debian and Ubuntu | Fedora and RHEL | macOS | Alpine | Vendor |
| --- | --- | --- | --- | --- | --- |
| Ansible | `apt install ansible`, then `/usr/bin/ansible-playbook` | `dnf install ansible-core`, then `/usr/bin/ansible-playbook` | `brew install ansible`, then `/opt/homebrew/bin/ansible-playbook` on Apple silicon or `/usr/local/bin/ansible-playbook` on Intel | `apk add ansible`, then `/usr/bin/ansible-playbook` | [Ansible installation guide](https://docs.ansible.com/ansible/latest/installation_guide/) |
| OpenTofu | the vendor `.deb`, then `/usr/bin/tofu` | the vendor DNF repository, then `/usr/bin/tofu` | `brew install opentofu`, then `/opt/homebrew/bin/tofu` on Apple silicon or `/usr/local/bin/tofu` on Intel | `apk add opentofu`, then `/usr/bin/tofu` | [OpenTofu installation](https://opentofu.org/docs/intro/install/) |
| Shell | `apt install dash`, then `/bin/sh` | `dnf install bash`, then `/bin/sh` | `/bin/sh` is part of macOS | BusyBox provides `/bin/sh` | [POSIX shell](https://pubs.opengroup.org/onlinepubs/9699919799/utilities/sh.html) |
| Git | `apt install git`, then `/usr/bin/git` | `dnf install git`, then `/usr/bin/git` | `/usr/bin/git`, or `brew install git` | `apk add git`, then `/usr/bin/git` | [Git downloads](https://git-scm.com/downloads) |
| SSH | `apt install openssh-client`, then `/usr/bin/ssh` | `dnf install openssh-clients`, then `/usr/bin/ssh` | `/usr/bin/ssh` is part of macOS | `apk add openssh-client`, then `/usr/bin/ssh` | [OpenSSH](https://www.openssh.com/) |
| Docker | [Docker Engine packages](https://docs.docker.com/engine/install/), then `/usr/bin/docker` | `dnf install docker`, then `/usr/bin/docker` | Docker Desktop or `brew install docker` | `apk add docker`, then `/usr/bin/docker` | [Docker Engine installation](https://docs.docker.com/engine/install/) |
| Podman | `apt install podman`, then `/usr/bin/podman` | `dnf install podman`, then `/usr/bin/podman` | `brew install podman` | `apk add podman`, then `/usr/bin/podman` | [Podman installation](https://podman.io/docs/installation) |

The paths differ when a package manager or Homebrew prefix installs elsewhere. The preflight reports the path it found, not the contents of `PATH`.

The UI is at `http://127.0.0.1:8080`. The language menu at the bottom of the sidebar and on the sign-in page switches the language. Create-lists share the same pill button, for example `+ Projekt`, `+ Zugang`, and `+ Umgebung` (English: `+ Project`, `+ Credential`, `+ Environment`). Without `KOREOGRAF_ADMIN_PASSWORD`, the first start writes the password to `data/initial-admin-password` (mode `0600`) and logs the path. The password is not in the log. The process does not delete the file. Remove it after the first sign-in, and set a new password of at least 8 characters on the account page.

Environment variables, including the runner mode, are in [Configuration](configuration.md).

## Docker Compose

Docker Engine 20.10 or newer, and Docker Compose v2. The published image is `linux/amd64` and `linux/arm64`. Pull requests compile the arm64 binary, and the tag workflow publishes the multi-architecture image.

```bash
git clone https://github.com/koreograf/koreograf.git
cd koreograf
cp .env.example .env
docker compose up -d
```

The UI is at `http://127.0.0.1:8080`. Sign in as `admin`. Read the generated password, then delete the file after the first sign-in. Koreograf does not remove it.

```bash
docker compose exec koreograf cat /var/lib/koreograf/initial-admin-password
docker compose exec koreograf rm /var/lib/koreograf/initial-admin-password
```

### Company certificate authorities

Koreograf trusts a company CA in addition to the public roots. Put the certificates in a directory on the host and mount it read-only at `/etc/koreograf/ca`. The directory holds files named `*.pem` or `*.crt`. Other files are ignored. The files must be readable by uid 10001. A missing or empty directory leaves trust unchanged. A file that is not a PEM, or one that cannot be read, stops the start with one English line that names the file. Certificate verification is never turned off by this directory.

```yaml
volumes:
  - koreograf-data:/var/lib/koreograf
  - /srv/koreograf/ca:/etc/koreograf/ca:ro
```

The same certificates apply to LDAP, NetBox, and git. Git gets them through a bundle in `/tmp`, so the image needs no `update-ca-certificates` and stays `scratch`. The Helm chart mounts the same path when `caDirectory.enabled` is true; set `caDirectory.hostPath` to the directory on the node. Execution environments keep their own trust store.

Compose pulls `ghcr.io/koreograf/koreograf:0.1.0-alpha.8`. <!-- x-release-please-version -->
It does not compile. Build this checkout without replacing that published tag:

```bash
docker compose -f docker-compose.yml -f docker-compose.build.yml up --build -d
```

The local image is tagged `koreograf:local`. buildx is only for publishing a multi-architecture image. The published image holds the server binary, `git`, an SSH client, the static Docker CLI, a static `tini`, and CA certificates. Ansible, OpenTofu, and shell jobs run in the execution environments the first start seeds, so the image has no `ansible-playbook`, `tofu`, `python3`, or shell. The image sets `KOREOGRAF_CONTAINER_IMAGE=1`, and its runner preflight checks only the container CLI and socket. A saved `local` runner mode runs in a container there; the image has no local mode.

### Upgrading from an earlier image

An existing Docker install needs the Docker socket mounted and the socket's host group under `group_add` when a job should run in an image; without the group the container stops at start with one English line. The local runner mode is removed from the image, and a saved `local` mode is switched to `docker`. A job then needs a reachable socket and fails at start with a line that names `/var/run/docker.sock` when it is missing. An older root-owned data volume still needs the one-time ownership fix below.

The process starts as uid 10001. A fresh named volume inherits the ownership of `/var/lib/koreograf` from the image. An older volume that is still owned by root needs a one-time ownership fix, and that is the only step that needs root:

```bash
docker compose run --rm -T --user 0 koreograf koreograf --version
```

`docker-compose.yml` sets `user: "10001:10001"`, so the entrypoint never starts as root and cannot fix an older root-owned volume on its own. Run the command above once after upgrading such a volume.

The entrypoint in the binary creates the data directory, gives an existing volume and its files to uid 10001, joins the Docker socket group, sets `no_new_privs`, and drops to uid 10001 before it starts the server. It needs no shell, `setpriv`, `find`, or `chown`. A mounted `/var/run/docker.sock` changes nothing about the volume ownership: the process still runs as uid 10001 and needs the socket's group through `group_add`.

`/sbin/tini` is PID 1 and starts `/usr/local/bin/koreograf entrypoint`. It forwards SIGTERM, so `docker stop` returns promptly, and it reaps orphaned processes. The server runs as its child, and the container stops when the server exits. A custom image command, Compose `entrypoint`, or Helm `command` replaces the tini entrypoint; start the binary with `entrypoint` when a command should still fix the volume.

The UI is published on `127.0.0.1` only. To listen on every interface, change the port mapping to `8080:8080`. Use a reverse proxy with TLS when other machines should open the UI.

### First-run execution environments

The first start creates `Ansible (curated)`, `OpenTofu (curated)`, and `Shell (curated)` as global environments and sets each as the default under **Settings → Jobs**. Their images are pinned by digest. Pull them on the Docker host before the first job so the job does not wait for a download:

```bash
docker pull ghcr.io/ansible-community/community-ee-base:2.21.5-2@sha256:d42f2597080e8e34ce4d1f70f66f987fc841bd0bbeb680f8d234e94ef1afc60a
docker pull ghcr.io/opentofu/opentofu:1.13.2@sha256:25bdfbe87000b8a88d44c7ef0cca53381da95f6203f7aed1e8cf5b81ecbe18eb
docker pull mcr.microsoft.com/devcontainers/base:bookworm@sha256:99f5250dd2d5f8b11046d5662dd94c1e3c1dae56b560c7caad5ceb7edf349046
```

For an air-gapped installation, pull and save them on a connected machine, copy the archives, load them on the isolated host, and push them to the private registry:

```bash
docker pull ghcr.io/ansible-community/community-ee-base:2.21.5-2@sha256:d42f2597080e8e34ce4d1f70f66f987fc841bd0bbeb680f8d234e94ef1afc60a
docker save ghcr.io/ansible-community/community-ee-base:2.21.5-2@sha256:d42f2597080e8e34ce4d1f70f66f987fc841bd0bbeb680f8d234e94ef1afc60a -o ansible-ee.tar
# Repeat for the OpenTofu and Shell images, then copy the tar files.
docker load -i ansible-ee.tar
docker tag ghcr.io/ansible-community/community-ee-base:2.21.5-2@sha256:d42f2597080e8e34ce4d1f70f66f987fc841bd0bbeb680f8d234e94ef1afc60a registry.example.com/koreograf/ansible-ee:2.21.5-2
docker push registry.example.com/koreograf/ansible-ee:2.21.5-2
```

Set `KOREOGRAF_SEED_ENVIRONMENTS=0` when the curated images are not available. Create the three global environments with the registry image names instead, then choose them as the defaults under **Settings → Jobs**. Image pulls use the Docker daemon, so sign in on the host with `docker login registry.example.com`. The `registry` credential in **Credentials** passes a host, user, and password to a job; Koreograf does not run `docker login` and does not pull with that credential.

The socket line in `docker-compose.yml` is commented out. Uncomment it when a job should run in an image. The image has no local runner, so a job without a reachable socket fails at start with a line that names `/var/run/docker.sock` and points to this page. `auto` uses the container runtime; `local` is not available in the image.

That line mounts `/var/run/docker.sock`. The socket is root on the Docker host. The process runs as uid 10001, so add the socket's host group under `group_add` as the comment in the Compose file shows, for example `KOREOGRAF_DOCKER_SOCKET_GID=$(stat -c '%g' /var/run/docker.sock) docker compose up -d`. A wrong or missing group stops the container with one English line that names the group and the mode to set. An owner creates an execution environment. A global environment needs an administrator. An operator can select an environment the team can see and store it on a template. A viewer cannot. Those people choose an image that the host daemon starts. The job container does not receive the socket. It does receive the job directory, which holds the decrypted credential files for that job at mode `0600`. The job can read them. They are removed when the job ends.

The Compose service runs with a read-only root filesystem, a `tmpfs` on `/tmp`, `cap_drop: [ALL]`, `no-new-privileges`, and process and memory bounds. The data volume stays writable. A path that needs another writable directory needs its own volume or `tmpfs`.

Uncomment the socket line, then build this checkout. The image tag in `docker-compose.yml` is the last published release and can be older than these steps:

```bash
docker compose -f docker-compose.yml -f docker-compose.build.yml up --build -d
```

Leave `KOREOGRAF_DATA_HOST` empty. Koreograf reads the daemon path of the `koreograf-data` volume and uses that path as the mount source. A path inside this container is not a path the daemon can mount. Set `KOREOGRAF_DATA_HOST` only when the lookup is wrong. A path the daemon cannot see fails the job before a container starts, so the daemon does not create empty `jobs/` or `cache/` directories on the host.

`KOREOGRAF_RUNNER=auto` uses Docker when the socket answers. The image has no local fallback, so a job fails with the socket line and the link to this page when the socket is missing. `local` is not available in the image. Mount details are in [Jobs](jobs.md#execution-environments). `tests/harness/scripts/ee-socket.sh` and `tests/harness/scripts/ee-awx.sh` run this setup. They are not part of `cargo test`, and CI does not start them.

The named volume in the file is `koreograf-data`. Compose prefixes the project name, so `docker volume ls` shows `<project>_koreograf-data`. The project name is the directory name in lowercase unless `COMPOSE_PROJECT_NAME` is set. A checkout named `Koreograf` therefore uses `koreograf_koreograf-data`.

Copy `.env.example` to `.env` when a forwarded variable should change. Compose reads `.env` for substitution and ignores `KOREOGRAF_BIND`, `KOREOGRAF_DATA`, `KOREOGRAF_WEB`, and `KOREOGRAF_EXAMPLES` from that file. Those four are fixed in `docker-compose.yml`. Log format and the database URL are forwarded. A proxy variable is forwarded only when it is set. A value already exported in the shell is used for that substitution too. The two lists are in [Configuration](configuration.md#docker-compose).

A path project is a directory inside the container, not a path on your laptop. The migration variables are in [Move from a binary or systemd install](#move-from-a-binary-or-systemd-install). Mount it and allow that directory. The stock compose file does not forward `KOREOGRAF_PROJECT_ROOTS`. Add the variable to the existing `environment` map, and add the mount next to `koreograf-data`:

```yaml
environment:
  KOREOGRAF_PROJECT_ROOTS: /srv/projects
volumes:
  - ./my-project:/srv/projects/my-project:ro
```

Use `/srv/projects/my-project` as `source` with `source_type` `path`. The examples directory in the image is already allowed, so the seeded Demo project needs no extra mount. Details are in [Jobs](jobs.md#path-projects).

## Move from a binary or systemd install

Koreograf keeps the database and `secret.key` in one data directory (`KOREOGRAF_DATA`, default `./data`). The container keeps the same directory in the `koreograf-data` volume. Both paths below need an empty volume on the new host. Keep the old installation running until the checks pass.

### Preferred: backup, then restore on start

On the old host, sign in as an administrator, open **Backup**, enable **Include OpenTofu state** when jobs use OpenTofu, and download `koreograf-backup.zip`. The zip contains the database, `secret.key`, and optionally the OpenTofu state. Keep it safe: it can read every credential.

Copy the file to the new Docker host and start with an empty volume:

```bash
mkdir -p /srv/koreograf/restore
cp koreograf-backup.zip /srv/koreograf/restore/
```

Add the file and the variable to the service in `docker-compose.yml`:

```yaml
environment:
  KOREOGRAF_RESTORE_FROM: /restore/koreograf-backup.zip
volumes:
  - /srv/koreograf/restore:/restore
```

Then start the stack:

```bash
docker compose up -d
docker compose logs -f koreograf
```

The log says that the installation was restored and names the marker file. The file is renamed so the next start does not restore it again. Remove the two lines and run `docker compose up -d` again after the restore, or leave them: a non-empty data directory ignores the variable with a warning.

Check the sign-in, the credentials, and an OpenTofu `plan` before you switch DNS or the reverse proxy to the new host. `secret.key` is inside the backup; without it the restored credentials cannot be decrypted. Do not delete or replace it afterwards.

### Existing data directory

This path copies the directory over without a zip. Mount the old directory read-only:

```yaml
environment:
  KOREOGRAF_IMPORT_FROM: /import
  # Only when path projects move. Comma-separate several mappings; do not guess.
  KOREOGRAF_PATH_MAP: /home/user/projects=/projects
  # Copy the rest of the cache too. The OpenTofu state is always copied.
  KOREOGRAF_IMPORT_CACHE: "1"
volumes:
  - /srv/koreograf/old-data:/import:ro
```

Start with an empty `koreograf-data` volume:

```bash
docker compose up -d
docker compose logs -f koreograf
```

Koreograf validates `koreograf.db` and `secret.key`, copies the database with the SQLite backup API (consistent while the old service still runs and writes its WAL), copies the key and the OpenTofu state, and writes a report. A cleanly stopped old service without a WAL file works as well. The import is all or nothing: it writes into a staging directory and moves the result into the volume only when everything is complete, so a failed start leaves the volume empty and retries on the next start. It never writes to `/import`. If the volume already has data, it logs a warning and imports nothing.

The log and **Overview** list stored paths that do not exist in the container, for example `/home/user/projects/demo`. Mount them at the same path or map the prefix with `KOREOGRAF_PATH_MAP=OLD=NEW`. A mapping is applied once during the import and stored in the database; a later start does not apply it again. Paths that are not mapped stay unchanged.

The new files belong to uid 10001. Check sign-in, credentials, path projects, and an OpenTofu `plan` before switching DNS or the reverse proxy. Remove the import lines afterwards so the next start does not read the old directory.

## Helm

```bash
helm upgrade --install koreograf deploy/helm/koreograf
```

The chart version, `appVersion`, and `image.tag` are `0.1.0-alpha.8`. <!-- x-release-please-version -->
The chart pulls `ghcr.io/koreograf/koreograf:0.1.0-alpha.8`, the same image Compose uses. <!-- x-release-please-version -->
It is published for `linux/amd64` and `linux/arm64`. On another architecture, build the image and set `image.repository` and `image.tag` to that build.

By default the pod runs as uid and gid 10001, with `fsGroup` 10001, `runAsNonRoot`, a read-only root filesystem, a `tmpfs` on `/tmp`, a dropped capability set, no privilege escalation, and the runtime default seccomp profile. `fsGroup` is what makes an existing volume readable. A volume driver that ignores `fsGroup` leaves root-owned files, and this pod cannot change them because it does not start as root. Set `runner.dockerSocket.enabled` and `runner.dockerSocket.group` to the socket's gid (`stat -c '%g' /var/run/docker.sock` on the host). The chart keeps uid 10001 and puts that gid in `supplementalGroups`; it does not switch to root. A wrong group stops the container with a clear line in the pod log. A non-empty `podSecurityContext` or `securityContext` replaces that default completely, so it then has to include the supplemental group, `readOnlyRootFilesystem`, and the `/tmp` mount itself. The chart does not set a container command, so the binary entrypoint prepares the volume for uid 10001. A pod `command` replaces that entrypoint and has to start `koreograf entrypoint` when it should still fix an existing volume.

Set `existingSecret` to a Secret managed outside this chart. The chart then skips its own Secret. The Deployment reads `existingSecretKeys.adminPassword` and `existingSecretKeys.secretKey`. When `ldap.url` is set it also reads `existingSecretKeys.ldapBindPassword`. Those keys have to exist in the Secret. The defaults are `admin-password`, `secret-key`, and `ldap-bind-password`.

`replicaCount` is 1. A second pod on the same SQLite file loses writes. The Docker socket is off by default (`runner.dockerSocket.enabled`).

Probes run `koreograf healthcheck`, which calls `/healthz`. Prometheus can scrape `GET /metrics` on the same port. The counters belong to this one replica and live only in memory. If `admin.password` is empty, the first password is in the volume at `/var/lib/koreograf/initial-admin-password`. Delete that file after the first sign-in. The chart does not remove it.

The chart has no extra-volume value. A path project needs a volume and a volume mount added to the deployment, plus `KOREOGRAF_PROJECT_ROOTS` set to that directory. A path outside an allowed root can be saved by an administrator and still does not run. The examples directory in the image is already allowed.
