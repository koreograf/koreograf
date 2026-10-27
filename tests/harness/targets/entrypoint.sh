#!/bin/sh
# Start sshd with one authorized key from the environment. No password login.
set -eu

user=ansible
key=${HARNESS_SSH_PUBLIC_KEY:-}

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

if [ ! -f /etc/ssh/ssh_host_ed25519_key ]; then
    ssh-keygen -A
fi

exec /usr/sbin/sshd -D -e
