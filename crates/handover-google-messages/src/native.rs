//! Bounded Chrome Native Messaging transport for authentication and local login.

use std::{future::Future, io, time::Duration};

use base64::{Engine, engine::general_purpose};
use handover_core::messaging::MessagingAccountId;
use handover_ipc::Client as IpcClient;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

use crate::{
    BrowserProof, ProbeError, ProbeResult, RpcReason, RpcStatus, probe, register_device,
    registration::UnpairedRegistration, session_store::SessionStore,
};

const MAX_REQUEST_BYTES: usize = 32 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024;
// A 2-KiB description can expand sixfold when JSON escapes control bytes.
const MAX_LOCAL_RESPONSE_BYTES: usize = 16 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Serialize)]
#[serde(untagged)]
enum HostResponse {
    Success {
        ok: bool,
        sources: usize,
    },
    Failure {
        ok: bool,
        error: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        http_status: Option<u16>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rpc_status: Option<RpcStatus>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rpc_reason: Option<RpcReason>,
        #[serde(skip_serializing_if = "Option::is_none")]
        local_description: Option<String>,
    },
}

/// Handle one native messaging request and write one bounded response.
///
/// `caller_origin` is supplied by Chrome as the host's positional argument.
/// Origin checks happen before any request bytes are read.
pub async fn run<R, W>(
    reader: R,
    writer: W,
    allowed_extension_id: &str,
    caller_origin: &str,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    run_with_probe(
        reader,
        writer,
        allowed_extension_id,
        caller_origin,
        READ_TIMEOUT,
        PROBE_TIMEOUT,
        |proof| async move {
            if matches!(proof.kind.as_str(), "gaia_login" | "gaia_pairing_start") {
                login_via_daemon(proof).await
            } else if proof.kind == "gaia_register" {
                let registration =
                    register_device(&proof, Duration::from_secs(30 * 24 * 60 * 60)).await?;
                persist_registration(registration)?;
                Ok(ProbeResult { sources: 0 })
            } else {
                probe(proof).await
            }
        },
    )
    .await
}

/// One local stdin/stdout authentication check. No registration, persistence,
/// pairing, or daemon login is allowed through this entry point.
pub async fn run_local_read_only<R, W>(reader: R, writer: W) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    run_framed_probe(
        reader,
        writer,
        true,
        READ_TIMEOUT,
        PROBE_TIMEOUT,
        |proof| async move {
            if proof.kind != "gaia_lookup_with_cookies" {
                return Err(ProbeError::InvalidBootstrap);
            }
            probe(proof).await
        },
    )
    .await
}

async fn login_via_daemon(proof: BrowserProof) -> Result<ProbeResult, ProbeError> {
    let store = SessionStore::default_store().map_err(|_| ProbeError::RegistrationFailed)?;
    let (account_id, bundle_b64) = build_login_bundle(&proof, &store)?;
    let mut client = IpcClient::connect()
        .await
        .map_err(|_| ProbeError::DaemonUnavailable)?;
    client
        .messaging_login(account_id, bundle_b64)
        .await
        .map_err(|_| ProbeError::DaemonUnavailable)?;
    Ok(ProbeResult { sources: 0 })
}

fn build_login_bundle(
    proof: &BrowserProof,
    store: &SessionStore,
) -> Result<(MessagingAccountId, String), ProbeError> {
    proof.validate_login()?;
    let mut registrations = UnpairedRegistration::restore_all_pending(store)
        .map_err(|_| ProbeError::RegistrationFailed)?;
    if registrations.is_empty() {
        return Err(ProbeError::NoPendingRegistration);
    }
    if registrations.len() != 1 {
        return Err(ProbeError::AmbiguousRegistration);
    }
    let registration = registrations.pop().expect("one registration was checked");
    let account_id = MessagingAccountId::new(registration.handover_account_id().to_owned());
    let serialized = Zeroizing::new({
        let mut bytes = b"HOVL\x01\0".to_vec();
        bytes.extend_from_slice(
            &serde_json::to_vec(proof).map_err(|_| ProbeError::InvalidBootstrap)?,
        );
        bytes
    });
    let bundle_b64 = general_purpose::STANDARD.encode(serialized.as_slice());
    if bundle_b64.len() > handover_gmessages::contract::MAX_BUNDLE_BYTES {
        return Err(ProbeError::InvalidBootstrap);
    }
    Ok((account_id, bundle_b64))
}

