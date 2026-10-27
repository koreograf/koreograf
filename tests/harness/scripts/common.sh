#!/bin/sh
# Shared paths for the lab scripts. Source this file; do not execute it.
# shellcheck shell=sh

set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
SECRETS="$ROOT/.secrets"
COMPOSE_FILE="$ROOT/compose.yml"
ENV_FILE="$SECRETS/compose.env"

compose() {
    docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" "$@"
}

read_secret() {
    tr -d '\n\r' < "$1"
}

need() {
    for cmd in "$@"; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            printf 'missing command: %s\n' "$cmd" >&2
            exit 1
        fi
    done
}

require_env_file() {
    if [ ! -f "$ENV_FILE" ]; then
        printf 'Run tests/harness/scripts/up.sh first.\n' >&2
        exit 1
    fi
}
