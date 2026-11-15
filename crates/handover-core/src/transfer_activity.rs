//! Recent observations, retained by the daemon for its current session.
use crate::{DeviceId, ReceivedShare, ShareProgress, ShareResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransferActivity {
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub event: TransferActivityEvent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransferActivityEvent {
    Received { share: ReceivedShare },
    Download { download: FileDownload },
    Progress { progress: ShareProgress },
    Result { result: ShareResult },
}

/// A requested incoming file. Completion is local storage evidence, never a remote receipt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FileDownload {
    pub device_id: DeviceId,
    pub transfer_id: String,
    pub source_path: String,
    pub state: FileDownloadState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FileDownloadState {
    Queued,
    Receiving,
    Completed { path: String },
    Failed { reason: FileDownloadFailure },
}
impl FileDownloadState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::Failed { .. })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileDownloadFailure {
    PermissionRequired,
    InvalidResource,
    SizeLimit,
    Unavailable,
    Storage,
    Interrupted,
    TimedOut,
    Disconnected,
}
