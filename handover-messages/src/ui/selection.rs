use super::*;

impl Window {
    pub(super) fn end_selection(&self) {
        self.selection_mode.set(false);
        self.selected_messages.borrow_mut().clear();
        self.render_selection();
    }
    pub(super) fn select_message(&self, id: &MessageId, selected: bool) {
        let model = self.model.borrow();
        if Some(&id.conversation_id) != model.selected.as_ref()
            || !model
                .messages
                .iter()
                .any(|message| &message.id == id && !message.deleted)
        {
            return;
        }
        drop(model);
        if selected {
            if self.selected_messages.borrow().len() >= 100 {
                return;
            }
            self.selected_messages.borrow_mut().insert(id.clone());
        } else {
            self.selected_messages.borrow_mut().remove(id);
        }
        self.render_selection();
    }
    pub(super) fn select_all_messages(&self) {
        self.selection_mode.set(true);
        *self.selected_messages.borrow_mut() = self
            .model
            .borrow()
            .messages
            .iter()
            .filter(|message| !message.deleted)
            .take(100)
            .map(|message| message.id.clone())
            .collect();
        self.render_selection();
    }
    pub(super) fn render_selection(&self) {
        let model = self.model.borrow();
        self.selected_messages.borrow_mut().retain(|id| {
            model
                .messages
                .iter()
                .any(|message| &message.id == id && !message.deleted)
        });
        let selected = self.selected_messages.borrow();
        let count = selected.len();
        self.selection_bar.set_visible(self.selection_mode.get());
        self.selection_count.set_label(&format!("{count} selected"));
        self.selection_count.set_tooltip_text(Some(
            "Select up to 100 messages from the loaded view. Ctrl+A selects them; Escape finishes.",
        ));
        self.selection_copy.set_sensitive(count > 0);
        let files = model
            .messages
            .iter()
            .filter(|message| selected.contains(&message.id))
            .map(|message| message.attachments.len())
            .sum::<usize>();
        self.selection_save.set_sensitive(files > 0);
        self.selection_save.set_tooltip_text(Some(&format!(
            "Save {files} attachments. Missing local files are skipped; up to 64 files per export."
        )));
        self.selection_updating.set(true);
        let mut child = self.thread.first_child();
        for id in self.rendered.borrow().iter() {
            let Some(row) = child else {
                break;
            };
            child = row.next_sibling();
            update_selection(&row, self.selection_mode.get(), selected.contains(id));
        }
        self.selection_updating.set(false);
    }
    pub(super) fn copy_selected_messages(&self) {
        let model = self.model.borrow();
        let selected = self.selected_messages.borrow();
        let text = model
            .messages
            .iter()
            .filter(|message| selected.contains(&message.id) && !message.deleted)
            .map(|message| {
                let name = if message.sender.is_self {
                    "You".into()
                } else {
                    self.photos.borrow().label(&message.sender)
                };
                let mut text = format!("{name}: {}", message.text.as_deref().unwrap_or(""));
                for attachment in &message.attachments {
                    text.push_str(&format!(
                        "\n[{}]",
                        attachment.name.as_deref().unwrap_or("Attachment")
                    ));
                }
                text
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        if !text.is_empty() {
            self.window.clipboard().set_text(&text);
        }
    }
    pub(super) fn save_selected_files(&self) {
        let model = self.model.borrow();
        let selected = self.selected_messages.borrow();
        let files = model
            .messages
            .iter()
            .filter(|message| selected.contains(&message.id) && !message.deleted)
            .flat_map(|message| message.attachments.iter().cloned())
            .collect::<Vec<_>>();
        if files.len() > 64 {
            drop(model);
            self.model.borrow_mut().notice = "Select at most 64 attachments for one export.".into();
            self.render_status();
            return;
        }
        crate::file_actions::save_many(&self.window, &files);
    }
    pub(super) fn show_shortcuts(&self) {
        let window = gtk::Window::builder()
            .application(&self.window.application().unwrap())
            .title("Keyboard shortcuts")
            .transient_for(&self.window)
            .default_width(560)
            .default_height(620)
            .build();
        let grid = gtk::Grid::builder()
            .row_spacing(12)
            .column_spacing(24)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(24)
            .margin_end(24)
            .build();
        for (row, (key, action)) in [
            ("Ctrl+K", "Search all messages and contacts"),
            ("Ctrl+F", "Find in this conversation"),
            ("Alt+Up / Alt+Down", "Previous / next conversation"),
            ("Alt+Left", "Return from message context"),
            ("Ctrl+Shift+M", "Select messages"),
            ("Ctrl+A", "Select loaded messages while selecting"),
            ("Escape", "Finish message selection"),
            ("Enter / Ctrl+Enter", "Send the current draft"),
            ("Shift+Enter", "New line in the composer"),
        ]
        .into_iter()
        .enumerate()
        {
            let label = gtk::Label::new(Some(key));
            label.set_xalign(0.0);
            label.add_css_class("monospace");
            let description = gtk::Label::new(Some(action));
            description.set_xalign(0.0);
            description.set_wrap(true);
            grid.attach(&label, 0, row as i32, 1, 1);
            grid.attach(&description, 1, row as i32, 1, 1);
        }
        window.set_child(Some(&grid));
        let keys = gtk::EventControllerKey::new();
        let weak = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(window) = weak.upgrade() {
                    window.close();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(keys);
        window.present();
    }
}

fn update_selection(widget: &gtk::Widget, mode: bool, selected: bool) {
    if widget.has_css_class("message-selection")
        && let Some(button) = widget.downcast_ref::<gtk::CheckButton>()
    {
        button.set_visible(mode);
        button.set_active(selected);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        update_selection(&widget, mode, selected);
        child = widget.next_sibling();
    }
}
