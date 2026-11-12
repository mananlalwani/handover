//! In-conversation message search controls and bounded asynchronous paging.

use gtk::{glib, prelude::*};
use handover_core::{ConversationId, MessageId, MessageSearchHit};
use handover_ipc::Client;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::{Rc, Weak},
    time::Duration,
};

const PAGE_SIZE: u32 = 25;
const MAX_OFFSET: u32 = 10_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

type Callback<T> = RefCell<Option<Rc<dyn Fn(T)>>>;

type SearchResult = Result<(Vec<MessageSearchHit>, bool), String>;

pub struct Find {
    path: PathBuf,
    pub entry: gtk::SearchEntry,
    pub bar: gtk::Box,
    pub next: gtk::Button,
    pub previous: gtk::Button,
    retry: gtk::Button,
    position: gtk::Label,
    conversation: RefCell<Option<ConversationId>>,
    query: RefCell<String>,
    offset: Cell<u32>,
    hits: RefCell<Vec<MessageSearchHit>>,
    has_more: Cell<bool>,
    selected: Cell<Option<usize>>,
    select_last: Cell<bool>,
    generation: Cell<u64>,
    debounce_generation: Cell<u64>,
    in_flight: Cell<bool>,
    pending: Cell<bool>,
    on_select: Callback<MessageId>,
    on_query: Callback<String>,
}

