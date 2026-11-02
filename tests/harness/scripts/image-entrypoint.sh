#!/bin/sh
set -eu

IMAGE=${1:-koreograf:ci}
ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
HELPER=debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3
VOLUME=koreograf-entrypoint-check
STOP_NAME=koreograf-entrypoint-stop
ORPHAN_NAME=koreograf-entrypoint-orphan
ORPHAN_VOLUME=koreograf-entrypoint-orphan
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
    docker rm -f "$VOLUME" "$STOP_NAME" "$ORPHAN_NAME" >/dev/null 2>&1 || true
    docker volume rm "$VOLUME" "$ORPHAN_VOLUME" >/dev/null 2>&1 || true
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

docker run -d --name "$STOP_NAME" "$IMAGE" >/dev/null
sleep 1
start=$(date +%s)
docker stop -t 30 "$STOP_NAME" >/dev/null
elapsed=$(( $(date +%s) - start ))
code=$(docker inspect --format '{{.State.ExitCode}}' "$STOP_NAME")
if [ "$elapsed" -gt 10 ]; then
    printf 'docker stop took %s seconds\n' "$elapsed" >&2
    exit 1
fi
case "$code" in
    0|143) ;;
    *)
        printf 'docker stop ended with exit code %s\n' "$code" >&2
        exit 1
        ;;
esac
docker rm -f "$STOP_NAME" >/dev/null

docker volume create "$ORPHAN_VOLUME" >/dev/null
docker run --rm -v "$ORPHAN_VOLUME":/test "$HELPER" cp /bin/sh /bin/sleep /test/
docker run -d --name "$ORPHAN_NAME" -v "$ORPHAN_VOLUME":/test \
    --entrypoint /sbin/tini "$IMAGE" -- /test/sh -c \
    '/test/sleep 1 & exec /test/sleep 5' >/dev/null
sleep 2
if docker top "$ORPHAN_NAME" -eo pid,ppid,stat,cmd \
    | awk 'NR > 1 && $3 ~ /^Z/ { found = 1 } END { exit !found }'; then
    printf 'a zombie process remained under PID 1\n' >&2
    docker top "$ORPHAN_NAME" -eo pid,ppid,stat,cmd >&2 || true
    exit 1
fi
docker top "$ORPHAN_NAME" -eo pid,ppid,stat,cmd | grep -q '/sbin/tini'
docker stop -t 30 "$ORPHAN_NAME" >/dev/null
docker rm -f "$ORPHAN_NAME" >/dev/null
docker volume rm "$ORPHAN_VOLUME" >/dev/null

printf 'entrypoint checks passed\n'
