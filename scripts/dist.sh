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

cargo build --release --locked -p handoverd -p handoverctl

rm -rf "$STAGE_DIR"
mkdir -p \
	"$STAGE_DIR/bin" \
	"$STAGE_DIR/share/man/man1" \
	"$STAGE_DIR/share/man/man8" \
	"$STAGE_DIR/share/bash-completion/completions" \
	"$STAGE_DIR/share/zsh/site-functions" \
	"$STAGE_DIR/share/fish/vendor_completions.d" \
	"$STAGE_DIR/share/systemd/user" \
	"$STAGE_DIR/share/handover/quickshell/pages"

install -Dm755 target/release/handoverd "$STAGE_DIR/bin/handoverd"
install -Dm755 target/release/handoverctl "$STAGE_DIR/bin/handoverctl"
if command -v strip >/dev/null 2>&1; then
	strip "$STAGE_DIR/bin/handoverd" "$STAGE_DIR/bin/handoverctl" || true
fi

install -Dm644 docs/man/handoverctl.1 "$STAGE_DIR/share/man/man1/handoverctl.1"
install -Dm644 docs/man/handoverd.8 "$STAGE_DIR/share/man/man8/handoverd.8"
install -Dm644 completions/handoverctl.bash \
	"$STAGE_DIR/share/bash-completion/completions/handoverctl"
install -Dm644 completions/_handoverctl \
	"$STAGE_DIR/share/zsh/site-functions/_handoverctl"
install -Dm644 completions/handoverctl.fish \
	"$STAGE_DIR/share/fish/vendor_completions.d/handoverctl.fish"
install -Dm644 packaging/systemd/handoverd.service \
	"$STAGE_DIR/share/systemd/user/handoverd.service"
install -Dm644 quickshell/HandoverService.qml \
	"$STAGE_DIR/share/handover/quickshell/HandoverService.qml"
install -Dm644 quickshell/example.qml \
	"$STAGE_DIR/share/handover/quickshell/example.qml"
install -Dm644 quickshell/README.md \
	"$STAGE_DIR/share/handover/quickshell/README.md"
install -Dm644 quickshell/pages/*.qml \
	"$STAGE_DIR/share/handover/quickshell/pages/"
install -Dm755 scripts/install-from-dist.sh "$STAGE_DIR/install.sh"

cat >"$STAGE_DIR/BUILD.txt" <<EOF
Handover ${VERSION}
Host triple: ${HOST}
Built with: cargo build --release --locked -p handoverd -p handoverctl
This binary needs a glibc close to the builder's. GitHub tag builds use Ubuntu 24.04.

Install:
  tar xf ${STAGE}.tar.gz
  cd ${STAGE}
  ./install.sh

Binaries go to ~/.local/bin. Put that directory on PATH.
EOF

mkdir -p "$OUT_DIR"
tar -C "$OUT_DIR" -czf "${OUT_DIR}/${STAGE}.tar.gz" "$STAGE"
rm -rf "$STAGE_DIR"
echo "${OUT_DIR}/${STAGE}.tar.gz"
