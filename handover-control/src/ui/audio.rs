use super::*;

impl Window {
    pub(super) fn laptop_audio(self: &Rc<Self>) {
        self.audio_diagnostics.borrow_mut().take();
        let model = self.model.borrow();
        self.body.append(&text("Laptop audio", "list-heading"));
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 14);
        panel.add_css_class("card");
        if let Some(id) = self.selected.borrow().as_ref() {
            if let Some(stream) = model.audio_streams.get(id) {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
                labels.set_hexpand(true);
                labels.append(&text(
                    &format!("Listen on {}", model.name(id)),
                    "card-title",
                ));
                labels.append(&text(
                    match stream.state {
                        AudioStreamPhase::Idle => "Not streaming",
                        AudioStreamPhase::Starting => "Starting",
                        AudioStreamPhase::Streaming => "Receiver output started",
                        AudioStreamPhase::Reconnecting => "Reconnecting",
                        AudioStreamPhase::Suspended => "Suspended",
                        AudioStreamPhase::Failed => "Stream failed",
                    },
                    "body-copy",
                ));
                row.append(&labels);
                let active = stream.enabled == Some(true)
                    || matches!(
                        stream.state,
                        AudioStreamPhase::Starting
                            | AudioStreamPhase::Streaming
                            | AudioStreamPhase::Reconnecting
                            | AudioStreamPhase::Suspended
                    );
                row.append(&self.button(
                    if active {
                        "Stop laptop audio"
                    } else {
                        "Start laptop audio"
                    },
                    model.ready(id),
                    Request::AudioStream(id.clone(), !active),
                ));
                panel.append(&row);
                if let Some(profile) = stream.profile {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    let quality = text("Quality", "card-subtitle");
                    quality.set_wrap(false);
                    row.append(&quality);
                    let segments = gtk::Box::new(gtk::Orientation::Horizontal, 2);
                    segments.add_css_class("quality-selector");
                    for (label, value) in [
                        ("Balanced", AudioProfile::Balanced),
                        ("Music", AudioProfile::Music),
                        ("Low delay", AudioProfile::LowDelay),
                    ] {
                        let button = self.button(
                            label,
                            model.ready(id),
                            Request::AudioProfile(id.clone(), value),
                        );
                        if profile == value {
                            button.add_css_class("selected-choice");
                        }
                        segments.append(&button);
                    }
                    row.append(&segments);
                    panel.append(&row);
                }
                if let Some(failure) = stream.failure {
                    panel.append(&text(
                        match failure {
                            AudioStreamFailure::Disconnected => "Phone disconnected",
                            AudioStreamFailure::ConnectionFailed => {
                                "Could not connect the audio stream"
                            }
                            AudioStreamFailure::CaptureFailed => "Could not capture laptop audio",
                            AudioStreamFailure::StartupTimeout => {
                                "The receiver did not start in time"
                            }
                            AudioStreamFailure::AudioFocus => {
                                "The phone could not obtain audio focus"
                            }
                            AudioStreamFailure::ReceiverFailed => {
                                "The phone's audio receiver failed"
                            }
                            AudioStreamFailure::RetriesExhausted => {
                                "Automatic recovery stopped. Start again to retry."
                            }
                        },
                        "transfer-error",
                    ));
                }
                let details = gtk::Box::new(gtk::Orientation::Vertical, 6);
                let (sender_text, receiver_text) = diagnostic_text(stream);
                let sender = text(&sender_text, "card-subtitle");
                let receiver = text(&receiver_text, "card-subtitle");
                details.append(&sender);
                details.append(&receiver);
                self.audio_diagnostics
                    .replace(Some((id.clone(), sender, receiver)));
                panel.append(&self.remembered_expander(
                    "audio-details",
                    "Audio diagnostics",
                    &details,
                ));
            } else {
                panel.append(&text(
                    "Laptop-audio receiving has not been reported by this phone.",
                    "card-subtitle",
                ));
            }
        } else {
            panel.append(&text(
                "Choose a phone in the sidebar to control its audio receiver.",
                "card-subtitle",
            ));
        }
        self.body.append(&panel);
        if model.audio_error.is_some() {
            self.body.append(&text("Could not refresh laptop routing. Showing the last reported settings, if available.", "transfer-warning"));
        }
        let Some(snapshot) = &model.audio_controls else {
            self.body.append(&text(
                if self
                    .pending
                    .borrow()
                    .values()
                    .any(|r| r.read_key() == Some(4))
                {
                    "Loading laptop routing controls…"
                } else {
                    "Laptop routing controls unavailable. Refresh to try again."
                },
                "card-subtitle",
            ));
            return;
        };
        let routing = gtk::Box::new(gtk::Orientation::Vertical, 14);
        routing.add_css_class("card");
        routing.append(&text(
            "Routing choices apply to all receiving phones.",
            "card-subtitle",
        ));
        let ready = model.connected && snapshot.settings_available && !self.busy();
        if !snapshot.settings_available {
            routing.append(&text(
                "Laptop audio settings are unavailable.",
                "transfer-error",
            ));
            self.body.append(&self.remembered_expander(
                "audio-routing",
                "Laptop audio routing",
                &routing,
            ));
            return;
        }
        let output = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let label = text("Output", "row-title");
        label.set_hexpand(true);
        output.append(&label);
        for (label, mode) in [
            ("Phone", AudioOutputMode::Phone),
            ("Laptop", AudioOutputMode::Laptop),
            ("Both", AudioOutputMode::Both),
        ] {
            let button = self.button(
                label,
                ready,
                Request::ConfigureAudio(AudioRoutingChange {
                    output: Some(mode),
                    ..Default::default()
                }),
            );
            if snapshot.settings.output == mode {
                button.add_css_class("selected-choice");
            }
            output.append(&button);
        }
        routing.append(&output);
        let draft = self
            .audio_source_draft
            .borrow()
            .clone()
            .unwrap_or_else(|| snapshot.settings.source.clone());
        let all = gtk::CheckButton::with_label("All application audio");
        let applications = gtk::CheckButton::with_label("Selected applications");
        applications.set_group(Some(&all));
        all.set_active(matches!(draft, AudioSourceSelection::All));
        applications.set_active(matches!(draft, AudioSourceSelection::Applications { .. }));
        all.set_sensitive(ready);
        applications.set_sensitive(ready && snapshot.applications_available);
        let weak = Rc::downgrade(self);
        all.connect_toggled(move |button| {
            if button.is_active()
                && let Some(this) = weak.upgrade()
                && !this.busy()
            {
                this.set_audio_source(AudioSourceSelection::All);
            }
        });
        let weak = Rc::downgrade(self);
        applications.connect_toggled(move |button| {
            if button.is_active()
                && let Some(this) = weak.upgrade()
                && !this.busy()
            {
                this.set_audio_source(AudioSourceSelection::Applications { ids: vec![] });
            }
        });
        routing.append(&all);
        routing.append(&applications);
        if let AudioSourceSelection::Applications { ids } = &draft {
            let mut choices = snapshot
                .applications
                .iter()
                .map(|a| (a.id.clone(), a.name.clone(), a.available))
                .collect::<Vec<_>>();
            for id in ids {
                if !choices.iter().any(|a| a.0 == *id) {
                    choices.push((id.clone(), "Unavailable application".into(), false));
                }
            }
            for (id, name, available) in choices {
                let check = gtk::CheckButton::with_label(&if available {
                    name
                } else {
                    format!("{name} · Unavailable")
                });
                check.set_active(ids.contains(&id));
                check.set_sensitive(ready && (available || ids.contains(&id)));
                check.set_margin_start(22);
                let weak = Rc::downgrade(self);
                check.connect_toggled(move |check| {
                    if let Some(this) = weak.upgrade()
                        && !this.busy()
                    {
                        // Merge against the live draft so rapid checkbox changes do not overwrite each other.
                        let source = this.audio_source_draft.borrow().clone().or_else(|| {
                            this.model
                                .borrow()
                                .audio_controls
                                .as_ref()
                                .map(|s| s.settings.source.clone())
                        });
                        let mut selected = match source {
                            Some(AudioSourceSelection::Applications { ids }) => {
                                ids.into_iter().collect::<BTreeSet<_>>()
                            }
                            _ => BTreeSet::new(),
                        };
                        if check.is_active() {
                            selected.insert(id.clone());
                        } else {
                            selected.remove(&id);
                        }
                        this.set_audio_source(AudioSourceSelection::Applications {
                            ids: selected.into_iter().collect(),
                        });
                    }
                });
                routing.append(&check);
            }
            if ids.is_empty() {
                routing.append(&text("Select at least one application.", "card-subtitle"));
            }
        }
        let valid_source = match &draft {
            AudioSourceSelection::All => true,
            AudioSourceSelection::Applications { ids } => {
                !ids.is_empty()
                    && snapshot.applications_available
                    && ids.iter().all(|id| {
                        snapshot
                            .applications
                            .iter()
                            .any(|a| a.id == *id && a.available)
                    })
            }
        };
        let apply = gtk::Button::with_label("Apply audio source");
        apply.set_halign(gtk::Align::Start);
        apply.set_sensitive(ready && valid_source && self.audio_source_draft.borrow().is_some());
        let weak = Rc::downgrade(self);
        apply.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade()
                && !this.busy()
            {
                let source = this.audio_source_draft.borrow().clone();
                if let Some(source) = source {
                    this.request(Request::ConfigureAudio(AudioRoutingChange {
                        source: Some(source),
                        ..Default::default()
                    }));
                }
            }
        });
        routing.append(&apply);
        let player_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let label = text("Laptop playback controls", "row-title");
        label.set_hexpand(true);
        player_row.append(&label);
        let mut players = vec![("Automatic".to_owned(), AudioPlaybackTarget::Automatic)];
        players.extend(snapshot.players.iter().map(|p| {
            (
                p.name.clone(),
                AudioPlaybackTarget::Player { id: p.id.clone() },
            )
        }));
        if !players.iter().any(|p| p.1 == snapshot.settings.playback) {
            players.push(("Saved player".into(), snapshot.settings.playback.clone()));
        }
        let strings = players.iter().map(|p| p.0.as_str()).collect::<Vec<_>>();
        let picker = gtk::DropDown::from_strings(&strings);
        picker.set_selected(
            players
                .iter()
                .position(|p| p.1 == snapshot.settings.playback)
                .unwrap_or(0) as u32,
        );
        picker.set_sensitive(ready);
        picker.update_property(&[gtk::accessible::Property::Label("Laptop playback controls")]);
        let weak = Rc::downgrade(self);
        let current = snapshot.settings.playback.clone();
        let valid_players = snapshot
            .players
            .iter()
            .map(|p| p.id.clone())
            .collect::<BTreeSet<_>>();
        picker.connect_selected_notify(move |picker| {
            if let Some(this) = weak.upgrade()
                && !this.busy()
                && !this.rendering.get()
                && let Some((_, target)) = players.get(picker.selected() as usize)
                && target != &current
            {
                if matches!(target,AudioPlaybackTarget::Player {id} if !valid_players.contains(id))
                {
                    return;
                }
                this.request(Request::ConfigureAudio(AudioRoutingChange {
                    playback: Some(target.clone()),
                    ..Default::default()
                }));
            }
        });
        player_row.append(&picker);
        routing.append(&player_row);
        if !snapshot.players_available {
            routing.append(&text(
                "Laptop player discovery is unavailable.",
                "card-subtitle",
            ));
        }
        self.body.append(&self.remembered_expander(
            "audio-routing",
            "Laptop audio routing",
            &routing,
        ));
    }

    pub(super) fn update_audio_diagnostics(&self) {
        if let Some((id, sender, receiver)) = self.audio_diagnostics.borrow().as_ref()
            && let Some(stream) = self.model.borrow().audio_streams.get(id)
        {
            let (sender_text, receiver_text) = diagnostic_text(stream);
            sender.set_label(&sender_text);
            receiver.set_label(&receiver_text);
        }
    }

    fn set_audio_source(self: &Rc<Self>, source: AudioSourceSelection) {
        if self.rendering.get() {
            return;
        }
        let unchanged = self
            .model
            .borrow()
            .audio_controls
            .as_ref()
            .is_some_and(|s| s.settings.source == source);
        self.audio_source_draft
            .replace(if unchanged { None } else { Some(source) });
        self.schedule();
    }
}

