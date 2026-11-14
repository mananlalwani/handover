use super::*;

impl Window {
    pub(super) fn reconcile_text_sends(&self) {
        let ready = {
            let model = self.model.borrow();
            if !model.connected {
                return;
            }
            model
                .drafts
                .values()
                .filter(|draft| {
                    draft.loaded
                        && draft.error.is_empty()
                        && draft
                            .value
                            .pending_operation
                            .as_ref()
                            .and_then(|id| model.outgoing.get(id))
                            .is_some_and(|operation| {
                                matches!(
                                    operation.outcome,
                                    OutgoingOutcome::Provider(
                                        MessageStatus::Sent
                                            | MessageStatus::Delivered
                                            | MessageStatus::Displayed
                                    )
                                )
                            })
                })
                .map(|draft| draft.value.clone())
                .collect::<Vec<_>>()
        };
        for draft in ready {
            if !self.pending.borrow().values().any(|request| matches!(request, Request::ResolveDraftSend(current, _) if current.conversation_id == draft.conversation_id)) {
                self.request(Request::ResolveDraftSend(draft, false));
            }
        }
    }

    pub(super) fn render_send_activity(self: &Rc<Self>) {
        while let Some(child) = self.outbox.first_child() {
            self.outbox.remove(&child);
        }
        let model = self.model.borrow();
        let submitting = self
            .pending
            .borrow()
            .values()
            .find_map(|request| match request {
                Request::Send(draft) if Some(&draft.conversation_id) == model.selected.as_ref() => {
                    Some(draft.clone())
                }
                _ => None,
            });
        let loaded = model.selected_draft().is_some_and(|draft| draft.loaded);
        let draft = model.selected_draft().map(|draft| draft.value.clone());
        let Some(draft) = draft
            .filter(|draft| draft.pending_operation.is_some())
            .or_else(|| submitting.clone())
        else {
            self.outbox.set_visible(false);
            return;
        };
        self.outbox.set_visible(true);
        let operation = draft
            .pending_operation
            .as_ref()
            .and_then(|id| model.outgoing.get(id));
        let outcome = operation
            .map(|operation| &operation.outcome)
            .or_else(|| submitting.as_ref().map(|_| &OutgoingOutcome::Submitting));
        let pending = matches!(
            outcome,
            Some(OutgoingOutcome::Submitting | OutgoingOutcome::Provider(MessageStatus::Accepted))
        );
        let failed = matches!(
            outcome,
            Some(OutgoingOutcome::Rejected | OutgoingOutcome::Provider(MessageStatus::Failed(_)))
        );
        let sent = matches!(
            outcome,
            Some(OutgoingOutcome::Provider(
                MessageStatus::Sent | MessageStatus::Delivered | MessageStatus::Displayed
            ))
        );
        let has_message = operation
            .and_then(|operation| operation.message_id.as_ref())
            .is_some_and(|id| model.messages.iter().any(|message| &message.id == id));
        let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
        row.set_halign(gtk::Align::End);
        if !has_message {
            let bubble = gtk::Box::new(gtk::Orientation::Vertical, 6);
            bubble.add_css_class("message-bubble");
            bubble.add_css_class("outgoing");
            bubble.add_css_class("submitted-message");
            bubble.set_halign(gtk::Align::End);
            if let Some(target) = &draft.reply_to {
                let reply = gtk::Label::new(Some(&self.quote_label(&model, target)));
                reply.set_wrap(true);
                reply.set_max_width_chars(42);
                reply.set_lines(2);
                reply.set_ellipsize(gtk::pango::EllipsizeMode::End);
                reply.set_xalign(0.0);
                reply.add_css_class("caption");
                bubble.append(&reply);
            }
            let text = gtk::Label::new(Some(&draft.text));
            text.add_css_class("message-text");
            text.set_xalign(0.0);
            text.set_wrap(true);
            text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            text.set_max_width_chars(48);
            text.set_lines(4);
            text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.set_selectable(true);
            bubble.append(&text);
            row.append(&bubble);
        }
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.set_halign(gtk::Align::End);
        if pending {
            let spinner = gtk::Spinner::new();
            spinner.start();
            footer.append(&spinner);
        }
        let label = gtk::Label::new(Some(match outcome {
            _ if pending && !model.connected => "Reconnecting to check send status…",
            Some(OutgoingOutcome::Submitting) => "Submitting message…",
            Some(OutgoingOutcome::Provider(MessageStatus::Accepted)) => {
                "Waiting for send confirmation…"
            }
            _ if failed => "Couldn't send. Your message is kept.",
            _ if sent => "Sent",
            _ => "Send outcome unknown. Check the conversation before retrying.",
        }));
        label.add_css_class("send-state");
        label.set_wrap(true);
        label.set_max_width_chars(42);
        if failed {
            label.add_css_class("error");
        }
        footer.append(&label);
        if !pending && !sent {
            let resolving = self.pending.borrow().values().any(|request| matches!(request, Request::ResolveDraftSend(current, _) if current.conversation_id == draft.conversation_id));
            if failed {
                let retry = gtk::Button::with_label("Retry");
                retry.add_css_class("retry-send");
                retry.set_sensitive(model.connected && loaded && !resolving);
                let weak = Rc::downgrade(self);
                let draft = draft.clone();
                retry.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.request(Request::ResolveDraftSend(draft.clone(), true));
                        this.render_send_activity();
                    }
                });
                footer.append(&retry);
            }
            let edit = gtk::Button::with_label("Keep editing");
            edit.add_css_class("edit-send");
            edit.set_sensitive(model.connected && loaded && !resolving);
            let weak = Rc::downgrade(self);
            edit.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.request(Request::ResolveDraftSend(draft.clone(), false));
                    this.render_send_activity();
                }
            });
            footer.append(&edit);
        }
        row.append(&footer);
        self.outbox.append(&row);
    }
}
