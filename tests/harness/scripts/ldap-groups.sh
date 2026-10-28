#!/bin/sh
# Group-search check against OpenLDAP. cargo test does not run this script.
# The user cannot read ou=groups. Sign-in still returns that group's team.
# A second sign-in drops the service re-bind and must still succeed.
set -eu
# shellcheck source-path=SCRIPTDIR
# shellcheck source=common.sh
. "$(dirname -- "$0")/common.sh"

# compose() in common.sh reads these names.
export COMPOSE_FILE="$ROOT/ldap-groups.yml"
export ENV_FILE="$SECRETS/ldap-groups.env"
BASE_DN="dc=example,dc=com"
PEOPLE="ou=people,${BASE_DN}"
GROUP_BASE="ou=groups,${BASE_DN}"
ADA_DN="uid=ada,${PEOPLE}"
READER_DN="cn=reader,${BASE_DN}"
GROUP_DN="cn=Plattform,${GROUP_BASE}"
ADMIN_DN="cn=admin,${BASE_DN}"
LOG="$SECRETS/ldap-groups.log"
PROXY_LOG="$SECRETS/ldap-proxy.log"
APP_PID="$SECRETS/ldap-groups.pid"
PROXY_PID="$SECRETS/ldap-proxy.pid"
DATA="$SECRETS/ldap-groups-data"
APP_URL="http://127.0.0.1:18082"
LDAP_URL="ldap://127.0.0.1:1389"
PROXY_URL="ldap://127.0.0.1:1390"

if [ "${1:-}" = "down" ]; then
    shift
    need docker
    if [ ! -f "$ENV_FILE" ]; then
        printf 'Run tests/harness/scripts/ldap-groups.sh first.\n' >&2
        exit 1
    fi
    compose down "$@"
    exit 0
fi

need docker python3 openssl cargo
python3 "$ROOT/scripts/ldap-proxy.py" --self-check

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

show_file() {
    file=$1
    if [ ! -f "$file" ]; then
        return 0
    fi
    python3 - "$file" \
        "$SECRETS/admin-password" \
        "$SECRETS/ldap-user-password" \
        "$SECRETS/ldap-reader-password" << 'PY'
import pathlib
import sys

text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
for name in sys.argv[2:]:
    path = pathlib.Path(name)
    if not path.is_file():
        continue
    secret = path.read_text(encoding="utf-8").strip()
    if secret:
        text = text.replace(secret, "[redacted]")
sys.stderr.write(text[-4000:])
if text and not text.endswith("\n"):
    sys.stderr.write("\n")
PY
}

with_pw() {
    secret=$1
    shift
    # The container shell expands $@. The host must not.
    # shellcheck disable=SC2016
    compose exec -T ldap sh -c '
        umask 077
        cat > /tmp/lab-pw
        rc=0
        "$@" -y /tmp/lab-pw || rc=$?
        rm -f /tmp/lab-pw
        exit "$rc"
    ' sh "$@" < "$secret"
}

copy_in() {
    dest=$1
    src=$2
    # The container shell expands $1. The host must not.
    # shellcheck disable=SC2016
    compose exec -T ldap sh -c 'umask 077; cat > "$1"' sh "$dest" < "$src"
}

discover_db() {
    # cn=admin,cn=config does not use LDAP_ADMIN_PASSWORD. Root in the
    # container can read cn=config over the local socket.
    if ! compose exec -T ldap \
        ldapsearch -Q -LLL -Y EXTERNAL -H ldapi:/// \
        -b cn=config "(olcSuffix=${BASE_DN})" dn > "$tmp/db.ldif"
    then
        printf 'could not read cn=config\n' >&2
        exit 1
    fi
    found=$(sed -n 's/^dn:[[:space:]]*//p' "$tmp/db.ldif" | head -n 1 | tr -d '\r')
    if [ -z "$found" ]; then
        printf 'OpenLDAP config has no database for %s\n' "$BASE_DN" >&2
        compose exec -T ldap \
            ldapsearch -Q -LLL -Y EXTERNAL -H ldapi:/// \
            -b cn=config "(objectClass=olcDatabaseConfig)" dn >&2 || true
        exit 1
    fi
    printf '%s' "$found"
}

