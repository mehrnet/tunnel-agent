#!/bin/sh
# Immutable tunnel-agent bundle bootstrap. Tunnel lanes never download binaries.
set -eu

: "${TUNNEL_AGENT_API:=https://tunnel-agent.mehrnet.com/api}"
: "${TUNNEL_AGENT_ENROLLMENT:?set TUNNEL_AGENT_ENROLLMENT from TunnelOwnersBot}"

ROOT="${TUNNEL_AGENT_ROOT:-/opt/tunnel-agent}"
ETC_DIR="${TUNNEL_AGENT_ETC_DIR:-/etc/tunnel-agent}"
STATE_DIR="${TUNNEL_AGENT_STATE_DIR:-/opt/tunnel-agent/state}"
SERVICE_NAME="${TUNNEL_AGENT_SERVICE_NAME:-tunnel-agent}"

if [ "$(id -u)" -ne 0 ]; then
  echo "run the bootstrap as root (for example: curl ... | sudo sh)" >&2
  exit 1
fi

if ! command -v systemctl >/dev/null 2>&1; then
  echo "tunnel-agent bootstrap currently requires a systemd Linux host" >&2
  exit 1
fi

case "$(uname -m)" in
  x86_64|amd64) architecture=amd64 ;;
  aarch64|arm64) architecture=arm64 ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

case "$TUNNEL_AGENT_API" in
  https://*) ;;
  *) echo "TUNNEL_AGENT_API must use HTTPS" >&2; exit 1 ;;
esac

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

fetch_to() {
  url=$1
  destination=$2
  if command -v curl >/dev/null 2>&1; then
    curl --fail --location --silent --show-error --output "$destination" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$destination" "$url"
  else
    echo "curl or wget is required for bootstrap" >&2
    return 127
  fi
}

post_to() {
  url=$1
  payload=$2
  destination=$3
  accept=${4:-application/json}
  if command -v curl >/dev/null 2>&1; then
    curl --fail --silent --show-error --request POST \
      --header 'content-type: application/json' \
      --header "accept: $accept" \
      --data "$payload" \
      --output "$destination" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$destination" \
      --header='content-type: application/json' \
      --header="accept: $accept" \
      --post-data="$payload" "$url"
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

read_env_value() {
  key=$1
  file=$2
  sed -n "s/^${key}=\([^[:space:]]*\)$/\1/p" "$file" | sed -n '1p'
}

bootstrap_response="$tmpdir/bootstrap.env"
bootstrap_payload=$(printf '{"grant":"%s","architecture":"%s"}' "$TUNNEL_AGENT_ENROLLMENT" "$architecture")
post_to "$TUNNEL_AGENT_API/v1/bootstrap" "$bootstrap_payload" "$bootstrap_response" text/x-tunnel-agent-env

bundle_version=$(read_env_value TUNNEL_AGENT_BUNDLE_VERSION "$bootstrap_response")
bundle_sha256=$(read_env_value TUNNEL_AGENT_BUNDLE_SHA256 "$bootstrap_response")
bundle_urls=$(read_env_value TUNNEL_AGENT_BUNDLE_URLS "$bootstrap_response")

case "$bundle_version" in
  ''|*[!A-Za-z0-9._+-]*) echo "bootstrap returned invalid bundle version" >&2; exit 1 ;;
esac
if [ "${#bundle_sha256}" -ne 64 ] || ! printf '%s' "$bundle_sha256" | grep -Eq '^[0-9a-f]{64}$'; then
  echo "bootstrap returned invalid bundle SHA-256" >&2
  exit 1
fi
if [ -z "$bundle_urls" ]; then
  echo "bootstrap returned no bundle mirror" >&2
  exit 1
fi

package="$tmpdir/tunnel-agent.tar.gz"
downloaded=false
urls_file="$tmpdir/bundle-urls"
printf '%s' "$bundle_urls" | tr ',' '\n' > "$urls_file"
while IFS= read -r url; do
  [ -n "$url" ] || continue
  case "$url" in
    https://*) ;;
    *) echo "bootstrap returned an unsafe bundle URL" >&2; exit 1 ;;
  esac
  rm -f "$package"
  if ! fetch_to "$url" "$package"; then
    echo "bundle mirror failed: $url" >&2
    continue
  fi
  actual=$(sha256_file "$package")
  if [ "$actual" != "$bundle_sha256" ]; then
    echo "bundle checksum mismatch: $url" >&2
    continue
  fi
  downloaded=true
  break
done < "$urls_file"

if [ "$downloaded" != true ]; then
  echo "no bundle mirror supplied the expected archive" >&2
  exit 1
