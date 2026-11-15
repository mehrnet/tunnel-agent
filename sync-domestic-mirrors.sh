#!/usr/bin/env bash
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
public_url() {
  printf '%s/%s' "${1%/}" "$2"
}
remote_size() {
  local headers
  headers=$(curl --noproxy '*' --fail --silent --show-error --location --head \
    --connect-timeout 10 --max-time 20 -H 'Cache-Control: no-cache' "$1" 2>/dev/null | tr -d '\r') || return 1
  printf '%s\n' "$headers" | awk -F ': *' 'tolower($1) == "content-length" { value = $2 } END { print value }'
}
remote_matches() {
  local origin=$1 relative=$2 source=$3 remote actual
  remote=$(public_url "$origin" "$relative")
  case $source in
    *.parts|*.inventory.json|install.sh)
      actual=$(curl --noproxy '*' --fail --silent --show-error --location \
        --connect-timeout 10 --max-time 20 -H 'Cache-Control: no-cache' "$remote" 2>/dev/null | sha256sum | awk '{print $1}') || return 1
      [[ $actual == "$(sha256_file "$source")" ]]
      ;;
    *) [[ $(remote_size "$remote") == "$(file_bytes "$source")" ]] ;;
  esac
}
ftp_url() {
  printf 'ftp://%s:%s/%s' "$1" "$2" "${3#/}"
}
ftp_options() {
  local tls_insecure=$1
  FTP_OPTIONS=(--ssl-reqd --ftp-pasv --disable-epsv)
  [[ $tls_insecure == true ]] && FTP_OPTIONS+=(--insecure)
}
ftp_list() {
  local host=$1 port=$2 path=$3 tls_insecure=$4 netrc=$5
  ftp_options "$tls_insecure"
  curl --noproxy '*' --fail --silent --show-error --connect-timeout 20 --max-time 120 \
    --netrc-file "$netrc" --netrc "${FTP_OPTIONS[@]}" --list-only "$(ftp_url "$host" "$port" "$path")" | tr -d '\r'
}
ftp_quote() {
  local host=$1 port=$2 remote_root=$3 command=$4 tls_insecure=$5 netrc=$6
  ftp_options "$tls_insecure"
  curl --noproxy '*' --fail --silent --show-error --connect-timeout 20 --max-time 120 \
    --netrc-file "$netrc" --netrc "${FTP_OPTIONS[@]}" --quote "$command" --list-only \
    "$(ftp_url "$host" "$port" "$remote_root/")" >/dev/null
}
prune_release() {
  local id=$1 host=$2 port=$3 remote_root=$4 stale_version=$5 tls_insecure=$6 netrc=$7
  local release_path="$remote_root/bundles/$stale_version" name listing
  local -a files=()
  listing=$(ftp_list "$host" "$port" "$release_path/" "$tls_insecure" "$netrc") || return 1
  [[ -z $listing ]] || mapfile -t files <<< "$listing"
  for name in "${files[@]}"; do
    [[ $name == . || $name == .. ]] && continue
    [[ $name != */* && $name != .* && $name == "tunnel-agent-$stale_version-"* ]] || {
      printf '[%s] refusing to prune unexpected file: %s/%s\n' "$id" "$stale_version" "$name" >&2
      return 1
    }
    ftp_quote "$host" "$port" "$remote_root" "DELE $release_path/$name" "$tls_insecure" "$netrc" || return 1
  done
  ftp_quote "$host" "$port" "$remote_root" "RMD $release_path" "$tls_insecure" "$netrc"
  printf '[%s] pruned bundles/%s\n' "$id" "$stale_version"
}
prune_stale_releases() {
  local id=$1 host=$2 port=$3 remote_root=$4 tls_insecure=$5 netrc=$6 stale_version listing
  local -a entries=()
  listing=$(ftp_list "$host" "$port" "$remote_root/bundles/" "$tls_insecure" "$netrc") || return 1
  [[ -z $listing ]] || mapfile -t entries <<< "$listing"
  for stale_version in "${entries[@]}"; do
    [[ $stale_version == . || $stale_version == .. || $stale_version == "$version" ]] && continue
    [[ $stale_version =~ ^v[0-9A-Za-z._+-]+$ ]] || continue
    prune_release "$id" "$host" "$port" "$remote_root" "$stale_version" "$tls_insecure" "$netrc" || return 1
  done
}
upload_file() {
  local id=$1 origin=$2 host=$3 port=$4 remote_root=$5 source=$6 relative=$7 tls_insecure=$8 netrc=$9
  local remote_path="$remote_root/$relative"
  ftp_options "$tls_insecure"
  if [[ $force == false ]] && remote_matches "$origin" "$relative" "$source"; then
    printf '[%s] unchanged %s\n' "$id" "$relative"
    return 0
  fi
  if [[ $dry_run == true ]]; then
    printf '[%s] upload %s\n' "$id" "$relative"
    return 0
  fi
  printf '[%s] upload %s\n' "$id" "$relative"
  curl --fail --silent --show-error --retry 2 --retry-all-errors --connect-timeout 20 --max-time 900 \
    --netrc-file "$netrc" --netrc "${FTP_OPTIONS[@]}" --ftp-create-dirs \
    --upload-file "$source" "$(ftp_url "$host" "$port" "$remote_path")"
}
probe_ftp() {
  local host=$1 port=$2 remote_root=$3 tls_insecure=$4 netrc=$5
  ftp_options "$tls_insecure"
  curl --noproxy '*' --fail --silent --show-error --connect-timeout 20 --max-time 60 \
    --netrc-file "$netrc" --netrc "${FTP_OPTIONS[@]}" --list-only \
    "$(ftp_url "$host" "$port" "$remote_root")" >/dev/null
}
append_netrc() {
  local netrc=$1 host=$2 username=$3 password=$4
  [[ $host != *[[:space:]]* && $username != *[[:space:]]* && $password != *[[:space:]]* ]] || \
    die "FTP values for $host may not contain whitespace"
  printf 'machine %s\nlogin %s\npassword %s\n' "$host" "$username" "$password" >> "$netrc"
}
cpanel_login() {
  local id=$1 control_panel=$2 username=$3 password=$4 headers session
  cpanel_cookie="$temporary_dir/$id.cookie"
  headers="$temporary_dir/$id.headers"
  curl --noproxy '*' --silent --show-error --connect-timeout 20 --max-time 60 \
    -D "$headers" -c "$cpanel_cookie" \
    --data-urlencode "user=$username" --data-urlencode "pass=$password" \
    --data-urlencode 'login_theme=cpanel' "${control_panel%/}/login/" -o /dev/null || return 1
  session=$(sed -n 's/^Location: \/\(cpsess[^/]*\).*/\1/pI' "$headers" | tail -n 1)
  [[ $session =~ ^cpsess[0-9A-Za-z]+$ ]] || return 1
  cpanel_base="${control_panel%/}/$session"
}
cpanel_mkdir() {
  local parent=$1 name=$2 response="$temporary_dir/cpanel-mkdir.json"
  curl --noproxy '*' --fail --silent --show-error --connect-timeout 20 --max-time 60 \
    -b "$cpanel_cookie" -G "$cpanel_base/json-api/cpanel" \
    --data-urlencode 'cpanel_jsonapi_module=Fileman' \
    --data-urlencode 'cpanel_jsonapi_func=mkdir' \
    --data-urlencode 'cpanel_jsonapi_apiversion=2' \
    --data-urlencode "dir=$parent" --data-urlencode "name=$name" -o "$response" || return 1
  grep -qi 'file exists' "$response" && return 0
  grep -Eq '"cpanelresult"[[:space:]]*:' "$response" && ! grep -Eq '"error"[[:space:]]*:[[:space:]]*"' "$response"
}
cpanel_delete_path() {
  local path=$1 response="$temporary_dir/cpanel-delete.json"
  curl --noproxy '*' --fail --silent --show-error --connect-timeout 20 --max-time 120 \
    -b "$cpanel_cookie" -G "$cpanel_base/execute/Fileman/delete_file" \
    --data-urlencode "path=$path" -o "$response" || return 1
  node - "$response" "$path" <<'NODE'
const fs = require('fs');
const response = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
process.exit(response.status === 1 && response.data?.path === process.argv[3] ? 0 : 1);
NODE
}
cpanel_upload_once() {
  local source=$1 remote_dir=$2 response=$3
  curl --noproxy '*' --fail --silent --show-error --retry 2 --retry-all-errors --connect-timeout 20 --max-time 900 \
    -b "$cpanel_cookie" -F "dir=$remote_dir" -F "file-1=@$source" \
    "$cpanel_base/execute/Fileman/upload_files" -o "$response" || return 1
  grep -Eq '"status"[[:space:]]*:[[:space:]]*1' "$response"
}
cpanel_upload() {
  local source=$1 relative=$2 remote_dir response target_path
  remote_dir="$cpanel_root/$(dirname "$relative")"
  [[ $remote_dir != */. ]] || remote_dir=$cpanel_root
  response="$temporary_dir/cpanel-upload.json"
  cpanel_upload_once "$source" "$remote_dir" "$response" && return 0
  grep -q 'already exists' "$response" || return 1
  target_path="$remote_dir/${source##*/}"
  cpanel_delete_path "$target_path" || return 1
  cpanel_upload_once "$source" "$remote_dir" "$response"
}
cpanel_list_release_versions() {
  local response="$temporary_dir/cpanel-list-releases.json"
  curl --noproxy '*' --fail --silent --show-error --connect-timeout 20 --max-time 60 \
    -b "$cpanel_cookie" -G "$cpanel_base/execute/Fileman/list_files" \
    --data-urlencode "dir=$cpanel_root/bundles" -o "$response" || return 1
  node - "$response" <<'NODE'
const fs = require('fs');
const entries = JSON.parse(fs.readFileSync(process.argv[2], 'utf8')).data ?? [];
for (const entry of entries) {
  if (entry.type === 'dir' && /^v[0-9A-Za-z._+-]+$/.test(entry.file)) console.log(entry.file);
}
NODE
}
cpanel_prune_release() {
  local id=$1 stale_version=$2
  [[ $stale_version =~ ^v[0-9A-Za-z._+-]+$ && $stale_version != "$version" ]] || return 1
  cpanel_delete_path "$cpanel_root/bundles/$stale_version" || return 1
  printf '[%s] pruned bundles/%s\n' "$id" "$stale_version"
}
cpanel_prune_stale_releases() {
  local id=$1 stale_version
  local -a versions=()
  mapfile -t versions < <(cpanel_list_release_versions) || return 1
  for stale_version in "${versions[@]}"; do
    [[ $stale_version == "$version" ]] || cpanel_prune_release "$id" "$stale_version" || return 1
  done
}
upload_cpanel_file() {
  local id=$1 origin=$2 source=$3 relative=$4
  if [[ $force == false ]] && remote_matches "$origin" "$relative" "$source"; then
    printf '[%s] unchanged %s\n' "$id" "$relative"
    return 0
  fi
  if [[ $dry_run == true ]]; then
    printf '[%s] upload %s\n' "$id" "$relative"
    return 0
  fi
  printf '[%s] upload %s\n' "$id" "$relative"
  cpanel_upload "$source" "$relative"
}
sync_mirror() {
  local id=$1 origin=$2 host=$3 port=$4 username=$5 password=$6 remote_root=$7 tls_insecure=$8 netrc=$9
  local bundle_dir="$STATIC_DIR/bundles/$version" name source relative
  local chunks inventories manifests
  shopt -s nullglob
  chunks=("$bundle_dir"/*.part-*)
  inventories=("$bundle_dir"/*.inventory.json)
  manifests=("$bundle_dir"/*.parts)
  shopt -u nullglob
  printf '[%s] syncing %s to %s\n' "$id" "$version" "$origin"
  if [[ $check_ftp == true ]]; then
    probe_ftp "$host" "$port" "$remote_root" "$tls_insecure" "$netrc" || return 1
    printf '[%s] FTPS authentication succeeded\n' "$id"
    return 0
  fi
  for name in "${chunks[@]}"; do
    source=$name; relative="bundles/$version/${name##*/}"
    upload_file "$id" "$origin" "$host" "$port" "$remote_root" "$source" "$relative" "$tls_insecure" "$netrc" || return 1
  done
  for name in "${inventories[@]}"; do
    source=$name; relative="bundles/$version/${name##*/}"
    upload_file "$id" "$origin" "$host" "$port" "$remote_root" "$source" "$relative" "$tls_insecure" "$netrc" || return 1
  done
  for name in "${manifests[@]}"; do
    source=$name; relative="bundles/$version/${name##*/}"
    upload_file "$id" "$origin" "$host" "$port" "$remote_root" "$source" "$relative" "$tls_insecure" "$netrc" || return 1
    [[ $dry_run == true ]] || remote_matches "$origin" "$relative" "$source" || {
      printf '[%s] public manifest verification failed\n' "$id" >&2; return 1;
    }
  done
  upload_file "$id" "$origin" "$host" "$port" "$remote_root" "$STATIC_DIR/install.sh" install.sh "$tls_insecure" "$netrc" || return 1
  [[ $dry_run == true ]] || remote_matches "$origin" install.sh "$STATIC_DIR/install.sh" || {
    printf '[%s] public installer verification failed\n' "$id" >&2; return 1;
  }
}
sync_cpanel_mirror() {
  local id=$1 origin=$2 control_panel=$3 username=$4 password=$5 remote_root=$6
  local bundle_dir="$STATIC_DIR/bundles/$version" name source relative
  local chunks inventories manifests
  shopt -s nullglob
  chunks=("$bundle_dir"/*.part-*)
  inventories=("$bundle_dir"/*.inventory.json)
  manifests=("$bundle_dir"/*.parts)
  shopt -u nullglob
  printf '[%s] syncing %s to %s\n' "$id" "$version" "$origin"
  if [[ $dry_run == false ]]; then
    cpanel_login "$id" "$control_panel" "$username" "$password" || return 1
    if [[ $check_ftp == true ]]; then
      printf '[%s] cPanel authentication succeeded\n' "$id"
      return 0
    fi
    cpanel_root="/home/$username/${remote_root#/}"
    cpanel_mkdir "$cpanel_root" bundles || return 1
    cpanel_mkdir "$cpanel_root/bundles" "$version" || return 1
  elif [[ $check_ftp == true ]]; then
    printf '[%s] --check-ftp cannot run with --dry-run\n' "$id" >&2
    return 1
  fi
  for name in "${chunks[@]}"; do
    source=$name; relative="bundles/$version/${name##*/}"
    upload_cpanel_file "$id" "$origin" "$source" "$relative" || return 1
  done
  for name in "${inventories[@]}"; do
    source=$name; relative="bundles/$version/${name##*/}"
    upload_cpanel_file "$id" "$origin" "$source" "$relative" || return 1
  done
  for name in "${manifests[@]}"; do
    source=$name; relative="bundles/$version/${name##*/}"
    upload_cpanel_file "$id" "$origin" "$source" "$relative" || return 1
    [[ $dry_run == true ]] || remote_matches "$origin" "$relative" "$source" || return 1
  done
  upload_cpanel_file "$id" "$origin" "$STATIC_DIR/install.sh" install.sh || return 1
  [[ $dry_run == true ]] || remote_matches "$origin" install.sh "$STATIC_DIR/install.sh"
}
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
