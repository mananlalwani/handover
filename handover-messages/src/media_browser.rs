//! Paginated photos and files from the daemon's saved conversation history.
use adw::prelude::*;
use gtk::{gio, glib};
use handover_core::{Attachment, ConversationId, Message};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

struct Browser {
    window: glib::WeakRef<gtk::Window>,
    parent: glib::WeakRef<adw::ApplicationWindow>,
    media: crate::media::Media,
    id: ConversationId,
    path: PathBuf,
    content: gtk::Box,
    status: gtk::Label,
    previous: gtk::Button,
    next: gtk::Button,
    retry: gtk::Button,
    offset: Cell<u32>,
    retry_offset: Cell<u32>,
    loading: Cell<bool>,
    closed: Cell<bool>,
    messages: RefCell<Vec<Message>>,
}

pub fn open(
    parent: &adw::ApplicationWindow,
    media: &crate::media::Media,
    messages: &[Message],
    id: ConversationId,
    path: PathBuf,
) -> gtk::Window {
    let window = gtk::Window::builder()
        .application(&parent.application().unwrap())
        .transient_for(parent)
        .title("Photos and files")
        .default_width(760)
        .default_height(620)
        .build();
    window.add_css_class("handover-media-browser");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    toolbar.add_css_class("viewer-toolbar");
    let title = gtk::Label::new(Some("Photos and files"));
    title.set_hexpand(true);
    title.set_xalign(0.0);
    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.set_tooltip_text(Some("Close media browser"));
    close.add_css_class("flat");
    toolbar.append(&title);
    toolbar.append(&close);
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&toolbar));
    root.append(&handle);
    let status = gtk::Label::new(Some("Loading saved photos and files…"));
    status.set_wrap(true);
    status.set_margin_top(12);
    status.set_margin_bottom(12);
    status.add_css_class("dim-label");
    root.append(&status);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    let scroll = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&content)
        .build();
    let images = media.clone();
    scroll
        .vadjustment()
        .connect_value_changed(move |_| images.refresh());
    root.append(&scroll);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    controls.set_halign(gtk::Align::Center);
    controls.set_margin_bottom(12);
    let previous = gtk::Button::with_label("Newer media");
    let next = gtk::Button::with_label("Older media");
    let retry = gtk::Button::with_label("Retry");
    retry.set_visible(false);
    controls.append(&previous);
    controls.append(&retry);
    controls.append(&next);
    root.append(&controls);
    window.set_child(Some(&root));
    let browser = Rc::new(Browser {
        window: window.downgrade(),
        parent: parent.downgrade(),
        media: media.clone(),
        id,
        path,
        content,
        status,
        previous,
        next,
        retry,
        offset: Cell::new(0),
        retry_offset: Cell::new(0),
        loading: Cell::new(false),
        closed: Cell::new(false),
        messages: RefCell::new(
            messages
                .iter()
                .rev()
                .filter(|m| !m.deleted && !m.attachments.is_empty())
                .take(16)
                .cloned()
                .collect(),
        ),
    });
    browser.render();
    for (button, direction) in [
        (&browser.previous, -1i32),
        (&browser.next, 1),
        (&browser.retry, 0),
    ] {
        let weak = Rc::downgrade(&browser);
        button.connect_clicked(move |_| {
            if let Some(browser) = weak.upgrade() {
                if browser.loading.get() {
                    return;
                }
                let offset = browser.offset.get();
                let offset = if direction < 0 {
                    offset.saturating_sub(16)
                } else if direction > 0 {
                    offset.saturating_add(16)
                } else {
                    browser.retry_offset.get()
                };
                browser.load(offset);
            }
        });
    }
    let alive = browser.clone();
    window.connect_close_request(move |_| {
        alive.closed.set(true);
        glib::Propagation::Proceed
    });
    let weak = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = weak.upgrade() {
            window.close();
        }
    });
    let keys = gtk::EventControllerKey::new();
    let weak = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    window.present();
    browser.load(0);
    window
}
impl Browser {
    fn load(self: &Rc<Self>, offset: u32) {
        if self.loading.replace(true) || self.closed.get() {
            return;
        }
        self.retry_offset.set(offset);
        self.previous.set_sensitive(false);
        self.next.set_sensitive(false);
        self.retry.set_visible(false);
        self.status.set_label("Loading saved photos and files…");
        let (sender, receiver) = async_channel::bounded(1);
        let path = self.path.clone();
        let id = self.id.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let result = runtime.block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    let mut client = handover_ipc::Client::connect_to(path).await?;
                    client.message_media(id, offset).await
                })
                .await
            });
            let _ = sender.send_blocking(match result {
                Ok(Ok(page)) => Some(page),
                _ => None,
            });
        });
        let this = self.clone();
        glib::spawn_future_local(async move {
            let result = receiver.recv().await;
            if this.closed.get() {
                return;
            }
            this.loading.set(false);
            match result {
                Ok(Some((messages, more))) => {
                    this.offset.set(offset);
                    *this.messages.borrow_mut() = messages;
                    this.render();
                    this.status
                        .set_label("Saved on this computer. History may be incomplete.");
                    this.previous.set_sensitive(offset > 0);
                    this.next.set_sensitive(more && offset <= 9984);
                }
                _ => {
                    this.status
                        .set_label("Could not load saved media. Try again.");
                    this.retry.set_visible(true);
                    this.previous.set_sensitive(this.offset.get() > 0);
                }
            }
        });
    }
    fn render(&self) {
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        let Some(parent) = self.parent.upgrade() else {
            return;
        };
        let messages = self.messages.borrow();
        let mut photos = Vec::new();
        let mut files = Vec::new();
        for attachment in messages
            .iter()
            .filter(|m| !m.deleted)
            .take(16)
            .flat_map(|m| m.attachments.iter().take(16))
        {
            if crate::media::is_image(attachment) {
                photos.push(attachment.clone());
            } else {
                files.push(attachment.clone());
            }
        }
        if photos.is_empty() && files.is_empty() {
            let empty = gtk::Label::new(Some("No saved photos or files in this page"));
            empty.add_css_class("empty-state");
            self.content.append(&empty);
        }
        if !photos.is_empty() {
            let heading = gtk::Label::new(Some("Photos"));
            heading.set_xalign(0.0);
            heading.add_css_class("heading");
            self.content.append(&heading);
            let grid = gtk::FlowBox::new();
            grid.set_selection_mode(gtk::SelectionMode::None);
            grid.set_homogeneous(true);
            grid.set_row_spacing(10);
            grid.set_column_spacing(10);
            grid.set_max_children_per_line(4);
            grid.set_min_children_per_line(2);
            let photos = Rc::new(photos);
            for (index, attachment) in photos.iter().enumerate() {
                let tile =
                    self.media
                        .attachment_gallery(attachment, &parent, photos.clone(), index);
                grid.insert(&tile, -1);
            }
            self.content.append(&grid);
        }
        if !files.is_empty() {
            let heading = gtk::Label::new(Some("Files"));
            heading.set_xalign(0.0);
            heading.add_css_class("heading");
            self.content.append(&heading);
            for attachment in &files {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                row.append(&file_row(attachment));
                row.append(&crate::file_actions::save_button(&parent, attachment));
                self.content.append(&row);
            }
        }
        if let Some(window) = self.window.upgrade() {
            self.media.schedule(&window);
        }
    }
}

