#!/bin/sh
# Exit non-zero unless health, admin login, the demo workflow, the distro
# ping, the LDAP team, and the NetBox host count all succeed.
set -eu
# shellcheck source-path=SCRIPTDIR
# shellcheck source=common.sh
. "$(dirname -- "$0")/common.sh"

need python3
require_env_file
exec python3 "$ROOT/scripts/verify.py"
