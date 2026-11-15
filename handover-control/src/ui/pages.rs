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
        self.body.append(&self.remote_input(&device.id));
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
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("notification-list");
        for note in notes.iter().take(100) {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.add_css_class("notification-item");
            let initials = note
                .app_name
                .split_whitespace()
                .filter_map(|word| word.chars().next())
                .take(2)
                .flat_map(char::to_uppercase)
                .collect::<String>();
            let badge = gtk::Label::new(Some(if initials.is_empty() { "?" } else { &initials }));
            badge.add_css_class("notification-app-badge");
            badge.set_size_request(32, 32);
            badge.set_valign(gtk::Align::Start);
            row.append(&badge);
            let content = gtk::Box::new(gtk::Orientation::Vertical, 5);
            content.set_hexpand(true);
            let context = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let app = text(&note.app_name, "notification-app-name");
            app.set_wrap(false);
            app.set_ellipsize(gtk::pango::EllipsizeMode::End);
            app.set_hexpand(true);
            context.append(&app);
            if self.selected.borrow().is_none() {
                let device = text(&model.name(&note.id.device_id), "notification-device");
                device.set_wrap(false);
                device.set_ellipsize(gtk::pango::EllipsizeMode::End);
                device.set_max_width_chars(18);
                context.append(&device);
            }
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
                dismiss.add_css_class("notification-dismiss");
                context.append(&dismiss);
            }
            content.append(&context);
            if !note.title.is_empty() {
                let title = text(&note.title, "notification-title");
                title.set_selectable(true);
                content.append(&title);
            }
            if !note.body.is_empty() {
                let body = text(&note.body, "notification-body");
                body.set_selectable(true);
                body.set_lines(6);
                body.set_ellipsize(gtk::pango::EllipsizeMode::End);
                content.append(&body);
            }
            if !note.actions.is_empty() || note.reply_supported {
                let actions = gtk::FlowBox::new();
                actions.set_selection_mode(gtk::SelectionMode::None);
                actions.set_min_children_per_line(1);
                actions.set_max_children_per_line(8);
                actions.set_column_spacing(4);
                actions.set_row_spacing(4);
                actions.set_halign(gtk::Align::Start);
                actions.add_css_class("notification-actions");
                for action in &note.actions {
                    let button = self.button(
                        &action.label,
                        model.ready(&note.id.device_id),
                        Request::Action(note.id.clone(), action.id.clone()),
                    );
                    button.add_css_class("flat");
                    actions.insert(&button, -1);
                }
                if note.reply_supported {
                    let reply = gtk::Button::with_label("Reply…");
                    reply.set_sensitive(model.ready(&note.id.device_id) && !self.busy());
                    reply.add_css_class("flat");
                    let weak = Rc::downgrade(self);
                    let id = note.id.clone();
                    reply.connect_clicked(move |_| {
                        if let Some(this) = weak.upgrade() {
                            this.open_notification_reply(id.clone());
                        }
                    });
                    actions.insert(&reply, -1);
                }
                content.append(&actions);
            }
            row.append(&content);
            list.append(&row);
        }
        self.body.append(&list);
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
                TransferActivityEvent::Download { download } => {
                    let basename = std::path::Path::new(&download.source_path)
                        .file_name()
                        .and_then(|v| v.to_str())
                        .unwrap_or(&download.source_path);
                    let (title, detail) = match &download.state {
                        FileDownloadState::Queued => (
                            "Download queued",
                            format!("{basename} · Waiting for the phone"),
                        ),
                        FileDownloadState::Preparing {
                            processed_bytes,
                            total_bytes,
                        } => (
                            "Checking source",
                            format!(
                                "{basename} · {}{} bytes checked",
                                processed_bytes,
                                total_bytes.map(|n| format!(" of {n}")).unwrap_or_default()
                            ),
                        ),
                        FileDownloadState::Receiving => {
                            ("Downloading", format!("{basename} · Receiving from phone"))
                        }
                        FileDownloadState::Verifying => (
                            "Verifying download",
                            format!("{basename} · Checking received files"),
                        ),
                        FileDownloadState::Paused { reason } => (
                            "Download paused",
                            format!("{basename} · {}", super::download_failure(*reason)),
                        ),
                        FileDownloadState::Cancelled => ("Download cancelled", basename.to_owned()),
                        FileDownloadState::Completed { path } => {
                            ("Download saved", format!("{basename} · {path}"))
                        }
                        FileDownloadState::Failed { reason } => (
                            "Download failed",
                            format!("{basename} · {}", super::download_failure(*reason)),
                        ),
                    };
                    (&download.device_id, title.into(), detail)
                }
            };
            if !self.matches_device(id) {
                continue;
            }
            if let TransferActivityEvent::Received { share } = &item.event
                && let SharedResource::File { path } = &share.resource
                && model.transfers.iter().any(|other| matches!(&other.event,
                    TransferActivityEvent::Download { download } if download.device_id == share.device_id
                        && matches!(&download.state, FileDownloadState::Completed { path: saved } if saved == path)))
            { continue; }
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
            if let TransferActivityEvent::Download { download } = &item.event {
                let p = &download.progress;
                let mut progress = Vec::new();
                if p.total_bytes.is_some() || p.received_bytes > 0 {
                    progress.push(match p.total_bytes {
                        Some(total) => format!("{} / {} bytes", p.received_bytes, total),
                        None => format!("{} bytes", p.received_bytes),
                    });
                }
                if p.total_files.is_some() || p.completed_files > 0 {
                    progress.push(match p.total_files {
                        Some(total) => format!("{} / {} files", p.completed_files, total),
                        None => format!("{} files", p.completed_files),
                    });
                }
                if !progress.is_empty() {
                    card.append(&text(&progress.join(" · "), "card-subtitle"));
                }
                if let FileDownloadState::Paused { reason } = &download.state {
                    card.append(&text(
                        &format!("Paused: {}", super::download_failure(*reason)),
                        "card-subtitle",
                    ));
                }
                for control in [DownloadControl::Resume, DownloadControl::Cancel] {
                    if download.controls.contains(&control) {
                        let label = if control == DownloadControl::Resume {
                            "Resume"
                        } else {
                            "Cancel"
                        };
                        let button = gtk::Button::with_label(label);
                        button.set_sensitive(if control == DownloadControl::Resume {
                            model.ready(id)
                        } else {
                            model.connected
                        });
                        let weak = Rc::downgrade(self);
                        let device = download.device_id.clone();
                        let transfer = download.transfer_id.clone();
                        button.connect_clicked(move |_| {
                            if let Some(this) = weak.upgrade() {
                                this.request(Request::DownloadControl(
                                    device.clone(),
                                    transfer.clone(),
                                    control,
                                ));
                            }
                        });
                        card.append(&button);
                    }
                }
            }
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
            if let TransferActivityEvent::Download {
                download:
                    FileDownload {
                        state: FileDownloadState::Completed { path },
                        kind,
                        ..
                    },
            } = &item.event
                && let Some(path) = std::path::Path::new(path).parent().map(|p| p.to_path_buf())
            {
                let target = if *kind == FileDownloadKind::Directory {
                    path.clone()
                } else {
                    path.parent()
                        .unwrap_or(std::path::Path::new("."))
                        .to_path_buf()
                };
                let open = gtk::Button::with_label(if *kind == FileDownloadKind::Directory {
                    "Open folder"
                } else {
                    "Open containing folder"
                });
                open.connect_clicked(move |_| {
                    let uri = gio::File::for_path(&target).uri();
                    let _ =
                        gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
                });
                card.append(&open);
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
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let avatar = contact_images::avatar(&contact.display_name, 42);
            contact_images::load_photo(
                &avatar,
                &format!("{}:{}", contact.device_id, contact.local_id),
                contact.photo.as_deref(),
            );
            row.append(&avatar);
            card.set_hexpand(true);
            row.append(&card);
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
                if contact.phones.contains(address)
                    && let Some(number) = dial_number(address)
                {
                    let call = gtk::Button::with_label("Call…");
                    call.set_sensitive(
                        model.ready(&contact.device_id)
                            && model
                                .calls
                                .get(&contact.device_id)
                                .is_some_and(|c| c.controls.contains(&CallAction::Place))
                            && !self.busy(),
                    );
                    let weak = Rc::downgrade(self);
                    let id = contact.device_id.clone();
                    call.connect_clicked(move |_| {
                        if let Some(this) = weak.upgrade() {
                            this.select_device(Some(id.clone()));
                            this.dial_address.replace(number.clone());
                            this.navigate(Page::Calls);
                        }
                    });
                    row.append(&call);
                }
                card.append(&row);
            }
            row.add_css_class("contact-with-avatar");
            list.append(&row);
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
        self.laptop_audio();
        self.body.append(&text("Phone playback", "list-heading"));
        if let Some(id) = self.selected.borrow().clone() {
            self.body.append(&self.phone_volume_panel(&id));
        }
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
            let device_id = DeviceId::new(format!("native:{}", peer.id));
            let fingerprint = text(&peer.fingerprint, "card-subtitle");
            fingerprint.set_selectable(true);
            let details = gtk::Expander::new(Some("Device fingerprint"));
            details.set_child(Some(&fingerprint));
            card.append(&details);
            card.append(&text(
                "This name is used only by Handover on this computer.",
                "card-subtitle",
            ));
            let name = gtk::Entry::new();
            name.set_max_length(80);
            name.set_placeholder_text(Some("Name in Handover"));
            let effective_name = model
                .devices
                .get(&device_id)
                .map(|device| device.name.as_str())
                .unwrap_or(&peer.name);
            name.set_text(
                self.device_name_drafts
                    .borrow()
                    .get(&device_id)
                    .map(String::as_str)
                    .unwrap_or(effective_name),
            );
            let weak = Rc::downgrade(self);
            let draft_device = device_id.clone();
            name.connect_changed(move |entry| {
                if let Some(this) = weak.upgrade() {
                    this.device_name_drafts
                        .borrow_mut()
                        .insert(draft_device.clone(), entry.text().to_string());
                }
            });
            card.append(&name);
            let name_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let save_name = gtk::Button::with_label("Save name");
            save_name.set_sensitive(model.connected && !self.busy());
            let weak = Rc::downgrade(self);
            let target = device_id.clone();
            let entry = name.clone();
            save_name.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.request(Request::RenameDevice(
                        target.clone(),
                        entry.text().to_string(),
                    ));
                }
            });
            name_actions.append(&save_name);
            let reset_name = gtk::Button::with_label("Use phone name");
            reset_name.set_sensitive(model.connected && !self.busy());
            let weak = Rc::downgrade(self);
            reset_name.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.request(Request::RenameDevice(device_id.clone(), String::new()));
                }
            });
            name_actions.append(&reset_name);
            card.append(&name_actions);
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
        self.desktop_tools();
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

fn dial_number(address: &str) -> Option<String> {
    if !address
        .chars()
        .all(|c| c.is_ascii_digit() || "+()- .".contains(c))
    {
        return None;
    }
    let number = address
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect::<String>();
    valid_call_address(&number).then_some(number)
}
