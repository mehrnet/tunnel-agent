#!/usr/bin/env bash
set -eu
unset NO_COLOR

: "${WSTUNNEL_BIN:?Set WSTUNNEL_BIN to the absolute path to the bundled wstunnel}"
agent=${TUNNEL_AGENT_BIN:-../target/debug/tunnel-agent}
tmp=$(mktemp -d)
server=
client=
target=
# Invoked by the EXIT trap.
# shellcheck disable=SC2329
cleanup() {
  for pid in "$client" "$server" "$target"; do
    if [ -n "$pid" ]; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  find "$tmp" -depth -delete
}
trap cleanup EXIT
if [ "${TUNNEL_TEST_LIFECYCLE:-0}" = 1 ] && [ "${TUNNEL_TEST_LISTENER:-tcp}" != tcp ]; then
  printf '%s\n' "The lifecycle fixture requires a TCP lane" >&2
  exit 2
fi
TUNNEL_TEST_MANIFEST_DIR="$tmp" node --test test/tunnel-owner.cjs
mkdir -p "$tmp/bin"
export WSTUNNEL_BIN TUNNEL_TEST_LOG="$tmp/transport.log"
cat >"$tmp/bin/wstunnel" <<'WRAPPER'
#!/usr/bin/env bash
args=("$@")
for i in "${!args[@]}"; do
  if [ "${args[i]}" = --log-lvl ]; then args[i+1]=INFO; fi
done
unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy
exec "$WSTUNNEL_BIN" "${args[@]}" >>"$TUNNEL_TEST_LOG" 2>&1
WRAPPER
chmod 700 "$tmp/bin/wstunnel"
openssl req -new -newkey rsa:2048 -nodes -x509 -days 1 -subj /CN=localhost \
  -addext subjectAltName=DNS:localhost -addext basicConstraints=critical,CA:FALSE \
  -out "$tmp/cert.pem" -keyout "$tmp/key.pem" 2>/dev/null
if [ "${TUNNEL_TEST_LIFECYCLE:-0}" = 1 ]; then
  node -e 'const {createServer}=require("node:http"); for (const [port,body] of [[31401,"primary"],[31404,"secondary"]]) createServer((req,res)=>res.end(body)).listen(port,"127.0.0.1");' &
  target=$!
  if TUNNEL_TEST_MANIFEST_DIR="$tmp" SSL_CERT_FILE="$tmp/cert.pem" cargo test --manifest-path ../Cargo.toml --locked worker_lifecycle_over_live_wstunnel -- --ignored; then
    printf '%s\n' 'Verified draft, enable, restart, destination edit, disable and removal with an unchanged management PID'
    exit 0
  fi
  if [ -f "$TUNNEL_TEST_LOG" ]; then tail -n 50 "$TUNNEL_TEST_LOG"; fi
  exit 1
fi
node --input-type=module - "$tmp" <<'JS'
import { readFileSync, writeFileSync } from "node:fs";
const dir = process.argv[2];
for (const name of ["client", "server"]) {
  const path = dir + "/" + name + ".json";
  const value = JSON.parse(readFileSync(path, "utf8"));
  value.bundle.root = dir;
  if (name === "server") {
    value.tunnels[0].tls_certificate = dir + "/cert.pem";
    value.tunnels[0].tls_private_key = dir + "/key.pem";
    value.tunnels[0].listener.bind = "127.0.0.1";
  }
  writeFileSync(path, JSON.stringify(value));
}
JS
"$agent" validate --config "$tmp/server.json" --state-dir "$tmp/server-state"
"$agent" validate --config "$tmp/client.json" --state-dir "$tmp/client-state"
node -e 'require("node:http").createServer((req,res)=>res.end("tunnel-e2e:"+req.url)).listen(31401,"127.0.0.1")' &
target=$!
"$agent" run --config "$tmp/server.json" --state-dir "$tmp/server-state" >"$tmp/server.log" 2>&1 &
server=$!
SSL_CERT_FILE="$tmp/cert.pem" "$agent" run --config "$tmp/client.json" --state-dir "$tmp/client-state" >"$tmp/client.log" 2>&1 &
client=$!
route=(--noproxy '*' http://127.0.0.1:31402/check)
if [ "${TUNNEL_TEST_LISTENER:-tcp}" = socks5 ]; then
  route=(--noproxy '' --proxy socks5h://127.0.0.1:31402 http://127.0.0.1:31401/check)
fi
for _ in {1..12}; do
  reply=$(curl -fsS --max-time 2 "${route[@]}" 2>/dev/null || true)
  if [ "$reply" = "tunnel-e2e:/check" ]; then
    printf '%s\n' "Verified HTTP payload through generated TLS reverse tunnel"
    exit 0
  fi
  sleep 1
done
sed -n '1,60p' "$tmp/server.log" "$tmp/client.log"
if [ -f "$TUNNEL_TEST_LOG" ]; then
  sed -n '1,25p' "$TUNNEL_TEST_LOG"
fi
exit 1
