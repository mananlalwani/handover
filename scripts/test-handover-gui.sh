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
touch "$DATA_HOME/handover/quickshell/example.qml" "$DATA_HOME/handover/quickshell/send.qml"

cat > "$TMP_DIR/fake-bin/quickshell" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$HANDOVER_GUI_ARGS"
printf '%s' "${HANDOVER_SHARE_RESOURCE-<unset>}" > "$HANDOVER_GUI_RESOURCE"
EOF
chmod +x "$TMP_DIR/fake-bin/quickshell"

run_gui() {
	HANDOVER_GUI_ARGS="$TMP_DIR/args" \
	HANDOVER_GUI_RESOURCE="$TMP_DIR/resource" \
	XDG_DATA_HOME="$EMPTY_DATA_HOME" \
	PATH="$TMP_DIR/fake-bin:$PATH" \
	"$BIN_DIR/handover-gui" "$@"
}

run_gui --example argument
expected="$TMP_DIR/expected"
printf '%s\n' --path "$DATA_HOME/handover/quickshell/example.qml" --example argument > "$expected"
if ! cmp -s "$expected" "$TMP_DIR/args"; then
	echo "handover-gui did not use its configured data home or forward arguments" >&2
	diff -u "$expected" "$TMP_DIR/args" >&2 || true
	exit 1
fi
if ! test "$(cat "$TMP_DIR/resource")" = '<unset>'; then
	echo "normal launch unexpectedly set HANDOVER_SHARE_RESOURCE" >&2
	exit 1
fi

run_gui --send
printf '%s\n' --path "$DATA_HOME/handover/quickshell/send.qml" > "$expected"
if ! cmp -s "$expected" "$TMP_DIR/args" || test -s "$TMP_DIR/resource"; then
	echo "--send without a payload did not choose send.qml with an empty resource" >&2
	exit 1
fi

hostile_path='weird $(touch SHOULD_NOT_EXIST); * file.txt'
run_gui --send "$hostile_path"
printf '%s\n' --path "$DATA_HOME/handover/quickshell/send.qml" > "$expected"
if ! cmp -s "$expected" "$TMP_DIR/args"; then
	echo "--send passed launcher arguments to Quickshell" >&2
	exit 1
fi
if test "$(cat "$TMP_DIR/resource")" != "$PWD/$hostile_path"; then
	echo "--send did not preserve and absolutize a relative resource" >&2
	exit 1
fi
if test -e SHOULD_NOT_EXIST; then
	echo "--send evaluated shell syntax in a resource path" >&2
	exit 1
fi

for resource in 'https://example.test/a?b=c' 'HTTP://example.test/a' 'FILE:///tmp/a%20b'; do
	run_gui --send "$resource"
	printf '%s' "$resource" > "$expected"
	if ! cmp -s "$expected" "$TMP_DIR/resource"; then
		echo "--send changed a URL resource" >&2
		exit 1
	fi
done

if run_gui --send one two >"$TMP_DIR/stdout" 2>"$TMP_DIR/stderr"; then
	echo "--send accepted multiple resources" >&2
	exit 1
fi
if ! grep -Fq 'at most one file or URL' "$TMP_DIR/stderr"; then
	echo "--send did not explain its multiple-resource error" >&2
	exit 1
fi

mkdir -p "$EMPTY_DATA_HOME/handover/quickshell"
touch "$EMPTY_DATA_HOME/handover/quickshell/example.qml" "$EMPTY_DATA_HOME/handover/quickshell/send.qml"
run_gui --send 'file:///tmp/a%20b.txt'
printf '%s\n' --path "$EMPTY_DATA_HOME/handover/quickshell/send.qml" > "$expected"
if ! cmp -s "$expected" "$TMP_DIR/args" || test "$(cat "$TMP_DIR/resource")" != 'file:///tmp/a%20b.txt'; then
	echo "--send did not prefer the installed XDG data home or preserve a file URL" >&2
	exit 1
fi

run_gui
printf '%s\n' --path "$EMPTY_DATA_HOME/handover/quickshell/example.qml" > "$expected"
if ! cmp -s "$expected" "$TMP_DIR/args"; then
	echo "normal launch did not prefer XDG_DATA_HOME" >&2
	exit 1
fi

echo "handover-gui launcher tests passed"
