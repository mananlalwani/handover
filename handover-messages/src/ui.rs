use crate::{
    connection::{self, Command, Event, Failure, Reply, Request},
    model::{Model, outcome_label},
};
use adw::prelude::*;
use gtk::{gio, glib};
use handover_core::*;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    rc::Rc,
};

type Target = (ConversationId, Option<MessageId>);

#[derive(Clone, PartialEq)]
struct RowStamp {
    previous: Option<(String, bool, Option<i64>)>,
    group: bool,
    replies: bool,
    reactions: bool,
    quote: Option<String>,
    delivery: Option<&'static str>,
    find_query: String,
    sender: String,
}

struct RecentThread {
    id: ConversationId,
    messages: Vec<Message>,
    cursor: Option<String>,
}

struct SidebarPreview {
    conversation: ConversationId,
    label: glib::WeakRef<gtk::Label>,
    text: String,
}

pub struct Window {
    pub window: adw::ApplicationWindow,
    model: RefCell<Model>,
    recent_threads: RefCell<VecDeque<RecentThread>>,
    cached_messages: RefCell<BTreeSet<MessageId>>,
    photos: RefCell<crate::contact_photos::ContactPhotos>,
    media: crate::media::Media,
    commands: tokio::sync::mpsc::Sender<Command>,
    pending: RefCell<BTreeMap<u64, Request>>,
    sequence: Cell<u64>,
    search: gtk::SearchEntry,
    find: Rc<crate::conversation_find::Find>,
    accounts: gtk::DropDown,
    list: gtk::ListBox,
    rows: RefCell<Vec<Target>>,
    sidebar_previews: RefCell<Vec<SidebarPreview>>,
    sidebar_timer: RefCell<Option<glib::SourceId>>,
    filter: RefCell<Option<MessagingAccountId>>,
    title: gtk::Label,
    subtitle: gtk::Label,
    avatar: adw::Avatar,
    details: gtk::Popover,
    media_button: gtk::Button,
    media_browser: RefCell<Option<glib::WeakRef<gtk::Window>>>,
    connection_status: gtk::Box,
    previewed: RefCell<BTreeSet<ConversationId>>,
    placeholder: gtk::Label,
    reply_row: gtk::Box,
    attachments: RefCell<BTreeMap<ConversationId, crate::attachments::PendingAttachment>>,
    attachment_row: gtk::Box,
    attachment_state: RefCell<BTreeMap<ConversationId, MessageAttachmentDraft>>,
    attachment_errors: RefCell<BTreeMap<ConversationId, String>>,
    quotes: RefCell<BTreeMap<MessageId, Option<Message>>>,
    reaction_error: RefCell<Option<(MessageId, String)>>,
    thread: gtk::Box,
    scroll: gtk::ScrolledWindow,
    stack: gtk::Stack,
    compose: gtk::TextView,
    send: gtk::Button,
    attach: gtk::Button,
    history_indicator: gtk::Box,
    history_spinner: gtk::Spinner,
    history_label: gtk::Label,
    history_retry: gtk::Button,
    history_error: RefCell<Option<String>>,
    history_cursors: RefCell<VecDeque<String>>,
    history_epoch: Cell<u64>,
    history_epochs: RefCell<BTreeMap<u64, u64>>,
    empty_history_pages: Cell<u8>,
    loading_latest: Cell<bool>,
    jump_pending: Cell<bool>,
    latest: gtk::Button,
    status: gtk::Label,
    typing_indicator: gtk::Label,
    composer_typing: gtk::Label,
    draft_status: gtk::Label,
    reply: gtk::Label,
    reload: gtk::Button,
    banner: adw::Banner,
    previous_results: gtk::Button,
    next_results: gtk::Button,
    rendered: RefCell<Vec<MessageId>>,
    row_stamps: RefCell<BTreeMap<MessageId, RowStamp>>,
    dirty_messages: RefCell<BTreeSet<MessageId>>,
    restoring: Cell<bool>,
    scroll_timer: RefCell<Option<glib::SourceId>>,
    last_read: RefCell<Option<MessageId>>,
    render_revision: Cell<u64>,
    search_request: Cell<Option<u64>>,
    contacts_request: Cell<Option<u64>>,
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
            .default_width(1160)
            .default_height(780)
            .width_request(740)
            .height_request(480)
            .build();
        window.add_css_class("handover-messages");
        let provider = gtk::CssProvider::new();
        provider.load_from_data(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &gtk::prelude::WidgetExt::display(&window),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        let toolbar = adw::ToolbarView::new();
        let banner = adw::Banner::new("Handover is offline. Reconnecting…");
        toolbar.add_top_bar(&banner);
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_position(300);
        paned.set_resize_start_child(false);
        paned.set_shrink_start_child(false);
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.set_width_request(260);
        sidebar.add_css_class("messages-sidebar");
        let sidebar_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        sidebar_header.add_css_class("app-chrome");
        let sidebar_handle = gtk::WindowHandle::new();
        sidebar_handle.set_child(Some(&sidebar_header));
        let brand = gtk::Label::new(Some("Messages"));
        brand.add_css_class("sidebar-title");
        sidebar_header.append(&brand);
        let sidebar_space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sidebar_space.set_hexpand(true);
        sidebar_header.append(&sidebar_space);
        let menu = gio::Menu::new();
        menu.append(
            Some("Connect Google Messages…"),
            Some("win.connect-account"),
        );
        menu.append(Some("Reconnect"), Some("win.reconnect"));
        menu.append(Some("About Handover Messages"), Some("win.about"));
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .tooltip_text("Main menu")
            .build();
        sidebar_header.append(&menu_button);
        sidebar.append(&sidebar_handle);
        let tools = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tools.add_css_class("sidebar-tools");
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search"));
        search.set_tooltip_text(Some("Search conversations and saved messages · Ctrl+K"));
        tools.append(&search);
        sidebar.append(&tools);
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_activate_on_single_click(true);
        list.add_css_class("navigation-sidebar");
        list.add_css_class("conversation-list");
        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .build();
        sidebar.append(&sidebar_scroll);
        let pagination = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        pagination.set_halign(gtk::Align::Center);
        let previous = gtk::Button::with_label("Previous");
        let next = gtk::Button::with_label("More results");
        previous.add_css_class("flat");
        next.add_css_class("flat");
        pagination.append(&previous);
        pagination.append(&next);
        sidebar.append(&pagination);
        let accounts = gtk::DropDown::from_strings(&["All accounts"]);
        accounts.add_css_class("account-picker");
        accounts.set_tooltip_text(Some("Filter conversations by account"));
        let account_area = gtk::Box::new(gtk::Orientation::Vertical, 4);
        account_area.add_css_class("account-area");
        account_area.append(&accounts);
        let filters = gtk::Popover::new();
        let filter_title = gtk::Label::new(Some("Accounts"));
        filter_title.add_css_class("heading");
        filter_title.set_margin_top(12);
        account_area.prepend(&filter_title);
        filters.set_child(Some(&account_area));
        let filter_button = gtk::MenuButton::builder()
            .label("Accounts")
            .tooltip_text("Filter accounts")
            .popover(&filters)
            .build();
        sidebar_header.insert_child_after(&filter_button, Some(&sidebar_space));
        paned.set_start_child(Some(&sidebar));
        let find = crate::conversation_find::Find::new(
            handover_ipc::socket_path().expect("runtime directory"),
        );
        let content = adw::ToolbarView::new();
        content.add_css_class("conversation-pane");
        content.set_hexpand(true);
        let content_header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content_header.add_css_class("app-chrome");
        content_header.add_css_class("thread-chrome");
        let content_handle = gtk::WindowHandle::new();
        content_handle.set_child(Some(&content_header));
        let title = gtk::Label::new(Some("Messages"));
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.add_css_class("contact-name");
        let subtitle = gtk::Label::new(None);
        subtitle.set_xalign(0.0);
        subtitle.add_css_class("contact-subtitle");
        let typing_indicator = gtk::Label::new(None);
        typing_indicator.add_css_class("typing-indicator");
        typing_indicator.set_xalign(0.0);
        typing_indicator.set_ellipsize(gtk::pango::EllipsizeMode::End);
        typing_indicator.set_max_width_chars(28);
        typing_indicator.set_single_line_mode(true);
        typing_indicator.set_visible(false);
        let identity = gtk::Box::new(gtk::Orientation::Vertical, 2);
        identity.set_valign(gtk::Align::Center);
        identity.append(&title);
        identity.append(&subtitle);
        identity.append(&typing_indicator);
        let avatar = adw::Avatar::new(40, None, true);
        avatar.set_visible(false);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        heading.append(&avatar);
        heading.append(&identity);
        heading.add_css_class("contact-heading");
        content_header.append(&heading);
        let thread_space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        thread_space.set_hexpand(true);
        content_header.append(&thread_space);
        let connection_status = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        connection_status.set_size_request(6, 6);
        connection_status.set_valign(gtk::Align::Center);
        connection_status.add_css_class("connection-indicator");
        content_header.append(&connection_status);
        let find_button = gtk::Button::from_icon_name("system-search-symbolic");
        find_button.set_tooltip_text(Some("Find in conversation (Ctrl+F)"));
        find_button.add_css_class("flat");
        content_header.append(&find_button);
        let media_button = gtk::Button::from_icon_name("folder-pictures-symbolic");
        media_button.set_tooltip_text(Some("Photos and files"));
        media_button.add_css_class("flat");
        content_header.append(&media_button);
        let details = gtk::Popover::new();
        let details_button = gtk::MenuButton::builder()
            .label("Details")
            .tooltip_text("Conversation details")
            .popover(&details)
            .build();
        content_header.append(&details_button);
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.set_tooltip_text(Some("Close window"));
        close.add_css_class("chrome-close");
        let weak_window = window.downgrade();
        close.connect_clicked(move |_| {
            if let Some(window) = weak_window.upgrade() {
                window.close();
            }
        });
        content_header.append(&close);
        content.add_top_bar(&content_handle);
        content.add_top_bar(&find.bar);
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 12);
        empty.set_halign(gtk::Align::Center);
        empty.set_valign(gtk::Align::Center);
        let icon = gtk::Image::from_icon_name("chat-message-new-symbolic");
        icon.set_pixel_size(36);
        icon.add_css_class("dim-label");
        empty.append(&icon);
        let welcome = gtk::Label::new(Some("Choose a conversation"));
        welcome.add_css_class("empty-title");
        empty.append(&welcome);
        let help = gtk::Label::new(Some("Your messages, photos and files appear here."));
        help.add_css_class("dim-label");
        empty.append(&help);
        stack.add_named(&empty, Some("empty"));
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let thread = gtk::Box::new(gtk::Orientation::Vertical, 0);
        thread.add_css_class("message-thread");
        thread.set_valign(gtk::Align::End);
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(840);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&thread));
        clamp.set_vexpand(true);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .build();
        let viewport = gtk::Overlay::new();
        viewport.set_vexpand(true);
        viewport.set_child(Some(&scroll));
        let history_indicator = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        history_indicator.add_css_class("history-indicator");
        history_indicator.set_halign(gtk::Align::Center);
        history_indicator.set_valign(gtk::Align::Start);
        history_indicator.set_margin_top(10);
        let history_spinner = gtk::Spinner::new();
        let history_label = gtk::Label::new(None);
        history_label.set_wrap(true);
        history_label.set_max_width_chars(32);
        let history_retry = gtk::Button::with_label("Retry");
        history_retry.add_css_class("flat");
        history_indicator.append(&history_spinner);
        history_indicator.append(&history_label);
        history_indicator.append(&history_retry);
        viewport.add_overlay(&history_indicator);
        let latest = gtk::Button::with_label("Latest messages ↓");
        latest.add_css_class("jump-latest");
        latest.set_tooltip_text(Some("Jump to the latest messages"));
        latest.set_halign(gtk::Align::End);
        latest.set_valign(gtk::Align::End);
        latest.set_margin_end(24);
        latest.set_margin_bottom(10);
        viewport.add_overlay(&latest);
        body.append(&viewport);
        let composer_area = gtk::Box::new(gtk::Orientation::Vertical, 6);
        composer_area.add_css_class("composer-area");
        let composer_typing = gtk::Label::new(None);
        composer_typing.add_css_class("typing-indicator");
        composer_typing.add_css_class("composer-typing-indicator");
        composer_typing.set_xalign(0.0);
        composer_typing.set_margin_start(40);
        composer_typing.set_single_line_mode(true);
        composer_typing.set_ellipsize(gtk::pango::EllipsizeMode::End);
        composer_typing.set_height_request(18);
        composer_area.append(&composer_typing);
        let status = gtk::Label::new(None);
        status.set_xalign(0.5);
        status.set_wrap(true);
        status.add_css_class("composer-note");

        let reply = gtk::Label::new(None);
        reply.set_xalign(0.0);
        reply.set_hexpand(true);
        reply.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let cancel_reply = gtk::Button::from_icon_name("window-close-symbolic");
        cancel_reply.set_tooltip_text(Some("Cancel reply"));
        cancel_reply.add_css_class("flat");
        cancel_reply.add_css_class("circular");
        let reply_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        reply_row.add_css_class("reply-preview");
        reply_row.append(&reply);
        reply_row.append(&cancel_reply);
        composer_area.append(&reply_row);
        let attachment_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        attachment_row.add_css_class("attachment-preview");
        composer_area.append(&attachment_row);
        let composer = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        composer.add_css_class("composer");
        let attach = gtk::Button::from_icon_name("list-add-symbolic");
        attach.set_tooltip_text(Some("Attach a file"));
        attach.add_css_class("flat");
        attach.add_css_class("circular");
        attach.set_valign(gtk::Align::Center);
        let composer_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        composer_row.add_css_class("composer-row");
        composer_row.append(&attach);
        let compose = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .top_margin(6)
            .bottom_margin(6)
            .left_margin(8)
            .right_margin(8)
            .build();
        compose.set_tooltip_text(Some("Write a message · Ctrl+Enter to send"));
        let compose_scroll = gtk::ScrolledWindow::builder()
            .min_content_height(18)
            .max_content_height(160)
            .vscrollbar_policy(gtk::PolicyType::External)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .hexpand(true)
            .child(&compose)
            .build();
        let overlay = gtk::Overlay::new();
        overlay.set_hexpand(true);
        overlay.set_child(Some(&compose_scroll));
        let placeholder = gtk::Label::new(Some("Message"));
        placeholder.add_css_class("composer-placeholder");
        placeholder.set_halign(gtk::Align::Start);
        placeholder.set_valign(gtk::Align::Center);
        placeholder.set_can_target(false);
        overlay.add_overlay(&placeholder);
        composer.append(&overlay);
        let send = gtk::Button::from_icon_name("go-up-symbolic");
        send.set_tooltip_text(Some("Send message · Ctrl+Enter"));
        send.add_css_class("suggested-action");
        send.add_css_class("circular");
        send.set_valign(gtk::Align::Center);
        composer.append(&send);
        composer.set_hexpand(true);
        composer_row.append(&composer);
        composer_area.append(&composer_row);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let draft_status = gtk::Label::new(None);
        draft_status.set_xalign(0.0);
        draft_status.set_hexpand(true);
        draft_status.set_wrap(true);
        draft_status.add_css_class("composer-note");
        let reload = gtk::Button::with_label("Reload saved draft");
        reload.add_css_class("flat");
        footer.append(&draft_status);
        footer.append(&status);
        footer.append(&reload);
        composer_area.append(&footer);
        let composer_clamp = adw::Clamp::new();
        composer_clamp.set_maximum_size(840);
        composer_clamp.set_tightening_threshold(600);
        composer_clamp.set_child(Some(&composer_area));
        body.append(&composer_clamp);
        stack.add_named(&body, Some("thread"));
        content.set_content(Some(&stack));
        paned.set_end_child(Some(&content));
        toolbar.set_content(Some(&paned));
        window.set_content(Some(&toolbar));
        let path = handover_ipc::socket_path()
            .unwrap_or_else(|_| std::path::PathBuf::from("/nonexistent/handover.sock"));
        let (commands, events) = connection::start(path);
        let this = Rc::new(Self {
            window,
            model: RefCell::new(Model::default()),
            recent_threads: RefCell::new(VecDeque::new()),
            cached_messages: RefCell::new(BTreeSet::new()),
            photos: RefCell::new(crate::contact_photos::ContactPhotos::default()),
            media: crate::media::Media::new(),
            commands,
            pending: RefCell::new(BTreeMap::new()),
            sequence: Cell::new(0),
            search,
            find,
            accounts,
            list,
            rows: RefCell::new(vec![]),
            sidebar_previews: RefCell::new(vec![]),
            sidebar_timer: RefCell::new(None),
            filter: RefCell::new(None),
            title,
            subtitle,
            avatar,
            details,
            media_button,
            media_browser: RefCell::new(None),
            connection_status,
            previewed: RefCell::new(BTreeSet::new()),
            placeholder,
            reply_row,
            attachments: RefCell::new(BTreeMap::new()),
            attachment_row,
            attachment_state: RefCell::new(BTreeMap::new()),
            attachment_errors: RefCell::new(BTreeMap::new()),
            quotes: RefCell::new(BTreeMap::new()),
            reaction_error: RefCell::new(None),
            thread,
            scroll,
            stack,
            compose,
            send,
            attach,
            history_indicator,
            history_spinner,
            history_label,
            history_retry,
            history_error: RefCell::new(None),
            history_cursors: RefCell::new(VecDeque::new()),
            history_epoch: Cell::new(0),
            history_epochs: RefCell::new(BTreeMap::new()),
            empty_history_pages: Cell::new(0),
            loading_latest: Cell::new(false),
            jump_pending: Cell::new(false),
            latest,
            status,
            typing_indicator,
            composer_typing,
            draft_status,
            reply,
            reload,
            banner,
            previous_results: previous.clone(),
            next_results: next.clone(),
            rendered: RefCell::new(vec![]),
            row_stamps: RefCell::new(BTreeMap::new()),
            dirty_messages: RefCell::new(BTreeSet::new()),
            restoring: Cell::new(false),
            scroll_timer: RefCell::new(None),
            last_read: RefCell::new(None),
            render_revision: Cell::new(0),
            search_request: Cell::new(None),
            contacts_request: Cell::new(None),
            updating: Cell::new(false),
            draft_timer: RefCell::new(None),
            search_timer: RefCell::new(None),
            pending_open: RefCell::new(None),
            closing: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.find.set_on_select(move |id| {
            if let Some(this) = weak.upgrade() {
                this.open(id.conversation_id.clone(), Some(id));
                this.find.entry.grab_focus();
            }
        });
        let weak = Rc::downgrade(&this);
        this.find.set_on_query(move |_| {
            if let Some(this) = weak.upgrade() {
                this.render_thread();
            }
        });
        let weak = Rc::downgrade(&this);
        find_button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.find.show();
            }
        });
        let weak = Rc::downgrade(&this);
        glib::spawn_future_local(async move {
            let mut turn = std::time::Instant::now();
            while let Ok(event) = events.recv().await {
                let Some(this) = weak.upgrade() else {
                    break;
                };
                this.event(event);
                // A continuously ready channel must still let GTK dispatch
                // input, redraws, and the compositor's responsiveness checks.
                if turn.elapsed() >= std::time::Duration::from_millis(8) {
                    glib::timeout_future(std::time::Duration::from_millis(1)).await;
                    turn = std::time::Instant::now();
                }
            }
        });
        let weak = Rc::downgrade(&this);
        sidebar_scroll
            .vadjustment()
            .connect_value_changed(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.media.refresh();
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
                if !this.window.is_active() {
                    this.last_read.borrow_mut().take();
                }
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
        this.history_retry.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.history_error.borrow_mut().take();
                this.history_cursors.borrow_mut().clear();
                this.empty_history_pages.set(0);
                if this.model.borrow().messages.is_empty() {
                    let selected = this.model.borrow().selected.clone();
                    if let Some(selected) = selected {
                        let anchor = this.model.borrow().anchor.clone();
                        this.open(selected, anchor);
                    }
                } else {
                    this.load_older(true);
                }
            }
        });
        let weak = Rc::downgrade(&this);
        this.latest.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                let id = this.model.borrow().selected.clone();
                if let Some(id) = id {
                    if this.model.borrow().anchor.is_none() && !this.model.borrow().newer_gap {
                        this.model.borrow_mut().browsing_older = false;
                        this.to_end();
                    } else {
                        this.open(id, None);
                    }
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
        this.media_button.connect_clicked({
            let weak = Rc::downgrade(&this);
            move |_| {
                if let Some(this) = weak.upgrade() {
                    if let Some(browser) = this
                        .media_browser
                        .borrow()
                        .as_ref()
                        .and_then(|window| window.upgrade())
                        .filter(|window| window.is_visible())
                    {
                        browser.close();
                    }
                    let model = this.model.borrow();
                    if let (Some(id), Ok(path)) =
                        (model.selected.clone(), handover_ipc::socket_path())
                    {
                        let browser = crate::media_browser::open(
                            &this.window,
                            &this.media,
                            &model.messages,
                            id,
                            path,
                        );
                        *this.media_browser.borrow_mut() = Some(browser.downgrade());
                    }
                }
            }
        });
        let drop_target = gtk::DropTarget::new(
            gtk::gdk::FileList::static_type(),
            gtk::gdk::DragAction::COPY,
        );
        let weak = Rc::downgrade(&this);
        drop_target.connect_drop(move |_, value, _, _| {
            let Some(this) = weak.upgrade() else {
                return false;
            };
            let Ok(files) = value.get::<gtk::gdk::FileList>() else {
                return false;
            };
            let files = files.files();
            if files.len() != 1 {
                this.model.borrow_mut().notice = "Attach one file per message.".into();
                this.render_status();
                return false;
            }
            let id = this.model.borrow().selected.clone();
            id.is_some_and(|id| this.stage_file(id, &files[0]))
        });
        this.stack.add_controller(drop_target);
        let weak = Rc::downgrade(&this);
        this.window.connect_close_request(move |_| {
            if let Some(this) = weak.upgrade() {
                if this.any_attachment_saving() {
                    this.closing.set(true);
                    this.save_drafts();
                    return glib::Propagation::Stop;
                }
                if this
                    .attachments
                    .borrow()
                    .keys()
                    .any(|id| this.unsaved_attachment(id))
                    || this.attachment_errors.borrow().keys().any(|id| {
                        this.attachment_state
                            .borrow()
                            .get(id)
                            .is_some_and(|state| state.attachment.is_some())
                    })
                {
                    this.unsaved_dialog();
                    return glib::Propagation::Stop;
                }
            }
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
            ("find", 6),
            ("reconnect", 7),
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
                        6 => this.find.show(),
                        7 => this.reconnect(),
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

    fn reconnect(&self) {
        let accounts = {
            let model = self.model.borrow();
            if !model.connected {
                return;
            }
            let selected = model
                .selected
                .as_ref()
                .map(|id| id.account_id.clone())
                .or_else(|| self.filter.borrow().clone());
            model
                .accounts
                .iter()
                .filter(|account| selected.as_ref().is_none_or(|id| id == &account.id))
                .map(|account| account.id.clone())
                .collect::<Vec<_>>()
        };
        for account in accounts {
            if !self
                .pending
                .borrow()
                .values()
                .any(|request| matches!(request, Request::Sync(id) if id == &account))
                && !self.request(Request::Sync(account))
            {
                break;
            }
        }
        self.render_status();
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
        if matches!(
            request,
            Request::History(..)
                | Request::Context(_)
                | Request::ReplyContext(_)
                | Request::ResolveReply(_)
        ) {
            self.history_epochs
                .borrow_mut()
                .insert(id, self.history_epoch.get());
        }
        if matches!(request, Request::Contacts) {
            self.contacts_request.set(Some(id));
        }
        self.pending.borrow_mut().insert(id, request);
        true
    }

    pub fn open(self: &Rc<Self>, id: ConversationId, anchor: Option<MessageId>) {
        if !self.model.borrow().connected {
            *self.pending_open.borrow_mut() = Some((id, anchor));
            return;
        }
        {
            let model = self.model.borrow();
            if let Some(current) = model.selected.as_ref()
                && model.anchor.is_none()
                && !model.newer_gap
                && !model.messages.is_empty()
                && !self.loading_latest.get()
            {
                let mut recent = self.recent_threads.borrow_mut();
                recent.retain(|thread| &thread.id != current);
                recent.push_back(RecentThread {
                    id: current.clone(),
                    messages: model.messages.clone(),
                    cursor: model.cursor.clone(),
                });
                while recent.len() > 8 {
                    recent.pop_front();
                }
            }
        }
        let cached = if anchor.is_none() {
            self.recent_threads
                .borrow()
                .iter()
                .find(|thread| thread.id == id)
                .map(|thread| (thread.messages.clone(), thread.cursor.clone()))
        } else {
            None
        };
        let cached_visible = cached.is_some();
        self.find.set_conversation(Some(id.clone()));
        self.save_drafts();
        self.quotes.borrow_mut().clear();
        self.history_epoch.set(self.history_epoch.get() + 1);
        self.history_epochs.borrow_mut().clear();
        self.history_cursors.borrow_mut().clear();
        self.history_error.borrow_mut().take();
        self.empty_history_pages.set(0);
        self.loading_latest.set(anchor.is_none());
        self.jump_pending.set(false);
        {
            let mut model = self.model.borrow_mut();
            model.newer_gap = false;
            model.clear_new_messages();
            model.selected = Some(id.clone());
            model.anchor = anchor.clone();
            model.scroll_anchor = anchor.clone();
            model.browsing_older = false;
            let (messages, cursor) = cached.unwrap_or_default();
            // Fresh history replaces these provisional rows, but not live updates.
            *self.cached_messages.borrow_mut() =
                messages.iter().map(|message| message.id.clone()).collect();
            model.messages = messages;
            model.cursor = cursor;
            model.notice = if cached_visible {
                ""
            } else {
                "Loading messages…"
            }
            .into();
        }
        self.request(Request::Draft(id.clone()));
        self.request(Request::AttachmentDraft(id.clone()));
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
        if cached_visible {
            self.to_end();
        }
        self.render_editor();
        self.render_attachments();
        self.render_status();
        self.compose.grab_focus();
    }

    fn event(self: &Rc<Self>, event: Event) {
        match event {
            Event::Connected(accounts, conversations, typing, read) => {
                self.last_read.borrow_mut().take();
                self.recent_threads.borrow_mut().clear();
                {
                    let mut model = self.model.borrow_mut();
                    model.connected = true;
                    model.accounts = accounts;
                    model.conversations = conversations;
                    model.reset_typing(typing);
                    model.reset_read(read);
                    model.statuses.clear();
                    model.outgoing.clear();
                    model.previews.clear();
                    self.previewed.borrow_mut().clear();
                    for draft in model.drafts.values_mut() {
                        draft.loaded = false;
                        draft.saving = false;
                    }
                }
                self.request(Request::Workspace);
                self.request(Request::Contacts);
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
                self.recent_threads.borrow_mut().clear();
                let mut model = self.model.borrow_mut();
                model.connected = false;
                model.typing.clear();
                model.notice = "Handover disconnected. Submitted sends are never retried.".into();
            }
            Event::State(payload) => {
                match payload.as_ref() {
                    ServerPayload::MessageAdded { message }
                    | ServerPayload::MessageUpdated { message } => {
                        self.cached_messages.borrow_mut().remove(&message.id);
                    }
                    ServerPayload::MessageRemoved { message_id } => {
                        self.cached_messages.borrow_mut().remove(message_id);
                    }
                    _ => {}
                }
                self.recent_threads
                    .borrow_mut()
                    .retain(|thread| match payload.as_ref() {
                        ServerPayload::Snapshot { .. } => false,
                        ServerPayload::AccountRemoved { account_id } => {
                            &thread.id.account_id != account_id
                        }
                        ServerPayload::AccountUpdated { account }
                            if !account.connected || !account.authenticated =>
                        {
                            thread.id.account_id != account.id
                        }
                        ServerPayload::ConversationRemoved { conversation_id } => {
                            &thread.id != conversation_id
                        }
                        ServerPayload::MessageAdded { message }
                        | ServerPayload::MessageUpdated { message } => {
                            thread.id != message.id.conversation_id
                        }
                        ServerPayload::MessageRemoved { message_id } => {
                            thread.id != message_id.conversation_id
                        }
                        _ => true,
                    });
                if !matches!(
                    payload.as_ref(),
                    ServerPayload::Snapshot { .. }
                        | ServerPayload::AccountAdded { .. }
                        | ServerPayload::AccountUpdated { .. }
                        | ServerPayload::AccountRemoved { .. }
                        | ServerPayload::ConversationAdded { .. }
                        | ServerPayload::ConversationUpdated { .. }
                        | ServerPayload::ConversationRemoved { .. }
                        | ServerPayload::MessageAdded { .. }
                        | ServerPayload::MessageUpdated { .. }
                        | ServerPayload::MessageRemoved { .. }
                        | ServerPayload::MessageStatus { .. }
                        | ServerPayload::OutgoingOperations { .. }
                        | ServerPayload::OutgoingOperation { .. }
                        | ServerPayload::OutgoingOperationRemoved { .. }
                        | ServerPayload::Typing { .. }
                        | ServerPayload::ContactsSynced { .. }
                        | ServerPayload::ContactsRemoved { .. }
                        | ServerPayload::ReadState { .. }
                ) {
                    return;
                }
                if matches!(payload.as_ref(), ServerPayload::Typing { .. }) {
                    self.model.borrow_mut().state(*payload);
                    self.render_typing();
                    return;
                }
                let thread_changed = {
                    let model = self.model.borrow();
                    let selected = model.selected.as_ref();
                    match payload.as_ref() {
                        ServerPayload::MessageAdded { message }
                        | ServerPayload::MessageUpdated { message } => {
                            selected == Some(&message.id.conversation_id)
                        }
                        ServerPayload::MessageRemoved { message_id } => {
                            selected == Some(&message_id.conversation_id)
                        }
                        ServerPayload::MessageStatus { update } => {
                            selected == Some(&update.message_id.conversation_id)
                        }
                        ServerPayload::ConversationUpdated { conversation } => {
                            selected == Some(&conversation.id)
                        }
                        ServerPayload::AccountUpdated { account } => {
                            selected.is_some_and(|id| id.account_id == account.id)
                        }
                        ServerPayload::AccountRemoved { account_id } => {
                            selected.is_some_and(|id| &id.account_id == account_id)
                        }
                        ServerPayload::OutgoingOperation { operation } => {
                            selected == Some(&operation.conversation_id)
                        }
                        ServerPayload::OutgoingOperations { operations, .. } => operations
                            .iter()
                            .any(|operation| selected == Some(&operation.conversation_id)),
                        ServerPayload::OutgoingOperationRemoved { operation_id } => model
                            .outgoing
                            .get(operation_id)
                            .is_some_and(|operation| selected == Some(&operation.conversation_id)),
                        ServerPayload::Snapshot { .. } => true,
                        _ => false,
                    }
                };
                let sidebar_changed = matches!(
                    payload.as_ref(),
                    ServerPayload::Snapshot { .. }
                        | ServerPayload::AccountAdded { .. }
                        | ServerPayload::AccountUpdated { .. }
                        | ServerPayload::AccountRemoved { .. }
                        | ServerPayload::ConversationAdded { .. }
                        | ServerPayload::ConversationUpdated { .. }
                        | ServerPayload::ConversationRemoved { .. }
                        | ServerPayload::MessageAdded { .. }
                        | ServerPayload::MessageUpdated { .. }
                        | ServerPayload::MessageRemoved { .. }
                        | ServerPayload::ContactsSynced { .. }
                        | ServerPayload::ContactsRemoved { .. }
                        | ServerPayload::ReadState { .. }
                );
                let contacts_changed = matches!(
                    *payload,
                    ServerPayload::ContactsSynced { .. } | ServerPayload::ContactsRemoved { .. }
                );
                let snapshots = matches!(*payload, ServerPayload::Snapshot { .. });
                if contacts_changed || snapshots {
                    self.contacts_request.set(None);
                }
                if let ServerPayload::MessageAdded { message }
                | ServerPayload::MessageUpdated { message } = payload.as_ref()
                {
                    self.note_message_changes(std::slice::from_ref(message));
                    if self.quotes.borrow().contains_key(&message.id)
                        || (self.quotes.borrow().len() < 256 && self.pending.borrow().values().any(
                            |request| matches!(request, Request::Quote(id) if id == &message.id)))
                    {
                        self.quotes
                            .borrow_mut()
                            .insert(message.id.clone(), Some(message.clone()));
                    }
                }
                if let ServerPayload::MessageRemoved { message_id } = payload.as_ref() {
                    let referenced = self
                        .model
                        .borrow()
                        .messages
                        .iter()
                        .any(|message| message.reply_to.as_ref() == Some(message_id));
                    if self.quotes.borrow().contains_key(message_id)
                        || (referenced && self.quotes.borrow().len() < 256)
                    {
                        self.quotes.borrow_mut().insert(message_id.clone(), None);
                    }
                }
                if let ServerPayload::AccountRemoved { account_id } = payload.as_ref() {
                    self.attachments
                        .borrow_mut()
                        .retain(|id, _| &id.account_id != account_id);
                    self.attachment_state
                        .borrow_mut()
                        .retain(|id, _| &id.account_id != account_id);
                    self.attachment_errors
                        .borrow_mut()
                        .retain(|id, _| &id.account_id != account_id);
                }
                self.model.borrow_mut().state(*payload);
                if sidebar_changed {
                    self.schedule_sidebar();
                }
                if thread_changed {
                    self.render_thread();
                }
                if contacts_changed || snapshots {
                    self.request(Request::Contacts);
                }
                if snapshots {
                    self.previewed.borrow_mut().clear();
                    self.request(Request::Workspace);
                }
            }
            Event::Reply(id, result) => {
                let Some(request) = self.pending.borrow_mut().remove(&id) else {
                    return;
                };
                if matches!(
                    request,
                    Request::History(..)
                        | Request::Context(_)
                        | Request::ReplyContext(_)
                        | Request::ResolveReply(_)
                ) && self.history_epochs.borrow_mut().remove(&id)
                    != Some(self.history_epoch.get())
                {
                    return;
                }
                if matches!(request, Request::Contacts) && self.contacts_request.get() != Some(id) {
                    return;
                }
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
        self.warm_previews();
        self.render_editor();
        self.render_status();
        if self.closing.get() {
            if self
                .model
                .borrow()
                .drafts
                .values()
                .all(|draft| !draft.dirty && !draft.saving)
                && !self.any_attachment_saving()
                && !self
                    .attachments
                    .borrow()
                    .keys()
                    .any(|id| self.unsaved_attachment(id))
                && self.attachment_errors.borrow().is_empty()
            {
                self.window.destroy();
            } else if self
                .model
                .borrow()
                .drafts
                .values()
                .any(|draft| !draft.error.is_empty())
                || (!self.any_attachment_saving() && !self.attachment_errors.borrow().is_empty())
            {
                self.unsaved_dialog();
            }
        }
    }

    fn reply(self: &Rc<Self>, request: Request, reply: Reply) {
        match reply {
            Reply::Workspace(workspace) => {
                let location = workspace.location;
                let sidebar_changed = self.model.borrow().drafted != workspace.draft_conversations;
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
                if sidebar_changed {
                    self.schedule_sidebar();
                }
            }
            Reply::Draft(draft) => {
                let id = draft.conversation_id.clone();
                let previous_text = self
                    .model
                    .borrow()
                    .drafts
                    .get(&id)
                    .map(|draft| draft.value.text.clone())
                    .unwrap_or_default();
                if matches!(request, Request::SaveDraft(_)) {
                    self.model.borrow_mut().draft_saved(draft);
                    self.save_drafts();
                } else {
                    self.model.borrow_mut().draft_loaded(draft, false);
                    self.save_drafts();
                }
                if self
                    .model
                    .borrow()
                    .drafts
                    .get(&id)
                    .map(|draft| draft.value.text.as_str())
                    .unwrap_or("")
                    != previous_text
                {
                    self.schedule_sidebar();
                }
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
                let older = matches!(request, Request::History(_, Some(_)));
                if self.model.borrow().selected.as_ref() == Some(&id)
                    && (older || self.model.borrow().anchor.is_none())
                {
                    if older {
                        let advances = messages.iter().any(|message| {
                            !self
                                .model
                                .borrow()
                                .messages
                                .iter()
                                .any(|old| old.id == message.id)
                        });
                        self.empty_history_pages.set(if advances {
                            0
                        } else {
                            self.empty_history_pages.get().saturating_add(1)
                        });
                        self.model.borrow_mut().browsing_older = true;
                        if self.empty_history_pages.get() >= 3 && cursor.is_some() {
                            *self.history_error.borrow_mut() =
                                Some("Couldn't load earlier messages. Try again.".into());
                        }
                    }
                    self.note_message_changes(&messages);
                    if !older {
                        // Event handlers remove live updates from this set.
                        let cached = self.cached_messages.take();
                        self.model
                            .borrow_mut()
                            .messages
                            .retain(|message| !cached.contains(&message.id));
                    }
                    self.model.borrow_mut().merge(messages);
                    self.model.borrow_mut().cursor = cursor;
                    self.model.borrow_mut().notice.clear();
                    self.render_thread();
                    if !older {
                        self.to_end();
                    }
                    self.mark_read();
                    if !older {
                        self.schedule_sidebar();
                    }
                }
            }
            Reply::Context(id, messages, cursor) => {
                let matches = match &request {
                    Request::Context(anchor) => self.model.borrow().anchor.as_ref() == Some(anchor),
                    Request::ReplyContext(anchor)
                        if self.model.borrow().selected.as_ref() == Some(&id)
                            && messages.iter().any(|message| &message.id == anchor) =>
                    {
                        self.history_epoch.set(self.history_epoch.get() + 1);
                        self.history_epochs.borrow_mut().clear();
                        self.quotes.borrow_mut().clear();
                        self.history_cursors.borrow_mut().clear();
                        self.history_error.borrow_mut().take();
                        self.cached_messages.borrow_mut().clear();
                        self.loading_latest.set(false);
                        let mut model = self.model.borrow_mut();
                        model.anchor = Some(anchor.clone());
                        model.scroll_anchor = Some(anchor.clone());
                        model.messages.clear();
                        model.browsing_older = true;
                        model.newer_gap = false;
                        drop(model);
                        self.request(Request::Select(Some(MessagingLocation {
                            conversation_id: id.clone(),
                            message_id: Some(anchor.clone()),
                        })));
                        true
                    }
                    _ => false,
                };
                if matches && self.model.borrow().selected.as_ref() == Some(&id) {
                    self.note_message_changes(&messages);
                    self.model.borrow_mut().merge(messages);
                    self.model.borrow_mut().cursor = cursor;
                    self.model.borrow_mut().notice = "Viewing saved message context".into();
                    self.render_thread();
                }
            }
            Reply::Sent(operation) => {
                let id = match &request {
                    Request::Send(draft) => Some(draft.conversation_id.clone()),
                    Request::Attachment(draft, ..) => Some(draft.conversation_id.clone()),
                    _ => None,
                };
                if let Request::Attachment(sent, uri, revision) = &request {
                    let mut attachments = self.attachments.borrow_mut();
                    if attachments
                        .get(&sent.conversation_id)
                        .is_some_and(|item| &item.uri == uri)
                    {
                        attachments.remove(&sent.conversation_id);
                    }
                    drop(attachments);
                    self.request(Request::SaveAttachmentDraft(
                        sent.conversation_id.clone(),
                        None,
                        None,
                        *revision,
                    ));
                }
                if let Request::Send(sent) | Request::Attachment(sent, ..) = request {
                    self.model.borrow_mut().accepted(&sent);
                    self.save_drafts();
                }
                self.render_attachments();
                if let Some(id) = id {
                    let mut model = self.model.borrow_mut();
                    model.send_notices.remove(&id);
                    model.last_sent.insert(id, operation);
                    model.notice.clear();
                }
            }
            Reply::AttachmentDraft(saved) => {
                let id = saved.conversation_id.clone();
                let had_attachment = self.attachments.borrow().contains_key(&id);
                if !self
                    .model
                    .borrow()
                    .accounts
                    .iter()
                    .any(|account| account.id == id.account_id)
                {
                    return;
                }
                let dirty = self.unsaved_attachment(&id);
                let old_revision = self
                    .attachment_state
                    .borrow()
                    .get(&id)
                    .map_or(0, |state| state.revision);
                let loading = matches!(request, Request::AttachmentDraft(_));
                self.attachment_state
                    .borrow_mut()
                    .insert(id.clone(), saved.clone());
                if loading && dirty {
                    self.attachment_errors.borrow_mut().insert(id, if old_revision == saved.revision {
                        "Attachment was not saved. Retry saving it.".into()
                    } else { "Attachment changed in another window. Reload the saved attachment before continuing.".into() });
                } else {
                    self.attachment_errors.borrow_mut().remove(&id);
                    if let Some(attachment) = saved.attachment {
                        if let Some(path) = attachment.staged_path.as_ref() {
                            self.attachments.borrow_mut().insert(
                                id,
                                crate::attachments::PendingAttachment {
                                    uri: gio::File::for_path(path).uri().to_string(),
                                    attachment,
                                },
                            );
                        }
                    } else {
                        self.attachments.borrow_mut().remove(&id);
                    }
                }
                if self.attachment_state.borrow().len() > 256 {
                    let selected = self.model.borrow().selected.clone();
                    self.attachment_state.borrow_mut().retain(|id, _| {
                        Some(id) == selected.as_ref() || self.attachments.borrow().contains_key(id)
                    });
                }
                self.render_attachments();
                if had_attachment
                    != self
                        .attachments
                        .borrow()
                        .contains_key(&saved.conversation_id)
                {
                    self.schedule_sidebar();
                }
            }
            Reply::Quote(id, message) => {
                if self.model.borrow().selected.as_ref() == Some(&id.conversation_id)
                    && self.quotes.borrow().len() < 256
                {
                    if let Some(message) = message {
                        self.quotes.borrow_mut().insert(id, Some(*message));
                    } else {
                        self.quotes.borrow_mut().entry(id).or_insert(None);
                    }
                    self.render_thread();
                }
            }
            Reply::Resolved(source, message) => {
                if self.model.borrow().selected.as_ref() != Some(&source.conversation_id) {
                    return;
                }
                if let Some(message) = message {
                    let target = message.reply_to.clone();
                    let mut model = self.model.borrow_mut();
                    if let Some(saved) = model.messages.iter_mut().find(|saved| saved.id == source)
                    {
                        *saved = *message;
                    }
                    drop(model);
                    self.render_thread();
                    if let Some(target) = target {
                        if self.rendered.borrow().contains(&target) {
                            self.jump_to(&target);
                        } else {
                            self.request(Request::ReplyContext(target));
                        }
                    } else {
                        self.model.borrow_mut().notice =
                            "The phone no longer supplies this reply's original.".into();
                    }
                } else {
                    self.model.borrow_mut().notice =
                        "The phone could not return this reply. Your conversation is still open."
                            .into();
                }
            }
            Reply::Contacts(contacts) => {
                if self.photos.borrow_mut().replace(contacts) {
                    self.schedule_sidebar();
                    self.render_thread();
                }
            }
            Reply::Previews(messages) => {
                if let Request::Previews(ids) = request {
                    let mut model = self.model.borrow_mut();
                    for message in messages {
                        if ids.contains(&message.id.conversation_id)
                            && model
                                .conversations
                                .iter()
                                .any(|conversation| conversation.id == message.id.conversation_id)
                        {
                            model.remember_preview(&message);
                        }
                    }
                    drop(model);
                    self.schedule_sidebar();
                }
            }
            Reply::Accepted => {
                if matches!(request, Request::Sync(_)) {
                    self.model.borrow_mut().notice = "Reconnect requested.".into();
                }
            }
        }
    }

    fn failed(self: &Rc<Self>, request: Request, error: Failure) {
        let attachment_error = matches!(
            &request,
            Request::AttachmentDraft(_) | Request::SaveAttachmentDraft(..)
        );
        match request {
            Request::Read(_, message) => {
                if *self.last_read.borrow() == message {
                    self.last_read.borrow_mut().take();
                }
                self.model.borrow_mut().notice = error.message;
            }
            Request::Context(anchor) if self.model.borrow().anchor.as_ref() == Some(&anchor) => {
                *self.history_error.borrow_mut() = Some(error.message);
                self.model.borrow_mut().notice.clear();
                self.loading_latest.set(false);
            }
            Request::History(id, _) if self.model.borrow().selected.as_ref() == Some(&id) => {
                *self.history_error.borrow_mut() = Some(error.message);
                self.model.borrow_mut().notice.clear();
                self.loading_latest.set(false);
            }
            Request::ReplyContext(id) => {
                if self.model.borrow().selected.as_ref() == Some(&id.conversation_id) {
                    self.model.borrow_mut().notice =
                        "The original message is unavailable. Your conversation is still open."
                            .into();
                }
            }
            Request::ResolveReply(id) => {
                if self.model.borrow().selected.as_ref() == Some(&id.conversation_id) {
                    self.model.borrow_mut().notice =
                        "Couldn't refresh this reply. Your conversation is still open.".into();
                }
            }
            Request::Quote(id) => {
                if self.model.borrow().selected.as_ref() == Some(&id.conversation_id)
                    && self.quotes.borrow().len() < 256
                {
                    self.quotes.borrow_mut().entry(id).or_insert(None);
                }
            }
            Request::AttachmentDraft(id) | Request::SaveAttachmentDraft(id, ..) => {
                if !self
                    .model
                    .borrow()
                    .accounts
                    .iter()
                    .any(|account| account.id == id.account_id)
                {
                    return;
                }
                self.attachment_errors
                    .borrow_mut()
                    .insert(id, error.message);
            }
            Request::Previews(_) | Request::Contacts => {}
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
            Request::React(id, ..) => {
                let message = if error.uncertain {
                    "Reaction outcome unknown. Check the message before trying again.".into()
                } else {
                    error.message
                };
                *self.reaction_error.borrow_mut() = Some((id, message));
            }
            Request::Send(MessageDraft {
                conversation_id, ..
            })
            | Request::Attachment(
                MessageDraft {
                    conversation_id, ..
                },
                ..,
            ) => {
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
        if attachment_error {
            if self.attachment_errors.borrow().len() > 256 {
                let selected = self.model.borrow().selected.clone();
                self.attachment_errors.borrow_mut().retain(|id, _| {
                    Some(id) == selected.as_ref() || self.attachments.borrow().contains_key(id)
                });
            }
            self.render_attachments();
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
    fn quote_label(&self, model: &Model, id: &MessageId) -> String {
        let quotes = self.quotes.borrow();
        let original = model
            .messages
            .iter()
            .find(|message| &message.id == id)
            .or_else(|| quotes.get(id).and_then(Option::as_ref));
        match original {
            Some(message) => {
                let sender = if message.sender.is_self {
                    "You".to_owned()
                } else {
                    self.photos.borrow().label(&message.sender)
                };
                let text = if message.deleted {
                    "Message deleted"
                } else {
                    message
                        .text
                        .as_deref()
                        .filter(|text| !text.is_empty())
                        .unwrap_or({
                            if message.attachments.is_empty() {
                                "Message"
                            } else {
                                "Attachment"
                            }
                        })
                };
                format!("{}\n{}", sender, text.chars().take(240).collect::<String>())
            }
            None if model.messages.iter().any(|message| {
                message.reply_to.as_ref() == Some(id) && message.reply_preview.is_some()
            }) =>
            {
                let preview = model
                    .messages
                    .iter()
                    .find(|message| {
                        message.reply_to.as_ref() == Some(id) && message.reply_preview.is_some()
                    })
                    .and_then(|message| message.reply_preview.as_deref())
                    .unwrap_or_default();
                format!("Reply\n{}", preview.chars().take(240).collect::<String>())
            }
            None if !quotes.contains_key(id) => "Loading original message…".into(),
            None => "Original message unavailable".into(),
        }
    }
    fn jump_to(self: &Rc<Self>, target: &MessageId) {
        let index = self.rendered.borrow().iter().position(|id| id == target);
        if let Some(index) = index {
            if let Some(widget) = self.thread.first_child().and_then(|first| {
                let mut current = first;
                for _ in 0..index {
                    current = current.next_sibling()?;
                }
                Some(current)
            }) {
                self.model.borrow_mut().browsing_older = true;
                self.scroll
                    .vadjustment()
                    .set_value(message_bounds(&widget, &self.thread).0.max(0.0));
                widget.add_css_class("quote-target");
                let weak = widget.downgrade();
                glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                    if let Some(widget) = weak.upgrade() {
                        widget.remove_css_class("quote-target");
                    }
                });
            }
        } else {
            self.open(target.conversation_id.clone(), Some(target.clone()));
        }
    }
    fn jump_to_reply(self: &Rc<Self>, source: &MessageId, target: &MessageId) {
        if self.rendered.borrow().contains(target) {
            self.jump_to(target);
        } else if self
            .quotes
            .borrow()
            .get(target)
            .is_some_and(Option::is_some)
        {
            self.request(Request::ReplyContext(target.clone()));
        } else if !self
            .pending
            .borrow()
            .values()
            .any(|request| matches!(request, Request::ResolveReply(id) if id == source))
        {
            self.request(Request::ResolveReply(source.clone()));
        }
    }
    fn reaction_details(
        &self,
        model: &Model,
        message: &Message,
        reaction: &Reaction,
    ) -> (gtk::ToggleButton, gtk::Widget) {
        let button = gtk::ToggleButton::builder()
            .label(format!("{} {}", reaction.emoji, reaction.count))
            .tooltip_text("Reactions")
            .build();
        button.add_css_class("flat");
        button.add_css_class("reaction-chip");
        if model.has_own_reaction(&message.id, &reaction.emoji) {
            button.add_css_class("own-reaction");
            button.set_tooltip_text(Some(&format!("You reacted with {}", reaction.emoji)));
        }
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let conversation = model
            .conversations
            .iter()
            .find(|conversation| conversation.id == message.id.conversation_id);
        let mut actors = BTreeSet::new();
        for actor in reaction
            .participant_ids
            .iter()
            .filter(|actor| actors.insert(actor.as_str()))
            .take(256)
        {
            let participant = conversation.and_then(|conversation| {
                conversation
                    .participants
                    .iter()
                    .find(|participant| &participant.local_id == actor)
            });
            let name = participant
                .map(|participant| {
                    if participant.is_self {
                        "You".to_owned()
                    } else {
                        self.photos.borrow().label(participant)
                    }
                })
                .unwrap_or_else(|| "Unknown participant".into());
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.add_css_class("reaction-person-row");
            let avatar = adw::Avatar::new(20, Some(&name), true);
            if let Some(participant) = participant
                && let Some(photo) = self.photos.borrow().participant(participant)
            {
                self.media.avatar(&avatar, photo);
            }
            row.append(&avatar);
            let label = gtk::Label::new(Some(&name));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_max_width_chars(24);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.add_css_class("reaction-person");
            row.append(&label);
            content.append(&row);
        }
        let unknown = reaction.count.saturating_sub(actors.len() as u64);
        if unknown > 0 {
            let label = gtk::Label::new(Some(&format!(
                "{unknown} {}",
                if unknown == 1 { "other" } else { "others" }
            )));
            label.set_tooltip_text(Some("Participant details are unavailable"));
            label.add_css_class("dim-label");
            label.add_css_class("reaction-person-row");
            content.append(&label);
        }
        let people = if actors.len() + usize::from(unknown > 0) <= 4 {
            content.upcast::<gtk::Widget>()
        } else {
            gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .min_content_height(0)
                .max_content_height(120)
                .propagate_natural_height(true)
                .propagate_natural_width(true)
                .child(&content)
                .build()
                .upcast()
        };
        (button, people)
    }

    fn message_actions(self: &Rc<Self>, message: &Message) -> gtk::MenuButton {
        let menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text("Message actions")
            .valign(gtk::Align::Center)
            .build();
        menu.add_css_class("flat");
        menu.add_css_class("message-actions");
        let popover = gtk::Popover::new();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let copy = gtk::Button::with_label("Copy text");
        copy.add_css_class("flat");
        copy.set_sensitive(
            !message.deleted && message.text.as_ref().is_some_and(|text| !text.is_empty()),
        );
        let text = message.text.clone().unwrap_or_default();
        let weak = self.window.downgrade();
        let popup = popover.downgrade();
        copy.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.clipboard().set_text(&text);
            }
            if let Some(popup) = popup.upgrade() {
                popup.popdown();
            }
        });
        content.append(&copy);
        if !message.deleted {
            for link in crate::message_text::links(message.text.as_deref().unwrap_or("")) {
                let button = gtk::Button::with_label("Copy link");
                button.set_tooltip_text(Some(&link));
                button.add_css_class("flat");
                let weak = self.window.downgrade();
                let popup = popover.downgrade();
                button.connect_clicked(move |_| {
                    if let Some(window) = weak.upgrade() {
                        window.clipboard().set_text(&link);
                    }
                    if let Some(popup) = popup.upgrade() {
                        popup.popdown();
                    }
                });
                content.append(&button);
            }
            for attachment in &message.attachments {
                let button = gtk::Button::with_label(&format!(
                    "Save {}…",
                    attachment.name.as_deref().unwrap_or("attachment")
                ));
                button.set_sensitive(attachment.staged_path.is_some());
                button.add_css_class("flat");
                let weak = self.window.downgrade();
                let attachment = attachment.clone();
                let popup = popover.downgrade();
                button.connect_clicked(move |_| {
                    if let Some(parent) = weak.upgrade() {
                        crate::file_actions::save(&parent, &attachment);
                    }
                    if let Some(popup) = popup.upgrade() {
                        popup.popdown();
                    }
                });
                content.append(&button);
            }
        }
        let reply = gtk::Button::with_label("Reply");
        reply.add_css_class("flat");
        reply.set_sensitive(
            !message.deleted && self.model.borrow().can(MessagingCapability::Replies),
        );
        let weak = Rc::downgrade(self);
        let id = message.id.clone();
        let popup = popover.downgrade();
        reply.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.set_reply(Some(id.clone()));
            }
            if let Some(popup) = popup.upgrade() {
                popup.popdown();
            }
        });
        content.append(&reply);
        if !message.deleted && self.model.borrow().can(MessagingCapability::Reactions) {
            let reactions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            for emoji in ["👍", "❤️", "😂", "😮", "😢", "🙏"] {
                let button = gtk::Button::with_label(emoji);
                button.add_css_class("flat");
                let weak = Rc::downgrade(self);
                let id = message.id.clone();
                let popup = popover.downgrade();
                button.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.react(&id, emoji, false);
                    }
                    if let Some(popup) = popup.upgrade() {
                        popup.popdown();
                    }
                });
                reactions.append(&button);
            }
            content.append(&reactions);
            for reaction in message
                .reactions
                .iter()
                .filter(|reaction| {
                    self.model
                        .borrow()
                        .has_own_reaction(&message.id, &reaction.emoji)
                })
                .take(32)
            {
                let button =
                    gtk::Button::with_label(&format!("Remove my {} reaction", reaction.emoji));
                button.add_css_class("flat");
                let weak = Rc::downgrade(self);
                let id = message.id.clone();
                let emoji = reaction.emoji.clone();
                let popup = popover.downgrade();
                button.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.react(&id, &emoji, true);
                    }
                    if let Some(popup) = popup.upgrade() {
                        popup.popdown();
                    }
                });
                content.append(&button);
            }
        }
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(420)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .child(&content)
            .build();
        popover.set_child(Some(&scroll));
        menu.set_popover(Some(&popover));
        menu
    }
    fn react(&self, id: &MessageId, emoji: &str, remove: bool) {
        let model = self.model.borrow();
        if model.selected.as_ref() != Some(&id.conversation_id)
            || !model.can(MessagingCapability::Reactions)
            || (remove && !model.has_own_reaction(id, emoji))
            || self
                .pending
                .borrow()
                .values()
                .any(|request| matches!(request, Request::React(target, ..) if target == id))
        {
            return;
        }
        drop(model);
        self.reaction_error.borrow_mut().take();
        self.request(Request::React(id.clone(), emoji.into(), remove));
        self.render_status();
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
            let attachment = self
                .attachments
                .borrow()
                .get(&draft.conversation_id)
                .cloned();
            if let Some(attachment) = attachment {
                let revision = self
                    .attachment_state
                    .borrow()
                    .get(&draft.conversation_id)
                    .map_or(0, |state| state.revision);
                self.request(Request::Attachment(draft, attachment.uri, revision));
            } else {
                self.request(Request::Send(draft));
            }
            self.render_status();
        }
    }
    fn set_reply(self: &Rc<Self>, reply: Option<MessageId>) {
        if reply.is_some()
            && self
                .model
                .borrow()
                .selected
                .as_ref()
                .is_some_and(|id| self.attachments.borrow().contains_key(id))
        {
            self.model.borrow_mut().notice =
                "Remove the attachment before replying to a message.".into();
            self.render_status();
            return;
        }
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
    fn warm_previews(&self) {
        if !self.model.borrow().connected {
            return;
        }
        if self
            .pending
            .borrow()
            .values()
            .any(|request| matches!(request, Request::Previews(_)))
        {
            return;
        }
        let ids = {
            let model = self.model.borrow();
            let mut conversations = model.conversations.iter().collect::<Vec<_>>();
            conversations
                .sort_by_key(|conversation| std::cmp::Reverse(conversation.last_activity_at));
            let eligible = conversations.into_iter().take(512).collect::<Vec<_>>();
            self.previewed
                .borrow_mut()
                .retain(|id| eligible.iter().any(|conversation| &conversation.id == id));
            eligible
                .into_iter()
                .filter(|conversation| {
                    !self.previewed.borrow().contains(&conversation.id)
                        && !model.previews.contains_key(&conversation.id)
                })
                .take(20)
                .map(|conversation| conversation.id.clone())
                .collect::<Vec<_>>()
        };
        if !ids.is_empty() && self.request(Request::Previews(ids.clone())) {
            self.previewed.borrow_mut().extend(ids);
        }
    }

    fn conversation_label(&self, model: &Model, id: &ConversationId) -> String {
        if let Some(conversation) = model
            .conversations
            .iter()
            .find(|conversation| &conversation.id == id)
            && conversation
                .title
                .as_deref()
                .is_none_or(|title| title.trim().is_empty())
        {
            let names = conversation
                .participants
                .iter()
                .filter(|participant| !participant.is_self)
                .map(|participant| self.photos.borrow().label(participant))
                .collect::<Vec<_>>()
                .join(", ");
            if !names.is_empty() {
                return names;
            }
        }
        model.label(id)
    }

    fn schedule_sidebar(self: &Rc<Self>) {
        if self.sidebar_timer.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        *self.sidebar_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            std::time::Duration::from_millis(16),
            move || {
                if let Some(this) = weak.upgrade() {
                    this.sidebar_timer.borrow_mut().take();
                    this.render_sidebar();
                }
            },
        ));
    }

    fn render_sidebar(&self) {
        if let Some(timer) = self.sidebar_timer.borrow_mut().take() {
            timer.remove();
        }
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
        self.sidebar_previews.borrow_mut().clear();
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
            let title = self.conversation_label(&model, &conversation.id);
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
                    if self.attachments.borrow().contains_key(&conversation.id) {
                        "Attachment draft".into()
                    } else if model.drafted.contains(&conversation.id) {
                        "Draft saved".into()
                    } else if let Some(message) =
                        model.previews.get(&conversation.id).filter(|message| {
                            conversation
                                .latest_message_id
                                .as_ref()
                                .is_none_or(|latest| latest == &message.id)
                        })
                    {
                        if message.deleted {
                            "Message deleted".into()
                        } else {
                            let text = message
                                .text
                                .clone()
                                .filter(|text| !text.is_empty())
                                .unwrap_or_else(|| {
                                    if message.attachments.is_empty() {
                                        "Message".into()
                                    } else {
                                        "Attachment".into()
                                    }
                                });
                            if message.sender.is_self {
                                format!("You: {text}")
                            } else {
                                text
                            }
                        }
                    } else {
                        match conversation.transport {
                            TransportKind::Rcs => "RCS conversation",
                            TransportKind::Sms => "SMS conversation",
                            TransportKind::Mms => "MMS conversation",
                            _ => "Conversation",
                        }
                        .into()
                    }
                });
            let label = self.add_row(
                &title,
                &preview,
                model.unread(&conversation.id) == Some(true),
                conversation.last_activity_at,
                conversation.kind == ConversationKind::Group,
                self.photos.borrow().conversation_photos(conversation),
            );
            self.sidebar_previews.borrow_mut().push(SidebarPreview {
                conversation: conversation.id.clone(),
                label: label.downgrade(),
                text: preview.replace(['\n', '\t'], " "),
            });
            rows.push((conversation.id.clone(), None));
        }
        for hit in &model.hits {
            self.add_row(
                &self.conversation_label(&model, &hit.message_id.conversation_id),
                &hit.snippet,
                false,
                hit.sent_at,
                false,
                self.photos
                    .borrow()
                    .participant(&hit.sender)
                    .into_iter()
                    .collect(),
            );
            rows.push((
                hit.message_id.conversation_id.clone(),
                Some(hit.message_id.clone()),
            ));
        }
        if rows.is_empty() {
            let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 10);
            placeholder.set_margin_top(36);
            placeholder.set_margin_start(24);
            placeholder.set_margin_end(24);
            let icon = gtk::Image::from_icon_name(if query.is_empty() {
                "chat-message-new-symbolic"
            } else {
                "edit-find-symbolic"
            });
            icon.set_pixel_size(32);
            icon.add_css_class("dim-label");
            placeholder.append(&icon);
            let title = gtk::Label::new(Some(if model.accounts.is_empty() {
                "No messaging accounts"
            } else if query.is_empty() {
                "No conversations yet"
            } else {
                "No saved matches"
            }));
            title.add_css_class("heading");
            title.set_wrap(true);
            placeholder.append(&title);
            let description = gtk::Label::new(Some(if model.accounts.is_empty() {
                "Connect an account from the menu to get started."
            } else {
                "Only history saved on this computer is searched."
            }));
            description.set_wrap(true);
            description.set_justify(gtk::Justification::Center);
            description.add_css_class("dim-label");
            description.add_css_class("caption");
            placeholder.append(&description);
            self.list.set_placeholder(Some(&placeholder));
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
        self.media.schedule(&self.window);
        self.render_typing();
    }
    fn add_row(
        &self,
        title: &str,
        subtitle: &str,
        unread: bool,
        time: Option<i64>,
        group: bool,
        photos: Vec<std::sync::Arc<str>>,
    ) -> gtk::Label {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let avatar = adw::Avatar::new(40, Some(title), true);
        style_avatar(&avatar, title, group);
        self.media.avatar_photos(&avatar, photos);
        row.append(&avatar);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
        text.set_hexpand(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let heading = gtk::Label::new(Some(title));
        heading.set_xalign(0.0);
        heading.set_hexpand(true);
        heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
        heading.add_css_class("conversation-name");
        top.append(&heading);
        let date = gtk::Label::new(Some(&sidebar_time(time)));
        date.add_css_class("conversation-time");
        top.append(&date);
        text.append(&top);
        let detail = gtk::Label::new(Some(&subtitle.replace(['\n', '\t'], " ")));
        detail.set_xalign(0.0);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        detail.set_wrap(true);
        detail.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        detail.set_lines(2);
        detail.set_max_width_chars(28);
        detail.add_css_class("conversation-preview");
        text.append(&detail);
        row.append(&text);
        if unread {
            let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            dot.add_css_class("unread-dot");
            dot.set_valign(gtk::Align::Center);
            row.append(&dot);
        }
        self.list.append(&row);
        detail
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
        let quote = model
            .selected_draft()
            .and_then(|draft| draft.value.reply_to.as_ref());
        self.reply.set_label(
            &quote
                .map(|id| self.quote_label(&model, id))
                .unwrap_or_default(),
        );
        self.updating.set(false);
    }
    fn render_typing(&self) {
        let model = self.model.borrow();
        let typing = model.typing_label();
        self.typing_indicator
            .set_label(if typing.is_some() { "Typing…" } else { "" });
        self.typing_indicator.set_visible(typing.is_some());
        self.typing_indicator.set_tooltip_text(typing.as_deref());
        self.composer_typing
            .set_label(typing.as_deref().unwrap_or(""));
        self.composer_typing.set_tooltip_text(typing.as_deref());
        self.subtitle.set_visible(typing.is_none());
        self.typing_indicator
            .update_property(&[gtk::accessible::Property::Label(
                typing.as_deref().unwrap_or(""),
            )]);
        // Update previews in place so typing cannot rebuild the list or disturb
        // its selection, scroll position, contact images, or media playback.
        for preview in self.sidebar_previews.borrow().iter() {
            if let Some(label) = preview.label.upgrade() {
                if let Some(typing) = model.typing_label_for(&preview.conversation) {
                    label.set_label("Typing…");
                    label.set_tooltip_text(Some(&typing));
                    label.update_property(&[gtk::accessible::Property::Label(&typing)]);
                    label.add_css_class("typing-preview");
                } else {
                    label.set_label(&preview.text);
                    label.set_tooltip_text(None);
                    label.update_property(&[gtk::accessible::Property::Label(&preview.text)]);
                    label.remove_css_class("typing-preview");
                }
            }
        }
    }
    fn render_status(&self) {
        self.render_typing();
        let model = self.model.borrow();
        if let Some(action) = self
            .window
            .lookup_action("reconnect")
            .and_downcast::<gio::SimpleAction>()
        {
            action.set_enabled(
                model.connected
                    && !model.accounts.is_empty()
                    && !self
                        .pending
                        .borrow()
                        .values()
                        .any(|request| matches!(request, Request::Sync(_))),
            );
        }
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
            .map(|id| self.conversation_label(&model, id))
            .unwrap_or_else(|| "Messages".into());
        self.title.set_label(&title);
        let conversation = model.selected.as_ref().and_then(|id| {
            model
                .conversations
                .iter()
                .find(|conversation| &conversation.id == id)
        });
        style_avatar(
            &self.avatar,
            &title,
            conversation.is_some_and(|conversation| conversation.kind == ConversationKind::Group),
        );
        self.avatar.set_visible(model.selected.is_some());
        let photos = conversation
            .map(|conversation| self.photos.borrow().conversation_photos(conversation))
            .unwrap_or_default();
        self.media.avatar_photos(&self.avatar, photos);
        self.media.schedule(&self.window);
        let account = model.selected.as_ref().and_then(|id| {
            model
                .accounts
                .iter()
                .find(|account| account.id == id.account_id)
        });
        let connected = model.connected
            && account.is_some_and(|account| account.connected && account.authenticated);
        let transport = conversation
            .map(|conversation| match conversation.transport {
                TransportKind::Rcs => "RCS",
                TransportKind::Sms => "SMS",
                TransportKind::Mms => "MMS",
                _ => "Messages",
            })
            .unwrap_or("");
        self.subtitle.set_label(if connected {
            transport
        } else if model.selected.is_some() {
            "Offline"
        } else {
            ""
        });
        self.connection_status.set_visible(model.selected.is_some());
        self.connection_status
            .remove_css_class(if connected { "offline" } else { "connected" });
        self.connection_status
            .add_css_class(if connected { "connected" } else { "offline" });
        self.connection_status.set_tooltip_text(Some(if connected {
            "Account connected"
        } else {
            "Account offline. You can still write a draft."
        }));
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 10);
        detail.set_margin_top(16);
        detail.set_margin_bottom(16);
        detail.set_margin_start(16);
        detail.set_margin_end(16);
        let detail_title = gtk::Label::new(Some(&title));
        detail_title.add_css_class("heading");
        detail_title.set_wrap(true);
        detail_title.set_max_width_chars(28);
        detail.append(&detail_title);
        if let Some(conversation) = conversation {
            for participant in conversation
                .participants
                .iter()
                .filter(|participant| !participant.is_self)
                .take(32)
            {
                let name = self.photos.borrow().label(participant);
                let label = gtk::Label::new(Some(&name));
                label.set_xalign(0.0);
                label.set_wrap(true);
                label.set_max_width_chars(30);
                detail.append(&label);
                if let Some(address) = participant
                    .address
                    .as_deref()
                    .filter(|address| *address != name)
                {
                    let label = gtk::Label::new(Some(address));
                    label.add_css_class("dim-label");
                    label.set_xalign(0.0);
                    label.set_selectable(true);
                    detail.append(&label);
                }
            }
        }
        if let Some(account) = account {
            let label = gtk::Label::new(Some(&account.label));
            label.add_css_class("caption");
            label.add_css_class("dim-label");
            detail.append(&label);
        }
        self.details.set_child(Some(&detail));
        let pending = self.pending.borrow().values().any(|request| match request {
            Request::Send(draft) => Some(&draft.conversation_id) == model.selected.as_ref(),
            Request::Attachment(draft, ..) => {
                Some(&draft.conversation_id) == model.selected.as_ref()
            }
            _ => false,
        });
        let draft = model.selected_draft();
        let editable = draft.is_some_and(|draft| draft.loaded);
        self.compose.set_sensitive(editable);
        self.placeholder
            .set_visible(draft.is_none_or(|draft| draft.value.text.is_empty()));
        self.reply_row
            .set_visible(draft.is_some_and(|draft| draft.value.reply_to.is_some()));
        let has_attachment = model
            .selected
            .as_ref()
            .is_some_and(|id| self.attachments.borrow().contains_key(id));
        self.media_button.set_sensitive(model.selected.is_some());
        self.send.set_sensitive(
            (if has_attachment {
                model.can(MessagingCapability::Media)
            } else {
                model.can(MessagingCapability::Text)
            }) && !pending
                && model.selected.as_ref().is_some_and(|id| {
                    self.attachment_state.borrow().contains_key(id)
                        && !self.attachment_loading(id)
                        && !self.attachment_saving(id)
                        && !self.unsaved_attachment(id)
                        && !self.attachment_errors.borrow().contains_key(id)
                })
                && draft.is_some_and(|draft| {
                    (has_attachment || !draft.value.text.trim().is_empty())
                        && (!has_attachment || draft.value.reply_to.is_none())
                        && !draft.conflict
                        && validate_message_draft(&draft.value).is_ok()
                        && (draft.value.reply_to.is_none()
                            || model.can(MessagingCapability::Replies))
                }),
        );
        self.attach.set_sensitive(
            model.can(MessagingCapability::Media)
                && !pending
                && !has_attachment
                && model.selected.as_ref().is_some_and(|id| {
                    self.attachment_state.borrow().contains_key(id)
                        && !self.attachment_loading(id)
                        && !self.attachment_saving(id)
                }),
        );
        self.render_history_status();
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
                        String::new()
                    } else {
                        "Draft held for this daemon session · Ctrl+Enter to send".into()
                    }
                })
                .unwrap_or_default(),
        );
        self.draft_status
            .set_visible(!self.draft_status.text().is_empty());
        let send_notice = model
            .selected
            .as_ref()
            .and_then(|id| model.send_notices.get(id));
        let sent = model
            .selected
            .as_ref()
            .and_then(|id| model.last_sent.get(id));
        let reaction_error = self.reaction_error.borrow();
        let reaction_notice = reaction_error
            .as_ref()
            .filter(|(id, _)| Some(&id.conversation_id) == model.selected.as_ref());
        let status = if let Some((_, notice)) = reaction_notice {
            notice.clone()
        } else if let Some(notice) = send_notice {
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
        self.status.set_visible(
            !status.is_empty()
                && status != "Viewing saved message context"
                && status != "Loading messages…",
        );
    }
    fn render_history_status(&self) {
        let model = self.model.borrow();
        let fetching = self.pending.borrow().iter().any(|(id, request)| {
            self.history_epochs.borrow().get(id) == Some(&self.history_epoch.get())
                && match request {
                    Request::History(id, _) => Some(id) == model.selected.as_ref(),
                    Request::Context(anchor) => model.anchor.as_ref() == Some(anchor),
                    _ => false,
                }
        });
        let history_error = self.history_error.borrow();
        self.history_indicator
            .set_visible(fetching || history_error.is_some());
        self.history_indicator
            .set_can_target(history_error.is_some());
        self.history_spinner.set_visible(fetching);
        self.history_spinner.set_spinning(fetching);
        self.history_label
            .set_label(
                history_error
                    .as_deref()
                    .unwrap_or(if model.messages.is_empty() {
                        "Loading messages…"
                    } else {
                        "Loading earlier messages…"
                    }),
            );
        self.history_retry.set_visible(history_error.is_some());
        self.history_retry
            .set_sensitive(model.connected && !fetching);
        let adjustment = self.scroll.vadjustment();
        let count = model.new_messages.len();
        self.latest.set_label(&if model.new_messages_overflow {
            "New messages ↓".into()
        } else if count == 1 {
            "1 new message ↓".into()
        } else if count > 1 {
            format!("{count} new messages ↓")
        } else {
            "Latest messages ↓".into()
        });
        self.latest.set_visible(
            model.selected.is_some()
                && (model.anchor.is_some()
                    || !model.new_messages.is_empty()
                    || model.newer_gap
                    || adjustment.upper() - adjustment.page_size() - adjustment.value() > 160.0),
        );
    }
    fn follow_tail(&self) {
        if self.model.borrow().anchor.is_some() {
            return;
        }
        let adjustment = self.scroll.vadjustment();
        let at_tail = adjustment.value() + adjustment.page_size() >= adjustment.upper() - 2.0;
        let mut model = self.model.borrow_mut();
        if at_tail && !model.newer_gap {
            model.browsing_older = false;
            model.clear_new_messages();
        } else if !at_tail {
            model.browsing_older = true;
        }
    }
    fn note_message_changes(&self, messages: &[Message]) {
        let model = self.model.borrow();
        let mut dirty = self.dirty_messages.borrow_mut();
        for message in messages {
            if model
                .messages
                .iter()
                .any(|old| old.id == message.id && old != message)
            {
                dirty.insert(message.id.clone());
            }
        }
    }
    fn render_thread(self: &Rc<Self>) {
        let generation = self.render_revision.get() + 1;
        self.render_revision.set(generation);
        self.restoring.set(true);
        let adjustment = self.scroll.vadjustment();
        let previous_y = adjustment.value();
        let following = self.jump_pending.get()
            || (self.model.borrow().anchor.is_none()
                && !self.model.borrow().browsing_older
                && previous_y + adjustment.page_size() >= adjustment.upper() - 2.0);
        let initial = self.rendered.borrow().is_empty();
        let mut previous = None;
        let mut child = self.thread.first_child();
        let mut index = 0;
        while let Some(widget) = child {
            let (y, height) = message_bounds(&widget, &self.thread);
            if y + height >= previous_y {
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
        let mut rows = BTreeMap::new();
        let mut widget = self.thread.first_child();
        for id in self.rendered.borrow().iter() {
            let Some(row) = widget else {
                break;
            };
            widget = row.next_sibling();
            rows.insert(id.clone(), row);
        }
        let model = self.model.borrow();
        rows.retain(|id, row| {
            if model.messages.iter().any(|message| &message.id == id) {
                true
            } else {
                self.thread.remove(row);
                false
            }
        });
        let group = model.conversations.iter().any(|conversation| {
            Some(&conversation.id) == model.selected.as_ref()
                && conversation.kind == ConversationKind::Group
        });
        let replies = model.can(MessagingCapability::Replies);
        let gallery = Rc::new(
            model
                .messages
                .iter()
                .filter(|message| !message.deleted)
                .flat_map(|message| &message.attachments)
                .filter(|attachment| crate::media::is_image(attachment))
                .take(256)
                .cloned()
                .collect::<Vec<_>>(),
        );
        let mut stamps = BTreeMap::new();
        let mut preceding: Option<gtk::Widget> = None;

        for (index, message) in model.messages.iter().enumerate() {
            let previous = index
                .checked_sub(1)
                .and_then(|index| model.messages.get(index));
            let stamp = RowStamp {
                previous: previous.map(|previous| {
                    (
                        previous.sender.local_id.clone(),
                        previous.sender.is_self,
                        previous.sent_at,
                    )
                }),
                group,
                replies,
                reactions: model.can(MessagingCapability::Reactions),
                quote: message
                    .reply_to
                    .as_ref()
                    .map(|id| self.quote_label(&model, id)),
                find_query: self.find.query(),
                sender: self.photos.borrow().label(&message.sender),
                delivery: if message.sender.is_self && !message.deleted {
                    model.message_status(&message.id)
                } else {
                    None
                },
            };
            let old = rows.remove(&message.id);
            if let Some(old) = old.as_ref().filter(|_| {
                !self.dirty_messages.borrow().contains(&message.id)
                    && self
                        .row_stamps
                        .borrow()
                        .get(&message.id)
                        .is_some_and(|previous| {
                            let mut content = stamp.clone();
                            content.delivery = previous.delivery;
                            previous == &content
                        })
            }) {
                if self
                    .row_stamps
                    .borrow()
                    .get(&message.id)
                    .is_some_and(|previous| previous.delivery != stamp.delivery)
                {
                    update_status_slot(old, stamp.delivery);
                }
                self.thread.reorder_child_after(old, preceding.as_ref());
                preceding = Some(old.clone());
                stamps.insert(message.id.clone(), stamp);
                continue;
            }
            if let Some(old) = old {
                self.thread.remove(&old);
            }
            let sender_name = stamp.sender.clone();
            stamps.insert(message.id.clone(), stamp);
            let grouped = previous.is_some_and(|previous| {
                previous.sender.local_id == message.sender.local_id
                    && previous.sender.is_self == message.sender.is_self
                    && previous
                        .sent_at
                        .zip(message.sent_at)
                        .is_some_and(|(a, b)| (0..=300_000_000).contains(&b.saturating_sub(a)))
            });
            // Keep one wrapper per message so saved scroll anchors retain their identity.
            let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 0);
            if let Some(time) = message.sent_at.filter(|time| {
                previous
                    .and_then(|message| message.sent_at)
                    .is_none_or(|previous| time.saturating_sub(previous) > 900_000_000)
            }) {
                let date = gtk::Label::new(Some(&message_time(time)));
                date.add_css_class("message-date");
                date.set_halign(gtk::Align::Center);
                wrapper.append(&date);
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.add_css_class("message-row");
            row.set_margin_top(if grouped { 3 } else { 8 });
            row.set_halign(if message.sender.is_self {
                gtk::Align::End
            } else {
                gtk::Align::Start
            });
            // Leave breathing room on the opposite side without forcing short bubbles to fill it.
            row.set_margin_start(if message.sender.is_self { 48 } else { 0 });
            row.set_margin_end(if message.sender.is_self { 0 } else { 48 });
            let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
            if !grouped
                && !message.sender.is_self
                && model.conversations.iter().any(|conversation| {
                    Some(&conversation.id) == model.selected.as_ref()
                        && conversation.kind == ConversationKind::Group
                })
            {
                let sender = gtk::Label::new(Some(&sender_name));
                sender.set_xalign(0.0);
                sender.add_css_class("message-sender");
                column.append(&sender);
            }
            let bubble = gtk::Box::new(gtk::Orientation::Vertical, 6);
            bubble.add_css_class("message-bubble");
            bubble.set_halign(if message.sender.is_self {
                gtk::Align::End
            } else {
                gtk::Align::Start
            });
            bubble.add_css_class(if message.sender.is_self {
                "outgoing"
            } else {
                "incoming"
            });
            if grouped {
                bubble.add_css_class("grouped");
            }
            if model.anchor.as_ref() == Some(&message.id) {
                bubble.add_css_class("search-anchor");
            }
            bubble.set_tooltip_text(message.sent_at.map(message_time).as_deref());
            if let Some(target) = message.reply_to.as_ref().filter(|_| !message.deleted) {
                let quote = gtk::Button::with_label(&self.quote_label(&model, target));
                quote.add_css_class("quoted-message");
                quote.set_tooltip_text(Some("Jump to original message"));
                if let Some(label) = quote.child().and_downcast::<gtk::Label>() {
                    label.set_xalign(0.0);
                    label.set_wrap(true);
                    label.set_lines(3);
                    label.set_max_width_chars(42);
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                }
                let weak = Rc::downgrade(self);
                let target = target.clone();
                let source = message.id.clone();
                quote.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.jump_to_reply(&source, &target);
                    }
                });
                bubble.append(&quote);
            }
            let query = self.find.query();
            let text = crate::message_text::label(
                if message.deleted {
                    "Message deleted"
                } else {
                    message.text.as_deref().unwrap_or("")
                },
                (!query.is_empty()).then_some(query.as_str()),
            );
            text.set_xalign(0.0);
            text.set_wrap(true);
            text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            text.set_max_width_chars(48);
            text.set_selectable(true);
            text.add_css_class("message-text");
            text.set_visible(
                message.deleted || message.text.as_ref().is_some_and(|text| !text.is_empty()),
            );
            bubble.append(&text);
            let mut images = Vec::new();
            for attachment in message.attachments.iter().filter(|_| !message.deleted) {
                if crate::media::is_image(attachment) {
                    let image =
                        if let Some(index) = gallery.iter().position(|item| item == attachment) {
                            self.media.attachment_gallery(
                                attachment,
                                &self.window,
                                gallery.clone(),
                                index,
                            )
                        } else {
                            self.media.attachment(attachment, &self.window)
                        };
                    image.set_halign(if message.sender.is_self {
                        gtk::Align::End
                    } else {
                        gtk::Align::Start
                    });
                    image.set_margin_top(6);
                    images.push(image);
                    continue;
                }
                if matches!(
                    AttachmentKind::of_mime(attachment.mime.as_deref()),
                    AttachmentKind::Audio | AttachmentKind::Video
                ) {
                    bubble.append(&crate::playback::widget(attachment, &self.window));
                    bubble.append(&crate::file_actions::save_button(&self.window, attachment));
                    continue;
                }
                let button =
                    gtk::Button::with_label(attachment.name.as_deref().unwrap_or("Attachment"));
                button.set_sensitive(attachment.staged_path.is_some());
                button.set_tooltip_text(attachment.name.as_deref());
                if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                    label.set_max_width_chars(32);
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                }
                let path = attachment.staged_path.clone();
                button.connect_clicked(move |_| {
                    if let Some(path) = &path {
                        let _ = gio::AppInfo::launch_default_for_uri(
                            &gio::File::for_path(path).uri(),
                            None::<&gio::AppLaunchContext>,
                        );
                    }
                });
                let file_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                file_row.append(&button);
                file_row.append(&crate::file_actions::save_button(&self.window, attachment));
                bubble.append(&file_row);
            }
            bubble.set_visible(
                text.is_visible()
                    || message.reply_to.is_some()
                    || message
                        .attachments
                        .iter()
                        .any(|attachment| !crate::media::is_image(attachment)),
            );
            column.append(&bubble);
            for image in images {
                column.append(&image);
            }
            if message.sender.is_self && !message.deleted {
                let label = gtk::Label::new(None);
                label.set_xalign(1.0);
                label.add_css_class("status-slot");
                column.append(&label);
                update_status_slot(label.upcast_ref(), model.message_status(&message.id));
            }
            let reply = gtk::Button::from_icon_name("mail-reply-sender-symbolic");
            reply.set_tooltip_text(Some("Reply to this message"));
            reply.add_css_class("flat");
            reply.add_css_class("circular");
            reply.add_css_class("message-actions");
            reply.set_valign(gtk::Align::Center);
            reply.set_sensitive(!message.deleted && model.can(MessagingCapability::Replies));
            let id = message.id.clone();
            let weak = Rc::downgrade(self);
            reply.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.set_reply(Some(id.clone()));
                }
            });
            if message.sender.is_self {
                row.append(&reply);
            }
            row.append(&column);
            if !message.sender.is_self {
                row.append(&reply);
            }
            let actions = self.message_actions(message);
            row.append(&actions);
            let click = gtk::GestureClick::new();
            click.set_button(3);
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak_actions = actions.downgrade();
            click.connect_pressed(move |gesture, _, _, _| {
                if let Some(actions) = weak_actions.upgrade() {
                    actions.popup();
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                }
            });
            row.add_controller(click);
            if !message.reactions.is_empty() && !message.deleted {
                let reactions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                reactions.add_css_class("reaction-strip");
                let align = if message.sender.is_self {
                    gtk::Align::End
                } else {
                    gtk::Align::Start
                };
                reactions.set_halign(align);
                let details = gtk::Revealer::builder()
                    .transition_type(gtk::RevealerTransitionType::SlideDown)
                    .transition_duration(140)
                    .halign(align)
                    .build();
                details.add_css_class("reaction-people");
                let items: Vec<_> = message
                    .reactions
                    .iter()
                    .take(32)
                    .map(|reaction| self.reaction_details(&model, message, reaction))
                    .collect();
                let buttons: Vec<_> = items.iter().map(|(button, _)| button.downgrade()).collect();
                for (button, content) in items {
                    let others = buttons.clone();
                    let panel = details.downgrade();
                    let media = self.media.clone();
                    button.connect_toggled(move |button| {
                        let Some(panel) = panel.upgrade() else {
                            return;
                        };
                        if button.is_active() {
                            for other in others.iter().filter_map(glib::WeakRef::upgrade) {
                                if other != *button {
                                    other.set_active(false);
                                }
                            }
                            panel.set_child(Some(&content));
                            panel.set_reveal_child(true);
                            media.schedule(&panel);
                        } else if panel.child().as_ref() == Some(content.upcast_ref()) {
                            panel.set_reveal_child(false);
                        }
                    });
                    reactions.append(&button);
                }
                column.append(&reactions);
                column.append(&details);
            }
            wrapper.append(&row);
            self.thread.insert_child_after(&wrapper, preceding.as_ref());
            preceding = Some(wrapper.upcast());
        }
        *self.row_stamps.borrow_mut() = stamps;
        self.dirty_messages.borrow_mut().clear();
        *self.rendered.borrow_mut() = model
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect();
        let target = if initial {
            model.anchor.clone().map(|id| (id, 0.0)).or(previous)
        } else {
            previous
        };
        let missing_quotes = model
            .messages
            .iter()
            .filter_map(|message| message.reply_to.clone())
            .chain(
                model
                    .selected_draft()
                    .and_then(|draft| draft.value.reply_to.clone()),
            )
            .filter(|id| {
                !model.messages.iter().any(|message| &message.id == id)
                    && !self.quotes.borrow().contains_key(id)
            })
            .take(4)
            .collect::<Vec<_>>();
        drop(model);
        for id in missing_quotes {
            if !self
                .pending
                .borrow()
                .values()
                .any(|request| matches!(request, Request::Quote(target) if target == &id))
                && self.quotes.borrow().len() < 256
                && self.pending.borrow().len() < 12
            {
                self.request(Request::Quote(id));
            }
        }
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
                    if this.jump_pending.replace(false) {
                        this.loading_latest.set(false);
                    }
                } else if let Some((id, offset)) = &target
                    && let Some(index) = this.rendered.borrow().iter().position(|known| known == id)
                {
                    let mut child = this.thread.first_child();
                    for _ in 0..index {
                        child = child.and_then(|widget| widget.next_sibling());
                    }
                    if let Some(child) = child {
                        adjustment
                            .set_value((message_bounds(&child, &this.thread).0 - offset).max(0.0));
                    }
                }
                this.restoring.set(false);
                this.follow_tail();
                if following {
                    this.mark_read();
                }
                this.media.refresh();
                this.load_older(false);
                this.render_history_status();
            }
            glib::ControlFlow::Break
        });
    }
    fn to_end(self: &Rc<Self>) {
        self.jump_pending.set(true);
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
                this.loading_latest.set(false);
                this.jump_pending.set(false);
                this.model.borrow_mut().clear_new_messages();
                this.mark_read();
                this.load_older(false);
                this.render_history_status();
            }
            glib::ControlFlow::Break
        });
    }
    fn load_older(self: &Rc<Self>, force: bool) {
        if self.closing.get()
            || self.restoring.get()
            || self.loading_latest.get()
            || self.history_error.borrow().is_some()
        {
            return;
        }
        if !force
            && self.scroll.vadjustment().value() > self.scroll.vadjustment().page_size().max(240.0)
        {
            return;
        }
        let request = {
            let model = self.model.borrow();
            if !model.connected {
                return;
            }
            let (Some(id), Some(cursor)) = (model.selected.clone(), model.cursor.clone()) else {
                return;
            };
            if self.pending.borrow().iter().any(|(request_id, request)| {
                self.history_epochs.borrow().get(request_id) == Some(&self.history_epoch.get())
                    && matches!(request, Request::History(pending, _) if pending == &id)
            }) {
                return;
            }
            if self.history_cursors.borrow().contains(&cursor) {
                *self.history_error.borrow_mut() =
                    Some("Couldn't load earlier messages. Try again.".into());
                return;
            }
            (id, cursor)
        };
        if self.request(Request::History(request.0, Some(request.1.clone()))) {
            let mut cursors = self.history_cursors.borrow_mut();
            cursors.push_back(request.1);
            if cursors.len() > 32 {
                cursors.pop_front();
            }
        } else {
            *self.history_error.borrow_mut() =
                Some("Could not load earlier messages. Try again.".into());
        }
        self.render_history_status();
    }

    fn scrolled(self: &Rc<Self>) {
        self.media.refresh();
        if self.restoring.get() || self.updating.get() {
            return;
        }
        self.follow_tail();
        self.load_older(false);
        self.render_history_status();
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
                            let (y, height) = message_bounds(&child, &this.thread);
                            if y + height >= adjustment.value() {
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
    fn attachment_loading(&self, id: &ConversationId) -> bool {
        self.pending
            .borrow()
            .values()
            .any(|request| matches!(request, Request::AttachmentDraft(target) if target == id))
    }
    fn attachment_saving(&self, id: &ConversationId) -> bool {
        self.pending.borrow().values().any(
            |request| matches!(request, Request::SaveAttachmentDraft(target, ..) if target == id),
        )
    }
    fn any_attachment_saving(&self) -> bool {
        self.pending
            .borrow()
            .values()
            .any(|request| matches!(request, Request::SaveAttachmentDraft(..)))
    }
    fn unsaved_attachment(&self, id: &ConversationId) -> bool {
        self.attachments.borrow().get(id).is_some_and(|pending| {
            self.attachment_state
                .borrow()
                .get(id)
                .and_then(|state| state.attachment.as_ref())
                .and_then(|part| part.staged_path.as_ref())
                != pending.attachment.staged_path.as_ref()
        })
    }
    fn attach_file(self: &Rc<Self>) {
        let Some(id) = self.model.borrow().selected.clone() else {
            return;
        };
        let dialog = gtk::FileDialog::builder().title("Attach a file").build();
        let weak = Rc::downgrade(self);
        dialog.open(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                if let (Some(this), Ok(file)) = (weak.upgrade(), result) {
                    this.stage_file(id.clone(), &file);
                }
            },
        );
    }
    fn stage_file(self: &Rc<Self>, id: ConversationId, file: &gio::File) -> bool {
        let model = self.model.borrow();
        if model.selected.as_ref() != Some(&id) || !model.can(MessagingCapability::Media)
            || !self.attachment_state.borrow().contains_key(&id) || self.attachment_loading(&id) || self.attachment_saving(&id)
            || self.pending.borrow().values().any(|request| matches!(request, Request::Attachment(draft, ..) if draft.conversation_id == id)) { return false; }
        let error = if model
            .selected_draft()
            .is_some_and(|draft| draft.value.reply_to.is_some())
        {
            Some("Cancel the reply before attaching a file.".into())
        } else if self.attachments.borrow().contains_key(&id) {
            Some("Remove the current attachment before choosing another.".into())
        } else if self.attachments.borrow().len() >= 32 {
            Some("Finish an attachment draft before adding another.".into())
        } else {
            None
        };
        drop(model);
        let result = error.map_or_else(|| crate::attachments::from_file(file), Err);
        match result {
            Ok(attachment) => {
                let revision = self
                    .attachment_state
                    .borrow()
                    .get(&id)
                    .map_or(0, |state| state.revision);
                self.request(Request::SaveAttachmentDraft(
                    id.clone(),
                    Some(attachment.uri.clone()),
                    attachment.attachment.mime.clone(),
                    revision,
                ));
                self.attachments.borrow_mut().insert(id.clone(), attachment);
                self.attachment_errors.borrow_mut().remove(&id);
                self.model.borrow_mut().notice.clear();
                self.render_attachments();
                self.render_status();
                true
            }
            Err(error) => {
                self.model.borrow_mut().notice = error;
                self.render_status();
                false
            }
        }
    }
    fn render_attachments(self: &Rc<Self>) {
        while let Some(child) = self.attachment_row.first_child() {
            self.attachment_row.remove(&child);
        }
        let model = self.model.borrow();
        let selected = model.selected.clone();
        let attachment = selected
            .as_ref()
            .and_then(|id| self.attachments.borrow().get(id).cloned());
        drop(model);
        let error = selected
            .as_ref()
            .and_then(|id| self.attachment_errors.borrow().get(id).cloned());
        self.attachment_row
            .set_visible(attachment.is_some() || error.is_some());
        if let (Some(id), Some(error)) = (selected.clone(), error) {
            let message = gtk::Label::new(Some(&error));
            message.set_wrap(true);
            message.set_max_width_chars(35);
            self.attachment_row.append(&message);
            let reload = gtk::Button::with_label("Reload saved attachment");
            reload.set_sensitive(!self.attachment_saving(&id) && !self.attachment_loading(&id));
            let weak = Rc::downgrade(self);
            reload.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.attachments.borrow_mut().remove(&id);
                    this.request(Request::AttachmentDraft(id.clone()));
                }
            });
            self.attachment_row.append(&reload);
        }
        if let (Some(id), Some(pending)) = (selected, attachment) {
            if crate::media::is_image(&pending.attachment) {
                self.attachment_row.append(
                    &self
                        .media
                        .composer_attachment(&pending.attachment, &self.window),
                );
            } else {
                self.attachment_row
                    .append(&gtk::Image::from_icon_name("text-x-generic-symbolic"));
            }
            let info = gtk::Box::new(gtk::Orientation::Vertical, 4);
            info.set_hexpand(true);
            let name = gtk::Label::new(pending.attachment.name.as_deref());
            name.set_xalign(0.0);
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            info.append(&name);
            let hint = gtk::Label::new(Some(
                "Add a caption below. The attachment is kept with your saved draft.",
            ));
            if !self.model.borrow().persistent {
                hint.set_label("Attachment kept for this daemon session.");
            }
            if self.attachment_saving(&id) {
                hint.set_label("Saving attachment…");
            } else if self.unsaved_attachment(&id) {
                hint.set_label("Attachment has not been saved.");
                let retry = gtk::Button::with_label("Retry saving attachment");
                let weak = Rc::downgrade(self);
                let target = id.clone();
                retry.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        let revision = this
                            .attachment_state
                            .borrow()
                            .get(&target)
                            .map_or(0, |state| state.revision);
                        let pending = this.attachments.borrow().get(&target).cloned();
                        if let Some(pending) = pending {
                            this.request(Request::SaveAttachmentDraft(
                                target.clone(),
                                Some(pending.uri),
                                pending.attachment.mime,
                                revision,
                            ));
                            this.render_attachments();
                        }
                    }
                });
                info.append(&retry);
            }
            hint.add_css_class("dim-label");
            hint.set_xalign(0.0);
            hint.set_wrap(true);
            info.append(&hint);
            self.attachment_row.append(&info);
            let remove = gtk::Button::from_icon_name("window-close-symbolic");
            remove.set_tooltip_text(Some("Remove attachment"));
            remove.add_css_class("flat");
            remove.set_sensitive(!self.attachment_saving(&id) && !self.pending.borrow().values().any(|request| matches!(request, Request::Attachment(draft, ..) if draft.conversation_id == id)));
            let weak = Rc::downgrade(self);
            remove.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    let revision = this
                        .attachment_state
                        .borrow()
                        .get(&id)
                        .map_or(0, |state| state.revision);
                    if this.unsaved_attachment(&id) && !this.attachment_saving(&id) {
                        this.attachments.borrow_mut().remove(&id);
                        this.attachment_errors.borrow_mut().remove(&id);
                        this.request(Request::AttachmentDraft(id.clone()));
                        this.render_attachments();
                    } else {
                        this.request(Request::SaveAttachmentDraft(
                            id.clone(),
                            None,
                            None,
                            revision,
                        ));
                        this.render_attachments();
                    }
                    this.render_status();
                }
            });
            self.attachment_row.append(&remove);
            self.media.schedule(&self.window);
        }
    }
    fn unsaved_dialog(self: &Rc<Self>) {
        self.closing.set(false);
        let dialog = adw::MessageDialog::new(
            Some(&self.window),
            Some("Keep your unsent changes?"),
            Some(
                "Keep editing, or close and discard changes that were not saved to Handover. Saved drafts and attachments remain available when you return.",
            ),
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

fn update_status_slot(widget: &gtk::Widget, status: Option<&str>) {
    if widget.has_css_class("status-slot")
        && let Some(label) = widget.downcast_ref::<gtk::Label>()
    {
        label.set_label(status.unwrap_or(""));
        label.set_visible(status.is_some());
        if status.is_some() {
            label.add_css_class("message-status");
        } else {
            label.remove_css_class("message-status");
        }
        label.set_tooltip_text(Some(match status {
            Some("Accepted") => "The command was accepted. Delivery has not been confirmed.",
            Some("Outcome unknown") => {
                "Check the conversation before sending again. Handover never resends automatically."
            }
            _ => "Reported message status",
        }));
        return;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        update_status_slot(&widget, status);
        child = widget.next_sibling();
    }
}

fn message_bounds(wrapper: &gtk::Widget, thread: &gtk::Box) -> (f64, f64) {
    fn content(widget: &gtk::Widget) -> Option<gtk::Widget> {
        if widget.is_visible()
            && (widget.has_css_class("message-bubble") || widget.has_css_class("image-attachment"))
        {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(content) = content(&widget) {
                return Some(content);
            }
            child = widget.next_sibling();
        }
        None
    }
    let widget = content(wrapper).unwrap_or_else(|| wrapper.clone());
    let y = widget
        .compute_point(thread, &gtk::graphene::Point::zero())
        .map(|point| f64::from(point.y()))
        .unwrap_or_else(|| f64::from(wrapper.allocation().y()));
    (y, f64::from(widget.height()))
}

fn style_avatar(avatar: &adw::Avatar, title: &str, group: bool) {
    avatar.set_text(Some(title));
    let human_name = title.chars().any(char::is_alphabetic);
    avatar.set_show_initials(human_name && !group);
    avatar.set_icon_name(Some(if group {
        "system-users-symbolic"
    } else {
        "avatar-default-symbolic"
    }));
}

fn sidebar_time(time: Option<i64>) -> String {
    let Some(time) = time.and_then(|time| glib::DateTime::from_unix_local(time / 1_000_000).ok())
    else {
        return String::new();
    };
    let format = glib::DateTime::now_local().ok().map_or("%b %e", |now| {
        if now.year() == time.year() && now.day_of_year() == time.day_of_year() {
            "%H:%M"
        } else {
            "%b %e"
        }
    });
    time.format(format)
        .map(|time| time.to_string())
        .unwrap_or_default()
}

fn message_time(time: i64) -> String {
    glib::DateTime::from_unix_local(time / 1_000_000)
        .ok()
        .and_then(|time| time.format("%a, %b %e · %H:%M").ok())
        .map(|time| time.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
