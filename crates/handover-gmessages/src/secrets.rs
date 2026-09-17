//! Secret storage owned by the helper process.
//!
//! Credential bundles (Google cookies, tokens, keys) rest in one 0600 file
//! per account below `${XDG_STATE_HOME:-~/.local/state}/handover/gmessages`
//! (0700 directory). Files are written via temp-file + atomic rename.
//! Nothing here logs bundle contents; errors name the account only.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum SecretError {
    Io(std::io::Error),
    Unavailable(String),
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "secret storage I/O error: {error}"),
            Self::Unavailable(detail) => write!(formatter, "secret storage unavailable: {detail}"),
        }
    }
}

impl std::error::Error for SecretError {}

impl From<std::io::Error> for SecretError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn default_directory() -> Result<PathBuf, SecretError> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_default();
            PathBuf::from(home).join(".local/state")
        });
    if !base.is_absolute() {
        return Err(SecretError::Unavailable(
            "state home must be absolute".into(),
        ));
    }
    Ok(base.join("handover/gmessages"))
}

fn account_file(directory: &Path, account: &str) -> Result<PathBuf, SecretError> {
    if account.is_empty()
        || account.len() > 128
        || account
            .chars()
            .any(|c| c == '/' || c == '\\' || c == '\0' || c.is_control() || c == '.')
    {
        return Err(SecretError::Unavailable("invalid account name".into()));
    }
    Ok(directory.join(format!("{account}.credentials.json")))
}

fn ensure_directory(directory: &Path) -> Result<(), SecretError> {
    std::fs::create_dir_all(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Persist a credential bundle for one account. Overwrites atomically.
pub fn store_bundle(directory: &Path, account: &str, bundle: &[u8]) -> Result<(), SecretError> {
    ensure_directory(directory)?;
    let target = account_file(directory, account)?;
    let tmp = target.with_extension("credentials.json.tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        file.write_all(bundle)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, &target)?;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub fn load_bundle(directory: &Path, account: &str) -> Result<Option<Vec<u8>>, SecretError> {
    let target = account_file(directory, account)?;
    match std::fs::read(&target) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Delete a credential bundle. Returns `true` when a bundle existed.
pub fn delete_bundle(directory: &Path, account: &str) -> Result<bool, SecretError> {
    let target = account_file(directory, account)?;
    match std::fs::remove_file(&target) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_account_names() {
        let directory = tempfile::tempdir().expect("tempdir").keep();
        for name in ["", "../escape", "a/b", "a\\b", ".", "a.b", "a\0b"] {
            assert!(
                account_file(&directory, name).is_err(),
                "name {name:?} must be rejected"
            );
        }
    }

    #[test]
    fn round_trips_bundle_with_strict_permissions() {
        let directory = tempfile::tempdir().expect("tempdir");
        store_bundle(directory.path(), "work", b"{\"opaque\":true}").expect("store");
        let loaded = load_bundle(directory.path(), "work")
            .expect("load")
            .expect("present");
        assert_eq!(loaded, b"{\"opaque\":true}");
        let permissions = std::fs::metadata(directory.path().join("work.credentials.json"))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(permissions, 0o600);
        assert!(delete_bundle(directory.path(), "work").expect("delete"));
        assert!(
            load_bundle(directory.path(), "work")
                .expect("load")
                .is_none()
        );
    }
}
