//! Native presentation of the existing setup tool's bounded, sanitized status stream.
use adw::prelude::*;
use gtk::{gio, glib};
use std::{
    cell::{Cell, RefCell},
    ffi::OsStr,
    path::PathBuf,
    rc::Rc,
};

struct Setup {
    window: adw::ApplicationWindow,
    picker: gtk::DropDown,
    choose: gtk::Button,
    connect: gtk::Button,
    status: gtk::Label,
    verification: gtk::Label,
    browsers: RefCell<Vec<PathBuf>>,
    browser: RefCell<Option<PathBuf>>,
    process: RefCell<Option<gio::Subprocess>>,
    buffer: RefCell<Vec<u8>>,
    inventory: Cell<bool>,
    closing: Cell<bool>,
    completed: Cell<bool>,
}

pub fn present(parent: &adw::ApplicationWindow) {
    let window = adw::ApplicationWindow::builder()
        .application(&parent.application().unwrap())
        .transient_for(parent)
        .modal(true)
        .title("Connect Google Messages")
        .default_width(480)
        .default_height(460)
        .build();
    let toolbar = adw::ToolbarView::new();
    window.add_css_class("handover-messages");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header.add_css_class("app-chrome");
    let space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    space.set_hexpand(true);
    header.append(&space);
    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.add_css_class("chrome-close");
    close.set_tooltip_text(Some("Close setup"));
    let weak = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = weak.upgrade() {
            window.close();
        }
    });
    header.append(&close);
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&header));
    toolbar.add_top_bar(&handle);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(24);
    content.set_margin_end(24);
    let icon = gtk::Image::from_icon_name("chat-message-new-symbolic");
    icon.set_pixel_size(48);
    icon.add_css_class("accent");
    content.append(&icon);
    let title = gtk::Label::new(Some("Connect your messages"));
    title.add_css_class("title-1");
    content.append(&title);
    let instructions = gtk::Label::new(Some(
        "Sign in to Google Messages in your browser. When you're done, close the browser from its menu and verify on your phone if asked.",
    ));
    instructions.set_wrap(true);
    instructions.set_justify(gtk::Justification::Center);
    instructions.set_max_width_chars(48);
    instructions.add_css_class("dim-label");
    content.append(&instructions);
    let picker = gtk::DropDown::from_strings(&["Looking for installed browsers…"]);
    let choose = gtk::Button::with_label("Choose browser executable…");
    let browsers = gtk::ListBox::new();
    browsers.set_selection_mode(gtk::SelectionMode::None);
    browsers.add_css_class("boxed-list");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_top(8);
    row.set_margin_bottom(8);
    row.set_margin_start(14);
    row.set_margin_end(8);
    let label = gtk::Label::new(Some("Browser"));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);
    row.append(&picker);
    browsers.append(&row);
    content.append(&browsers);
    choose.add_css_class("flat");
    choose.set_halign(gtk::Align::Center);
    content.append(&choose);
    let status = gtk::Label::new(Some("Looking for installed browsers…"));
    status.set_wrap(true);
    status.set_xalign(0.0);
    content.append(&status);
    let verification = gtk::Label::new(None);
    verification.set_wrap(true);
    verification.add_css_class("title-1");
    verification.set_visible(false);
    content.append(&verification);
    let privacy = gtk::Label::new(Some(
        "Your normal browser profile stays separate. Authentication is saved in your desktop credential store. Messaging pauses during setup.",
    ));
    privacy.set_wrap(true);
    privacy.add_css_class("dim-label");
    privacy.add_css_class("caption");
    privacy.set_max_width_chars(52);
    privacy.set_justify(gtk::Justification::Center);
    content.append(&privacy);
    let connect = gtk::Button::with_label("Continue in browser");
    connect.add_css_class("suggested-action");
    connect.set_sensitive(false);
    connect.set_halign(gtk::Align::Center);
    connect.set_margin_top(4);
    content.append(&connect);
    toolbar.set_content(Some(&content));
    window.set_content(Some(&toolbar));
    let setup = Rc::new(Setup {
        window,
        picker,
        choose,
        connect,
        status,
        verification,
        browsers: RefCell::new(vec![]),
        browser: RefCell::new(None),
        process: RefCell::new(None),
        buffer: RefCell::new(vec![]),
        inventory: Cell::new(true),
        closing: Cell::new(false),
        completed: Cell::new(false),
    });
    let weak = Rc::downgrade(&setup);
    setup.picker.connect_selected_notify(move |picker| {
        if let Some(setup) = weak.upgrade() {
            *setup.browser.borrow_mut() = setup
                .browsers
                .borrow()
                .get(picker.selected() as usize)
                .cloned();
            setup.controls();
        }
    });
    let weak = Rc::downgrade(&setup);
    setup.choose.connect_clicked(move |_| {
        if let Some(setup) = weak.upgrade() {
            let weak = Rc::downgrade(&setup);
            gtk::FileDialog::builder()
                .title("Choose a Chromium browser executable")
                .build()
                .open(
                    Some(&setup.window),
                    None::<&gio::Cancellable>,
                    move |result| {
                        if let (Some(setup), Ok(file)) = (weak.upgrade(), result)
                            && let Some(path) = file.path()
                        {
                            *setup.browser.borrow_mut() = Some(path);
                            setup
                                .status
                                .set_label("Browser selected. Continue to sign in.");
                            setup.controls();
                        }
                    },
                );
        }
    });
    let weak = Rc::downgrade(&setup);
    setup.connect.connect_clicked(move |_| {
        if let Some(setup) = weak.upgrade() {
            let browser = setup.browser.borrow().clone();
            if let Some(browser) = browser {
                setup.start(vec!["--browser".into(), browser.into_os_string()], false);
            }
        }
    });
    let weak = Rc::downgrade(&setup);
    setup.window.connect_close_request(move |_| {
        if let Some(setup) = weak.upgrade() {
            if let Some(process) = setup.process.borrow().as_ref() {
                setup.closing.set(true);
                setup.status.set_label("Finishing cancellation…");
                process.send_signal(libc::SIGTERM);
                let weak = Rc::downgrade(&setup);
                glib::timeout_add_local_once(std::time::Duration::from_secs(30), move || {
                    if let Some(setup) = weak.upgrade()
                        && let Some(process) = setup.process.borrow().as_ref()
                    {
                        process.force_exit();
                    }
                });
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        } else {
            glib::Propagation::Proceed
        }
    });
    let keepalive = Rc::new(RefCell::new(Some(setup.clone())));
    setup.window.connect_destroy(move |_| {
        keepalive.borrow_mut().take();
    });
    setup.window.present();
    setup.start(vec!["--list-browsers".into()], true);
}

