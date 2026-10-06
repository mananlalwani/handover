PREFIX := $(HOME)/.local
DESTDIR :=
USER_DATA_HOME := $(or $(XDG_DATA_HOME),$(HOME)/.local/share)
USER_UNIT_DIR := $(DESTDIR)$(USER_DATA_HOME)/systemd/user
UNIT_SRC := packaging/systemd/handoverd.local.service
QUICKSHELL_INSTALL_DIR := $(DESTDIR)$(USER_DATA_HOME)/handover/quickshell
GOOGLE_MESSAGES_TOOLS_DIR := $(DESTDIR)$(USER_DATA_HOME)/handover/google-messages/tools
MAN_DIR := $(DESTDIR)$(PREFIX)/share/man/man1
MAN8_DIR := $(DESTDIR)$(PREFIX)/share/man/man8
LICENSE_DIR := $(DESTDIR)$(PREFIX)/share/licenses/handover
BASH_COMPLETION_DIR := $(DESTDIR)$(USER_DATA_HOME)/bash-completion/completions
ZSH_COMPLETION_DIR := $(DESTDIR)$(USER_DATA_HOME)/zsh/site-functions
FISH_COMPLETION_DIR := $(DESTDIR)$(USER_DATA_HOME)/fish/vendor_completions.d
APPLICATIONS_DIR := $(DESTDIR)$(USER_DATA_HOME)/applications
BIN_DIR := $(DESTDIR)$(PREFIX)/bin

.PHONY: install-user uninstall-user systemd-smoke dist install-files

systemd-smoke: target/debug/handoverd target/debug/handoverctl
	./scripts/systemd-smoke.sh target/debug/handoverd target/debug/handoverctl

dist:
	./scripts/dist.sh

