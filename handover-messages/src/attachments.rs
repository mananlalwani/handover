//! Local files staged by the message composer.

use gtk::gio::prelude::*;
use std::fs;

use handover_core::Attachment;

/// A single file waiting to be sent in a conversation.
///
/// The composer keeps at most one of these per conversation because the
/// normalized send-file command accepts one file and an optional caption.
#[derive(Clone, Eq, PartialEq)]
pub struct PendingAttachment {
    pub uri: String,
    pub attachment: Attachment,
}

/// Composer file size limit, checked again by the daemon at send time.
pub const MAX_ATTACHMENT_BYTES: u64 = 50 * 1024 * 1024;

/// Build composer metadata for a local regular file without reading its data.
pub fn from_file(file: &gtk::gio::File) -> Result<PendingAttachment, String> {
    let path = file
        .path()
        .ok_or_else(|| "Only local files can be attached".to_owned())?;
    let display_path = path
        .to_str()
        .ok_or_else(|| "The file path must use UTF-8".to_owned())?;
    let metadata = fs::symlink_metadata(&path).map_err(|_| "Unable to access file".to_owned())?;
    if !metadata.file_type().is_file() {
        return Err("Attachment must be a regular file".to_owned());
    }
    let size_bytes = metadata.len();
    if size_bytes > MAX_ATTACHMENT_BYTES {
        return Err("File exceeds the 50 MiB attachment limit".to_owned());
    }

    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    let content_type = gtk::gio::content_type_guess(name.as_deref(), None).0;
    let mime = gtk::gio::content_type_get_mime_type(&content_type).map(|mime| mime.to_string());
    let uri = file.uri().to_string();

    Ok(PendingAttachment {
        uri: uri.clone(),
        attachment: Attachment {
            local_id: "composer-preview".into(),
            mime,
            name,
            size_bytes: Some(size_bytes),
            staged_path: Some(display_path.to_owned()),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn builds_metadata_for_a_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        fs::write(&path, b"hello").unwrap();

        let pending = from_file(&gtk::gio::File::for_path(&path)).unwrap();

        assert_eq!(pending.attachment.name.as_deref(), Some("note.txt"));
        assert_eq!(pending.attachment.mime.as_deref(), Some("text/plain"));
        assert_eq!(pending.attachment.size_bytes, Some(5));
        assert_eq!(pending.attachment.staged_path.as_deref(), path.to_str());
        assert_eq!(pending.uri, gtk::gio::File::for_path(path).uri());
    }

    #[test]
    fn rejects_nonlocal_files() {
        let file = gtk::gio::File::for_uri("https://example.test/file.txt");
        assert!(from_file(&file).is_err());
    }

    #[test]
    fn rejects_missing_paths_directories_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.txt");
        assert!(from_file(&gtk::gio::File::for_path(missing)).is_err());
        assert!(from_file(&gtk::gio::File::for_path(dir.path())).is_err());

        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        fs::write(&target, b"data").unwrap();
        symlink(&target, &link).unwrap();
        assert!(from_file(&gtk::gio::File::for_path(link)).is_err());
    }

    #[test]
    fn rejects_files_over_the_composer_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.bin");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_ATTACHMENT_BYTES + 1).unwrap();
        assert!(from_file(&gtk::gio::File::for_path(path)).is_err());
    }
}
