use super::*;

impl Window {
    pub(super) fn button(
        self: &Rc<Self>,
        label: &str,
        ready: bool,
        request: Request,
    ) -> gtk::Button {
        let button = gtk::Button::with_label(label);
        button.set_sensitive(ready && !self.busy());
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade()
                && !this.busy()
            {
                this.request(request.clone());
            }
        });
        button
    }
    pub(super) fn render_phone(self: &Rc<Self>) {
        while let Some(child) = self.phone_pane.first_child() {
            self.phone_pane.remove(&child);
        }
        let model = self.model.borrow();
        let selected = self.selected.borrow();
        let Some(device) = selected.as_ref().and_then(|id| model.devices.get(id)) else {
            self.phone_pane.append(&toolbar_text(
                if model.connected {
                    "All devices"
                } else {
                    "Service offline"
                },
                "card-subtitle",
            ));
            return;
        };
        let ready = model.ready(&device.id);
        let state = if ready {
            "Connected"
        } else if device.paired {
            "Offline"
        } else {
            "Not paired"
        };
        let battery = device
            .battery
            .map(|b| {
                format!(
                    " · {}%{}",
                    b.percentage(),
                    if ready {
                        if b.charging { " charging" } else { " battery" }
                    } else {
                        " last known"
                    }
                )
            })
            .unwrap_or_default();
        self.phone_pane.append(&toolbar_text(
            &format!("{state}{battery}"),
            if ready {
                "connected-label"
            } else {
                "card-subtitle"
            },
        ));
    }
    pub(super) fn overview(self: &Rc<Self>) {
        let model = self.model.borrow();
        let selected = self.selected.borrow().clone();
        let Some(device) = selected.as_ref().and_then(|id| model.devices.get(id)) else {
            let panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
            panel.add_css_class("detail-list");
            for device in model.devices.values() {
                let button = gtk::Button::with_label(&device.name);
                let weak = Rc::downgrade(self);
                let id = device.id.clone();
                button.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.select_device(Some(id.clone()));
                        this.render();
                    }
                });
                panel.append(&button);
            }
            self.body.append(&panel);
            if model.devices.is_empty() {
                self.body.append(&empty(
                    "phone-symbolic",
                    "Connect your phone",
                    "Open Handover on Android and pair it with this computer in Device settings.",
                ));
            }
            return;
        };
        let hero = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        hero.add_css_class("phone-identity");
        let icon = gtk::Image::from_icon_name("phone-symbolic");
        icon.set_pixel_size(40);
        hero.append(&icon);
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 4);
        labels.append(&text(&device.name, "device-heading"));
        labels.append(&text(
            if model.ready(&device.id) {
                "Connected to this computer"
            } else {
                "Offline · Open Handover on the phone to reconnect"
            },
            "card-subtitle",
        ));
        hero.append(&labels);
        self.body.append(&hero);
        self.body.append(&text("Device controls", "list-heading"));
        self.body.append(&self.phone_controls(&device.id));
        self.body.append(&text("Send to phone", "list-heading"));
        self.body.append(&self.share_actions(&device.id));
    }
    fn share_actions(self: &Rc<Self>, id: &DeviceId) -> gtk::Box {
        let model = self.model.borrow();
        let ready = model.ready(id);
        let files = ready
            && model
                .devices
                .get(id)
                .is_some_and(|d| d.capabilities.contains(&Capability::FileTransfer));
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        for (label, icon, kind) in [
            ("Send file", "folder-open-symbolic", 0),
            ("Send link", "insert-link-symbolic", 1),
            ("Send clipboard text", "edit-paste-symbolic", 2),
        ] {
            let button = gtk::Button::with_label(label);
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content.set_halign(gtk::Align::Center);
            content.append(&gtk::Image::from_icon_name(icon));
            content.append(&toolbar_text(label, "action-label"));
            button.set_child(Some(&content));
            button.set_tooltip_text(Some(label));
            button.add_css_class("share-action");
            button.set_sensitive((if kind == 2 { ready } else { files }) && !self.busy());
            let weak = Rc::downgrade(self);
            let id = id.clone();
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    if kind == 0 {
                        this.select_device(Some(id.clone()));
                        this.navigate(Page::Sharing);
                        this.choose_files();
                    } else {
                        this.text_dialog_to(id.clone(), kind - 1);
                    }
                }
            });
            actions.append(&button);
        }
        actions
    }
    pub(super) fn notifications(self: &Rc<Self>) {
        self.notification_drafts();
        let model = self.model.borrow();
        let query = self.search.text().to_lowercase();
        let mut notes = model
            .notifications
            .values()
            .filter(|n| {
                self.matches_device(&n.id.device_id)
                    && format!("{} {} {}", n.app_name, n.title, n.body)
                        .to_lowercase()
                        .contains(&query)
            })
            .collect::<Vec<_>>();
        notes.sort_by(|a, b| (&a.app_name, &a.id).cmp(&(&b.app_name, &b.id)));
        if notes.is_empty() {
            self.body.append(&empty(
                "preferences-system-notifications-symbolic",
                if query.is_empty() {
                    "No notifications"
                } else {
                    "No results"
                },
                if query.is_empty() {
                    "Notifications from your phone appear here."
                } else {
                    "Try a different search."
                },
            ));
            return;
        }
        let mut group_key = None;
        let mut group = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for note in notes.iter().take(100) {
            let key = (&note.app_name, &note.id.device_id);
            if group_key != Some(key) {
                let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                header.append(&gtk::Image::from_icon_name(
                    "application-x-executable-symbolic",
                ));
                header.append(&text(&note.app_name, "list-heading"));
                let device = text(&model.name(&note.id.device_id), "card-subtitle");
                device.set_hexpand(true);
                device.set_xalign(1.0);
                header.append(&device);
                self.body.append(&header);
                group = gtk::Box::new(gtk::Orientation::Vertical, 0);
                group.add_css_class("detail-list");
                self.body.append(&group);
                group_key = Some(key);
            }
            let item = gtk::Box::new(gtk::Orientation::Vertical, 6);
            item.add_css_class("notification-row");
            let title = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let label = text(&note.title, "notification-title");
            label.set_hexpand(true);
            title.append(&label);
            if note.clearable {
                let dismiss = self.button(
                    "Dismiss on phone",
                    model.ready(&note.id.device_id),
                    Request::Dismiss(note.id.clone()),
                );
                dismiss.set_icon_name("window-close-symbolic");
                dismiss.set_tooltip_text(Some("Dismiss on phone"));
                dismiss.update_property(&[gtk::accessible::Property::Label("Dismiss on phone")]);
                dismiss.add_css_class("flat");
                dismiss.add_css_class("circular");
                title.append(&dismiss);
            }
            item.append(&title);
            let body = text(&note.body, "body-copy");
            body.set_selectable(true);
            body.set_lines(6);
            body.set_ellipsize(gtk::pango::EllipsizeMode::End);
            item.append(&body);
            if !note.actions.is_empty() {
                let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                for action in &note.actions {
                    let button = self.button(
                        &action.label,
                        model.ready(&note.id.device_id),
                        Request::Action(note.id.clone(), action.id.clone()),
                    );
                    button.add_css_class("flat");
                    actions.append(&button);
                }
                item.append(&actions);
            }
            if note.reply_supported {
                let reply = gtk::Button::with_label("Reply…");
                reply.set_halign(gtk::Align::Start);
                reply.set_sensitive(model.ready(&note.id.device_id) && !self.busy());
                let weak = Rc::downgrade(self);
                let id = note.id.clone();
                reply.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.open_notification_reply(id.clone());
                    }
                });
                item.append(&reply);
            }
            group.append(&item);
        }
        if notes.len() > 100 {
            self.body.append(&text(
                "100 shown. Search to narrow the list.",
                "card-subtitle",
            ));
        }
    }
    pub(super) fn sharing(self: &Rc<Self>) {
        self.file_tabs();
        let model = self.model.borrow();
        if let Some(id) = self.selected.borrow().as_ref() {
            self.body.append(&self.share_actions(id));
        }
        self.body.append(&self.file_composer());
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        heading.append(&text("Recent transfers", "list-heading"));
        let note = text("This session", "card-subtitle");
        note.set_hexpand(true);
        note.set_xalign(1.0);
        note.set_tooltip_text(Some(
            "Keeps the last 128 observed transfers until Handover restarts.",
        ));
        heading.append(&note);
        self.body.append(&heading);
        let mut count = 0;
        for item in &model.transfers {
            let (id, title, detail): (&DeviceId, String, String) = match &item.event {
                TransferActivityEvent::Received { share } => (
                    &share.device_id,
                    "Received".into(),
                    match &share.resource {
                        SharedResource::File { path } => std::path::Path::new(path)
                            .file_name()
                            .map(|v| v.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "File".into()),
                        SharedResource::Url { url } => url.clone(),
                    },
                ),
                TransferActivityEvent::Progress { progress } => (
                    &progress.device_id,
                    "Sending".into(),
                    format!(
                        "{} of {} bytes · Awaiting confirmation",
                        progress.bytes_sent, progress.total_bytes
                    ),
                ),
                TransferActivityEvent::Result { result } => (
                    &result.device_id,
                    match result.status {
                        ShareStatus::Completed => "Received by phone",
                        ShareStatus::Failed => "Transfer failed",
                    }
                    .into(),
                    result
                        .reason
                        .map(|r| failure(r).to_string())
                        .unwrap_or_else(|| "Transfer complete".into()),
                ),
            };
            if !self.matches_device(id) {
                continue;
            }
            count += 1;
            let time = glib::DateTime::from_unix_local(
                (item.observed_at_ms / 1000).min(i64::MAX as u64) as i64,
            )
            .ok()
            .and_then(|d| d.format("%b %e · %H:%M").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
            let card = card(&title, &format!("{} · {time}", model.name(id)));
            card.add_css_class("transfer-row");
            let label = text(&detail, "body-copy");
            label.set_lines(6);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_selectable(true);
            card.append(&label);
            // Paths are not opened automatically. The user chooses whether to reveal a received file.
            if let TransferActivityEvent::Received {
                share:
                    ReceivedShare {
                        resource: SharedResource::File { path },
                        ..
                    },
            } = &item.event
            {
                let path = std::path::Path::new(path).parent().map(|p| p.to_path_buf());
                if let Some(path) = path {
                    let open = gtk::Button::with_label("Open containing folder");
                    open.connect_clicked(move |_| {
                        let uri = gio::File::for_path(&path).uri();
                        let _ = gio::AppInfo::launch_default_for_uri(
                            &uri,
                            None::<&gio::AppLaunchContext>,
                        );
                    });
                    card.append(&open);
                }
            }
            self.body.append(&card);
        }
        if count == 0 {
            self.body
                .append(&text("No recent transfers", "card-subtitle"));
        }
    }
    pub(super) fn text_dialog_to(self: &Rc<Self>, id: DeviceId, kind: u8) {
        let dialog = adw::MessageDialog::new(
            Some(&self.window),
            Some(if kind == 0 {
                "Send link"
            } else {
                "Send clipboard text"
            }),
            Some(&format!("To {}", self.model.borrow().name(&id))),
        );
        dialog.set_application(self.window.application().as_ref());
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("send", "Send");
        dialog.set_close_response("cancel");
        dialog.set_default_response(Some("cancel"));
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(if kind == 0 {
            "https://…"
        } else {
            "Text to send to your phone"
        }));
        entry.set_margin_start(20);
        entry.set_margin_end(20);
        entry.set_margin_top(20);
        entry.set_margin_bottom(20);
        dialog.set_extra_child(Some(&entry));
        dialog.set_response_enabled("send", false);
        let weak_dialog = dialog.downgrade();
        entry.connect_changed(move |entry| {
            if let Some(dialog) = weak_dialog.upgrade() {
                dialog.set_response_enabled("send", !entry.text().trim().is_empty());
            }
        });
        if kind == 1 {
            let weak = entry.downgrade();
            self.window
                .clipboard()
                .read_text_async(None::<&gio::Cancellable>, move |result| {
                    if let (Some(entry), Ok(Some(text))) = (weak.upgrade(), result)
                        && entry.text().is_empty()
                    {
                        entry.set_text(&text);
                    }
                });
        }
        let weak = Rc::downgrade(self);
        let input = entry.clone();
        dialog.connect_response(None, move |dialog, response| {
            let entry = &input;
            if response == "send" {
                let Some(this) = weak.upgrade() else {
                    return;
                };
                let value = entry.text().to_string();
                if value.trim().is_empty() {
                    entry.grab_focus();
                    return;
                }
                if !this.model.borrow().ready(&id) {
                    entry.set_placeholder_text(Some(
                        "Device disconnected. Reconnect before sending.",
                    ));
                    return;
                }
                this.request(if kind == 0 {
                    Request::SendLink(id.clone(), value)
                } else {
                    Request::Clipboard(id.clone(), value)
                });
            }
            dialog.close();
        });
        dialog.present();
        entry.grab_focus();
    }
    pub(super) fn contacts(self: &Rc<Self>) {
        let model = self.model.borrow();
        let query = self.search.text().to_lowercase();
        let contacts = model
            .contacts
            .iter()
            .filter(|c| {
                self.matches_device(&c.device_id)
                    && format!(
                        "{} {} {}",
                        c.display_name,
                        c.phones.join(" "),
                        c.emails.join(" ")
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .collect::<Vec<_>>();
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let count = text(
            &format!(
                "{} contact{}",
                contacts.len(),
                if contacts.len() == 1 { "" } else { "s" }
            ),
            "card-subtitle",
        );
        count.set_hexpand(true);
        toolbar.append(&count);
        if let Some(id) = self.selected.borrow().clone() {
            let sync = self.button(
                "Sync contacts from phone",
                model.ready(&id),
                Request::SyncContacts(id),
            );
            sync.set_icon_name("view-refresh-symbolic");
            sync.set_tooltip_text(Some("Sync contacts from phone"));
            sync.update_property(&[gtk::accessible::Property::Label("Sync contacts from phone")]);
            sync.add_css_class("flat");
            toolbar.append(&sync);
        }
        self.body.append(&toolbar);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("detail-list");
        if !contacts.is_empty() {
            self.body.append(&list);
        }
        for contact in contacts.iter().take(100) {
            let device = if self.selected.borrow().is_none() {
                model.name(&contact.device_id)
            } else {
                String::new()
            };
            let card = card(&contact.display_name, &device);
            card.add_css_class("contact-row");
            for address in contact.phones.iter().chain(&contact.emails) {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let label = text(address, "body-copy");
                label.set_hexpand(true);
                label.set_selectable(true);
                row.append(&label);
                let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
                copy.set_tooltip_text(Some("Copy"));
                copy.update_property(&[gtk::accessible::Property::Label("Copy address")]);
                copy.add_css_class("flat");
                let value = address.clone();
                let clipboard = self.window.clipboard();
                copy.connect_clicked(move |_| clipboard.set_text(&value));
                row.append(&copy);
                card.append(&row);
            }
            list.append(&card);
        }
        if contacts.is_empty() {
            self.body.append(&empty(
                "avatar-default-symbolic",
                if query.is_empty() {
                    "No contacts"
                } else {
                    "No results"
                },
                if query.is_empty() {
                    "Sync contacts from a connected phone."
                } else {
                    "Try a different search."
                },
            ));
        }
        if contacts.len() > 100 {
            self.body.append(&text(
                "Showing the first 100 contacts. Search to narrow the list.",
                "card-subtitle",
            ));
        }
    }
    pub(super) fn media(self: &Rc<Self>) {
        let model = self.model.borrow();
        let mut count = 0;
        for session in model
            .media
            .values()
            .filter(|s| self.matches_device(&s.id.device_id))
        {
            count += 1;
            let panel = gtk::Box::new(gtk::Orientation::Vertical, 18);
            panel.add_css_class("device-panel");
            let header = gtk::Box::new(gtk::Orientation::Horizontal, 16);
            let icon = gtk::Image::from_icon_name("audio-x-generic-symbolic");
            icon.set_pixel_size(32);
            icon.add_css_class("device-icon");
            header.append(&icon);
            let identity = gtk::Box::new(gtk::Orientation::Vertical, 5);
            identity.set_hexpand(true);
            identity.append(&text(
                session.title.as_deref().unwrap_or(&session.application),
                "media-title",
            ));
            if let Some(artist) = &session.artist {
                identity.append(&text(artist, "body-copy"));
            }
            identity.append(&text(
                &format!(
                    "{} · {}",
                    model.name(&session.id.device_id),
                    session.application
                ),
                "card-subtitle",
            ));
            header.append(&identity);
            panel.append(&header);
            let mut commands = Vec::new();
            if session.controls.contains(&MediaControl::Previous) {
                commands.push((
                    "Previous",
                    "media-skip-backward-symbolic",
                    MediaCommand::Previous {
                        id: session.id.clone(),
                    },
                ));
            }
            if session.playback == PlaybackState::Playing
                && session.controls.contains(&MediaControl::Pause)
            {
                commands.push((
                    "Pause",
                    "media-playback-pause-symbolic",
                    MediaCommand::Pause {
                        id: session.id.clone(),
                    },
                ));
            } else if matches!(
                session.playback,
                PlaybackState::Paused | PlaybackState::Stopped
            ) && session.controls.contains(&MediaControl::Play)
            {
                commands.push((
                    "Play",
                    "media-playback-start-symbolic",
                    MediaCommand::Play {
                        id: session.id.clone(),
                    },
                ));
            } else if session.controls.contains(&MediaControl::PlayPause) {
                commands.push((
                    "Play / pause",
                    "media-playback-start-symbolic",
                    MediaCommand::PlayPause {
                        id: session.id.clone(),
                    },
                ));
            } else {
                if session.controls.contains(&MediaControl::Play) {
                    commands.push((
                        "Play",
                        "media-playback-start-symbolic",
                        MediaCommand::Play {
                            id: session.id.clone(),
                        },
                    ));
                }
                if session.controls.contains(&MediaControl::Pause) {
                    commands.push((
                        "Pause",
                        "media-playback-pause-symbolic",
                        MediaCommand::Pause {
                            id: session.id.clone(),
                        },
                    ));
                }
            }
            if session.controls.contains(&MediaControl::Next) {
                commands.push((
                    "Next",
                    "media-skip-forward-symbolic",
                    MediaCommand::Next {
                        id: session.id.clone(),
                    },
                ));
            }
            let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            controls.set_halign(gtk::Align::Center);
            for (label, icon, command) in commands {
                let button = self.button(
                    label,
                    model.ready(&session.id.device_id),
                    Request::Media(command),
                );
                button.set_icon_name(icon);
                button.set_tooltip_text(Some(label));
                button.update_property(&[gtk::accessible::Property::Label(label)]);
                button.add_css_class("circular");
                if matches!(label, "Play" | "Pause" | "Play / pause") {
                    button.add_css_class("suggested-action");
                }
                controls.append(&button);
            }
            panel.append(&controls);
            panel.append(&text(
                match session.playback {
                    PlaybackState::Playing => "Playing",
                    PlaybackState::Paused => "Paused",
                    PlaybackState::Stopped => "Stopped",
                    PlaybackState::Unknown => "Playback state unavailable",
                },
                "card-subtitle",
            ));
            self.body.append(&panel);
        }
        if count == 0 {
            self.body.append(&empty(
                "multimedia-player-symbolic",
                "Nothing playing",
                "Start playback on your phone.",
            ));
        }
    }
    pub(super) fn pairing(self: &Rc<Self>) {
        self.body.append(&text("Pair a device", "list-heading"));
        self.body.append(&text(
            "Open Handover on your phone, then connect to this computer.",
            "card-subtitle",
        ));
        let model = self.model.borrow();
        if model.pairing.is_empty() {
            self.body
                .append(&text("No pending pairing requests.", "card-subtitle"));
        }
        for peer in &model.pairing {
            let card = card(
                &peer.name,
                "Compare this code with the phone. Approve only when both codes match.",
            );
            let code = text(&peer.code, "pair-code");
            code.set_xalign(0.5);
            card.append(&code);
            let confirm = gtk::CheckButton::with_label("The codes match on both devices");
            card.append(&confirm);
            let approve = self.button(
                "Pair device",
                false,
                Request::Pair(peer.id.clone(), peer.code.clone()),
            );
            approve.add_css_class("suggested-action");
            let weak = approve.downgrade();
            let enabled = model.connected && !self.busy();
            confirm.connect_toggled(move |check| {
                if let Some(button) = weak.upgrade() {
                    button.set_sensitive(enabled && check.is_active());
                }
            });
            card.append(&approve);
            self.body.append(&card);
        }
        for peer in &model.peers {
            let card = card(&peer.name, "Trusted device");
            let fingerprint = text(&peer.fingerprint, "card-subtitle");
            fingerprint.set_selectable(true);
            let details = gtk::Expander::new(Some("Device fingerprint"));
            details.set_child(Some(&fingerprint));
            card.append(&details);
            let forget = gtk::Button::with_label("Forget device…");
            forget.set_sensitive(model.connected && !self.busy());
            let weak = Rc::downgrade(self);
            let id = peer.id.clone();
            let name = peer.name.clone();
            forget.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    let dialog = adw::MessageDialog::new(
                        Some(&this.window),
                        Some(&format!("Forget {name}?")),
                        Some(
                            "This removes the saved trust. You will need to pair the device again.",
                        ),
                    );
                    dialog.set_application(this.window.application().as_ref());
                    dialog.add_response("cancel", "Cancel");
                    dialog.add_response("forget", "Forget device");
                    dialog.set_response_appearance("forget", adw::ResponseAppearance::Destructive);
                    dialog.set_default_response(Some("cancel"));
                    dialog.set_close_response("cancel");
                    let weak = Rc::downgrade(&this);
                    let id = id.clone();
                    dialog.connect_response(None, move |_, response| {
                        if response == "forget"
                            && let Some(this) = weak.upgrade()
                        {
                            this.request(Request::Forget(id.clone()));
                        }
                    });
                    dialog.present();
                }
            });
            card.append(&forget);
            self.body.append(&card);
        }
        let help = gtk::Expander::new(Some("Phone not finding this computer?"));
        help.set_child(Some(&text("Connect both devices to the same network. On the phone, enter this computer's address with port 24837.","card-subtitle")));
        self.body.append(&help);
    }
}
pub(super) fn failure(value: ShareFailure) -> &'static str {
    match value {
        ShareFailure::InvalidResource => "Invalid resource",
        ShareFailure::SizeLimit => "Size limit",
        ShareFailure::Storage => "Storage unavailable",
        ShareFailure::Interrupted => "Interrupted",
        ShareFailure::Rejected => "Rejected or cancelled",
        ShareFailure::TimedOut => "Timed out",
        ShareFailure::Disconnected => "Disconnected",
        ShareFailure::Transport => "Connection failure",
    }
}

fn toolbar_text(value: &str, class: &str) -> gtk::Label {
    let label = text(value, class);
    label.set_wrap(false);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_tooltip_text(Some(value));
    label
}
