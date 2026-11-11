//! A fit/zoom image viewport without resizing or re-decoding the source texture.
use gtk::{gdk, glib, prelude::*, subclass::prelude::*};
use std::cell::{Cell, RefCell};

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct Canvas {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub requested: Cell<(i32, i32)>,
        pub thumbnail: Cell<bool>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "HandoverImageCanvas";
        type Type = super::ImageCanvas;
        type ParentType = gtk::Widget;
        fn class_init(class: &mut Self::Class) {
            class.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }
    impl ObjectImpl for Canvas {}
    impl WidgetImpl for Canvas {
        fn measure(&self, orientation: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            if self.thumbnail.get() {
                return if orientation == gtk::Orientation::Horizontal {
                    (220, 220, -1, -1)
                } else {
                    (165, 165, -1, -1)
                };
            }
            let (width, height) = self.requested.get();
            let size = if orientation == gtk::Orientation::Horizontal {
                width
            } else {
                height
            };
            (size, size, -1, -1)
        }
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let object = self.obj();
            let image = self.texture.borrow();
            let Some(image) = image.as_ref() else {
                return;
            };
            let (requested_width, requested_height) = self.requested.get();
            let (width, height) = if self.thumbnail.get() {
                let scale = (object.width() as f32 / image.width() as f32)
                    .max(object.height() as f32 / image.height() as f32);
                (image.width() as f32 * scale, image.height() as f32 * scale)
            } else if requested_width == 0 {
                let scale = (object.width() as f32 / image.width() as f32)
                    .min(object.height() as f32 / image.height() as f32);
                (image.width() as f32 * scale, image.height() as f32 * scale)
            } else {
                (requested_width as f32, requested_height as f32)
            };
            let x = (object.width() as f32 - width) / 2.0;
            let y = (object.height() as f32 - height) / 2.0;
            let bounds =
                gtk::graphene::Rect::new(0.0, 0.0, object.width() as f32, object.height() as f32);
            if self.thumbnail.get() {
                snapshot.push_rounded_clip(&gtk::gsk::RoundedRect::from_rect(bounds, 12.0));
            } else {
                snapshot.push_clip(&bounds);
            }
            snapshot.append_texture(image, &gtk::graphene::Rect::new(x, y, width, height));
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct ImageCanvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}
impl ImageCanvas {
    pub fn new() -> Self {
        let widget: Self = glib::Object::new();
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.add_css_class("image-canvas");
        widget
    }
    pub fn thumbnail() -> Self {
        let widget = Self::new();
        widget.imp().thumbnail.set(true);
        widget.set_hexpand(false);
        widget.set_vexpand(false);
        widget.add_css_class("image-thumbnail");
        widget
    }
    pub fn texture(&self) -> Option<gdk::Texture> {
        self.imp().texture.borrow().clone()
    }
    pub fn set_texture(&self, texture: Option<&gdk::Texture>) {
        *self.imp().texture.borrow_mut() = texture.cloned();
        self.queue_draw();
    }
    pub fn fit(&self) {
        self.imp().requested.set((0, 0));
        self.queue_resize();
        self.queue_draw();
    }
    pub fn zoom(&self, viewport_width: i32, viewport_height: i32, factor: f64) {
        let Some(texture) = self.texture() else {
            return;
        };
        let scale = (viewport_width.max(1) as f64 / texture.width() as f64)
            .min(viewport_height.max(1) as f64 / texture.height() as f64)
            * factor;
        self.imp().requested.set((
            (texture.width() as f64 * scale).max(1.0) as i32,
            (texture.height() as f64 * scale).max(1.0) as i32,
        ));
        self.queue_resize();
        self.queue_draw();
    }
    pub fn zoomed_size(&self) -> (i32, i32) {
        self.imp().requested.get()
    }
}
impl Default for ImageCanvas {
    fn default() -> Self {
        Self::new()
    }
}
