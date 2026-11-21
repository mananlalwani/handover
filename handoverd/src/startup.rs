//! Claim the daemon runtime before state recovery can mutate a live session.
use std::fs::{self, File, OpenOptions, Permissions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;
use tokio::net::UnixStream;

pub(crate) async fn claim(directory: &Path) -> io::Result<File> {
    fs::create_dir_all(directory)?;
    fs::set_permissions(directory, Permissions::from_mode(0o700))?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join("handoverd.lock"))?;
    lock.try_lock()
        .map_err(|error| io::Error::other(format!("daemon runtime is already claimed: {error}")))?;
    // Older daemons do not hold this lock. Detect their live IPC socket before
    // opening the backend or restoring its routing journal. Never unlink the lock
    // file: another process may already have opened the same inode.
    match tokio::time::timeout(
        Duration::from_secs(1),
        UnixStream::connect(directory.join("handoverd.sock")),
    )
    .await
    {
        Ok(Err(error))
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            Ok(lock)
        }
        Ok(Ok(_)) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "another handoverd is already listening",
        )),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "could not check the existing daemon socket",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn lock_covers_startup_before_the_socket_exists_and_releases_on_exit() {
        let directory = tempfile::tempdir().unwrap();
        let first = claim(directory.path()).await.unwrap();
        assert!(claim(directory.path()).await.is_err());
        drop(first);
        let _replacement = claim(directory.path()).await.unwrap();
    }
}
