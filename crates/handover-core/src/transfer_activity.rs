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
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub kind: FileDownloadKind,
    #[serde(default)]
    pub progress: DownloadProgress,
    #[serde(default)]
    pub controls: std::collections::BTreeSet<DownloadControl>,
    pub state: FileDownloadState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FileDownloadState {
    Queued,
    Preparing {
        processed_bytes: u64,
        total_bytes: Option<u64>,
    },
    Receiving,
    Verifying,
    Paused {
        reason: FileDownloadFailure,
    },
    Cancelled,
    Completed {
        path: String,
    },
    Failed {
        reason: FileDownloadFailure,
    },
}
impl FileDownloadState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Failed { .. } | Self::Cancelled
        )
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
    SourceChanged,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileDownloadKind {
    #[default]
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadControl {
    Cancel,
    Resume,
}
impl FileDownloadKind {
    pub fn is_file(&self) -> bool {
        *self == Self::File
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DownloadProgress {
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub completed_files: u32,
    pub total_files: Option<u32>,
}
