use super::*;

impl Window {
    pub(super) fn review_failed_message(self: &Rc<Self>, id: &MessageId) {
        let bound = {
            let model = self.model.borrow();
            model
                .selected_draft()
                .filter(|draft| {
                    model.selected.as_ref() == Some(&id.conversation_id)
                        && draft.loaded
                        && model.message_failure(id).is_some()
                        && draft
                            .value
                            .pending_operation
                            .as_ref()
                            .and_then(|operation| model.outgoing.get(operation))
                            .is_some_and(|operation| {
                                operation.kind == OutgoingOperationKind::Text
                                    && operation.message_id.as_ref() == Some(id)
                            })
                })
                .map(|draft| draft.value.clone())
        };
        if let Some(draft) = bound {
            self.request(Request::ResolveDraftSend(draft, false));
            self.render_send_activity();
            return;
        }
        let restored = {
            let model = self.model.borrow();
            if model.selected.as_ref() != Some(&id.conversation_id)
                || model.message_failure(id).is_none()
            {
                return;
            }
            let Some(message) = model.messages.iter().find(|message| {
                message.id == *id
                    && message.sender.is_self
                    && !message.deleted
                    && message.attachments.is_empty()
            }) else {
                return;
            };
            let Some(text) = message.text.as_ref().filter(|text| !text.trim().is_empty()) else {
                return;
            };
            let available = model.selected_draft().is_some_and(|draft| {
                draft.loaded && !draft.saving && !draft.conflict && draft.error.is_empty()
                    && draft.value.pending_operation.is_none()
                    && draft.value.text.is_empty() && draft.value.reply_to.is_none()
            }) && self.attachment_state.borrow().contains_key(&id.conversation_id)
                && !self.attachments.borrow().contains_key(&id.conversation_id)
                && !self.attachment_loading(&id.conversation_id)
                && !self.attachment_busy(&id.conversation_id)
                && !self.attachment_saving(&id.conversation_id)
                && !self.attachment_errors.borrow().contains_key(&id.conversation_id)
                && !self.unsaved_attachment(&id.conversation_id)
                && !self.paste_pending.get()
                && !self.pending.borrow().values().any(|request| matches!(request,
                    Request::Send(draft) | Request::Attachment(draft, ..) | Request::ResolveDraftSend(draft, _)
                        if draft.conversation_id == id.conversation_id));
            available.then(|| (text.clone(), message.reply_to.clone()))
        };
        if let Some((text, reply)) = restored {
            self.model
                .borrow_mut()
                .edit(id.conversation_id.clone(), text, reply);
            self.model.borrow_mut().notice =
                "Message restored for review. Press Send when ready.".into();
            self.render_editor();
            self.save_drafts();
            self.compose.grab_focus();
        } else {
            self.model.borrow_mut().notice =
                "Finish or clear your current draft before restoring this message.".into();
        }
        self.render_status();
    }

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
            Some(OutgoingOutcome::Provider(MessageStatus::Failed(reason))) => {
                crate::model::failure_label(*reason)
            }
            _ if failed => "Couldn't send. Your message is kept.",
            _ if sent => "Sent",
            _ => "Send outcome unknown. Check the conversation before retrying.",
        }));
        label.add_css_class("send-state");
        if let Some(OutgoingOutcome::Provider(MessageStatus::Failed(reason))) = outcome {
            label.set_tooltip_text(Some(crate::model::failure_detail(*reason)));
        }
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
