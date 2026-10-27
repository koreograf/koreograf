#!/bin/sh
# The image starts as root so a new data volume can be given to uid 10001.
# A mounted docker.sock stays root, because that socket is root on the host.
set -eu
if [ "$(id -u)" -eq 0 ]; then
  mkdir -p /var/lib/koreograf
  chown koreograf:koreograf /var/lib/koreograf
  if [ ! -S /var/run/docker.sock ]; then
    exec setpriv --reuid=koreograf --regid=koreograf --init-groups --inh-caps=-all --bounding-set=-all --no-new-privs "$@"
  fi
fi
exec "$@"
