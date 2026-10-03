//! Per-user restricted storage for native Google Messages session records.
//!
//! Payloads are opaque, versioned by the caller, and capped. This module only
//! provides local file protection; it does not encrypt data against the same
//! user or define which protocol fields belong in a restorable session.

use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use zeroize::{Zeroize, Zeroizing};

const MAX_RECORD_SIZE: usize = 1024 * 1024;
const MAX_ACCOUNT_KEY: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStoreError {
    InvalidAccountKey,
    UnsafeDirectory,
    UnsafeFile,
    RecordTooLarge,
    InvalidRecord,
    Io,
}

/// Session bytes are redacted and erased when dropped.
pub struct SessionRecord(Zeroizing<Vec<u8>>);

impl std::fmt::Debug for SessionRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionRecord { redacted }")
    }
}

impl SessionRecord {
    pub fn new(mut bytes: Vec<u8>) -> Result<Self, SessionStoreError> {
        if bytes.is_empty() {
            return Err(SessionStoreError::InvalidRecord);
        }
        if bytes.len() > MAX_RECORD_SIZE {
            bytes.zeroize();
            return Err(SessionStoreError::RecordTooLarge);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    directory: PathBuf,
}

/// Build a stable filename key without placing the account identifier in the
/// directory listing. This hash is a name, not a password or encryption key.
pub fn account_key_for_identity(identity: &[u8]) -> Result<String, SessionStoreError> {
    if identity.is_empty() || identity.len() > 1024 {
        return Err(SessionStoreError::InvalidAccountKey);
    }
    let digest = Sha256::digest(identity);
    let mut key = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut key, "{byte:02x}").expect("writing into a String cannot fail");
    }
    Ok(key)
}

impl SessionStore {
    pub fn default_store() -> Result<Self, SessionStoreError> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .ok_or(SessionStoreError::UnsafeDirectory)?;
        if !base.is_absolute() {
            return Err(SessionStoreError::UnsafeDirectory);
        }
        Ok(Self::new(base.join("handover/gmessages-native")))
    }

    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn store(
        &self,
        account_key: &str,
        record: &SessionRecord,
    ) -> Result<(), SessionStoreError> {
        ensure_directory(&self.directory)?;
        let target = self.account_path(account_key)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".session-")
            .tempfile_in(&self.directory)
            .map_err(|_| SessionStoreError::Io)?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| SessionStoreError::Io)?;
        temporary
            .write_all(record.as_bytes())
            .map_err(|_| SessionStoreError::Io)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| SessionStoreError::Io)?;
        temporary
            .persist(&target)
            .map_err(|_| SessionStoreError::Io)?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
            .map_err(|_| SessionStoreError::Io)?;
        sync_directory(&self.directory)?;
        Ok(())
    }

    pub fn load(&self, account_key: &str) -> Result<Option<SessionRecord>, SessionStoreError> {
        let path = self.account_path(account_key)?;
        if !check_directory(&self.directory)? {
            return Ok(None);
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
                return Err(SessionStoreError::UnsafeFile);
            }
            Err(_) => return Err(SessionStoreError::Io),
        };
        let metadata = file.metadata().map_err(|_| SessionStoreError::Io)?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(SessionStoreError::UnsafeFile);
        }
        if metadata.len() > MAX_RECORD_SIZE as u64 {
            return Err(SessionStoreError::RecordTooLarge);
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(metadata.len() as usize));
        file.take((MAX_RECORD_SIZE + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| SessionStoreError::Io)?;
        if bytes.len() > MAX_RECORD_SIZE {
            return Err(SessionStoreError::RecordTooLarge);
        }
        if bytes.is_empty() {
            return Err(SessionStoreError::InvalidRecord);
        }
        Ok(Some(SessionRecord(bytes)))
    }

    pub fn delete(&self, account_key: &str) -> Result<bool, SessionStoreError> {
        let path = self.account_path(account_key)?;
        if !check_directory(&self.directory)? {
            return Ok(false);
        }
        match fs::remove_file(path) {
            Ok(()) => {
                sync_directory(&self.directory)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(SessionStoreError::Io),
        }
    }

    fn account_path(&self, account_key: &str) -> Result<PathBuf, SessionStoreError> {
        if account_key.is_empty()
            || account_key.len() > MAX_ACCOUNT_KEY
            || !account_key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(SessionStoreError::InvalidAccountKey);
        }
        Ok(self.directory.join(format!("{account_key}.session")))
    }
}

fn ensure_directory(directory: &Path) -> Result<(), SessionStoreError> {
    fs::create_dir_all(directory).map_err(|_| SessionStoreError::Io)?;
    let metadata = fs::symlink_metadata(directory).map_err(|_| SessionStoreError::Io)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(SessionStoreError::UnsafeDirectory);
    }
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .map_err(|_| SessionStoreError::Io)?;
    let _ = check_directory(directory)?;
    Ok(())
}

fn check_directory(directory: &Path) -> Result<bool, SessionStoreError> {
    let metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(SessionStoreError::Io),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(SessionStoreError::UnsafeDirectory);
    }
    Ok(true)
}

fn sync_directory(directory: &Path) -> Result<(), SessionStoreError> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| SessionStoreError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_loads_and_deletes_with_private_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("sessions");
        let store = SessionStore::new(&directory);
        let record = SessionRecord::new(b"synthetic-session".to_vec()).unwrap();
        store.store("account_1", &record).unwrap();
        let loaded = store.load("account_1").unwrap().unwrap();
        assert_eq!(loaded.as_bytes(), b"synthetic-session");
        assert_eq!(format!("{loaded:?}"), "SessionRecord { redacted }");
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(directory.join("account_1.session"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(store.delete("account_1").unwrap());
        assert!(store.load("account_1").unwrap().is_none());
        assert!(!store.delete("account_1").unwrap());
    }

    #[test]
    fn rejects_bad_account_names_and_oversized_records() {
        let temp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(temp.path().join("sessions"));
        for invalid in ["", "../escape", "person@example.com", "a/b", "."] {
            assert_eq!(
                store.account_path(invalid),
                Err(SessionStoreError::InvalidAccountKey)
            );
        }
        assert_eq!(
            SessionRecord::new(vec![0; MAX_RECORD_SIZE + 1]).unwrap_err(),
            SessionStoreError::RecordTooLarge
        );
    }

    #[test]
    fn account_filename_key_is_stable_and_does_not_expose_identity() {
        let first = account_key_for_identity(b"person@example.test").unwrap();
        assert_eq!(
            first,
            account_key_for_identity(b"person@example.test").unwrap()
        );
        assert_ne!(
            first,
            account_key_for_identity(b"other@example.test").unwrap()
        );
        assert!(!first.contains("person"));
        assert_eq!(first.len(), 64);
        assert!(account_key_for_identity(b"").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_files_and_directories() {
        let temp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(temp.path().join("sessions"));
        fs::create_dir(&store.directory).unwrap();
        fs::set_permissions(&store.directory, fs::Permissions::from_mode(0o700)).unwrap();
        let secret = temp.path().join("outside");
        fs::write(&secret, b"synthetic-secret").unwrap();
        std::os::unix::fs::symlink(&secret, store.directory.join("acct.session")).unwrap();
        assert_eq!(
            store.load("acct").unwrap_err(),
            SessionStoreError::UnsafeFile
        );

        let linked = temp.path().join("linked");
        std::os::unix::fs::symlink(temp.path(), &linked).unwrap();
        assert_eq!(
            SessionStore::new(linked).store("acct", &SessionRecord::new(b"data".to_vec()).unwrap()),
            Err(SessionStoreError::UnsafeDirectory)
        );
    }
}
