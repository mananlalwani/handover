//! Bounded requested downloads. Chunk framing keeps the control connection usable.
//! Partial files and checkpoints stay private until their whole-file hashes verify.
use crate::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use handover_core::{
    DownloadControl, DownloadProgress, FileDownload, FileDownloadFailure as Failure,
    FileDownloadKind, FileDownloadState as Status, FilesystemEntry,
};
use sha2::{Digest, Sha256};
use std::io::{Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;

const MAX_FILE: u64 = 8 * 1024 * 1024 * 1024;
const MAX_PARTIALS: u64 = 16 * 1024 * 1024 * 1024;
const MAX_ITEMS: usize = 10_000;
const CHUNK: u64 = 32 * 1024;
const WINDOW: usize = 4;
const TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) struct DownloadListing {
    pub entries: Vec<FilesystemEntry>,
    pub cursor: Option<String>,
    pub failure: Option<Failure>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Item {
    source: String,
    relative: String,
    size: Option<u64>,
    version: Option<String>,
    received: u64,
    complete: bool,
}
#[derive(Clone, Deserialize, Serialize)]
struct Checkpoint {
    download: FileDownload,
    peer: String,
    created: u64,
    items: Vec<Item>,
    folders: VecDeque<String>,
    seen: BTreeSet<String>,
    manifest_done: bool,
    #[serde(default)]
    cursors: BTreeSet<String>,
}
#[derive(Clone)]
enum Purpose {
    Listing { path: String },
    Info,
    Data,
    Verify,
}
struct Job {
    saved: Checkpoint,
    wire: Option<(String, Purpose)>,
    fd: Option<File>,
    outstanding: BTreeMap<u64, u32>,
    buffered: BTreeMap<u64, Vec<u8>>,
    last_activity: Instant,
    last_checkpoint: Instant,
    last_event: Instant,
}
impl Job {
    fn current(&self) -> Option<usize> {
        self.saved.items.iter().position(|item| !item.complete)
    }
    fn active(&self) -> bool {
        self.wire.is_some()
    }
    fn record(&mut self, state: Status) -> FileDownload {
        let before = self.saved.download.clone();
        self.saved.download.state = state;
        self.saved.download.progress = DownloadProgress {
            received_bytes: self.saved.items.iter().map(|i| i.received).sum(),
            total_bytes: if self.saved.manifest_done {
                self.saved
                    .items
                    .iter()
                    .try_fold(0u64, |sum, i| sum.checked_add(i.size?))
            } else {
                None
            },
            completed_files: self.saved.items.iter().filter(|i| i.complete).count() as u32,
            total_files: self
                .saved
                .manifest_done
                .then_some(self.saved.items.len() as u32),
        };
        self.saved.download.controls = match self.saved.download.state {
            Status::Paused { .. } => {
                BTreeSet::from([DownloadControl::Cancel, DownloadControl::Resume])
            }
            Status::Queued | Status::Preparing { .. } | Status::Receiving | Status::Verifying => {
                BTreeSet::from([DownloadControl::Cancel])
            }
            _ => BTreeSet::new(),
        };
        if self.saved.download != before {
            self.saved.download.revision = before.revision.saturating_add(1);
        }
        self.saved.download.clone()
    }
}
#[derive(Default)]
pub(crate) struct DownloadManager {
    root: Option<PathBuf>,
    jobs: BTreeMap<String, Job>,
    online: BTreeSet<String>,
}
#[derive(Default)]
struct Actions {
    messages: Vec<(String, Message)>,
    events: Vec<FileDownload>,
    received: Vec<handover_core::ReceivedShare>,
    verify: Vec<(String, String, usize, PathBuf, String)>,
}
fn valid_path(path: &str) -> bool {
    path != "."
        && safe_browse_path(path)
        && path
            .split('/')
            .all(|p| safe_share_name(p) && p.len() <= 255)
}
fn version_valid(version: &str) -> bool {
    version.len() == 64
        && version
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn private_dir(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
            return Err(std::io::Error::other("unsafe download directory"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(path)?,
        Err(error) => return Err(error),
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}
impl DownloadManager {
    fn stage(&self, id: &str) -> PathBuf {
        self.root
            .as_ref()
            .unwrap()
            .join(".partial-downloads")
            .join(id)
    }
    fn payload(&self, id: &str, relative: &str) -> Result<PathBuf, NativeError> {
        if !valid_path(relative) {
            return Err(NativeError::InvalidFrame);
        }
        let mut path = self.stage(id).join("payload");
        let parts: Vec<_> = relative.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            path.push(part);
            private_dir(&path)?;
        }
        Ok(path.join(parts.last().unwrap()))
    }
    fn save(&mut self, id: &str) -> Result<(), NativeError> {
        let stage = self.stage(id);
        let job = self.jobs.get_mut(id).ok_or(NativeError::InvalidFrame)?;
        if let Some(fd) = &job.fd {
            fd.sync_data()?;
        }
        let data = serde_json::to_vec(&job.saved)?;
        if data.len() > 8 * 1024 * 1024 {
            return Err(NativeError::InvalidFrame);
        }
        write_private(&stage.join("checkpoint.tmp"), &data)?;
        File::open(stage.join("checkpoint.tmp"))?.sync_all()?;
        fs::rename(stage.join("checkpoint.tmp"), stage.join("checkpoint.json"))?;
        File::open(&stage)?.sync_all()?;
        job.last_checkpoint = Instant::now();
        Ok(())
    }
    fn initialize(&mut self, root: PathBuf, peers: &BTreeSet<String>) -> Result<(), NativeError> {
        if self.root.is_some() {
            return Ok(());
        }
        fs::create_dir_all(&root)?;
        let root = fs::canonicalize(root)?;
        private_dir(&root)?;
        let partials = root.join(".partial-downloads");
        private_dir(&partials)?;
        self.root = Some(root);
        for entry in fs::read_dir(partials)?.take(128) {
            if self.jobs.len() >= 32 {
                break;
            }
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if !valid_transfer_id(&id) || !entry.file_type()?.is_dir() {
                continue;
            }
            let parsed = (|| -> Result<Checkpoint, NativeError> {
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(entry.path().join("checkpoint.json"))?;
                let mut bytes = Vec::new();
                file.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err(NativeError::InvalidFrame);
                }
                Ok(serde_json::from_slice(&bytes)?)
            })();
            let Ok(saved) = parsed else {
                continue;
            };
            if saved.download.state.is_terminal() {
                let _ = fs::remove_dir_all(entry.path());
                continue;
            }

            if saved.download.transfer_id != id
                || !peers.contains(&saved.peer)
                || saved.download.device_id != DeviceId::new(format!("native:{}", saved.peer))
                || !valid_path(&saved.download.source_path)
                || saved.items.len() > MAX_ITEMS
                || saved.folders.len() > MAX_ITEMS
                || saved.seen.len() > MAX_ITEMS
                || saved.items.iter().any(|i| {
                    !valid_path(&i.source)
                        || !valid_path(&i.relative)
                        || i.size.is_some_and(|n| n > MAX_FILE || i.received > n)
                        || i.version.as_ref().is_some_and(|v| !version_valid(v))
                })
                || saved.folders.iter().any(|path| !valid_path(path))
            {
                continue;
            }
            let mut job = Job {
                saved,
                wire: None,
                fd: None,
                outstanding: BTreeMap::new(),
                buffered: BTreeMap::new(),
                last_activity: Instant::now(),
                last_checkpoint: Instant::now(),
                last_event: Instant::now(),
            };
            job.record(Status::Paused {
                reason: Failure::Disconnected,
            });
            self.jobs.insert(id, job);
        }
        Ok(())
    }
    fn pause(&mut self, id: &str, reason: Failure, actions: &mut Actions) {
        let Some(job) = self.jobs.get_mut(id) else {
            return;
        };
        if job.saved.download.state.is_terminal() {
            return;
        }
        if let Some((wire, _)) = job.wire.take() {
            actions.messages.push((
                job.saved.peer.clone(),
                Message::FileCloseV2 {
                    protocol: WIRE_VERSION,
                    transfer_id: wire,
                },
            ));
        }
        job.outstanding.clear();
        job.buffered.clear();
        let record = job.record(Status::Paused { reason });
        let saved = self.save(id).is_ok();
        let job = self.jobs.get_mut(id).unwrap();
        job.fd.take();
        if saved {
            actions.events.push(record);
        } else {
            actions.events.push(job.record(Status::Paused {
                reason: Failure::Storage,
            }));
        }
    }
    fn fail(&mut self, id: &str, reason: Failure, actions: &mut Actions) {
        self.pause(id, reason, actions);
        if let Some(job) = self.jobs.get_mut(id) {
            actions.events.retain(|e| e.transfer_id != id);
            actions.events.push(job.record(Status::Failed { reason }));
        }
        let _ = fs::remove_dir_all(self.stage(id));
    }
    fn pump(&mut self, actions: &mut Actions) -> Result<(), NativeError> {
        while self.jobs.values().filter(|j| j.active()).count() < 2 {
            let id = self
                .jobs
                .iter()
                .filter(|(_, j)| {
                    matches!(j.saved.download.state, Status::Queued)
                        && j.wire.is_none()
                        && self.online.contains(&j.saved.peer)
                })
                .min_by_key(|(_, j)| j.saved.created)
                .map(|(id, _)| id.clone());
            let Some(id) = id else {
                break;
            };
            if self.start(&id, actions).is_err() {
                self.pause(&id, Failure::Storage, actions);
            }
        }
        Ok(())
    }
    fn start(&mut self, id: &str, actions: &mut Actions) -> Result<(), NativeError> {
        let wire = new_transfer_id()?;
        let job = self.jobs.get_mut(id).ok_or(NativeError::InvalidFrame)?;
        job.last_activity = Instant::now();
        if !job.saved.manifest_done {
            if let Some(path) = job.saved.folders.pop_front() {
                job.wire = Some((wire.clone(), Purpose::Listing { path: path.clone() }));
                actions.messages.push((
                    job.saved.peer.clone(),
                    Message::FilesystemList {
                        protocol: WIRE_VERSION,
                        request_id: wire,
                        path,
                        paged: true,
                        cursor: None,
                    },
                ));
                return Ok(());
            }
            job.saved.manifest_done = true;
        }
        if let Some(index) = job.current() {
            let item = &job.saved.items[index];
            job.wire = Some((wire.clone(), Purpose::Info));
            actions.messages.push((
                job.saved.peer.clone(),
                Message::FileOpenV2 {
                    protocol: WIRE_VERSION,
                    transfer_id: wire,
                    path: item.source.clone(),
                    expected_version: item.version.clone(),
                },
            ));
        } else {
            job.wire = Some((wire.clone(), Purpose::Verify));
            actions.events.push(job.record(Status::Verifying));
            // An empty directory has no file hash to compute.
            actions.verify.push((
                id.to_owned(),
                wire,
                usize::MAX,
                PathBuf::new(),
                String::new(),
            ));
        }
        self.save(id)?;
        Ok(())
    }
    fn reads(&mut self, id: &str, actions: &mut Actions) -> Result<(), NativeError> {
        let job = self.jobs.get_mut(id).ok_or(NativeError::InvalidFrame)?;
        let index = job.current().ok_or(NativeError::InvalidFrame)?;
        let item = &job.saved.items[index];
        let total = item.size.ok_or(NativeError::InvalidFrame)?;
        let wire = job
            .wire
            .as_ref()
            .ok_or(NativeError::InvalidFrame)?
            .0
            .clone();
        let mut offset = item.received
            + job.buffered.values().map(|v| v.len() as u64).sum::<u64>()
            + job.outstanding.values().map(|n| u64::from(*n)).sum::<u64>();
        while job.outstanding.len() + job.buffered.len() < WINDOW && offset < total {
            let size = CHUNK.min(total - offset) as u32;
            job.outstanding.insert(offset, size);
            actions.messages.push((
                job.saved.peer.clone(),
                Message::FileReadV2 {
                    protocol: WIRE_VERSION,
                    transfer_id: wire.clone(),
                    offset,
                    size,
                },
            ));
            offset += u64::from(size);
        }
        Ok(())
    }
    fn verify_current(&mut self, id: &str, actions: &mut Actions) -> Result<(), NativeError> {
        self.save(id)?;
        let job = self.jobs.get_mut(id).ok_or(NativeError::InvalidFrame)?;
        job.fd.take();
        let index = job.current().ok_or(NativeError::InvalidFrame)?;
        let item = job.saved.items[index].clone();
        let wire = job
            .wire
            .as_ref()
            .ok_or(NativeError::InvalidFrame)?
            .0
            .clone();
        job.wire = Some((wire.clone(), Purpose::Verify));
        actions.events.push(job.record(Status::Verifying));
        actions.messages.push((
            job.saved.peer.clone(),
            Message::FileCloseV2 {
                protocol: WIRE_VERSION,
                transfer_id: wire.clone(),
            },
        ));
        let path = self.payload(id, &item.relative)?;
        actions.verify.push((
            id.to_owned(),
            wire,
            index,
            path,
            item.version.ok_or(NativeError::InvalidFrame)?,
        ));
        Ok(())
    }
}

impl NativeBackend {
    fn prepare_download_manager(&self) -> Result<(), NativeError> {
        let peers = self
            .inner
            .lock()
            .unwrap()
            .peers
            .peers
            .keys()
            .cloned()
            .collect();
        let root = self
            .downloads_directory
            .clone()
            .unwrap_or_else(|| self.directory.join("received"));
        self.downloads_v2.lock().unwrap().initialize(root, &peers)
    }
    pub fn restored_downloads(&self) -> Vec<FileDownload> {
        if self.prepare_download_manager().is_err() {
            return Vec::new();
        }
        self.downloads_v2
            .lock()
            .unwrap()
            .jobs
            .values()
            .map(|j| j.saved.download.clone())
            .collect()
    }
    pub fn request_download(
        &self,
        peer: &str,
        path: String,
        kind: FileDownloadKind,
    ) -> Result<FileDownload, NativeCommandError> {
        if !valid_path(&path) {
            return Err(NativeCommandError::InvalidResource);
        }
        {
            let inner = self.inner.lock().unwrap();
            if !inner.active.contains_key(peer) {
                return Err(NativeCommandError::Offline);
            }
            if !inner.peers.peers.get(peer).is_some_and(|p| p.files_v2) {
                return Err(NativeCommandError::Unsupported);
            }
        }
        self.prepare_download_manager()
            .map_err(|_| NativeCommandError::QueueFull)?;
        let id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        let (record, actions) = {
            let mut manager = self.downloads_v2.lock().unwrap();
            if manager
                .jobs
                .values()
                .filter(|j| !j.saved.download.state.is_terminal())
                .count()
                >= 32
                || manager.jobs.values().any(|j| {
                    j.saved.peer == peer
                        && j.saved.download.source_path == path
                        && !j.saved.download.state.is_terminal()
                })
            {
                return Err(NativeCommandError::QueueFull);
            }
            if manager.jobs.len() >= 128 {
                manager
                    .jobs
                    .retain(|_, j| !j.saved.download.state.is_terminal());
            }
            let stage = manager.stage(&id);
            private_dir(&stage).map_err(|_| NativeCommandError::QueueFull)?;
            private_dir(&stage.join("payload")).map_err(|_| NativeCommandError::QueueFull)?;
            let basename = path.rsplit('/').next().unwrap().to_owned();
            if kind == FileDownloadKind::Directory {
                private_dir(&stage.join("payload").join(&basename))
                    .map_err(|_| NativeCommandError::QueueFull)?;
            }
            let record = FileDownload {
                device_id: DeviceId::new(format!("native:{peer}")),
                transfer_id: id.clone(),
                source_path: path.clone(),
                revision: 0,
                kind,
                progress: Default::default(),
                controls: BTreeSet::from([DownloadControl::Cancel]),
                state: Status::Queued,
            };
            let saved = Checkpoint {
                download: record.clone(),
                peer: peer.into(),
                created: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
                items: if kind == FileDownloadKind::File {
                    vec![Item {
                        source: path.clone(),
                        relative: basename,
                        size: None,
                        version: None,
                        received: 0,
                        complete: false,
                    }]
                } else {
                    Vec::new()
                },
                folders: if kind == FileDownloadKind::Directory {
                    VecDeque::from([path.clone()])
                } else {
                    VecDeque::new()
                },
                seen: BTreeSet::from([path]),
                manifest_done: kind == FileDownloadKind::File,
                cursors: BTreeSet::new(),
            };
            manager.jobs.insert(
                id.clone(),
                Job {
                    saved,
                    wire: None,
                    fd: None,
                    outstanding: BTreeMap::new(),
                    buffered: BTreeMap::new(),
                    last_activity: Instant::now(),
                    last_checkpoint: Instant::now(),
                    last_event: Instant::now(),
                },
            );
            manager.online.insert(peer.into());
            if manager.save(&id).is_err() {
                manager.jobs.remove(&id);
                let _ = fs::remove_dir_all(&stage);
                return Err(NativeCommandError::QueueFull);
            }
            let mut actions = Actions::default();
            manager
                .pump(&mut actions)
                .map_err(|_| NativeCommandError::QueueFull)?;
            (manager.jobs[&id].saved.download.clone(), actions)
        };
        if let Some(event) = self.download_events.get() {
            self.apply_download_actions(actions, event);
        } else {
            for (target, message) in actions.messages {
                self.queue_simple(&target, message)?;
            }
        }
        Ok(record)
    }
    pub fn control_download(
        &self,
        peer: &str,
        id: &str,
        control: DownloadControl,
    ) -> Result<FileDownload, NativeCommandError> {
        self.prepare_download_manager()
            .map_err(|_| NativeCommandError::QueueFull)?;
        if control == DownloadControl::Resume {
            let inner = self.inner.lock().unwrap();
            if !inner.active.contains_key(peer) {
                return Err(NativeCommandError::Offline);
            }
            if !inner.peers.peers.get(peer).is_some_and(|p| p.files_v2) {
                return Err(NativeCommandError::Unsupported);
            }
        }
        let (record, actions) = {
            let mut manager = self.downloads_v2.lock().unwrap();
            let payload = manager.stage(id).join("payload");
            let job = manager
                .jobs
                .get_mut(id)
                .filter(|j| j.saved.peer == peer)
                .ok_or(NativeCommandError::InvalidResource)?;
            if !job.saved.download.controls.contains(&control) {
                return Err(NativeCommandError::InvalidResource);
            }
            let mut actions = Actions::default();
            match control {
                DownloadControl::Cancel => {
                    if let Some((wire, _)) = job.wire.take() {
                        actions.messages.push((
                            peer.into(),
                            Message::FileCloseV2 {
                                protocol: WIRE_VERSION,
                                transfer_id: wire,
                            },
                        ));
                    }
                    job.fd.take();
                    job.outstanding.clear();
                    job.buffered.clear();
                    job.record(Status::Cancelled);
                    if manager.save(id).is_err() {
                        manager.jobs.get_mut(id).unwrap().record(Status::Failed {
                            reason: Failure::Storage,
                        });
                    }
                    let _ = fs::remove_dir_all(manager.stage(id));
                }
                DownloadControl::Resume => {
                    for item in &mut job.saved.items {
                        item.complete = false;
                    }
                    if !job.saved.manifest_done {
                        // A fresh listing must not retain directories from an abandoned
                        // manifest. No file bytes are read until that manifest is complete.
                        fs::remove_dir_all(&payload).map_err(|_| NativeCommandError::QueueFull)?;
                        private_dir(&payload).map_err(|_| NativeCommandError::QueueFull)?;
                        let name = job.saved.download.source_path.rsplit('/').next().unwrap();
                        private_dir(&payload.join(name))
                            .map_err(|_| NativeCommandError::QueueFull)?;
                        job.saved.items.clear();
                        job.saved.cursors.clear();
                        job.saved.folders =
                            VecDeque::from([job.saved.download.source_path.clone()]);
                        job.saved.seen = BTreeSet::from([job.saved.download.source_path.clone()]);
                    }
                    job.record(Status::Queued);
                    manager.online.insert(peer.into());
                    manager
                        .pump(&mut actions)
                        .map_err(|_| NativeCommandError::QueueFull)?;
                }
            }
            let record = manager.jobs[id].saved.download.clone();
            (record, actions)
        };
        if let Some(event) = self.download_events.get() {
            self.apply_download_actions(actions, event);
        } else {
            for (target, message) in actions.messages {
                let _ = self.queue_simple(&target, message);
            }
        }
        Ok(record)
    }
    pub(crate) fn forget_downloads_for_peer(&self, peer: &str) {
        let mut manager = self.downloads_v2.lock().unwrap();
        let ids: Vec<_> = manager
            .jobs
            .iter()
            .filter(|(_, job)| job.saved.peer == peer)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            manager.jobs.remove(&id);
            let _ = fs::remove_dir_all(manager.stage(&id));
        }
        manager.online.remove(peer);
    }
    pub(crate) fn pause_downloads_for_peer(&self, peer: &str) -> Vec<FileDownload> {
        let mut manager = self.downloads_v2.lock().unwrap();
        manager.online.remove(peer);
        let ids: Vec<_> = manager
            .jobs
            .iter()
            .filter(|(_, j)| {
                j.saved.peer == peer
                    && !j.saved.download.state.is_terminal()
                    && !matches!(j.saved.download.state, Status::Paused { .. })
            })
            .map(|(id, _)| id.clone())
            .collect();
        let mut actions = Actions::default();
        for id in ids {
            manager.pause(&id, Failure::Disconnected, &mut actions);
        }
        actions.events
    }
    pub(crate) fn expire_downloads_v2(
        &self,
        peer: &str,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) {
        let mut actions = Actions::default();
        {
            let mut manager = self.downloads_v2.lock().unwrap();
            let ids: Vec<_> = manager
                .jobs
                .iter()
                .filter(|(_, j)| {
                    j.saved.peer == peer
                        && j.active()
                        && !matches!(j.wire, Some((_, Purpose::Verify)))
                        && j.last_activity.elapsed() >= TIMEOUT
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in ids {
                manager.pause(&id, Failure::TimedOut, &mut actions);
            }
            let _ = manager.pump(&mut actions);
        }
        self.apply_download_actions(actions, event);
    }
    pub fn filesystem_list_page(
        &self,
        peer: &str,
        path: String,
        cursor: Option<String>,
    ) -> Result<String, NativeCommandError> {
        if !safe_browse_path(&path)
            || cursor
                .as_ref()
                .is_some_and(|c| c.len() > 1024 || c.chars().any(char::is_control))
        {
            return Err(NativeCommandError::InvalidResource);
        }
        let paged = self
            .inner
            .lock()
            .unwrap()
            .peers
            .peers
            .get(peer)
            .is_some_and(|p| p.files_v2);
        if cursor.is_some() && !paged {
            return Err(NativeCommandError::Unsupported);
        }
        let id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        self.queue_simple(
            peer,
            Message::FilesystemList {
                protocol: WIRE_VERSION,
                request_id: id.clone(),
                path,
                paged,
                cursor,
            },
        )?;
        Ok(id)
    }
    pub(crate) fn owns_download_listing(&self, peer: &str, id: &str) -> bool {
        self.downloads_v2.lock().unwrap().jobs.values().any(|j| {
            j.saved.peer == peer && matches!(&j.wire,Some((wire,Purpose::Listing{..}))if wire==id)
        })
    }
    pub(crate) fn download_listing(
        &self,
        peer: &str,
        request: &str,
        path: &str,
        page: DownloadListing,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        let DownloadListing {
            entries,
            cursor,
            failure,
        } = page;
        let mut actions = Actions::default();
        {
            let mut manager = self.downloads_v2.lock().unwrap();
            let Some(id)=manager.jobs.iter().find(|(_,j)|j.saved.peer==peer&&matches!(&j.wire,Some((wire,Purpose::Listing{path:expected}))if wire==request&&expected==path)).map(|(id,_)|id.clone()) else{return Ok(());};
            if let Some(reason) = failure {
                manager.pause(&id, reason, &mut actions);
            } else {
                if entries
                    .len()
                    .saturating_add(manager.jobs[&id].saved.seen.len())
                    > MAX_ITEMS
                    || entries
                        .iter()
                        .any(|e| e.size.is_some_and(|size| size > MAX_FILE))
                {
                    manager.fail(&id, Failure::SizeLimit, &mut actions);
                    drop(manager);
                    self.apply_download_actions(actions, event);
                    return Ok(());
                }
                if cursor.as_ref().is_some_and(|c| {
                    c.is_empty() || c.len() > 1024 || c.chars().any(char::is_control)
                }) {
                    manager.fail(&id, Failure::InvalidResource, &mut actions);
                    drop(manager);
                    self.apply_download_actions(actions, event);
                    return Ok(());
                }
                if let Some(cursor) = &cursor
                    && !manager
                        .jobs
                        .get_mut(&id)
                        .unwrap()
                        .saved
                        .cursors
                        .insert(format!("{path}\0{cursor}"))
                {
                    manager.fail(&id, Failure::InvalidResource, &mut actions);
                    drop(manager);
                    self.apply_download_actions(actions, event);
                    return Ok(());
                }
                let root = manager.jobs[&id].saved.download.source_path.clone();
                let root_name = root.rsplit('/').next().unwrap();
                let mut dirs = Vec::new();
                let job = manager.jobs.get_mut(&id).unwrap();
                job.last_activity = Instant::now();
                for entry in entries {
                    if !safe_share_name(&entry.name) || entry.name.len() > 255 {
                        return Err(NativeError::InvalidFrame);
                    }
                    let source = format!("{path}/{}", entry.name);
                    if !valid_path(&source)
                        || !job.saved.seen.insert(source.clone())
                        || job.saved.seen.len() > MAX_ITEMS
                    {
                        return Err(NativeError::InvalidFrame);
                    }
                    let relative = format!(
                        "{root_name}/{}",
                        source
                            .strip_prefix(&format!("{root}/"))
                            .ok_or(NativeError::InvalidFrame)?
                    );
                    if entry.directory {
                        job.saved.folders.push_back(source);
                        dirs.push(relative);
                    } else {
                        job.saved.items.push(Item {
                            source,
                            relative,
                            size: entry.size,
                            version: None,
                            received: 0,
                            complete: false,
                        });
                    }
                }
                let next = cursor.filter(|c| {
                    !c.is_empty() && c.len() <= 1024 && !c.chars().any(char::is_control)
                });
                if let Some(cursor) = next {
                    let wire = new_transfer_id()?;
                    job.wire = Some((wire.clone(), Purpose::Listing { path: path.into() }));
                    actions.messages.push((
                        peer.into(),
                        Message::FilesystemList {
                            protocol: WIRE_VERSION,
                            request_id: wire,
                            path: path.into(),
                            paged: true,
                            cursor: Some(cursor),
                        },
                    ));
                } else {
                    job.wire = None;
                    manager.start(&id, &mut actions)?;
                }
                for relative in dirs {
                    let path = manager.payload(&id, &relative)?;
                    private_dir(&path)?;
                }
                manager.save(&id)?;
            }
        }
        self.apply_download_actions(actions, event);
        Ok(())
    }
    pub(crate) fn handle_download_v2(
        &self,
        peer: &str,
        message: Message,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        let wire = match &message {
            Message::FilePreparingV2 { transfer_id, .. }
            | Message::FileInfoV2 { transfer_id, .. }
            | Message::FileChunkV2 { transfer_id, .. }
            | Message::FileErrorV2 { transfer_id, .. } => transfer_id.clone(),
            _ => return Err(NativeError::InvalidFrame),
        };
        if !valid_transfer_id(&wire) {
            return Err(NativeError::InvalidFrame);
        }
        let mut actions = Actions::default();
        let outcome = (|| -> Result<(), NativeError> {
            let mut manager = self.downloads_v2.lock().unwrap();
            let Some(id) = manager
                .jobs
                .iter()
                .find(|(_, j)| {
                    j.saved.peer == peer && j.wire.as_ref().is_some_and(|(id, _)| id == &wire)
                })
                .map(|(id, _)| id.clone())
            else {
                return Ok(());
            };
            manager.jobs.get_mut(&id).unwrap().last_activity = Instant::now();
            match message {
                Message::FileErrorV2 { reason, .. } => {
                    if matches!(
                        reason,
                        Failure::SourceChanged | Failure::InvalidResource | Failure::SizeLimit
                    ) {
                        manager.fail(&id, reason, &mut actions);
                    } else {
                        manager.pause(&id, reason, &mut actions);
                    }
                    manager.pump(&mut actions)?;
                }
                Message::FilePreparingV2 {
                    processed_bytes,
                    total_bytes,
                    ..
                } => {
                    let job = manager.jobs.get_mut(&id).unwrap();
                    if !matches!(job.wire, Some((_, Purpose::Info)))
                        || processed_bytes > MAX_FILE
                        || total_bytes.is_some_and(|n| n > MAX_FILE || processed_bytes > n)
                    {
                        return Err(NativeError::InvalidFrame);
                    }
                    if let Status::Preparing {
                        processed_bytes: old,
                        ..
                    } = job.saved.download.state
                        && processed_bytes < old
                    {
                        return Err(NativeError::InvalidFrame);
                    }
                    actions.events.push(job.record(Status::Preparing {
                        processed_bytes,
                        total_bytes,
                    }));
                }
                Message::FileInfoV2 {
                    name,
                    size,
                    version,
                    ..
                } => {
                    let job = &manager.jobs[&id];
                    let index = job.current().ok_or(NativeError::InvalidFrame)?;
                    let item = job.saved.items[index].clone();
                    if !matches!(job.wire, Some((_, Purpose::Info)))
                        || name != item.source.rsplit('/').next().unwrap()
                        || !safe_share_name(&name)
                        || !version_valid(&version)
                    {
                        return Err(NativeError::InvalidFrame);
                    }
                    if size > MAX_FILE {
                        manager.fail(&id, Failure::SizeLimit, &mut actions);
                        return Ok(());
                    }
                    if item.version.as_ref().is_some_and(|v| v != &version)
                        || item
                            .size
                            .is_some_and(|s| item.version.is_some() && s != size)
                    {
                        manager.fail(&id, Failure::SourceChanged, &mut actions);
                        return Ok(());
                    }
                    let reserved: u64 = manager
                        .jobs
                        .values()
                        .filter(|j| !j.saved.download.state.is_terminal())
                        .flat_map(|j| j.saved.items.iter())
                        .filter_map(|i| i.size)
                        .fold(0u64, |sum, size| sum.saturating_add(size));
                    if reserved
                        .saturating_sub(item.size.unwrap_or(0))
                        .saturating_add(size)
                        > MAX_PARTIALS
                    {
                        manager.pause(&id, Failure::Storage, &mut actions);
                        return Ok(());
                    }
                    let path = manager.payload(&id, &item.relative)?;
                    let fd = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .mode(0o600)
                        .custom_flags(libc::O_NOFOLLOW)
                        .open(path)?;
                    if !fd.metadata()?.is_file() || fd.metadata()?.len() < item.received {
                        manager.fail(&id, Failure::SourceChanged, &mut actions);
                        return Ok(());
                    }
                    fd.set_len(item.received)?;
                    let job = manager.jobs.get_mut(&id).unwrap();
                    job.saved.items[index].size = Some(size);
                    job.saved.items[index].version = Some(version);
                    job.fd = Some(fd);
                    job.fd
                        .as_mut()
                        .unwrap()
                        .seek(SeekFrom::Start(item.received))?;
                    job.wire = Some((wire.clone(), Purpose::Data));
                    actions.events.push(job.record(Status::Receiving));
                    manager.save(&id)?;
                    if item.received == size {
                        manager.verify_current(&id, &mut actions)?;
                    } else {
                        manager.reads(&id, &mut actions)?;
                    }
                }
                Message::FileChunkV2 { offset, data, .. } => {
                    if data.len() > 48 * 1024 {
                        return Err(NativeError::InvalidFrame);
                    }
                    let bytes = STANDARD
                        .decode(data)
                        .map_err(|_| NativeError::InvalidFrame)?;
                    let job = manager.jobs.get_mut(&id).unwrap();
                    if !matches!(job.wire, Some((_, Purpose::Data))) {
                        return Ok(());
                    }
                    let expected = job
                        .outstanding
                        .remove(&offset)
                        .ok_or(NativeError::InvalidFrame)?;
                    if bytes.len() != expected as usize {
                        return Err(NativeError::InvalidFrame);
                    }
                    job.buffered.insert(offset, bytes);
                    let index = job.current().ok_or(NativeError::InvalidFrame)?;
                    while let Some(bytes) = job.buffered.remove(&job.saved.items[index].received) {
                        job.fd
                            .as_mut()
                            .ok_or(NativeError::InvalidFrame)?
                            .write_all(&bytes)?;
                        job.saved.items[index].received += bytes.len() as u64;
                    }
                    let complete =
                        job.saved.items[index].size == Some(job.saved.items[index].received);
                    let checkpoint = job.last_checkpoint.elapsed() >= Duration::from_secs(1);
                    if job.last_event.elapsed() >= Duration::from_millis(250)
                        || job.saved.download.progress.received_bytes == 0
                        || complete
                    {
                        job.last_event = Instant::now();
                        actions.events.push(job.record(Status::Receiving));
                    }
                    if complete {
                        manager.verify_current(&id, &mut actions)?;
                    } else {
                        if checkpoint {
                            manager.save(&id)?;
                        }
                        manager.reads(&id, &mut actions)?;
                    }
                }
                _ => return Err(NativeError::InvalidFrame),
            }
            Ok(())
        })();
        if let Err(error) = outcome {
            // Framing/protocol violations end the session; I/O failure pauses this download.
            if matches!(error, NativeError::Io(_) | NativeError::QuotaExceeded) {
                let mut manager = self.downloads_v2.lock().unwrap();
                let id = manager
                    .jobs
                    .iter()
                    .find(|(_, j)| {
                        j.saved.peer == peer && j.wire.as_ref().is_some_and(|(id, _)| id == &wire)
                    })
                    .map(|(id, _)| id.clone());
                if let Some(id) = id {
                    manager.pause(&id, Failure::Storage, &mut actions);
                }
            } else {
                return Err(error);
            }
        }
        self.apply_download_actions(actions, event);
        Ok(())
    }
    fn apply_download_actions(
        &self,
        actions: Actions,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) {
        for record in actions.events {
            event(StateEvent::FileDownload(record));
        }
        for share in actions.received {
            event(StateEvent::ShareReceived(share));
        }
        let mut blocked = BTreeSet::new();
        for (peer, message) in actions.messages {
            if !blocked.contains(&peer) && self.queue_simple(&peer, message).is_err() {
                blocked.insert(peer);
            }
        }
        // The command queue is bounded. A failed admission pauses affected jobs
        // immediately instead of leaving an accepted download waiting for a timeout.
        for peer in blocked {
            let records = {
                let mut manager = self.downloads_v2.lock().unwrap();
                let ids: Vec<_> = manager
                    .jobs
                    .iter()
                    .filter(|(_, j)| j.saved.peer == peer && j.active())
                    .map(|(id, _)| id.clone())
                    .collect();
                let mut paused = Actions::default();
                for id in ids {
                    manager.pause(&id, Failure::Unavailable, &mut paused);
                }
                paused.events
            };
            for record in records {
                event(StateEvent::FileDownload(record));
            }
        }
        for (id, wire, index, path, version) in actions.verify {
            let backend = self.clone();
            let event = event.clone();
            std::thread::spawn(move || {
                let result = (|| -> Result<(), Failure> {
                    if index != usize::MAX {
                        let mut file = OpenOptions::new()
                            .read(true)
                            .custom_flags(libc::O_NOFOLLOW)
                            .open(&path)
                            .map_err(|_| Failure::Storage)?;
                        let mut hash = Sha256::new();
                        let mut buffer = [0u8; 64 * 1024];
                        loop {
                            if !backend.downloads_v2.lock().unwrap().jobs.get(&id).is_some_and(|j| matches!(&j.wire,Some((current,Purpose::Verify)) if current==&wire)) { return Err(Failure::Interrupted); }
                            let count = file.read(&mut buffer).map_err(|_| Failure::Storage)?;
                            if count == 0 {
                                break;
                            }
                            hash.update(&buffer[..count]);
                        }
                        if hex::encode(hash.finalize()) != version {
                            return Err(Failure::SourceChanged);
                        }
                    }
                    Ok(())
                })();
                let mut actions = Actions::default();
                {
                    let mut manager = backend.downloads_v2.lock().unwrap();
                    if !manager.jobs.get(&id).is_some_and(
                        |j| matches!(&j.wire,Some((current,Purpose::Verify))if current==&wire),
                    ) {
                        return;
                    }
                    if let Err(reason) = result {
                        manager.fail(&id, reason, &mut actions);
                    } else {
                        if index != usize::MAX {
                            manager.jobs.get_mut(&id).unwrap().saved.items[index].complete = true;
                        }
                        let complete = manager.jobs[&id].current().is_none();
                        if complete {
                            let saved = manager.jobs[&id].saved.clone();
                            let name = saved.download.source_path.rsplit('/').next().unwrap();
                            let root = manager.root.as_ref().unwrap().clone();
                            let source = manager.stage(&id).join("payload").join(name);
                            let finalized = (|| -> Result<PathBuf, NativeError> {
                                let stamp =
                                    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
                                let destination =
                                    create_received_directory(&root, &stamp)?.join(name);
                                fs::rename(&source, &destination)?;
                                if let Err(error) =
                                    File::open(destination.parent().unwrap())?.sync_all()
                                {
                                    let _ = fs::rename(&destination, &source);
                                    return Err(error.into());
                                }
                                Ok(destination)
                            })();
                            match finalized {
                                Ok(path) => {
                                    let job = manager.jobs.get_mut(&id).unwrap();
                                    job.wire = None;
                                    let path = path.to_string_lossy().into_owned();
                                    actions
                                        .events
                                        .push(job.record(Status::Completed { path: path.clone() }));
                                    if job.saved.download.kind == FileDownloadKind::File {
                                        actions.received.push(handover_core::ReceivedShare {
                                            device_id: job.saved.download.device_id.clone(),
                                            resource: handover_core::SharedResource::File { path },
                                        });
                                    }
                                    let _ = fs::remove_dir_all(manager.stage(&id));
                                }
                                Err(_) => manager.pause(&id, Failure::Storage, &mut actions),
                            }
                        } else {
                            let job = manager.jobs.get_mut(&id).unwrap();
                            job.wire = None;
                            job.record(Status::Queued);
                            if manager.start(&id, &mut actions).is_err() {
                                manager.pause(&id, Failure::Storage, &mut actions);
                            }
                        }
                    }
                    let _ = manager.pump(&mut actions);
                }
                backend.apply_download_actions(actions, &event);
            });
        }
    }
}
