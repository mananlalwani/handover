#!/bin/sh
# Install a packed Handover prefix (bin/, share/) into the user profile.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
PREFIX="${PREFIX:-$HOME/.local}"
USER_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"

install -Dm755 "$ROOT/bin/handoverd" "$PREFIX/bin/handoverd"
install -Dm755 "$ROOT/bin/handoverctl" "$PREFIX/bin/handoverctl"
install -Dm755 "$ROOT/bin/handover-gui" "$PREFIX/bin/handover-gui"
printf '%s\n' "$USER_DATA_HOME" > "$PREFIX/bin/handover-gui.data-home"
install -Dm644 "$ROOT/share/applications/handover.desktop" \
	"$USER_DATA_HOME/applications/handover.desktop"
install -Dm644 "$ROOT/share/man/man1/handoverctl.1" "$PREFIX/share/man/man1/handoverctl.1"
install -Dm644 "$ROOT/share/man/man8/handoverd.8" "$PREFIX/share/man/man8/handoverd.8"
install -Dm644 "$ROOT/share/bash-completion/completions/handoverctl" \
	"$USER_DATA_HOME/bash-completion/completions/handoverctl"
install -Dm644 "$ROOT/share/zsh/site-functions/_handoverctl" \
	"$USER_DATA_HOME/zsh/site-functions/_handoverctl"
install -Dm644 "$ROOT/share/fish/vendor_completions.d/handoverctl.fish" \
	"$USER_DATA_HOME/fish/vendor_completions.d/handoverctl.fish"
install -Dm644 "$ROOT/share/systemd/user/handoverd.service" \
	"$USER_DATA_HOME/systemd/user/handoverd.service"
install -d "$USER_DATA_HOME/handover/quickshell/pages"
install -Dm644 "$ROOT/share/handover/quickshell/HandoverService.qml" \
	"$USER_DATA_HOME/handover/quickshell/HandoverService.qml"
install -Dm644 "$ROOT/share/handover/quickshell/example.qml" \
	"$USER_DATA_HOME/handover/quickshell/example.qml"
install -Dm644 "$ROOT/share/handover/quickshell/README.md" \
	"$USER_DATA_HOME/handover/quickshell/README.md"
install -Dm644 "$ROOT/share/handover/quickshell/pages/"*.qml \
	"$USER_DATA_HOME/handover/quickshell/pages/"

systemctl --user daemon-reload
systemctl --user reenable handoverd.service
systemctl --user restart handoverd.service

attempt=1
while test "$attempt" -le 20; do
	if test -S "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/handover/handoverd.sock"; then
		exit 0
	fi
	sleep 0.1
	attempt=$((attempt + 1))
done

echo "handoverd did not create its socket; check: systemctl --user status handoverd" >&2
exit 1
