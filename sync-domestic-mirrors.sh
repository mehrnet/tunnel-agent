#!/usr/bin/env bash
# shellcheck disable=SC2034
# Publish one immutable local bundle version to every configured domestic mirror.
set -uo pipefail
REPOSITORY_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
readonly REPOSITORY_DIR
readonly STATIC_DIR="$REPOSITORY_DIR/tunnel-files/static"
readonly DEFAULT_ACCOUNTS_FILE="$REPOSITORY_DIR/private/domestic-mirror-accounts.csv"

accounts_file=${DOMESTIC_MIRROR_ACCOUNTS:-$DEFAULT_ACCOUNTS_FILE}
version=${BUNDLE_VERSION:-}
mirror_filter=''
dry_run=false
force=false
check_ftp=false
prune=false
prune_before_upload=false
temporary_dir=''
cpanel_base=''
cpanel_cookie=''

usage() {
  cat <<'EOF'
Usage: ./sync-domestic-mirrors.sh [options]

Synchronize the newest local immutable bundle release (or --version) and
install.sh to the domestic FTP/FTPS mirror accounts in private/
domestic-mirror-accounts.csv. The script uploads chunks before their .parts
manifest so an incomplete release is never advertised.

Options:
  --version VERSION  Publish a specific local version, such as v0.4.9.
  --mirror ID        Publish only one CSV mirror ID.
  --dry-run          Validate inputs and show planned uploads only.
  --force            Upload every artifact even if its public copy matches.
  --check-ftp        Verify the configured mirror upload access without writing.
  --prune            Remove stale bundles only after the selected release is synced.
  --prune-before-upload
                     Remove stale bundles before syncing. Use only when storage
                     cannot hold both an old and the selected release.
  --help             Show this help.

The credentials CSV is private, must be owner-readable only, and is excluded
from Git. Transfers require explicit FTPS; plaintext FTP is never attempted.
EOF
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command is unavailable: $1"
}

sha256_file() {
  sha256sum "$1" | awk '{print $1}'
}

file_bytes() {
  wc -c < "$1" | tr -d '[:space:]'
}

validate_version() {
  [[ $1 =~ ^v[0-9A-Za-z._+-]+$ ]] || die "invalid bundle version: $1"
}

latest_version() {
  find "$STATIC_DIR/bundles" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' \
    | grep -E '^v[0-9A-Za-z._+-]+$' | sort -V | tail -n 1
}

validate_private_file() {
  [[ -f $accounts_file ]] || die "mirror credentials file is missing: $accounts_file"
  [[ -O $accounts_file ]] || die "mirror credentials file must be owned by the current user"
  local mode
  mode=$(stat -c '%a' "$accounts_file") || die "cannot read permissions for: $accounts_file"
  [[ ${mode: -2} == '00' ]] || die "mirror credentials file must not be readable by group or others"
}

