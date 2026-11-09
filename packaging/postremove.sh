#!/bin/sh
set -eu

case "${1:-}" in
    0)
        install -d -o root -g root -m 0755 /etc/koreograf
        if [ -e /etc/koreograf/koreograf.env ]; then
            chown root:root /etc/koreograf/koreograf.env
            chmod 0600 /etc/koreograf/koreograf.env
        fi
        ;;
esac
if [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
fi
