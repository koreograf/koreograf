#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
PROJECT=koreograf-image-smoke
PASSWORD=${KOREOGRAF_SMOKE_PASSWORD:-image-smoke-password}

need() {
    for cmd in "$@"; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            printf 'missing command: %s\n' "$cmd" >&2
            exit 1
        fi
    done
}

compose() {
    docker compose -p "$PROJECT" \
        -f "$ROOT/docker-compose.yml" \
        -f "$ROOT/docker-compose.build.yml" \
        -f "$ROOT/tests/harness/image-smoke-compose.yml" "$@"
}

cleanup() {
    status=$?
    trap - EXIT INT TERM
    compose down -v >/dev/null 2>&1 || true
    rm -rf "$SSH_DIR"
    exit "$status"
}

need docker python3 ssh-keygen
if [ ! -S /var/run/docker.sock ]; then
    printf 'missing /var/run/docker.sock\n' >&2
    exit 1
fi

SSH_DIR=$(mktemp -d)
ssh-keygen -q -t ed25519 -N '' -f "$SSH_DIR/id_ed25519"
KOREOGRAF_SMOKE_SSH_PUBLIC_KEY=$(cat "$SSH_DIR/id_ed25519.pub")
export KOREOGRAF_SMOKE_SSH_PUBLIC_KEY
KOREOGRAF_SMOKE_SSH_PRIVATE_KEY_FILE="$SSH_DIR/id_ed25519"
export KOREOGRAF_SMOKE_SSH_PRIVATE_KEY_FILE

KOREOGRAF_DOCKER_SOCKET_GID=$(stat -c '%g' /var/run/docker.sock)
export KOREOGRAF_DOCKER_SOCKET_GID
KOREOGRAF_PORT=${KOREOGRAF_PORT:-18084}
export KOREOGRAF_PORT
export KOREOGRAF_IMAGE_SMOKE_URL="http://127.0.0.1:$KOREOGRAF_PORT"
export KOREOGRAF_SMOKE_PASSWORD="$PASSWORD"
export KOREOGRAF_ADMIN_PASSWORD="$PASSWORD"
export KOREOGRAF_GIT_SHA="${KOREOGRAF_GIT_SHA:-}"
export KOREOGRAF_BUILT_AT="${KOREOGRAF_BUILT_AT:-}"
trap cleanup EXIT INT TERM

cd "$ROOT"
compose up --build -d
python3 "$ROOT/tests/harness/scripts/image-smoke.py"
printf 'image smoke passed\n'
