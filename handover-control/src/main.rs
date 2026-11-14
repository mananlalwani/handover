mod connection;
mod model;
mod ui;
use adw::prelude::*;
fn main() -> gtk::glib::ExitCode {
    let application = adw::Application::builder()
        .application_id("org.handover.Control")
        .build();
    let controller = std::rc::Rc::new(std::cell::RefCell::new(None::<std::rc::Rc<ui::Window>>));
    application.connect_activate(move |app| {
        let window = controller
            .borrow_mut()
            .get_or_insert_with(|| ui::Window::new(app))
            .clone();
        window.window.present();
    });
    application.run()
}
