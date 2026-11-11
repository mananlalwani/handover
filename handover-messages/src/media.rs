//! Bounded local image decoding shared by message thumbnails, photos and the viewer.
use adw::prelude::*;
use gtk::{gdk, gdk_pixbuf, gio, glib};
use handover_core::Attachment;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    rc::Rc,
    sync::Arc,
};

const CACHE_BYTES: usize = 64 * 1024 * 1024;
const FILE_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
enum Source {
    File(String, String),
    Photo(Arc<str>),
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
    photo_epoch: Cell<u64>,
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
            photo_epoch: Cell::new(0),
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

    pub fn forget_photos(&self) {
        self.0.photo_epoch.set(self.0.photo_epoch.get() + 1);
        self.0
            .cache
            .borrow_mut()
            .retain(|key, _| !matches!(key.0, Source::Photo(_)));
        self.0
            .queue
            .borrow_mut()
            .retain(|key| !matches!(key.0, Source::Photo(_)));
        self.0.bindings.borrow_mut().retain(|binding| {
            if matches!(binding.key.0, Source::Photo(_)) {
                binding.target.apply(None, false);
                false
            } else {
                true
            }
        });
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
                binding.target.apply(None, false);
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
            let photo_epoch = self.0.photo_epoch.get();
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
                if matches!(key.0, Source::Photo(_)) && photo_epoch != this.0.photo_epoch.get() {
                    this.refresh();
                    return;
                }
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
        let button = gtk::Button::new();
        button.add_css_class("image-attachment");
        button.update_property(&[gtk::accessible::Property::Label(
            attachment.name.as_deref().unwrap_or("Open image"),
        )]);
        button.set_tooltip_text(Some(attachment.name.as_deref().unwrap_or("Open image")));
        let overlay = gtk::Overlay::new();
        let picture = crate::image_canvas::ImageCanvas::thumbnail();
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
            let name = attachment.name.clone().unwrap_or_else(|| "Image".into());
            button.connect_clicked(move |_| {
                if let Some(parent) = parent.upgrade() {
                    this.open(&parent, source.clone(), &name);
                }
            });
        } else {
            button.set_sensitive(false);
        }
        button
    }
    fn open(&self, parent: &adw::ApplicationWindow, source: Source, name: &str) {
        if let Some(previous) = self.0.viewer.borrow_mut().take() {
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
        let external = gtk::Button::with_label("Open externally");
        if let Source::File(path, _) = &source {
            let path = path.clone();
            external.connect_clicked(move |_| {
                let _ = gio::AppInfo::launch_default_for_uri(
                    &gio::File::for_path(&path).uri(),
                    None::<&gio::AppLaunchContext>,
                );
            });
        }
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
        bar.append(&external);
        bar.append(&minus);
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
        self.bind(
            Target::Canvas(picture.downgrade(), status.downgrade()),
            Key(source, 4096),
        );
        let zoom = Rc::new(Cell::new(1.0_f64));
        for (button, factor) in [(&plus, 1.35), (&minus, 1.0 / 1.35), (&fit, 0.0)] {
            let picture = picture.clone();
            let scroll = scroll.clone();
            let zoom = zoom.clone();
            button.connect_clicked(move |_| {
                if picture.texture().is_none() {
                    return;
                }
                let value = if factor == 0.0 {
                    1.0
                } else {
                    (zoom.get() * factor).clamp(0.25, 8.0)
                };
                zoom.set(value);
                if value == 1.0 {
                    picture.fit();
                    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
                } else {
                    picture.zoom(scroll.width(), scroll.height(), value);
                    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
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
        keys.connect_key_pressed(move |_, key, _, _| {
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
fn decode(source: &Source, edge: i32) -> Result<Decoded, ()> {
    let (mut input, format): (Box<dyn Read>, &str) = match source {
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
            let format = match mime
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
                .as_str()
            {
                "image/jpeg" | "image/jpg" => "jpeg",
                "image/png" => "png",
                "image/webp" => "webp",
                "image/gif" => "gif",
                "image/bmp" => "bmp",
                _ => return Err(()),
            };
            (Box::new(file.take(FILE_BYTES + 1)), format)
        }
        Source::Photo(photo) => {
            if photo.len() > 48 * 1024 {
                return Err(());
            }
            (
                Box::new(std::io::Cursor::new(glib::base64_decode(photo.as_ref()))),
                "jpeg",
            )
        }
    };
    let loader = gdk_pixbuf::PixbufLoader::with_type(format).map_err(|_| ())?;
    let invalid = Rc::new(Cell::new(false));
    let prepared = invalid.clone();
    loader.connect_size_prepared(move |loader, width, height| {
        if width <= 0
            || height <= 0
            || width > 32768
            || height > 32768
            || i64::from(width) * i64::from(height) > 100_000_000
        {
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
            let _ = loader.close();
            return Err(());
        }
        if loader.write(&buffer[..count]).is_err() || invalid.get() {
            let _ = loader.close();
            return Err(());
        }
    }
    loader.close().map_err(|_| ())?;
    let original = loader.pixbuf().ok_or(())?;
    let image = original.apply_embedded_orientation().unwrap_or(original);
    if image.width() > edge || image.height() > edge {
        return Err(());
    }
    Ok(Decoded {
        pixels: image.read_pixel_bytes().as_ref().to_vec(),
        width: image.width(),
        height: image.height(),
        stride: image.rowstride() as usize,
        alpha: image.has_alpha(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
