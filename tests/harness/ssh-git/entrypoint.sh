#!/bin/sh
# Start sshd and a bare repository for the image smoke. No password login.
set -eu

user=git
key=${HARNESS_SSH_PUBLIC_KEY:-}
repo=/srv/git/smoke.git

if [ -z "$key" ]; then
    echo "HARNESS_SSH_PUBLIC_KEY is empty" >&2
    exit 1
fi

if ! id "$user" >/dev/null 2>&1; then
    useradd --create-home --shell /bin/sh "$user"
fi

home=$(getent passwd "$user" | cut -d: -f6)
if [ -z "$home" ] || [ ! -d "$home" ]; then
    echo "home directory for $user is missing" >&2
    exit 1
fi

mkdir -p "$home/.ssh"
printf '%s\n' "$key" > "$home/.ssh/authorized_keys"
chmod 700 "$home/.ssh"
chmod 600 "$home/.ssh/authorized_keys"
chown -R "$user:$user" "$home/.ssh"

if [ ! -d "$repo" ]; then
    mkdir -p /srv/git
    git init --bare -b main "$repo" >/dev/null
    work=$(mktemp -d)
    git -C "$work" init -q
    git -C "$work" config user.email smoke@example.test
    git -C "$work" config user.name smoke
    printf '%s\n' "smoke" > "$work/README.md"
    git -C "$work" add README.md
    git -C "$work" commit -qm init
    git -C "$work" push -q "$repo" HEAD:main
    rm -rf "$work"
fi
chown -R "$user:$user" /srv/git

mkdir -p /run/sshd
printf '%s\n' 'PasswordAuthentication no' 'PermitRootLogin no' 'PubkeyAuthentication yes' \
    > /etc/ssh/sshd_config.d/99-koreograf-smoke.conf

ssh-keygen -A
exec /usr/sbin/sshd -D -e
