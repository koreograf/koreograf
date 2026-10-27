#!/bin/sh
# Install sshd and Python. The ping module runs on the target.
set -eu

if command -v apt-get >/dev/null 2>&1; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    apt-get install -y --no-install-recommends \
        ca-certificates \
        openssh-server \
        python3
    rm -rf /var/lib/apt/lists/*
elif command -v dnf >/dev/null 2>&1; then
    dnf install -y ca-certificates openssh-server python3
    dnf clean all
else
    echo "install.sh: no apt-get or dnf" >&2
    exit 1
fi

mkdir -p /run/sshd /var/run/sshd
ssh-keygen -A
