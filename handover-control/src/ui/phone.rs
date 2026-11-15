use super::*;

impl Window {
    pub(super) fn phone_controls(self: &Rc<Self>, id: &DeviceId) -> gtk::Box {
        let ready = self.model.borrow().ready(id);
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel.add_css_class("detail-list");
        for (title, detail, label, action, enabled) in [
            (
                "Find your phone",
                "Play its ringing sound",
                "Ring",
                DeviceCommandAction::Ring,
                None,
            ),
            (
                "Lock screen",
                "Lock the phone",
                "Lock",
                DeviceCommandAction::Lock,
                None,
            ),
            (
                "Screen timeout",
                "Keep the screen awake or restore its normal timeout",
                "Keep awake",
                DeviceCommandAction::KeepAwake,
                Some(true),
            ),
            (
                "Hotspot",
                "Open tethering settings on the phone",
                "Open settings",
                DeviceCommandAction::Tethering,
                None,
            ),
        ] {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
            row.add_css_class("settings-row");
            let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
            labels.set_hexpand(true);
            labels.append(&text(title, "row-title"));
            labels.append(&text(detail, "card-subtitle"));
            row.append(&labels);
            row.append(&self.button(label, ready, Request::Phone(id.clone(), action, enabled)));
            if action == DeviceCommandAction::KeepAwake {
                row.append(&self.button(
                    "Release",
                    ready,
                    Request::Phone(id.clone(), action, Some(false)),
                ));
            }
            panel.append(&row);
        }
        panel
    }

