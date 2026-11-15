use super::*;

#[derive(Default)]
pub(super) struct Draft {
    pub text: String,
    pub status: String,
}

pub(super) struct Editor {
    pub id: NotificationId,
    pub window: gtk::Window,
    pub input: gtk::TextView,
    pub send: gtk::Button,
    discard: gtk::Button,
    pub status: gtk::Label,
}

impl Window {
    pub(super) fn open_notification_reply(self: &Rc<Self>, id: NotificationId) {
        if let Some(editor) = self.reply_editor.borrow().as_ref() {
            editor.window.present();
            return;
        }
        let model = self.model.borrow();
        let note = model.notifications.get(&id);
        if !note.is_some_and(|n| n.reply_supported) && !self.reply_drafts.borrow().contains_key(&id)
        {
            return;
        }
        if !self.reply_drafts.borrow().contains_key(&id) && self.reply_drafts.borrow().len() >= 16 {
            drop(model);
            self.transient_notice("Finish or discard an existing reply draft first.");
            return;
        }
        let window = gtk::Window::builder()
            .application(self.window.application().as_ref().unwrap())
            .transient_for(&self.window)
            .destroy_with_parent(true)
            .modal(true)
            .title("Reply to notification")
            .default_width(440)
            .default_height(300)
            .build();
        let layout = gtk::Box::new(gtk::Orientation::Vertical, 12);
        layout.set_margin_start(20);
        layout.set_margin_end(20);
        layout.set_margin_top(20);
        layout.set_margin_bottom(20);
        layout.append(&text(
            note.map(|n| n.title.as_str()).unwrap_or("Saved reply"),
            "card-title",
        ));
        layout.append(&text(&model.name(&id.device_id), "card-subtitle"));
        drop(model);
        let input = gtk::TextView::new();
        input.set_wrap_mode(gtk::WrapMode::WordChar);
        input.set_top_margin(10);
        input.set_bottom_margin(10);
        input.set_left_margin(10);
        input.set_right_margin(10);
        if let Some(draft) = self.reply_drafts.borrow().get(&id) {
            input.buffer().set_text(&draft.text);
        }
        let scroll = gtk::ScrolledWindow::builder()
            .child(&input)
            .min_content_height(110)
            .vexpand(true)
            .build();
        scroll.add_css_class("detail-list");
        layout.append(&scroll);
        let status = text("", "card-subtitle");
        layout.append(&status);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let discard = gtk::Button::with_label("Discard draft");
        let close = gtk::Button::with_label("Keep draft");
        let send = gtk::Button::with_label("Send reply");
        send.add_css_class("suggested-action");
        actions.append(&discard);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        actions.append(&spacer);
        actions.append(&close);
        actions.append(&send);
        layout.append(&actions);
        window.set_child(Some(&layout));
        let editor = Rc::new(Editor {
            id: id.clone(),
            window,
            input,
            send,
            discard: discard.clone(),
            status,
        });
        *self.reply_editor.borrow_mut() = Some(editor.clone());
        let weak = Rc::downgrade(self);
        let draft_id = id.clone();
        editor.input.buffer().connect_changed(move |buffer| {
            if let Some(this) = weak.upgrade() {
                let value = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string();
                let mut drafts = this.reply_drafts.borrow_mut();
                if value.is_empty() {
                    drafts.remove(&draft_id);
                } else {
                    drafts.entry(draft_id.clone()).or_default().text = value;
                }
                drop(drafts);
                this.refresh_notification_reply();
            }
        });
        let weak = Rc::downgrade(self);
        let weak_editor = Rc::downgrade(&editor);
        editor.send.connect_clicked(move |_| {
            if let (Some(this), Some(editor)) = (weak.upgrade(), weak_editor.upgrade()) {
                this.refresh_notification_reply();
                if !editor.send.is_sensitive() {
                    return;
                }
                let buffer = editor.input.buffer();
                let value = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string();
                this.request(Request::NotificationReply(editor.id.clone(), value));
                if !this.reply_pending(&editor.id)
                    && let Some(draft) = this.reply_drafts.borrow_mut().get_mut(&editor.id)
                {
                    draft.status = this.model.borrow().notice.clone();
                }
                this.refresh_notification_reply();
            }
        });
        let weak_editor = Rc::downgrade(&editor);
        close.connect_clicked(move |_| {
            if let Some(editor) = weak_editor.upgrade() {
                editor.window.close();
            }
        });
        let weak = Rc::downgrade(self);
        let weak_editor = Rc::downgrade(&editor);
        discard.connect_clicked(move |_| {
            if let (Some(this), Some(editor)) = (weak.upgrade(), weak_editor.upgrade()) {
                if this.reply_pending(&editor.id) {
                    return;
                }
                this.reply_drafts.borrow_mut().remove(&editor.id);
                editor.window.close();
            }
        });
        let weak = Rc::downgrade(self);
        editor.window.connect_close_request(move |_| {
            if let Some(this) = weak.upgrade() {
                this.reply_editor.borrow_mut().take();
                this.schedule();
            }
            glib::Propagation::Proceed
        });
        self.refresh_notification_reply();
        editor.window.present();
        editor.input.grab_focus();
    }