validate_manifest() {
  local manifest=$1 bundle_dir=$2 line fields part_sha part_bytes part_name part_path parts=0
  while IFS= read -r line || [[ -n $line ]]; do
    case $line in
      'PART '*)
        fields=${line#PART }
        part_sha=${fields%% *}; fields=${fields#* }
        part_bytes=${fields%% *}; part_name=${fields#* }
        [[ $part_name != "$fields" && $part_name != */* && $part_name != .* ]] || die "invalid part entry: $manifest"
        [[ $part_sha =~ ^[0-9a-f]{64}$ && $part_bytes =~ ^[0-9]+$ ]] || die "invalid part digest: $manifest"
        part_path="$bundle_dir/$part_name"
        [[ -f $part_path ]] || die "missing part referenced by $manifest: $part_name"
        [[ $(file_bytes "$part_path") == "$part_bytes" ]] || die "invalid part size: $part_name"
        [[ $(sha256_file "$part_path") == "$part_sha" ]] || die "invalid part checksum: $part_name"
        parts=$((parts + 1))
        ;;
    esac
  done < "$manifest"
  (( parts > 0 )) || die "bundle manifest has no parts: $manifest"
}

validate_release() {
  local bundle_dir="$STATIC_DIR/bundles/$version" manifest
  [[ -x $STATIC_DIR/install.sh ]] || die "installer is missing or not executable: $STATIC_DIR/install.sh"
  [[ -d $bundle_dir ]] || die "local bundle version is missing: $bundle_dir"
  shopt -s nullglob
  local manifests=("$bundle_dir"/*.tar.gz.parts)
  shopt -u nullglob
  (( ${#manifests[@]} == 2 )) || die "expected two architecture manifests in: $bundle_dir"
  for manifest in "${manifests[@]}"; do
    validate_manifest "$manifest" "$bundle_dir"
  done
}

# shellcheck source=tools/domestic-mirror-transports.sh
source "$REPOSITORY_DIR/tools/domestic-mirror-transports.sh"

while (( $# )); do
  case $1 in
    --version) shift; (( $# )) || die '--version requires a value'; version=$1 ;;
    --mirror) shift; (( $# )) || die '--mirror requires a value'; mirror_filter=$1 ;;
    --dry-run) dry_run=true ;;
    --force) force=true ;;
    --check-ftp) check_ftp=true ;;
    --prune) prune=true ;;
    --prune-before-upload) prune=true; prune_before_upload=true ;;
    --help|-h) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
  shift
done

require_command curl
require_command sha256sum
require_command stat
validate_private_file
[[ -n $version ]] || version=$(latest_version)
[[ -n $version ]] || die 'no local bundle version is available'
validate_version "$version"
validate_release
[[ $prune == false || $check_ftp == false ]] || die '--prune cannot be combined with --check-ftp'
[[ $prune == false || $dry_run == false ]] || die '--prune cannot be combined with --dry-run'

temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/tunnel-agent-mirrors.XXXXXX")
chmod 700 "$temporary_dir"
netrc="$temporary_dir/netrc"
: > "$netrc"
chmod 600 "$netrc"
trap 'truncate -s 0 "$temporary_dir"/* 2>/dev/null || :' EXIT

line_number=0
succeeded=0
failed=0
while IFS=, read -r id origin host port username password control_panel remote_root sync_method tls_insecure _notes extra; do
  line_number=$((line_number + 1))
  [[ $line_number -gt 1 && -n $id ]] || continue
  [[ -z $extra && -n $origin && -n $host && -n $port && -n $username && -n $password && -n $remote_root && -n $sync_method ]] || {
    printf 'invalid mirror CSV row %d\n' "$line_number" >&2; failed=$((failed + 1)); continue;
  }
  [[ $id =~ ^[a-z0-9-]+$ && $origin =~ ^https?://[^[:space:],/]+$ && $host != *[[:space:],]* && $port =~ ^[0-9]+$ ]] || {
    printf 'invalid mirror CSV values on row %d\n' "$line_number" >&2; failed=$((failed + 1)); continue;
  }
  [[ $tls_insecure == true || $tls_insecure == false ]] || {
    printf 'invalid FTPS verification setting on row %d\n' "$line_number" >&2; failed=$((failed + 1)); continue;
  }
  [[ $sync_method == ftps || $sync_method == cpanel ]] || {
    printf 'invalid sync method on row %d\n' "$line_number" >&2; failed=$((failed + 1)); continue;
  }
  [[ $sync_method != cpanel || $control_panel =~ ^https://[^[:space:],]+$ ]] || {
    printf 'cPanel URL is invalid on row %d\n' "$line_number" >&2; failed=$((failed + 1)); continue;
  }
  [[ -z $mirror_filter || $id == "$mirror_filter" ]] || continue
  [[ $sync_method != ftps ]] || append_netrc "$netrc" "$host" "$username" "$password"
  sync_ok=false
  if [[ $prune_before_upload == true ]]; then
    if [[ $sync_method == cpanel ]]; then
      cpanel_login "$id" "$control_panel" "$username" "$password" && cpanel_root="/home/$username/${remote_root#/}" && \
        cpanel_prune_stale_releases "$id"
    else
      prune_stale_releases "$id" "$host" "$port" "$remote_root" "$tls_insecure" "$netrc"
    fi || {
      printf '[%s] prune failed\n' "$id" >&2; failed=$((failed + 1)); continue;
    }
  fi
  if [[ $sync_method == ftps ]]; then
    sync_mirror "$id" "$origin" "$host" "$port" "$username" "$password" "$remote_root" "$tls_insecure" "$netrc" && sync_ok=true
  else
    sync_cpanel_mirror "$id" "$origin" "$control_panel" "$username" "$password" "$remote_root" && sync_ok=true
  fi
  if [[ $sync_ok == true ]]; then
    if [[ $prune == true && $prune_before_upload == false ]]; then
      if [[ $sync_method == cpanel ]]; then
        cpanel_login "$id" "$control_panel" "$username" "$password" && cpanel_root="/home/$username/${remote_root#/}" && \
          cpanel_prune_stale_releases "$id"
      else
        prune_stale_releases "$id" "$host" "$port" "$remote_root" "$tls_insecure" "$netrc"
      fi || {
        printf '[%s] prune failed\n' "$id" >&2; failed=$((failed + 1)); continue;
      }
    fi
    succeeded=$((succeeded + 1))
  else
    printf '[%s] sync failed\n' "$id" >&2
    failed=$((failed + 1))
  fi
done < "$accounts_file"
(( succeeded > 0 || failed > 0 )) || die 'no mirror was selected'
(( failed == 0 )) || die "$failed mirror sync(s) failed"
printf 'published %s to %d domestic mirror(s)\n' "$version" "$succeeded"