fi

stage="$tmpdir/stage"
mkdir -p "$stage"
tar -xzf "$package" -C "$stage"
bundle_dir="$stage/tunnel-agent"
if [ ! -x "$bundle_dir/bin/tunnel-agent" ] || [ ! -x "$bundle_dir/hpnssh/bin/hpnssh" ] || \
   [ ! -x "$bundle_dir/hpnssh/sbin/hpnsshd" ] || [ ! -x "$bundle_dir/bin/wstunnel" ] || \
   [ ! -x "$bundle_dir/bin/rathole" ] || [ ! -x "$bundle_dir/bin/trusttunnel_endpoint" ]; then
  echo "bundle is missing one or more required tunnel binaries" >&2
  exit 1
fi

release_dir="$ROOT/releases/$bundle_version"
install -d -m 0755 "$ROOT/releases" "$STATE_DIR" "$ETC_DIR"
if [ -e "$release_dir" ]; then
  echo "bundle version already exists: $release_dir" >&2
  exit 1
fi
mv "$bundle_dir" "$release_dir"
ln -s "$release_dir" "$ROOT/current.next"
mv -Tf "$ROOT/current.next" "$ROOT/current"

umask 077
identity="$ETC_DIR/identity_ed25519"
if [ ! -s "$identity" ]; then
  "$ROOT/current/hpnssh/bin/hpnssh-keygen" -q -t ed25519 -N '' -f "$identity"
fi
public_key=$(tr -d '\n' < "$identity.pub")
hostname=$(hostname 2>/dev/null | tr -cd 'A-Za-z0-9.-' || printf unknown)
enroll_response="$tmpdir/enroll.env"
enroll_payload=$(printf '{"grant":"%s","hostname":"%s","public_key":"%s"}' "$TUNNEL_AGENT_ENROLLMENT" "$hostname" "$public_key")
post_to "$TUNNEL_AGENT_API/v1/enroll" "$enroll_payload" "$enroll_response" text/x-tunnel-agent-env

node_id=$(read_env_value TUNNEL_AGENT_NODE_ID "$enroll_response")
node_token=$(read_env_value TUNNEL_AGENT_NODE_TOKEN "$enroll_response")
document_revision=$(read_env_value TUNNEL_AGENT_DOCUMENT_REVISION "$enroll_response")
manifest_b64=$(read_env_value TUNNEL_AGENT_MANIFEST_B64 "$enroll_response")
case "$node_id" in ????????-????-????-????-????????????) ;; *) echo "enrollment returned invalid node id" >&2; exit 1 ;; esac
case "$node_token" in "$node_id".*) ;; *) echo "enrollment returned invalid node token" >&2; exit 1 ;; esac
case "$document_revision" in ''|*[!0-9]*) echo "enrollment returned invalid document revision" >&2; exit 1 ;; esac
if [ -z "$manifest_b64" ]; then
  echo "enrollment returned no tunnel manifest" >&2
  exit 1
fi
printf '%s' "$manifest_b64" | base64 -d > "$ETC_DIR/manifest.json"
printf '%s\n' "$node_id" > "$ETC_DIR/node-id"
printf '%s\n' "$node_token" > "$ETC_DIR/node-token"
printf '%s\n' "$TUNNEL_AGENT_API" > "$ETC_DIR/api-url"
printf '%s\n' "$document_revision" > "$STATE_DIR/document-revision"
chmod 0600 "$ETC_DIR/api-url" "$ETC_DIR/node-id" "$ETC_DIR/node-token" "$ETC_DIR/manifest.json" "$identity" "$STATE_DIR/document-revision"

cat > "/etc/systemd/system/${SERVICE_NAME}.service" <<EOF
[Unit]
Description=Tunnel Agent static bundle runtime
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
ExecStartPre=${ROOT}/current/bin/tunnel-agent validate --config ${ETC_DIR}/manifest.json --state-dir ${STATE_DIR}
ExecStart=${ROOT}/current/bin/tunnel-agent agent --config ${ETC_DIR}/manifest.json --state-dir ${STATE_DIR} --api-file ${ETC_DIR}/api-url --node-id-file ${ETC_DIR}/node-id --node-token-file ${ETC_DIR}/node-token
Restart=always
RestartSec=5
PrivateTmp=true
ReadWritePaths=${ROOT} ${ETC_DIR}

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now "${SERVICE_NAME}.service"
"$ROOT/current/bin/tunnel-agent" --version
echo "installed tunnel-agent bundle ${bundle_version} for ${architecture}; node ${node_id} is enrolled"
