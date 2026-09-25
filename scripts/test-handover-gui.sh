#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT HUP INT TERM

BIN_DIR="$TMP_DIR/prefix/bin"
DATA_HOME="$TMP_DIR/custom-data"
EMPTY_DATA_HOME="$TMP_DIR/empty-data"
mkdir -p "$BIN_DIR" "$DATA_HOME/handover/quickshell" "$EMPTY_DATA_HOME" "$TMP_DIR/fake-bin"
cp "$ROOT/scripts/handover-gui" "$BIN_DIR/handover-gui"
printf '%s\n' "$DATA_HOME" > "$BIN_DIR/handover-gui.data-home"
touch "$DATA_HOME/handover/quickshell/example.qml"

cat > "$TMP_DIR/fake-bin/quickshell" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$HANDOVER_GUI_ARGS"
EOF
chmod +x "$TMP_DIR/fake-bin/quickshell"

HANDOVER_GUI_ARGS="$TMP_DIR/args" \
XDG_DATA_HOME="$EMPTY_DATA_HOME" \
PATH="$TMP_DIR/fake-bin:$PATH" \
"$BIN_DIR/handover-gui" --example argument

expected="$TMP_DIR/expected"
printf '%s\n' --path "$DATA_HOME/handover/quickshell/example.qml" --example argument > "$expected"
if ! cmp -s "$expected" "$TMP_DIR/args"; then
	echo "handover-gui did not use its configured data home or forward arguments" >&2
	diff -u "$expected" "$TMP_DIR/args" >&2 || true
	exit 1
fi
