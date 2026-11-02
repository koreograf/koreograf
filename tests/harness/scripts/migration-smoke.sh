#!/bin/sh
# Docker smoke for the two migration paths. cargo test does not run this
# script and CI does not start it. Without Docker it says that it skipped.
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
PASSWORD=${KOREOGRAF_SMOKE_PASSWORD:-migration-smoke-password}
IMAGE=${KOREOGRAF_SMOKE_IMAGE:-koreograf:migration-smoke}
LOCAL_PORT=${KOREOGRAF_SMOKE_LOCAL_PORT:-18190}
RESTORE_PORT=${KOREOGRAF_SMOKE_RESTORE_PORT:-18191}
IMPORT_PORT=${KOREOGRAF_SMOKE_IMPORT_PORT:-18192}

if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
    printf 'no container runtime; migration smoke test skipped\n'
    exit 0
fi
for cmd in cargo curl python3 tofu sha256sum; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        printf 'missing command: %s\n' "$cmd" >&2
        exit 1
    fi
done

WORK=$(mktemp -d)
OLD="$WORK/old-data"
RESTORE_DIR="$WORK/restore"
RESTORE_DATA="$WORK/restore-data"
IMPORT_DATA="$WORK/import-data"
PROJECTS="$WORK/projects"
OLD_PROJECTS="$WORK/old-projects"
RESTORE_NAME=koreograf-migration-restore
IMPORT_NAME=koreograf-migration-import
LOCAL_PID=

cleanup() {
    status=$?
    trap - EXIT INT TERM
    if [ -n "$LOCAL_PID" ]; then
        kill "$LOCAL_PID" >/dev/null 2>&1 || true
        wait "$LOCAL_PID" >/dev/null 2>&1 || true
    fi
    docker rm -f "$RESTORE_NAME" "$IMPORT_NAME" >/dev/null 2>&1 || true
    docker run --rm -v "$WORK:/work" alpine:3.22 chmod -R a+rwX /work >/dev/null 2>&1 || true
    rm -rf "$WORK"
    exit "$status"
}
trap cleanup EXIT INT TERM

checksums() {
    find "$OLD" -type f -print0 | sort -z | xargs -0 sha256sum
}

wait_health() {
    count=0
    while [ "$count" -lt 90 ]; do
        if curl -fsS "$1/healthz" >/dev/null 2>&1; then
            return 0
        fi
        count=$((count + 1))
        sleep 1
    done
    printf '%s/healthz did not answer\n' "$1" >&2
    if [ -n "${2:-}" ]; then
        docker logs "$2" >&2 || true
    fi
    return 1
}

cd "$ROOT"
cargo build --quiet --bin koreograf
docker build --quiet --tag "$IMAGE" . >/dev/null

mkdir -p "$OLD" "$RESTORE_DIR" "$RESTORE_DATA" "$IMPORT_DATA" "$PROJECTS/demo" "$OLD_PROJECTS/demo"
chmod 777 "$RESTORE_DIR" "$RESTORE_DATA" "$IMPORT_DATA"
printf 'local module\n' > "$PROJECTS/demo/main.tf"

KOREOGRAF_SMOKE_URL="http://127.0.0.1:$LOCAL_PORT" \
KOREOGRAF_SMOKE_PASSWORD="$PASSWORD" \
KOREOGRAF_SMOKE_DATA="$OLD" \
KOREOGRAF_SMOKE_BACKUP="$RESTORE_DIR/koreograf-backup.zip" \
KOREOGRAF_DATA="$OLD" \
KOREOGRAF_BIND="127.0.0.1:$LOCAL_PORT" \
KOREOGRAF_ADMIN_PASSWORD="$PASSWORD" \
KOREOGRAF_RUNNER=local \
KOREOGRAF_SEED_ENVIRONMENTS=0 \
KOREOGRAF_WEB="$ROOT/web" \
KOREOGRAF_EXAMPLES="$ROOT/examples" \
"$ROOT/target/debug/koreograf" >"$WORK/local.log" 2>&1 &
LOCAL_PID=$!
KOREOGRAF_SMOKE_URL="http://127.0.0.1:$LOCAL_PORT" \
KOREOGRAF_SMOKE_PASSWORD="$PASSWORD" \
KOREOGRAF_SMOKE_DATA="$OLD" \
KOREOGRAF_SMOKE_BACKUP="$RESTORE_DIR/koreograf-backup.zip" \
KOREOGRAF_SMOKE_OLD_PROJECTS="$OLD_PROJECTS" \
python3 "$ROOT/tests/harness/scripts/migration-smoke.py" seed
kill "$LOCAL_PID"
wait "$LOCAL_PID" >/dev/null 2>&1 || true
LOCAL_PID=
chmod -R a+rX "$OLD"

