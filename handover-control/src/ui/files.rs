use super::*;

pub(super) const MAX_FILES: usize = 8;
#[derive(Clone)]
pub(super) struct PickedFile {
    pub file: gio::File,
    pub status: FileStatus,
}
#[derive(Clone)]
pub(super) enum FileStatus {
    Ready,
    Submitting {
        target: String,
    },
    Accepted {
        target: String,
        device: DeviceId,
        transfer_id: Option<String>,
    },
    Received {
        target: String,
    },
    Failed {
        reason: String,
    },
    Unconfirmed {
        reason: String,
    },
}
impl FileStatus {
    fn label(&self) -> String {
        match self {
            Self::Ready => "Ready to send".into(),
            Self::Submitting { target } => format!("Submitting to {target}…"),
            Self::Accepted { target, .. } => format!("Accepted for {target} · Awaiting receipt"),
            Self::Received { target } => format!("Received by {target}"),
            Self::Failed { reason } => format!("Failed · {reason}"),
            Self::Unconfirmed { reason } => format!("Not confirmed · {reason}"),
        }
    }
}
impl Window {
    pub(super) fn choose_files(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Choose files to send")
            .build();
        let weak = Rc::downgrade(self);
        dialog.open_multiple(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                if let (Some(this), Ok(files)) = (weak.upgrade(), result) {
                    this.add_files(
                        (0..files.n_items())
                            .filter_map(|n| files.item(n).and_downcast::<gio::File>())
                            .collect(),
                    );
                }
            },
        );
    }
    pub(super) fn add_files(self: &Rc<Self>, files: Vec<gio::File>) -> bool {
        let mut selected = self.file_queue.borrow_mut();
        let mut seen = selected
            .iter()
            .map(|file| file.file.uri().to_string())
            .collect::<BTreeSet<_>>();
        let new = files
            .into_iter()
            .filter(|file| file.path().is_some())
            .filter(|file| seen.insert(file.uri().to_string()))
            .collect::<Vec<_>>();
        if new.len() + selected.len() > MAX_FILES {
            drop(selected);
            self.model.borrow_mut().notice = format!("Choose up to {MAX_FILES} files at a time.");
            self.schedule_chrome();
            return false;
        }
        let mut added = false;
        for file in new {
            if !selected.iter().any(|existing| existing.file.equal(&file)) {
                selected.push(PickedFile {
                    file,
                    status: FileStatus::Ready,
                });
                added = true;
            }
        }
        drop(selected);
        if added {
            self.navigate(Page::Sharing);
        }
        added
    }
    pub(super) fn file_composer(self: &Rc<Self>) -> gtk::Box {
        let area = gtk::Box::new(gtk::Orientation::Vertical, 14);
        area.add_css_class("file-composer");
        let drop_area = gtk::Box::new(gtk::Orientation::Vertical, 10);
        drop_area.add_css_class("file-drop");
        let icon = gtk::Image::from_icon_name("folder-download-symbolic");
        icon.set_pixel_size(32);
        drop_area.append(&icon);
        let label = text("Drop files here", "drop-title");
        label.set_xalign(0.5);
        drop_area.append(&label);
        let choose = gtk::Button::with_label("Choose files…");
        choose.set_halign(gtk::Align::Center);
        let weak = Rc::downgrade(self);
        choose.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.choose_files();
            }
        });
        drop_area.append(&choose);
        let target = gtk::DropTarget::new(
            gtk::gdk::FileList::static_type(),
            gtk::gdk::DragAction::COPY,
        );
        let weak_drop = drop_area.downgrade();
        target.connect_enter(move |_, _, _| {
            if let Some(area) = weak_drop.upgrade() {
                area.add_css_class("drop-active");
            }
            gtk::gdk::DragAction::COPY
        });
        let weak_drop = drop_area.downgrade();
        target.connect_leave(move |_| {
            if let Some(area) = weak_drop.upgrade() {
                area.remove_css_class("drop-active");
            }
        });
        let weak = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            if let (Some(this), Ok(files)) = (weak.upgrade(), value.get::<gtk::gdk::FileList>()) {
                this.add_files(files.files())
            } else {
                false
            }
        });
        drop_area.add_controller(target);
        area.append(&drop_area);
        let selected = self.file_queue.borrow();
        if !selected.is_empty() {
            let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
            list.add_css_class("file-list");
            for picked in selected.iter() {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                row.add_css_class("file-row");
                let (icon, class) = match picked.status {
                    FileStatus::Received { .. } => ("emblem-ok-symbolic", "transfer-success"),
                    FileStatus::Failed { .. } => ("dialog-error-symbolic", "transfer-error"),
                    FileStatus::Unconfirmed { .. } => {
                        ("dialog-warning-symbolic", "transfer-warning")
                    }
                    _ => ("text-x-generic-symbolic", "file-symbol"),
                };
                let image = gtk::Image::from_icon_name(icon);
                image.add_css_class(class);
                row.append(&image);
                let labels = gtk::Box::new(gtk::Orientation::Vertical, 4);
                labels.set_hexpand(true);
                let name = text(
                    &picked
                        .file
                        .basename()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "File".into()),
                    "notification-title",
                );
                name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                name.set_wrap(false);
                labels.append(&name);
                let status = text(&picked.status.label(), "card-subtitle");
                if matches!(
                    picked.status,
                    FileStatus::Failed { .. } | FileStatus::Unconfirmed { .. }
                ) {
                    status.add_css_class(class);
                }
                labels.append(&status);
                row.append(&labels);
                let remove = gtk::Button::from_icon_name("window-close-symbolic");
                remove.add_css_class("flat");
                remove.set_tooltip_text(Some(
                    "Remove from list; submitted transfers are not cancelled",
                ));
                remove.set_sensitive(!matches!(picked.status, FileStatus::Submitting { .. }));
                let file = picked.file.clone();
                let weak = Rc::downgrade(self);
                remove.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.file_queue
                            .borrow_mut()
                            .retain(|picked| !picked.file.equal(&file));
                        this.schedule();
                    }
                });
                row.append(&remove);
                list.append(&row);
            }
            area.append(&list);
        }
        let ready = selected
            .iter()
            .filter(|file| matches!(file.status, FileStatus::Ready))
            .count();
        drop(selected);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let selected_id = self.selected.borrow().clone();
        let caption = text(
            &selected_id
                .as_ref()
                .map(|id| format!("To {}", self.model.borrow().name(id)))
                .unwrap_or_else(|| "Choose a phone to send to".into()),
            "card-subtitle",
        );
        caption.set_hexpand(true);
        footer.append(&caption);
        let send = gtk::Button::with_label(&format!(
            "Send {ready} file{}",
            if ready == 1 { "" } else { "s" }
        ));
        if ready == 0 {
            send.set_label("Send files");
        }
        send.set_tooltip_text(Some("Send selected files"));
        send.add_css_class("suggested-action");
        send.set_sensitive(
            ready > 0
                && selected_id
                    .as_ref()
                    .is_some_and(|id| self.can_send_files(id))
                && !self.busy(),
        );
        let weak = Rc::downgrade(self);
        send.connect_clicked(move |button| {
            button.set_sensitive(false);
            if let Some(this) = weak.upgrade() {
                this.send_files();
            }
        });
        footer.append(&send);
        area.append(&footer);
        if selected_id.is_none() {
            let recipients = gtk::FlowBox::new();
            recipients.set_selection_mode(gtk::SelectionMode::None);
            recipients.set_max_children_per_line(3);
            for device in self.model.borrow().devices.values() {
                let button = gtk::Button::with_label(&device.name);
                button.set_sensitive(self.can_send_files(&device.id));
                let weak = Rc::downgrade(self);
                let id = device.id.clone();
                button.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        *this.selected.borrow_mut() = Some(id.clone());
                        this.render();
                    }
                });
                recipients.insert(&button, -1);
            }
            area.append(&recipients);
        }
        area
    }
    fn can_send_files(&self, id: &DeviceId) -> bool {
        let model = self.model.borrow();
        model.ready(id)
            && model
                .devices
                .get(id)
                .is_some_and(|d| d.capabilities.contains(&Capability::FileTransfer))
    }
    pub(super) fn send_files(self: &Rc<Self>) {
        let Some(id) = self.selected.borrow().clone() else {
            return;
        };
        if self.busy() || !self.can_send_files(&id) {
            return;
        }
        let name = self.model.borrow().name(&id);
        let urls = {
            let mut files = self.file_queue.borrow_mut();
            files
                .iter_mut()
                .filter_map(|file| {
                    if matches!(file.status, FileStatus::Ready) {
                        file.status = FileStatus::Submitting {
                            target: name.clone(),
                        };
                        Some(file.file.uri().to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        for url in urls {
            let previous = self.next.get();
            self.request(Request::SendFile(id.clone(), url.clone()));
            let command = self.next.get();
            if (command == previous || !self.pending.borrow().contains_key(&command))
                && let Some(file) = self
                    .file_queue
                    .borrow_mut()
                    .iter_mut()
                    .find(|file| file.file.uri() == url)
            {
                file.status = FileStatus::Failed {
                    reason: "Not submitted. Try again when connected.".into(),
                };
            }
        }
    }
    pub(super) fn file_reply(&self, request: &Request, result: &Result<Reply, Failure>) {
        let Request::SendFile(device, url) = request else {
            return;
        };
        let name = self.model.borrow().name(device);
        if let Some(file) = self
            .file_queue
            .borrow_mut()
            .iter_mut()
            .find(|file| file.file.uri() == *url)
        {
            file.status = match result {
                Ok(Reply::Share(id)) => FileStatus::Accepted {
                    target: name,
                    device: device.clone(),
                    transfer_id: id.clone(),
                },
                Err(failure) if failure.uncertain => FileStatus::Unconfirmed {
                    reason: failure.message.clone(),
                },
                Err(failure) => FileStatus::Failed {
                    reason: failure.message.clone(),
                },
                _ => FileStatus::Unconfirmed {
                    reason: "No acceptance was reported.".into(),
                },
            };
        }
        let results = self
            .model
            .borrow()
            .transfers
            .iter()
            .filter_map(|activity| match &activity.event {
                TransferActivityEvent::Result { result } => Some(result.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        for result in results {
            self.file_result(&result);
        }
    }
    pub(super) fn file_result(&self, result: &ShareResult) {
        for file in self.file_queue.borrow_mut().iter_mut() {
            if let FileStatus::Accepted {
                target,
                device,
                transfer_id: Some(id),
            } = &file.status
                && device == &result.device_id
                && id == &result.transfer_id
            {
                file.status = match result.status {
                    ShareStatus::Completed => FileStatus::Received {
                        target: target.clone(),
                    },
                    ShareStatus::Failed => FileStatus::Failed {
                        reason: result
                            .reason
                            .map(super::pages::failure)
                            .unwrap_or("Phone did not accept the file")
                            .to_string(),
                    },
                };
            }
        }
    }
}