fn diagnostic_text(stream: &AudioStreamStatus) -> (String, String) {
    let diagnostics = stream.diagnostics.as_ref();
    let sender = diagnostics
        .and_then(|d| d.sender.as_ref())
        .map(|s| {
            format!(
                "Sender: {} kb/s · {} ms packets · {} dropped{}",
                s.encoder_bitrate_bps / 1000,
                s.packet_duration_ms,
                s.capture_dropped_packets,
                sample_time(s.updated_at_ms)
            )
        })
        .unwrap_or_else(|| "Sender measurements not reported".into());
    let receiver = diagnostics
        .and_then(|d| d.receiver.as_ref())
        .map(|r| {
            let route = match r.route {
                AudioRoute::Direct => "Direct",
                AudioRoute::Vpn => "VPN",
                AudioRoute::Relay => "Relay",
                AudioRoute::Unknown => "Not reported",
            };
            let mut values = vec![format!("Receiver route: {route}")];
            if let Some(value) = r.transport {
                values.push(
                    match value {
                        AudioTransport::Udp => "UDP",
                        AudioTransport::Tcp => "TCP",
                    }
                    .into(),
                );
            }
            if let Some(value) = r.round_trip_ms {
                values.push(format!("{value} ms round trip"));
            }
            if let Some(value) = r.jitter_ms {
                values.push(format!("{value} ms jitter"));
            }
            if let Some(value) = r.jitter_buffer_ms {
                values.push(format!("{value} ms buffer"));
            }
            if let Some(value) = r.bitrate_bps {
                values.push(format!("{} kb/s", value / 1000));
            }
            if let Some(value) = r.packets_received {
                values.push(format!("{value} packets received"));
            }
            if let Some(value) = r.packets_lost {
                values.push(format!("{value} packets lost"));
            }
            format!("{}{}", values.join(" · "), sample_time(r.updated_at_ms))
        })
        .unwrap_or_else(|| "Receiver measurements not reported".into());
    (sender, receiver)
}

fn sample_time(milliseconds: i64) -> String {
    if milliseconds <= 0 {
        return String::new();
    }
    glib::DateTime::from_unix_local(milliseconds / 1000)
        .ok()
        .and_then(|d| d.format("%H:%M:%S").ok())
        .map(|s| format!(" · Reported at {s}"))
        .unwrap_or_default()
}
