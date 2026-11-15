use super::*;

impl Window {
    pub(super) fn remembered_expander(
        self: &Rc<Self>,
        key: &'static str,
        title: &str,
        child: &impl IsA<gtk::Widget>,
    ) -> gtk::Expander {
        let expander = gtk::Expander::new(Some(title));
        expander.set_expanded(self.expanded_sections.borrow().contains(key));
        expander.set_child(Some(child));
        expander.add_css_class("advanced-section");
        let weak = Rc::downgrade(self);
        expander.connect_expanded_notify(move |expander| {
            if let Some(this) = weak.upgrade() {
                if expander.is_expanded() {
                    this.expanded_sections.borrow_mut().insert(key);
                } else {
                    this.expanded_sections.borrow_mut().remove(key);
                }
            }
        });
        expander
    }

    pub(super) fn remote_input(self: &Rc<Self>, id: &DeviceId) -> gtk::Expander {
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
        panel.add_css_class("card");
        panel.append(&text(
            "Input access must be allowed on the phone.",
            "card-subtitle",
        ));
        let ready = self.model.borrow().ready(id);
        let directions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for (label, action, x, y, button) in [
            ("Move left", RemoteInputAction::Move, -30, 0, 0),
            ("Move up", RemoteInputAction::Move, 0, -30, 0),
            ("Move down", RemoteInputAction::Move, 0, 30, 0),
            ("Move right", RemoteInputAction::Move, 30, 0, 0),
        ] {
            let command = RemoteInputCommand {
                device_id: id.clone(),
                action,
                delta_x: x,
                delta_y: y,
                button,
                text: None,
            };
            directions.append(&self.button(label, ready, Request::RemoteInput(command)));
        }
        panel.append(&directions);
        let clicks = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for (label, action, y, button) in [
            ("Left click", RemoteInputAction::Click, 0, 1),
            ("Right click", RemoteInputAction::Click, 0, 3),
            ("Scroll up", RemoteInputAction::Scroll, -1, 0),
            ("Scroll down", RemoteInputAction::Scroll, 1, 0),
        ] {
            clicks.append(&self.button(
                label,
                ready,
                Request::RemoteInput(RemoteInputCommand {
                    device_id: id.clone(),
                    action,
                    delta_x: 0,
                    delta_y: y,
                    button,
                    text: None,
                }),
            ));
        }
        panel.append(&clicks);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        entry.set_max_length(512);
        entry.set_placeholder_text(Some("Text to type on the phone"));
        entry.set_text(
            self.remote_drafts
                .borrow()
                .get(id)
                .map(String::as_str)
                .unwrap_or_default(),
        );
        let send = gtk::Button::with_label("Type text");
        send.set_sensitive(
            ready && !self.busy() && !entry.text().is_empty() && entry.text().len() <= 512,
        );
        let weak = Rc::downgrade(self);
        let target = id.clone();
        let weak_send = send.downgrade();
        entry.connect_changed(move |entry| {
            if let Some(this) = weak.upgrade() {
                let value = entry.text().to_string();
                if let Some(send) = weak_send.upgrade() {
                    send.set_sensitive(
                        this.model.borrow().ready(&target)
                            && !this.busy()
                            && !value.is_empty()
                            && value.len() <= 512,
                    );
                }
                if value.is_empty() {
                    this.remote_drafts.borrow_mut().remove(&target);
                } else {
                    this.remote_drafts
                        .borrow_mut()
                        .insert(target.clone(), value);
                }
            }
        });
        let weak = Rc::downgrade(self);
        let target = id.clone();
        send.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade()
                && !this.busy()
                && this.model.borrow().ready(&target)
            {
                let value = this
                    .remote_drafts
                    .borrow()
                    .get(&target)
                    .cloned()
                    .unwrap_or_default();
                if !value.is_empty() && value.len() <= 512 {
                    this.request(Request::RemoteInput(RemoteInputCommand {
                        device_id: target.clone(),
                        action: RemoteInputAction::Type,
                        delta_x: 0,
                        delta_y: 0,
                        button: 0,
                        text: Some(value),
                    }));
                }
            }
        });
        row.append(&entry);
        row.append(&send);
        panel.append(&row);
        self.remembered_expander("remote", "Remote input", &panel)
    }

    pub(super) fn desktop_tools(self: &Rc<Self>) {
        if let Some(id) = self.selected.borrow().clone() {
            let panel = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            panel.add_css_class("card");
            let label = text(
                &format!(
                    "Send a connection test to {}",
                    self.model.borrow().name(&id)
                ),
                "body-copy",
            );
            label.set_hexpand(true);
            panel.append(&label);
            panel.append(&self.button(
                "Ping phone",
                self.model.borrow().ready(&id),
                Request::Phone(id, DeviceCommandAction::Ping, None),
            ));
            self.body.append(&self.remembered_expander(
                "connection-test",
                "Phone connection test",
                &panel,
            ));
        }
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
        panel.add_css_class("card");
        panel.append(&text("Desktop sleep policy", "card-title"));
        panel.append(&text("Automatic follows Handover's connected-device policy. These controls affect this computer.", "card-subtitle"));
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        for (label, policy) in [
            ("Automatic", None),
            ("Keep desktop awake", Some(true)),
            ("Allow desktop sleep", Some(false)),
        ] {
            controls.append(&self.button(
                label,
                self.model.borrow().connected,
                Request::Screensaver(policy),
            ));
        }
        panel.append(&controls);
        self.body
            .append(&self.remembered_expander("desktop-sleep", "Desktop behavior", &panel));
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
        panel.add_css_class("card");
        panel.append(&text(
            "Commands run on this computer using its configured allowlist.",
            "card-subtitle",
        ));
        let model = self.model.borrow();
        match &model.commands {
            None => panel.append(&text(
                "Refresh to load configured commands.",
                "card-subtitle",
            )),
            Some(commands) if commands.is_empty() => {
                panel.append(&text("No commands configured.", "card-subtitle"))
            }
            Some(commands) => {
                for command in commands {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                    let name = text(&command.name, "body-copy");
                    name.set_hexpand(true);
                    row.append(&name);
                    let run = self.button(
                        "Run",
                        model.connected,
                        Request::RunCommand(command.name.clone()),
                    );
                    run.set_tooltip_text(Some(&format!("Run {} on this computer", command.name)));
                    row.append(&run);
                    panel.append(&row);
                }
            }
        }
        if let Some(result) = &model.command_result {
            let outcome = if result.accepted {
                format!(
                    "Finished{}",
                    result
                        .exit_code
                        .map(|c| format!(" · Exit {c}"))
                        .unwrap_or_default()
                )
            } else {
                match result.failure {
                    Some(CustomCommandFailure::UnknownCommand) => "No longer configured".into(),
                    Some(CustomCommandFailure::SpawnFailed) => "Could not start".into(),
                    Some(CustomCommandFailure::TimedOut) => "Timed out".into(),
                    None => "Rejected".into(),
                }
            };
            panel.append(&text(
                &format!("{} · {outcome}", result.name),
                if result.accepted && result.exit_code == Some(0) {
                    "card-subtitle"
                } else {
                    "transfer-error"
                },
            ));
        }
        self.body
            .append(&self.remembered_expander("commands", "Configured commands", &panel));
    }
}