    pub(super) fn calls(self: &Rc<Self>) {
        let model = self.model.borrow();
        let Some(id) = self.selected.borrow().clone() else {
            self.body.append(&empty(
                "call-start-symbolic",
                "Choose a phone",
                "Select a device in the sidebar to see its call controls.",
            ));
            self.body.append(&self.call_audio_panel());
            return;
        };
        let call = model.calls.get(&id);
        let ready = model.ready(&id);
        let panel = card("Phone calls", &model.name(&id));
        panel.append(&text(
            match call.map(|c| c.phase) {
                Some(CallPhase::Idle) => "No active call",
                Some(CallPhase::Ringing) => "Incoming call",
                Some(CallPhase::OffHook) => "Dialing or call in progress",
                _ => "Call state not reported",
            },
            "call-state",
        ));
        let number = gtk::Entry::new();
        number.set_placeholder_text(Some("Phone number, including country code"));
        number.set_text(&self.dial_address.borrow());
        if call.is_some_and(|c| c.controls.contains(&CallAction::Place)) {
            panel.append(&number);
        }
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let place = gtk::Button::with_label("Call");
        let generation = call.map(|c| c.generation);
        let can_place =
            ready && call.is_some_and(|c| c.controls.contains(&CallAction::Place)) && !self.busy();
        place.set_sensitive(can_place && valid_call_address(&number.text()));
        let weak = Rc::downgrade(self);
        let weak_place = place.downgrade();
        number.connect_changed(move |entry| {
            if let Some(this) = weak.upgrade() {
                this.dial_address.replace(entry.text().to_string());
            }
            if let Some(button) = weak_place.upgrade() {
                button.set_sensitive(can_place && valid_call_address(&entry.text()));
            }
        });
        let weak = Rc::downgrade(self);
        let target = id.clone();
        place.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                let model = this.model.borrow();
                let current = model.calls.get(&target);
                let valid = model.ready(&target)
                    && !this.busy()
                    && current.is_some_and(|c| {
                        Some(c.generation) == generation && c.controls.contains(&CallAction::Place)
                    });
                drop(model);
                let address = this.dial_address.borrow().clone();
                if valid
                    && valid_call_address(&address)
                    && let Some(generation) = generation
                {
                    this.request(Request::Call(
                        target.clone(),
                        CallAction::Place,
                        Some(address),
                        generation,
                    ));
                }
            }
        });
        if call.is_some_and(|c| c.controls.contains(&CallAction::Place)) {
            actions.append(&place);
        }
        if let Some(call) = call {
            for (label, action) in [
                ("Answer", CallAction::Answer),
                ("Decline", CallAction::Decline),
                ("Hang up", CallAction::Hangup),
            ] {
                if call.controls.contains(&action) {
                    actions.append(&self.button(
                        label,
                        ready,
                        Request::Call(id.clone(), action, None, call.generation),
                    ));
                }
            }
        }
        panel.append(&actions);
        self.body.append(&panel);
        self.body.append(&self.call_audio_panel());
    }

    fn call_audio_panel(self: &Rc<Self>) -> gtk::Expander {
        let model = self.model.borrow();
        let status = model.call_audio.as_ref();
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 6);
        panel.add_css_class("card");
        panel.append(&text(
            match status {
                Some(s) if !s.observed => "Local call-audio status unavailable",
                Some(s) if !s.gateway_ready => "No local HFP gateway profile selected",
                Some(s) if s.duplex_running => "Local HFP input and output running",
                Some(_) => "HFP profile selected · Input and output not running",
                None => "Refresh to inspect local call audio",
            },
            "body-copy",
        ));
        panel.append(&text("This computer's audio observations do not confirm the phone association or microphone/speaker routing.", "card-subtitle"));
        self.remembered_expander("call-audio", "Call audio on this computer", &panel)
    }

    pub(super) fn file_tabs(self: &Rc<Self>) {
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        tabs.add_css_class("section-tabs");
        for (label, page) in [("Transfers", Page::Sharing), ("On phone", Page::PhoneFiles)] {
            let button = gtk::Button::with_label(label);
            if self.page.get() == page {
                button.add_css_class("selected");
            }
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.navigate(page);
                }
            });
            tabs.append(&button);
        }
        self.body.append(&tabs);
    }

    fn browse_folder(self: &Rc<Self>, id: DeviceId, path: String) {
        if self.folder_busy.get() || self.busy() || !self.model.borrow().ready(&id) {
            return;
        }
        let path = if path.trim().is_empty() {
            ".".to_owned()
        } else {
            path.trim().to_owned()
        };
        if std::path::Path::new(&path).is_absolute()
            || path.split('/').any(|s| s == "..")
            || path.contains(['\0', '\\'])
        {
            self.transient_notice("Use a folder path relative to the phone's shared root.");
            return;
        }
        self.folder_path.replace(path.clone());
        self.folder_requested
            .replace(Some((id.clone(), path.clone())));
        self.model.borrow_mut().folder = None;
        self.folder_busy.set(true);
        self.request(Request::Folder(id, path));
        let weak = Rc::downgrade(self);
        let generation = self.folder_generation.get().wrapping_add(1);
        self.folder_generation.set(generation);
        glib::timeout_add_local_once(std::time::Duration::from_secs(20), move || {
            if let Some(this) = weak.upgrade()
                && this.folder_busy.get()
                && this.folder_generation.get() == generation
            {
                this.folder_busy.set(false);
                this.transient_notice(
                    "No folder listing received. Reconnect the phone or try Open folder again.",
                );
                this.schedule();
            }
        });
    }

    pub(super) fn phone_files(self: &Rc<Self>) {
        self.file_tabs();
        let Some(id) = self.selected.borrow().clone() else {
            self.body.append(&empty(
                "folder-symbolic",
                "Choose a phone",
                "Select a device in the sidebar to browse shared folders.",
            ));
            return;
        };
        let model = self.model.borrow();
        let ready = model.ready(&id) && !self.busy() && !self.folder_busy.get();
        let can_download = model
            .devices
            .get(&id)
            .is_some_and(|device| device.capabilities.contains(&Capability::FileDownload));
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let path = gtk::Entry::new();
        path.set_hexpand(true);
        path.set_text(&self.folder_path.borrow());
        path.set_placeholder_text(Some("Folder under the shared root"));
        let weak = Rc::downgrade(self);
        path.connect_changed(move |entry| {
            if let Some(this) = weak.upgrade() {
                this.folder_path.replace(entry.text().to_string());
            }
        });
        toolbar.append(&path);
        let open = gtk::Button::with_label("Open folder");
        open.set_sensitive(ready);
        let weak = Rc::downgrade(self);
        let target = id.clone();
        open.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                let path = this.folder_path.borrow().clone();
                this.browse_folder(target.clone(), path);
            }
        });
        toolbar.append(&open);
        self.body.append(&toolbar);
        if !can_download {
            self.body.append(&text("This phone app does not support downloads yet. Update the phone app to download files.", "card-subtitle"));
        }
        let result = model.folder.as_ref().filter(|r| {
            r.device_id == id
                && self.folder_requested.borrow().as_ref() == Some(&(id.clone(), r.path.clone()))
        });
        if self.folder_busy.get() {
            self.body.append(&text("Loading folder…", "card-subtitle"));
        } else if let Some(result) = result {
            if result.failure.is_some() {
                let detail = if result.failure.as_deref() == Some("permission_required") {
                    "Enable file sharing in the phone app, then try again."
                } else {
                    "This folder is unavailable. Android may restrict access, or it may have moved."
                };
                self.body.append(&text(detail, "transfer-error"));
            } else if result.entries.is_empty() {
                self.body.append(&empty(
                    "folder-symbolic",
                    "Empty folder",
                    "The phone returned no entries.",
                ));
            } else {
                self.body.append(&text(
                    &format!("Contents of {}", result.path),
                    "card-subtitle",
                ));
                let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
                list.add_css_class("detail-list");
                for entry in result.entries.iter().take(512) {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                    row.add_css_class("settings-row");
                    row.append(&gtk::Image::from_icon_name(if entry.directory {
                        "folder-symbolic"
                    } else {
                        "text-x-generic-symbolic"
                    }));
                    let name = text(&entry.name, "body-copy");
                    name.set_hexpand(true);
                    row.append(&name);
                    if let Some(size) = entry.size {
                        row.append(&text(&format!("{size} bytes"), "card-subtitle"));
                    }
                    if entry.directory
                        && !entry.name.contains(['/', '\\'])
                        && entry.name != ".."
                        && entry.name != "."
                    {
                        let child = if result.path == "." {
                            entry.name.clone()
                        } else {
                            format!("{}/{}", result.path, entry.name)
                        };
                        let button = gtk::Button::with_label("Open");
                        button.set_sensitive(ready);
                        let weak = Rc::downgrade(self);
                        let target = id.clone();
                        button.connect_clicked(move |_| {
                            if let Some(this) = weak.upgrade() {
                                this.browse_folder(target.clone(), child.clone());
                            }
                        });
                        row.append(&button);
                    }
                    if !entry.directory
                        && can_download
                        && !entry.name.contains(['/', '\\'])
                        && entry.name != "."
                        && entry.name != ".."
                    {
                        let source = if result.path == "." {
                            entry.name.clone()
                        } else {
                            format!("{}/{}", result.path, entry.name)
                        };
                        let download = model.transfers.iter().find_map(|activity| match &activity
                            .event
                        {
                            TransferActivityEvent::Download { download }
                                if download.device_id == id && download.source_path == source =>
                            {
                                Some(download.clone())
                            }
                            _ => None,
                        });
                        if let Some(download) = &download {
                            match &download.state {
                                FileDownloadState::Queued => {
                                    row.append(&text("Queued…", "card-subtitle"))
                                }
                                FileDownloadState::Receiving => {
                                    row.append(&text("Receiving…", "card-subtitle"))
                                }
                                FileDownloadState::Failed { reason } => {
                                    row.append(&text(
                                        &format!("Failed · {}", super::download_failure(*reason)),
                                        "transfer-error",
                                    ));
                                    let retry = gtk::Button::with_label("Retry");
                                    retry.set_sensitive(ready);
                                    let weak = Rc::downgrade(self);
                                    let target = id.clone();
                                    retry.connect_clicked(move |_| {
                                        if let Some(this) = weak.upgrade() {
                                            this.request(Request::Download(
                                                target.clone(),
                                                source.clone(),
                                            ));
                                        }
                                    });
                                    row.append(&retry);
                                }
                                FileDownloadState::Completed { path } => {
                                    row.append(&text("Saved", "transfer-success"));
                                    let saved = path.clone();
                                    let reveal = gtk::Button::with_label("Open containing folder");
                                    reveal.connect_clicked(move |_| {
                                        if let Some(parent) = std::path::Path::new(&saved).parent()
                                        {
                                            let uri = gio::File::for_path(parent).uri();
                                            let _ = gio::AppInfo::launch_default_for_uri(
                                                &uri,
                                                None::<&gio::AppLaunchContext>,
                                            );
                                        }
                                    });
                                    row.append(&reveal);
                                    let again = gtk::Button::with_label("Download again");
                                    again.set_sensitive(ready);
                                    let weak = Rc::downgrade(self);
                                    let target = id.clone();
                                    again.connect_clicked(move |_| {
                                        if let Some(this) = weak.upgrade() {
                                            this.request(Request::Download(
                                                target.clone(),
                                                source.clone(),
                                            ));
                                        }
                                    });
                                    row.append(&again);
                                }
                            }
                        } else {
                            let button = gtk::Button::with_label("Download");
                            button.set_sensitive(ready && !self.busy());
                            let weak = Rc::downgrade(self);
                            let target = id.clone();
                            button.connect_clicked(move |_| {
                                if let Some(this) = weak.upgrade() {
                                    this.request(Request::Download(target.clone(), source.clone()));
                                }
                            });
                            row.append(&button);
                        }
                    }
                    list.append(&row);
                }
                self.body.append(&list);
            }
        } else {
            self.body.append(&empty(
                "folder-symbolic",
                "Phone folders",
                if can_download {
                    "Browse folders shared by your phone and download files to this computer."
                } else {
                    "Downloads need the updated phone app with the file download capability."
                },
            ));
        }
        let downloads = model
            .transfers
            .iter()
            .filter_map(|activity| match &activity.event {
                TransferActivityEvent::Download { download } if download.device_id == id => {
                    Some(download)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if !downloads.is_empty() {
            let recent = card("Recent downloads", &model.name(&id));
            for download in downloads {
                let basename = std::path::Path::new(&download.source_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&download.source_path);
                let state = match &download.state {
                    FileDownloadState::Queued => "Queued",
                    FileDownloadState::Receiving => "Receiving",
                    FileDownloadState::Completed { .. } => "Saved",
                    FileDownloadState::Failed { reason } => match reason {
                        FileDownloadFailure::PermissionRequired => {
                            "Enable file sharing in the phone app"
                        }
                        _ => "Failed",
                    },
                };
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row.append(&text(basename, "body-copy"));
                row.append(&text(state, "card-subtitle"));
                if let FileDownloadState::Completed { path } = &download.state {
                    let saved = path.clone();
                    let reveal = gtk::Button::with_label("Open containing folder");
                    reveal.connect_clicked(move |_| {
                        if let Some(parent) = std::path::Path::new(&saved).parent() {
                            let uri = gio::File::for_path(parent).uri();
                            let _ = gio::AppInfo::launch_default_for_uri(
                                &uri,
                                None::<&gio::AppLaunchContext>,
                            );
                        }
                    });
                    row.append(&reveal);
                }
                recent.append(&row);
            }
            self.body.append(&recent);
        }
    }
}
