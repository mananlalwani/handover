//! Bounded Chrome Native Messaging transport for the authentication probe.

use std::{future::Future, io, time::Duration};

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{BrowserProof, ProbeError, ProbeResult, RpcStatus, probe};

const MAX_REQUEST_BYTES: usize = 32 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024;
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
        probe,
    )
    .await
}

async fn run_with_probe<R, W, F, Fut>(
    mut reader: R,
    mut writer: W,
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
    let response = if !valid_extension_id(allowed_extension_id)
        || caller_origin != format!("chrome-extension://{allowed_extension_id}/")
    {
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

async fn read_proof<R: AsyncRead + Unpin>(reader: &mut R) -> Result<BrowserProof, &'static str> {
    let mut length = [0_u8; 4];
    reader
        .read_exact(&mut length)
        .await
        .map_err(|_| "invalid_frame")?;
    let length = u32::from_ne_bytes(length) as usize;
    if length > MAX_REQUEST_BYTES {
        return Err("invalid_frame");
    }
    let mut payload = vec![0; length];
    reader
        .read_exact(&mut payload)
        .await
        .map_err(|_| "invalid_frame")?;
    serde_json::from_slice(&payload).map_err(|_| "invalid_bootstrap")
}

async fn write_response<W: AsyncWrite + Unpin>(
    writer: &mut W,
    response: HostResponse,
) -> io::Result<()> {
    let payload = serde_json::to_vec(&response)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "response_encoding_failed"))?;
    if payload.len() > MAX_RESPONSE_BYTES {
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
        assert!(length <= MAX_RESPONSE_BYTES);
        let mut payload = vec![0; length];
        reader.read_exact(&mut payload).await.unwrap();
        serde_json::from_slice(&payload).unwrap()
    }

    async fn fake_probe(_: BrowserProof) -> Result<ProbeResult, ProbeError> {
        Ok(ProbeResult { sources: 2 })
    }

    async fn rejected_probe(_: BrowserProof) -> Result<ProbeResult, ProbeError> {
        Err(ProbeError::HttpErrorWithStatus(
            400,
            RpcStatus::InvalidArgument,
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
            serde_json::json!({"ok":false,"error":"http_error","http_status":400,"rpc_status":"INVALID_ARGUMENT"})
        );
        task.await.unwrap().unwrap();
    }
}
