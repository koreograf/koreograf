#!/bin/sh
set -eu

if ! getent group koreograf >/dev/null 2>&1; then
    groupadd --system koreograf
fi
if ! getent passwd koreograf >/dev/null 2>&1; then
    useradd --system --gid koreograf --home-dir /var/lib/koreograf --shell /usr/sbin/nologin koreograf
fi
install -d -o koreograf -g koreograf -m 0750 /var/lib/koreograf
install -d -o root -g root -m 0755 /etc/koreograf
if [ ! -e /etc/koreograf/koreograf.env ]; then
    umask 077
    printf '%s\n' 'KOREOGRAF_DATA=/var/lib/koreograf' 'KOREOGRAF_BIND=127.0.0.1:8080' > /etc/koreograf/koreograf.env
fi
chown root:root /etc/koreograf/koreograf.env
chmod 0600 /etc/koreograf/koreograf.env
if [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
    systemctl enable koreograf.service
    systemctl restart koreograf.service
fi
