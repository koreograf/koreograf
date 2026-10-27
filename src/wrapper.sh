#!/bin/sh
# Job entrypoint. Koreograf sets the KOREOGRAF_* variables.
# shellcheck disable=SC2153 # The runner exports KOREOGRAF_* before this script starts.
set -u

log() {
  printf '%s\n' "KOREOGRAF $1"
}

# OpenSSH aborts locally with "No user exists for uid …" when getpwuid()
# fails, before it dials the remote host. ansible-builder images run as a
# numeric uid and append that uid from their entrypoint. Koreograf replaces
# the entrypoint, so the same line is written here. A local run keeps
# KOREOGRAF_WORK on the host and must not touch /etc/passwd.
uid_in_passwd() {
  probe=$1
  if command -v getent >/dev/null 2>&1; then
    getent passwd "$probe" >/dev/null 2>&1
    return $?
  fi
  awk -F: -v uid="$probe" '$3 == uid { found = 1 } END { exit found ? 0 : 1 }' /etc/passwd
}

ensure_container_user() {
  uid=$(id -u)
  if uid_in_passwd "$uid"; then
    return 0
  fi
  if [ ! -w /etc/passwd ]; then
    log "UID ${uid} is missing from /etc/passwd and the file is not writable. SSH then fails with 'No user exists for uid'."
    return 0
  fi
  gid=$(id -g)
  home="${HOME:-/runner/work}"
  if [ -s /etc/passwd ]; then
    nl=$(printf '\n')
    last=$(tail -c 1 /etc/passwd 2>/dev/null || true)
    if [ "$last" != "$nl" ]; then
      printf '\n' >> /etc/passwd
    fi
  fi
  printf '%s\n' "${uid}:x:${uid}:${gid}:container user:${home}:/bin/sh" >> /etc/passwd
  log "UID ${uid} was missing from /etc/passwd and was added."
}

if [ "${KOREOGRAF_WORK}" = "/runner/work" ]; then
  ensure_container_user
fi

mkdir -p "$KOREOGRAF_COLLECTIONS" "$KOREOGRAF_ROLES" "$KOREOGRAF_TOFU" "$KOREOGRAF_WORK/tmp"

if [ -n "${KOREOGRAF_GIT_PASSWORD_FILE:-}" ]; then
  cat > "$KOREOGRAF_WORK/git-askpass" <<'EOF'
#!/bin/sh
case "$1" in
  *sername*) printf '%s\n' "${KOREOGRAF_GIT_USERNAME:-git}" ;;
  *) cat "$KOREOGRAF_GIT_PASSWORD_FILE" ;;
esac
EOF
  chmod 700 "$KOREOGRAF_WORK/git-askpass"
  export GIT_ASKPASS="$KOREOGRAF_WORK/git-askpass"
  export GIT_TERMINAL_PROMPT=0
fi
if [ -n "${KOREOGRAF_GIT_SSH_KEY:-}" ]; then
  export GIT_SSH_COMMAND="ssh -i \"$KOREOGRAF_GIT_SSH_KEY\" -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new"
fi

list_cols() {
  root="$KOREOGRAF_COLLECTIONS/ansible_collections"
  if [ -d "$root" ]; then
    find "$root" -mindepth 2 -maxdepth 2 -type d | sed "s|^$root/||" | sort
  fi
}