docker run -d --name "$RESTORE_NAME" \
    -p "127.0.0.1:$RESTORE_PORT:8080" \
    -v "$RESTORE_DIR:/restore" \
    -v "$RESTORE_DATA:/var/lib/koreograf" \
    -e KOREOGRAF_RESTORE_FROM=/restore/koreograf-backup.zip \
    "$IMAGE" >/dev/null
wait_health "http://127.0.0.1:$RESTORE_PORT" "$RESTORE_NAME"
[ "$(stat -c '%u' "$RESTORE_DATA/koreograf.db")" = 10001 ]
mkdir -p "$WORK/restore-cache"
docker cp "$RESTORE_NAME:/var/lib/koreograf/cache/." "$WORK/restore-cache" >/dev/null
RESTORE_STATE=$(find "$WORK/restore-cache" -path '*tofu-state/smoke-tofu/terraform.tfstate')
KOREOGRAF_SMOKE_URL="http://127.0.0.1:$RESTORE_PORT" \
KOREOGRAF_SMOKE_PASSWORD="$PASSWORD" \
KOREOGRAF_SMOKE_DATA="$RESTORE_DATA" \
KOREOGRAF_SMOKE_STATE="$RESTORE_STATE" \
KOREOGRAF_SMOKE_EXPECT_SOURCE="$OLD_PROJECTS/demo" \
python3 "$ROOT/tests/harness/scripts/migration-smoke.py" verify
[ -f "$RESTORE_DIR/koreograf-backup.zip.applied" ]
docker logs "$RESTORE_NAME" 2>&1 | grep -q 'restored the installation from the startup backup'
docker rm -f "$RESTORE_NAME" >/dev/null

BEFORE=$(checksums)
docker run -d --name "$IMPORT_NAME" \
    -p "127.0.0.1:$IMPORT_PORT:8080" \
    -v "$OLD:/import:ro" \
    -v "$PROJECTS:/projects:ro" \
    -v "$IMPORT_DATA:/var/lib/koreograf" \
    -e KOREOGRAF_IMPORT_FROM=/import \
    -e "KOREOGRAF_PATH_MAP=$OLD_PROJECTS=/projects" \
    "$IMAGE" >/dev/null
wait_health "http://127.0.0.1:$IMPORT_PORT" "$IMPORT_NAME"
[ "$(stat -c '%u' "$IMPORT_DATA/koreograf.db")" = 10001 ]
mkdir -p "$WORK/import-cache"
docker cp "$IMPORT_NAME:/var/lib/koreograf/cache/." "$WORK/import-cache" >/dev/null
IMPORT_STATE=$(find "$WORK/import-cache" -path '*tofu-state/smoke-tofu/terraform.tfstate')
KOREOGRAF_SMOKE_URL="http://127.0.0.1:$IMPORT_PORT" \
KOREOGRAF_SMOKE_PASSWORD="$PASSWORD" \
KOREOGRAF_SMOKE_DATA="$IMPORT_DATA" \
KOREOGRAF_SMOKE_STATE="$IMPORT_STATE" \
KOREOGRAF_SMOKE_EXPECT_SOURCE=/projects/demo \
python3 "$ROOT/tests/harness/scripts/migration-smoke.py" verify
docker logs "$IMPORT_NAME" 2>&1 | grep -q 'imported the existing data directory'
AFTER=$(checksums)
[ "$BEFORE" = "$AFTER" ] || {
    printf '/import changed during the import\n' >&2
    exit 1
}
docker rm -f "$IMPORT_NAME" >/dev/null

printf 'migration smoke passed\n'