fn file_row(attachment: &Attachment) -> gtk::Widget {
    let row = gtk::Button::new();
    row.add_css_class("media-file-row");
    row.set_halign(gtk::Align::Fill);
    let layout = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = gtk::Image::from_icon_name("text-x-generic-symbolic");
    icon.add_css_class("dim-label");
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
    labels.set_hexpand(true);
    labels.set_halign(gtk::Align::Fill);
    let name = attachment
        .name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Unnamed file");
    let title = gtk::Label::new(Some(name));
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let detail_text = attachment
        .size_bytes
        .map(format_size)
        .unwrap_or_else(|| attachment.mime.clone().unwrap_or_else(|| "File".into()));
    let detail = gtk::Label::new(Some(&detail_text));
    detail.set_xalign(0.0);
    detail.add_css_class("dim-label");
    labels.append(&title);
    labels.append(&detail);
    layout.append(&icon);
    layout.append(&labels);
    row.set_child(Some(&layout));

    if let Some(path) = attachment.staged_path.as_deref() {
        let path = path.to_owned();
        row.set_tooltip_text(Some("Open this locally staged file"));
        row.connect_clicked(move |_| {
            let uri = gio::File::for_path(&path).uri();
            let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
        });
    } else {
        row.set_sensitive(false);
        detail.set_label(&format!("{detail_text} · Not available locally"));
    }
    row.upcast()
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
