#!/bin/sh
set -eu

case "${1:-}" in
    0)
        install -d -o root -g root -m 0755 /etc/koreograf
        if [ ! -e /etc/koreograf/koreograf.env ]; then
            umask 077
            printf '%s\n' 'KOREOGRAF_DATA=/var/lib/koreograf' 'KOREOGRAF_BIND=127.0.0.1:8080' > /etc/koreograf/koreograf.env
        fi
        chown root:root /etc/koreograf/koreograf.env
        chmod 0600 /etc/koreograf/koreograf.env
        ;;
esac
if [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
fi
