//! Recent observations, retained by the daemon for its current session.
use crate::{ReceivedShare, ShareProgress, ShareResult};
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
    Progress { progress: ShareProgress },
    Result { result: ShareResult },
}
