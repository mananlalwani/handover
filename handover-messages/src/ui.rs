use crate::{
    connection::{self, Command, Event, Failure, Reply, Request},
    model::{Model, outcome_label},
};
use adw::prelude::*;
use gtk::{gio, glib};
use handover_core::*;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

type Target = (ConversationId, Option<MessageId>);

pub struct Window {
    pub window: adw::ApplicationWindow,
    model: RefCell<Model>,
    commands: tokio::sync::mpsc::Sender<Command>,
    pending: RefCell<BTreeMap<u64, Request>>,
    sequence: Cell<u64>,
    search: gtk::SearchEntry,
    accounts: gtk::DropDown,
    list: gtk::ListBox,
    rows: RefCell<Vec<Target>>,
    filter: RefCell<Option<MessagingAccountId>>,
    title: adw::WindowTitle,
    thread: gtk::Box,
    scroll: gtk::ScrolledWindow,
    stack: gtk::Stack,
    compose: gtk::TextView,
    send: gtk::Button,
    attach: gtk::Button,
    older: gtk::Button,
    latest: gtk::Button,
    status: gtk::Label,
    draft_status: gtk::Label,
    reply: gtk::Label,
    reload: gtk::Button,
    banner: adw::Banner,
    previous_results: gtk::Button,
    next_results: gtk::Button,
    rendered: RefCell<Vec<MessageId>>,
    restoring: Cell<bool>,
    scroll_timer: RefCell<Option<glib::SourceId>>,
    last_read: RefCell<Option<MessageId>>,
    render_revision: Cell<u64>,
    search_request: Cell<Option<u64>>,
    updating: Cell<bool>,
    draft_timer: RefCell<Option<glib::SourceId>>,
    search_timer: RefCell<Option<glib::SourceId>>,
    pending_open: RefCell<Option<Target>>,
    closing: Cell<bool>,
}

