use super::*;

pub(crate) fn route_call(
    state: &Arc<RwLock<StateStore>>,
    device_id: DeviceId,
    action: handover_core::CallAction,
    address: Option<String>,
    expected_generation: Option<u64>,
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
    if !allowed
        || expected_generation
            .is_some_and(|generation| call.is_none_or(|c| c.generation != generation))
    {
        return ServerPayload::Error {
            code: ErrorCode::BackendRejected,
            message: "call action unavailable in current phone state or invalid address".into(),
            conversation_id: None,
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
            conversation_id: None,
        },
        None => ServerPayload::Error {
            code: ErrorCode::BackendUnavailable,
            message: "call backend unavailable".into(),
            conversation_id: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::{
        CallAction, CallEvent, CallPhase, CallState, Device, DeviceEvent, StateEvent,
    };
    use std::collections::BTreeSet;

    #[test]
    fn call_control_rejects_an_action_from_a_previous_call_generation() {
        let id = DeviceId::new("native:fixture");
        let mut store = StateStore::default();
        store.apply(StateEvent::Device(DeviceEvent::Added(Device {
            id: id.clone(),
            name: "Fixture".into(),
            connected: true,
            paired: true,
            battery: None,
            connectivity: None,
            capabilities: BTreeSet::new(),
        })));
        store.apply(StateEvent::Call(CallEvent::Updated(CallState {
            device_id: id.clone(),
            phase: CallPhase::Ringing,
            controls: BTreeSet::from([CallAction::Answer]),
            generation: 9,
        })));
        let state = Arc::new(RwLock::new(store));
        assert!(matches!(
            route_call(&state, id, CallAction::Answer, None, Some(8)),
            ServerPayload::Error {
                code: ErrorCode::BackendRejected,
                ..
            }
        ));
        assert_eq!(state.read().unwrap().snapshot().calls[0].generation, 9);
    }
}