impl Setup {
    fn controls(&self) {
        let running = self.process.borrow().is_some();
        self.picker.set_sensitive(!running);
        self.choose.set_sensitive(!running);
        self.connect
            .set_sensitive(!running && self.browser.borrow().is_some());
    }
    fn start(self: &Rc<Self>, args: Vec<std::ffi::OsString>, inventory: bool) {
        if self.process.borrow().is_some() {
            return;
        }
        self.inventory.set(inventory);
        self.completed.set(false);
        self.buffer.borrow_mut().clear();
        self.verification.set_visible(false);
        self.verification.set_label("");
        let program = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.parent()
                    .map(|parent| parent.join("handover-google-messages-setup"))
            })
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from("handover-google-messages-setup"));
        let mut argv = vec![program.as_os_str()];
        argv.extend(args.iter().map(AsRef::<OsStr>::as_ref));
        let process = match gio::Subprocess::newv(
            &argv,
            gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
        ) {
            Ok(process) => process,
            Err(_) => {
                self.status.set_label(
                    "Install Handover's optional Google Messages setup component to continue.",
                );
                return;
            }
        };
        let stream = process.stdout_pipe().unwrap();
        *self.process.borrow_mut() = Some(process.clone());
        self.controls();
        self.read(stream);
        let setup = self.clone();
        process.wait_check_async(None::<&gio::Cancellable>,move|result|{
            setup.process.borrow_mut().take();setup.controls();
            setup.verification.set_label("");
            setup.verification.set_visible(false);
            if !setup.inventory.get()&&!setup.completed.get(){setup.status.set_label("Setup ended before completion could be confirmed.");}
            else if setup.inventory.get()&&result.is_err(){setup.status.set_label("Browser discovery failed. Install the optional setup component or choose a browser executable.");}
            if setup.closing.get(){setup.window.destroy();}
        });
    }
    fn read(self: &Rc<Self>, stream: gio::InputStream) {
        let setup = self.clone();
        let next = stream.clone();
        stream.read_bytes_async(
            1024,
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |result| {
                let Ok(bytes) = result else {
                    return;
                };
                if bytes.is_empty() {
                    return;
                }
                {
                    let mut pending = setup.buffer.borrow_mut();
                    pending.extend_from_slice(&bytes);
                    if pending.len() > 16 * 1024 {
                        drop(pending);
                        setup.invalid();
                        return;
                    }
                }
                loop {
                    let line = {
                        let mut pending = setup.buffer.borrow_mut();
                        pending
                            .iter()
                            .position(|byte| *byte == b'\n')
                            .map(|end| pending.drain(..=end).collect::<Vec<_>>())
                    };
                    let Some(line) = line else {
                        break;
                    };
                    if !setup.record(&line) {
                        setup.invalid();
                        return;
                    }
                }
                setup.read(next);
            },
        );
    }
    fn invalid(&self) {
        self.status
            .set_label("Setup returned an invalid status. Please try again.");
        if let Some(process) = self.process.borrow().as_ref() {
            process.send_signal(libc::SIGTERM);
        }
    }
    fn record(&self, line: &[u8]) -> bool {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
            return false;
        };
        if self.inventory.get() {
            let Some(browsers) = value
                .get("browsers")
                .and_then(|value| value.as_array())
                .filter(|items| items.len() <= 32)
            else {
                return false;
            };
            let mut paths = vec![];
            let mut names = vec![];
            for browser in browsers {
                let (Some(name), Some(path)) = (
                    browser
                        .get("name")
                        .and_then(|value| value.as_str())
                        .filter(|name| name.len() <= 128),
                    browser
                        .get("path")
                        .and_then(|value| value.as_str())
                        .filter(|path| path.len() <= 4096),
                ) else {
                    return false;
                };
                if !std::path::Path::new(path).is_absolute() {
                    return false;
                }
                names.push(name.to_owned());
                paths.push(PathBuf::from(path));
            }
            *self.browsers.borrow_mut() = paths;
            *self.browser.borrow_mut() = self.browsers.borrow().first().cloned();
            self.picker.set_model(Some(&gtk::StringList::new(
                &names.iter().map(String::as_str).collect::<Vec<_>>(),
            )));
            self.status.set_label(if names.is_empty() {
                "Choose an installed Chromium browser executable."
            } else {
                "Choose a browser, then continue."
            });
            true
        } else {
            let Some(status) = value
                .get("status")
                .and_then(|value| value.as_str())
                .filter(|status| matches!(*status, "progress" | "failed" | "saved"))
            else {
                return false;
            };
            let Some(message) = value
                .get("message")
                .and_then(|value| value.as_str())
                .filter(|message| message.len() <= 2048)
            else {
                return false;
            };
            self.completed.set(matches!(status, "saved" | "failed"));
            self.status.set_label(if status == "saved" {
                "Setup saved. Handover is reconnecting the account."
            } else {
                message
            });
            let symbol = value
                .get("verification")
                .and_then(|value| value.as_str())
                .filter(|symbol| symbol.chars().count() <= 32);
            self.verification.set_visible(
                status == "progress" && symbol.is_some_and(|symbol| !symbol.is_empty()),
            );
            self.verification.set_label(&if status == "progress" {
                symbol
                    .map(|symbol| format!("Confirm {symbol} on your phone"))
                    .unwrap_or_default()
            } else {
                String::new()
            });
            true
        }
    }
}
