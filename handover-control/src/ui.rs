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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    selection_initialized: Cell<bool>,
    page: Cell<Page>,
    devices: gtk::ListBox,
    navigation: gtk::ListBox,
    rail_state: RefCell<Option<(bool, Vec<Device>)>>,
    device_rows: RefCell<Vec<Option<DeviceId>>>,
    body: gtk::Box,
    scroll: gtk::ScrolledWindow,
    body_dirty: Cell<bool>,
    heading: gtk::Label,
    context: gtk::Label,
    status_bar: gtk::Box,
    status: gtk::Label,
    search: gtk::SearchEntry,
    refresh: gtk::Button,
    commands: tokio::sync::mpsc::Sender<Command>,
    pending: RefCell<BTreeMap<u64, Request>>,
    refresh_again: RefCell<BTreeSet<u8>>,
    next: Cell<u64>,
    notice_generation: Cell<u64>,
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
        // The page header owns window controls and dragging; no second empty title bar.
        window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
        let rail = gtk::Box::new(gtk::Orientation::Vertical, 4);
        rail.set_width_request(210);
        rail.add_css_class("device-rail");
        let brand = gtk::Label::new(Some("Handover"));
        brand.set_xalign(0.0);
        brand.add_css_class("brand");
        rail.append(&brand);
        let caption = text("Devices", "section-caption");
        rail.append(&caption);
        let devices = gtk::ListBox::new();
        rail.append(&devices);
        let divider = gtk::Separator::new(gtk::Orientation::Horizontal);
        divider.set_margin_top(14);
        divider.set_margin_bottom(14);
        divider.set_margin_start(10);
        divider.set_margin_end(10);
        rail.append(&divider);
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
        for (page, icon) in pages.into_iter().zip([
            "view-grid-symbolic",
            "preferences-system-notifications-symbolic",
            "folder-download-symbolic",
            "avatar-default-symbolic",
            "multimedia-player-symbolic",
            "preferences-system-symbolic",
        ]) {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.append(&gtk::Image::from_icon_name(icon));
            row.append(&text(page.name(), "nav-label"));
            navigation.append(&row);
        }
        navigation.select_row(navigation.row_at_index(0).as_ref());
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_vexpand(true);
        rail.append(&spacer);
        let messages = gtk::Button::with_label("Messages ↗");
        messages.add_css_class("flat");
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
        let title = gtk::Box::new(gtk::Orientation::Vertical, 2);
        title.set_hexpand(true);
        let context = text("", "card-subtitle");
        title.append(&heading);
        title.append(&context);
        header.append(&title);
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some("Refresh current view"));
        header.append(&refresh);
        header.append(&gtk::WindowControls::new(gtk::PackType::End));
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&header));
        main.append(&handle);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search this view"));
        search.set_margin_top(12);
        search.set_margin_start(24);
        search.set_margin_end(24);
        main.append(&search);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
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
        let status_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        status_bar.add_css_class("status-banner");
        status.set_hexpand(true);
        status_bar.append(&status);
        let dismiss_status = gtk::Button::from_icon_name("window-close-symbolic");
        dismiss_status.add_css_class("flat");
        dismiss_status.set_tooltip_text(Some("Dismiss notice"));
        status_bar.append(&dismiss_status);
        main.append(&status_bar);
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
            selection_initialized: Cell::new(false),
            page: Cell::new(Page::Overview),
            devices,
            navigation: navigation.clone(),
            rail_state: RefCell::new(None),
            device_rows: RefCell::new(vec![]),
            body,
            scroll: scroll.clone(),
            body_dirty: Cell::new(true),
            heading,
            context,
            status_bar,
            status,
            search,
            refresh: refresh.clone(),
            commands,
            pending: RefCell::new(BTreeMap::new()),
            refresh_again: RefCell::new(BTreeSet::new()),
            next: Cell::new(0),
            notice_generation: Cell::new(0),
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
                let page = pages[row.index() as usize];
                if !this.rendering.get() && this.page.get() != page {
                    this.navigate(page);
                }
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
                this.scroll.vadjustment().set_value(0.0);
                this.render();
            }
        });
        let weak = Rc::downgrade(&this);
        dismiss_status.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.model.borrow_mut().notice.clear();
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
                this.scroll.vadjustment().set_value(0.0);
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
            if key == gtk::gdk::Key::f
                && mods.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                && let Some(this) = weak.upgrade()
                && this.search.is_visible()
            {
                this.search.grab_focus();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        this.window.add_controller(keys);
        this.render();
        this
    }
    fn navigate(self: &Rc<Self>, page: Page) {
        self.page.set(page);
        self.search.set_text("");
        self.scroll.vadjustment().set_value(0.0);
        let index = match page {
            Page::Overview => 0,
            Page::Notifications => 1,
            Page::Sharing => 2,
            Page::Contacts => 3,
            Page::Media => 4,
            Page::Connection => 5,
        };
        self.rendering.set(true);
        self.navigation
            .select_row(self.navigation.row_at_index(index).as_ref());
        self.rendering.set(false);
        self.refresh_page();
        self.render();
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
        let read = request.read_key().is_some();
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
        if read {
            self.schedule_chrome();
        } else {
            self.schedule();
        }
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
        let mut rebuild = true;
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
                if !self.selection_initialized.replace(true) {
                    let model = self.model.borrow();
                    let online = model
                        .devices
                        .values()
                        .filter(|d| d.connected && d.paired)
                        .collect::<Vec<_>>();
                    *self.selected.borrow_mut() = if online.len() == 1 {
                        Some(online[0].id.clone())
                    } else if model.devices.len() == 1 {
                        model.devices.keys().next().cloned()
                    } else {
                        None
                    };
                }
                self.request(Request::Pairing);
                self.request(Request::Contacts);
                self.request(Request::Transfers);
            }
            Event::Disconnected => {
                let mut model = self.model.borrow_mut();
                model.connected = false;
                model.pairing.clear();
                model.notice = "Connection lost. Reconnecting…".into();
            }
            Event::State(payload) => {
                rebuild = self.affects_page(&payload);
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
                rebuild = match key {
                    Some(0) => matches!(self.page.get(), Page::Connection | Page::Overview),
                    Some(1) => matches!(self.page.get(), Page::Contacts | Page::Overview),
                    Some(2) => self.page.get() == Page::Sharing,
                    _ => true,
                };
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
                    Ok(Reply::Share(id)) => self.transient_notice(if id.is_some() {
                        "Transfer accepted"
                    } else {
                        "Share accepted · No tracking available"
                    }),
                    Ok(Reply::Accepted) => {
                        self.model.borrow_mut().notice.clear();
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
        if rebuild {
            self.schedule();
        } else {
            self.schedule_chrome();
        }
    }
    fn transient_notice(self: &Rc<Self>, notice: &str) {
        let generation = self.notice_generation.get().wrapping_add(1);
        self.notice_generation.set(generation);
        self.model.borrow_mut().notice = notice.into();
        let notice = notice.to_string();
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
            if let Some(this) = weak.upgrade()
                && this.notice_generation.get() == generation
                && this.model.borrow().notice == notice
            {
                this.model.borrow_mut().notice.clear();
                this.schedule_chrome();
            }
        });
    }
    fn affects_page(&self, payload: &ServerPayload) -> bool {
        let page = self.page.get();
        match payload {
            ServerPayload::Snapshot { .. }
            | ServerPayload::DeviceAdded { .. }
            | ServerPayload::DeviceRemoved { .. } => true,
            ServerPayload::DeviceUpdated { device } => {
                page == Page::Overview
                    || self
                        .model
                        .borrow()
                        .devices
                        .get(&device.id)
                        .is_none_or(|old| {
                            old.connected != device.connected
                                || old.paired != device.paired
                                || old.capabilities != device.capabilities
                        })
            }
            ServerPayload::NotificationAdded { .. }
            | ServerPayload::NotificationUpdated { .. }
            | ServerPayload::NotificationRemoved { .. } => {
                matches!(page, Page::Overview | Page::Notifications)
            }
            ServerPayload::MediaAdded { .. }
            | ServerPayload::MediaUpdated { .. }
            | ServerPayload::MediaRemoved { .. } => matches!(page, Page::Overview | Page::Media),
            ServerPayload::CallUpdated { .. } | ServerPayload::CallRemoved { .. } => {
                page == Page::Overview
            }
            ServerPayload::ContactsSynced { .. } | ServerPayload::ContactsRemoved { .. } => {
                matches!(page, Page::Overview | Page::Contacts)
            }
            _ => false,
        }
    }
    fn schedule(self: &Rc<Self>) {
        self.body_dirty.set(true);
        self.schedule_chrome();
    }
    fn schedule_chrome(self: &Rc<Self>) {
        if self.scheduled.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_millis(40), move || {
            if let Some(this) = weak.upgrade() {
                this.scheduled.set(false);
                this.render_inner(this.body_dirty.replace(false));
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
        self.body_dirty.set(false);
        self.render_inner(true);
    }
    fn render_inner(self: &Rc<Self>, rebuild: bool) {
        self.rendering.set(true);
        let model = self.model.borrow();
        if self
            .selected
            .borrow()
            .as_ref()
            .is_some_and(|id| !model.devices.contains_key(id))
        {
            self.selected.borrow_mut().take();
        }
        let state = (
            model.connected,
            model
                .devices
                .values()
                .cloned()
                .map(|mut d| {
                    d.battery = None;
                    d.connectivity = None;
                    d.capabilities.clear();
                    d
                })
                .collect::<Vec<_>>(),
        );
        if self.rail_state.borrow().as_ref() != Some(&state) {
            while let Some(child) = self.devices.first_child() {
                self.devices.remove(&child);
            }
            let all = gtk::Box::new(gtk::Orientation::Horizontal, 9);
            all.append(&gtk::Image::from_icon_name("computer-symbolic"));
            all.append(&text("All devices", "nav-label"));
            self.devices.append(&all);
            let mut rows = vec![None];
            for device in model.devices.values() {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
                row.append(&gtk::Image::from_icon_name("phone-symbolic"));
                let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
                labels.set_hexpand(true);
                let name = text(&device.name, "nav-label");
                name.set_wrap(false);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                labels.append(&name);
                labels.append(&text(
                    if model.connected && device.connected {
                        "Connected"
                    } else {
                        "Offline"
                    },
                    "device-status",
                ));
                row.append(&labels);
                self.devices.append(&row);
                rows.push(Some(device.id.clone()));
            }
            *self.device_rows.borrow_mut() = rows;
            *self.rail_state.borrow_mut() = Some(state);
        }
        let index = self
            .device_rows
            .borrow()
            .iter()
            .position(|id| *id == *self.selected.borrow())
            .unwrap_or(0);
        self.devices
            .select_row(self.devices.row_at_index(index as i32).as_ref());
        self.heading.set_label(self.page.get().name());
        let context = self.selected.borrow().as_ref().map(|id| model.name(id));
        self.context.set_label(context.as_deref().unwrap_or(""));
        self.context.set_visible(context.is_some());
        self.status_bar
            .set_visible(!model.connected || !model.notice.is_empty());
        self.status.set_label(if model.notice.is_empty() {
            "Connecting…"
        } else {
            &model.notice
        });
        drop(model);
        self.refresh
            .set_visible(matches!(self.page.get(), Page::Connection | Page::Sharing));
        self.search
            .set_placeholder_text(Some(if self.page.get() == Page::Contacts {
                "Search contacts"
            } else {
                "Search notifications"
            }));
        self.search.set_visible(matches!(
            self.page.get(),
            Page::Notifications | Page::Contacts
        ));
        if !rebuild {
            self.rendering.set(false);
            return;
        }
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

fn empty(icon: &str, title: &str, description: &str) -> gtk::Box {
    let area = gtk::Box::new(gtk::Orientation::Vertical, 10);
    area.add_css_class("empty-state");
    area.set_halign(gtk::Align::Center);
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(38);
    image.add_css_class("dim-label");
    area.append(&image);
    let title = text(title, "empty-title");
    title.set_xalign(0.5);
    area.append(&title);
    let label = text(description, "card-subtitle");
    label.set_xalign(0.5);
    area.append(&label);
    area
}