fn persist_registration(registration: UnpairedRegistration) -> Result<(), ProbeError> {
    let store = SessionStore::default_store().map_err(|_| ProbeError::RegistrationFailed)?;
    registration
        .persist_pending(&store)
        .map_err(|_| ProbeError::RegistrationFailed)
}

async fn run_with_probe<R, W, F, Fut>(
    reader: R,
    writer: W,
    allowed_extension_id: &str,
    caller_origin: &str,
    read_timeout: Duration,
    probe_timeout: Duration,
    probe_fn: F,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    F: FnOnce(BrowserProof) -> Fut,
    Fut: Future<Output = Result<ProbeResult, ProbeError>>,
{
    let valid_origin = valid_extension_id(allowed_extension_id)
        && caller_origin == format!("chrome-extension://{allowed_extension_id}/");
    run_framed_probe(
        reader,
        writer,
        valid_origin,
        read_timeout,
        probe_timeout,
        probe_fn,
    )
    .await
}

async fn run_framed_probe<R, W, F, Fut>(
    mut reader: R,
    mut writer: W,
    valid_origin: bool,
    read_timeout: Duration,
    probe_timeout: Duration,
    probe_fn: F,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    F: FnOnce(BrowserProof) -> Fut,
    Fut: Future<Output = Result<ProbeResult, ProbeError>>,
{
    let response = if !valid_origin {
        failure("invalid_origin", None)
    } else {
        match tokio::time::timeout(read_timeout, read_proof(&mut reader)).await {
            Err(_) => failure("timeout", None),
            Ok(Err(code)) => failure(code, None),
            Ok(Ok(proof)) => match tokio::time::timeout(probe_timeout, probe_fn(proof)).await {
                Err(_) => failure("timeout", None),
                Ok(Err(error)) => HostResponse::Failure {
                    ok: false,
                    error: error.code(),
                    http_status: error.http_status(),
                    rpc_status: error.rpc_status(),
                    rpc_reason: error.rpc_reason(),
                    local_description: error.local_description().map(str::to_owned),
                },
                Ok(Ok(result)) => HostResponse::Success {
                    ok: true,
                    sources: result.sources,
                },
            },
        }
    };
    write_response(&mut writer, response).await
}

pub(crate) async fn read_proof<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<BrowserProof, &'static str> {
    let mut length = [0_u8; 4];
    reader
        .read_exact(&mut length)
        .await
        .map_err(|_| "invalid_frame")?;
    let length = u32::from_ne_bytes(length) as usize;
    if length > MAX_REQUEST_BYTES {
        return Err("invalid_frame");
    }
    let mut payload = Zeroizing::new(vec![0; length]);
    reader
        .read_exact(&mut payload)
        .await
        .map_err(|_| "invalid_frame")?;
    serde_json::from_slice(&payload).map_err(|_| "invalid_bootstrap")
}

/// Local setup progress is public ceremony/status text only, never proof bytes.
pub async fn run_local_setup<R, W>(mut reader: R, mut writer: W) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let proof = tokio::time::timeout(READ_TIMEOUT, read_proof(&mut reader)).await;
    let proof = match proof {
        Ok(Ok(proof)) => proof,
        _ => {
            return write_setup_event(
                &mut writer,
                serde_json::json!({"status":"failed","message":"Invalid setup proof."}),
            )
            .await;
        }
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let work = crate::setup::connect(proof, move |progress| {
        tx.try_send(progress).map_err(|_| ProbeError::NativeError)
    });
    tokio::pin!(work);
    let result = loop {
        tokio::select! {
            result = &mut work => break result,
            Some(progress) = rx.recv() => write_setup_progress(&mut writer, progress).await?,
        }
    };
    while let Ok(progress) = rx.try_recv() {
        write_setup_progress(&mut writer, progress).await?;
    }
    let event = match result {
        Ok(account) => serde_json::json!({"status":"saved","account":account,
            "message":"Account credentials saved. Waiting for Handover to reconnect."}),
        Err(error) => serde_json::json!({"status":"failed","error":error.code(),
            "message":"Setup did not complete. Check phone pairing state before trying again."}),
    };
    write_setup_event(&mut writer, event).await
}

async fn write_setup_progress<W: AsyncWrite + Unpin>(
    writer: &mut W,
    progress: crate::login::LoginProgress,
) -> io::Result<()> {
    use crate::login::LoginProgress;
    let message = match progress {
        LoginProgress::Ready => "Account registration verified.".to_owned(),
        LoginProgress::RegistrationVerified => {
            "Account verified. Starting phone pairing.".to_owned()
        }
        LoginProgress::InitialSendAccepted | LoginProgress::InitialAcknowledgementAccepted => {
            "Waiting for the phone pairing handshake.".to_owned()
        }
        LoginProgress::FinalSendAccepted => "Waiting for confirmation on your phone.".to_owned(),
        LoginProgress::Verification(symbol) => format!("Confirm {symbol} on your phone."),
    };
    write_setup_event(
        writer,
        serde_json::json!({"status":"progress","message":message}),
    )
    .await
}

async fn write_setup_event<W: AsyncWrite + Unpin>(
    writer: &mut W,
    event: serde_json::Value,
) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(&event).map_err(io::Error::other)?;
    if bytes.len() > 1024 {
        return Err(io::Error::other("setup status exceeds bound"));
    }
    bytes.push(b'\n');
    tokio::time::timeout(WRITE_TIMEOUT, async {
        writer.write_all(&bytes).await?;
        writer.flush().await
    })
    .await
    .map_err(|_| io::Error::other("setup status write timeout"))?
}

