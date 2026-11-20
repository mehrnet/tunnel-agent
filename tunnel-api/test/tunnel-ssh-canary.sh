#!/usr/bin/env bash
# Remote command arguments intentionally expand on the local machine.
# shellcheck disable=SC2029
set -euo pipefail

: "${1:?Usage: tunnel-ssh-canary.sh connector-ip listener-ip}"
: "${2:?Missing listener IPv4 address}"
[[ $1 =~ ^[0-9.]+$ && $2 =~ ^[0-9.]+$ && $1 != "$2" ]]
connector=root@$1
listener=root@$2
ssh_opts=(-o BatchMode=yes -o ConnectTimeout=8)
nonce=$(openssl rand -hex 12)
unit=wss-test-$nonce
remote=/opt/tunnel-agent/$unit
tmp=$(mktemp -d)

cleanup() {
  ssh "${ssh_opts[@]}" "$connector" "systemctl stop '$unit-client' '$unit-http' 2>/dev/null || true; if test -d '$remote'; then find '$remote' -depth -delete; fi" || true
  ssh "${ssh_opts[@]}" "$listener" "systemctl stop '$unit-server' 2>/dev/null || true; if test -d '$remote'; then find '$remote' -depth -delete; fi" || true
  find "$tmp" -depth -delete
}
trap cleanup EXIT

TUNNEL_TEST_MANIFEST_DIR="$tmp" node --test test/tunnel-owner.cjs
openssl req -new -newkey rsa:2048 -nodes -x509 -days 1 -subj /CN=canary.invalid \
  -addext subjectAltName=DNS:canary.invalid -addext basicConstraints=critical,CA:FALSE \
  -out "$tmp/cert.pem" -keyout "$tmp/key.pem" 2>/dev/null
printf '%s' "$nonce" >"$tmp/payload"
node --input-type=module - "$tmp" "$remote" "$2" <<'JS'
import { readFileSync, writeFileSync } from "node:fs";
const [, , dir, remote, listener] = process.argv;
for (const name of ["client", "server"]) {
  const path = dir + "/" + name + ".json";
  const value = JSON.parse(readFileSync(path, "utf8"));
  value.bundle.root = remote;
  const lane = value.tunnels[0];
  if (name === "server") {
    lane.tls_certificate = remote + "/cert.pem";
    lane.tls_private_key = remote + "/key.pem";
    lane.listener.bind = listener;
  } else {
    lane.endpoint.url = `wss://${listener}:31403`;
    lane.endpoint.tls_sni = "canary.invalid";
  }
  writeFileSync(path, JSON.stringify(value), { mode: 0o600 });
}
JS

for host in "$connector" "$listener"; do
  ssh "${ssh_opts[@]}" "$host" "test -z \"\$(ss -H -ltn '( sport = :31401 or sport = :31402 or sport = :31403 )')\" && install -d -m 0700 '$remote/bin' &&
    printf '%s\n' '#!/bin/sh' 'exec /opt/tunnel-agent/current/bin/wstunnel \"\$@\" >>\"\$(dirname \"\$0\")/../transport.log\" 2>&1' >'$remote/bin/wstunnel' &&
    chmod 0700 '$remote/bin/wstunnel'"
done
scp -q "${ssh_opts[@]}" "$tmp/server.json" "$tmp/cert.pem" "$tmp/key.pem" "$listener:$remote/"
scp -q "${ssh_opts[@]}" "$tmp/client.json" "$tmp/cert.pem" "$tmp/payload" "$connector:$remote/"

start_agent() {
  local host=$1 mode=$2
  ssh "${ssh_opts[@]}" "$host" "set -e
    pid=\$(systemctl show -p MainPID --value tunnel-agent)
    agent=\$(readlink -f /proc/\$pid/exe)
    \"\$agent\" validate --config '$remote/$mode.json' --state-dir '$remote/state'
    systemd-run --quiet --collect --unit='$unit-$mode' -p RuntimeMaxSec=90s \
      --setenv=SSL_CERT_FILE='$remote/cert.pem' \"\$agent\" run --config '$remote/$mode.json' --state-dir '$remote/state'"
}

ssh "${ssh_opts[@]}" "$connector" "install -d -m 0700 '$remote/www' && mv '$remote/payload' '$remote/www/' &&
  systemd-run --quiet --collect --unit='$unit-http' -p RuntimeMaxSec=90s /usr/bin/python3 -m http.server 31401 --bind 127.0.0.1 --directory '$remote/www'"
start_agent "$listener" server
start_agent "$connector" client
request="--noproxy '*' http://127.0.0.1:31402/payload"
if [ "${TUNNEL_TEST_LISTENER:-tcp}" = socks5 ]; then
  request="--noproxy '' --proxy socks5h://127.0.0.1:31402 http://127.0.0.1:31401/payload"
fi
if ssh "${ssh_opts[@]}" "$listener" "set -eu
  for attempt in \$(seq 1 12); do
    result=\$(curl --max-time 2 -fsS $request 2>/dev/null || true)
    if test \"\$result\" = '$nonce'; then exit 0; fi
    sleep 1
  done
  exit 1"; then
  printf '%s\n' 'Verified HTTP payload across both canary hosts over TLS'
else
  ssh "${ssh_opts[@]}" "$listener" "ss -H -ltn '( sport = :31402 or sport = :31403 )'; journalctl -u '$unit-server' -n 12 --no-pager; tail -n 10 '$remote/transport.log'"
  ssh "${ssh_opts[@]}" "$connector" "journalctl -u '$unit-client' -n 12 --no-pager; tail -n 10 '$remote/transport.log'"
  exit 1
fi
