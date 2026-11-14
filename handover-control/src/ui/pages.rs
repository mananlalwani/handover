use super::*;

impl Window {
    fn button(self: &Rc<Self>, label: &str, ready: bool, request: Request) -> gtk::Button {
        let button = gtk::Button::with_label(label);
        button.set_sensitive(ready && !self.busy());
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.request(request.clone());
            }
        });
        button
    }
    pub(super) fn overview(self: &Rc<Self>) {
        let model = self.model.borrow();
        if model.devices.is_empty() {
            self.body.append(&card("Your phone, on your desktop","Open Handover on your phone, connect to this computer, then review the comparison code in Connection."));
            return;
        }
        for device in model
            .devices
            .values()
            .filter(|d| self.matches_device(&d.id))
        {
            let connected = model.connected && device.connected;
            let card = card(
                &device.name,
                if connected {
                    if device.paired {
                        "Connected · Paired"
                    } else {
                        "Connected · Not paired"
                    }
                } else {
                    "Offline · Last reported state"
                },
            );
            if let Some(battery) = device.battery {
                card.append(&text(
                    &format!(
                        "{}% battery{}{}",
                        battery.percentage(),
                        if battery.charging { " · Charging" } else { "" },
                        if !connected { " · Last reported" } else { "" }
                    ),
                    "body-copy",
                ));
            } else {
                card.append(&text("Battery not reported", "card-subtitle"));
            }
            if let Some(network) = device.connectivity {
                card.append(&text(
                    &format!(
                        "Network: {}{}",
                        match network.transport {
                            ConnectivityTransport::Wifi => "Wi-Fi",
                            ConnectivityTransport::Ethernet => "Ethernet",
                            ConnectivityTransport::Cellular => "Cellular",
                            ConnectivityTransport::Bluetooth => "Bluetooth",
                            ConnectivityTransport::Other => "Other",
                            ConnectivityTransport::None => "None",
                        },
                        if network.metered { " · Metered" } else { "" }
                    ),
                    "card-subtitle",
                ));
            }
            let count = model
                .notifications
                .keys()
                .filter(|id| id.device_id == device.id)
                .count();
            card.append(&text(
                &format!(
                    "{count} phone notifications · {} synced contacts",
                    model
                        .contacts
                        .iter()
                        .filter(|c| c.device_id == device.id)
                        .count()
                ),
                "card-subtitle",
            ));
            if let Some(call) = model.calls.get(&device.id) {
                card.append(&text(
                    &format!(
                        "Phone call: {}",
                        match call.phase {
                            CallPhase::Unknown => "State not reported",
                            CallPhase::Idle => "No active call",
                            CallPhase::Ringing => "Ringing",
                            CallPhase::OffHook => "In progress",
                        }
                    ),
                    "body-copy",
                ));
            }
            let caps = device
                .capabilities
                .iter()
                .map(|c| match c {
                    Capability::Battery => "Battery",
                    Capability::Connectivity => "Network",
                    Capability::FileTransfer => "File sharing",
                    Capability::Media => "Media",
                    Capability::Notifications => "Notifications",
                })
                .collect::<Vec<_>>()
                .join(" · ");
            card.append(&text(
                &format!("Reported capabilities: {caps}"),
                "card-subtitle",
            ));
            self.body.append(&card);
        }
        if !model.pairing.is_empty() {
            self.body.append(&card(
                "Pairing request waiting",
                "Open Connection to compare the code with your phone before approving.",
            ));
        }
        if self.selected.borrow().is_none() {
            self.body.append(&text(
                "Choose a device to filter notifications, contacts, and media.",
                "card-subtitle",
            ));
        }
    }
    pub(super) fn notifications(self: &Rc<Self>) {
        let model = self.model.borrow();
        let query = self.search.text().to_lowercase();
        let notes = model
            .notifications
            .values()
            .filter(|n| {
                self.matches_device(&n.id.device_id)
                    && format!("{} {} {}", n.app_name, n.title, n.body)
                        .to_lowercase()
                        .contains(&query)
            })
            .collect::<Vec<_>>();
        if notes.is_empty() {
            self.body.append(&card("No notifications","Notifications from the selected devices appear here. Choose which apps forward them in Handover on your phone."));
        }
        for note in notes.iter().take(100) {
            let card = card(
                &note.title,
                &format!("{} · {}", note.app_name, model.name(&note.id.device_id)),
            );
            let body = text(&note.body, "body-copy");
            body.set_selectable(true);
            body.set_lines(8);
            body.set_ellipsize(gtk::pango::EllipsizeMode::End);
            card.append(&body);
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let ready = model.ready(&note.id.device_id);
            if note.clearable {
                actions.append(&self.button(
                    "Dismiss on phone",
                    ready,
                    Request::Dismiss(note.id.clone()),
                ));
            }
            for action in &note.actions {
                actions.append(&self.button(
                    &action.label,
                    ready,
                    Request::Action(note.id.clone(), action.id.clone()),
                ));
            }
            let scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .child(&actions)
                .build();
            card.append(&scroll);
            self.body.append(&card);
        }
        if notes.len() > 100 {
            self.body.append(&text(
                "Showing the first 100 notifications. Search to narrow the list.",
                "card-subtitle",
            ));
        }
    }
    pub(super) fn sharing(self: &Rc<Self>) {
        let model = self.model.borrow();
        let selected = self.selected.borrow().clone();
        let ready = selected.as_ref().is_some_and(|id| {
            model.ready(id)
                && model
                    .devices
                    .get(id)
                    .is_some_and(|d| d.capabilities.contains(&Capability::FileTransfer))
        });
        let send = card(
            "Send to your phone",
            "Choose a connected device on the left. Files and links are sent only when you explicitly submit them.",
        );
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let file = gtk::Button::with_label("Send file");
        file.set_sensitive(ready && !self.busy());
        let weak = Rc::downgrade(self);
        file.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.choose_file();
            }
        });
        actions.append(&file);
        for (label, kind) in [("Send link", 0), ("Send clipboard text", 1)] {
            let button = gtk::Button::with_label(label);
            button.set_sensitive(
                if kind == 0 {
                    ready
                } else {
                    selected.as_ref().is_some_and(|id| model.ready(id))
                } && !self.busy(),
            );
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.text_dialog(kind);
                }
            });
            actions.append(&button);
        }
        send.append(&actions);
        self.body.append(&send);
        self.body.append(&text("Recent activity · Up to 128 transfers observed by the current daemon session. This list resets when the daemon restarts. Accepted requests are not confirmed deliveries.","card-subtitle"));
        let mut count = 0;
        for item in &model.transfers {
            let (id, title, detail): (&DeviceId, String, String) = match &item.event {
                TransferActivityEvent::Received { share } => (
                    &share.device_id,
                    "Received on this computer".into(),
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
                    "Transfer progress".into(),
                    format!(
                        "{} of {} bytes written to the connection. Phone completion has not been reported.",
                        progress.bytes_sent, progress.total_bytes
                    ),
                ),
                TransferActivityEvent::Result { result } => (
                    &result.device_id,
                    match result.status {
                        ShareStatus::Completed => "Phone confirmed receipt",
                        ShareStatus::Failed => "Transfer failed",
                    }
                    .into(),
                    format!(
                        "Transfer {}{}",
                        result.transfer_id,
                        result
                            .reason
                            .map(|r| format!(" · {}", failure(r)))
                            .unwrap_or_default()
                    ),
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
            self.body.append(&card("No recent transfers","Completed incoming shares and reported outgoing progress or results will appear here."));
        }
    }
    fn choose_file(self: &Rc<Self>) {
        let Some(id) = self.selected.borrow().clone() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title("Send file to phone")
            .build();
        let weak = Rc::downgrade(self);
        dialog.open(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                if let (Some(this), Ok(file)) = (weak.upgrade(), result) {
                    if file.path().is_none() {
                        this.model.borrow_mut().notice = "Choose a local file.".into();
                        this.schedule();
                        return;
                    }
                    this.request(Request::SendFile(id, file.uri().to_string()));
                }
            },
        );
    }
    fn text_dialog(self: &Rc<Self>, kind: u8) {
        let Some(id) = self.selected.borrow().clone() else {
            return;
        };
        let dialog = adw::MessageDialog::new(
            Some(&self.window),
            Some(if kind == 0 {
                "Send link"
            } else {
                "Send clipboard text"
            }),
            Some("Review the content before sending it to your selected phone."),
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
        if let Some(id) = self.selected.borrow().clone() {
            self.body.append(&self.button(
                "Sync contacts from phone",
                model.ready(&id),
                Request::SyncContacts(id),
            ));
        }
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
        self.body.append(&text(
            &format!(
                "{} synced contacts · Search by name, phone number, or email",
                contacts.len()
            ),
            "card-subtitle",
        ));
        for contact in contacts.iter().take(100) {
            let card = card(&contact.display_name, &model.name(&contact.device_id));
            for address in contact.phones.iter().chain(&contact.emails) {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let label = text(address, "body-copy");
                label.set_hexpand(true);
                label.set_selectable(true);
                row.append(&label);
                let copy = gtk::Button::with_label("Copy");
                let value = address.clone();
                let clipboard = self.window.clipboard();
                copy.connect_clicked(move |_| clipboard.set_text(&value));
                row.append(&copy);
                card.append(&row);
            }
            self.body.append(&card);
        }
        if contacts.is_empty() {
            self.body.append(&card("Your address book","Select a connected phone and choose Sync contacts. Contact permission must be enabled on the phone."));
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
            let card = card(
                session.title.as_deref().unwrap_or(&session.application),
                &format!(
                    "{} · {}",
                    model.name(&session.id.device_id),
                    session.application
                ),
            );
            if let Some(artist) = &session.artist {
                card.append(&text(artist, "body-copy"));
            }
            card.append(&text(
                match session.playback {
                    PlaybackState::Playing => "Playing",
                    PlaybackState::Paused => "Paused",
                    PlaybackState::Stopped => "Stopped",
                    PlaybackState::Unknown => "Playback state not reported",
                },
                "card-subtitle",
            ));
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            for (control, label, command) in [
                (
                    MediaControl::Previous,
                    "Previous",
                    MediaCommand::Previous {
                        id: session.id.clone(),
                    },
                ),
                (
                    MediaControl::Play,
                    "Play",
                    MediaCommand::Play {
                        id: session.id.clone(),
                    },
                ),
                (
                    MediaControl::Pause,
                    "Pause",
                    MediaCommand::Pause {
                        id: session.id.clone(),
                    },
                ),
                (
                    MediaControl::PlayPause,
                    "Play / pause",
                    MediaCommand::PlayPause {
                        id: session.id.clone(),
                    },
                ),
                (
                    MediaControl::Next,
                    "Next",
                    MediaCommand::Next {
                        id: session.id.clone(),
                    },
                ),
            ] {
                if session.controls.contains(&control) {
                    row.append(&self.button(
                        label,
                        model.ready(&session.id.device_id),
                        Request::Media(command),
                    ));
                }
            }
            card.append(&row);
            self.body.append(&card);
        }
        if count == 0 {
            self.body.append(&card(
                "Nothing playing yet",
                "Start media on your phone. Reported players and supported controls appear here.",
            ));
        }
    }
    pub(super) fn pairing(self: &Rc<Self>) {
        self.body.append(&card("Connect your phone","Open Handover on Android and connect on the same network. If discovery is unavailable, enter this computer's reachable address and port 24837 on the phone. Refresh this view when a pairing request arrives."));
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
            card.append(&fingerprint);
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
    }
}
fn failure(value: ShareFailure) -> &'static str {
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