impl Window {
    pub fn new(application: &adw::Application) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(application)
            .title("Handover Messages")
            .default_width(1120)
            .default_height(780)
            .width_request(740)
            .height_request(480)
            .build();
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&adw::WindowTitle::new("Handover Messages", "")));
        let menu = gio::Menu::new();
        menu.append(
            Some("Connect Google Messages…"),
            Some("win.connect-account"),
        );
        menu.append(Some("About Handover Messages"), Some("win.about"));
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .build();
        header.pack_end(&menu_button);
        toolbar.add_top_bar(&header);
        let banner = adw::Banner::new("Handover is offline. Reconnecting…");
        toolbar.add_top_bar(&banner);
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_position(320);
        paned.set_resize_start_child(false);
        paned.set_shrink_start_child(false);
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 10);
        sidebar.set_width_request(280);
        {
            let side = &sidebar;
            side.set_margin_top(12);
            side.set_margin_bottom(12);
            side.set_margin_start(12);
            side.set_margin_end(12);
        }
        let accounts = gtk::DropDown::from_strings(&["All accounts"]);
        sidebar.append(&accounts);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search saved messages"));
        search.set_tooltip_text(Some(
            "Search conversations and messages saved on this computer · Ctrl+K",
        ));
        sidebar.append(&search);
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_activate_on_single_click(true);
        list.add_css_class("navigation-sidebar");
        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .build();
        sidebar.append(&sidebar_scroll);
        let pagination = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let previous = gtk::Button::with_label("Previous results");
        let next = gtk::Button::with_label("More results");
        pagination.append(&previous);
        pagination.append(&next);
        sidebar.append(&pagination);
        paned.set_start_child(Some(&sidebar));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_hexpand(true);
        let title = adw::WindowTitle::new("Messages", "Choose a conversation");
        title.set_margin_top(14);
        title.set_margin_bottom(14);
        content.append(&title);
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        let empty = adw::StatusPage::builder()
            .title("Your conversations, on Linux")
            .description("Choose a conversation, or search messages saved on this computer.")
            .icon_name("chat-message-new-symbolic")
            .build();
        stack.add_named(&empty, Some("empty"));
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_margin_start(24);
        body.set_margin_end(24);
        body.set_margin_bottom(18);
        let thread = gtk::Box::new(gtk::Orientation::Vertical, 12);
        thread.set_margin_top(12);
        thread.set_margin_bottom(12);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&thread)
            .build();
        body.append(&scroll);
        let history_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let older = gtk::Button::with_label("Load older messages");
        let latest = gtk::Button::with_label("Back to latest");
        history_buttons.append(&older);
        history_buttons.append(&latest);
        body.append(&history_buttons);
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.add_css_class("caption");
        body.append(&status);
        let reply = gtk::Label::new(None);
        reply.set_xalign(0.0);
        reply.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let cancel_reply = gtk::Button::from_icon_name("window-close-symbolic");
        cancel_reply.set_tooltip_text(Some("Cancel reply"));
        cancel_reply.add_css_class("flat");
        let reply_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        reply.set_hexpand(true);
        reply_row.append(&reply);
        reply_row.append(&cancel_reply);
        body.append(&reply_row);
        let compose = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .top_margin(12)
            .bottom_margin(12)
            .left_margin(12)
            .right_margin(12)
            .build();
        compose.set_tooltip_text(Some("Write a message · Ctrl+Enter to send"));
        let compose_scroll = gtk::ScrolledWindow::builder()
            .min_content_height(86)
            .max_content_height(180)
            .propagate_natural_height(true)
            .child(&compose)
            .build();
        compose_scroll.add_css_class("card");
        body.append(&compose_scroll);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let attach = gtk::Button::from_icon_name("mail-attachment-symbolic");
        attach.set_tooltip_text(Some("Attach a file"));
        let draft_status = gtk::Label::new(None);
        draft_status.set_xalign(0.0);
        draft_status.set_hexpand(true);
        draft_status.set_wrap(true);
        draft_status.add_css_class("dim-label");
        draft_status.add_css_class("caption");
        let reload = gtk::Button::with_label("Reload saved draft");
        let send = gtk::Button::with_label("Send");
        send.add_css_class("suggested-action");
        footer.append(&attach);
        footer.append(&draft_status);
        footer.append(&reload);
        footer.append(&send);
        body.append(&footer);
        stack.add_named(&body, Some("thread"));
        content.append(&stack);
        paned.set_end_child(Some(&content));
        toolbar.set_content(Some(&paned));
        window.set_content(Some(&toolbar));
        let path = handover_ipc::socket_path()
            .unwrap_or_else(|_| std::path::PathBuf::from("/nonexistent/handover.sock"));
        let (commands, events) = connection::start(path);
        let this = Rc::new(Self {
            window,
            model: RefCell::new(Model::default()),
            commands,
            pending: RefCell::new(BTreeMap::new()),
            sequence: Cell::new(0),
            search,
            accounts,
            list,
            rows: RefCell::new(vec![]),
            filter: RefCell::new(None),
            title,
            thread,
            scroll,
            stack,
            compose,
            send,
            attach,
            older,
            latest,
            status,
            draft_status,
            reply,
            reload,
            banner,
            previous_results: previous.clone(),
            next_results: next.clone(),
            rendered: RefCell::new(vec![]),
            restoring: Cell::new(false),
            scroll_timer: RefCell::new(None),
            last_read: RefCell::new(None),
            render_revision: Cell::new(0),
            search_request: Cell::new(None),
            updating: Cell::new(false),
            draft_timer: RefCell::new(None),
            search_timer: RefCell::new(None),
            pending_open: RefCell::new(None),
            closing: Cell::new(false),
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
        this.search.connect_search_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.search_changed();
            }
        });
        let weak = Rc::downgrade(&this);
        this.accounts.connect_selected_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                if this.updating.get() {
                    return;
                }
                let selected = this.accounts.selected();
                *this.filter.borrow_mut() = selected.checked_sub(1).and_then(|index| {
                    this.model
                        .borrow()
                        .accounts
                        .get(index as usize)
                        .map(|a| a.id.clone())
                });
                this.model.borrow_mut().offset = 0;
                this.render_sidebar();
                this.search_now();
            }
        });
        let weak = Rc::downgrade(&this);
        this.list.connect_row_activated(move |_, row| {
            if let Some(this) = weak.upgrade() {
                let target = this.rows.borrow().get(row.index() as usize).cloned();
                if let Some((id, anchor)) = target {
                    this.open(id, anchor);
                }
            }
        });
        let weak = Rc::downgrade(&this);
        this.compose.buffer().connect_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.edited();
            }
        });
        let weak = Rc::downgrade(&this);
        this.scroll.vadjustment().connect_value_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.scrolled();
            }
        });
        let weak = Rc::downgrade(&this);
        this.window.connect_is_active_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.mark_read();
            }
        });
        let weak = Rc::downgrade(&this);
        this.send.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.send_text();
            }
        });
        let weak = Rc::downgrade(&this);
        this.attach.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.attach_file();
            }
        });
        let weak = Rc::downgrade(&this);
        cancel_reply.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.set_reply(None);
            }
        });
        let weak = Rc::downgrade(&this);
        this.older.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                let request = {
                    let model = this.model.borrow();
                    model
                        .selected
                        .clone()
                        .map(|id| Request::History(id, model.cursor.clone()))
                };
                if let Some(request) = request {
                    this.request(request);
                }
            }
        });
        let weak = Rc::downgrade(&this);
        this.latest.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                let id = this.model.borrow().selected.clone();
                if let Some(id) = id {
                    this.open(id, None);
                }
            }
        });
        let weak = Rc::downgrade(&this);
        this.reload.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                let id = this.model.borrow().selected.clone();
                if let Some(id) = id {
                    this.model.borrow_mut().drafts.remove(&id);
                    this.request(Request::Draft(id));
                }
            }
        });
        let weak = Rc::downgrade(&this);
        previous.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                let offset = this.model.borrow().offset.saturating_sub(25);
                this.model.borrow_mut().offset = offset;
                this.search_now();
            }
        });
        let weak = Rc::downgrade(&this);
        next.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade()
                && this.model.borrow().more
            {
                this.model.borrow_mut().offset += 25;
                this.search_now();
            }
        });
        let weak = Rc::downgrade(&this);
        this.window.connect_close_request(move |_| {
            if let Some(this) = weak.upgrade()
                && this
                    .model
                    .borrow()
                    .drafts
                    .values()
                    .any(|draft| draft.dirty || draft.saving)
            {
                if !this.model.borrow().connected
                    || this
                        .model
                        .borrow()
                        .drafts
                        .values()
                        .any(|draft| !draft.error.is_empty())
                {
                    this.unsaved_dialog();
                    return glib::Propagation::Stop;
                }
                this.closing.set(true);
                this.save_drafts();
                this.render_status();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        for (name, callback) in [
            ("search", 0),
            ("send", 1),
            ("previous", 2),
            ("next", 3),
            ("connect-account", 4),
            ("about", 5),
        ] {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(&this);
            action.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    match callback {
                        0 => {
                            this.search.grab_focus();
                        }
                        1 => this.send_text(),
                        2 => this.navigate(-1),
                        3 => this.navigate(1),
                        4 => this.connect_account(),
                        _ => this.about(),
                    }
                }
            });
            this.window.add_action(&action);
        }
        this.render_sidebar();
        this.render_status();
        this
    }

    fn request(&self, request: Request) -> bool {
        if matches!(request, Request::Select(_))
            && self
                .pending
                .borrow()
                .values()
                .any(|request| matches!(request, Request::Select(_)))
        {
            return true;
        }
        let id = self.sequence.get() + 1;
        self.sequence.set(id);
        if self
            .commands
            .try_send(Command {
                id,
                request: request.clone(),
            })
            .is_err()
        {
            self.model.borrow_mut().notice = "Too many requests. Try again shortly.".into();
            return false;
        }
        self.pending.borrow_mut().insert(id, request);
        true
    }

    pub fn open(self: &Rc<Self>, id: ConversationId, anchor: Option<MessageId>) {
        if !self.model.borrow().connected {
            *self.pending_open.borrow_mut() = Some((id, anchor));
            return;
        }
        self.save_drafts();
        {
            let mut model = self.model.borrow_mut();
            model.selected = Some(id.clone());
            model.anchor = anchor.clone();
            model.scroll_anchor = anchor.clone();
            model.browsing_older = false;
            model.messages.clear();
            model.cursor = None;
            model.notice = "Loading messages…".into();
        }
        self.request(Request::Draft(id.clone()));
        self.request(Request::Select(Some(MessagingLocation {
            conversation_id: id.clone(),
            message_id: anchor.clone(),
        })));
        if let Some(anchor) = anchor {
            self.request(Request::Context(anchor));
        } else {
            self.request(Request::History(id, None));
        }
        self.render_thread();
        self.render_editor();
        self.render_status();
        self.compose.grab_focus();
    }

    fn event(self: &Rc<Self>, event: Event) {
        match event {
            Event::Connected(accounts, conversations) => {
                {
                    let mut model = self.model.borrow_mut();
                    model.connected = true;
                    model.accounts = accounts;
                    model.conversations = conversations;
                    model.outgoing.clear();
                    for draft in model.drafts.values_mut() {
                        draft.loaded = false;
                        draft.saving = false;
                    }
                }
                self.request(Request::Workspace);
                self.render_sidebar();
                if let Some((id, anchor)) = self.pending_open.borrow_mut().take() {
                    self.open(id, anchor);
                } else {
                    let target = {
                        let model = self.model.borrow();
                        model
                            .selected
                            .clone()
                            .map(|id| (id, model.scroll_anchor.clone()))
                    };
                    if let Some((id, anchor)) = target {
                        self.open(id, anchor);
                    }
                }
            }
            Event::Disconnected => {
                let mut model = self.model.borrow_mut();
                model.connected = false;
                model.notice = "Handover disconnected. Submitted sends are never retried.".into();
            }
            Event::State(payload) => {
                let thread_changed = matches!(
                    *payload,
                    ServerPayload::MessageAdded { .. }
                        | ServerPayload::MessageUpdated { .. }
                        | ServerPayload::MessageRemoved { .. }
                        | ServerPayload::AccountRemoved { .. }
                );
                let snapshots = matches!(*payload, ServerPayload::Snapshot { .. });
                self.model.borrow_mut().state(*payload);
                self.render_sidebar();
                if thread_changed {
                    self.render_thread();
                }
                if snapshots {
                    self.request(Request::Workspace);
                }
            }
            Event::Reply(id, result) => {
                let Some(request) = self.pending.borrow_mut().remove(&id) else {
                    return;
                };
                if matches!(request, Request::Search { .. })
                    && self.search_request.get() != Some(id)
                {
                    return;
                }
                match result {
                    Ok(reply) => self.reply(request, reply),
                    Err(error) => self.failed(request, error),
                }
            }
        }
        self.render_editor();
        self.render_status();
        if self.closing.get() {
            if self
                .model
                .borrow()
                .drafts
                .values()
                .all(|draft| !draft.dirty && !draft.saving)
            {
                self.window.destroy();
            } else if self
                .model
                .borrow()
                .drafts
                .values()
                .any(|draft| !draft.error.is_empty())
            {
                self.unsaved_dialog();
            }
        }
    }

    fn reply(self: &Rc<Self>, request: Request, reply: Reply) {
        match reply {
            Reply::Workspace(workspace) => {
                let location = workspace.location;
                {
                    let mut model = self.model.borrow_mut();
                    model.persistent = workspace.persistent;
                    model.drafted = workspace.draft_conversations;
                }
                if matches!(request, Request::Workspace)
                    && self.model.borrow().selected.is_none()
                    && let Some(location) = location
                {
                    self.open(location.conversation_id, location.message_id);
                }
                if let Request::Select(submitted) = request {
                    let desired = {
                        let model = self.model.borrow();
                        model
                            .selected
                            .clone()
                            .map(|conversation_id| MessagingLocation {
                                conversation_id,
                                message_id: model.scroll_anchor.clone(),
                            })
                    };
                    if desired != submitted {
                        self.request(Request::Select(desired));
                    }
                }
                self.render_sidebar();
            }
            Reply::Draft(draft) => {
                if matches!(request, Request::SaveDraft(_)) {
                    self.model.borrow_mut().draft_saved(draft);
                    self.save_drafts();
                } else {
                    self.model.borrow_mut().draft_loaded(draft, false);
                    self.save_drafts();
                }
                self.render_sidebar();
            }
            Reply::Search(hits, more) => {
                if let Request::Search {
                    query,
                    account,
                    offset,
                } = request
                    && query == self.model.borrow().query
                    && account == *self.filter.borrow()
                    && offset == self.model.borrow().offset
                {
                    self.model.borrow_mut().hits = hits;
                    self.model.borrow_mut().more = more;
                    self.render_sidebar();
                }
            }
            Reply::History(id, messages, cursor) => {
                if self.model.borrow().selected.as_ref() == Some(&id)
                    && self.model.borrow().anchor.is_none()
                {
                    let older = matches!(request, Request::History(_, Some(_)));
                    if older {
                        self.model.borrow_mut().browsing_older = true;
                    }
                    self.model.borrow_mut().merge(messages);
                    self.model.borrow_mut().cursor = cursor;
                    self.model.borrow_mut().notice.clear();
                    self.render_thread();
                    if !older {
                        self.to_end();
                    }
                    self.mark_read();
                }
            }
            Reply::Context(id, messages) => {
                let matches = if let Request::Context(anchor) = request {
                    self.model.borrow().anchor.as_ref() == Some(&anchor)
                } else {
                    false
                };
                if matches && self.model.borrow().selected.as_ref() == Some(&id) {
                    self.model.borrow_mut().merge(messages);
                    self.model.borrow_mut().notice = "Viewing saved message context".into();
                    self.render_thread();
                }
            }
            Reply::Sent(operation) => {
                let id = match &request {
                    Request::Send(draft) => Some(draft.conversation_id.clone()),
                    Request::Attachment(id, _) => Some(id.clone()),
                    _ => None,
                };
                if let Request::Send(sent) = request {
                    self.model.borrow_mut().accepted(&sent);
                    self.save_drafts();
                }
                if let Some(id) = id {
                    let mut model = self.model.borrow_mut();
                    model.send_notices.remove(&id);
                    model.last_sent.insert(id, operation);
                    model.notice.clear();
                }
            }
            Reply::Accepted => {}
        }
    }

    fn failed(&self, request: Request, error: Failure) {
        match request {
            Request::SaveDraft(sent) => {
                if let Some(draft) = self
                    .model
                    .borrow_mut()
                    .drafts
                    .get_mut(&sent.conversation_id)
                {
                    draft.saving = false;
                    draft.error = error.message;
                    draft.conflict = error.code == Some(handover_ipc::ErrorCode::DraftConflict);
                }
            }
            Request::Send(MessageDraft {
                conversation_id, ..
            })
            | Request::Attachment(conversation_id, _) => {
                let message = if error.uncertain {
                    "Send outcome unknown. Check the conversation before sending again.".into()
                } else {
                    error.message
                };
                let mut model = self.model.borrow_mut();
                model
                    .send_notices
                    .insert(conversation_id.clone(), message.clone());
                if model.selected.as_ref() == Some(&conversation_id) {
                    model.notice = message;
                }
            }
            _ => self.model.borrow_mut().notice = error.message,
        }
    }
    fn search_changed(self: &Rc<Self>) {
        if self.updating.get() {
            return;
        }
        {
            let mut model = self.model.borrow_mut();
            model.query = self.search.text().to_string();
            model.offset = 0;
            model.hits.clear();
            model.more = false;
        }
        self.render_sidebar();
        if let Some(timer) = self.search_timer.borrow_mut().take() {
            timer.remove();
        }
        let weak = Rc::downgrade(self);
        *self.search_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            std::time::Duration::from_millis(250),
            move || {
                if let Some(this) = weak.upgrade() {
                    this.search_timer.borrow_mut().take();
                    this.search_now();
                }
            },
        ));
    }
    fn search_now(&self) {
        let request = {
            let model = self.model.borrow();
            (model.connected && !model.query.trim().is_empty()).then(|| Request::Search {
                query: model.query.clone(),
                account: self.filter.borrow().clone(),
                offset: model.offset,
            })
        };
        if let Some(request) = request
            && self.request(request)
        {
            self.search_request.set(Some(self.sequence.get()));
        }
    }
    fn edited(self: &Rc<Self>) {
        if self.updating.get() {
            return;
        }
        let (buffer, id, reply) = {
            let model = self.model.borrow();
            (
                self.compose.buffer(),
                model.selected.clone(),
                model
                    .selected_draft()
                    .and_then(|draft| draft.value.reply_to.clone()),
            )
        };
        if let Some(id) = id.clone() {
            self.model.borrow_mut().edit(
                id.clone(),
                buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string(),
                reply,
            );
        }
        if let Some(id) = id
            && let Some(draft) = self.model.borrow_mut().drafts.get_mut(&id)
            && validate_message_draft(&draft.value).is_err()
        {
            draft.error =
                "Messages are limited to 8000 characters; shorten the draft to save or send it."
                    .into();
        }
        self.render_status();
        if let Some(timer) = self.draft_timer.borrow_mut().take() {
            timer.remove();
        }
        let weak = Rc::downgrade(self);
        *self.draft_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            std::time::Duration::from_millis(350),
            move || {
                if let Some(this) = weak.upgrade() {
                    this.draft_timer.borrow_mut().take();
                    this.save_drafts();
                }
            },
        ));
    }
    fn save_drafts(&self) {
        let drafts = {
            let model = self.model.borrow();
            if !model.connected {
                return;
            }
            model
                .drafts
                .values()
                .filter(|draft| {
                    draft.loaded
                        && draft.dirty
                        && !draft.saving
                        && !draft.conflict
                        && draft.error.is_empty()
                })
                .map(|draft| draft.value.clone())
                .collect::<Vec<_>>()
        };
        for draft in drafts {
            if self.request(Request::SaveDraft(draft.clone())) {
                self.model
                    .borrow_mut()
                    .drafts
                    .get_mut(&draft.conversation_id)
                    .unwrap()
                    .saving = true;
            }
        }
    }
    fn send_text(&self) {
        if !self.send.is_sensitive() {
            return;
        }
        let draft = self
            .model
            .borrow()
            .selected_draft()
            .map(|draft| draft.value.clone());
        if let Some(draft) = draft {
            {
                let mut model = self.model.borrow_mut();
                model.send_notices.remove(&draft.conversation_id);
                model.last_sent.remove(&draft.conversation_id);
            }
            self.model.borrow_mut().notice = "Submitting…".into();
            self.request(Request::Send(draft));
            self.render_status();
        }
    }
    fn set_reply(self: &Rc<Self>, reply: Option<MessageId>) {
        let value = self
            .model
            .borrow()
            .selected_draft()
            .map(|draft| draft.value.clone());
        if let Some(value) = value {
            self.model
                .borrow_mut()
                .edit(value.conversation_id, value.text, reply);
            self.save_drafts();
            self.render_editor();
            self.compose.grab_focus();
        }
    }
    fn mark_read(&self) {
        let adjustment = self.scroll.vadjustment();
        if !self.window.is_active()
            || adjustment.value() + adjustment.page_size() < adjustment.upper() - 2.0
        {
            return;
        }
        let request = {
            let model = self.model.borrow();
            if model.can(MessagingCapability::ReadReceipts)
                && model.anchor.is_none()
                && !model.browsing_older
            {
                model
                    .messages
                    .last()
                    .filter(|message| self.last_read.borrow().as_ref() != Some(&message.id))
                    .map(|message| {
                        Request::Read(message.id.conversation_id.clone(), Some(message.id.clone()))
                    })
            } else {
                None
            }
        };
        if let Some(request) = request
            && self.request(request.clone())
            && let Request::Read(_, message) = request
        {
            *self.last_read.borrow_mut() = message;
        }
    }
    fn navigate(self: &Rc<Self>, direction: i32) {
        let selected = self
            .list
            .selected_row()
            .map(|row| row.index())
            .unwrap_or(-1);
        let index = (selected + direction).max(0);
        if let Some(row) = self.list.row_at_index(index) {
            self.list.select_row(Some(&row));
            let target = self.rows.borrow().get(index as usize).cloned();
            if let Some((id, anchor)) = target {
                self.open(id, anchor);
            }
        }
    }
    fn render_sidebar(&self) {
        self.updating.set(true);
        let selected = self
            .list
            .selected_row()
            .and_then(|row| self.rows.borrow().get(row.index() as usize).cloned());
        let model = self.model.borrow();
        let mut labels = vec!["All accounts".to_string()];
        labels.extend(model.accounts.iter().map(|account| {
            format!(
                "{}{}",
                account.label,
                if account.connected && account.authenticated {
                    ""
                } else {
                    " · offline"
                }
            )
        }));
        let strings = gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        self.accounts.set_model(Some(&strings));
        self.accounts.set_selected(
            self.filter
                .borrow()
                .as_ref()
                .and_then(|id| model.accounts.iter().position(|a| &a.id == id))
                .map_or(0, |index| index as u32 + 1),
        );
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::new();
        let query = model.query.to_lowercase();
        let mut conversations = model
            .conversations
            .iter()
            .filter(|c| {
                self.filter
                    .borrow()
                    .as_ref()
                    .is_none_or(|id| &c.id.account_id == id)
            })
            .collect::<Vec<_>>();
        conversations.sort_by_key(|c| std::cmp::Reverse(c.last_activity_at));
        for conversation in conversations {
            let title = model.label(&conversation.id);
            if !query.is_empty() && !title.to_lowercase().contains(&query) {
                continue;
            }
            let draft = model
                .drafts
                .get(&conversation.id)
                .filter(|draft| !draft.value.text.is_empty())
                .map(|draft| draft.value.text.clone());
            let preview = draft
                .as_ref()
                .map(|text| format!("Draft · {text}"))
                .unwrap_or_else(|| {
                    if model.drafted.contains(&conversation.id) {
                        "Draft saved".into()
                    } else {
                        String::new()
                    }
                });
            self.add_row(
                &title,
                &preview,
                conversation.unread_count.is_some_and(|count| count > 0),
            );
            rows.push((conversation.id.clone(), None));
        }
        for hit in &model.hits {
            self.add_row(
                &model.label(&hit.message_id.conversation_id),
                &hit.snippet,
                false,
            );
            rows.push((
                hit.message_id.conversation_id.clone(),
                Some(hit.message_id.clone()),
            ));
        }
        if rows.is_empty() {
            self.add_row(
                if model.accounts.is_empty() {
                    "No messaging accounts"
                } else if query.is_empty() {
                    "No conversations yet"
                } else {
                    "No saved matches"
                },
                if model.accounts.is_empty() {
                    "Connect an account from the menu to get started."
                } else {
                    "Only history saved on this computer is searched."
                },
                false,
            );
        }
        let selected =
            selected.or_else(|| model.selected.clone().map(|id| (id, model.anchor.clone())));
        let index = selected.and_then(|target| rows.iter().position(|row| row == &target));
        *self.rows.borrow_mut() = rows;
        if let Some(index) = index
            && let Some(row) = self.list.row_at_index(index as i32)
        {
            self.list.select_row(Some(&row));
        }
        self.previous_results.set_visible(!query.is_empty());
        self.next_results.set_visible(!query.is_empty());
        self.previous_results
            .set_sensitive(model.connected && model.offset > 0);
        self.next_results
            .set_sensitive(model.connected && model.more);
        self.updating.set(false);
    }
    fn add_row(&self, title: &str, subtitle: &str, unread: bool) {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
        row.set_margin_top(10);
        row.set_margin_bottom(10);
        row.set_margin_start(8);
        row.set_margin_end(8);
        let heading = gtk::Label::new(Some(title));
        heading.set_xalign(0.0);
        heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
        heading.add_css_class(if unread { "heading" } else { "body" });
        row.append(&heading);
        let detail = gtk::Label::new(Some(subtitle));
        detail.set_xalign(0.0);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        detail.add_css_class("dim-label");
        detail.add_css_class("caption");
        row.append(&detail);
        self.list.append(&row);
    }
    fn render_editor(&self) {
        self.updating.set(true);
        let model = self.model.borrow();
        let text = model
            .selected_draft()
            .map_or("", |draft| draft.value.text.as_str());
        let buffer = self.compose.buffer();
        if buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .as_str()
            != text
        {
            buffer.set_text(text);
        }
        self.reply.set_label(
            if model
                .selected_draft()
                .is_some_and(|draft| draft.value.reply_to.is_some())
            {
                "Replying to a message"
            } else {
                ""
            },
        );
        self.updating.set(false);
    }
    fn render_status(&self) {
        let model = self.model.borrow();
        self.banner.set_revealed(!model.connected);
        self.stack
            .set_visible_child_name(if model.selected.is_some() {
                "thread"
            } else {
                "empty"
            });
        let title = model
            .selected
            .as_ref()
            .map(|id| model.label(id))
            .unwrap_or_else(|| "Messages".into());
        self.title.set_title(&title);
        let account = model
            .selected
            .as_ref()
            .and_then(|id| model.accounts.iter().find(|a| a.id == id.account_id));
        self.title.set_subtitle(
            &account
                .map(|a| {
                    format!(
                        "{} · {}",
                        a.label,
                        if model.connected && a.connected && a.authenticated {
                            "Connected"
                        } else {
                            "Offline · you can still write a draft"
                        }
                    )
                })
                .unwrap_or_default(),
        );
        let pending = self.pending.borrow().values().any(|request| match request {
            Request::Send(draft) => Some(&draft.conversation_id) == model.selected.as_ref(),
            Request::Attachment(id, _) => Some(id) == model.selected.as_ref(),
            _ => false,
        });
        let draft = model.selected_draft();
        let editable = draft.is_some_and(|draft| draft.loaded);
        self.compose.set_sensitive(editable);
        self.send.set_sensitive(
            model.can(MessagingCapability::Text)
                && !pending
                && draft.is_some_and(|draft| {
                    !draft.value.text.trim().is_empty()
                        && !draft.conflict
                        && validate_message_draft(&draft.value).is_ok()
                        && (draft.value.reply_to.is_none()
                            || model.can(MessagingCapability::Replies))
                }),
        );
        self.attach
            .set_sensitive(model.can(MessagingCapability::Media) && !pending);
        self.older
            .set_visible(model.anchor.is_none() && model.cursor.is_some());
        self.older.set_sensitive(model.connected);
        self.latest
            .set_visible(model.anchor.is_some() || model.browsing_older);
        self.reload
            .set_visible(draft.is_some_and(|draft| !draft.error.is_empty()));
        self.reload
            .set_sensitive(model.connected && !draft.is_some_and(|draft| draft.saving));
        self.draft_status.set_label(
            &draft
                .map(|draft| {
                    if !draft.error.is_empty() {
                        draft.error.clone()
                    } else if !model.connected && draft.dirty {
                        "Draft stays in this window until Handover reconnects".into()
                    } else if draft.saving {
                        "Saving draft…".into()
                    } else if draft.dirty {
                        "Draft not saved yet".into()
                    } else if model.persistent {
                        "Draft saved on this computer · Ctrl+Enter to send".into()
                    } else {
                        "Draft held for this daemon session · Ctrl+Enter to send".into()
                    }
                })
                .unwrap_or_default(),
        );
        let send_notice = model
            .selected
            .as_ref()
            .and_then(|id| model.send_notices.get(id));
        let sent = model
            .selected
            .as_ref()
            .and_then(|id| model.last_sent.get(id));
        let status = if let Some(notice) = send_notice {
            notice.clone()
        } else if let Some(sent) = sent {
            model
                .outgoing
                .get(sent)
                .map(|operation| outcome_label(&operation.outcome).to_owned())
                .unwrap_or_else(|| "Accepted; delivery is not confirmed".into())
        } else if !model.notice.is_empty() {
            model.notice.clone()
        } else {
            model
                .outcome()
                .map(|outcome| format!("Latest outgoing message: {}", outcome_label(outcome)))
                .unwrap_or_default()
        };
        self.status.set_label(&status);
    }
    fn render_thread(self: &Rc<Self>) {
        let generation = self.render_revision.get() + 1;
        self.render_revision.set(generation);
        self.restoring.set(true);
        let adjustment = self.scroll.vadjustment();
        let previous_y = adjustment.value();
        let following = self.model.borrow().anchor.is_none()
            && !self.model.borrow().browsing_older
            && previous_y + adjustment.page_size() >= adjustment.upper() - 2.0;
        let mut previous = None;
        let mut child = self.thread.first_child();
        let mut index = 0;
        while let Some(widget) = child {
            let y = f64::from(widget.allocation().y());
            if y + f64::from(widget.height()) >= previous_y {
                previous = self
                    .rendered
                    .borrow()
                    .get(index)
                    .cloned()
                    .map(|id| (id, y - previous_y));
                break;
            }
            child = widget.next_sibling();
            index += 1;
        }
        while let Some(child) = self.thread.first_child() {
            self.thread.remove(&child);
        }
        let model = self.model.borrow();
        for message in &model.messages {
            let bubble = gtk::Box::new(gtk::Orientation::Vertical, 6);
            bubble.add_css_class("card");
            bubble.set_margin_start(if message.sender.is_self { 80 } else { 0 });
            bubble.set_margin_end(if message.sender.is_self { 0 } else { 80 });
            let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);
            inner.set_margin_top(12);
            inner.set_margin_bottom(12);
            inner.set_margin_start(14);
            inner.set_margin_end(14);
            let sender = gtk::Label::new(Some(if message.sender.is_self {
                "You"
            } else {
                message
                    .sender
                    .display_name
                    .as_deref()
                    .or(message.sender.address.as_deref())
                    .unwrap_or("Contact")
            }));
            sender.set_xalign(0.0);
            sender.add_css_class("caption");
            sender.add_css_class("dim-label");
            inner.append(&sender);
            let text = gtk::Label::new(Some(if message.deleted {
                "Message deleted"
            } else {
                message.text.as_deref().unwrap_or("")
            }));
            text.set_xalign(0.0);
            text.set_wrap(true);
            text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            text.set_selectable(true);
            inner.append(&text);
            for attachment in message.attachments.iter().filter(|_| !message.deleted) {
                let button = gtk::Button::with_label(
                    &attachment
                        .name
                        .clone()
                        .unwrap_or_else(|| "Attachment".into()),
                );
                button.set_sensitive(attachment.staged_path.is_some());
                let path = attachment.staged_path.clone();
                button.connect_clicked(move |_| {
                    if let Some(path) = &path {
                        let _ = gio::AppInfo::launch_default_for_uri(
                            &gio::File::for_path(path).uri(),
                            None::<&gio::AppLaunchContext>,
                        );
                    }
                });
                inner.append(&button);
            }
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let timestamp = message
                .sent_at
                .and_then(|time| glib::DateTime::from_unix_local(time / 1_000_000).ok())
                .and_then(|time| time.format("%b %e · %H:%M").ok())
                .map(|time| time.to_string())
                .unwrap_or_default();
            let time = gtk::Label::new(Some(&timestamp));
            time.add_css_class("caption");
            time.add_css_class("dim-label");
            time.set_hexpand(true);
            time.set_xalign(0.0);
            actions.append(&time);
            let reply = gtk::Button::with_label("Reply");
            reply.add_css_class("flat");
            reply.set_sensitive(!message.deleted && model.can(MessagingCapability::Replies));
            let id = message.id.clone();
            let weak = Rc::downgrade(self);
            reply.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.set_reply(Some(id.clone()));
                }
            });
            actions.append(&reply);
            inner.append(&actions);
            if model.anchor.as_ref() == Some(&message.id) {
                bubble.add_css_class("accent");
            }
            bubble.append(&inner);
            self.thread.append(&bubble);
        }
        *self.rendered.borrow_mut() = model
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect();
        let target = model.anchor.clone().map(|id| (id, 0.0)).or(previous);
        drop(model);
        let weak = Rc::downgrade(self);
        let frame = Cell::new(0);
        self.scroll.add_tick_callback(move |_, _| {
            frame.set(frame.get() + 1);
            if frame.get() < 2 {
                return glib::ControlFlow::Continue;
            }
            if let Some(this) = weak.upgrade() {
                if this.render_revision.get() != generation {
                    return glib::ControlFlow::Break;
                }
                let adjustment = this.scroll.vadjustment();
                if following {
                    adjustment.set_value((adjustment.upper() - adjustment.page_size()).max(0.0));
                } else if let Some((id, offset)) = &target
                    && let Some(index) = this.rendered.borrow().iter().position(|known| known == id)
                {
                    let mut child = this.thread.first_child();
                    for _ in 0..index {
                        child = child.and_then(|widget| widget.next_sibling());
                    }
                    if let Some(child) = child {
                        adjustment.set_value((f64::from(child.allocation().y()) - offset).max(0.0));
                    }
                }
                this.restoring.set(false);
            }
            glib::ControlFlow::Break
        });
    }
    fn to_end(self: &Rc<Self>) {
        let generation = self.render_revision.get();
        let weak = Rc::downgrade(self);
        let frame = Cell::new(0);
        self.scroll.add_tick_callback(move |_, _| {
            frame.set(frame.get() + 1);
            if frame.get() < 2 {
                return glib::ControlFlow::Continue;
            }
            if let Some(this) = weak.upgrade() {
                if this.render_revision.get() != generation {
                    return glib::ControlFlow::Break;
                }
                let adjustment = this.scroll.vadjustment();
                adjustment.set_value((adjustment.upper() - adjustment.page_size()).max(0.0));
                this.mark_read();
            }
            glib::ControlFlow::Break
        });
    }
    fn scrolled(self: &Rc<Self>) {
        if self.restoring.get() || self.updating.get() {
            return;
        }
        if let Some(timer) = self.scroll_timer.borrow_mut().take() {
            timer.remove();
        }
        let weak = Rc::downgrade(self);
        *self.scroll_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            std::time::Duration::from_millis(350),
            move || {
                if let Some(this) = weak.upgrade() {
                    this.scroll_timer.borrow_mut().take();
                    let adjustment = this.scroll.vadjustment();
                    let mut anchor = None;
                    if adjustment.value() + adjustment.page_size() < adjustment.upper() - 2.0 {
                        let mut widget = this.thread.first_child();
                        let mut index = 0;
                        while let Some(child) = widget {
                            if f64::from(child.allocation().y() + child.height())
                                >= adjustment.value()
                            {
                                anchor = this.rendered.borrow().get(index).cloned();
                                break;
                            }
                            widget = child.next_sibling();
                            index += 1;
                        }
                    }
                    let location = {
                        let mut model = this.model.borrow_mut();
                        model.scroll_anchor = anchor.clone();
                        if !model.connected {
                            return;
                        }
                        model
                            .selected
                            .clone()
                            .map(|conversation_id| MessagingLocation {
                                conversation_id,
                                message_id: anchor,
                            })
                    };
                    if let Some(location) = location {
                        this.request(Request::Select(Some(location)));
                    }
                    this.mark_read();
                }
            },
        ));
    }
    fn attach_file(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title("Attach a file").build();
        let weak = Rc::downgrade(self);
        dialog.open(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                if let (Some(this), Ok(file)) = (weak.upgrade(), result) {
                    let id = this.model.borrow().selected.clone();
                    if let Some(id) = id {
                        this.request(Request::Attachment(id, file.uri().to_string()));
                        this.render_status();
                    }
                }
            },
        );
    }
    fn unsaved_dialog(self: &Rc<Self>) {
        self.closing.set(false);
        let dialog = adw::MessageDialog::new(
            Some(&self.window),
            Some("Draft could not be saved"),
            Some("Keep editing, or close and discard changes that were not saved to Handover."),
        );
        dialog.add_responses(&[
            ("keep", "Keep editing"),
            ("discard", "Discard unsaved changes"),
        ]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("keep"));
        dialog.set_close_response("keep");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response == "discard"
                && let Some(this) = weak.upgrade()
            {
                this.window.destroy();
            }
        });
        dialog.present();
    }
    fn connect_account(self: &Rc<Self>) {
        crate::setup::present(&self.window);
    }
    fn about(&self) {
        let dialog=adw::AboutWindow::builder().transient_for(&self.window).application_name("Handover Messages").version(env!("CARGO_PKG_VERSION"))
        .developer_name("Handover contributors").license_type(gtk::License::MitX11).comments("Your phone messages on Linux. Search saved history and keep your drafts across restarts.").build();
        dialog.present();
    }
}

use handover_ipc::ServerPayload;

#[cfg(test)]
mod tests;
