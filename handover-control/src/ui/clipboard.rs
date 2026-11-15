use super::*;

impl Window {
    pub(super) fn clipboard_history(self: &Rc<Self>) {
        let model = self.model.borrow();
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let scope = text("History on this computer · All devices", "card-subtitle");
        scope.set_hexpand(true);
        toolbar.append(&scope);
        let clear = gtk::Button::with_label("Clear history…");
        clear.set_sensitive(
            model.connected
                && !self.busy()
                && model
                    .clipboard_history
                    .as_ref()
                    .is_some_and(|v| !v.is_empty()),
        );
        let weak = Rc::downgrade(self);
        clear.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.clear_clipboard_dialog();
            }
        });
        toolbar.append(&clear);
        self.body.append(&toolbar);
        let Some(entries) = &model.clipboard_history else {
            self.body
                .append(&text("Refresh to load clipboard history.", "card-subtitle"));
            return;
        };
        let query = self.search.text().to_lowercase();
        let mut visible = entries
            .iter()
            .filter(|e| e.text.to_lowercase().contains(&query))
            .collect::<Vec<_>>();
        visible.sort_by_key(|e| {
            (
                std::cmp::Reverse(e.pinned),
                std::cmp::Reverse(e.created_at_ms),
                std::cmp::Reverse(e.id),
            )
        });
        if visible.is_empty() {
            self.body.append(&empty(
                "edit-paste-symbolic",
                if query.is_empty() {
                    "No clipboard history"
                } else {
                    "No results"
                },
                if query.is_empty() {
                    "Received clipboard text appears here."
                } else {
                    "Try a different search."
                },
            ));
            return;
        }
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("detail-list");
        for entry in visible.into_iter().take(100) {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
            row.add_css_class("notification-row");
            let value = text(&entry.text, "body-copy");
            value.set_selectable(true);
            value.set_lines(5);
            value.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&value);
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let date = glib::DateTime::from_unix_local(
                (entry.created_at_ms / 1000).min(i64::MAX as u64) as i64,
            )
            .ok()
            .and_then(|d| d.format("%b %e · %H:%M").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
            let label = text(&date, "card-subtitle");
            label.set_hexpand(true);
            actions.append(&label);
            let pin = self.button(
                if entry.pinned { "Unpin" } else { "Pin" },
                model.connected,
                Request::PinClipboard(entry.id, !entry.pinned),
            );
            pin.add_css_class("flat");
            actions.append(&pin);
            let copy = gtk::Button::with_label("Copy");
            let clipboard = self.window.clipboard();
            let value = entry.text.clone();
            copy.connect_clicked(move |_| clipboard.set_text(&value));
            actions.append(&copy);
            row.append(&actions);
            list.append(&row);
        }
        self.body.append(&list);
    }

    fn clear_clipboard_dialog(self: &Rc<Self>) {
        let dialog = adw::MessageDialog::new(
            Some(&self.window),
            Some("Clear clipboard history?"),
            Some("Pinned entries will be kept unless you include them below."),
        );
        dialog.set_application(self.window.application().as_ref());
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("clear", "Clear history");
        dialog.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let pinned = gtk::CheckButton::with_label("Also remove pinned entries");
        dialog.set_extra_child(Some(&pinned));
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |dialog, response| {
            if response == "clear"
                && let Some(this) = weak.upgrade()
            {
                this.request(Request::ClearClipboard(pinned.is_active()));
            }
            dialog.close();
        });
        dialog.present();
    }
}
