//! Export a locally staged attachment to a user-selected destination.
use adw::prelude::*;
use gtk::{gio, glib};
use handover_core::Attachment;
use std::{
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

const MAX_BULK_ATTACHMENTS: usize = 64;
const MAX_BULK_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SUFFIX_ATTEMPTS: u32 = 1024;

/// Save a bounded batch of locally staged attachments into a chosen folder.
pub fn save_many(parent: &impl IsA<gtk::Window>, attachments: &[Attachment]) {
    if attachments.is_empty() {
        return;
    }
    let attachments = attachments.to_vec();
    let dialog = gtk::FileDialog::builder()
        .title("Choose folder for attachments")
        .build();
    let parent: gtk::Window = parent.as_ref().clone();
    let weak = parent.downgrade();
    dialog.select_folder(Some(&parent), None::<&gio::Cancellable>, move |result| {
        let Ok(folder) = result else {
            return;
        };
        let Some(destination) = folder.path() else {
            if let Some(parent) = weak.upgrade() {
                show_error(&parent, "Choose a local folder to save these attachments.");
            }
            return;
        };
        let (sender, receiver) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = sender.send_blocking(save_many_to_dir(&attachments, &destination));
        });
        let weak = weak.clone();
        glib::spawn_future_local(async move {
            if let Ok(summary) = receiver.recv().await
                && let Some(parent) = weak.upgrade()
            {
                let dialog = adw::MessageDialog::new(
                    Some(&parent),
                    Some("Attachment export complete"),
                    Some(&format!(
                        "Saved: {}\nSkipped: {}\nFailed: {}\nOnly locally available files can be saved.",
                        summary.saved, summary.skipped, summary.failed
                    )),
                );
                dialog.add_response("close", "Close");
                dialog.set_close_response("close");
                dialog.present();
            }
        });
    });
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ExportSummary {
    saved: usize,
    skipped: usize,
    failed: usize,
}

fn save_many_to_dir(attachments: &[Attachment], directory: &Path) -> ExportSummary {
    let mut summary = ExportSummary::default();
    let mut attempted = 0u64;
    for attachment in attachments {
        if summary.saved + summary.skipped + summary.failed >= MAX_BULK_ATTACHMENTS {
            summary.skipped += 1;
            continue;
        }
        let Some(source) = attachment.staged_path.as_deref().map(Path::new) else {
            summary.skipped += 1;
            continue;
        };
        let Some((size, input)) = source_size(source) else {
            summary.skipped += 1;
            continue;
        };
        let remaining = MAX_BULK_BYTES.saturating_sub(attempted);
        if size > remaining {
            summary.skipped += 1;
            continue;
        }
        attempted += size;
        let name = attachment
            .name
            .as_deref()
            .and_then(handover_core::sanitize_file_name)
            .unwrap_or_else(|| "Attachment".into());
        match copy_unique(input, size, remaining, directory, &name) {
            Ok(()) => summary.saved += 1,
            Err(()) => summary.failed += 1,
        }
    }
    summary
}

fn source_size(source: &Path) -> Option<(u64, std::fs::File)> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(source)
        .ok()?;
    let metadata = file.metadata().ok()?;
    (metadata.is_file() && metadata.len() <= crate::attachments::MAX_ATTACHMENT_BYTES)
        .then_some((metadata.len(), file))
}

fn copy_unique(
    mut input: std::fs::File,
    size: u64,
    remaining_budget: u64,
    directory: &Path,
    name: &str,
) -> Result<(), ()> {
    let mut staged = tempfile::NamedTempFile::new_in(directory).map_err(|_| ())?;
    let count = std::io::copy(
        &mut (&mut input).take(
            crate::attachments::MAX_ATTACHMENT_BYTES
                .min(size)
                .min(remaining_budget)
                .saturating_add(1),
        ),
        &mut staged,
    )
    .map_err(|_| ())?;
    if count > crate::attachments::MAX_ATTACHMENT_BYTES || count != size {
        return Err(());
    }
    staged.as_file().sync_all().map_err(|_| ())?;
    for index in 1..=MAX_SUFFIX_ATTEMPTS {
        let destination = unique_path(directory, name, index);
        match staged.persist_noclobber(&destination) {
            Ok(_) => return Ok(()),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                staged = error.file;
            }
            Err(_) => return Err(()),
        }
    }
    Err(())
}

fn unique_path(directory: &Path, name: &str, index: u32) -> PathBuf {
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Attachment");
    let extension = path.extension().and_then(|s| s.to_str());
    let candidate = if index == 1 {
        name.to_owned()
    } else if let Some(extension) = extension {
        format!("{stem} ({index}).{extension}")
    } else {
        format!("{stem} ({index})")
    };
    directory.join(candidate)
}

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
    fn attachment(path: Option<&Path>, name: &str) -> Attachment {
        Attachment {
            local_id: "id".into(),
            mime: None,
            name: Some(name.into()),
            size_bytes: None,
            staged_path: path.map(|p| p.to_string_lossy().into_owned()),
        }
    }
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

    #[test]
    fn bulk_export_numbers_duplicate_names_and_never_overwrites_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        std::fs::write(&first, b"first bytes").unwrap();
        std::fs::write(&second, b"second bytes").unwrap();
        std::fs::write(out.join("photo.jpg"), b"preexisting").unwrap();
        let items = [
            attachment(Some(&first), "photo.jpg"),
            attachment(Some(&second), "photo.jpg"),
        ];
        let result = save_many_to_dir(&items, &out);
        assert_eq!(
            result,
            ExportSummary {
                saved: 2,
                skipped: 0,
                failed: 0
            }
        );
        assert_eq!(
            std::fs::read(out.join("photo.jpg")).unwrap(),
            b"preexisting"
        );
        assert_eq!(
            std::fs::read(out.join("photo (2).jpg")).unwrap(),
            b"first bytes"
        );
        assert_eq!(
            std::fs::read(out.join("photo (3).jpg")).unwrap(),
            b"second bytes"
        );
        assert_eq!(std::fs::read(&first).unwrap(), b"first bytes");
        assert_eq!(std::fs::read(&second).unwrap(), b"second bytes");
    }

    #[test]
    fn bulk_export_skips_unavailable_sources_without_touching_outputs() {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let missing = root.path().join("missing");
        let items = [
            attachment(Some(&missing), "gone.bin"),
            attachment(None, "remote.bin"),
        ];
        let result = save_many_to_dir(&items, &out);
        assert_eq!(
            result,
            ExportSummary {
                saved: 0,
                skipped: 2,
                failed: 0
            }
        );
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
    }

    #[test]
    fn bulk_export_preserves_a_run_of_existing_suffix_destinations() {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let source = root.path().join("source");
        std::fs::write(&source, b"source bytes").unwrap();
        for index in 1..=24 {
            std::fs::write(
                unique_path(&out, "report.txt", index),
                format!("keep {index}"),
            )
            .unwrap();
        }

        let result = save_many_to_dir(&[attachment(Some(&source), "report.txt")], &out);

        assert_eq!(result.saved, 1);
        for index in 1..=24 {
            assert_eq!(
                std::fs::read(unique_path(&out, "report.txt", index)).unwrap(),
                format!("keep {index}").as_bytes()
            );
        }
        assert_eq!(
            std::fs::read(unique_path(&out, "report.txt", 25)).unwrap(),
            b"source bytes"
        );
        assert_eq!(std::fs::read(&source).unwrap(), b"source bytes");
    }
}
