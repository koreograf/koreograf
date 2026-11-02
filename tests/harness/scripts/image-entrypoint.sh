#!/bin/sh
set -eu

IMAGE=${1:-koreograf:ci}
ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
HELPER=debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3
VOLUME=koreograf-entrypoint-check
SOCKET_DIR=$(mktemp -d)
SOCKET_PID=

cleanup() {
    status=$?
    trap - EXIT INT TERM
    if [ -n "$SOCKET_PID" ]; then
        kill "$SOCKET_PID" >/dev/null 2>&1 || true
        wait "$SOCKET_PID" 2>/dev/null || true
    fi
    rm -rf "$SOCKET_DIR"
    docker rm -f "$VOLUME" >/dev/null 2>&1 || true
    docker volume rm "$VOLUME" >/dev/null 2>&1 || true
    exit "$status"
}
trap cleanup EXIT INT TERM

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    docker build -t "$IMAGE" "$ROOT"
fi

docker volume create "$VOLUME" >/dev/null

docker run --rm -v "$VOLUME":/var/lib/koreograf --entrypoint /usr/local/bin/koreograf \
    "$IMAGE" entrypoint koreograf --version >/dev/null
owner=$(docker run --rm -v "$VOLUME":/data "$HELPER" stat -c '%u:%g' /data)
test "$owner" = "10001:10001"

docker run --rm -v "$VOLUME":/var/lib/koreograf "$HELPER" \
    sh -c 'chown 0:0 /var/lib/koreograf && printf x > /var/lib/koreograf/secret.key && chmod 600 /var/lib/koreograf/secret.key && chown 0:0 /var/lib/koreograf/secret.key'
docker run --rm --user 0 -v "$VOLUME":/var/lib/koreograf --entrypoint /usr/local/bin/koreograf \
    "$IMAGE" entrypoint koreograf --version >/dev/null
owner=$(docker run --rm -v "$VOLUME":/data "$HELPER" stat -c '%u:%g' /data/secret.key)
test "$owner" = "10001:10001"

docker run --rm --entrypoint /usr/local/bin/koreograf "$IMAGE" entrypoint koreograf --version >/dev/null

chmod 755 "$SOCKET_DIR"
python3 "$ROOT/tests/harness/scripts/unix-socket.py" "$SOCKET_DIR/docker.sock" 660 &
SOCKET_PID=$!
sleep 1
gid=$(stat -c '%g' "$SOCKET_DIR/docker.sock")
docker run --rm --group-add "$gid" -v "$SOCKET_DIR/docker.sock:/var/run/docker.sock" \
    --entrypoint /usr/local/bin/koreograf "$IMAGE" entrypoint koreograf --version >/dev/null

chmod 600 "$SOCKET_DIR/docker.sock"
if docker run --rm -v "$SOCKET_DIR/docker.sock:/var/run/docker.sock" \
    --entrypoint /usr/local/bin/koreograf "$IMAGE" entrypoint koreograf --version \
    >"$SOCKET_DIR/out" 2>&1; then
    printf 'an unreadable socket was accepted\n' >&2
    exit 1
fi
grep -q 'not readable for uid 10001' "$SOCKET_DIR/out"

if docker run --rm --user 0 -v "$SOCKET_DIR/docker.sock:/var/run/docker.sock" \
    --entrypoint /usr/local/bin/koreograf "$IMAGE" entrypoint koreograf --version \
    >"$SOCKET_DIR/root-out" 2>&1; then
    printf 'an unreadable socket was accepted as root\n' >&2
    exit 1
fi
grep -q 'with group' "$SOCKET_DIR/root-out"

printf 'entrypoint checks passed\n'
