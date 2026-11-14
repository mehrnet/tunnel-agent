#!/bin/sh
# Assemble one immutable release directory from statically built transport files.
set -eu

: "${BUNDLE_VERSION:?set BUNDLE_VERSION}"
: "${BUNDLE_TARGET:?set BUNDLE_TARGET}"
: "${TUNNEL_AGENT_BIN:?set TUNNEL_AGENT_BIN}"
: "${WSTUNNEL_BIN:?set WSTUNNEL_BIN}"
: "${RATHOLE_BIN:?set RATHOLE_BIN}"
: "${TRUSTTUNNEL_BIN:?set TRUSTTUNNEL_BIN}"
: "${HPNSSH_PREFIX:?set HPNSSH_PREFIX}"
: "${BUNDLE_OUTPUT_DIR:?set BUNDLE_OUTPUT_DIR}"

case "$BUNDLE_TARGET" in
  x86_64-unknown-linux-musl) architecture=amd64 ;;
  aarch64-unknown-linux-musl) architecture=arm64 ;;
  *) echo "unsupported bundle target: $BUNDLE_TARGET" >&2; exit 2 ;;
esac

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
root="$stage/tunnel-agent"
mkdir -p "$root/bin" "$BUNDLE_OUTPUT_DIR"

sha256_file() {
  sha256sum "$1" | awk '{print $1}'
}

file_bytes() {
  wc -c < "$1" | tr -d '[:space:]'
}

install -m 0755 "$TUNNEL_AGENT_BIN" "$root/bin/tunnel-agent"
install -m 0755 "$WSTUNNEL_BIN" "$root/bin/wstunnel"
install -m 0755 "$RATHOLE_BIN" "$root/bin/rathole"
install -m 0755 "$TRUSTTUNNEL_BIN" "$root/bin/trusttunnel_endpoint"
cp -a "$HPNSSH_PREFIX" "$root/hpnssh"

cat > "$root/bundle.json" <<EOF
{
  "format": 1,
  "version": "$BUNDLE_VERSION",
  "architecture": "$architecture",
  "artifacts": {
    "hpnssh": "hpnssh/bin/hpnssh",
    "hpnsshd": "hpnssh/sbin/hpnsshd",
    "wstunnel": "bin/wstunnel",
    "rathole": "bin/rathole",
    "trusttunnel": "bin/trusttunnel_endpoint"
  }
}
EOF

# Keep the update unit content-addressed. bundle-index.json intentionally does
# not index itself, avoiding a recursive digest while covering every payload
# file that a peer may later seed to another agent.
if find "$root" -type l -print -quit | grep -q .; then
  echo "bundle payload must not contain symbolic links" >&2
  exit 1
fi
file_list="$stage/files"
find "$root" -type f ! -name bundle-index.json -printf '%P\n' | LC_ALL=C sort > "$file_list"
index="$root/bundle-index.json"
{
  printf '{"format":1,"version":"%s","architecture":"%s","files":[' "$BUNDLE_VERSION" "$architecture"
  first=true
  while IFS= read -r relative; do
    case "$relative" in
      ''|/*|*..*|*[!A-Za-z0-9._/-]*)
        echo "unsafe bundle payload path: $relative" >&2
        exit 1
        ;;
    esac
    if [ "$first" = true ]; then first=false; else printf ','; fi
    path="$root/$relative"
    mode=$(stat -c '%a' "$path")
    mode_decimal=$(printf '%d' "0$mode")
    printf '{"path":"%s","sha256":"%s","bytes":%s,"mode":%s}' \
      "$relative" "$(sha256_file "$path")" "$(file_bytes "$path")" "$mode_decimal"
  done < "$file_list"
  printf ']}\n'
} > "$index"

archive="$BUNDLE_OUTPUT_DIR/tunnel-agent-${BUNDLE_VERSION}-${BUNDLE_TARGET}.tar.gz"
tar -C "$stage" -czf "$archive" tunnel-agent
sha256sum "$archive" > "$archive.sha256"
