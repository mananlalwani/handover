#!/bin/sh
# Install a service template with one literal executable path.
set -eu

template=${1:?usage: install-service-unit.sh TEMPLATE TARGET EXECUTABLE}
target=${2:?usage: install-service-unit.sh TEMPLATE TARGET EXECUTABLE}
executable=${3:?usage: install-service-unit.sh TEMPLATE TARGET EXECUTABLE}

case "$executable" in
    /*) ;;
    *) echo "Service executable must be an absolute path" >&2; exit 1 ;;
esac
case "$executable" in
    *[[:cntrl:]]*) echo "Service executable cannot contain control characters" >&2; exit 1 ;;
esac

# systemd quoting, specifiers, and environment expansion are separate from shell quoting.
escaped=$(printf '%s\n' "$executable" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g; s/\$/$$/g')
unit_tmp=$(mktemp "${TMPDIR:-/tmp}/handover-unit.XXXXXX")
trap 'rm "$unit_tmp"' 0
trap 'exit 1' 1 2 15
found=0
while IFS= read -r line || test -n "$line"; do
    case "$line" in
        ExecStart=*)
            # env execs the literal path without a shell. systemd restricts
            # executable-name characters more than ordinary argument characters.
            printf 'ExecStart=/usr/bin/env -- "%s"\n' "$escaped"
            found=$((found + 1))
            ;;
        *) printf '%s\n' "$line" ;;
    esac
done < "$template" > "$unit_tmp"
if test "$found" -ne 1; then
    echo "Service template must contain one ExecStart command" >&2
    exit 1
fi
install -Dm644 "$unit_tmp" "$target"
