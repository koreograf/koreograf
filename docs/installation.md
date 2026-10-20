# Installation

The shortest commands are also in the [README](../README.md#quick-start).

## Run on a machine

Requirements: Rust 1.89 or newer, plus `ansible-playbook` and `tofu` on `PATH` when those tools should run locally. The shell health check needs only `sh`.

```bash
cargo run
```

The UI is at `http://127.0.0.1:8080`. The language menu at the bottom of the sidebar and on the sign-in page switches the language. Create-lists share the same pill button, for example `+ Projekt`, `+ Zugang`, and `+ Umgebung` (English: `+ Project`, `+ Credential`, `+ Environment`). Without `KOREOGRAF_ADMIN_PASSWORD`, the first start writes the password to `data/initial-admin-password` (mode `0600`) and logs the path. The password is not in the log.

Environment variables, including the runner mode, are in [Configuration](configuration.md).

## Docker Compose

```bash
docker compose up --build
```

Compose builds the image on your machine. The published image `ghcr.io/koreograf/koreograf:0.1.0-alpha.1` exists after the git tag `v0.1.0-alpha.1`. The image is `linux/amd64`. It contains Ansible from Debian, the Docker client, and OpenTofu 1.12.1. The process runs as root because a mounted Docker socket is root on the host.

The socket is commented out in `docker-compose.yml`. Execution environments need that line. Without the socket the runner stays local, and the demo health check still runs.

Data lives in the volume `koreograf-data`. Compose reads `.env` for variable substitution. The compose file fixes the bind address, the data directory, the web files, and the examples directory. Log format and the database URL are forwarded. A proxy variable is forwarded only when it is set. A value already exported in the shell is used for that substitution too. The two lists are in [Configuration](configuration.md#docker-compose).

## Helm

```bash
helm upgrade --install koreograf deploy/helm/koreograf
```

The chart pulls `ghcr.io/koreograf/koreograf:0.1.0-alpha.1`. That tag is published with the git tag `v0.1.0-alpha.1`. GitHub keeps a new package private until its visibility is public. Until the package exists and is public, build the image yourself and set `image.repository` and `image.tag` to that build.

`replicaCount` is 1. A second pod on the same SQLite file loses writes. The Docker socket is off by default (`runner.dockerSocket.enabled`).

Probes call `/healthz`. Prometheus can scrape `GET /metrics` on the same port. The counters belong to this one replica and live only in memory. If `admin.password` is empty, the first password is in the volume at `/var/lib/koreograf/initial-admin-password`.
