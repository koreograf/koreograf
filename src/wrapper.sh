#!/bin/sh
# Job entrypoint. Koreograf sets the KOREOGRAF_* variables.
# shellcheck disable=SC2153 # The runner exports KOREOGRAF_* before this script starts.
set -u

log() {
  printf '%s\n' "KOREOGRAF $1"
}

mkdir -p "$KOREOGRAF_COLLECTIONS" "$KOREOGRAF_ROLES" "$KOREOGRAF_TOFU" "$KOREOGRAF_WORK/tmp"

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
      log "Rolle aus Cache: $name"
    else
      cp -a "$d" "$dest"
      log "Rolle neu in den Cache gelegt: $name"
    fi
  done
}

run_ansible() {
  cd "$KOREOGRAF_PROJECT" || exit 1
  log "Collection-Cache vor dem Lauf:"
  before=$(list_cols || true)
  if [ -z "$before" ]; then
    log "  (leer)"
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
      log "Galaxy-Install fehlgeschlagen. Vorhandener Cache wird trotzdem verwendet."
    fi
    if grep -q '^roles:' "$req"; then
      log "ansible-galaxy role install -r $req"
      if ! ansible-galaxy role install -r "$req" -p "$KOREOGRAF_ROLES"; then
        log "Rollen-Install fehlgeschlagen. Vorhandener Cache wird trotzdem verwendet."
      fi
    fi
  else
    log "Keine requirements.yml, Galaxy-Schritt übersprungen."
  fi
  log "Collection-Cache nach dem Lauf:"
  after=$(list_cols || true)
  if [ -z "$after" ]; then
    log "  (leer)"
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
  if [ -n "${KOREOGRAF_VAULT_FILE:-}" ]; then
    set -- "$@" --vault-password-file "$KOREOGRAF_VAULT_FILE"
  fi
  log "Starte: $*"
  exec "$@"
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
  log "OpenTofu Plugin-Cache: $TF_PLUGIN_CACHE_DIR"
  before=$(find "$TF_PLUGIN_CACHE_DIR" -type f 2>/dev/null | wc -l | tr -d ' ')
  log "Plugin-Dateien im Cache vorher: $before"
  tofu init -input=false -no-color
  after=$(find "$TF_PLUGIN_CACHE_DIR" -type f 2>/dev/null | wc -l | tr -d ' ')
  log "Plugin-Dateien im Cache nachher: $after"
  if [ "$KOREOGRAF_ACTION" = "apply" ]; then
    log "tofu apply"
    exec tofu apply -input=false -auto-approve -no-color
  fi
  log "tofu plan"
  exec tofu plan -input=false -no-color
}

run_shell() {
  cd "$KOREOGRAF_PROJECT" || exit 1
  log "Shell-Skript: $KOREOGRAF_ENTRYPOINT"
  log "Cache-Verzeichnis: $KOREOGRAF_COLLECTIONS"
  exec sh "$KOREOGRAF_ENTRYPOINT"
}

case "$KOREOGRAF_TOOL" in
  ansible) run_ansible ;;
  opentofu) run_tofu ;;
  shell) run_shell ;;
  *)
    log "Unbekanntes Werkzeug: $KOREOGRAF_TOOL"
    exit 2
    ;;
esac
