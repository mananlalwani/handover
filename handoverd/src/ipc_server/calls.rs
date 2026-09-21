use super::*;

pub(crate) fn route_call(
    state: &Arc<RwLock<StateStore>>,
    device_id: DeviceId,
    action: handover_core::CallAction,
    address: Option<String>,
) -> ServerPayload {
    let current = snapshot(state);
    let call = current
        .calls
        .iter()
        .find(|call| call.device_id == device_id);
    let allowed = current
        .devices
        .iter()
        .any(|d| d.id == device_id && d.connected && d.paired)
        && call.is_some_and(|call| call.controls.contains(&action))
        && match action {
            handover_core::CallAction::Place => address
                .as_deref()
                .is_some_and(handover_core::valid_call_address),
            _ => address.is_none(),
        };
    let generation = call.map(|call| call.generation);
    if !allowed {
        return ServerPayload::Error {
            code: ErrorCode::BackendRejected,
            message: "call action unavailable in current phone state or invalid address".into(),
        };
    }
    let result = device_id.as_str().strip_prefix("native:").and_then(|id| {
        native_backend().map(|native| native.call_control(id, action.as_str(), address, generation))
    });
    match result {
        Some(Ok(request_id)) => ServerPayload::CallQueued { request_id },
        Some(Err(_)) => ServerPayload::Error {
            code: ErrorCode::BackendRejected,
            message: "call command could not be queued".into(),
        },
        None => ServerPayload::Error {
            code: ErrorCode::BackendUnavailable,
            message: "call backend unavailable".into(),
        },
    }
}
