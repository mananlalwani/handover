use crate::{
    connection::{self, Command, Event, Failure, Reply, Request},
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

mod audio;
mod clipboard;
mod contact_images;
mod files;
mod notification_reply;
mod pages;
mod phone;
#[cfg(test)]
mod tests;
mod tools;

pub(super) fn download_failure(reason: FileDownloadFailure) -> &'static str {
    match reason {
        FileDownloadFailure::PermissionRequired => "Enable file sharing in the phone app",
        FileDownloadFailure::InvalidResource => "The phone rejected this file path",
        FileDownloadFailure::SizeLimit => "The file is too large to download",
        FileDownloadFailure::Unavailable => "The file is unavailable on the phone",
        FileDownloadFailure::Storage => "There is not enough storage to save the file",
        FileDownloadFailure::Interrupted => "The download was interrupted",
        FileDownloadFailure::TimedOut => "The phone did not respond in time",
        FileDownloadFailure::Disconnected => "The phone disconnected during the download",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Page {
    Overview,
    Calls,
    PhoneFiles,
    Notifications,
    Sharing,
    Contacts,
    Clipboard,
    Media,
    Connection,
}
impl Page {
    fn name(self) -> &'static str {
        match self {
            Self::Overview => "Phone",
            Self::Calls => "Calls",
            Self::PhoneFiles => "Phone folders",
            Self::Notifications => "Notifications",
            Self::Sharing => "Files",
            Self::Contacts => "Contacts",
            Self::Clipboard => "Clipboard",
            Self::Media => "Media",
            Self::Connection => "Devices & pairing",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Overview => "Connection and device controls",
            Self::Calls => "Calls on your phone",
            Self::PhoneFiles => "Folders shared by your phone",
            Self::Notifications => "Notifications from your phones",
            Self::Sharing => "Send a file or link to your phone",
            Self::Contacts => "Contacts synced from your phone",
            Self::Clipboard => "Send text and manage clipboard history",
            Self::Media => "Laptop audio and phone playback",
            Self::Connection => "Trusted devices and pairing requests",
        }
    }
}
pub struct Window {
    pub window: gtk::ApplicationWindow,
    model: RefCell<Model>,
    file_queue: RefCell<Vec<files::PickedFile>>,
    dial_address: RefCell<String>,
    remote_drafts: RefCell<BTreeMap<DeviceId, String>>,
    audio_source_draft: RefCell<Option<AudioSourceSelection>>,
    audio_diagnostics: RefCell<Option<(DeviceId, gtk::Label, gtk::Label)>>,
    expanded_sections: RefCell<BTreeSet<&'static str>>,
    folder_path: RefCell<String>,
    folder_requested: RefCell<Option<(DeviceId, String)>>,
    folder_busy: Cell<bool>,
    folder_generation: Cell<u64>,
    reply_drafts: RefCell<BTreeMap<NotificationId, notification_reply::Draft>>,
    reply_editor: RefCell<Option<Rc<notification_reply::Editor>>>,
    selected: RefCell<Option<DeviceId>>,
    selection_initialized: Cell<bool>,
    page: Cell<Page>,
    devices: gtk::ListBox,
    tabs: Vec<(Page, gtk::ToggleButton)>,
    device_picker: gtk::MenuButton,
    device_name: gtk::Label,
    device_popover: gtk::Popover,
    phone_pane: gtk::Box,
    phone_state: RefCell<Option<(Option<Device>, bool, bool)>>,
    rail_state: RefCell<Option<(bool, Vec<Device>)>>,
    device_rows: RefCell<Vec<Option<DeviceId>>>,
    body: gtk::Box,
    scroll: gtk::ScrolledWindow,
    body_dirty: Cell<bool>,
    heading: gtk::Label,
    description: gtk::Label,
    service_status: gtk::Label,
    status_bar: gtk::Box,
    status: gtk::Label,
    search: gtk::SearchEntry,
    refresh: gtk::Button,
    commands: tokio::sync::mpsc::Sender<Command>,
    pending: RefCell<BTreeMap<u64, Request>>,
    refresh_again: RefCell<BTreeSet<u8>>,
    next: Cell<u64>,
    notice_generation: Cell<u64>,
    progress_refresh: Cell<bool>,
    close_confirmed: Cell<bool>,
    close_prompt: Cell<bool>,
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
            .default_width(1120)
            .default_height(780)
            .build();
        window.add_css_class("control");
        // The top toolbar owns window controls and dragging.
        window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
        let devices = gtk::ListBox::new();
        devices.add_css_class("device-list");
        let device_popover = gtk::Popover::new();
        device_popover.set_child(Some(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .min_content_width(235)
                .max_content_height(320)
                .propagate_natural_height(true)
                .child(&devices)
                .build(),
        ));
        let device_name = text("Choose a device", "device-title");
        device_name.set_wrap(false);
        device_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        device_name.set_hexpand(false);
        device_name.set_max_width_chars(16);
        let device_label = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        device_label.append(&device_name);
        device_label.append(&gtk::Image::from_icon_name("pan-down-symbolic"));
        let device_picker = gtk::MenuButton::builder()
            .popover(&device_popover)
            .child(&device_label)
            .build();
        device_picker.add_css_class("device-selector");
        let phone_pane = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let messages = gtk::Button::with_label("Messages ↗");
        messages.add_css_class("nav-button");
        messages.set_tooltip_text(Some("Open Messages"));
        if let Some(label) = messages.child().and_downcast::<gtk::Label>() {
            label.set_xalign(0.0);
        }
        let rail = gtk::Box::new(gtk::Orientation::Vertical, 4);
        rail.set_width_request(204);
        rail.add_css_class("navigation-rail");
        let brand = text("Handover", "brand");
        let brand_handle = gtk::WindowHandle::new();
        brand_handle.set_child(Some(&brand));
        rail.append(&brand_handle);
        rail.append(&device_picker);
        rail.append(&phone_pane);
        let navigation = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let mut tabs = Vec::new();
        let mut first = None::<gtk::ToggleButton>;
        for (page, icon) in [
            (Page::Overview, "phone-symbolic"),
            (
                Page::Notifications,
                "preferences-system-notifications-symbolic",
            ),
            (Page::Calls, "call-start-symbolic"),
            (Page::Sharing, "folder-symbolic"),
            (Page::Clipboard, "edit-paste-symbolic"),
            (Page::Contacts, "system-users-symbolic"),
            (Page::Media, "multimedia-player-symbolic"),
            (Page::Connection, "preferences-system-symbolic"),
        ] {
            let tab = gtk::ToggleButton::new();
            if let Some(first) = &first {
                tab.set_group(Some(first));
            } else {
                first = Some(tab.clone());
            }
            tab.add_css_class("nav-button");
            let label = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            label.append(&gtk::Image::from_icon_name(icon));
            label.append(&text(
                if page == Page::Connection {
                    "Device settings"
                } else {
                    page.name()
                },
                "nav-label",
            ));
            tab.set_child(Some(&label));
            tab.update_property(&[gtk::accessible::Property::Label(page.name())]);
            navigation.append(&tab);
            tabs.push((page, tab));
        }
        navigation.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        navigation.append(&messages);
        let nav_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&navigation)
            .build();
        rail.append(&nav_scroll);
        let service_status = text("Connecting…", "service-status");
        rail.append(&service_status);
        tabs[0].1.set_active(true);
        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main.set_hexpand(true);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 20);
        header.add_css_class("page-header");
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 5);
        titles.set_hexpand(true);
        let heading = text("Overview", "page-title");
        let description = text(Page::Overview.description(), "page-description");
        titles.append(&heading);
        titles.append(&description);
        header.append(&titles);
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&header));
        main.append(&handle);
        let filters = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        filters.add_css_class("filter-toolbar");
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search this view"));
        search.set_hexpand(true);
        filters.append(&search);
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.add_css_class("flat");
        refresh.set_tooltip_text(Some("Refresh"));
        header.append(&refresh);
        header.append(&gtk::WindowControls::new(gtk::PackType::End));
        main.append(&filters);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_margin_start(28);
        body.set_margin_end(28);
        body.set_margin_top(18);
        body.set_margin_bottom(28);
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
            file_queue: RefCell::new(Vec::new()),
            dial_address: RefCell::new(String::new()),
            remote_drafts: RefCell::new(BTreeMap::new()),
            audio_source_draft: RefCell::new(None),
            audio_diagnostics: RefCell::new(None),
            expanded_sections: RefCell::new(BTreeSet::new()),
            folder_path: RefCell::new(".".into()),
            folder_requested: RefCell::new(None),
            folder_busy: Cell::new(false),
            folder_generation: Cell::new(0),
            reply_drafts: RefCell::new(BTreeMap::new()),
            reply_editor: RefCell::new(None),
            selected: RefCell::new(None),
            selection_initialized: Cell::new(false),
            page: Cell::new(Page::Overview),
            devices,
            tabs,
            device_picker,
            device_name,
            device_popover,
            phone_pane,
            phone_state: RefCell::new(None),
            rail_state: RefCell::new(None),
            device_rows: RefCell::new(vec![]),
            body,
            scroll: scroll.clone(),
            body_dirty: Cell::new(true),
            heading,
            description,
            service_status,
            status_bar,
            status,
            search,
            refresh: refresh.clone(),
            commands,
            pending: RefCell::new(BTreeMap::new()),
            refresh_again: RefCell::new(BTreeSet::new()),
            next: Cell::new(0),
            notice_generation: Cell::new(0),
            progress_refresh: Cell::new(false),
            close_confirmed: Cell::new(false),
            close_prompt: Cell::new(false),
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
        for (page, tab) in &this.tabs {
            let page = *page;
            let weak = Rc::downgrade(&this);
            tab.connect_toggled(move |tab| {
                if let Some(this) = weak.upgrade()
                    && tab.is_active()
                    && !this.rendering.get()
                    && this.page.get() != page
                {
                    this.navigate(page);
                }
            });
        }
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
                this.select_device(selected);
                this.device_popover.popdown();
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
        let weak = Rc::downgrade(&this);
        this.window.connect_close_request(move |_| {
            weak.upgrade()
                .map_or(glib::Propagation::Proceed, |this| this.confirm_close())
        });
        let file_drop = gtk::DropTarget::new(
            gtk::gdk::FileList::static_type(),
            gtk::gdk::DragAction::COPY,
        );
        let weak = Rc::downgrade(&this);
        file_drop.connect_drop(move |_, value, _, _| {
            if let (Some(this), Ok(files)) = (weak.upgrade(), value.get::<gtk::gdk::FileList>()) {
                this.add_files(files.files())
            } else {
                false
            }
        });
        this.window.add_controller(file_drop);
        this.window.add_controller(keys);
        this.render();
        this
    }
    fn confirm_close(self: &Rc<Self>) -> glib::Propagation {
        let needs_review = self.file_queue.borrow().iter().any(|file| {
            matches!(
                file.status,
                files::FileStatus::Ready
                    | files::FileStatus::Submitting { .. }
                    | files::FileStatus::Unconfirmed { .. }
            )
        });
        if self.close_confirmed.get()
            || (!needs_review
                && self.reply_drafts.borrow().is_empty()
                && self.remote_drafts.borrow().values().all(|s| s.is_empty())
                && self.audio_source_draft.borrow().is_none())
        {
            return glib::Propagation::Proceed;
        }
        if self.close_prompt.replace(true) {
            return glib::Propagation::Stop;
        }
        let dialog = adw::MessageDialog::new(
            Some(&self.window),
            Some("Close Handover?"),
            Some(
                "Unsent file selections, reply drafts, remote text, and unapplied audio choices will be discarded. Closing does not cancel submitted commands.",
            ),
        );
        dialog.set_application(self.window.application().as_ref());
        dialog.add_response("keep", "Keep open");
        dialog.add_response("close", "Close");
        dialog.set_close_response("keep");
        dialog.set_default_response(Some("keep"));
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |dialog, response| {
            if let Some(this) = weak.upgrade() {
                this.close_prompt.set(false);
                if response == "close" {
                    this.close_confirmed.set(true);
                    this.window.close();
                }
            }
            dialog.close();
        });
        dialog.present();
        glib::Propagation::Stop
    }
    fn select_device(&self, selected: Option<DeviceId>) {
        if *self.selected.borrow() != selected {
            self.folder_path.replace(".".into());
            self.folder_requested.borrow_mut().take();
            self.folder_busy.set(false);
            self.model.borrow_mut().folder = None;
        }
        self.selected.replace(selected);
    }
    fn navigate(self: &Rc<Self>, page: Page) {
        self.page.set(page);
        self.search.set_text("");
        self.scroll.vadjustment().set_value(0.0);
        self.rendering.set(true);
        for (candidate, tab) in &self.tabs {
            tab.set_active(*candidate == page);
        }
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
            Page::Connection | Page::Overview => {
                self.request(Request::Pairing);
                if self.page.get() == Page::Connection {
                    self.request(Request::Tools);
                }
            }
            Page::Media => self.request(Request::Audio),
            Page::Calls => self.request(Request::CallAudio),
            Page::Sharing => self.request(Request::Transfers),
            Page::Contacts => self.request(Request::Contacts),
            Page::Clipboard => self.request(Request::ClipboardHistory),
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
                    model.peers.clear();
                    model.pairing.clear();
                    model.clipboard_history = None;
                    model.clipboard_mirror = None;
                    model.folder = None;
                    model.audio_controls = None;
                    model.audio_error = None;
                    model.audio_streams.clear();
                    model.call_audio = None;
                    model.commands = None;
                    model.command_result = None;
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
                match self.page.get() {
                    Page::Media => self.request(Request::Audio),
                    Page::Calls => self.request(Request::CallAudio),
                    Page::Connection => self.request(Request::Tools),
                    _ => {}
                }
                if self.page.get() == Page::Clipboard {
                    self.request(Request::ClipboardHistory);
                }
            }
            Event::Disconnected => {
                self.folder_busy.set(false);
                let mut model = self.model.borrow_mut();
                model.connected = false;
                model.pairing.clear();
            }
            Event::State(payload) => {
                if let ServerPayload::Filesystem { result } = payload.as_ref() {
                    if self.folder_requested.borrow().as_ref()
                        != Some(&(result.device_id.clone(), result.path.clone()))
                    {
                        return;
                    }
                    self.folder_busy.set(false);
                }
                rebuild = self.affects_page(&payload);
                if let ServerPayload::ShareResult { result } = payload.as_ref() {
                    self.file_result(result);
                    if self.page.get() == Page::Sharing {
                        rebuild = true;
                    }
                }
                if self.page.get() == Page::Calls
                    && matches!(
                        *payload,
                        ServerPayload::CallUpdated { .. } | ServerPayload::CallRemoved { .. }
                    )
                {
                    self.request(Request::CallAudio);
                }
                let snapshot = matches!(*payload, ServerPayload::Snapshot { .. });
                let transfers = matches!(
                    *payload,
                    ServerPayload::ShareReceived { .. }
                        | ServerPayload::ShareProgress { .. }
                        | ServerPayload::ShareResult { .. }
                        | ServerPayload::FileDownload { .. }
                );
                let contacts = matches!(
                    *payload,
                    ServerPayload::ContactsSynced { .. } | ServerPayload::ContactsRemoved { .. }
                );
                let progress = matches!(payload.as_ref(), ServerPayload::ShareProgress { .. });
                self.model.borrow_mut().apply(*payload);
                if progress {
                    if self.page.get() == Page::Sharing && !self.progress_refresh.replace(true) {
                        let weak = Rc::downgrade(self);
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(250),
                            move || {
                                if let Some(this) = weak.upgrade() {
                                    this.progress_refresh.set(false);
                                    if this.model.borrow().connected {
                                        this.request(Request::Transfers);
                                    }
                                }
                            },
                        );
                    }
                } else if transfers || snapshot {
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
                if matches!(request, Some(Request::Folder(..))) && result.is_err() {
                    self.folder_busy.set(false);
                }
                if let Some(request) = &request {
                    self.file_reply(request, &result);
                    self.notification_reply_result(request, &result);
                }
                let key = request.as_ref().and_then(Request::read_key);
                rebuild = match key {
                    Some(0) => matches!(self.page.get(), Page::Connection | Page::Overview),
                    Some(1) => self.page.get() == Page::Contacts,
                    Some(2) => self.page.get() == Page::Sharing,
                    Some(3) => self.page.get() == Page::Clipboard,
                    Some(4) => self.page.get() == Page::Media,
                    Some(5) => self.page.get() == Page::Connection,
                    Some(6) => self.page.get() == Page::Calls,
                    _ => true,
                };
                match result {
                    Ok(Reply::Pairing(peers, pairing)) => {
                        let mut model = self.model.borrow_mut();
                        model.peers = peers;
                        model.pairing = pairing;
                    }
                    Ok(Reply::Contacts(contacts)) => {
                        self.model.borrow_mut().contacts = contacts;
                    }
                    Ok(Reply::ClipboardHistory(entries, mirror)) => {
                        let mut model = self.model.borrow_mut();
                        model.clipboard_history = Some(entries);
                        model.clipboard_mirror = Some(mirror);
                    }
                    Ok(Reply::Audio(controls, streams)) => {
                        let mut model = self.model.borrow_mut();
                        model.observe_audio_controls(controls);
                        for stream in streams {
                            model.observe_audio_stream(stream);
                        }
                    }
                    Ok(Reply::AudioControls(controls)) => {
                        self.model.borrow_mut().observe_audio_controls(controls);
                        if matches!(
                            request,
                            Some(Request::ConfigureAudio(AudioRoutingChange {
                                source: Some(_),
                                ..
                            }))
                        ) {
                            self.audio_source_draft.borrow_mut().take();
                        }
                    }
                    Ok(Reply::CallAudio(status)) => {
                        self.model.borrow_mut().call_audio = Some(status)
                    }
                    Ok(Reply::Tools(commands)) => self.model.borrow_mut().commands = Some(commands),
                    Ok(Reply::CommandResult(result)) => {
                        self.model.borrow_mut().command_result = Some(result)
                    }
                    Ok(Reply::Transfers(activities)) => {
                        for activity in &activities {
                            if let TransferActivityEvent::Result { result } = &activity.event {
                                self.file_result(result);
                            }
                        }
                        self.model.borrow_mut().transfers = activities
                    }
                    Ok(Reply::Share(id)) => {
                        self.request(Request::Transfers);
                        self.transient_notice(if id.is_some() {
                            "Transfer accepted"
                        } else {
                            "Share accepted · No tracking available"
                        });
                    }
                    Ok(Reply::Download) => {
                        self.model.borrow_mut().notice.clear();
                        self.request(Request::Transfers);
                        self.transient_notice(
                            "Download request accepted · Check the phone for completion",
                        );
                    }
                    Ok(Reply::Accepted) => {
                        self.model.borrow_mut().notice.clear();
                        if matches!(
                            request,
                            Some(
                                Request::PinClipboard(..)
                                    | Request::ClearClipboard(..)
                                    | Request::Mirror(..)
                            )
                        ) {
                            self.request(Request::ClipboardHistory);
                        }
                        if let Some(Request::RemoteInput(command)) = &request
                            && command.action == RemoteInputAction::Type
                            && self.remote_drafts.borrow().get(&command.device_id)
                                == command.text.as_ref()
                        {
                            self.remote_drafts.borrow_mut().remove(&command.device_id);
                        }
                        if matches!(
                            request,
                            Some(Request::AudioStream(..) | Request::AudioProfile(..))
                        ) {
                            self.request(Request::Audio);
                        }
                        if matches!(request, Some(Request::Screensaver(..))) {
                            self.transient_notice("Desktop sleep policy request accepted.");
                        }
                        if matches!(request, Some(Request::RemoteInput(..))) {
                            self.transient_notice(
                                "Input request accepted. Check the phone for its effect.",
                            );
                        }
                        if matches!(request, Some(Request::Phone(..) | Request::Call(..))) {
                            self.transient_notice("Request queued for the phone.");
                        }
                        if matches!(request, Some(Request::NotificationReply(..))) {
                            self.transient_notice(
                                "Reply accepted. Check the conversation for delivery.",
                            );
                        }
                        if matches!(request, Some(Request::Pair(..) | Request::Forget(..))) {
                            self.request(Request::Pairing);
                        }
                    }
                    Err(failure) if key == Some(4) => {
                        self.model.borrow_mut().audio_error = Some(failure.message)
                    }
                    Err(failure) => self.model.borrow_mut().notice = failure.message,
                }
                if let Some(key) = key
                    && self.refresh_again.borrow_mut().remove(&key)
                {
                    self.request(match key {
                        0 => Request::Pairing,
                        1 => Request::Contacts,
                        2 => Request::Transfers,
                        3 => Request::ClipboardHistory,
                        4 => Request::Audio,
                        5 => Request::Tools,
                        _ => Request::CallAudio,
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
            ServerPayload::DeviceUpdated { .. } if page == Page::Overview => true,
            ServerPayload::DeviceUpdated { device } => self
                .model
                .borrow()
                .devices
                .get(&device.id)
                .is_none_or(|old| {
                    old.connected != device.connected
                        || old.paired != device.paired
                        || old.capabilities != device.capabilities
                }),
            ServerPayload::NotificationAdded { .. }
            | ServerPayload::NotificationUpdated { .. }
            | ServerPayload::NotificationRemoved { .. } => page == Page::Notifications,
            ServerPayload::MediaAdded { .. }
            | ServerPayload::MediaUpdated { .. }
            | ServerPayload::MediaRemoved { .. } => page == Page::Media,
            ServerPayload::CallUpdated { .. } | ServerPayload::CallRemoved { .. } => {
                matches!(page, Page::Overview | Page::Calls)
            }
            ServerPayload::Filesystem { .. } => page == Page::PhoneFiles,
            ServerPayload::FileDownload { .. } => matches!(page, Page::PhoneFiles | Page::Sharing),
            ServerPayload::AudioStreamUpdated { status } => {
                page == Page::Media
                    && self
                        .model
                        .borrow()
                        .audio_streams
                        .get(&status.device_id)
                        .is_none_or(|old| {
                            old.state != status.state
                                || old.enabled != status.enabled
                                || old.profile != status.profile
                                || old.failure != status.failure
                                || old.retry_attempt != status.retry_attempt
                        })
            }
            ServerPayload::AudioStreams { .. } | ServerPayload::AudioControlsUpdated { .. } => {
                page == Page::Media
            }
            ServerPayload::ContactsSynced { .. } | ServerPayload::ContactsRemoved { .. } => {
                page == Page::Contacts
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
        self.heading
            .set_label(if self.page.get() == Page::PhoneFiles {
                "Files"
            } else {
                self.page.get().name()
            });
        for (page, tab) in &self.tabs {
            tab.set_active(
                *page == self.page.get()
                    || (*page == Page::Sharing && self.page.get() == Page::PhoneFiles),
            );
        }
        self.description.set_label(self.page.get().description());
        self.service_status.set_visible(!model.connected);
        self.service_status.set_label(if model.connected {
            "●  Service connected"
        } else {
            "●  Service offline"
        });
        if model.connected {
            self.service_status.add_css_class("connected");
        } else {
            self.service_status.remove_css_class("connected");
        }
        let selected = self
            .selected
            .borrow()
            .as_ref()
            .and_then(|id| model.devices.get(id))
            .cloned();
        self.device_name.set_label(
            selected
                .as_ref()
                .map(|d| d.name.as_str())
                .unwrap_or("All devices"),
        );
        self.device_picker
            .set_tooltip_text(selected.as_ref().map(|device| device.name.as_str()));
        let phone_state = (selected.clone(), model.connected, self.busy());
        let phone_changed = self.phone_state.borrow().as_ref() != Some(&phone_state);
        if phone_changed {
            *self.phone_state.borrow_mut() = Some(phone_state);
        }
        self.status_bar
            .set_visible(!model.connected || !model.notice.is_empty());
        let status = if !model.connected {
            if model.notice.is_empty() {
                "Reconnecting to Handover…".to_owned()
            } else {
                format!("Reconnecting to Handover…\n{}", model.notice)
            }
        } else {
            model.notice.clone()
        };
        self.status.set_label(&status);
        drop(model);
        self.refresh_notification_reply();
        if self.page.get() == Page::Media {
            self.update_audio_diagnostics();
        } else {
            self.audio_diagnostics.borrow_mut().take();
        }
        if phone_changed {
            self.render_phone();
        }
        self.refresh.set_visible(matches!(
            self.page.get(),
            Page::Overview
                | Page::Connection
                | Page::Sharing
                | Page::Clipboard
                | Page::Media
                | Page::Calls
        ));
        self.search
            .set_placeholder_text(Some(if self.page.get() == Page::Contacts {
                "Search contacts"
            } else if self.page.get() == Page::Clipboard {
                "Search clipboard history"
            } else {
                "Search notifications"
            }));
        self.search.set_visible(matches!(
            self.page.get(),
            Page::Notifications | Page::Contacts | Page::Clipboard
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
            Page::Calls => self.calls(),
            Page::PhoneFiles => self.phone_files(),
            Page::Notifications => self.notifications(),
            Page::Sharing => self.sharing(),
            Page::Contacts => self.contacts(),
            Page::Clipboard => self.clipboard_history(),
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
