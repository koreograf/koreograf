# Installation

The shortest commands are also in the [README](../README.md#quick-start).

## Run on a machine

Rust 1.89 is the minimum (`rust-version` in `Cargo.toml`). This repository pins 1.96.0 in `rust-toolchain.toml`, and rustup installs that toolchain in this checkout. You also need `ansible-playbook` and `tofu` on `PATH` when those tools should run locally. The shell health check needs only `sh`.

```bash
cargo run
```

The UI is at `http://127.0.0.1:8080`. The language menu at the bottom of the sidebar and on the sign-in page switches the language. Create-lists share the same pill button, for example `+ Projekt`, `+ Zugang`, and `+ Umgebung` (English: `+ Project`, `+ Credential`, `+ Environment`). Without `KOREOGRAF_ADMIN_PASSWORD`, the first start writes the password to `data/initial-admin-password` (mode `0600`) and logs the path. The password is not in the log. The process does not delete the file. Remove it after the first sign-in, and set a new password of at least 8 characters on the account page.

Environment variables, including the runner mode, are in [Configuration](configuration.md).

## Docker Compose

Docker Engine 20.10 or newer, and Docker Compose v2. The published image is `linux/amd64`.

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

Compose pulls `ghcr.io/koreograf/koreograf:0.1.0-alpha.5`. <!-- x-release-please-version -->
It does not compile. Build this checkout without replacing that published tag:

```bash
docker compose -f docker-compose.yml -f docker-compose.build.yml up --build -d
```

The local image is tagged `koreograf:local`. buildx is only for publishing a multi-architecture image. Ansible, the Docker client, and OpenTofu in the published image are the versions from the Dockerfile at that tag. The process starts as root, gives `/var/lib/koreograf` and every file already in it to uid 10001, and then drops to that user. An older volume keeps `secret.key` mode `0600` owned by root; without that hand-over the new process cannot read it. A mounted `/var/run/docker.sock` stays root, because that socket is root on the host.

The UI is published on `127.0.0.1` only. To listen on every interface, change the port mapping to `8080:8080`. Use a reverse proxy with TLS when other machines should open the UI.

The socket is commented out in `docker-compose.yml`. Execution environments need that line. Without the socket the runner stays local, and the demo health check still runs.

The named volume in the file is `koreograf-data`. Compose prefixes the project name, so `docker volume ls` shows `<project>_koreograf-data`. The project name is the directory name in lowercase unless `COMPOSE_PROJECT_NAME` is set. A checkout named `Koreograf` therefore uses `koreograf_koreograf-data`.

Copy `.env.example` to `.env` when a forwarded variable should change. Compose reads `.env` for substitution and ignores `KOREOGRAF_BIND`, `KOREOGRAF_DATA`, `KOREOGRAF_WEB`, and `KOREOGRAF_EXAMPLES` from that file. Those four are fixed in `docker-compose.yml`. Log format and the database URL are forwarded. A proxy variable is forwarded only when it is set. A value already exported in the shell is used for that substitution too. The two lists are in [Configuration](configuration.md#docker-compose).

A path project is a directory inside the container, not a path on your laptop. Mount it and allow that directory. The stock compose file does not forward `KOREOGRAF_PROJECT_ROOTS`. Add the variable to the existing `environment` map, and add the mount next to `koreograf-data`:

```yaml
environment:
  KOREOGRAF_PROJECT_ROOTS: /srv/projects
volumes:
  - ./my-project:/srv/projects/my-project:ro
```

Use `/srv/projects/my-project` as `source` with `source_type` `path`. The examples directory in the image is already allowed, so the seeded Demo project needs no extra mount. Details are in [Jobs](jobs.md#path-projects).

## Helm

```bash
helm upgrade --install koreograf deploy/helm/koreograf
```

The chart version, `appVersion`, and `image.tag` are `0.1.0-alpha.5`. <!-- x-release-please-version -->
The chart pulls `ghcr.io/koreograf/koreograf:0.1.0-alpha.5`, the same image Compose uses. <!-- x-release-please-version -->
It is `linux/amd64`. On another architecture, build the image and set `image.repository` and `image.tag` to that build.

By default the pod runs as uid and gid 10001, with `fsGroup` 10001, `runAsNonRoot`, a dropped capability set, and the runtime default seccomp profile. `fsGroup` is what makes an existing volume readable. A volume driver that ignores `fsGroup` leaves root-owned files, and this pod cannot change them because it does not start as root. Set `runner.dockerSocket.enabled` to run as uid 0 without the capability drop. A non-empty `podSecurityContext` or `securityContext` replaces that default completely.

Set `existingSecret` to a Secret managed outside this chart. The chart then skips its own Secret. The Deployment reads `existingSecretKeys.adminPassword` and `existingSecretKeys.secretKey`. When `ldap.url` is set it also reads `existingSecretKeys.ldapBindPassword`. Those keys have to exist in the Secret. The defaults are `admin-password`, `secret-key`, and `ldap-bind-password`.

`replicaCount` is 1. A second pod on the same SQLite file loses writes. The Docker socket is off by default (`runner.dockerSocket.enabled`).

Probes call `/healthz`. Prometheus can scrape `GET /metrics` on the same port. The counters belong to this one replica and live only in memory. If `admin.password` is empty, the first password is in the volume at `/var/lib/koreograf/initial-admin-password`. Delete that file after the first sign-in. The chart does not remove it.

The chart has no extra-volume value. A path project needs a volume and a volume mount added to the deployment, plus `KOREOGRAF_PROJECT_ROOTS` set to that directory. A path outside an allowed root can be saved by an administrator and still does not run. The examples directory in the image is already allowed.
