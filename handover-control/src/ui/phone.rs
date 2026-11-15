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
        self.body.append(&self.call_routing_panel());
        self.body.append(&self.phone_volume_panel(&id));
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
}