async fn write_response<W: AsyncWrite + Unpin>(
    writer: &mut W,
    response: HostResponse,
) -> io::Result<()> {
    let limit = match &response {
        HostResponse::Failure {
            local_description: Some(_),
            ..
        } => MAX_LOCAL_RESPONSE_BYTES,
        _ => MAX_RESPONSE_BYTES,
    };
    let payload = serde_json::to_vec(&response)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "response_encoding_failed"))?;
    if payload.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "response_too_large",
        ));
    }
    let length = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "response_too_large"))?;
    tokio::time::timeout(WRITE_TIMEOUT, async {
        writer.write_all(&length.to_ne_bytes()).await?;
        writer.write_all(&payload).await?;
        writer.flush().await
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "response_timeout"))??;
    Ok(())
}

fn failure(code: &'static str, http_status: Option<u16>) -> HostResponse {
    HostResponse::Failure {
        ok: false,
        error: code,
        http_status,
        rpc_status: None,
        rpc_reason: None,
        local_description: None,
    }
}

fn valid_extension_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| (b'a'..=b'p').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    const EXTENSION_ID: &str = "abcdefghijklmnopabcdefghijklmnop";
    const ORIGIN: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop/";

    #[tokio::test]
    async fn local_read_only_rejects_every_side_effect_mode() {
        for kind in [
            "gaia_register",
            "gaia_login",
            "gaia_pairing_start",
            "gaia_pairing",
        ] {
            let mut proof = pairing_proof();
            proof.kind = kind.into();
            let payload = serde_json::to_vec(&proof).unwrap();
            let (host_reader, mut client_writer) = duplex(4096);
            let (host_writer, client_reader) = duplex(4096);
            client_writer.write_all(&frame(&payload)).await.unwrap();
            let task = tokio::spawn(run_local_read_only(host_reader, host_writer));
            let result = response(client_reader).await;
            task.await.unwrap().unwrap();
            assert_eq!(result["ok"], false);
            assert_eq!(result["error"], "invalid_bootstrap");
        }
    }

    fn pairing_proof() -> BrowserProof {
        BrowserProof {
            kind: "gaia_pairing".into(),
            endpoint: "https://instantmessaging-pa.googleapis.com".into(),
            origin: "https://messages.google.com".into(),
            authorization: "Bearer synthetic".into(),
            api_key: "synthetic".into(),
            auth_user: Some("0".into()),
            service_cookie: Some("SID=synthetic".into()),
            account_email: Some("person@example.test".into()),
            browser_request: None,
        }
    }

    fn proof_json(extra: &str) -> Vec<u8> {
        format!(
            "{{\"type\":\"gaia_lookup\",\"endpoint\":\"https://example.test/\",\
             \"origin\":\"https://messages.google.com\",\"authorization\":\"redacted\",\
             \"api_key\":\"redacted\",\"auth_user\":null{extra}}}"
        )
        .into_bytes()
    }

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut bytes = (payload.len() as u32).to_ne_bytes().to_vec();
        bytes.extend_from_slice(payload);
        bytes
    }

    async fn response(mut reader: impl AsyncRead + Unpin) -> Value {
        let mut length = [0; 4];
        reader.read_exact(&mut length).await.unwrap();
        let length = u32::from_ne_bytes(length) as usize;
        assert!(length <= MAX_LOCAL_RESPONSE_BYTES);
        let mut payload = vec![0; length];
        reader.read_exact(&mut payload).await.unwrap();
        serde_json::from_slice(&payload).unwrap()
    }

    async fn fake_probe(_: BrowserProof) -> Result<ProbeResult, ProbeError> {
        Ok(ProbeResult { sources: 2 })
    }

    async fn rejected_probe(_: BrowserProof) -> Result<ProbeResult, ProbeError> {
        Err(ProbeError::HttpErrorWithReason(
            400,
            RpcStatus::InvalidArgument,
            RpcReason::ApiKeyInvalid,
        ))
    }

    #[tokio::test]
    async fn accepts_native_endian_frame_and_returns_small_success() {
        let (host_reader, mut client_writer) = duplex(4096);
        let (host_writer, mut client_reader) = duplex(4096);
        client_writer
            .write_all(&frame(&proof_json("")))
            .await
            .unwrap();
        let task = tokio::spawn(run_with_probe(
            host_reader,
            host_writer,
            EXTENSION_ID,
            ORIGIN,
            Duration::from_secs(1),
            Duration::from_secs(1),
            fake_probe,
        ));
        assert_eq!(
            response(&mut client_reader).await,
            serde_json::json!({"ok":true,"sources":2})
        );
        task.await.unwrap().unwrap();
    }

    #[test]
    fn login_bundle_uses_the_persisted_alias_and_keeps_email_inside_the_opaque_bundle() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        let registration_response =
            br#"[[],"c3ludGhldGljLWlk",null,["c3ludGhldGljLXRva2Vu","3600000000"]]"#;
        let registration = crate::registration::RegistrationAttempt::prepare()
            .unwrap()
            .accept_response(registration_response, Duration::from_secs(3600))
            .unwrap();
        let account_id = registration.handover_account_id().to_owned();
        registration.persist_pending(&store).unwrap();

        let mut proof = pairing_proof();
        proof.kind = "gaia_login".into();
        let (selected_account, bundle_b64) = build_login_bundle(&proof, &store).unwrap();
        assert_eq!(selected_account.as_str(), account_id);
        assert!(!selected_account.as_str().contains('@'));
        assert!(
            bundle_b64
                .starts_with(handover_gmessages::contract::NATIVE_BROWSER_LOGIN_BUNDLE_PREFIX)
        );
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(bundle_b64)
            .unwrap();
        let forwarded: BrowserProof = serde_json::from_slice(&decoded[6..]).unwrap();
        assert_eq!(
            forwarded.account_email.as_deref(),
            Some("person@example.test")
        );
        assert_eq!(forwarded.kind, "gaia_login");
    }

    #[test]
    fn login_bundle_requires_exactly_one_pending_registration() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        let mut proof = pairing_proof();
        proof.kind = "gaia_login".into();
        assert!(matches!(
            build_login_bundle(&proof, &store),
            Err(ProbeError::NoPendingRegistration)
        ));

        for identity in ["first-id", "second-id"] {
            let response = serde_json::to_vec(&serde_json::json!([
                [],
                base64::engine::general_purpose::STANDARD.encode(identity),
                null,
                [
                    base64::engine::general_purpose::STANDARD.encode("synthetic-token"),
                    "3600000000"
                ]
            ]))
            .unwrap();
            crate::registration::RegistrationAttempt::prepare()
                .unwrap()
                .accept_response(&response, Duration::from_secs(3600))
                .unwrap()
                .persist_pending(&store)
                .unwrap();
        }
        assert!(matches!(
            build_login_bundle(&proof, &store),
            Err(ProbeError::AmbiguousRegistration)
        ));
    }

    #[tokio::test]
    async fn rejects_origin_and_extension_before_reading_request() {
        let (host_reader, _client_writer) = duplex(4096);
        let (host_writer, mut client_reader) = duplex(4096);
        run_with_probe(
            host_reader,
            host_writer,
            "bad-id",
            ORIGIN,
            Duration::from_millis(10),
            Duration::from_millis(10),
            fake_probe,
        )
        .await
        .unwrap();
        assert_eq!(
            response(&mut client_reader).await["error"],
            "invalid_origin"
        );

        let (host_reader, _client_writer) = duplex(4096);
        let (host_writer, mut client_reader) = duplex(4096);
        run_with_probe(
            host_reader,
            host_writer,
            EXTENSION_ID,
            "chrome-extension://wrong/",
            Duration::from_millis(10),
            Duration::from_millis(10),
            fake_probe,
        )
        .await
        .unwrap();
        assert_eq!(
            response(&mut client_reader).await["error"],
            "invalid_origin"
        );
    }

    #[tokio::test]
    async fn rejects_unknown_json_fields_and_oversized_frames() {
        for (payload, expected) in [
            (
                proof_json(",\"secret\":\"do-not-echo\""),
                "invalid_bootstrap",
            ),
            (vec![0; MAX_REQUEST_BYTES + 1], "invalid_frame"),
        ] {
            let (host_reader, mut client_writer) = duplex(MAX_REQUEST_BYTES + 16);
            let (host_writer, mut client_reader) = duplex(4096);
            client_writer.write_all(&frame(&payload)).await.unwrap();
            let task = tokio::spawn(run_with_probe(
                host_reader,
                host_writer,
                EXTENSION_ID,
                ORIGIN,
                Duration::from_secs(1),
                Duration::from_secs(1),
                fake_probe,
            ));
            let result = response(&mut client_reader).await;
            assert_eq!(result["error"], expected);
            assert!(!result.to_string().contains("do-not-echo"));
            task.await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn partial_frame_times_out_and_returns_fixed_error() {
        let (host_reader, mut client_writer) = duplex(4096);
        let (host_writer, mut client_reader) = duplex(4096);
        client_writer.write_all(&[12, 0]).await.unwrap();
        let task = tokio::spawn(run_with_probe(
            host_reader,
            host_writer,
            EXTENSION_ID,
            ORIGIN,
            Duration::from_millis(20),
            Duration::from_millis(20),
            fake_probe,
        ));
        assert_eq!(response(&mut client_reader).await["error"], "timeout");
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn emits_only_fixed_probe_error_and_status() {
        let (host_reader, mut client_writer) = duplex(4096);
        let (host_writer, mut client_reader) = duplex(4096);
        client_writer
            .write_all(&frame(&proof_json("")))
            .await
            .unwrap();
        let task = tokio::spawn(run_with_probe(
            host_reader,
            host_writer,
            EXTENSION_ID,
            ORIGIN,
            Duration::from_secs(1),
            Duration::from_secs(1),
            rejected_probe,
        ));
        assert_eq!(
            response(&mut client_reader).await,
            serde_json::json!({"ok":false,"error":"http_error","http_status":400,"rpc_status":"INVALID_ARGUMENT","rpc_reason":"API_KEY_INVALID"})
        );
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn local_inspection_can_frame_the_bounded_description() {
        let (host_reader, mut client_writer) = duplex(4096);
        let (host_writer, mut client_reader) = duplex(16 * 1024);
        client_writer
            .write_all(&frame(&proof_json("")))
            .await
            .unwrap();
        let task = tokio::spawn(run_with_probe(
            host_reader,
            host_writer,
            EXTENSION_ID,
            ORIGIN,
            Duration::from_secs(1),
            Duration::from_secs(1),
            |_| async {
                Err(ProbeError::HttpErrorForLocalInspection(
                    400,
                    Some(RpcStatus::InvalidArgument),
                    None,
                    crate::LocalDescription("\u{0001}".repeat(2048)),
                ))
            },
        ));
        let result = response(&mut client_reader).await;
        assert_eq!(result["local_description"].as_str().unwrap().len(), 2048);
        task.await.unwrap().unwrap();
    }
}
