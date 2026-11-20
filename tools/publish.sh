#!/bin/sh
# Publish checksum-verified release archives as Cloudflare-sized static chunks.
set -eu

: "${BUNDLE_VERSION:?set BUNDLE_VERSION, for example v0.4.0}"
: "${BUNDLE_SOURCE_DIR:?set BUNDLE_SOURCE_DIR to the GitHub release download directory}"
bundle_activate=${BUNDLE_ACTIVATE:-false}
track=${BUNDLE_TRACK:-testing}
case "$track" in
  testing|stable) ;;
  *) echo "BUNDLE_TRACK must be testing or stable" >&2; exit 2 ;;
esac
case "$bundle_activate" in
  true|false) ;;
  *) echo "BUNDLE_ACTIVATE must be true or false" >&2; exit 2 ;;
esac
if [ "$bundle_activate" = true ]; then
  : "${BUNDLE_PUBLISH_TOKEN:?set BUNDLE_PUBLISH_TOKEN to the Worker activation secret}"
fi

case "$BUNDLE_VERSION" in
  ''|*[!0-9A-Za-z._+-]*)
    echo "invalid BUNDLE_VERSION" >&2
    exit 2
    ;;
esac

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repository_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
api_dir="$repository_dir/tunnel-api"
files_dir="$repository_dir/tunnel-files/static"
files_config="$repository_dir/tunnel-files/wrangler.jsonc"
site_bundle_dir="$files_dir/bundles/$BUNDLE_VERSION"
domestic_bases=${DOMESTIC_BUNDLE_BASE_URL:-}
domestic_extra_bases=${DOMESTIC_BUNDLE_EXTRA_BASE_URLS:-}
domestic_format=${DOMESTIC_BUNDLE_FORMAT:-manifest}
domestic_amd64_url=${DOMESTIC_BUNDLE_URL_AMD64:-}
domestic_arm64_url=${DOMESTIC_BUNDLE_URL_ARM64:-}
api_origin=${TUNNEL_API_ORIGIN:-https://tunnel-api.mehrnet.com}
chunk_bytes=${BUNDLE_CHUNK_BYTES:-20971520}

if [ -n "$domestic_extra_bases" ]; then
  domestic_bases=${domestic_bases:+"$domestic_bases,"}$domestic_extra_bases
fi

case "$chunk_bytes" in
  ''|*[!0-9]*)
    echo "BUNDLE_CHUNK_BYTES must be a positive integer" >&2
    exit 2
    ;;
esac

# Cloudflare Workers static assets are limited to 25 MiB. Leave margin for
# platform changes and ensure a bundle chunk can always be a static object.
if [ "$chunk_bytes" -le 0 ] || [ "$chunk_bytes" -gt 25165824 ]; then
  echo "BUNDLE_CHUNK_BYTES must be between 1 and 25165824 bytes" >&2
  exit 2
fi

if [ -n "$domestic_bases" ] && { [ -n "$domestic_amd64_url" ] || [ -n "$domestic_arm64_url" ]; }; then
  echo "set either DOMESTIC_BUNDLE_BASE_URL or architecture-specific DOMESTIC_BUNDLE_URL_* values" >&2
  exit 2
fi

valid_domestic_url() {
  case "$1" in https://*|http://dl.mehr-mir.ir|http://dl.mehr-mir.ir/*) ;; *) return 1;; esac
  ! printf '%s' "$1" | grep -Eq '[[:space:]"\\,?#]'
}

if [ -n "$domestic_bases" ]; then
  case "$domestic_bases" in ,*|*,,*|*,)
    echo "DOMESTIC_BUNDLE_BASE_URLS contains an empty mirror" >&2
    exit 2
    ;;
  esac
  old_ifs=$IFS
  IFS=,
  # shellcheck disable=SC2086 # Intentional comma-delimited split after validation.
  set -- $domestic_bases
  IFS=$old_ifs
  if [ "$#" -gt 4 ]; then
    echo "at most four domestic bundle mirrors are supported" >&2
    exit 2
  fi
  for domestic_base in "$@"; do
    if ! valid_domestic_url "$domestic_base"; then
      echo "domestic bundle mirror must use HTTPS or the approved HTTP mirror" >&2
      exit 2
    fi
  done
  if [ -n "$domestic_bases" ] && [ -z "$domestic_format" ]; then
    echo "DOMESTIC_BUNDLE_FORMAT is required when domestic mirrors are set" >&2
    exit 2
  fi
  case "$domestic_format" in
    manifest|archive) ;;
    *)
      echo "DOMESTIC_BUNDLE_FORMAT must be manifest or archive" >&2
      exit 2
      ;;
  esac
fi

if [ -n "$domestic_amd64_url" ] || [ -n "$domestic_arm64_url" ]; then
  if [ -z "$domestic_amd64_url" ] || [ -z "$domestic_arm64_url" ]; then
    echo "both DOMESTIC_BUNDLE_URL_AMD64 and DOMESTIC_BUNDLE_URL_ARM64 are required" >&2
    exit 2
  fi
  for domestic_url in "$domestic_amd64_url" "$domestic_arm64_url"; do
    if ! valid_domestic_url "$domestic_url"; then
      echo "domestic bundle URL contains unsafe characters" >&2
      exit 2
    fi
  done
fi

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    echo "sha256sum or shasum is required" >&2
    exit 127
  fi
}

file_bytes() {
  wc -c < "$1" | tr -d '[:space:]'
}

