use super::*;

impl Window {
    pub(super) fn change_organization(self: &Rc<Self>, index: usize, value: bool) {
        let settings = {
            let model = self.model.borrow();
            let Some(id) = model.selected.clone() else {
                return;
            };
            if !model.connected || !model.settings_loaded || self.pending.borrow().values().any(|request| matches!(request, Request::SaveSettings(settings) if settings.conversation_id == id)) { return; }
            let mut settings = model
                .settings
                .get(&id)
                .cloned()
                .unwrap_or_else(|| ConversationSettings::new(id));
            match index {
                0 => settings.pinned = value,
                1 => settings.archived = value,
                2 => settings.muted = value,
                _ => settings.unread_reminder = value,
            }
            settings
        };
        self.request(Request::SaveSettings(settings));
        self.render_organization();
    }
    pub(super) fn render_organization(&self) {
        self.organization_updating.set(true);
        let model = self.model.borrow();
        let settings = model
            .selected
            .as_ref()
            .and_then(|id| model.settings.get(id));
        let values = settings
            .map(|settings| {
                [
                    settings.pinned,
                    settings.archived,
                    settings.muted,
                    settings.unread_reminder,
                ]
            })
            .unwrap_or([false; 4]);
        for (button, value) in self.organization_buttons.iter().zip(values) {
            button.set_active(value);
        }
        let busy = self.pending.borrow().values().any(|request| matches!(request, Request::SaveSettings(settings) if Some(&settings.conversation_id) == model.selected.as_ref()));
        self.organization_box.set_sensitive(
            model.connected && model.settings_loaded && model.selected.is_some() && !busy,
        );
        self.organization_updating.set(false);
    }
    pub(super) fn clear_open_reminder(&self) {
        let settings = {
            let model = self.model.borrow();
            if !model.settings_loaded {
                return;
            }
            let Some(id) = self.reminder_open.borrow_mut().take() else {
                return;
            };
            if model.selected.as_ref() != Some(&id) {
                return;
            }
            model
                .settings
                .get(&id)
                .filter(|settings| settings.unread_reminder)
                .cloned()
                .map(|mut settings| {
                    settings.unread_reminder = false;
                    settings
                })
        };
        if let Some(settings) = settings {
            self.request(Request::SaveSettings(settings));
        }
    }
    pub fn focus_reply(&self, id: ConversationId) {
        *self.reply_focus.borrow_mut() = Some(id.clone());
        let model = self.model.borrow();
        if model.selected.as_ref() == Some(&id)
            && model.selected_draft().is_some_and(|draft| draft.loaded)
        {
            self.reply_focus.borrow_mut().take();
            self.compose.grab_focus();
        }
    }
}
