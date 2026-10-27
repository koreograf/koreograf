#!/bin/sh
# Stop the lab. Pass -v to remove volumes so the next up.sh seeds a fresh stack.
set -eu
# shellcheck source-path=SCRIPTDIR
# shellcheck source=common.sh
. "$(dirname -- "$0")/common.sh"

need docker
require_env_file
compose down "$@"
