//! Image interaction uses the decoded texture and viewport adjustments only.
use adw::prelude::*;
use gtk::{gdk, glib};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub struct Navigation {
    canvas: crate::image_canvas::ImageCanvas,
    scroll: gtk::ScrolledWindow,
    level: gtk::Label,
    factor: Cell<f64>,
    pending: RefCell<Option<gtk::TickCallbackId>>,
}

impl Navigation {
    pub fn new(
        canvas: &crate::image_canvas::ImageCanvas,
        scroll: &gtk::ScrolledWindow,
        level: &gtk::Label,
    ) -> Rc<Self> {
        let this = Rc::new(Self {
            canvas: canvas.clone(),
            scroll: scroll.clone(),
            level: level.clone(),
            factor: Cell::new(1.0),
            pending: RefCell::new(None),
        });
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let weak = Rc::downgrade(&this);
        wheel.connect_scroll(move |controller, _, dy| {
            if !controller
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                return glib::Propagation::Proceed;
            }
            if let Some(this) = weak.upgrade() {
                let anchor = controller
                    .current_event()
                    .and_then(|event| event.position())
                    .and_then(|(x, y)| {
                        let root = this.scroll.root()?.downcast::<gtk::Window>().ok()?;
                        root.compute_point(
                            &this.scroll,
                            &gtk::graphene::Point::new(x as f32, y as f32),
                        )
                        .map(|point| (point.x() as f64, point.y() as f64))
                    });
                this.set(
                    this.factor.get() * 1.15_f64.powf(-dy.clamp(-4.0, 4.0)),
                    anchor,
                );
            }
            glib::Propagation::Stop
        });
        scroll.add_controller(wheel);
        let drag_allowed = Rc::new(Cell::new(false));
        let pinch = gtk::GestureZoom::new();
        let start = Rc::new(Cell::new(1.0));
        let weak = Rc::downgrade(&this);
        let origin = start.clone();
        let panning = drag_allowed.clone();
        pinch.connect_begin(move |_, _| {
            panning.set(false);
            if let Some(this) = weak.upgrade() {
                origin.set(this.factor.get());
            }
        });
        let weak = Rc::downgrade(&this);
        pinch.connect_scale_changed(move |gesture, scale| {
            if let Some(this) = weak.upgrade() {
                let center = gesture.bounding_box_center().map(|(x, y)| {
                    (
                        x - this.scroll.hadjustment().value(),
                        y - this.scroll.vadjustment().value(),
                    )
                });
                this.set(start.get() * scale, center);
            }
        });
        canvas.add_controller(pinch.clone());
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        let origin = Rc::new(Cell::new((0.0, 0.0)));
        let weak = Rc::downgrade(&this);
        let start = origin.clone();
        let panning = drag_allowed.clone();
        drag.connect_drag_begin(move |_, _, _| {
            panning.set(false);
            if let Some(this) = weak.upgrade()
                && this.factor.get() > 1.0
            {
                start.set((
                    this.scroll.hadjustment().value(),
                    this.scroll.vadjustment().value(),
                ));
                panning.set(true);
                this.canvas.set_cursor_from_name(Some("grabbing"));
            }
        });
        let weak = Rc::downgrade(&this);
        drag.connect_drag_update(move |gesture, x, y| {
            if let Some(this) = weak.upgrade()
                && drag_allowed.get()
                && this.factor.get() > 1.0
            {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                this.scroll.hadjustment().set_value(origin.get().0 - x);
                this.scroll.vadjustment().set_value(origin.get().1 - y);
            }
        });
        let weak = Rc::downgrade(&this);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(this) = weak.upgrade() {
                this.cursor();
            }
        });
        canvas.add_controller(drag.clone());
        drag.group_with(&pinch);
        let click = gtk::GestureClick::new();
        click.set_button(1);
        let weak = Rc::downgrade(&this);
        click.connect_pressed(move |gesture, count, x, y| {
            if count == 2
                && let Some(this) = weak.upgrade()
            {
                if (this.factor.get() - 1.0).abs() < 0.001 {
                    this.set(
                        2.0,
                        Some((
                            x - this.scroll.hadjustment().value(),
                            y - this.scroll.vadjustment().value(),
                        )),
                    );
                } else {
                    this.fit();
                }
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        canvas.add_controller(click.clone());
        click.group_with(&drag);
        this.fit();
        this
    }
    fn cursor(&self) {
        self.canvas
            .set_cursor_from_name((self.factor.get() > 1.0).then_some("grab"));
    }
    pub fn fit(&self) {
        if let Some(callback) = self.pending.borrow_mut().take() {
            callback.remove();
        }
        self.factor.set(1.0);
        self.canvas.fit();
        self.scroll
            .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
        self.level.set_label("Fit");
        self.cursor();
    }
    pub fn multiply(self: &Rc<Self>, factor: f64) {
        self.set(self.factor.get() * factor, None);
    }
    pub fn pan(&self, x: f64, y: f64) {
        self.scroll
            .hadjustment()
            .set_value(self.scroll.hadjustment().value() + x);
        self.scroll
            .vadjustment()
            .set_value(self.scroll.vadjustment().value() + y);
    }
    fn set(self: &Rc<Self>, factor: f64, anchor: Option<(f64, f64)>) {
        let Some(texture) = self.canvas.texture() else {
            return;
        };
        if !factor.is_finite() {
            return;
        }
        let factor = factor.clamp(0.25, 8.0);
        if let Some(callback) = self.pending.borrow_mut().take() {
            callback.remove();
        }
        let width = self.scroll.width().max(1) as f64;
        let height = self.scroll.height().max(1) as f64;
        let point = anchor.unwrap_or((width / 2.0, height / 2.0));
        let base = (width / texture.width() as f64).min(height / texture.height() as f64);
        let requested = self.canvas.zoomed_size();
        let old = if requested.0 > 0 {
            (requested.0 as f64, requested.1 as f64)
        } else {
            (
                texture.width() as f64 * base,
                texture.height() as f64 * base,
            )
        };
        let offset = (
            (self.canvas.width() as f64 - old.0) / 2.0,
            (self.canvas.height() as f64 - old.1) / 2.0,
        );
        let position = (
            (self.scroll.hadjustment().value() + point.0 - offset.0) / old.0,
            (self.scroll.vadjustment().value() + point.1 - offset.1) / old.1,
        );
        self.factor.set(factor);
        self.canvas.zoom(width as i32, height as i32, factor);
        self.scroll
            .set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        self.level.set_label(&format!("{factor:.2}×"));
        self.cursor();
        let weak = Rc::downgrade(self);
        let frames = Cell::new(0);
        let callback = self.scroll.add_tick_callback(move |_, _| {
            frames.set(frames.get() + 1);
            if frames.get() < 2 {
                return glib::ControlFlow::Continue;
            }
            if let Some(this) = weak.upgrade() {
                let size = this.canvas.zoomed_size();
                let x = position.0 * size.0 as f64 + (this.canvas.width() - size.0) as f64 / 2.0
                    - point.0;
                let y = position.1 * size.1 as f64 + (this.canvas.height() - size.1) as f64 / 2.0
                    - point.1;
                this.scroll.hadjustment().set_value(x);
                this.scroll.vadjustment().set_value(y);
                this.pending.borrow_mut().take();
            }
            glib::ControlFlow::Break
        });
        *self.pending.borrow_mut() = Some(callback);
    }
}

impl Drop for Navigation {
    fn drop(&mut self) {
        if let Some(callback) = self.pending.borrow_mut().take() {
            callback.remove();
        }
    }
}
