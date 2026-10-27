#!/bin/sh
# The image starts as root so a data volume can be given to uid 10001.
# An older volume keeps secret.key mode 0600 and the database owned by root.
# A mounted docker.sock stays root, because that socket is root on the host.
set -eu
if [ "$(id -u)" -eq 0 ]; then
  mkdir -p /var/lib/koreograf
  if [ ! -S /var/run/docker.sock ]; then
    find /var/lib/koreograf -xdev ! -user koreograf -exec chown koreograf:koreograf {} +
    exec setpriv --reuid=koreograf --regid=koreograf --init-groups --inh-caps=-all --bounding-set=-all --no-new-privs "$@"
  fi
fi
exec "$@"
