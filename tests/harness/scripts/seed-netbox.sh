#!/bin/sh
# Create a v1 API token and two devices. Existing rows are left as they are.
# Koreograf sends "Authorization: Token <plaintext>". NetBox 4.7 still
# accepts a v1 token of exactly 40 characters. The image bootstrap
# creates a v2 token, which this client does not send.
set -eu
# shellcheck source-path=SCRIPTDIR
# shellcheck source=common.sh
. "$(dirname -- "$0")/common.sh"

need docker python3
require_env_file

token=$(read_secret "$SECRETS/netbox-token")
if [ "${#token}" -ne 40 ]; then
    printf 'netbox token must be 40 characters\n' >&2
    exit 1
fi

compose exec -T netbox \
    /opt/netbox/venv/bin/python /opt/netbox/netbox/manage.py shell \
    --no-startup --no-imports --interface python <<'PY'
import os
import sys

from users.choices import TokenVersionChoices
from users.models import Token, User

plaintext = os.environ.get("NETBOX_HARNESS_TOKEN", "").strip()
if len(plaintext) != 40:
    sys.exit("NETBOX_HARNESS_TOKEN must be 40 characters")
user = User.objects.get(username="admin")
if Token.objects.filter(plaintext=plaintext).exists():
    print("NetBox token already present.")
else:
    Token.objects.create(
        user=user,
        token=plaintext,
        version=TokenVersionChoices.V1,
        write_enabled=True,
        description="harness",
    )
    print("NetBox token created.")
PY

python3 "$ROOT/scripts/seed-netbox.py"
printf 'NetBox seed finished.\n'
