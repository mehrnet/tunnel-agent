#!/bin/sh
# Publish two checksum-verified release archives before activating them in D1.
set -eu

: "${BUNDLE_VERSION:?set BUNDLE_VERSION, for example v0.4.0}"
: "${BUNDLE_SOURCE_DIR:?set BUNDLE_SOURCE_DIR to the GitHub release download directory}"

case "$BUNDLE_VERSION" in
  ''|*[!0-9A-Za-z._+-]*)
    echo "invalid BUNDLE_VERSION" >&2
    exit 2
    ;;
esac

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
distribution_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
site_bundle_dir="$distribution_dir/public/bundles/$BUNDLE_VERSION"
domestic_base=${DOMESTIC_BUNDLE_BASE_URL:-}

if [ -n "$domestic_base" ]; then
  case "$domestic_base" in
    https://*) ;;
    *)
      echo "DOMESTIC_BUNDLE_BASE_URL must be an HTTPS base URL" >&2
      exit 2
      ;;
  esac
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

sql_literal() {
  printf '%s' "$1" | sed "s/'/''/g"
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

  install -D -m 0644 "$source_archive" "$site_bundle_dir/$archive"
  install -D -m 0644 "$source_checksum" "$site_bundle_dir/$archive.sha256"

  public_url="/bundles/$BUNDLE_VERSION/$archive"
  domestic_sql=NULL
  if [ -n "$domestic_base" ]; then
    domestic_url="${domestic_base%/}/$archive"
    domestic_sql="'$(sql_literal "$domestic_url")'"
  fi
  public_sql="'$(sql_literal "$public_url")'"

  cat >> "$sql_file" <<EOF
UPDATE bundle_releases SET active = 0 WHERE architecture = '$architecture';
INSERT INTO bundle_releases
  (version, architecture, sha256, public_url, domestic_url, active, created_at)
VALUES
  ('$BUNDLE_VERSION', '$architecture', '$actual', $public_sql, $domestic_sql, 1, unixepoch())
ON CONFLICT(version, architecture) DO UPDATE SET
  sha256 = excluded.sha256,
  public_url = excluded.public_url,
  domestic_url = excluded.domestic_url,
  active = 1,
  created_at = excluded.created_at;
EOF
}

sql_file=$(mktemp)
trap 'rm -f "$sql_file"' EXIT HUP INT TERM
printf 'BEGIN;\n' > "$sql_file"
mkdir -p "$site_bundle_dir"

stage_bundle amd64 x86_64-unknown-linux-musl
stage_bundle arm64 aarch64-unknown-linux-musl
printf 'COMMIT;\n' >> "$sql_file"

# Static files must exist before their D1 records can be returned to a node.
(
  cd "$distribution_dir"
  npm run deploy
  npx wrangler d1 execute tunnel-agent --remote --file "$sql_file"
)

echo "published and activated $BUNDLE_VERSION"