cache_roles() {
  src="$KOREOGRAF_PROJECT/roles"
  if [ ! -d "$src" ]; then
    return 0
  fi
  for d in "$src"/*; do
    if [ ! -d "$d" ]; then
      continue
    fi
    name=$(basename "$d")
    dest="$KOREOGRAF_ROLES/$name"
    if [ -d "$dest" ]; then
      log "Role from cache: $name"
    else
      cp -a "$d" "$dest"
      log "Role added to the cache: $name"
    fi
  done
}

run_ansible() {
  cd "$KOREOGRAF_PROJECT" || exit 1
  log "Collection cache before the run:"
  before=$(list_cols || true)
  if [ -z "$before" ]; then
    log "  (empty)"
  else
    printf '%s\n' "$before" | while IFS= read -r line; do
      log "  $line"
    done
  fi
  cache_roles
  req=""
  if [ -f requirements.yml ]; then
    req="requirements.yml"
  elif [ -f collections/requirements.yml ]; then
    req="collections/requirements.yml"
  fi
  if [ -n "$req" ]; then
    log "ansible-galaxy collection install -r $req"
    if ! ansible-galaxy collection install -r "$req" -p "$KOREOGRAF_COLLECTIONS"; then
      log "Galaxy install failed. The existing cache is used anyway."
    fi
    if grep -q '^roles:' "$req"; then
      log "ansible-galaxy role install -r $req"
      if ! ansible-galaxy role install -r "$req" -p "$KOREOGRAF_ROLES"; then
        log "Role install failed. The existing cache is used anyway."
      fi
    fi
  else
    log "No requirements.yml, Galaxy step skipped."
  fi
  log "Collection cache after the run:"
  after=$(list_cols || true)
  if [ -z "$after" ]; then
    log "  (empty)"
  else
    printf '%s\n' "$after" | while IFS= read -r line; do
      log "  $line"
    done
  fi
  export ANSIBLE_COLLECTIONS_PATH="$KOREOGRAF_COLLECTIONS"
  export ANSIBLE_ROLES_PATH="$KOREOGRAF_ROLES:$KOREOGRAF_PROJECT/roles"
  export ANSIBLE_HOST_KEY_CHECKING=False
  export ANSIBLE_FORCE_COLOR=0
  export ANSIBLE_RETRY_FILES_ENABLED=0
  export ANSIBLE_LOCAL_TEMP="$KOREOGRAF_WORK/tmp"
  export ANSIBLE_SSH_CONTROL_PATH_DIR="$KOREOGRAF_WORK/tmp/cp"
  if [ -n "${KOREOGRAF_PRIVATE_KEY:-}" ]; then
    export ANSIBLE_PRIVATE_KEY_FILE="$KOREOGRAF_PRIVATE_KEY"
  fi
  if [ -n "${KOREOGRAF_REMOTE_USER:-}" ]; then
    export ANSIBLE_REMOTE_USER="$KOREOGRAF_REMOTE_USER"
  fi
  if [ -n "${KOREOGRAF_CONNECTION_PASSWORD_FILE:-}" ]; then
    export ANSIBLE_CONNECTION_PASSWORD_FILE="$KOREOGRAF_CONNECTION_PASSWORD_FILE"
  fi
  if [ -n "${KOREOGRAF_BECOME_PASSWORD_FILE:-}" ]; then
    export ANSIBLE_BECOME_PASSWORD_FILE="$KOREOGRAF_BECOME_PASSWORD_FILE"
  fi
  if [ -n "${KOREOGRAF_BECOME_METHOD:-}" ]; then
    export ANSIBLE_BECOME=1
    export ANSIBLE_BECOME_METHOD="$KOREOGRAF_BECOME_METHOD"
  fi
  if [ -n "${KOREOGRAF_BECOME_USER:-}" ]; then
    export ANSIBLE_BECOME_USER="$KOREOGRAF_BECOME_USER"
  fi
  if [ -n "${KOREOGRAF_KEY_PASSPHRASE_FILE:-}" ] || [ -n "${KOREOGRAF_CONNECTION_PASSWORD_FILE:-}" ]; then
    cat > "$KOREOGRAF_WORK/askpass" <<'EOF'
#!/bin/sh
case "$1" in
  *assphrase*)
    if [ -n "${KOREOGRAF_KEY_PASSPHRASE_FILE:-}" ]; then
      cat "$KOREOGRAF_KEY_PASSPHRASE_FILE"
    fi
    ;;
  *)
    if [ -n "${KOREOGRAF_CONNECTION_PASSWORD_FILE:-}" ]; then
      cat "$KOREOGRAF_CONNECTION_PASSWORD_FILE"
    fi
    ;;
esac
EOF
    chmod 700 "$KOREOGRAF_WORK/askpass"
    export SSH_ASKPASS="$KOREOGRAF_WORK/askpass"
    export SSH_ASKPASS_REQUIRE=force
    if [ -z "${DISPLAY:-}" ]; then
      export DISPLAY=none
    fi
  fi
  set -- ansible-playbook -i "$KOREOGRAF_INVENTORY" "$KOREOGRAF_ENTRYPOINT" --extra-vars "@$KOREOGRAF_EXTRA"
  if [ "${KOREOGRAF_CHECK:-}" = "1" ]; then
    set -- "$@" --check
  fi
  if [ "${KOREOGRAF_DIFF:-}" = "1" ]; then
    set -- "$@" --diff
  fi
  if [ -n "${KOREOGRAF_TAGS:-}" ]; then
    set -- "$@" --tags "$KOREOGRAF_TAGS"
  fi
  if [ -n "${KOREOGRAF_SKIP_TAGS:-}" ]; then
    set -- "$@" --skip-tags "$KOREOGRAF_SKIP_TAGS"
  fi
  if [ -n "${KOREOGRAF_LIMIT:-}" ]; then
    set -- "$@" --limit "$KOREOGRAF_LIMIT"
  fi
  if [ -n "${KOREOGRAF_FORKS:-}" ]; then
    set -- "$@" --forks "$KOREOGRAF_FORKS"
  fi
  case "${KOREOGRAF_VERBOSITY:-0}" in
    1) set -- "$@" -v ;;
    2) set -- "$@" -vv ;;
    3) set -- "$@" -vvv ;;
    4) set -- "$@" -vvvv ;;
  esac
  if [ -n "${KOREOGRAF_VAULT_LIST:-}" ]; then
    # A pipe keeps an empty vault id. A tab is IFS whitespace, so `read` would
    # drop that leading field and skip the password file.
    while IFS='|' read -r vid vpath; do
      [ -n "$vpath" ] || continue
      if [ -n "$vid" ]; then
        set -- "$@" --vault-id "${vid}@${vpath}"
      else
        set -- "$@" --vault-password-file "$vpath"
      fi
    done < "$KOREOGRAF_VAULT_LIST"
  elif [ -n "${KOREOGRAF_VAULT_FILE:-}" ]; then
    if [ -n "${KOREOGRAF_VAULT_ID:-}" ]; then
      set -- "$@" --vault-id "${KOREOGRAF_VAULT_ID}@${KOREOGRAF_VAULT_FILE}"
    else
      set -- "$@" --vault-password-file "$KOREOGRAF_VAULT_FILE"
    fi
  fi
  if [ -n "${KOREOGRAF_AUTH_EXTRA:-}" ]; then
    set -- "$@" --extra-vars "@$KOREOGRAF_AUTH_EXTRA"
  fi
  log "Starting: $*"
  exec "$@"
}

backend_lines() {
  find "$1" -type f \( -name '*.tf' -o -name '*.tf.json' \) -exec grep -E -h 'backend[[:space:]]+"[A-Za-z0-9_-]+"' {} + 2>/dev/null || true
}

has_backend_block() {
  lines=$(backend_lines "$1")
  [ -n "$lines" ]
}

has_remote_backend() {
  lines=$(backend_lines "$1")
  printf '%s\n' "$lines" | grep -E -v 'backend[[:space:]]+"local"' | grep -E -q 'backend[[:space:]]+"'
}

run_tofu() {
  export TF_PLUGIN_CACHE_DIR="$KOREOGRAF_TOFU"
  export TF_DATA_DIR="$KOREOGRAF_WORK/tfdata"
  export TF_IN_AUTOMATION=1
  mkdir -p "$TF_DATA_DIR"
  src="$KOREOGRAF_PROJECT/$KOREOGRAF_ENTRYPOINT"
  stage="$KOREOGRAF_WORK/tfsrc"
  rm -rf "$stage"
  mkdir -p "$stage"
  cp -a "$src"/. "$stage"/
  cd "$stage" || exit 1

  # The local backend writes terraform.tfstate in the module directory.
  # That directory is the job work dir, so the file used to die with the job.
  # A module with no backend gets a local backend whose path lives in the
  # team cache. TF_DATA_DIR stays in the job directory. The plugin cache is
  # unchanged.
  state_dir="$KOREOGRAF_WORK/tfdata"
  state_kept=0
  if [ -n "${KOREOGRAF_TOFU_STATE:-}" ]; then
    case "$KOREOGRAF_TOFU_STATE" in
      *..*)
        log "OpenTofu state path was rejected. This run keeps state in the job directory."
        ;;
      *)
        state_dir="$KOREOGRAF_TOFU_STATE"
        state_kept=1
        ;;
    esac
  else
    log "OpenTofu state directory is missing. This run keeps state in the job directory."
  fi
  mkdir -p "$state_dir"
  state_file="$state_dir/terraform.tfstate"
  injected=0
  if has_remote_backend "$stage"; then
    log "OpenTofu uses the backend in the module. This controller does not keep a copy of that state."
  elif has_backend_block "$stage"; then
    if [ "$KOREOGRAF_ACTION" = "apply" ]; then
      log "Warning: backend \"local\" stores state on a path from the module. A relative path is removed with the job directory. Use an absolute path or a remote backend."
    fi
  elif [ "$state_kept" -eq 1 ]; then
    cat > "$stage/koreograf-local-backend.tf" <<EOF
terraform {
  backend "local" {
    path = "$state_file"
  }
}
EOF
    injected=1
    log "OpenTofu state: $state_file"
    if [ "$KOREOGRAF_ACTION" = "apply" ]; then
      log "Warning: this module has no remote backend. State for this team and template stays on this controller. A remote backend is the setup to use when that state must outlive this data directory."
    fi
  fi

  log "OpenTofu plugin cache: $TF_PLUGIN_CACHE_DIR"
  before=$(find "$TF_PLUGIN_CACHE_DIR" -type f 2>/dev/null | wc -l | tr -d ' ')
  log "Plugin files in the cache before: $before"
  tofu init -input=false -no-color
  init_code=$?
  after=$(find "$TF_PLUGIN_CACHE_DIR" -type f 2>/dev/null | wc -l | tr -d ' ')
  log "Plugin files in the cache after: $after"
  if [ "$init_code" -ne 0 ]; then
    exit "$init_code"
  fi
  if [ "$KOREOGRAF_ACTION" = "apply" ]; then
    log "tofu apply"
    tofu apply -input=false -auto-approve -no-color
    code=$?
  else
    log "tofu plan"
    tofu plan -input=false -no-color
    code=$?
  fi
  if [ "$injected" -eq 1 ] && [ -f "$state_file" ]; then
    chmod 600 "$state_file" 2>/dev/null || true
    if [ -f "$state_dir/terraform.tfstate.backup" ]; then
      chmod 600 "$state_dir/terraform.tfstate.backup" 2>/dev/null || true
    fi
  fi
  exit "$code"
}

run_shell() {
  cd "$KOREOGRAF_PROJECT" || exit 1
  log "Shell script: $KOREOGRAF_ENTRYPOINT"
  log "Cache directory: $KOREOGRAF_COLLECTIONS"
  exec sh "$KOREOGRAF_ENTRYPOINT"
}

case "$KOREOGRAF_TOOL" in
  ansible) run_ansible ;;
  opentofu) run_tofu ;;
  shell) run_shell ;;
  *)
    log "Unknown tool: $KOREOGRAF_TOOL"
    exit 2
    ;;
esac
