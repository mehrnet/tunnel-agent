#!/bin/sh
# Immutable tunnel-agent bundle bootstrap. Tunnel lanes never download binaries.
set -eu

: "${TUNNEL_AGENT_API:=https://tunnel-api.mehrnet.com/api}"
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
  fetch_url=$1
  fetch_destination=$2
  if command -v curl >/dev/null 2>&1; then
    curl --fail --location --silent --show-error --output "$fetch_destination" "$fetch_url"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$fetch_destination" "$fetch_url"
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

file_bytes() {
  wc -c < "$1" | tr -d '[:space:]'
}

safe_chunk_name() {
  case "$1" in
    ''|*/*|*..*|*[!A-Za-z0-9._+-]*) return 1 ;;
    *) return 0 ;;
  esac
}

safe_https_url() {
  case "$1" in
    https://*) ;;
    *) return 1 ;;
  esac
  case "$1" in
    *[[:space:],\\]*|*'#'*) return 1 ;;
    *) return 0 ;;
  esac
}

is_chunk_manifest() {
  case "$(sed -n '1p' "$1")" in
    TUNNEL_AGENT_BUNDLE_PARTS=1|TUNNEL_AGENT_BUNDLE_PARTS=2) return 0 ;;
    *) return 1 ;;
  esac
}

download_chunked_bundle() {
  source_url=$1
  manifest=$2
  destination=$3
  expected_sha=$4
  source_base=${source_url%/*}
  manifest_format=''
  manifest_sha=''
  manifest_bytes=''
  part_count=0
  seen_names="$tmpdir/chunk-names"

  safe_https_url "$source_base" || { echo "chunked bundle has an unsafe base URL" >&2; return 1; }

  : > "$destination"
  : > "$seen_names"
  first_line=true
  while IFS= read -r line || [ -n "$line" ]; do
    if [ "$first_line" = true ]; then
      case "$line" in
        TUNNEL_AGENT_BUNDLE_PARTS=1) manifest_format=1 ;;
        TUNNEL_AGENT_BUNDLE_PARTS=2) manifest_format=2 ;;
        *) return 1 ;;
      esac
      first_line=false
      continue
    fi
    case "$line" in
      ARCHIVE_SHA256=*)
        [ -z "$manifest_sha" ] || { echo "duplicate archive SHA in chunk manifest" >&2; return 1; }
        manifest_sha=${line#ARCHIVE_SHA256=}
        ;;
      ARCHIVE_BYTES=*)
        [ -z "$manifest_bytes" ] || { echo "duplicate archive size in chunk manifest" >&2; return 1; }
        manifest_bytes=${line#ARCHIVE_BYTES=}
        ;;
      PART\ *)
        fields=${line#PART }
        part_sha=${fields%% *}
        [ "$part_sha" != "$fields" ] || { echo "invalid chunk manifest part" >&2; return 1; }
        fields=${fields#* }
        part_bytes=${fields%% *}
        [ "$part_bytes" != "$fields" ] || { echo "invalid chunk manifest part" >&2; return 1; }
        part_source=${fields#* }
        case "$part_source" in *[[:space:]]*) echo "invalid chunk source" >&2; return 1;; esac
        if [ "${#part_sha}" -ne 64 ] || ! printf '%s' "$part_sha" | grep -Eq '^[0-9a-f]{64}$' || \
           { [ "$manifest_format" = 1 ] && ! safe_chunk_name "$part_source"; } || \
           { [ "$manifest_format" = 2 ] && ! safe_https_url "$part_source"; }; then
          echo "invalid chunk manifest part" >&2
          return 1
        fi
        case "$part_bytes" in ''|*[!0-9]*) echo "invalid chunk size" >&2; return 1;; esac
        if grep -F -x -- "$part_source" "$seen_names" >/dev/null 2>&1; then
          echo "duplicate chunk source in manifest" >&2
          return 1
        fi
        printf '%s\n' "$part_source" >> "$seen_names"
        if [ "$manifest_format" = 1 ]; then
          part_url="$source_base/$part_source"
        else
          part_url="$part_source"
        fi
        part_file="$tmpdir/chunk-$part_count"
        if ! fetch_to "$part_url" "$part_file"; then
          echo "chunk download failed: $part_source" >&2
          return 1
        fi
        if [ "$(file_bytes "$part_file")" != "$part_bytes" ] || [ "$(sha256_file "$part_file")" != "$part_sha" ]; then
          echo "chunk validation failed: $part_source" >&2
          return 1
        fi
        cat "$part_file" >> "$destination"
        part_count=$((part_count + 1))
        ;;
      *)
        echo "invalid chunk manifest record" >&2
        return 1
        ;;
    esac
  done < "$manifest"

  if [ "$part_count" -eq 0 ] || [ "$manifest_sha" != "$expected_sha" ] || \
     [ "${#manifest_sha}" -ne 64 ] || ! printf '%s' "$manifest_sha" | grep -Eq '^[0-9a-f]{64}$'; then
    echo "chunk manifest does not match the expected archive" >&2
    return 1
  fi
  case "$manifest_bytes" in ''|*[!0-9]*) echo "invalid archive size in chunk manifest" >&2; return 1;; esac
  if [ "$(file_bytes "$destination")" != "$manifest_bytes" ] || [ "$(sha256_file "$destination")" != "$expected_sha" ]; then
    echo "reconstructed bundle validation failed" >&2
    return 1
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
printf '%s\n' "$bundle_urls" | tr ',' '\n' > "$urls_file"
while IFS= read -r url; do
  [ -n "$url" ] || continue
  case "$url" in
    https://*) ;;
    *) echo "bootstrap returned an unsafe bundle URL" >&2; exit 1 ;;
  esac
  source_file="$tmpdir/bundle-source"
  rm -f "$package" "$source_file"
  if ! fetch_to "$url" "$source_file"; then
    echo "bundle mirror failed: $url" >&2
    continue
  fi
  if is_chunk_manifest "$source_file"; then
    if ! download_chunked_bundle "$url" "$source_file" "$package" "$bundle_sha256"; then
      echo "chunked bundle mirror failed: $url" >&2
      continue
    fi
  else
    mv "$source_file" "$package"
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
