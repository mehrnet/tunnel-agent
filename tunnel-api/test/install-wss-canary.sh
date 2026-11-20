#!/usr/bin/env bash
# Remote command arguments intentionally expand on the local machine.
# shellcheck disable=SC2029
set -euo pipefail

: "${1:?Usage: install-wss-canary.sh root@host /absolute/path/to/canary-binary}"
: "${2:?Missing canary binary}"
host=$1
binary=$2
sha=$(sha256sum "$binary")
sha=${sha%% *}
ssh_opts=(-o BatchMode=yes -o ConnectTimeout=8)

stage=$(ssh "${ssh_opts[@]}" "$host" 'mktemp /opt/tunnel-agent/.wss-canary.XXXXXXXX')
scp -q "${ssh_opts[@]}" "$binary" "$host:$stage"
ssh "${ssh_opts[@]}" "$host" "bash -s -- '$stage' '$sha'" <<'REMOTE'
set -euo pipefail
stage=$1
sha=$2
service=tunnel-agent.service
drop=/etc/systemd/system/$service.d/90-wss-canary.conf
original=/opt/tunnel-agent/current/bin/tunnel-agent
patch=/opt/tunnel-agent/canary/tunnel-agent-$sha
config=/etc/tunnel-agent/manifest.json
state=/opt/tunnel-agent/state
changed=0
finish() {
    local status=$?
    if (( status != 0 && changed )); then
        unlink "$drop"
        systemctl daemon-reload
        systemctl restart "$service"
    fi
    unlink "$stage"
    return "$status"
}
trap finish EXIT

printf '%s  %s\n' "$sha" "$stage" | sha256sum -c -
test "$(uname -m)" = x86_64
test -z "$(systemctl show -p DropInPaths --value "$service")"
systemctl is-active --quiet "$service"
fragment=$(systemctl show -p FragmentPath --value "$service")
test "$(grep -c "^ExecStartPre=$original " "$fragment")" -eq 1
test "$(grep -c "^ExecStart=$original " "$fragment")" -eq 1
install -d -m 0755 "$(dirname "$patch")" "$(dirname "$drop")"
install -m 0755 "$stage" "$patch"
"$patch" --version
"$patch" validate --config "$config" --state-dir "$state"
{
    printf '%s\n' '[Service]' 'ExecStartPre=' 'ExecStart='
    sed -n -e "s|^ExecStartPre=$original |ExecStartPre=$patch |p" \
           -e "s|^ExecStart=$original |ExecStart=$patch |p" "$fragment"
} >"$drop"
changed=1
systemctl daemon-reload
systemctl restart "$service"
sleep 8
systemctl is-active --quiet "$service"
pid=$(systemctl show -p MainPID --value "$service")
test "$(readlink "/proc/$pid/exe")" = "$patch"
test "$(systemctl show -p NRestarts --value "$service")" -eq 0
printf '%s: patched service active, pid=%s\n' "$(hostname)" "$pid"
REMOTE
