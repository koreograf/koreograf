#!/bin/sh
# Add the lab user and the Plattform group. Existing entries are left as they are.
set -eu
# shellcheck source-path=SCRIPTDIR
# shellcheck source=common.sh
. "$(dirname -- "$0")/common.sh"

need docker mktemp
require_env_file

bind=$(read_secret "$SECRETS/ldap-admin-password")
user_password=$(read_secret "$SECRETS/ldap-user-password")
hashed=$(printf '%s' "$user_password" | compose exec -T ldap slappasswd -T /dev/stdin)
hashed=$(printf '%s' "$hashed" | tr -d '\n\r')
if [ -z "$hashed" ]; then
    printf 'slappasswd returned an empty hash\n' >&2
    exit 1
fi

ldif=$(mktemp)
trap 'rm -f "$ldif"' EXIT
cat > "$ldif" <<EOF
dn: ou=people,dc=example,dc=com
objectClass: organizationalUnit
ou: people

dn: ou=groups,dc=example,dc=com
objectClass: organizationalUnit
ou: groups

dn: uid=ada,ou=people,dc=example,dc=com
objectClass: inetOrgPerson
uid: ada
cn: Ada Lovelace
sn: Lovelace
mail: ada@example.com
userPassword: ${hashed}

dn: cn=Plattform,ou=groups,dc=example,dc=com
objectClass: groupOfNames
cn: Plattform
member: uid=ada,ou=people,dc=example,dc=com
EOF

set +e
out=$(compose exec -T ldap ldapadd -c -x \
    -D "cn=admin,dc=example,dc=com" \
    -w "$bind" \
    -f /dev/stdin < "$ldif" 2>&1)
code=$?
set -e
if [ "$code" -ne 0 ]; then
    bad=$(printf '%s\n' "$out" | grep 'ldap_add:' | grep -v 'Already exists' || true)
    if [ -n "$bad" ] || ! printf '%s\n' "$out" | grep -q 'Already exists'; then
        printf '%s\n' "$out" >&2
        exit 1
    fi
fi
printf 'LDAP seed finished.\n'
