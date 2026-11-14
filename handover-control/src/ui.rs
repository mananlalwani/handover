use crate::{
    connection::{self, Command, Event, Reply, Request},
    model::Model,
};
use adw::prelude::*;
use gtk::{gio, glib};
use handover_core::*;
use handover_ipc::ServerPayload;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

mod pages;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Page {
    Overview,
    Notifications,
    Sharing,
    Contacts,
    Media,
    Connection,
}
impl Page {
    fn name(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Notifications => "Notifications",
            Self::Sharing => "Sharing",
            Self::Contacts => "Contacts",
            Self::Media => "Media",
            Self::Connection => "Connection",
        }
    }
}
pub struct Window {
    pub window: gtk::ApplicationWindow,
    model: RefCell<Model>,
    selected: RefCell<Option<DeviceId>>,
    page: Cell<Page>,
    devices: gtk::ListBox,
    device_rows: RefCell<Vec<Option<DeviceId>>>,
    body: gtk::Box,
    heading: gtk::Label,
    status: gtk::Label,
    search: gtk::SearchEntry,
    refresh: gtk::Button,
    commands: tokio::sync::mpsc::Sender<Command>,
    pending: RefCell<BTreeMap<u64, Request>>,
    refresh_again: RefCell<BTreeSet<u8>>,
    next: Cell<u64>,
    rendering: Cell<bool>,
    scheduled: Cell<bool>,
}
impl Window {
    pub fn new(application: &adw::Application) -> Rc<Self> {
        let provider = gtk::CssProvider::new();
        provider.load_from_data(include_str!("style.css"));
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let window = gtk::ApplicationWindow::builder()
            .application(application)
            .title("Handover")
            .default_width(1060)
            .default_height(740)
            .build();
        window.add_css_class("control");
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let drag = gtk::WindowHandle::new();
        drag.set_child(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
        drag.set_hexpand(true);
        controls.append(&drag);
        controls.append(&gtk::WindowControls::new(gtk::PackType::End));
        window.set_titlebar(Some(&controls));
        let rail = gtk::Box::new(gtk::Orientation::Vertical, 4);
        rail.set_width_request(215);
        rail.add_css_class("device-rail");
        let brand = gtk::Label::new(Some("Handover"));
        brand.set_xalign(0.0);
        brand.add_css_class("brand");
        rail.append(&brand);
        let devices = gtk::ListBox::new();
        rail.append(&devices);
        let caption = gtk::Label::new(Some("WORKSPACE"));
        caption.set_xalign(0.0);
        caption.add_css_class("section-caption");
        rail.append(&caption);
        let navigation = gtk::ListBox::new();
        rail.append(&navigation);
        let pages = [
            Page::Overview,
            Page::Notifications,
            Page::Sharing,
            Page::Contacts,
            Page::Media,
            Page::Connection,
        ];
        for page in pages {
            let label = gtk::Label::new(Some(page.name()));
            label.set_xalign(0.0);
            navigation.append(&label);
        }
        navigation.select_row(navigation.row_at_index(0).as_ref());
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_vexpand(true);
        rail.append(&spacer);
        let messages = gtk::Button::with_label("Open Messages");
        rail.append(&messages);
        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main.set_hexpand(true);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("page-header");
        let heading = gtk::Label::new(Some("Overview"));
        heading.set_xalign(0.0);
        heading.set_hexpand(true);
        heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
        heading.add_css_class("page-title");
        header.append(&heading);
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some("Refresh current view"));
        header.append(&refresh);
        main.append(&header);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search this view"));
        search.set_margin_top(12);
        search.set_margin_start(24);
        search.set_margin_end(24);
        main.append(&search);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
        body.set_margin_start(24);
        body.set_margin_end(24);
        body.set_margin_top(22);
        body.set_margin_bottom(22);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&body)
            .build();
        main.append(&scroll);
        let status = gtk::Label::new(Some("Connecting to Handover…"));
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.add_css_class("status-strip");
        main.append(&status);
        let layout = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        layout.append(&rail);
        layout.append(&main);
        window.set_child(Some(&layout));
        let (commands, events) =
            connection::start(handover_ipc::socket_path().expect("Handover runtime directory"));
        let this = Rc::new(Self {
            window,
            model: RefCell::new(Model::default()),
            selected: RefCell::new(None),
            page: Cell::new(Page::Overview),
            devices,
            device_rows: RefCell::new(vec![]),
            body,
            heading,
            status,
            search,
            refresh: refresh.clone(),
            commands,
            pending: RefCell::new(BTreeMap::new()),
            refresh_again: RefCell::new(BTreeSet::new()),
            next: Cell::new(0),
            rendering: Cell::new(false),
            scheduled: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                let Some(this) = weak.upgrade() else {
                    break;
                };
                this.event(event);
            }
        });
        let weak = Rc::downgrade(&this);
        navigation.connect_row_selected(move |_, row| {
            if let (Some(this), Some(row)) = (weak.upgrade(), row) {
                this.page.set(pages[row.index() as usize]);
                this.search.set_text("");
                this.refresh_page();
                this.render();
            }
        });
        let weak = Rc::downgrade(&this);
        this.devices.connect_row_selected(move |_, row| {
            if let (Some(this), Some(row)) = (weak.upgrade(), row) {
                if this.rendering.get() {
                    return;
                }
                let selected = this
                    .device_rows
                    .borrow()
                    .get(row.index() as usize)
                    .cloned()
                    .flatten();
                *this.selected.borrow_mut() = selected;
                this.render();
            }
        });
        let weak = Rc::downgrade(&this);
        refresh.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.refresh_page();
            }
        });
        let weak = Rc::downgrade(&this);
        this.search.connect_search_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.schedule();
            }
        });
        let weak = Rc::downgrade(&this);
        messages.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.launch_messages();
            }
        });
        let weak = Rc::downgrade(&this);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, mods| {
            if key == gtk::gdk::Key::f && mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                if let Some(this) = weak.upgrade() {
                    this.search.grab_focus();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        this.window.add_controller(keys);
        this.render();
        this
    }
    fn launch_messages(&self) {
        let program = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("handover-messages")))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| "handover-messages".into());
        match std::process::Command::new(program).spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(_) => {
                self.model.borrow_mut().notice =
                    "Messages is not installed or could not start.".into();
                self.status.set_label(&self.model.borrow().notice);
            }
        }
    }
    fn request(self: &Rc<Self>, request: Request) {
        if let Some(key) = request.read_key()
            && self
                .pending
                .borrow()
                .values()
                .any(|r| r.read_key() == Some(key))
        {
            self.refresh_again.borrow_mut().insert(key);
            return;
        }
        if !self.model.borrow().connected {
            self.model.borrow_mut().notice = "Handover is offline. Nothing was submitted.".into();
            self.schedule();
            return;
        }
        let id = self.next.get().saturating_add(1);
        self.next.set(id);
        if self
            .commands
            .try_send(Command {
                id,
                request: request.clone(),
            })
            .is_ok()
        {
            self.pending.borrow_mut().insert(id, request);
        } else {
            self.model.borrow_mut().notice =
                "Handover is busy. Nothing was submitted; try again shortly.".into();
        }
        self.schedule();
    }
    fn refresh_page(self: &Rc<Self>) {
        match self.page.get() {
            Page::Connection => self.request(Request::Pairing),
            Page::Sharing => self.request(Request::Transfers),
            Page::Contacts => self.request(Request::Contacts),
            _ => {}
        }
    }
    fn event(self: &Rc<Self>, event: Event) {
        match event {
            Event::Connected(snapshot) => {
                {
                    let mut model = self.model.borrow_mut();
                    model.connected = true;
                    model.apply(*snapshot);
                    model.notice.clear();
                    model.peers.clear();
                    model.pairing.clear();
                }
                self.request(Request::Pairing);
                self.request(Request::Contacts);
                self.request(Request::Transfers);
            }
            Event::Disconnected => {
                let mut model = self.model.borrow_mut();
                model.connected = false;
                model.pairing.clear();
                model.notice =
                    "Handover is disconnected. Showing last reported state; commands are disabled."
                        .into();
            }
            Event::State(payload) => {
                let snapshot = matches!(*payload, ServerPayload::Snapshot { .. });
                let transfers = matches!(
                    *payload,
                    ServerPayload::ShareReceived { .. }
                        | ServerPayload::ShareProgress { .. }
                        | ServerPayload::ShareResult { .. }
                );
                let contacts = matches!(
                    *payload,
                    ServerPayload::ContactsSynced { .. } | ServerPayload::ContactsRemoved { .. }
                );
                self.model.borrow_mut().apply(*payload);
                if transfers || snapshot {
                    self.request(Request::Transfers);
                }
                if contacts || snapshot {
                    self.request(Request::Contacts);
                }
                if snapshot {
                    self.request(Request::Pairing);
                }
            }
            Event::Reply(id, result) => {
                let request = self.pending.borrow_mut().remove(&id);
                let key = request.as_ref().and_then(Request::read_key);
                match result {
                    Ok(Reply::Pairing(peers, pairing)) => {
                        let mut model = self.model.borrow_mut();
                        model.peers = peers;
                        model.pairing = pairing;
                    }
                    Ok(Reply::Contacts(contacts)) => {
                        self.model.borrow_mut().contacts = contacts
                            .into_iter()
                            .map(|mut c| {
                                c.photo = None;
                                c
                            })
                            .collect();
                    }
                    Ok(Reply::Transfers(activities)) => {
                        self.model.borrow_mut().transfers = activities
                    }
                    Ok(Reply::Share(id)) => self.model.borrow_mut().notice = if id.is_some() {
                        "Transfer accepted. Completion will appear only after the phone reports it."
                    } else {
                        "Share accepted. This device did not provide a tracking ID."
                    }
                    .into(),
                    Ok(Reply::Accepted) => {
                        self.model.borrow_mut().notice =
                            "Request accepted. Waiting for reported device state.".into();
                        if matches!(request, Some(Request::Pair(..) | Request::Forget(..))) {
                            self.request(Request::Pairing);
                        }
                    }
                    Err(message) => self.model.borrow_mut().notice = message,
                }
                if let Some(key) = key
                    && self.refresh_again.borrow_mut().remove(&key)
                {
                    self.request(match key {
                        0 => Request::Pairing,
                        1 => Request::Contacts,
                        _ => Request::Transfers,
                    });
                }
            }
        }
        self.schedule();
    }
    fn schedule(self: &Rc<Self>) {
        if self.scheduled.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_millis(40), move || {
            if let Some(this) = weak.upgrade() {
                this.scheduled.set(false);
                this.render();
            }
        });
    }
    fn busy(&self) -> bool {
        self.pending
            .borrow()
            .values()
            .any(|r| r.read_key().is_none())
    }
    fn render(self: &Rc<Self>) {
        self.rendering.set(true);
        while let Some(child) = self.devices.first_child() {
            self.devices.remove(&child);
        }
        let model = self.model.borrow();
        if self
            .selected
            .borrow()
            .as_ref()
            .is_some_and(|id| !model.devices.contains_key(id))
        {
            self.selected.borrow_mut().take();
        }
        let mut rows = vec![None];
        let all = gtk::Label::new(Some("All devices"));
        all.set_xalign(0.0);
        self.devices.append(&all);
        for device in model.devices.values() {
            let text = format!(
                "{} {}",
                if model.connected && device.connected {
                    "●"
                } else {
                    "○"
                },
                device.name
            );
            let label = gtk::Label::new(Some(&text));
            label.set_xalign(0.0);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            self.devices.append(&label);
            rows.push(Some(device.id.clone()));
        }
        let index = rows
            .iter()
            .position(|id| *id == *self.selected.borrow())
            .unwrap_or(0);
        *self.device_rows.borrow_mut() = rows;
        self.devices
            .select_row(self.devices.row_at_index(index as i32).as_ref());
        let title = self
            .selected
            .borrow()
            .as_ref()
            .map(|id| format!("{} · {}", self.page.get().name(), model.name(id)))
            .unwrap_or_else(|| self.page.get().name().into());
        self.heading.set_label(&title);
        self.status.set_label(if model.notice.is_empty() {
            if model.connected {
                "Connected to Handover"
            } else {
                "Connecting to Handover…"
            }
        } else {
            &model.notice
        });
        drop(model);
        self.refresh.set_visible(matches!(
            self.page.get(),
            Page::Connection | Page::Contacts | Page::Sharing
        ));
        self.search.set_visible(matches!(
            self.page.get(),
            Page::Notifications | Page::Contacts
        ));
        while let Some(child) = self.body.first_child() {
            self.body.remove(&child);
        }
        match self.page.get() {
            Page::Overview => self.overview(),
            Page::Notifications => self.notifications(),
            Page::Sharing => self.sharing(),
            Page::Contacts => self.contacts(),
            Page::Media => self.media(),
            Page::Connection => self.pairing(),
        }
        self.rendering.set(false);
    }
    fn matches_device(&self, id: &DeviceId) -> bool {
        self.selected
            .borrow()
            .as_ref()
            .is_none_or(|selected| selected == id)
    }
}
fn text(value: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(value));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.add_css_class(class);
    label
}
fn card(title: &str, subtitle: &str) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("card");
    card.append(&text(title, "card-title"));
    if !subtitle.is_empty() {
        card.append(&text(subtitle, "card-subtitle"));
    }
    card
}