seed_directory() {
    hash_ada=$(compose exec -T ldap slappasswd -T /dev/stdin < "$SECRETS/ldap-user-password" | tr -d '\n\r')
    hash_reader=$(compose exec -T ldap slappasswd -T /dev/stdin < "$SECRETS/ldap-reader-password" | tr -d '\n\r')
    if [ -z "$hash_ada" ] || [ -z "$hash_reader" ]; then
        printf 'slappasswd returned an empty hash\n' >&2
        exit 1
    fi
    cat > "$tmp/seed.ldif" << EOF
dn: ${PEOPLE}
objectClass: organizationalUnit
ou: people

dn: ${GROUP_BASE}
objectClass: organizationalUnit
ou: groups

dn: ${ADA_DN}
objectClass: inetOrgPerson
uid: ada
cn: Ada Lovelace
sn: Lovelace
mail: ada@example.com
userPassword: ${hash_ada}

dn: ${GROUP_DN}
objectClass: groupOfNames
cn: Plattform
member: ${ADA_DN}

dn: ${READER_DN}
objectClass: organizationalRole
objectClass: simpleSecurityObject
cn: reader
description: Service account for group search
userPassword: ${hash_reader}
EOF
    copy_in /tmp/lab.ldif "$tmp/seed.ldif"
    set +e
    out=$(with_pw "$SECRETS/ldap-admin-password" \
        ldapadd -c -x -H ldap://127.0.0.1:389 \
        -D "$ADMIN_DN" -f /tmp/lab.ldif 2>&1)
    code=$?
    set -e
    if [ "$code" -ne 0 ]; then
        bad=$(printf '%s\n' "$out" | grep 'ldap_add:' | grep -v 'Already exists' || true)
        if [ -n "$bad" ] || ! printf '%s\n' "$out" | grep -q 'Already exists'; then
            printf 'ldapadd failed\n' >&2
            printf '%s\n' "$out" | grep 'ldap_' >&2 || true
            exit 1
        fi
    fi
    cat > "$tmp/passwords.ldif" << EOF
dn: ${ADA_DN}
changetype: modify
replace: userPassword
userPassword: ${hash_ada}

dn: ${READER_DN}
changetype: modify
replace: userPassword
userPassword: ${hash_reader}
EOF
    copy_in /tmp/lab.ldif "$tmp/passwords.ldif"
    with_pw "$SECRETS/ldap-admin-password" \
        ldapmodify -x -H ldap://127.0.0.1:389 \
        -D "$ADMIN_DN" -f /tmp/lab.ldif >/dev/null
    compose exec -T ldap rm -f /tmp/lab.ldif
}

apply_acl() {
    db_dn=$(discover_db)
    printf 'OpenLDAP database is %s\n' "$db_dn"
    cat > "$tmp/acl.ldif" << EOF
dn: ${db_dn}
changetype: modify
replace: olcAccess
olcAccess: {0}to attrs=userPassword,shadowLastChange by self write by anonymous auth by * none
olcAccess: {1}to dn.subtree="${GROUP_BASE}" by dn.exact="${READER_DN}" read by * none
olcAccess: {2}to dn.subtree="${PEOPLE}" by dn.exact="${READER_DN}" read by self read by * none
olcAccess: {3}to * by * none
EOF
    copy_in /tmp/lab.ldif "$tmp/acl.ldif"
    compose exec -T ldap ldapmodify -Q -Y EXTERNAL -H ldapi:/// -f /tmp/lab.ldif
    compose exec -T ldap rm -f /tmp/lab.ldif
}

assert_acl() {
    if with_pw "$SECRETS/ldap-user-password" \
        ldapsearch -x -LLL -H ldap://127.0.0.1:389 \
        -D "$ADA_DN" -b "$GROUP_BASE" -s base dn \
        > "$tmp/ada-groups.txt" 2> "$tmp/ada-groups.err"
    then
        printf 'the user can read %s\n' "$GROUP_BASE" >&2
        exit 1
    fi
    if grep -F -q 'cn=Plattform' "$tmp/ada-groups.txt"; then
        printf 'the user search returned the group\n' >&2
        exit 1
    fi
    if ! grep -E -q 'Insufficient access|No such object' "$tmp/ada-groups.err"; then
        printf 'the user search failed for an unexpected reason\n' >&2
        cat "$tmp/ada-groups.err" >&2
        exit 1
    fi
    if compose exec -T ldap \
        ldapsearch -x -H ldap://127.0.0.1:389 -b "$GROUP_BASE" -s base dn \
        > "$tmp/anon.txt" 2> "$tmp/anon.err"
    then
        printf 'anonymous can read %s\n' "$GROUP_BASE" >&2
        exit 1
    fi
    if grep -F -q 'cn=Plattform' "$tmp/anon.txt"; then
        printf 'anonymous search returned the group\n' >&2
        exit 1
    fi
    if ! with_pw "$SECRETS/ldap-reader-password" \
        ldapsearch -x -LLL -H ldap://127.0.0.1:389 \
        -D "$READER_DN" -b "$GROUP_BASE" "(objectClass=groupOfNames)" dn \
        > "$tmp/reader-groups.txt" 2> "$tmp/reader-groups.err"
    then
        printf 'the service account cannot read the group\n' >&2
        cat "$tmp/reader-groups.err" >&2
        exit 1
    fi
    if ! grep -F -q "$GROUP_DN" "$tmp/reader-groups.txt"; then
        printf 'the service account cannot read the group\n' >&2
        grep '^dn:' "$tmp/reader-groups.txt" >&2 || true
        exit 1
    fi
    if ! with_pw "$SECRETS/ldap-admin-password" \
        ldapsearch -x -LLL -H ldap://127.0.0.1:389 \
        -D "$ADMIN_DN" -b "$ADA_DN" -s base memberOf \
        > "$tmp/ada-memberof.txt" 2> "$tmp/ada-memberof.err"
    then
        printf 'could not read the user entry\n' >&2
        cat "$tmp/ada-memberof.err" >&2
        exit 1
    fi
    if grep -q '^memberOf:' "$tmp/ada-memberof.txt"; then
        printf 'the user entry has memberOf, so sign-in would skip the group search\n' >&2
        exit 1
    fi
    if ! with_pw "$SECRETS/ldap-user-password" \
        ldapsearch -x -LLL -H ldap://127.0.0.1:389 \
        -D "$ADA_DN" -b "$ADA_DN" -s base uid \
        > "$tmp/ada-self.txt" 2> "$tmp/ada-self.err"
    then
        printf 'the user cannot read their own entry\n' >&2
        cat "$tmp/ada-self.err" >&2
        exit 1
    fi
    printf 'only the service account can read %s\n' "$GROUP_BASE"
}

