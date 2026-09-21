use super::*;

pub(crate) fn native_command_response(
    result: Option<Result<(), handover_native::NativeCommandError>>,
    operation: &str,
) -> ServerPayload {
    match result {
        Some(Ok(())) => ServerPayload::NativeAccepted,
        Some(Err(error)) => ServerPayload::Error {
            code: match error {
                handover_native::NativeCommandError::Offline => ErrorCode::DeviceDisconnected,
                handover_native::NativeCommandError::QueueFull => ErrorCode::BackendRejected,
            },
            message: format!("{operation} was not accepted"),
        },
        None => ServerPayload::Error {
            code: ErrorCode::BackendUnavailable,
            message: "native backend unavailable".into(),
        },
    }
}