    pub(super) fn notification_drafts(self: &Rc<Self>) {
        let drafts = self.reply_drafts.borrow();
        if drafts.is_empty() {
            return;
        }
        let menu = gtk::MenuButton::builder()
            .label(format!("Reply drafts ({})", drafts.len()))
            .halign(gtk::Align::Start)
            .build();
        let popover = gtk::Popover::new();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        for (id, draft) in drafts.iter() {
            let title = self
                .model
                .borrow()
                .notifications
                .get(id)
                .map(|n| n.title.clone())
                .unwrap_or_else(|| "Unavailable notification".into());
            let row = gtk::Button::new();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 3);
            content.append(&text(
                &format!("{} · {}", title, self.model.borrow().name(&id.device_id)),
                "card-subtitle",
            ));
            let preview = text(&draft.text, "body-copy");
            preview.set_wrap(false);
            preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
            preview.set_max_width_chars(36);
            content.append(&preview);
            row.set_child(Some(&content));
            row.set_tooltip_text(Some(&format!("Reply draft for {title}")));
            row.add_css_class("flat");
            let weak = Rc::downgrade(self);
            let weak_popover = popover.downgrade();
            let id = id.clone();
            row.connect_clicked(move |_| {
                if let Some(popover) = weak_popover.upgrade() {
                    popover.popdown();
                }
                if let Some(this) = weak.upgrade() {
                    this.open_notification_reply(id.clone());
                }
            });
            list.append(&row);
        }
        popover.set_child(Some(&list));
        menu.set_popover(Some(&popover));
        self.body.append(&menu);
    }

    fn reply_pending(&self, id: &NotificationId) -> bool {
        self.pending
            .borrow()
            .values()
            .any(|r| matches!(r, Request::NotificationReply(note, _) if note == id))
    }

    pub(super) fn refresh_notification_reply(&self) {
        let editor = self.reply_editor.borrow().clone();
        let Some(editor) = editor else {
            return;
        };
        let model = self.model.borrow();
        let pending = self.reply_pending(&editor.id);
        let available = model
            .notifications
            .get(&editor.id)
            .is_some_and(|n| n.reply_supported);
        let online = model.ready(&editor.id.device_id);
        let drafts = self.reply_drafts.borrow();
        let draft = drafts.get(&editor.id);
        editor.send.set_sensitive(
            available && online && !self.busy() && draft.is_some_and(|d| !d.text.trim().is_empty()),
        );
        editor.input.set_editable(!pending);
        editor.discard.set_sensitive(!pending);
        editor.status.set_label(if pending {
            "Submitting reply…"
        } else if let Some(draft) = draft.filter(|d| !d.status.is_empty()) {
            &draft.status
        } else if !available {
            "This notification can no longer receive replies. Your draft is kept."
        } else if !online {
            "Phone disconnected. Your draft is kept."
        } else {
            "Enter starts a new line. Send reply submits to this notification."
        });
    }

    pub(super) fn notification_reply_result(
        &self,
        request: &Request,
        result: &Result<Reply, Failure>,
    ) {
        let Request::NotificationReply(id, sent) = request else {
            return;
        };
        match result {
            Ok(Reply::Accepted) => {
                if self
                    .reply_drafts
                    .borrow()
                    .get(id)
                    .is_some_and(|d| d.text == *sent)
                {
                    self.reply_drafts.borrow_mut().remove(id);
                }
                let editor = self.reply_editor.borrow().clone();
                if let Some(editor) = editor.filter(|e| e.id == *id) {
                    editor.window.close();
                }
            }
            Err(error) => {
                if let Some(draft) = self.reply_drafts.borrow_mut().get_mut(id) {
                    draft.status = error.message.clone();
                }
            }
            _ => {}
        }
    }
}