cleanup() {
    rc=$?
    if [ "$rc" -ne 0 ]; then
        printf 'ldap group-search check failed\n' >&2
        show_file "$LOG" || true
        show_file "$PROXY_LOG" || true
    fi
    if [ -f "$APP_PID" ]; then
        pid=$(cat "$APP_PID" 2>/dev/null || true)
        if [ -n "${pid:-}" ]; then
            kill -- "-$pid" 2>/dev/null || true
        fi
        rm -f "$APP_PID"
    fi
    if [ -f "$PROXY_PID" ]; then
        pid=$(cat "$PROXY_PID" 2>/dev/null || true)
        if [ -n "${pid:-}" ]; then
            kill "$pid" 2>/dev/null || true
        fi
        rm -f "$PROXY_PID"
    fi
    if [ -f "$ENV_FILE" ]; then
        compose down >/dev/null 2>&1 || true
    fi
    if [ -n "${tmp:-}" ]; then
        rm -rf "$tmp"
    fi
    exit "$rc"
}

umask 077
mkdir -p "$SECRETS" "$DATA"
gen admin-password 16
gen ldap-admin-password 16
gen ldap-user-password 16
gen ldap-reader-password 16
{
    printf 'LDAP_ADMIN_PASSWORD=%s\n' "$(read_secret "$SECRETS/ldap-admin-password")"
} > "$ENV_FILE"
chmod 600 "$ENV_FILE"

tmp=$(mktemp -d)
trap cleanup EXIT

printf 'Starting OpenLDAP for the group-search check.\n'
compose up -d --wait --wait-timeout 600
seed_directory
apply_acl
assert_acl

: > "$PROXY_LOG"
python3 "$ROOT/scripts/ldap-proxy.py" \
    --listen 127.0.0.1:1390 \
    --upstream 127.0.0.1:1389 \
    --reader-dn "$READER_DN" >> "$PROXY_LOG" 2>&1 &
printf '%s\n' "$!" > "$PROXY_PID"
sleep 0.3
if ! kill -0 "$(cat "$PROXY_PID")" 2>/dev/null; then
    printf 'LDAP proxy exited\n' >&2
    exit 1
fi
if ! grep -q 'ldap proxy listening' "$PROXY_LOG"; then
    sleep 0.5
fi
if ! grep -q 'ldap proxy listening' "$PROXY_LOG"; then
    printf 'LDAP proxy did not listen\n' >&2
    exit 1
fi

rm -rf "$DATA"
mkdir -p "$DATA"
python3 "$ROOT/scripts/ldap-groups.py" \
    --base "$APP_URL" \
    --ldap-url "$LDAP_URL" \
    --proxy-url "$PROXY_URL" \
    --secrets "$SECRETS" \
    --data "$DATA" \
    --log "$LOG" \
    --pid-file "$APP_PID" \
    --proxy-log "$PROXY_LOG" \
    --reader-dn "$READER_DN" \
    --user-base "$PEOPLE" \
    --group-base "$GROUP_BASE" \
    --group-dn "$GROUP_DN"

printf 'LDAP group-search check passed.\n'
