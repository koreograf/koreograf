#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
REPO=$(CDPATH='' cd -- "$ROOT/../.." && pwd)
IMAGE=koreograf:checkout-socket
CONTAINER=koreograf-checkout-socket
DATA_VOLUME=koreograf-checkout-socket-data
SOURCE_VOLUME=koreograf-checkout-socket-source
PORT=18085

need() {
    for cmd in "$@"; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            printf 'missing command: %s\n' "$cmd" >&2
            exit 1
        fi
    done
}

cleanup() {
    status=$?
    trap - EXIT INT TERM
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    docker volume rm "$DATA_VOLUME" "$SOURCE_VOLUME" >/dev/null 2>&1 || true
    exit "$status"
}

if [ "${1:-}" = "down" ]; then
    need docker
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    docker volume rm "$DATA_VOLUME" "$SOURCE_VOLUME" >/dev/null 2>&1 || true
    exit 0
fi

need docker python3 openssl
CHECKOUT_PASSWORD=$(openssl rand -hex 16)
export CHECKOUT_PASSWORD
export CHECKOUT_CONTAINER=$CONTAINER
export CHECKOUT_SOCKET_GID
CHECKOUT_SOCKET_GID=$(stat -c '%g' /var/run/docker.sock)
trap cleanup EXIT INT TERM

start() {
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    if [ "${1:-}" = "socket" ]; then
        docker run -d --name "$CONTAINER" \
            -p "127.0.0.1:$PORT:8080" \
            -e KOREOGRAF_ADMIN_PASSWORD="$CHECKOUT_PASSWORD" \
            -e KOREOGRAF_RUNNER=local \
            -e KOREOGRAF_PROJECT_ROOTS=/srv/source \
            -v "$DATA_VOLUME":/var/lib/koreograf \
            -v "$SOURCE_VOLUME":/srv/source:ro \
            -v /var/run/docker.sock:/var/run/docker.sock \
            "$IMAGE" >/dev/null
    else
        docker run -d --name "$CONTAINER" \
            -p "127.0.0.1:$PORT:8080" \
            -e KOREOGRAF_ADMIN_PASSWORD="$CHECKOUT_PASSWORD" \
            -e KOREOGRAF_RUNNER=local \
            -e KOREOGRAF_PROJECT_ROOTS=/srv/source \
            -v "$DATA_VOLUME":/var/lib/koreograf \
            -v "$SOURCE_VOLUME":/srv/source:ro \
            "$IMAGE" >/dev/null
    fi
}

run_check() {
    if ! python3 "$ROOT/scripts/checkout-socket.py" "$@"; then
        docker logs --no-color --tail 80 "$CONTAINER" >&2 || true
        exit 1
    fi
}

cd "$REPO"
docker build -t "$IMAGE" .
docker volume create "$DATA_VOLUME" >/dev/null
docker volume create "$SOURCE_VOLUME" >/dev/null
docker run --rm --user 0 --entrypoint sh -v "$SOURCE_VOLUME":/src "$IMAGE" -c \
    'git init -q /src; printf "%s\n" "[all]" "localhost ansible_connection=local" > /src/site.yml; git -C /src add site.yml; git -C /src -c user.name=Koreograf -c user.email=koreograf@example.invalid commit -q -m init; git -C /src branch -M main; chown -R 10001:10001 /src'

start
run_check sync
start socket
run_check sync socket
start
run_check sync
printf 'checkout socket checks passed\n'
