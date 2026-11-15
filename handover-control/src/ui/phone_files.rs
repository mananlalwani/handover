use super::*;

impl Window {
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

    fn browse_folder(self: &Rc<Self>, id: DeviceId, path: String, cursor: Option<String>) {
        if self.folder_busy.get() || self.busy() || !self.model.borrow().ready(&id) {
            return;
        }
        let path = if path.trim().is_empty() {
            ".".into()
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
        let root = cursor.is_none();
        self.folder_path.replace(path.clone());
        self.folder_requested
            .replace(Some((id.clone(), path.clone())));
        self.folder_pending_cursor.replace(cursor.clone());
        if root {
            self.folder_entries.borrow_mut().clear();
            self.folder_cursor.replace(None);
        }
        self.folder_request_id.borrow_mut().take();
        self.folder_early_result.borrow_mut().clear();
        self.folder_busy.set(true);
        self.request(Request::FolderPage(id, path, cursor));
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
                    "No folder listing received. Reconnect the phone or try again.",
                );
                this.schedule();
            }
        });
    }

    pub(super) fn folder_submitted(self: &Rc<Self>, request_id: String) {
        if !self.folder_busy.get() {
            return;
        }
        self.folder_request_id.replace(Some(request_id.clone()));
        let early = self.folder_early_result.borrow_mut().remove(&request_id);
        self.folder_early_result.borrow_mut().clear();
        if let Some(result) = early {
            self.accept_folder_result(result);
        }
    }

    pub(super) fn accept_folder_result(self: &Rc<Self>, result: FilesystemResult) {
        if self.folder_requested.borrow().as_ref()
            != Some(&(result.device_id.clone(), result.path.clone()))
        {
            return;
        }
        let expected = self.folder_request_id.borrow().clone();
        if expected.is_none() && self.folder_busy.get() {
            let mut early = self.folder_early_result.borrow_mut();
            if early.len() >= 4 {
                early.pop_first();
            }
            early.insert(result.request_id.clone(), result);
            return;
        }
        if expected.as_deref() != Some(result.request_id.as_str()) {
            return;
        }
        self.folder_busy.set(false);
        if result.failure.is_none() {
            if result.cursor_next.is_some()
                && result.cursor_next == *self.folder_pending_cursor.borrow()
            {
                self.transient_notice(
                    "The phone repeated a folder page. Refresh the folder to try again.",
                );
                return;
            }
            let mut entries = self.folder_entries.borrow_mut();
            if self.folder_pending_cursor.borrow().is_none() {
                entries.clear();
            }
            for entry in &result.entries {
                if !entries.iter().any(|old| old.name == entry.name) {
                    entries.push(entry.clone());
                }
            }
            self.folder_cursor.replace(result.cursor_next.clone());
        }
        self.model.borrow_mut().folder = Some(result);
    }

    fn transfer_progress(download: &FileDownload) -> String {
        let p = &download.progress;
        let human_bytes = |bytes: u64| {
            const KIB: u64 = 1024;
            const MIB: u64 = KIB * 1024;
            const GIB: u64 = MIB * 1024;
            if bytes >= GIB {
                format!("{:.1} GiB", bytes as f64 / GIB as f64)
            } else if bytes >= MIB {
                format!("{:.1} MiB", bytes as f64 / MIB as f64)
            } else if bytes >= KIB {
                format!("{:.1} KiB", bytes as f64 / KIB as f64)
            } else {
                format!("{bytes} bytes")
            }
        };
        let bytes = match p.total_bytes {
            Some(total) if total > 0 => {
                format!("{} / {}", human_bytes(p.received_bytes), human_bytes(total))
            }
            _ if p.received_bytes > 0 => human_bytes(p.received_bytes),
            _ => String::new(),
        };
        let files = match p.total_files {
            Some(total) => format!("{} / {} files", p.completed_files, total),
            _ if p.completed_files > 0 => format!("{} files", p.completed_files),
            _ => String::new(),
        };
        [bytes, files]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    }

    fn reveal(download: &FileDownload) -> gtk::Button {
        let path = match &download.state {
            FileDownloadState::Completed { path } => path.clone(),
            _ => String::new(),
        };
        let target = if download.kind == FileDownloadKind::Directory {
            std::path::PathBuf::from(path)
        } else {
            std::path::Path::new(&path)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .to_path_buf()
        };
        let button = gtk::Button::with_label(if download.kind == FileDownloadKind::Directory {
            "Open folder"
        } else {
            "Open containing folder"
        });
        button.connect_clicked(move |_| {
            let uri = gio::File::for_path(&target).uri();
            let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
        });
        button
    }

    fn download_rows(self: &Rc<Self>, id: &DeviceId, ready: bool) {
        let model = self.model.borrow();
        let downloads = model
            .transfers
            .iter()
            .filter_map(|activity| match &activity.event {
                TransferActivityEvent::Download { download } if &download.device_id == id => {
                    Some(download)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if downloads.is_empty() {
            return;
        }
        let recent = card("Recent downloads", &model.name(id));
        for download in downloads {
            let basename = std::path::Path::new(&download.source_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&download.source_path);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let name = text(basename, "body-copy");
            name.set_hexpand(true);
            name.set_wrap(false);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&name);
            let state = match &download.state {
                FileDownloadState::Queued => "Queued",
                FileDownloadState::Preparing { .. } => "Checking source",
                FileDownloadState::Receiving => "Receiving",
                FileDownloadState::Verifying => "Verifying",
                FileDownloadState::Paused { reason } => super::download_failure(*reason),
                FileDownloadState::Cancelled => "Cancelled",
                FileDownloadState::Completed { .. } => "Saved",
                FileDownloadState::Failed { reason } => super::download_failure(*reason),
            };
            let state_label = text(
                state,
                if matches!(&download.state, FileDownloadState::Failed { .. }) {
                    "transfer-error"
                } else {
                    "card-subtitle"
                },
            );
            state_label.set_wrap(false);
            row.append(&state_label);
            let progress = Self::transfer_progress(download);
            if !progress.is_empty() {
                row.append(&text(&progress, "card-subtitle"));
            }
            if let FileDownloadState::Paused { .. } = &download.state {
                row.append(&text(
                    "Paused. Resume when the phone is connected.",
                    "card-subtitle",
                ));
            }
            if matches!(&download.state, FileDownloadState::Completed { .. }) {
                row.append(&Self::reveal(download));
            }
            if download.controls.contains(&DownloadControl::Cancel) {
                let cancel = gtk::Button::with_label("Cancel");
                cancel.set_sensitive(model.connected);
                let weak = Rc::downgrade(self);
                let device = id.clone();
                let transfer = download.transfer_id.clone();
                cancel.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.request(Request::DownloadControl(
                            device.clone(),
                            transfer.clone(),
                            DownloadControl::Cancel,
                        ));
                    }
                });
                row.append(&cancel);
            }
            if download.controls.contains(&DownloadControl::Resume) {
                let resume = gtk::Button::with_label("Resume");
                resume.set_sensitive(ready);
                let weak = Rc::downgrade(self);
                let device = id.clone();
                let transfer = download.transfer_id.clone();
                resume.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.request(Request::DownloadControl(
                            device.clone(),
                            transfer.clone(),
                            DownloadControl::Resume,
                        ));
                    }
                });
                row.append(&resume);
            }
            recent.append(&row);
        }
        self.body.append(&recent);
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
        let management = model
            .devices
            .get(&id)
            .is_some_and(|d| d.capabilities.contains(&Capability::FileManagement));
        let can_download = management
            || model
                .devices
                .get(&id)
                .is_some_and(|d| d.capabilities.contains(&Capability::FileDownload));
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let up = gtk::Button::with_label("Up");
        up.set_sensitive(ready && self.folder_path.borrow().as_str() != ".");
        let weak = Rc::downgrade(self);
        let target = id.clone();
        up.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                let path = this.folder_path.borrow().clone();
                let parent = std::path::Path::new(&path)
                    .parent()
                    .and_then(|p| p.to_str())
                    .unwrap_or(".")
                    .to_owned();
                this.browse_folder(
                    target.clone(),
                    if parent.is_empty() {
                        ".".into()
                    } else {
                        parent
                    },
                    None,
                );
            }
        });
        toolbar.append(&up);
        let path = gtk::Entry::new();
        path.set_hexpand(true);
        path.set_text(&self.folder_path.borrow());
        path.set_placeholder_text(Some("Folder under the shared root"));
        let weak = Rc::downgrade(self);
        path.connect_changed(move |e| {
            if let Some(this) = weak.upgrade() {
                this.folder_path.replace(e.text().to_string());
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
                this.browse_folder(target.clone(), path, None);
            }
        });
        toolbar.append(&open);
        let refresh = gtk::Button::with_label("Refresh");
        refresh.set_sensitive(ready);
        let weak = Rc::downgrade(self);
        let target = id.clone();
        refresh.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                let path = this.folder_path.borrow().clone();
                this.browse_folder(target.clone(), path, None);
            }
        });
        toolbar.append(&refresh);
        self.body.append(&toolbar);
        if !can_download {
            self.body.append(&text("This phone app does not support downloads yet. Update the phone app to download files.", "card-subtitle"));
        }
        if management {
            let count = self.file_selection.borrow().len();
            let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            bar.append(&text(
                &format!("{count} selected (up to 32)"),
                "card-subtitle",
            ));
            let clear = gtk::Button::with_label("Clear selection");
            clear.set_sensitive(count > 0);
            let weak = Rc::downgrade(self);
            clear.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.file_selection.borrow_mut().clear();
                    this.schedule();
                }
            });
            bar.append(&clear);
            let download = gtk::Button::with_label("Download selected");
            download.set_sensitive(ready && count > 0 && can_download);
            let weak = Rc::downgrade(self);
            let target = id.clone();
            download.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    let paths = this
                        .file_selection
                        .borrow()
                        .iter()
                        .map(|(p, k)| (p.clone(), *k))
                        .collect();
                    this.request(Request::DownloadMany(target.clone(), paths));
                }
            });
            bar.append(&download);
            self.body.append(&bar);
        }
        if self.folder_busy.get() {
            self.body.append(&text("Loading folder…", "card-subtitle"));
        }
        let entries = self.folder_entries.borrow().clone();
        let result = model.folder.as_ref().filter(|r| {
            r.device_id == id
                && self.folder_requested.borrow().as_ref() == Some(&(id.clone(), r.path.clone()))
        });
        if let Some(result) = result
            && result.failure.is_some()
        {
            self.body.append(&text(
                if result.failure.as_deref() == Some("permission_required") {
                    "Enable file sharing in the phone app, then try again."
                } else {
                    "This folder is unavailable. Android may restrict access, or it may have moved."
                },
                "transfer-error",
            ));
        } else if !entries.is_empty() {
            let path_now = self.folder_path.borrow().clone();
            self.body
                .append(&text(&format!("Contents of {path_now}"), "card-subtitle"));
            let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
            list.add_css_class("detail-list");
            for entry in &entries {
                let source = if path_now == "." {
                    entry.name.clone()
                } else {
                    format!("{path_now}/{}", entry.name)
                };
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                row.add_css_class("settings-row");
                row.append(&gtk::Image::from_icon_name(if entry.directory {
                    "folder-symbolic"
                } else {
                    "text-x-generic-symbolic"
                }));
                if management && can_download {
                    let check = gtk::CheckButton::new();
                    check.set_active(self.file_selection.borrow().contains_key(&source));
                    check.set_sensitive(
                        self.file_selection.borrow().contains_key(&source)
                            || self.file_selection.borrow().len() < 32,
                    );
                    let weak = Rc::downgrade(self);
                    let key = source.clone();
                    let kind = if entry.directory {
                        FileDownloadKind::Directory
                    } else {
                        FileDownloadKind::File
                    };
                    check.connect_toggled(move |c| {
                        if let Some(this) = weak.upgrade() {
                            let mut s = this.file_selection.borrow_mut();
                            if c.is_active() {
                                if s.len() < 32 || s.contains_key(&key) {
                                    s.insert(key.clone(), kind);
                                }
                            } else {
                                s.remove(&key);
                            }
                            drop(s);
                            this.schedule();
                        }
                    });
                    row.append(&check);
                }
                let name = text(&entry.name, "body-copy");
                name.set_hexpand(true);
                row.append(&name);
                if let Some(size) = entry.size {
                    row.append(&text(&format!("{size} bytes"), "card-subtitle"));
                }
                if entry.directory
                    && !entry.name.contains(['/', '\\'])
                    && entry.name != "."
                    && entry.name != ".."
                {
                    let button = gtk::Button::with_label("Open");
                    button.set_sensitive(ready);
                    let weak = Rc::downgrade(self);
                    let target = id.clone();
                    let child = source.clone();
                    button.connect_clicked(move |_| {
                        if let Some(this) = weak.upgrade() {
                            this.browse_folder(target.clone(), child.clone(), None);
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
                    let previous =
                        model
                            .transfers
                            .iter()
                            .find_map(|activity| match &activity.event {
                                TransferActivityEvent::Download { download }
                                    if download.device_id == id
                                        && download.source_path == source =>
                                {
                                    Some(&download.state)
                                }
                                _ => None,
                            });
                    let action = match previous {
                        Some(
                            FileDownloadState::Queued
                            | FileDownloadState::Preparing { .. }
                            | FileDownloadState::Receiving
                            | FileDownloadState::Verifying
                            | FileDownloadState::Paused { .. },
                        ) => None,
                        Some(FileDownloadState::Failed { .. }) => Some("Retry"),
                        Some(
                            FileDownloadState::Completed { .. } | FileDownloadState::Cancelled,
                        ) => Some("Download again"),
                        None => Some("Download"),
                    };
                    if let Some(action) = action {
                        let download = gtk::Button::with_label(action);
                        download.set_sensitive(ready);
                        let weak = Rc::downgrade(self);
                        let target = id.clone();
                        let src = source.clone();
                        download.connect_clicked(move |_| {
                            if let Some(this) = weak.upgrade() {
                                this.request(Request::Download(target.clone(), src.clone()));
                            }
                        });
                        row.append(&download);
                    }
                }
                list.append(&row);
            }
            self.body.append(&list);
            if self.folder_cursor.borrow().is_some() {
                let more = gtk::Button::with_label("Load more");
                more.set_sensitive(ready);
                let weak = Rc::downgrade(self);
                let target = id.clone();
                let path = path_now.clone();
                let cursor = self.folder_cursor.borrow().clone();
                more.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.browse_folder(target.clone(), path.clone(), cursor.clone());
                    }
                });
                self.body.append(&more);
            }
        } else if !self.folder_busy.get() && self.folder_requested.borrow().is_some() {
            self.body.append(&empty(
                "folder-symbolic",
                "Empty folder",
                "The phone returned no entries.",
            ));
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
        drop(model);
        self.download_rows(&id, ready);
    }
}
