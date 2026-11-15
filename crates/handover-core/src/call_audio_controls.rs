use crate::DeviceId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CallAudioChoice {
    pub id: String,
    pub label: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CallAudioRoute {
    pub device_id: DeviceId,
    pub endpoint_id: String,
    pub input_id: String,
    pub output_id: String,
    pub state: CallAudioRouteState,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CallAudioRouteState {
    WaitingForCall,
    Routing { duplex_running: bool },
    Failed { message: String },
}
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CallAudioControls {
    pub revision: u64,
    pub observed: bool,
    pub endpoints: Vec<CallAudioChoice>,
    pub inputs: Vec<CallAudioChoice>,
    pub outputs: Vec<CallAudioChoice>,
    pub route: Option<CallAudioRoute>,
    pub notice: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CallAudioSelection {
    pub expected_revision: u64,
    pub device_id: DeviceId,
    pub endpoint_id: String,
    pub input_id: String,
    pub output_id: String,
}
