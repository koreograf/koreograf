#!/bin/sh
# Build the lab and seed LDAP and NetBox. Safe to run again.
set -eu
# shellcheck source-path=SCRIPTDIR
# shellcheck source=common.sh
. "$(dirname -- "$0")/common.sh"

need docker openssl ssh-keygen
umask 077
mkdir -p "$SECRETS"

gen() {
    name=$1
    nbytes=$2
    dest="$SECRETS/$name"
    if [ ! -s "$dest" ]; then
        openssl rand -hex "$nbytes" | tr -d '\n' > "$dest"
        if [ ! -s "$dest" ]; then
            printf 'failed to write %s\n' "$name" >&2
            exit 1
        fi
        chmod 600 "$dest"
    fi
}

gen admin-password 16
gen ldap-admin-password 16
gen ldap-user-password 16
gen netbox-db-password 16
gen netbox-redis-password 16
gen netbox-secret-key 32
gen netbox-superuser-password 16
# NetBox rejects an API token pepper shorter than 50 characters.
gen netbox-api-token-pepper 32
pepper_file="$SECRETS/netbox-api-token-pepper"
pepper_len=$(wc -c < "$pepper_file" | tr -d ' ')
if [ "$pepper_len" -lt 50 ]; then
    openssl rand -hex 32 | tr -d '\n' > "$pepper_file"
    chmod 600 "$pepper_file"
fi
# NetBox v1 tokens are exactly 40 characters.
gen netbox-token 20

if [ ! -s "$SECRETS/id_ed25519" ]; then
    ssh-keygen -q -t ed25519 -N "" -C "koreograf-harness" -f "$SECRETS/id_ed25519"
    chmod 600 "$SECRETS/id_ed25519"
fi

{
    printf 'KOREOGRAF_ADMIN_PASSWORD=%s\n' "$(read_secret "$SECRETS/admin-password")"
    printf 'LDAP_ADMIN_PASSWORD=%s\n' "$(read_secret "$SECRETS/ldap-admin-password")"
    printf 'LDAP_USER_PASSWORD=%s\n' "$(read_secret "$SECRETS/ldap-user-password")"
    printf 'NETBOX_DB_PASSWORD=%s\n' "$(read_secret "$SECRETS/netbox-db-password")"
    printf 'NETBOX_REDIS_PASSWORD=%s\n' "$(read_secret "$SECRETS/netbox-redis-password")"
    printf 'NETBOX_SECRET_KEY=%s\n' "$(read_secret "$SECRETS/netbox-secret-key")"
    printf 'NETBOX_SUPERUSER_PASSWORD=%s\n' "$(read_secret "$SECRETS/netbox-superuser-password")"
    printf 'NETBOX_API_TOKEN_PEPPER=%s\n' "$(read_secret "$SECRETS/netbox-api-token-pepper")"
    printf 'NETBOX_HARNESS_TOKEN=%s\n' "$(read_secret "$SECRETS/netbox-token")"
    printf 'HARNESS_SSH_PUBLIC_KEY=%s\n' "$(read_secret "$SECRETS/id_ed25519.pub")"
} > "$ENV_FILE"
chmod 600 "$ENV_FILE"

printf 'Starting the harness. The first build compiles Koreograf and can take a long time.\n'
compose up --build -d
compose up --wait --wait-timeout 900

"$ROOT/scripts/seed-ldap.sh"
"$ROOT/scripts/seed-netbox.sh"

printf 'Harness is up. UI: http://127.0.0.1:18080\n'
printf 'Next: tests/harness/scripts/verify.sh\n'
