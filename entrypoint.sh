#!/bin/sh
set -eu

if [ "$(id -u)" -eq 0 ]; then
  mkdir -p /var/lib/koreograf
  find /var/lib/koreograf -xdev ! -user koreograf -exec chown koreograf:koreograf {} +
  if [ -S /var/run/docker.sock ]; then
    socket_gid=$(stat -c '%g' /var/run/docker.sock 2>/dev/null || true)
    case "$socket_gid" in
      '' | *[!0-9]*)
        printf '%s\n' "Cannot determine the group of /var/run/docker.sock. Check the mounted socket and give it a numeric group that uid 10001 can use." >&2
        exit 1
        ;;
    esac
    if ! setpriv --reuid=koreograf --regid=koreograf --groups="$socket_gid" --inh-caps=-all --bounding-set=-all --no-new-privs \
      sh -c '[ -r /var/run/docker.sock ]'; then
      printf '%s\n' "The Docker socket is not readable for uid 10001 with group $socket_gid. Give the socket that group and mode 0660, or add the group to the container." >&2
      exit 1
    fi
    exec setpriv --reuid=koreograf --regid=koreograf --groups="$socket_gid" --inh-caps=-all --bounding-set=-all --no-new-privs "$@"
  fi
  exec setpriv --reuid=koreograf --regid=koreograf --init-groups --inh-caps=-all --bounding-set=-all --no-new-privs "$@"
fi

if [ -S /var/run/docker.sock ] && [ ! -r /var/run/docker.sock ]; then
  printf '%s\n' "The Docker socket is not readable for uid 10001. Set runner.dockerSocket.group to the socket's gid, or give the mounted socket that group and mode 0660." >&2
  exit 1
fi

exec "$@"
