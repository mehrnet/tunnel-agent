#!/bin/sh
# Bootstrap only. The daemon itself does not use curl, wget, or this script.
set -eu

: "${TUNNEL_AGENT_URLS:?set an ordered whitespace-separated list of HTTPS package URLs}"
: "${TUNNEL_AGENT_SHA256:?set the package SHA-256 from trusted desired state}"

TUNNEL_AGENT_DEST="${TUNNEL_AGENT_DEST:-/usr/local/bin/tunnel-agent}"
TUNNEL_AGENT_PACKAGE="${TUNNEL_AGENT_PACKAGE:-tar.gz}"
TUNNEL_AGENT_ARCHIVE_ENTRY="${TUNNEL_AGENT_ARCHIVE_ENTRY:-tunnel-agent}"

case "$TUNNEL_AGENT_PACKAGE" in
  binary|tar.gz) ;;
  *) echo "unsupported TUNNEL_AGENT_PACKAGE: $TUNNEL_AGENT_PACKAGE" >&2; exit 2 ;;
esac

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM
package="$tmpdir/package"
binary="$tmpdir/tunnel-agent"

download() {
  url=$1
  if command -v curl >/dev/null 2>&1; then
    curl --fail --location --silent --show-error --output "$package" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$package" "$url"
  else
    echo "curl or wget is required for bootstrap" >&2
    return 127
  fi
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    echo "sha256sum or shasum is required for bootstrap" >&2
    return 127
  fi
}

ok=false
for url in $TUNNEL_AGENT_URLS; do
  rm -f "$package"
  if ! download "$url"; then
    echo "mirror failed: $url" >&2
    continue
  fi
  actual=$(sha256_file "$package")
  if [ "$actual" != "$TUNNEL_AGENT_SHA256" ]; then
    echo "mirror checksum mismatch: $url" >&2
    continue
  fi
  ok=true
  break
done

if [ "$ok" != true ]; then
  echo "no mirror supplied the expected package" >&2
  exit 1
fi

case "$TUNNEL_AGENT_PACKAGE" in
  binary) cp "$package" "$binary" ;;
  tar.gz) tar -xOzf "$package" "$TUNNEL_AGENT_ARCHIVE_ENTRY" > "$binary" ;;
esac

chmod 0755 "$binary"
install -D -m 0755 "$binary" "$TUNNEL_AGENT_DEST"
"$TUNNEL_AGENT_DEST" --version
