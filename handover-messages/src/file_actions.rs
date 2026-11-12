//! Export a locally staged attachment to a user-selected destination.
use adw::prelude::*;
use gtk::{gio, glib};
use handover_core::Attachment;
use std::{io::Read, os::unix::fs::OpenOptionsExt, path::Path};

pub fn save(parent: &impl IsA<gtk::Window>, attachment: &Attachment) {
    let Some(source) = attachment.staged_path.clone() else {
        return;
    };
    let name = attachment
        .name
        .as_deref()
        .and_then(handover_core::sanitize_file_name)
        .unwrap_or_else(|| "Attachment".into());
    let dialog = gtk::FileDialog::builder()
        .title("Save attachment")
        .initial_name(&name)
        .build();
    let parent: gtk::Window = parent.as_ref().clone();
    let weak = parent.downgrade();
    dialog.save(Some(&parent),None::<&gio::Cancellable>,move |result| {
        let Ok(file)=result else {return;};
        let Some(destination)=file.path() else { if let Some(parent)=weak.upgrade(){show_error(&parent,"Choose a local folder to save this attachment.");}return;};
        let (sender,receiver)=async_channel::bounded(1);
        let source=source.clone();
        std::thread::spawn(move || {let _=sender.send_blocking(copy_file(Path::new(&source),&destination));});
        let weak=weak.clone();
        glib::spawn_future_local(async move {
            if !matches!(receiver.recv().await,Ok(Ok(()))) && let Some(parent)=weak.upgrade(){show_error(&parent,"The attachment could not be saved. Check that its local file is available and the destination is writable.");}
        });
    });
}
pub fn save_button(parent: &impl IsA<gtk::Window>, attachment: &Attachment) -> gtk::Button {
    let button = gtk::Button::from_icon_name("document-save-symbolic");
    button.add_css_class("flat");
    button.set_tooltip_text(Some("Save attachment as…"));
    button.set_sensitive(attachment.staged_path.is_some());
    let parent: gtk::Window = parent.as_ref().clone();
    let weak = parent.downgrade();
    let attachment = attachment.clone();
    button.connect_clicked(move |_| {
        if let Some(parent) = weak.upgrade() {
            save(&parent, &attachment);
        }
    });
    button
}
fn show_error(parent: &gtk::Window, message: &str) {
    let dialog = adw::MessageDialog::new(
        Some(parent),
        Some("Unable to save attachment"),
        Some(message),
    );
    dialog.add_response("close", "Close");
    dialog.set_close_response("close");
    dialog.present();
}
fn copy_file(source: &Path, destination: &Path) -> Result<(), ()> {
    let mut input = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(source)
        .map_err(|_| ())?;
    let metadata = input.metadata().map_err(|_| ())?;
    let limit = crate::attachments::MAX_ATTACHMENT_BYTES;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(());
    }
    let parent = destination.parent().ok_or(())?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|_| ())?;
    let count = std::io::copy(&mut (&mut input).take(limit + 1), &mut staged).map_err(|_| ())?;
    if count > limit || count != metadata.len() {
        return Err(());
    }
    staged.as_file().sync_all().map_err(|_| ())?;
    staged.persist(destination).map_err(|_| ())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_replaces_the_chosen_file_without_following_its_symlink() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("attachment");
        let existing = root.path().join("existing");
        let target = root.path().join("chosen");
        std::fs::write(&source, b"attachment bytes").unwrap();
        std::fs::write(&existing, b"keep").unwrap();
        std::os::unix::fs::symlink(&existing, &target).unwrap();
        copy_file(&source, &target).unwrap();
        assert_eq!(std::fs::read(target).unwrap(), b"attachment bytes");
        assert_eq!(std::fs::read(existing).unwrap(), b"keep");
    }
    #[test]
    fn unavailable_or_nonregular_sources_preserve_the_destination() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"keep").unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(copy_file(&link, &target).is_err());
        assert!(copy_file(root.path(), &target).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"keep");
    }
}
