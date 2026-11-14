//! Compact attachment selection shown above the message composer.

use std::rc::Rc;

use gtk::prelude::*;
use handover_core::Attachment;

/// Build a compact, wrapping preview grid for the current attachment selection.
pub fn selection(
    files: &[Attachment],
    media: &crate::media::Media,
    parent: &adw::ApplicationWindow,
    editable: bool,
    remove: impl Fn(usize) + 'static,
) -> gtk::ScrolledWindow {
    let grid = gtk::FlowBox::new();
    grid.add_css_class("attachment-grid");
    grid.set_selection_mode(gtk::SelectionMode::None);
    grid.set_min_children_per_line(1);
    grid.set_max_children_per_line(5);
    grid.set_row_spacing(7);
    grid.set_column_spacing(7);
    grid.set_homogeneous(false);
    grid.set_valign(gtk::Align::Start);
    grid.set_halign(gtk::Align::Start);

    let remove = Rc::new(remove);
    for (index, file) in files.iter().enumerate() {
        let filename = file
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("Unnamed file");
        let tile = gtk::Overlay::new();
        tile.add_css_class("attachment-tile");
        tile.set_halign(gtk::Align::Start);
        tile.set_hexpand(false);

        if crate::media::is_image(file) {
            let preview = media.composer_attachment(file, parent);
            preview.set_halign(gtk::Align::Start);
            tile.set_child(Some(&preview));
        } else {
            let preview = gtk::Box::new(gtk::Orientation::Horizontal, 9);
            preview.add_css_class("file-preview");
            preview.set_size_request(148, 84);
            let icon = gtk::Image::from_icon_name("text-x-generic-symbolic");
            icon.set_pixel_size(24);
            icon.add_css_class("dim-label");
            let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
            labels.set_hexpand(true);
            labels.set_valign(gtk::Align::Center);
            let name = gtk::Label::new(Some(filename));
            name.add_css_class("attachment-name");
            name.set_xalign(0.0);
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            name.set_max_width_chars(18);
            let detail = gtk::Label::new(Some(
                &file
                    .size_bytes
                    .map(format_size)
                    .unwrap_or_else(|| "File".into()),
            ));
            detail.set_xalign(0.0);
            detail.add_css_class("dim-label");
            labels.append(&name);
            labels.append(&detail);
            preview.append(&icon);
            preview.append(&labels);
            tile.set_child(Some(&preview));
        }

        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("attachment-remove");
        close.set_tooltip_text(Some(&format!("Remove {filename}")));
        close.set_sensitive(editable);
        close.set_visible(editable);
        close.set_halign(gtk::Align::End);
        close.set_valign(gtk::Align::Start);
        let callback = Rc::clone(&remove);
        close.connect_clicked(move |_| callback(index));
        tile.add_overlay(&close);

        let child = gtk::FlowBoxChild::new();
        child.set_child(Some(&tile));
        grid.insert(&child, -1);
    }

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.add_css_class("attachment-selection");
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.set_max_content_height(180);
    scrolled.set_propagate_natural_height(true);
    scrolled.set_child(Some(&grid));
    scrolled
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
