#!/bin/sh
set -eu

daemon=${1:?usage: $0 HANDOVERD HANDOVERCTL}
cli=${2:?usage: $0 HANDOVERD HANDOVERCTL}

if ! systemctl --user show-environment >/dev/null 2>&1; then
    printf '%s\n' 'SKIP: systemd user manager is unavailable'
    exit 0
fi
smoke_port=$((30000 + ($$ % 20000)))
while ss -H -ltn "sport = :$smoke_port" | grep -q .; do
    smoke_port=$((smoke_port + 1))
    [ "$smoke_port" -le 59999 ] || smoke_port=30000
done

unit="handoverd-smoke-$$"
root=$(mktemp -d "${TMPDIR:-/tmp}/handover-systemd-smoke.XXXXXX")
runtime="$root/runtime"
state="$root/state"
config="$root/config"
cache="$root/cache"
mkdir -p "$runtime" "$state" "$config" "$cache"
chmod 700 "$root" "$runtime" "$state" "$config" "$cache"

cleanup() {
    systemctl --user stop "$unit.service" >/dev/null 2>&1 || true
    systemctl --user reset-failed "$unit.service" >/dev/null 2>&1 || true
    rm -rf "$root"
}
trap cleanup EXIT INT TERM

daemon=$(readlink -f "$daemon")
cli=$(readlink -f "$cli")

systemd-run --user --unit="$unit" --collect --service-type=simple \
    --setenv="XDG_RUNTIME_DIR=$runtime" \
    --setenv="XDG_STATE_HOME=$state" \
    --setenv="XDG_CONFIG_HOME=$config" \
    --setenv="XDG_CACHE_HOME=$cache" \
    --setenv="HANDOVER_NATIVE_SMOKE_PORT=$smoke_port" \
    --property=NoNewPrivileges=yes \
    --property=CapabilityBoundingSet= \
    --property=AmbientCapabilities= \
    --property=ProtectSystem=full \
    --property=ProtectKernelTunables=yes \
    --property=ProtectKernelModules=yes \
    --property=ProtectControlGroups=yes \
    --property=LockPersonality=yes \
    --property=RestrictRealtime=yes \
    --property=RestrictSUIDSGID=yes \
    --property="RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK" \
    "$daemon" >/dev/null

socket="$runtime/handover/handoverd.sock"
ready=false
for _ in $(seq 1 100); do
    if [ -S "$socket" ]; then
        ready=true
        break
    fi
    if ! systemctl --user is-active --quiet "$unit.service"; then
        break
    fi
    sleep 0.1
done

if [ "$ready" != true ]; then
    journalctl --user -u "$unit.service" --no-pager -n 80 >&2 || true
    printf '%s\n' 'handoverd did not create its IPC socket' >&2
    exit 1
fi

[ "$(stat -c '%a' "$runtime/handover")" = 700 ]
[ "$(stat -c '%a' "$socket")" = 600 ]

XDG_RUNTIME_DIR="$runtime" \
XDG_STATE_HOME="$state" \
XDG_CONFIG_HOME="$config" \
XDG_CACHE_HOME="$cache" \
    "$cli" devices >/dev/null

for _ in $(seq 1 100); do
    main_pid=$(systemctl --user show "$unit.service" -p MainPID --value)
    if [ -n "$main_pid" ] && [ "$main_pid" != 0 ] &&
        ss -H -ltnp "sport = :$smoke_port" | grep -q "pid=$main_pid,"; then
        break
    fi
    sleep 0.1
done
main_pid=$(systemctl --user show "$unit.service" -p MainPID --value)
if [ -z "$main_pid" ] || [ "$main_pid" = 0 ] ||
    ! ss -H -ltnp "sport = :$smoke_port" | grep -q "pid=$main_pid,"; then
    journalctl --user -u "$unit.service" --no-pager -n 80 >&2 || true
    printf '%s\n' 'native LAN/mDNS listener did not become available' >&2
    exit 1
fi
if journalctl --user -u "$unit.service" --no-pager -n 120 2>/dev/null | \
    grep -Eq 'native backend stopped|failed to start.*discovery|discovery:'; then
    journalctl --user -u "$unit.service" --no-pager -n 120 >&2 || true
    printf '%s\n' 'native discovery reported a startup failure' >&2
    exit 1
fi

printf '%s\n' 'systemd smoke test passed: startup, IPC, LAN listener, and mDNS setup'
