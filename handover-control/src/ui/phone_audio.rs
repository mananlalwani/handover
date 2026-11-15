use super::*;

impl Window {
    pub(super) fn phone_volume_panel(self: &Rc<Self>, id: &DeviceId) -> gtk::Box {
        let model = self.model.borrow();
        let panel = card("Phone volume", "Levels reported by your phone");
        let ready = model.ready(id);
        let refresh = self.button("Refresh volume", ready, Request::RefreshVolume(id.clone()));
        panel.append(&refresh);
        let Some(state) = model.volumes.get(id).filter(|s| !s.streams.is_empty()) else {
            panel.append(&text(
                if model
                    .devices
                    .get(id)
                    .is_some_and(|d| d.capabilities.contains(&Capability::VolumeControl))
                {
                    "Waiting for the phone's volume report"
                } else {
                    "Update the phone app to control its volume here"
                },
                "card-subtitle",
            ));
            return panel;
        };
        for stream in &state.streams {
            let name = match stream.kind {
                VolumeKind::Media => "Media",
                VolumeKind::Ringtone => "Ringtone",
                VolumeKind::Alarm => "Alarm",
                VolumeKind::Call => "Call",
            };
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let label = text(name, "row-title");
            label.set_width_chars(9);
            row.append(&label);
            let key = (id.clone(), stream.kind);
            let draft = self
                .volume_drafts
                .borrow()
                .get(&key)
                .copied()
                .unwrap_or(stream.level);
            let scale = gtk::Scale::with_range(
                gtk::Orientation::Horizontal,
                f64::from(stream.min_level),
                f64::from(stream.max_level.max(stream.min_level + 1)),
                1.0,
            );
            scale.set_draw_value(false);
            scale.set_hexpand(true);
            scale.set_value(f64::from(draft));
            scale.set_tooltip_text(Some(&format!("{name} volume")));
            scale.set_sensitive(ready && stream.adjustable);
            let weak = Rc::downgrade(self);
            let target = key.clone();
            scale.connect_value_changed(move |s| {
                if let Some(this) = weak.upgrade() {
                    this.volume_drafts
                        .borrow_mut()
                        .insert(target.clone(), s.value().round() as u32);
                }
            });
            row.append(&scale);
            let percent = if stream.muted {
                0
            } else {
                stream.level.saturating_mul(100) / stream.max_level.max(1)
            };
            let level_label = text(&format!("{percent}%"), "card-subtitle");
            level_label.set_wrap(false);
            level_label.set_width_chars(4);
            row.append(&level_label);
            let apply = gtk::Button::with_label("Apply");
            apply.set_tooltip_text(Some(&format!("Set {} volume", name.to_lowercase())));
            apply.set_sensitive(ready && stream.adjustable && !self.busy());
            let weak = Rc::downgrade(self);
            let device = id.clone();
            let kind = stream.kind;
            let control = scale.clone();
            apply.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.request(Request::SetVolume(
                        device.clone(),
                        kind,
                        control.value().round() as u32,
                    ));
                }
            });
            row.append(&apply);
            panel.append(&row);
        }
        if !ready {
            panel.append(&text(
                "Last reported levels · Phone disconnected",
                "card-subtitle",
            ));
        }
        panel
    }

    pub(super) fn call_routing_panel(self: &Rc<Self>) -> gtk::Box {
        let model = self.model.borrow();
        let panel = card(
            "Call audio",
            "Use this computer's microphone and speakers for phone calls",
        );
        let Some(id) = self.selected.borrow().clone() else {
            panel.append(&text("Choose a phone first", "card-subtitle"));
            return panel;
        };
        let Some(controls) = model.call_audio_controls.as_ref() else {
            panel.append(&self.button(
                "Find audio devices",
                model.connected,
                Request::CallAudioControls,
            ));
            return panel;
        };
        if !controls.observed {
            panel.append(&text(
                "PipeWire call-audio devices are unavailable",
                "card-subtitle",
            ));
        }
        if controls.endpoints.is_empty() {
            panel.append(&text("Pair and connect your phone in this computer's Bluetooth settings. A phone with a supported call-audio profile will appear here.","card-subtitle"));
        }
        let draft = self
            .call_audio_draft
            .borrow()
            .as_ref()
            .filter(|(phone, _)| phone == &id)
            .map(|(_, s)| s.clone())
            .or_else(|| {
                controls
                    .route
                    .as_ref()
                    .filter(|r| r.device_id == id)
                    .map(|r| {
                        (
                            r.endpoint_id.clone(),
                            r.input_id.clone(),
                            r.output_id.clone(),
                        )
                    })
            })
            .unwrap_or_default();
        if self
            .call_audio_draft
            .borrow()
            .as_ref()
            .is_none_or(|(phone, _)| phone != &id)
        {
            self.call_audio_draft
                .replace(Some((id.clone(), draft.clone())));
        }
        let columns = [
            ("Bluetooth phone", &controls.endpoints, draft.0.clone()),
            ("Microphone", &controls.inputs, draft.1.clone()),
            ("Speakers", &controls.outputs, draft.2.clone()),
        ];
        let mut dropdowns = Vec::new();
        for (index, (name, choices, selected)) in columns.into_iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let label = text(name, "row-title");
            label.set_width_chars(15);
            row.append(&label);
            let labels = std::iter::once("Choose…".to_string())
                .chain(choices.iter().map(|c| c.label.clone()))
                .collect::<Vec<_>>();
            let borrowed = labels.iter().map(String::as_str).collect::<Vec<_>>();
            let dropdown = gtk::DropDown::from_strings(&borrowed);
            dropdown.set_hexpand(true);
            dropdown.set_selected(
                choices
                    .iter()
                    .position(|c| c.id == selected)
                    .map_or(0, |p| p as u32 + 1),
            );
            dropdown.set_sensitive(model.connected);
            let ids = choices.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
            let weak = Rc::downgrade(self);
            let phone = id.clone();
            dropdown.connect_selected_notify(move |d| {
                if let Some(this) = weak.upgrade() {
                    let value = d
                        .selected()
                        .checked_sub(1)
                        .and_then(|n| ids.get(n as usize))
                        .cloned()
                        .unwrap_or_default();
                    let mut current = this.call_audio_draft.borrow_mut();
                    if current.as_ref().is_none_or(|(p, _)| p != &phone) {
                        *current = Some((phone.clone(), Default::default()));
                    }
                    if let Some((_, fields)) = current.as_mut() {
                        match index {
                            0 => fields.0 = value,
                            1 => fields.1 = value,
                            _ => fields.2 = value,
                        }
                    }
                }
            });
            row.append(&dropdown);
            panel.append(&row);
            dropdowns.push(dropdown);
        }
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let use_laptop = gtk::Button::with_label("Use laptop for calls");
        use_laptop.set_sensitive(
            model.ready(&id)
                && controls.observed
                && !controls.endpoints.is_empty()
                && !controls.inputs.is_empty()
                && !controls.outputs.is_empty()
                && !self.busy(),
        );
        let weak = Rc::downgrade(self);
        let phone = id.clone();
        let snapshot = controls.clone();
        use_laptop.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                let selected = dropdowns
                    .iter()
                    .map(|d| d.selected().checked_sub(1).map(|n| n as usize))
                    .collect::<Vec<_>>();
                let fields = selected[0]
                    .and_then(|i| snapshot.endpoints.get(i))
                    .zip(selected[1].and_then(|i| snapshot.inputs.get(i)))
                    .zip(selected[2].and_then(|i| snapshot.outputs.get(i)));
                if let Some(((endpoint, input), output)) = fields {
                    this.request(Request::ConfigureCallAudio(
                        snapshot.revision,
                        Some(CallAudioSelection {
                            expected_revision: snapshot.revision,
                            device_id: phone.clone(),
                            endpoint_id: endpoint.id.clone(),
                            input_id: input.id.clone(),
                            output_id: output.id.clone(),
                        }),
                    ));
                } else {
                    this.transient_notice(
                        "Choose the Bluetooth phone, microphone, and speakers first.",
                    );
                }
            }
        });
        buttons.append(&use_laptop);
        buttons.append(&self.button(
            "Refresh devices",
            model.connected,
            Request::CallAudioControls,
        ));
        if controls.route.is_some() {
            buttons.append(&self.button(
                "Stop using laptop",
                model.connected && !self.busy(),
                Request::ConfigureCallAudio(controls.revision, None),
            ));
        }
        panel.append(&buttons);
        if let Some(route) = &controls.route {
            let owner = model.name(&route.device_id);
            let status = match &route.state {
                CallAudioRouteState::WaitingForCall => format!(
                    "Ready for calls on {owner}. The microphone is off until a call starts."
                ),
                CallAudioRouteState::Routing {
                    duplex_running: true,
                } => format!("Call audio is running for {owner}"),
                CallAudioRouteState::Routing {
                    duplex_running: false,
                } => format!("Audio routes are connected for {owner}. Waiting for call audio."),
                CallAudioRouteState::Failed { message } => message.clone(),
            };
            panel.append(&text(&status, "body-copy"));
        }
        if let Some(notice) = &controls.notice {
            panel.append(&text(notice, "card-subtitle"));
        }
        panel
    }
}
