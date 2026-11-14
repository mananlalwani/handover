mod attachment_tiles;
mod attachments;
mod connection;
mod contact_photos;
mod conversation_find;
mod file_actions;
mod image_canvas;
mod image_navigation;
mod media;
mod media_browser;
mod message_text;
mod model;
mod playback;
mod setup;
mod ui;

use adw::prelude::*;

fn main() -> gtk::glib::ExitCode {
    let application = adw::Application::builder()
        .application_id("org.handover.Messages")
        .flags(gtk::gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    application.add_main_option(
        "account",
        gtk::glib::Char::from(0),
        gtk::glib::OptionFlags::NONE,
        gtk::glib::OptionArg::String,
        "Open this messaging account",
        Some("ACCOUNT"),
    );
    application.add_main_option(
        "conversation",
        gtk::glib::Char::from(0),
        gtk::glib::OptionFlags::NONE,
        gtk::glib::OptionArg::String,
        "Open this conversation",
        Some("THREAD"),
    );
    application.add_main_option(
        "reply",
        gtk::glib::Char::from(0),
        gtk::glib::OptionFlags::NONE,
        gtk::glib::OptionArg::None,
        "Focus the conversation composer without sending",
        None,
    );
    let controller = std::rc::Rc::new(std::cell::RefCell::new(None::<std::rc::Rc<ui::Window>>));
    application.connect_command_line(move |app, command_line| {
        let options = command_line.options_dict();
        let account = options.lookup::<String>("account").ok().flatten();
        let thread = options.lookup::<String>("conversation").ok().flatten();
        let reply = options
            .lookup::<bool>("reply")
            .ok()
            .flatten()
            .unwrap_or(false);
        if reply && (account.is_none() || thread.is_none()) {
            return gtk::glib::ExitCode::from(2);
        }
        let target = match (account, thread) {
            (Some(account), Some(thread)) => {
                let id = handover_core::ConversationId::new(
                    handover_core::MessagingAccountId::new(account),
                    thread,
                );
                if handover_core::validate_conversation_id(&id).is_err() {
                    return gtk::glib::ExitCode::from(2);
                }
                Some(id)
            }
            (None, None) => None,
            _ => return gtk::glib::ExitCode::from(2),
        };
        let window = controller
            .borrow_mut()
            .get_or_insert_with(|| ui::Window::new(app))
            .clone();
        if let Some(target) = target {
            window.open(target.clone(), None);
            if reply {
                window.focus_reply(target);
            }
        }
        window.window.present();
        gtk::glib::ExitCode::SUCCESS
    });
    application.set_accels_for_action("win.search", &["<Control>k"]);
    application.set_accels_for_action("win.find", &["<Control>f"]);
    application.set_accels_for_action("win.send", &["<Control>Return"]);
    application.set_accels_for_action("win.previous", &["<Alt>Up"]);
    application.set_accels_for_action("win.next", &["<Alt>Down"]);
    application.set_accels_for_action("win.back-context", &["<Alt>Left"]);
    application.set_accels_for_action("win.select-messages", &["<Control><Shift>m"]);
    application.run()
}
