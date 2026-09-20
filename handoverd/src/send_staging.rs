//! Private staging for accepted outbound files.
//!
//! Sharing queues a path that the backend opens later. A file in a
//! directory writable by another local user could be swapped between
//! validation and transmission, so accepted files are copied into
//! daemon-owned storage first and the backend streams the copy.
//! Staged directories are deleted on every terminal share result,
//! on cancellation, and (as a backstop for crashes) swept at startup.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use handover_ipc::runtime_directory;

/// Upper bound for one staged send. Mirrors the native per-file cap.
pub(crate) const MAX_STAGED_SEND_BYTES: u64 = 100 * 1024 * 1024;

fn staging_root() -> io::Result<PathBuf> {
    let directory = runtime_directory()
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "runtime directory unavailable"))?
        .join("send-staging");
    std::fs::create_dir_all(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(directory)
}

fn random_suffix() -> String {
    // Unpredictable subdirectory names: no guessable temp paths.
    let mut bytes = [0u8; 16];
    if getrandom_fallback(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        bytes[..16].copy_from_slice(&nanos.to_ne_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn getrandom_fallback(buffer: &mut [u8]) -> io::Result<()> {
    let mut file = std::fs::File::open("/dev/urandom")?;
    use std::io::Read;
    file.read_exact(buffer)
}

/// Copy an accepted file into private staging, preserving its
/// basename inside a unique subdirectory so the receiver still sees
/// the original name. Fails when the source is not a bounded regular
/// file. The source is opened once with O_NOFOLLOW and streamed from
/// that descriptor: a path re-open after validation could resolve to
/// a swapped file.
pub(crate) fn stage_send_file(source: &Path) -> io::Result<PathBuf> {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != ".." && !name.contains('/'))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe file name"))?;
    let mut input = open_nofollow_read(source)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_STAGED_SEND_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "file is not usable",
        ));
    }
    let root = staging_root()?;
    let directory = root.join(random_suffix());
    std::fs::create_dir(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    }
    let staged = directory.join(name);
    {
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            output.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let copied = std::io::copy(&mut input, &mut output)?;
        if copied != metadata.len() {
            drop(output);
            let _ = std::fs::remove_dir_all(&directory);
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file changed during staging",
            ));
        }
    }
    Ok(staged)
}

/// Open a source file for reading without following a trailing
/// symlink. Follows the same Linux O_NOFOLLOW precedent as the
/// staging crate.
fn open_nofollow_read(path: &Path) -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(target_os = "linux")]
        options.custom_flags(0o400000);
        options.open(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::File::open(path)
    }
}

fn staged_dirs() -> &'static Mutex<HashMap<String, PathBuf>> {
    static DIRS: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    DIRS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Remember which staged directory belongs to an accepted transfer.
pub(crate) fn note_accepted(transfer_id: &str, staged: &Path) {
    if let Some(directory) = staged.parent().map(Path::to_path_buf) {
        if let Ok(mut dirs) = staged_dirs().lock() {
            dirs.insert(transfer_id.to_string(), directory);
        }
    }
}

/// Delete the staged directory for a transfer that reached a terminal
/// state (completed, failed, cancelled, disconnected).
pub(crate) fn release(transfer_id: &str) {
    let directory = staged_dirs()
        .lock()
        .map(|mut dirs| dirs.remove(transfer_id))
        .unwrap_or(None);
    if let Some(directory) = directory {
        let _ = std::fs::remove_dir_all(directory);
    }
}

/// Delete staged directories left by crashed runs. Best effort.
pub(crate) fn sweep_startup() -> usize {
    let Ok(root) = staging_root() else {
        return 0;
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir())
            && std::fs::remove_dir_all(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
}
