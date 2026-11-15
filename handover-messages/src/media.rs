//! Bounded local image decoding shared by message thumbnails, photos and the viewer.
use adw::prelude::*;
use gtk::{gdk, gdk_pixbuf, gio, glib};
use handover_core::{Attachment, AttachmentKind};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    io::{BufRead, BufReader, Read},
    os::unix::fs::OpenOptionsExt,
    rc::Rc,
    sync::Arc,
};

const CACHE_BYTES: usize = 64 * 1024 * 1024;
const FILE_BYTES: u64 = 25 * 1024 * 1024;

pub fn is_image(attachment: &Attachment) -> bool {
    if AttachmentKind::of_mime(attachment.mime.as_deref()) == AttachmentKind::Image {
        return true;
    }
    let mime = attachment
        .mime
        .as_deref()
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if !mime.is_empty() && !mime.eq_ignore_ascii_case("application/octet-stream") {
        return false;
    }
    let Some(extension) = attachment
        .name
        .as_deref()
        .and_then(|name| std::path::Path::new(name).extension())
        .and_then(|extension| extension.to_str())
    else {
        return false;
    };
    // A filename selects the preview UI only. Decoding still validates bytes.
    gdk_pixbuf::Pixbuf::formats().iter().any(|format| {
        !format.is_disabled()
            && !format.is_scalable()
            && format
                .mime_types()
                .iter()
                .any(|mime| mime.starts_with("image/"))
            && format
                .extensions()
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
    })
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
enum Source {
    File(String, String),
    Photo(Arc<str>),
    Group(Vec<Arc<str>>),
}
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct Key(Source, i32);
struct Entry {
    texture: Option<gdk::Texture>,
    ready: bool,
    used: u64,
}
#[derive(Clone)]
enum Target {
    Avatar(glib::WeakRef<adw::Avatar>),
    Canvas(
        glib::WeakRef<crate::image_canvas::ImageCanvas>,
        glib::WeakRef<gtk::Label>,
    ),
}
impl Target {
    fn widget(&self) -> Option<gtk::Widget> {
        match self {
            Self::Avatar(avatar) => avatar.upgrade().map(|w| w.upcast()),
            Self::Canvas(canvas, _) => canvas.upgrade().map(|w| w.upcast()),
        }
    }
    fn visible(&self) -> bool {
        let Some(widget) = self.widget() else {
            return false;
        };
        let Some(root) = widget.root().and_downcast::<gtk::Window>() else {
            return false;
        };
        widget.is_mapped()
            && widget.compute_bounds(&root).is_some_and(|bounds| {
                bounds.y() + bounds.height() > 0.0
                    && bounds.y() < root.height() as f32
                    && bounds.x() + bounds.width() > 0.0
                    && bounds.x() < root.width() as f32
            })
    }
    fn has_image(&self) -> bool {
        match self {
            Self::Canvas(canvas, _) => canvas
                .upgrade()
                .is_some_and(|canvas| canvas.texture().is_some()),
            Self::Avatar(avatar) => avatar
                .upgrade()
                .is_some_and(|avatar| avatar.custom_image().is_some()),
        }
    }
    fn apply(&self, texture: Option<&gdk::Texture>, failed: bool) {
        match self {
            Self::Canvas(canvas, label) => {
                if let Some(canvas) = canvas.upgrade() {
                    canvas.set_texture(texture);
                }
                if let Some(label) = label.upgrade() {
                    label.set_visible(texture.is_none());
                    label.set_label(if failed {
                        "Image unavailable"
                    } else {
                        "Loading image…"
                    });
                }
            }
            Self::Avatar(avatar) => {
                if let Some(avatar) = avatar.upgrade() {
                    avatar.set_custom_image(texture);
                }
            }
        }
    }
}

struct Binding {
    target: Target,
    key: Key,
}
struct Inner {
    bindings: RefCell<Vec<Binding>>,
    cache: RefCell<BTreeMap<Key, Entry>>,
    queue: RefCell<VecDeque<Key>>,
    active: Cell<usize>,
    clock: Cell<u64>,
    scheduled: RefCell<Vec<glib::WeakRef<gtk::Widget>>>,
    viewer: RefCell<Option<gtk::Window>>,
}
#[derive(Clone)]
pub struct Media(Rc<Inner>);
impl Media {
    pub fn new() -> Self {
        Self(Rc::new(Inner {
            bindings: RefCell::new(vec![]),
            cache: RefCell::new(BTreeMap::new()),
            queue: RefCell::new(VecDeque::new()),
            active: Cell::new(0),
            clock: Cell::new(0),
            scheduled: RefCell::new(Vec::new()),
            viewer: RefCell::new(None),
        }))
    }
    fn bind(&self, target: Target, key: Key) {
        let mut bindings = self.0.bindings.borrow_mut();
        let widget = target.widget();
        if bindings
            .iter()
            .any(|binding| binding.target.widget() == widget && binding.key == key)
        {
            return;
        }
        bindings.retain(|binding| {
            binding.target.widget().is_some() && binding.target.widget() != widget
        });
        let cached = self
            .0
            .cache
            .borrow()
            .get(&key)
            .filter(|entry| entry.ready)
            .map(|entry| entry.texture.clone());
        if bindings.len() < 1024 {
            bindings.push(Binding {
                target: target.clone(),
                key,
            });
        }
        drop(bindings);
        if let Some(texture) = cached {
            target.apply(texture.as_ref(), texture.is_none());
        } else {
            target.apply(None, false);
        }
    }

    pub fn clear_avatar(&self, avatar: &adw::Avatar) {
        let widget: gtk::Widget = avatar.clone().upcast();
        self.0
            .bindings
            .borrow_mut()
            .retain(|binding| binding.target.widget().as_ref() != Some(&widget));
        avatar.set_custom_image(None::<&gdk::Texture>);
    }
    pub fn avatar(&self, avatar: &adw::Avatar, photo: Arc<str>) {
        self.bind(
            Target::Avatar(avatar.downgrade()),
            Key(Source::Photo(photo), 128),
        );
    }
    pub fn avatar_photos(&self, avatar: &adw::Avatar, photos: Vec<Arc<str>>) {
        match photos.as_slice() {
            [] => self.clear_avatar(avatar),
            [photo] => self.avatar(avatar, photo.clone()),
            _ => self.bind(
                Target::Avatar(avatar.downgrade()),
                Key(Source::Group(photos), 128),
            ),
        }
    }
    pub fn schedule(&self, widget: &impl IsA<gtk::Widget>) {
        let owner = widget.as_ref().downgrade();
        let mut scheduled = self.0.scheduled.borrow_mut();
        scheduled.retain(|widget| widget.upgrade().is_some_and(|widget| widget.is_mapped()));
        if scheduled
            .iter()
            .any(|current| current.upgrade() == owner.upgrade())
        {
            return;
        }
        if scheduled.len() >= 8 {
            scheduled.remove(0);
        }
        scheduled.push(owner.clone());
        drop(scheduled);
        let this = self.clone();
        let frames = Cell::new(0);
        widget.add_tick_callback(move |_, _| {
            frames.set(frames.get() + 1);
            if frames.get() < 2 {
                return glib::ControlFlow::Continue;
            }
            this.0.scheduled.borrow_mut().retain(|current| {
                current.upgrade().is_some() && current.upgrade() != owner.upgrade()
            });
            this.refresh();
            glib::ControlFlow::Break
        });
    }
    pub fn refresh(&self) {
        self.0
            .bindings
            .borrow_mut()
            .retain(|binding| binding.target.widget().is_some());
        let mut cache = self.0.cache.borrow_mut();
        for binding in self.0.bindings.borrow().iter() {
            if !binding.target.visible() {
                // Keep small decoded avatars through unmaps and scrolling.
                // Large message/viewer images still release their textures.
                // Avatar bindings are capped at 1024 and textures at 128 px.
                if matches!(binding.target, Target::Canvas(..)) {
                    binding.target.apply(None, false);
                }
                continue;
            }
            if let Some(entry) = cache.get_mut(&binding.key) {
                self.0.clock.set(self.0.clock.get() + 1);
                entry.used = self.0.clock.get();
                binding.target.apply(
                    entry.texture.as_ref(),
                    entry.ready && entry.texture.is_none(),
                );
            } else if !binding.target.has_image() && self.0.queue.borrow().len() < 32 {
                cache.insert(
                    binding.key.clone(),
                    Entry {
                        texture: None,
                        ready: false,
                        used: self.0.clock.get(),
                    },
                );
                self.0.queue.borrow_mut().push_back(binding.key.clone());
            }
        }
        drop(cache);
        self.pump();
    }
    fn pump(&self) {
        while self.0.active.get() < 2 {
            let Some(key) = self.0.queue.borrow_mut().pop_front() else {
                break;
            };
            self.0.active.set(self.0.active.get() + 1);
            let this = self.clone();
            glib::spawn_future_local(async move {
                let input = key.clone();
                let result = gio::spawn_blocking(move || decode(&input.0, input.1)).await;
                let texture = result.ok().and_then(Result::ok).map(|image| {
                    gdk::MemoryTexture::new(
                        image.width,
                        image.height,
                        if image.alpha {
                            gdk::MemoryFormat::R8g8b8a8
                        } else {
                            gdk::MemoryFormat::R8g8b8
                        },
                        &glib::Bytes::from_owned(image.pixels),
                        image.stride,
                    )
                    .upcast::<gdk::Texture>()
                });
                this.0.active.set(this.0.active.get() - 1);
                this.0.clock.set(this.0.clock.get() + 1);
                this.0.cache.borrow_mut().insert(
                    key,
                    Entry {
                        texture,
                        ready: true,
                        used: this.0.clock.get(),
                    },
                );
                this.trim();
                this.refresh();
            });
        }
    }
    fn trim(&self) {
        let mut cache = self.0.cache.borrow_mut();
        loop {
            let bytes: usize = cache
                .values()
                .filter_map(|e| e.texture.as_ref())
                .map(|t| t.width() as usize * t.height() as usize * 4)
                .sum();
            if cache.len() <= 96 && bytes <= CACHE_BYTES {
                break;
            }
            let Some(oldest) = cache
                .iter()
                .filter(|(_, e)| e.ready)
                .min_by_key(|(_, e)| e.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            cache.remove(&oldest);
        }
    }
    pub fn attachment(
        &self,
        attachment: &Attachment,
        parent: &adw::ApplicationWindow,
    ) -> gtk::Button {
        self.attachment_gallery(attachment, parent, Rc::new(vec![attachment.clone()]), 0)
    }
    pub fn attachment_gallery(
        &self,
        attachment: &Attachment,
        parent: &adw::ApplicationWindow,
        gallery: Rc<Vec<Attachment>>,
        index: usize,
    ) -> gtk::Button {
        self.tile(attachment, parent, gallery, index, (220, 165))
    }
    /// Build the image preview for one message. The gallery is scoped to this
    /// message and retains attachment order for the viewer's previous/next keys.
    pub fn message_images(
        &self,
        attachments: &[Attachment],
        parent: &adw::ApplicationWindow,
    ) -> gtk::Widget {
        let images: Vec<_> = attachments
            .iter()
            .filter(|attachment| is_image(attachment))
            .cloned()
            .collect();
        match images.as_slice() {
            [] => gtk::Box::new(gtk::Orientation::Vertical, 0).upcast(),
            [image] => self.attachment(image, parent).upcast(),
            _ => {
                let gallery = Rc::new(images.clone());
                let grid = gtk::Grid::new();
                grid.add_css_class("message-image-grid");
                grid.set_column_spacing(4);
                grid.set_row_spacing(4);
                grid.set_column_homogeneous(true);
                grid.set_halign(gtk::Align::Start);

                let wide_lead = images.len() == 3;
                for (index, image) in images.iter().take(4).enumerate() {
                    let (column, row, width, height, span) = if wide_lead && index == 0 {
                        (0, 0, 268, 130, 2)
                    } else if wide_lead {
                        (index as i32 - 1, 1, 132, 100, 1)
                    } else {
                        ((index % 2) as i32, (index / 2) as i32, 132, 100, 1)
                    };
                    let tile = self.tile(image, parent, gallery.clone(), index, (width, height));
                    tile.add_css_class("message-image-tile");
                    if index == 3 && images.len() > 4 {
                        if let Some(content) = tile.child() {
                            tile.set_child(None::<&gtk::Widget>);
                            let overlay = gtk::Overlay::new();
                            overlay.set_child(Some(&content));
                            let more = gtk::Label::new(Some(&format!("+{}", images.len() - 4)));
                            more.add_css_class("message-image-overflow");
                            more.set_halign(gtk::Align::Fill);
                            more.set_valign(gtk::Align::Fill);
                            more.set_can_target(false);
                            overlay.add_overlay(&more);
                            tile.set_child(Some(&overlay));
                        }
                        tile.set_tooltip_text(Some(&format!(
                            "Open image {} of {}",
                            index + 1,
                            images.len()
                        )));
                    }
                    grid.attach(&tile, column, row, span, 1);
                }

                grid.upcast()
            }
        }
    }
    pub fn composer_attachment(
        &self,
        attachment: &Attachment,
        parent: &adw::ApplicationWindow,
    ) -> gtk::Button {
        self.tile(
            attachment,
            parent,
            Rc::new(vec![attachment.clone()]),
            0,
            (112, 84),
        )
    }
    fn tile(
        &self,
        attachment: &Attachment,
        parent: &adw::ApplicationWindow,
        gallery: Rc<Vec<Attachment>>,
        index: usize,
        dimensions: (i32, i32),
    ) -> gtk::Button {
        let button = gtk::Button::new();
        button.add_css_class("image-attachment");
        button.update_property(&[gtk::accessible::Property::Label(
            attachment.name.as_deref().unwrap_or("Open image"),
        )]);
        button.set_tooltip_text(Some(attachment.name.as_deref().unwrap_or("Open image")));
        let overlay = gtk::Overlay::new();
        let picture = crate::image_canvas::ImageCanvas::thumbnail(dimensions.0, dimensions.1);
        overlay.set_child(Some(&picture));
        let status = gtk::Label::new(Some(if attachment.staged_path.is_some() {
            "Loading image…"
        } else {
            "Image not available locally"
        }));
        status.add_css_class("image-placeholder");
        status.set_wrap(true);
        overlay.add_overlay(&status);
        button.set_child(Some(&overlay));
        if let Some(path) = &attachment.staged_path {
            let source = Source::File(path.clone(), attachment.mime.clone().unwrap_or_default());
            self.bind(
                Target::Canvas(picture.downgrade(), status.downgrade()),
                Key(source.clone(), 640),
            );
            let this = self.clone();
            let parent = parent.downgrade();
            button.connect_clicked(move |_| {
                if let Some(parent) = parent.upgrade() {
                    this.open(&parent, gallery.clone(), index);
                }
            });
        } else {
            button.set_sensitive(false);
        }
        button
    }
    fn open(&self, parent: &adw::ApplicationWindow, gallery: Rc<Vec<Attachment>>, index: usize) {
        if gallery.is_empty() {
            return;
        }
        let index = Rc::new(Cell::new(index.min(gallery.len() - 1)));
        let name = gallery[index.get()].name.as_deref().unwrap_or("Image");
        let previous = self.0.viewer.borrow_mut().take();
        if let Some(previous) = previous {
            previous.close();
        }
        let window = gtk::Window::builder()
            .application(&parent.application().unwrap())
            .transient_for(parent)
            .title(name)
            .default_width(980)
            .default_height(720)
            .build();
        window.add_css_class("handover-image-viewer");
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.add_css_class("viewer-toolbar");
        let title = gtk::Label::new(Some(name));
        title.set_hexpand(true);
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let save = gtk::Button::with_label("Save as…");
        let current_gallery = gallery.clone();
        let current_index = index.clone();
        let parent_weak = parent.downgrade();
        save.connect_clicked(move |_| {
            if let Some(parent) = parent_weak.upgrade() {
                crate::file_actions::save(&parent, &current_gallery[current_index.get()]);
            }
        });
        let external = gtk::Button::with_label("Open externally");
        let current = gallery.clone();
        let current_index = index.clone();
        external.connect_clicked(move |_| {
            if let Some(path) = current[current_index.get()].staged_path.as_ref() {
                let _ = gio::AppInfo::launch_default_for_uri(
                    &gio::File::for_path(path).uri(),
                    None::<&gio::AppLaunchContext>,
                );
            }
        });
        let previous = gtk::Button::from_icon_name("go-previous-symbolic");
        previous.set_tooltip_text(Some("Previous image"));
        let next = gtk::Button::from_icon_name("go-next-symbolic");
        next.set_tooltip_text(Some("Next image"));
        previous.set_visible(gallery.len() > 1);
        next.set_visible(gallery.len() > 1);
        bar.append(&previous);
        bar.append(&next);
        let fit = gtk::Button::with_label("Fit");
        let minus = gtk::Button::from_icon_name("zoom-out-symbolic");
        minus.set_tooltip_text(Some("Zoom out"));
        let plus = gtk::Button::from_icon_name("zoom-in-symbolic");
        plus.set_tooltip_text(Some("Zoom in"));
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.set_tooltip_text(Some("Close image"));
        for button in [&external, &fit, &minus, &plus, &close] {
            button.add_css_class("flat");
        }
        bar.append(&title);
        bar.append(&save);
        bar.append(&external);
        let zoom_level = gtk::Label::new(Some("Fit"));
        zoom_level.add_css_class("viewer-zoom-level");
        zoom_level.set_width_chars(5);
        zoom_level.set_tooltip_text(Some(
            "Zoom relative to the fitted image. Ctrl+scroll or pinch to zoom; drag to pan.",
        ));
        bar.append(&minus);
        bar.append(&zoom_level);
        bar.append(&plus);
        bar.append(&fit);
        bar.append(&close);
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&bar));
        body.append(&handle);
        let picture = crate::image_canvas::ImageCanvas::new();
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&picture));
        let status = gtk::Label::new(Some("Loading image…"));
        overlay.add_overlay(&status);
        let scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&overlay)
            .build();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
        body.append(&scroll);
        window.set_child(Some(&body));
        let navigation = crate::image_navigation::Navigation::new(&picture, &scroll, &zoom_level);
        let update: Rc<dyn Fn()> = {
            let this = self.clone();
            let gallery = gallery.clone();
            let index = index.clone();
            let title = title.clone();
            let picture = picture.clone();
            let status = status.clone();
            let external = external.clone();
            let save = save.clone();
            let previous = previous.downgrade();
            let next = next.downgrade();
            let window = window.downgrade();
            let navigation = navigation.clone();
            Rc::new(move || {
                let item = &gallery[index.get()];
                let name = item.name.as_deref().unwrap_or("Image");
                title.set_label(&format!(
                    "{} · {} of {}",
                    name,
                    index.get() + 1,
                    gallery.len()
                ));
                if let Some(window) = window.upgrade() {
                    window.set_title(Some(name));
                }
                navigation.fit();
                if let Some(previous) = previous.upgrade() {
                    previous.set_sensitive(index.get() > 0);
                }
                if let Some(next) = next.upgrade() {
                    next.set_sensitive(index.get() + 1 < gallery.len());
                }
                external.set_sensitive(item.staged_path.is_some());
                save.set_sensitive(item.staged_path.is_some());
                if let Some(path) = item.staged_path.as_ref() {
                    this.bind(
                        Target::Canvas(picture.downgrade(), status.downgrade()),
                        Key(
                            Source::File(path.clone(), item.mime.clone().unwrap_or_default()),
                            4096,
                        ),
                    );
                } else {
                    let widget: gtk::Widget = picture.clone().upcast();
                    this.0
                        .bindings
                        .borrow_mut()
                        .retain(|binding| binding.target.widget().as_ref() != Some(&widget));
                    picture.set_texture(None);
                    status.set_label("Image not available locally");
                    status.set_visible(true);
                }
                if let Some(window) = window.upgrade() {
                    this.schedule(&window);
                }
            })
        };
        update();
        for (button, forward) in [(&previous, false), (&next, true)] {
            let index = index.clone();
            let update = update.clone();
            let count = gallery.len();
            button.connect_clicked(move |_| {
                let value = index.get();
                if forward && value + 1 < count {
                    index.set(value + 1);
                    update();
                } else if !forward && value > 0 {
                    index.set(value - 1);
                    update();
                }
            });
        }
        for (button, factor) in [(&plus, 1.35), (&minus, 1.0 / 1.35), (&fit, 0.0)] {
            let navigation = navigation.clone();
            button.connect_clicked(move |_| {
                if factor == 0.0 {
                    navigation.fit();
                } else {
                    navigation.multiply(factor);
                }
            });
        }
        let weak = window.downgrade();
        close.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
        });
        let keys = gtk::EventControllerKey::new();
        let weak = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, state| {
            if state.contains(gdk::ModifierType::CONTROL_MASK) && key == gdk::Key::s {
                save.emit_clicked();
                return glib::Propagation::Stop;
            }
            if state.contains(gdk::ModifierType::CONTROL_MASK) && key == gdk::Key::o {
                external.emit_clicked();
                return glib::Propagation::Stop;
            }
            if matches!(key, gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add) {
                navigation.multiply(1.35);
                return glib::Propagation::Stop;
            }
            if matches!(key, gdk::Key::minus | gdk::Key::KP_Subtract) {
                navigation.multiply(1.0 / 1.35);
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::f || key == gdk::Key::_0 {
                navigation.fit();
                return glib::Propagation::Stop;
            }
            if state.contains(gdk::ModifierType::SHIFT_MASK) {
                let delta = match key {
                    gdk::Key::Left => Some((-80.0, 0.0)),
                    gdk::Key::Right => Some((80.0, 0.0)),
                    gdk::Key::Up => Some((0.0, -80.0)),
                    gdk::Key::Down => Some((0.0, 80.0)),
                    _ => None,
                };
                if let Some((x, y)) = delta {
                    navigation.pan(x, y);
                    return glib::Propagation::Stop;
                }
            }
            if key == gdk::Key::Left {
                previous.emit_clicked();
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Right {
                next.emit_clicked();
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Escape {
                if let Some(window) = weak.upgrade() {
                    window.close();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(keys);
        let inner = Rc::downgrade(&self.0);
        window.connect_close_request(move |window| {
            if let Some(inner) = inner.upgrade() {
                let is_current = inner.viewer.borrow().as_ref() == Some(window);
                if is_current {
                    inner.viewer.borrow_mut().take();
                }
            }
            glib::Propagation::Proceed
        });
        *self.0.viewer.borrow_mut() = Some(window.clone());
        window.present();
        self.schedule(&window);
    }
}

struct Decoded {
    pixels: Vec<u8>,
    width: i32,
    height: i32,
    stride: usize,
    alpha: bool,
}
// The Ubuntu HEIF loader can return false without setting GError when a codec
// is missing or input is incomplete. The generated binding asserts that these
// agree; inspect both results so a bad image remains a decode failure.
fn close_image_loader(loader: &gdk_pixbuf::PixbufLoader) -> Result<(), ()> {
    unsafe {
        let mut error = std::ptr::null_mut();
        let success = gdk_pixbuf::ffi::gdk_pixbuf_loader_close(
            glib::object::ObjectType::as_ptr(loader),
            &mut error,
        );
        let has_error = !error.is_null();
        if has_error {
            glib::ffi::g_error_free(error);
        }
        if success == glib::ffi::GFALSE || has_error {
            Err(())
        } else {
            Ok(())
        }
    }
}

fn decode(source: &Source, edge: i32) -> Result<Decoded, ()> {
    if edge <= 0 {
        return Err(());
    }
    let (input, hint): (Box<dyn Read>, &str) = match source {
        Source::Group(photos) => return decode_group(photos, edge),
        Source::File(path, mime) => {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)
                .map_err(|_| ())?;
            let metadata = file.metadata().map_err(|_| ())?;
            if !metadata.is_file() || metadata.len() > FILE_BYTES {
                return Err(());
            }
            (Box::new(file.take(FILE_BYTES + 1)), mime)
        }
        Source::Photo(photo) => {
            if photo.len() > 48 * 1024 {
                return Err(());
            }
            (
                Box::new(std::io::Cursor::new(glib::base64_decode(photo.as_ref()))),
                "",
            )
        }
    };
    // Use the file bytes first. Providers can omit MIME types or label HEIF
    // variants differently, and contact photos are not necessarily JPEG.
    let mut input = BufReader::new(input);
    let prefix = input.fill_buf().map_err(|_| ())?;
    let content_type = gio::content_type_guess(None::<&str>, Some(prefix)).0;
    let detected = gio::content_type_get_mime_type(&content_type);
    let format = raster_format(detected.as_deref(), hint).ok_or(())?;
    let loader = gdk_pixbuf::PixbufLoader::with_type(&format).map_err(|_| ())?;
    let invalid = Rc::new(Cell::new(false));
    let prepared = invalid.clone();
    loader.connect_size_prepared(move |loader, width, height| {
        if !valid_dimensions(width, height) {
            prepared.set(true);
            loader.set_size(1, 1);
            return;
        }
        let scale = (f64::from(edge) / f64::from(width.max(height))).min(1.0);
        loader.set_size(
            (f64::from(width) * scale).max(1.0) as i32,
            (f64::from(height) * scale).max(1.0) as i32,
        );
    });
    let mut buffer = [0u8; 8192];
    let mut read = 0;
    loop {
        let count = input.read(&mut buffer).map_err(|_| ())?;
        if count == 0 {
            break;
        }
        read += count;
        if read as u64 > FILE_BYTES {
            let _ = close_image_loader(&loader);
            return Err(());
        }
        if loader.write(&buffer[..count]).is_err() || invalid.get() {
            let _ = close_image_loader(&loader);
            return Err(());
        }
    }
    close_image_loader(&loader)?;
    let original = loader.pixbuf().ok_or(())?;
    if invalid.get() || !valid_dimensions(original.width(), original.height()) {
        return Err(());
    }
    let image = original.apply_embedded_orientation().unwrap_or(original);
    // Some loaders, including HEIF implementations, decode at full size even
    // after set_size. Always bound the texture independently of that behavior.
    let image = if image.width() > edge || image.height() > edge {
        let scale = f64::from(edge) / f64::from(image.width().max(image.height()));
        image
            .scale_simple(
                (f64::from(image.width()) * scale).max(1.0) as i32,
                (f64::from(image.height()) * scale).max(1.0) as i32,
                gdk_pixbuf::InterpType::Bilinear,
            )
            .ok_or(())?
    } else {
        image
    };
    Ok(Decoded {
        pixels: image.read_pixel_bytes().as_ref().to_vec(),
        width: image.width(),
        height: image.height(),
        stride: image.rowstride() as usize,
        alpha: image.has_alpha(),
    })
}

fn decode_group(photos: &[Arc<str>], edge: i32) -> Result<Decoded, ()> {
    if !(2..=4).contains(&photos.len()) || !(2..=512).contains(&edge) {
        return Err(());
    }
    let images = photos
        .iter()
        .filter_map(|photo| decode(&Source::Photo(photo.clone()), edge).ok())
        .collect::<Vec<_>>();
    if images.len() < 2 {
        return images.into_iter().next().ok_or(());
    }
    let stride = edge as usize * 4;
    let mut pixels = vec![0; stride * edge as usize];
    for (index, image) in images.iter().enumerate() {
        let half = edge / 2;
        let (x, y, width, height) = match (images.len(), index) {
            (2, 0) | (3, 0) => (0, 0, half, edge),
            (2, _) => (half, 0, edge - half, edge),
            (3, 1) => (half, 0, edge - half, half),
            (3, _) => (half, half, edge - half, edge - half),
            (_, i) => {
                let x = (i as i32 % 2) * half;
                let y = (i as i32 / 2) * half;
                (
                    x,
                    y,
                    if x == 0 { half } else { edge - half },
                    if y == 0 { half } else { edge - half },
                )
            }
        };
        let scale = (f64::from(width) / f64::from(image.width))
            .max(f64::from(height) / f64::from(image.height));
        let left = (f64::from(image.width) - f64::from(width) / scale) / 2.0;
        let top = (f64::from(image.height) - f64::from(height) / scale) / 2.0;
        let channels = if image.alpha { 4 } else { 3 };
        for dy in 0..height {
            for dx in 0..width {
                let sx = ((f64::from(dx) + 0.5) / scale + left) as usize;
                let sy = ((f64::from(dy) + 0.5) / scale + top) as usize;
                let source = sy.min(image.height as usize - 1) * image.stride
                    + sx.min(image.width as usize - 1) * channels;
                let target = (y + dy) as usize * stride + (x + dx) as usize * 4;
                pixels[target..target + 3].copy_from_slice(&image.pixels[source..source + 3]);
                pixels[target + 3] = if image.alpha {
                    image.pixels[source + 3]
                } else {
                    255
                };
            }
        }
    }
    Ok(Decoded {
        pixels,
        width: edge,
        height: edge,
        stride,
        alpha: true,
    })
}

fn valid_dimensions(width: i32, height: i32) -> bool {
    width > 0
        && height > 0
        && width <= 32768
        && height <= 32768
        && i64::from(width) * i64::from(height) <= 100_000_000
}

fn raster_format(detected: Option<&str>, hint: &str) -> Option<String> {
    fn normalize(mime: &str) -> String {
        match mime
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "image/jpg" | "image/pjpeg" => "image/jpeg".into(),
            "image/heif-sequence" => "image/heif".into(),
            "image/heic-sequence" => "image/heic".into(),
            "image/x-tiff" => "image/tiff".into(),
            "image/x-icon" => "image/vnd.microsoft.icon".into(),
            mime => mime.into(),
        }
    }
    let hint = normalize(hint);
    // Vector documents remain outside the local raster decoder.
    if hint == "image/svg+xml" {
        return None;
    }
    let formats = gdk_pixbuf::Pixbuf::formats();
    for mime in detected.into_iter().chain(Some(hint.as_str())) {
        let mime = normalize(mime);
        if !mime.starts_with("image/") {
            continue;
        }
        if let Some(format) = formats.iter().find(|format| {
            !format.is_disabled()
                && !format.is_scalable()
                && format
                    .mime_types()
                    .iter()
                    .any(|supported| normalize(supported) == mime)
        }) {
            return format.name().map(|name| name.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn group_avatars_combine_member_photos_and_tolerate_a_bad_photo() {
        let make = |color| {
            let image =
                gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 4, 4).unwrap();
            image.fill(color);
            Arc::<str>::from(
                glib::base64_encode(&image.save_to_bufferv("png", &[]).unwrap()).as_str(),
            )
        };
        let red = make(0xff0000ff);
        let blue = make(0x0000ffff);
        let image = decode(&Source::Group(vec![red.clone(), blue.clone()]), 32).unwrap();
        assert_eq!((image.width, image.height), (32, 32));
        assert_eq!(&image.pixels[..4], &[255, 0, 0, 255]);
        assert_eq!(&image.pixels[16 * 4..16 * 4 + 4], &[0, 0, 255, 255]);
        let image = decode(
            &Source::Group(vec![red.clone(), Arc::from("bad"), blue.clone()]),
            32,
        )
        .unwrap();
        assert_eq!(&image.pixels[16 * 4..16 * 4 + 4], &[0, 0, 255, 255]);
        for count in [3, 4] {
            let photos = (0..count)
                .map(|i| if i == 0 { red.clone() } else { blue.clone() })
                .collect();
            let image = decode(&Source::Group(photos), 33).unwrap();
            assert_eq!(image.pixels.len(), 33 * 33 * 4);
            assert!(
                image
                    .pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| pixel[3] == 255)
            );
        }
        assert!(decode(&Source::Group(vec![Arc::from("bad"), Arc::from("bad")]), 32).is_err());
    }
    const FORMATS: &[(&str, &str)] = &[
        ("jpg", "image/jpeg"),
        ("png", "image/png"),
        ("gif", "image/gif"),
        ("webp", "image/webp"),
        ("bmp", "image/bmp"),
        ("tiff", "image/tiff"),
        ("ico", "image/vnd.microsoft.icon"),
        ("heic", "image/heic"),
        ("avif", "image/avif"),
        ("jxl", "image/jxl"),
    ];

    #[test]
    fn incomplete_raster_files_are_rejected_without_panicking() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("empty-image");
        std::fs::write(&path, []).unwrap();
        for (_, mime) in FORMATS {
            assert!(
                decode(
                    &Source::File(path.to_string_lossy().into(), (*mime).into()),
                    16
                )
                .is_err(),
                "empty {mime}"
            );
        }
    }

    #[test]
    fn installed_raster_loaders_decode_real_files_and_scale_viewer_textures() {
        for (extension, mime) in FORMATS {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("tests/fixtures/images/sample.{extension}"));
            if raster_format(Some(mime), mime).is_none() {
                assert!(
                    std::env::var_os("HANDOVER_MESSAGES_REQUIRE_ALL_IMAGE_FIXTURES").is_none(),
                    "required image loader is missing: {extension}"
                );
                assert!(
                    decode(
                        &Source::File(path.to_string_lossy().into(), (*mime).into()),
                        16
                    )
                    .is_err()
                );
                continue;
            }
            let mut attachment = Attachment {
                local_id: "test".into(),
                mime: None,
                name: Some(format!("PHOTO.{}", extension.to_ascii_uppercase())),
                size_bytes: None,
                staged_path: None,
            };
            assert!(is_image(&attachment), "{extension} without MIME metadata");
            attachment.mime = Some("application/octet-stream".into());
            assert!(
                is_image(&attachment),
                "{extension} with generic MIME metadata"
            );
            attachment.mime = Some("text/plain".into());
            assert!(
                !is_image(&attachment),
                "explicit nonimage MIME is preserved"
            );
            for hint in [*mime, "application/octet-stream", "image/jpeg"] {
                let decoded = decode(
                    &Source::File(path.to_string_lossy().into(), hint.into()),
                    16,
                )
                .unwrap_or_else(|_| panic!("decode {extension} with MIME hint {hint}"));
                let expected = if *extension == "ico" {
                    (16, 16)
                } else {
                    (16, 8)
                };
                assert_eq!((decoded.width, decoded.height), expected, "{extension}");
                assert!(!decoded.pixels.is_empty());
            }
        }
    }

    #[test]
    fn contact_photos_detect_png_instead_of_assuming_jpeg() {
        let bytes = include_bytes!("../tests/fixtures/images/sample.png");
        let encoded = glib::base64_encode(bytes);
        let decoded = decode(&Source::Photo(Arc::from(encoded.as_str())), 32).unwrap();
        assert_eq!((decoded.width, decoded.height), (32, 16));
        assert!(raster_format(Some("image/svg+xml"), "image/svg+xml").is_none());
    }

    #[test]
    fn decoder_scales_real_images_and_rejects_unsafe_local_sources() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        let image =
            gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 1024, 256).unwrap();
        image.fill(0x3684e4ff);
        image.savev(&path, "png", &[]).unwrap();
        let decoded = decode(
            &Source::File(path.to_string_lossy().into(), "image/png".into()),
            128,
        )
        .unwrap_or_else(|_| panic!("valid scaled image"));
        assert_eq!((decoded.width, decoded.height), (128, 32));
        let link = directory.path().join("link.png");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(
            decode(
                &Source::File(link.to_string_lossy().into(), "image/png".into()),
                128
            )
            .is_err()
        );
        assert!(
            decode(
                &Source::File(path.to_string_lossy().into(), "image/svg+xml".into()),
                128
            )
            .is_err()
        );
        assert!(decode(&Source::Photo(Arc::from("invalid image")), 128).is_err());
        let oversized = directory.path().join("large.png");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(FILE_BYTES + 1)
            .unwrap();
        assert!(
            decode(
                &Source::File(oversized.to_string_lossy().into(), "image/png".into()),
                128
            )
            .is_err()
        );
    }
}
