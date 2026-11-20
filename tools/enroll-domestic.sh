#!/usr/bin/env bash
# Reads the one-time grant from stdin; the signed response leaves via loopback only.
set -euo pipefail
umask 077
port=${1:?Specify the IR loopback port of the enrollment relay}
key=${2:?Specify the root-owned private key matching the owner-pinned public key}
etc=${3:-/etc/tunnel-agent}
[[ $port =~ ^[0-9]{1,5}$ ]] && (( 10#$port >= 1024 && 10#$port <= 65535 ))
read -r grant
[[ $grant =~ ^[0-9a-f-]{36}\.[a-f0-9]{64}$ ]]
read -r node_id < <(printf '%s\n' "$(< "$etc/node-id")")
[[ $node_id =~ ^[0-9a-f-]{36}$ ]]
[[ -f $key && -O $key && $(stat -c %a "$key") == 600 ]]
tmp=$(mktemp -d)
trap 'find "$tmp" -depth -delete' EXIT
pub=$(openssl pkey -in "$key" -pubout -outform DER | openssl base64 -A)
printf '%s\n%s\n%s\n%s' tunnel-agent-domestic-enrollment-v1 "$node_id" "$grant" "$pub" >"$tmp/message"
signature=$(openssl pkeyutl -sign -rawin -inkey "$key" -in "$tmp/message" | openssl base64 -A)
printf '{"grant":"%s","public_key":"%s","signature":"%s"}' "$grant" "$pub" "$signature" |
  curl --noproxy '*' --fail-with-body --silent --show-error --max-time 15 -H 'Content-Type: application/json' --data-binary @- "http://127.0.0.1:$port/confirm"
printf '\n'
