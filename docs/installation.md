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

Docker Engine 20.10 or newer, and Docker Compose v2. A local `docker compose up --build` does not need buildx.

```bash
git clone https://github.com/koreograf/koreograf.git
cd koreograf
cp .env.example .env
docker compose up --build -d
```

The UI is at `http://127.0.0.1:8080`. Sign in as `admin`. Read the generated password, then delete the file after the first sign-in. Koreograf does not remove it.

```bash
docker compose exec koreograf cat /var/lib/koreograf/initial-admin-password
docker compose exec koreograf rm /var/lib/koreograf/initial-admin-password
```

Compose builds the image on your machine. The published image `ghcr.io/koreograf/koreograf:0.1.0-alpha.4` matches the git tag `v0.1.0-alpha.4`. The image is `linux/amd64`. It contains Ansible from Debian bookworm, the Docker client, and OpenTofu 1.13.2. The process runs as root because a mounted Docker socket is root on the host.

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

The chart version and `appVersion` are `0.1.0-alpha.4`. `image.tag` is the same release. That tag is published with the git tag `v0.1.0-alpha.4`. GitHub keeps a new package private until its visibility is public. Until the package exists and is public, build the image yourself and set `image.repository` and `image.tag` to that build.

`replicaCount` is 1. A second pod on the same SQLite file loses writes. The Docker socket is off by default (`runner.dockerSocket.enabled`).

Probes call `/healthz`. Prometheus can scrape `GET /metrics` on the same port. The counters belong to this one replica and live only in memory. If `admin.password` is empty, the first password is in the volume at `/var/lib/koreograf/initial-admin-password`. Delete that file after the first sign-in. The chart does not remove it.

The chart has no extra-volume value. A path project needs a volume and a volume mount added to the deployment, plus `KOREOGRAF_PROJECT_ROOTS` set to that directory. A path outside an allowed root can be saved by an administrator and still does not run. The examples directory in the image is already allowed.