domestic_urls_json() {
  architecture=$1
  archive=$2
  urls=''
  if [ -n "$domestic_amd64_url" ]; then
    case "$architecture" in
      amd64) urls=$domestic_amd64_url ;;
      arm64) urls=$domestic_arm64_url ;;
    esac
  elif [ -n "$domestic_bases" ]; then
    old_ifs=$IFS
    IFS=,
    # shellcheck disable=SC2086 # Intentional comma-delimited split after validation.
    set -- $domestic_bases
    IFS=$old_ifs
    for domestic_base in "$@"; do
      case "$domestic_format" in
        manifest) domestic_url="${domestic_base%/}/bundles/$BUNDLE_VERSION/$archive.parts" ;;
        archive) domestic_url="${domestic_base%/}/bundles/$BUNDLE_VERSION/$archive" ;;
      esac
      urls=${urls:+"$urls,"}$domestic_url
    done
  fi

  json=''
  old_ifs=$IFS
  IFS=,
  # shellcheck disable=SC2086 # URLs were constructed from validated mirrors above.
  set -- $urls
  IFS=$old_ifs
  for domestic_url in "$@"; do
    [ -n "$domestic_url" ] || continue
    if [ -n "$json" ]; then json="$json,"; fi
    json="$json\"$domestic_url\""
  done
  printf '[%s]' "$json"
}

stage_bundle() {
  architecture=$1
  target=$2
  archive="tunnel-agent-$BUNDLE_VERSION-$target.tar.gz"
  source_archive="$BUNDLE_SOURCE_DIR/$archive"
  source_checksum="$source_archive.sha256"

  if [ ! -f "$source_archive" ] || [ ! -f "$source_checksum" ]; then
    echo "missing release archive or checksum: $archive" >&2
    exit 1
  fi

  expected=$(awk 'NR == 1 { print $1 }' "$source_checksum")
  actual=$(sha256_file "$source_archive")
  if [ "${#expected}" -ne 64 ] || ! printf '%s' "$expected" | grep -Eq '^[0-9a-f]{64}$' || [ "$actual" != "$expected" ]; then
    echo "checksum verification failed: $archive" >&2
    exit 1
  fi

  archive_bytes=$(file_bytes "$source_archive")
  manifest="$site_bundle_dir/$archive.parts"
  inventory="$site_bundle_dir/$archive.inventory.json"
  manifest_tmp=$(mktemp)
  inventory_tmp=$(mktemp)
  parts_tmp=$(mktemp -d)

  if ! tar -xOf "$source_archive" tunnel-agent/bundle-index.json > "$inventory_tmp"; then
    echo "release archive is missing bundle-index.json: $archive" >&2
    exit 1
  fi
  inventory_sha=$(sha256_file "$inventory_tmp")
  install -D -m 0644 "$inventory_tmp" "$inventory"
  rm -f "$inventory_tmp"

  # Do not publish the archive itself: an ARM64 release is larger than the
  # Cloudflare static-object limit. The installer verifies every chunk and
  # the reconstructed archive against the SHA registered in D1.
  split -b "$chunk_bytes" -d -a 4 "$source_archive" "$parts_tmp/part-"
  {
    printf '%s\n' 'TUNNEL_AGENT_BUNDLE_PARTS=1'
    printf 'ARCHIVE_SHA256=%s\n' "$actual"
    printf 'ARCHIVE_BYTES=%s\n' "$archive_bytes"
    for part in "$parts_tmp"/part-*; do
      part_name="${archive}.part-${part##*-}"
      part_sha=$(sha256_file "$part")
      part_bytes=$(file_bytes "$part")
      install -D -m 0644 "$part" "$site_bundle_dir/$part_name"
      printf 'PART %s %s %s\n' "$part_sha" "$part_bytes" "$part_name"
    done
  } > "$manifest_tmp"
  install -D -m 0644 "$manifest_tmp" "$manifest"
  rm -rf "$parts_tmp"
  rm -f "$manifest_tmp"

  public_url="/bundles/$BUNDLE_VERSION/$archive.parts"
  domestic_json=$(domestic_urls_json "$architecture" "$archive")

  if [ "$bundle_count" -gt 0 ]; then
    printf ',\n' >> "$payload_file"
  fi
  printf '    {"architecture":"%s","sha256":"%s","inventory_sha256":"%s","public_url":"%s","domestic_urls":%s}' \
    "$architecture" "$actual" "$inventory_sha" "$public_url" "$domestic_json" >> "$payload_file"
  bundle_count=$((bundle_count + 1))
}

payload_file=$(mktemp)
trap 'rm -f "$payload_file"' EXIT HUP INT TERM
mkdir -p "$site_bundle_dir"

printf '{"version":"%s","track":"%s","bundles":[\n' "$BUNDLE_VERSION" "$track" > "$payload_file"
bundle_count=0
stage_bundle amd64 x86_64-unknown-linux-musl
stage_bundle arm64 aarch64-unknown-linux-musl
printf '\n]}\n' >> "$payload_file"

# Static files must exist before their D1 records can be returned to a node.
(
  cd "$api_dir"
  npm exec -- wrangler deploy --config "$files_config" >&2
)

if [ "$bundle_activate" = false ]; then
  cat "$payload_file"
  echo "published static assets for $BUNDLE_VERSION; activation was intentionally skipped" >&2
  exit 0
fi

curl --fail-with-body --silent --show-error \
  --header "authorization: Bearer $BUNDLE_PUBLISH_TOKEN" \
  --header "content-type: application/json" \
  --data-binary "@$payload_file" \
  "${api_origin%/}/api/v1/admin/bundles/activate"
echo "published and activated $BUNDLE_VERSION"
