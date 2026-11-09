#!/bin/sh
set -eu

case "${1:-}" in
    remove|deconfigure|0)
        if [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; then
            systemctl stop koreograf.service || true
            systemctl disable koreograf.service || true
        fi
        ;;
esac
