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

archive="$BUNDLE_OUTPUT_DIR/tunnel-agent-${BUNDLE_VERSION}-${BUNDLE_TARGET}.tar.gz"
tar -C "$stage" -czf "$archive" tunnel-agent
sha256sum "$archive" > "$archive.sha256"