impl Find {
    pub fn new(path: PathBuf) -> Rc<Self> {
        let entry = gtk::SearchEntry::new();
        entry.set_hexpand(true);
        entry.set_placeholder_text(Some("Find in conversation"));
        let previous = gtk::Button::from_icon_name("go-up-symbolic");
        previous.add_css_class("flat");
        previous.set_tooltip_text(Some("Previous match (Shift+Enter)"));
        let next = gtk::Button::from_icon_name("go-down-symbolic");
        next.add_css_class("flat");
        next.set_tooltip_text(Some("Next match (Enter)"));
        let position = gtk::Label::new(None);
        position.add_css_class("dim-label");
        position.set_width_chars(14);
        let retry = gtk::Button::with_label("Retry");
        retry.add_css_class("flat");
        retry.set_visible(false);
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("flat");
        close.set_tooltip_text(Some("Close find"));
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.add_css_class("search-bar");
        bar.append(&entry);
        bar.append(&previous);
        bar.append(&next);
        bar.append(&position);
        bar.append(&retry);
        bar.append(&close);
        bar.set_visible(false);

        let this = Rc::new(Self {
            path,
            entry,
            bar,
            next,
            previous,
            retry,
            position,
            conversation: RefCell::new(None),
            query: RefCell::new(String::new()),
            offset: Cell::new(0),
            hits: RefCell::new(Vec::new()),
            has_more: Cell::new(false),
            selected: Cell::new(None),
            select_last: Cell::new(false),
            generation: Cell::new(0),
            debounce_generation: Cell::new(0),
            in_flight: Cell::new(false),
            pending: Cell::new(false),
            on_select: RefCell::new(None),
            on_query: RefCell::new(None),
        });

        let weak = Rc::downgrade(&this);
        this.entry.connect_search_changed(move |entry| {
            let Some(this) = weak.upgrade() else { return };
            let raw = entry.text().to_string();
            let limited = limit_query(&raw);
            if raw != limited {
                entry.set_text(&limited);
                return;
            }
            this.query_changed(limited);
        });
        let weak = Rc::downgrade(&this);
        this.next.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.navigate_next();
            }
        });
        let weak = Rc::downgrade(&this);
        this.previous.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.navigate_previous();
            }
        });
        let weak = Rc::downgrade(&this);
        this.retry.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.request_current();
            }
        });
        let weak = Rc::downgrade(&this);
        close.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.close();
            }
        });
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&this);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(this) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if key == gtk::gdk::Key::Escape {
                this.close();
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter {
                if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                    this.navigate_previous();
                } else {
                    this.navigate_next();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        this.entry.add_controller(keys);
        this.refresh_controls();
        this
    }

    pub fn set_conversation(self: &Rc<Self>, conversation: Option<ConversationId>) {
        if *self.conversation.borrow() == conversation {
            return;
        }
        *self.conversation.borrow_mut() = conversation;
        self.offset.set(0);
        self.hits.borrow_mut().clear();
        self.has_more.set(false);
        self.selected.set(None);
        self.select_last.set(false);
        self.position.set_label("");
        self.retry.set_visible(false);
        self.invalidate_request();
        self.set_query_text(String::new());
        self.entry
            .set_sensitive(self.conversation.borrow().is_some());
        self.refresh_controls();
    }

    pub fn show(&self) {
        if self.conversation.borrow().is_none() {
            return;
        }
        self.bar.set_visible(true);
        self.entry.grab_focus();
    }

    pub fn close(self: &Rc<Self>) {
        self.bar.set_visible(false);
        self.set_query_text(String::new());
    }

    pub fn query(&self) -> String {
        self.query.borrow().clone()
    }

    pub fn set_on_select(&self, callback: impl Fn(MessageId) + 'static) {
        *self.on_select.borrow_mut() = Some(Rc::new(callback));
    }

    pub fn set_on_query(&self, callback: impl Fn(String) + 'static) {
        let callback = Rc::new(callback);
        *self.on_query.borrow_mut() = Some(callback.clone());
        callback(self.query());
    }

    fn set_query_text(self: &Rc<Self>, query: String) {
        if self.entry.text().as_str() != query {
            self.entry.set_text(&query);
        }
        if *self.query.borrow() != query {
            self.query_changed(query);
        }
    }

    fn query_changed(self: &Rc<Self>, query: String) {
        if *self.query.borrow() == query {
            return;
        }
        *self.query.borrow_mut() = query.clone();
        self.offset.set(0);
        self.hits.borrow_mut().clear();
        self.has_more.set(false);
        self.selected.set(None);
        self.select_last.set(false);
        self.position.set_label("");
        self.retry.set_visible(false);
        self.invalidate_request();
        if let Some(callback) = self.on_query.borrow().clone() {
            callback(query.clone());
        }
        self.refresh_controls();
        if query.trim().is_empty() || self.conversation.borrow().is_none() {
            return;
        }
        let token = self.debounce_generation.get().wrapping_add(1);
        self.debounce_generation.set(token);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(250), move || {
            let Some(this) = weak.upgrade() else { return };
            if this.debounce_generation.get() == token {
                this.request_current();
            }
        });
    }

    fn invalidate_request(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.debounce_generation
            .set(self.debounce_generation.get().wrapping_add(1));
        self.pending.set(false);
    }

    fn navigate_next(self: &Rc<Self>) {
        if self.query.borrow().is_empty() {
            return;
        }
        let count = self.hits.borrow().len();
        if count == 0 {
            return;
        }
        if let Some(selected) = self.selected.get() {
            if selected + 1 < count {
                self.select_index(selected + 1);
            } else if self.has_more.get() && self.offset.get() < MAX_OFFSET {
                self.offset
                    .set((self.offset.get() + PAGE_SIZE).min(MAX_OFFSET));
                self.selected.set(None);
                self.select_last.set(false);
                self.begin_page_request();
            }
        } else {
            self.select_index(0);
        }
    }

    fn navigate_previous(self: &Rc<Self>) {
        if self.query.borrow().is_empty() {
            return;
        }
        let count = self.hits.borrow().len();
        if let Some(selected) = self.selected.get() {
            if selected > 0 {
                self.select_index(selected - 1);
            } else if self.offset.get() > 0 {
                self.offset.set(self.offset.get().saturating_sub(PAGE_SIZE));
                self.selected.set(None);
                self.select_last.set(true);
                self.begin_page_request();
            }
        } else if count > 0 {
            self.select_index(count - 1);
        }
    }

    fn begin_page_request(self: &Rc<Self>) {
        self.invalidate_request();
        self.pending.set(true);
        self.request_current();
    }

    fn request_current(self: &Rc<Self>) {
        if self.in_flight.get() {
            self.pending.set(true);
            return;
        }
        let Some(conversation) = self.conversation.borrow().clone() else {
            return;
        };
        let raw_query = self.query.borrow().clone();
        let query = limit_query(&raw_query);
        if query != raw_query {
            self.entry.set_text(&query);
            if *self.query.borrow() != query {
                self.query_changed(query);
            }
            return;
        }
        if query.trim().is_empty() {
            return;
        }
        self.in_flight.set(true);
        self.pending.set(false);
        self.position.set_label("Searching…");
        self.retry.set_visible(false);
        self.refresh_controls();
        let generation = self.generation.get();
        let offset = self.offset.get().min(MAX_OFFSET);
        let path = self.path.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel::<SearchResult>();
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
                .and_then(|runtime| {
                    runtime.block_on(async move {
                        tokio::time::timeout(REQUEST_TIMEOUT, async move {
                            let mut client = Client::connect_to(path).await?;
                            client
                                .search_conversation_messages(conversation, query, offset)
                                .await
                        })
                        .await
                        .map_err(|_| "Search timed out".to_owned())
                        .and_then(|result| result.map_err(|error| error.to_string()))
                    })
                });
            let _ = sender.send(result);
        });
        let weak: Weak<Self> = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Search worker stopped".into()));
            if let Some(this) = weak.upgrade() {
                this.search_finished(generation, result);
            }
        });
    }

    fn search_finished(self: &Rc<Self>, generation: u64, result: SearchResult) {
        self.in_flight.set(false);
        if generation == self.generation.get() {
            match result {
                Ok((hits, has_more)) => {
                    *self.hits.borrow_mut() = hits.into_iter().take(PAGE_SIZE as usize).collect();
                    self.has_more.set(has_more);
                    let count = self.hits.borrow().len();
                    let selected = if self.select_last.replace(false) {
                        count.checked_sub(1)
                    } else {
                        count.checked_sub(1).map(|_| 0)
                    };
                    self.selected.set(selected);
                    if let Some(index) = selected {
                        self.select_index(index);
                    } else {
                        self.position.set_label("No matches");
                    }
                }
                Err(_) => {
                    self.hits.borrow_mut().clear();
                    self.has_more.set(false);
                    self.selected.set(None);
                    self.position.set_label("Search failed");
                    self.retry.set_visible(true);
                }
            }
            self.refresh_controls();
        }
        if self.pending.replace(false) {
            self.request_current();
        }
    }

    fn select_index(&self, index: usize) {
        let hit = self.hits.borrow().get(index).cloned();
        let Some(hit) = hit else { return };
        self.selected.set(Some(index));
        if let Some(callback) = self.on_select.borrow().clone() {
            callback(hit.message_id);
        }
        self.refresh_controls();
    }

    fn refresh_controls(&self) {
        let count = self.hits.borrow().len();
        let selected = self.selected.get();
        if count > 0
            && let Some(index) = selected
        {
            let number = self.offset.get() + index as u32 + 1;
            let known_total = self.offset.get() + count as u32;
            self.position.set_label(&format!(
                "{number} of {known_total}{}",
                if self.has_more.get() { "+" } else { "" }
            ));
        }
        self.previous.set_sensitive(
            !self.in_flight.get()
                && count > 0
                && (selected.unwrap_or(0) > 0 || self.offset.get() > 0),
        );
        self.next.set_sensitive(
            !self.in_flight.get()
                && count > 0
                && (selected.is_none_or(|index| index + 1 < count)
                    || (self.has_more.get() && self.offset.get() < MAX_OFFSET)),
        );
    }
}

fn limit_query(query: &str) -> String {
    let mut result = String::new();
    let mut terms = 0usize;
    let mut in_term = false;
    for ch in query.chars().take(256) {
        if ch.is_whitespace() {
            if terms >= 16 {
                break;
            }
            result.push(ch);
            in_term = false;
        } else {
            if !in_term {
                if terms == 16 {
                    break;
                }
                terms += 1;
                in_term = true;
            }
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::limit_query;

    #[test]
    fn query_limits_are_applied_by_characters_and_terms() {
        assert_eq!(limit_query("one two three"), "one two three");
        assert_eq!(
            limit_query(&(0..18).map(|n| format!("t{n} ")).collect::<String>()),
            (0..16)
                .map(|n| format!("t{n}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert_eq!(limit_query(&"é".repeat(300)).chars().count(), 256);
    }
}
