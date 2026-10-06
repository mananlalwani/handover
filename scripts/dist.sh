#!/bin/sh
# Build a user-install tarball matching make install-user.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$ROOT"

VERSION="${HANDOVER_VERSION:-}"
if test -z "$VERSION"; then
	VERSION=$(awk -F '"' '/^version = / { print $2; exit }' handoverd/Cargo.toml)
fi
HOST=$(rustc -vV | awk '/^host: / { print $2; exit }')
STAGE="handover-${VERSION}-${HOST}"
OUT_DIR="${ROOT}/dist"
STAGE_DIR="${OUT_DIR}/${STAGE}"

cargo build --release --locked -p handoverd -p handoverctl -p handover-google-messages

rm -rf "$STAGE_DIR"
mkdir -p \
	"$STAGE_DIR/bin" \
	"$STAGE_DIR/share/man/man1" \
	"$STAGE_DIR/share/man/man8" \
	"$STAGE_DIR/share/bash-completion/completions" \
	"$STAGE_DIR/share/zsh/site-functions" \
	"$STAGE_DIR/share/fish/vendor_completions.d" \
	"$STAGE_DIR/share/applications" \
	"$STAGE_DIR/share/licenses/handover/ukey2" \
	"$STAGE_DIR/share/systemd/user" \
	"$STAGE_DIR/share/handover/quickshell/pages" \
	"$STAGE_DIR/share/handover/google-messages/tools/chrome-observer"

install -Dm755 target/release/handoverd "$STAGE_DIR/bin/handoverd"
install -Dm755 target/release/handoverctl "$STAGE_DIR/bin/handoverctl"
install -Dm755 target/release/handover-google-messages-helper \
	"$STAGE_DIR/bin/handover-google-messages-helper"
install -Dm755 target/release/handover-google-messages-auth-probe \
	"$STAGE_DIR/bin/handover-google-messages-auth-probe"
install -Dm755 scripts/handover-gui "$STAGE_DIR/bin/handover-gui"
install -Dm755 scripts/handover-google-messages-setup "$STAGE_DIR/bin/handover-google-messages-setup"
install -Dm644 google-messages/tools/setup_google_messages.py \
	google-messages/tools/chromium_login_probe.py google-messages/tools/chromium_auth_capture.py \
	google-messages/tools/chromium_browsers.py "$STAGE_DIR/share/handover/google-messages/tools/"
if command -v strip >/dev/null 2>&1; then
	strip "$STAGE_DIR/bin/handoverd" "$STAGE_DIR/bin/handoverctl" \
		"$STAGE_DIR/bin/handover-google-messages-helper" \
		"$STAGE_DIR/bin/handover-google-messages-auth-probe" || true
fi

install -Dm644 docs/man/handoverctl.1 "$STAGE_DIR/share/man/man1/handoverctl.1"
install -Dm644 docs/man/handoverd.8 "$STAGE_DIR/share/man/man8/handoverd.8"
install -Dm644 completions/handoverctl.bash \
	"$STAGE_DIR/share/bash-completion/completions/handoverctl"
install -Dm644 completions/_handoverctl \
	"$STAGE_DIR/share/zsh/site-functions/_handoverctl"
install -Dm644 completions/handoverctl.fish \
	"$STAGE_DIR/share/fish/vendor_completions.d/handoverctl.fish"
install -Dm644 packaging/systemd/handoverd.local.service \
	"$STAGE_DIR/share/systemd/user/handoverd.service"
install -Dm644 packaging/handover.desktop \
	"$STAGE_DIR/share/applications/handover.desktop"
install -Dm644 quickshell/HandoverService.qml \
	"$STAGE_DIR/share/handover/quickshell/HandoverService.qml"
install -Dm644 quickshell/*.js "$STAGE_DIR/share/handover/quickshell/"
install -Dm644 quickshell/example.qml \
	"$STAGE_DIR/share/handover/quickshell/example.qml"
install -Dm644 quickshell/README.md \
	"$STAGE_DIR/share/handover/quickshell/README.md"
install -Dm644 quickshell/pages/*.qml \
	"$STAGE_DIR/share/handover/quickshell/pages/"
install -Dm755 google-messages/tools/install_native_probe.py \
	"$STAGE_DIR/share/handover/google-messages/tools/install_native_probe.py"
install -Dm644 google-messages/tools/chrome-observer/manifest.json \
	"$STAGE_DIR/share/handover/google-messages/tools/chrome-observer/manifest.json"
install -Dm644 google-messages/tools/chrome-observer/popup.html \
	"$STAGE_DIR/share/handover/google-messages/tools/chrome-observer/popup.html"
install -Dm644 google-messages/tools/chrome-observer/*.js \
	"$STAGE_DIR/share/handover/google-messages/tools/chrome-observer/"
install -Dm644 google-messages/tools/chrome-observer/privacy.mjs \
	google-messages/tools/chrome-observer/wire-shape.mjs \
	google-messages/tools/chrome-observer/README.md \
	"$STAGE_DIR/share/handover/google-messages/tools/chrome-observer/"
install -Dm755 scripts/install-from-dist.sh "$STAGE_DIR/install.sh"
install -Dm644 LICENSE "$STAGE_DIR/share/licenses/handover/LICENSE"
install -Dm644 third_party/ukey2/LICENSE \
	"$STAGE_DIR/share/licenses/handover/ukey2/LICENSE"
install -Dm644 third_party/ukey2/README.md \
	"$STAGE_DIR/share/licenses/handover/ukey2/README.md"

cat >"$STAGE_DIR/BUILD.txt" <<EOF
Handover ${VERSION}
Host triple: ${HOST}
Built with: cargo build --release --locked -p handoverd -p handoverctl -p handover-google-messages
This binary needs a glibc close to the builder's. GitHub tag builds use Ubuntu 24.04.

Install:
  tar xf ${STAGE}.tar.gz
  cd ${STAGE}
  ./install.sh
  # Optional Google Messages sign-in using an installed Chromium browser:
  ./install.sh --with-google-messages-setup

Binaries go to ~/.local/bin. Put that directory on PATH.
EOF

mkdir -p "$OUT_DIR"
tar -C "$OUT_DIR" -czf "${OUT_DIR}/${STAGE}.tar.gz" "$STAGE"
rm -rf "$STAGE_DIR"
echo "${OUT_DIR}/${STAGE}.tar.gz"