install-files:
	install -Dm755 target/release/handoverd $(BIN_DIR)/handoverd
	install -Dm755 target/release/handoverctl $(BIN_DIR)/handoverctl
	install -Dm755 target/release/handover-google-messages-helper $(BIN_DIR)/handover-google-messages-helper
	install -Dm755 target/release/handover-google-messages-auth-probe $(BIN_DIR)/handover-google-messages-auth-probe
	install -Dm755 scripts/handover-gui $(BIN_DIR)/handover-gui
	printf '%s\n' '$(USER_DATA_HOME)' > $(BIN_DIR)/handover-gui.data-home
	install -Dm644 packaging/handover.desktop $(APPLICATIONS_DIR)/handover.desktop
	install -Dm644 docs/man/handoverctl.1 $(MAN_DIR)/handoverctl.1
	install -Dm644 docs/man/handoverd.8 $(MAN8_DIR)/handoverd.8
	install -Dm644 completions/handoverctl.bash $(BASH_COMPLETION_DIR)/handoverctl
	install -Dm644 completions/_handoverctl $(ZSH_COMPLETION_DIR)/_handoverctl
	install -Dm644 completions/handoverctl.fish $(FISH_COMPLETION_DIR)/handoverctl.fish
	install -Dm644 $(UNIT_SRC) $(USER_UNIT_DIR)/handoverd.service
	install -Dm644 quickshell/HandoverService.qml $(QUICKSHELL_INSTALL_DIR)/HandoverService.qml
	install -Dm644 quickshell/*.js $(QUICKSHELL_INSTALL_DIR)/
	install -Dm644 quickshell/example.qml $(QUICKSHELL_INSTALL_DIR)/example.qml
	install -Dm644 quickshell/README.md $(QUICKSHELL_INSTALL_DIR)/README.md
	install -d $(QUICKSHELL_INSTALL_DIR)/pages
	install -Dm644 quickshell/pages/*.qml $(QUICKSHELL_INSTALL_DIR)/pages/
	install -Dm755 google-messages/tools/install_native_probe.py $(GOOGLE_MESSAGES_TOOLS_DIR)/install_native_probe.py
	install -Dm644 google-messages/tools/chrome-observer/manifest.json $(GOOGLE_MESSAGES_TOOLS_DIR)/chrome-observer/manifest.json
	install -Dm644 google-messages/tools/chrome-observer/popup.html $(GOOGLE_MESSAGES_TOOLS_DIR)/chrome-observer/popup.html
	install -Dm644 google-messages/tools/chrome-observer/*.js $(GOOGLE_MESSAGES_TOOLS_DIR)/chrome-observer/
	install -Dm644 google-messages/tools/chrome-observer/privacy.mjs google-messages/tools/chrome-observer/wire-shape.mjs google-messages/tools/chrome-observer/README.md $(GOOGLE_MESSAGES_TOOLS_DIR)/chrome-observer/
	install -Dm644 LICENSE $(LICENSE_DIR)/LICENSE
	install -Dm644 third_party/ukey2/LICENSE $(LICENSE_DIR)/ukey2/LICENSE
	install -Dm644 third_party/ukey2/README.md $(LICENSE_DIR)/ukey2/README.md

install-user:
	cargo build --release --locked -p handoverd -p handoverctl -p handover-google-messages
	$(MAKE) install-files
	systemctl --user daemon-reload
	systemctl --user reenable handoverd.service
	systemctl --user restart handoverd.service
	@for attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do \
		if test -S "$$XDG_RUNTIME_DIR/handover/handoverd.sock"; then exit 0; fi; \
		sleep 0.1; \
	done; \
	echo "handoverd did not create its socket; check: systemctl --user status handoverd" >&2; \
	exit 1

uninstall-user:
	-systemctl --user disable --now handoverd.service
	rm -f $(USER_UNIT_DIR)/handoverd.service
	rm -f $(PREFIX)/bin/handoverd $(PREFIX)/bin/handoverctl
	rm -f $(PREFIX)/bin/handover-google-messages-helper
	rm -f $(PREFIX)/bin/handover-google-messages-auth-probe
	rm -f $(BIN_DIR)/handover-gui
	rm -f $(BIN_DIR)/handover-gui.data-home
	rm -f $(APPLICATIONS_DIR)/handover.desktop
	rm -f $(MAN_DIR)/handoverctl.1
	rm -f $(MAN8_DIR)/handoverd.8
	rm -f $(LICENSE_DIR)/LICENSE $(LICENSE_DIR)/ukey2/LICENSE $(LICENSE_DIR)/ukey2/README.md
	-rmdir $(LICENSE_DIR)/ukey2
	-rmdir $(LICENSE_DIR)
	rm -f $(BASH_COMPLETION_DIR)/handoverctl
	rm -f $(ZSH_COMPLETION_DIR)/_handoverctl
	rm -f $(FISH_COMPLETION_DIR)/handoverctl.fish
	rm -f $(QUICKSHELL_INSTALL_DIR)/HandoverService.qml
	rm -f $(QUICKSHELL_INSTALL_DIR)/*.js
	rm -f $(QUICKSHELL_INSTALL_DIR)/example.qml
	rm -f $(QUICKSHELL_INSTALL_DIR)/README.md
	rm -f $(QUICKSHELL_INSTALL_DIR)/pages/*.qml
	rm -f $(GOOGLE_MESSAGES_TOOLS_DIR)/install_native_probe.py
	rm -f $(GOOGLE_MESSAGES_TOOLS_DIR)/chrome-observer/manifest.json
	rm -f $(GOOGLE_MESSAGES_TOOLS_DIR)/chrome-observer/popup.html
	rm -f $(GOOGLE_MESSAGES_TOOLS_DIR)/chrome-observer/*.js
	rm -f $(GOOGLE_MESSAGES_TOOLS_DIR)/chrome-observer/privacy.mjs $(GOOGLE_MESSAGES_TOOLS_DIR)/chrome-observer/wire-shape.mjs $(GOOGLE_MESSAGES_TOOLS_DIR)/chrome-observer/README.md
	-rmdir $(GOOGLE_MESSAGES_TOOLS_DIR)/chrome-observer
	-rmdir $(GOOGLE_MESSAGES_TOOLS_DIR)
	-rmdir $(DESTDIR)$(USER_DATA_HOME)/handover/google-messages
	-rmdir $(QUICKSHELL_INSTALL_DIR)/pages
	-rmdir $(QUICKSHELL_INSTALL_DIR)
	-systemctl --user daemon-reload
