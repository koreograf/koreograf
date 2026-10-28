#!/bin/sh
# Compose with the Docker socket: one successful execution environment,
# one cancel that removes the container, and a wrong data path that fails
# before docker run. cargo test does not run this script, and CI does not
# start it.
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
REPO=$(CDPATH='' cd -- "$ROOT/../.." && pwd)
COMPOSE_FILE="$ROOT/ee-socket-compose.yml"
PROJECT=koreograf-ee-socket
WRONG=/var/lib/koreograf-ee-socket-wrong

need() {
    for cmd in "$@"; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            printf 'missing command: %s\n' "$cmd" >&2
            exit 1
        fi
    done
}

compose() {
    docker compose -p "$PROJECT" -f "$COMPOSE_FILE" "$@"
}

cleanup() {
    status=$?
    trap - EXIT INT TERM
    compose down -v >/dev/null 2>&1 || true
    rm -f "$PASS_FILE"
    if [ -e "$WRONG" ]; then
        printf 'leftover path: %s\n' "$WRONG" >&2
        status=1
    fi
    exit "$status"
}

if [ "${1:-}" = "down" ]; then
    need docker
    shift
    # Compose reads the file before down. The password is not used.
    EE_SOCKET_PASSWORD=${EE_SOCKET_PASSWORD:-unused}
    export EE_SOCKET_PASSWORD
    compose down "$@"
    exit 0
fi

need docker python3 openssl
PASS_FILE=$(mktemp)
openssl rand -hex 16 | tr -d '\n' > "$PASS_FILE"
chmod 600 "$PASS_FILE"
# shellcheck disable=SC2034
EE_SOCKET_PASSWORD=$(tr -d '\n\r' < "$PASS_FILE")
export EE_SOCKET_PASSWORD
unset KOREOGRAF_DATA_HOST || true
trap cleanup EXIT INT TERM

run_check() {
    if ! python3 "$ROOT/scripts/ee-socket.py" "$1"; then
        compose logs --no-color --tail 80 >&2 || true
        exit 1
    fi
}

cd "$REPO"
compose up --build -d
run_check success

export KOREOGRAF_DATA_HOST=$WRONG
compose up -d --no-build --force-recreate
run_check wrong
printf 'socket checks passed\n'
