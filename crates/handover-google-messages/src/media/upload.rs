//! Independently observed Gaia Condenser upload flow. No retries or phone sends.
use super::{MAX_MEDIA_BYTES, MediaError, MediaIdentity, RequestHeader, encrypt, request_header};
use base64::{Engine, engine::general_purpose::STANDARD};
use prost::Message;
use rand::{RngCore, rngs::OsRng};
use std::{io::Read, os::unix::fs::OpenOptionsExt, path::Path};
use zeroize::Zeroizing;

const ENDPOINT: &str = "https://instantmessaging-pa.googleapis.com/upload";
const RESPONSE_LIMIT: usize = 64 * 1024;

pub(crate) struct Uploaded {
    pub blob: Zeroizing<String>,
    pub key: Zeroizing<[u8; 32]>,
    pub name: Zeroizing<String>,
    pub mime: &'static str,
    pub kind: i32,
    pub size: usize,
}
struct Prepared {
    ciphertext: Zeroizing<Vec<u8>>,
    key: Zeroizing<[u8; 32]>,
    name: Zeroizing<String>,
    mime: &'static str,
    kind: i32,
    size: usize,
}

#[derive(Message)]
#[prost(skip_debug)]
struct Metadata {
    #[prost(int32, tag = "1")]
    kind: i32,
    #[prost(message, optional, tag = "2")]
    header: Option<RequestHeader>,
}
#[derive(Message)]
#[prost(skip_debug)]
struct Response {
    #[prost(message, optional, tag = "1")]
    media: Option<MediaIdentity>,
}
pub(crate) fn metadata(token: &[u8]) -> Zeroizing<Vec<u8>> {
    // lWb/CVb uses kind 1. Gaia tJ.WW returns no identity, so field 3
    // is absent. This does not require account email or additional persistence.
    Zeroizing::new(
        Metadata {
            kind: 1,
            header: Some(request_header(token)),
        }
        .encode_to_vec(),
    )
}

fn prepare(path: &Path, root: &Path) -> Result<Prepared, MediaError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(handover_core::sanitize_file_name)
        .ok_or(MediaError::InvalidFile)?;
    if name.len() > 240 || !path.is_absolute() {
        return Err(MediaError::InvalidFile);
    }
    let root = root.canonicalize().map_err(|_| MediaError::InvalidFile)?;
    let link = std::fs::symlink_metadata(path).map_err(|_| MediaError::InvalidFile)?;
    if !link.is_file() || link.file_type().is_symlink() {
        return Err(MediaError::InvalidFile);
    }
    let canonical = path.canonicalize().map_err(|_| MediaError::InvalidFile)?;
    if !canonical.starts_with(&root) || canonical == root {
        return Err(MediaError::InvalidFile);
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&canonical)
        .map_err(|_| MediaError::InvalidFile)?;
    // Recheck the opened descriptor, not just the path before opening it.
    use std::os::fd::AsRawFd;
    let descriptor = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
        .map_err(|_| MediaError::InvalidFile)?;
    if !descriptor.starts_with(&root) {
        return Err(MediaError::InvalidFile);
    }
    let size = file.metadata().map_err(|_| MediaError::InvalidFile)?;
    if !size.is_file() || size.len() == 0 {
        return Err(MediaError::InvalidFile);
    }
    if size.len() > MAX_MEDIA_BYTES as u64 {
        return Err(MediaError::TooLarge);
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(size.len() as usize));
    (&mut file)
        .take(MAX_MEDIA_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| MediaError::InvalidFile)?;
    if bytes.len() != size.len() as usize {
        return Err(MediaError::InvalidFile);
    }
    let (mime, kind) = identify(&bytes);
    let mut key = Zeroizing::new([0; 32]);
    OsRng
        .try_fill_bytes(key.as_mut())
        .map_err(|_| MediaError::Randomness)?;
    let ciphertext = encrypt(&bytes, &key)?;
    Ok(Prepared {
        ciphertext,
        key,
        name: Zeroizing::new(name),
        mime,
        kind,
        size: bytes.len(),
    })
}

fn identify(bytes: &[u8]) -> (&'static str, i32) {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("image/png", 3)
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        ("image/jpeg", 1)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        ("image/gif", 4)
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        ("image/webp", 7)
    } else if bytes.starts_with(b"%PDF-") {
        ("application/pdf", 25)
    } else if bytes.starts_with(b"PK\x03\x04") {
        ("application/zip", 35)
    } else {
        ("application/octet-stream", 0)
    }
}

pub(crate) async fn upload_staged(
    registration: &crate::registration::UnpairedRegistration,
    path: Zeroizing<String>,
) -> Result<Uploaded, MediaError> {
    let root = handover_gmessages::staging::default_staging_directory()
        .map_err(|_| MediaError::InvalidFile)?;
    let prepared = tokio::task::spawn_blocking(move || prepare(Path::new(path.as_str()), &root))
        .await
        .map_err(|_| MediaError::InvalidFile)??;
    let metadata = registration
        .media_upload_metadata()
        .map_err(|_| MediaError::CredentialUnavailable)?;
    let http = crate::client(true).map_err(|_| MediaError::Network)?;
    upload_at(&http, ENDPOINT, prepared, &metadata).await
}

fn upload_url(base: &str, value: &str) -> Result<Zeroizing<String>, MediaError> {
    if value.len() > 4096 {
        return Err(MediaError::UploadEndpoint);
    }
    let base = reqwest::Url::parse(base).map_err(|_| MediaError::UploadEndpoint)?;
    let url = reqwest::Url::parse(value).map_err(|_| MediaError::UploadEndpoint)?;
    if url.origin() != base.origin()
        || url.path() != base.path()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(MediaError::UploadEndpoint);
    }
    Ok(Zeroizing::new(value.into()))
}

async fn body(mut response: reqwest::Response) -> Result<Zeroizing<Vec<u8>>, MediaError> {
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| MediaError::Network)? {
        if chunk.len() > RESPONSE_LIMIT - bytes.len() {
            return Err(MediaError::UploadProtocol);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn blob(bytes: &[u8]) -> Result<Zeroizing<String>, MediaError> {
    if bytes.len() > RESPONSE_LIMIT {
        return Err(MediaError::UploadProtocol);
    }
    let mut response = Response::decode(bytes)
        .ok()
        .filter(|response| response.media.is_some());
    if response.is_none() {
        let decoded = Zeroizing::new(
            STANDARD
                .decode(bytes)
                .map_err(|_| MediaError::UploadProtocol)?,
        );
        response =
            Some(Response::decode(decoded.as_slice()).map_err(|_| MediaError::UploadProtocol)?);
    }
    let mut media = response
        .and_then(|mut response| response.media.take())
        .ok_or(MediaError::UploadProtocol)?;
    if media.kind != 1
        || media.id.is_empty()
        || media.id.len() > 1024
        || media.id.chars().any(char::is_control)
    {
        return Err(MediaError::UploadProtocol);
    }
    Ok(Zeroizing::new(std::mem::take(&mut media.id)))
}

async fn upload_at(
    http: &reqwest::Client,
    endpoint: &str,
    file: Prepared,
    metadata: &[u8],
) -> Result<Uploaded, MediaError> {
    let metadata = Zeroizing::new(STANDARD.encode(metadata));
    let start = http
        .post(endpoint)
        .header("X-Goog-Upload-Protocol", "resumable")
        .header("X-Goog-Upload-Command", "start")
        .header("X-Goog-Upload-Header-Content-Length", file.ciphertext.len())
        .header("X-Goog-Upload-Header-Content-Type", file.mime)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded;charset=utf-8",
        )
        .body(metadata.as_bytes().to_vec())
        .send()
        .await
        .map_err(|_| MediaError::Network)?;
    if start.status() != reqwest::StatusCode::OK {
        return Err(MediaError::Http(start.status().as_u16()));
    }
    if start
        .headers()
        .get("X-Goog-Upload-Status")
        .and_then(|value| value.to_str().ok())
        != Some("active")
    {
        return Err(MediaError::UploadProtocol);
    }
    let url = start
        .headers()
        .get("X-Goog-Upload-URL")
        .and_then(|value| value.to_str().ok())
        .ok_or(MediaError::UploadProtocol)?;
    let url = upload_url(endpoint, url)?;
    let granularity = start
        .headers()
        .get("X-Goog-Upload-Chunk-Granularity")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if !granularity.is_some_and(|value| value > 0 && value <= 1 << 30) {
        return Err(MediaError::UploadProtocol);
    }
    body(start).await?;
    let finish = http
        .post(url.as_str())
        .header("X-Goog-Upload-Header-Content-Length", file.ciphertext.len())
        .header("X-Goog-Upload-Header-Content-Type", file.mime)
        .header("X-Goog-Upload-Command", "upload, finalize")
        .header("X-Goog-Upload-Offset", "0")
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded;charset=utf-8",
        )
        .body(file.ciphertext.to_vec())
        .send()
        .await
        .map_err(|_| MediaError::Network)?;
    if finish.status() != reqwest::StatusCode::OK {
        return Err(MediaError::Http(finish.status().as_u16()));
    }
    if finish
        .headers()
        .get("X-Goog-Upload-Status")
        .and_then(|value| value.to_str().ok())
        != Some("final")
    {
        return Err(MediaError::UploadProtocol);
    }
    let blob = blob(&body(finish).await?)?;
    Ok(Uploaded {
        blob,
        key: file.key,
        name: file.name,
        mime: file.mime,
        kind: file.kind,
        size: file.size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn prepared(data: &[u8]) -> Prepared {
        Prepared {
            ciphertext: crate::media::encrypt(data, &[9; 32]).unwrap(),
            key: Zeroizing::new([9; 32]),
            name: Zeroizing::new("fixture.bin".into()),
            mime: "application/octet-stream",
            kind: 0,
            size: data.len(),
        }
    }

    #[test]
    fn prepare_accepts_a_nonempty_file_beneath_root() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\nfixture").unwrap();

        let file = prepare(&path, directory.path()).unwrap();
        assert_eq!(file.name.as_str(), "fixture.png");
        assert_eq!(file.mime, "image/png");
        assert_eq!(file.kind, 3);
        assert_eq!(file.size, 15);
        assert!(!file.ciphertext.is_empty());
    }

    #[test]
    fn prepare_rejects_outside_empty_symlink_and_fifo_paths() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.bin");
        std::fs::write(&outside_file, b"outside").unwrap();
        assert_eq!(
            prepare(&outside_file, root.path()).err().unwrap(),
            MediaError::InvalidFile
        );

        let empty = root.path().join("empty.bin");
        std::fs::write(&empty, []).unwrap();
        assert_eq!(
            prepare(&empty, root.path()).err().unwrap(),
            MediaError::InvalidFile
        );

        let link = root.path().join("link.bin");
        symlink(&outside_file, &link).unwrap();
        assert_eq!(
            prepare(&link, root.path()).err().unwrap(),
            MediaError::InvalidFile
        );

        let fifo = root.path().join("pipe.bin");
        let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: path points to a valid NUL-terminated pathname and mode is conventional.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert_eq!(
            prepare(&fifo, root.path()).err().unwrap(),
            MediaError::InvalidFile
        );

        let oversized = root.path().join("oversized.bin");
        let file = std::fs::File::create(&oversized).unwrap();
        file.set_len(MAX_MEDIA_BYTES as u64 + 1).unwrap();
        assert_eq!(
            prepare(&oversized, root.path()).err().unwrap(),
            MediaError::TooLarge
        );
    }

    #[test]
    fn upload_url_allows_only_same_origin_and_path_without_credentials_or_fragment() {
        let endpoint = "https://example.test/upload";
        assert!(upload_url(endpoint, "https://example.test/upload?token=opaque").is_ok());
        for value in [
            "http://example.test/upload",
            "https://other.test/upload",
            "https://example.test/other",
            "https://user@example.test/upload",
            "https://example.test/upload#fragment",
        ] {
            assert!(upload_url(endpoint, value).is_err());
        }
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut byte = [0; 1];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
            assert!(
                request.len() < 16 * 1024,
                "request headers exceeded fixture bound"
            );
        }
        let headers = String::from_utf8_lossy(&request).to_ascii_lowercase();
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let header_length = request.len();
        request.resize(header_length + length, 0);
        socket
            .read_exact(&mut request[header_length..])
            .await
            .unwrap();
        request
    }

    async fn response(
        socket: &mut tokio::net::TcpStream,
        status: &str,
        headers: &str,
        body: &[u8],
    ) {
        let prefix = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(prefix.as_bytes()).await.unwrap();
        socket.write_all(body).await.unwrap();
    }

    fn header<'a>(request: &'a [u8], name: &str) -> Option<&'a str> {
        let end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")?;
        std::str::from_utf8(&request[..end])
            .ok()?
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case(name).then_some(value.trim())
            })
    }

    #[tokio::test]
    async fn upload_uses_exactly_two_posts_and_sends_metadata_then_ciphertext() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/upload", listener.local_addr().unwrap());
        let server_endpoint = endpoint.clone();
        let server = tokio::spawn(async move {
            let (mut start, _) = listener.accept().await.unwrap();
            let first = read_request(&mut start).await;
            response(
                &mut start,
                "200 OK",
                &format!(
                    "X-Goog-Upload-Status: active\r\nX-Goog-Upload-URL: {server_endpoint}\r\nX-Goog-Upload-Chunk-Granularity: 4096\r\n"
                ),
                b"",
            )
            .await;
            drop(start);

            let (mut finish, _) = listener.accept().await.unwrap();
            let second = read_request(&mut finish).await;
            let raw_response = Response {
                media: Some(MediaIdentity {
                    id: "opaque-media-id".into(),
                    kind: 1,
                }),
            }
            .encode_to_vec();
            response(
                &mut finish,
                "200 OK",
                "X-Goog-Upload-Status: final\r\n",
                &raw_response,
            )
            .await;
            (first, second)
        });

        let metadata = b"private metadata fixture";
        let result = upload_at(
            &crate::client(false).unwrap(),
            &endpoint,
            prepared(b"plain fixture body"),
            metadata,
        )
        .await
        .unwrap();
        assert_eq!(result.blob.as_str(), "opaque-media-id");

        let (first, second) = server.await.unwrap();
        let first_text = String::from_utf8_lossy(&first);
        let second_text = String::from_utf8_lossy(&second);
        assert!(first_text.starts_with("POST /upload HTTP/1.1"));
        assert!(second_text.starts_with("POST /upload HTTP/1.1"));
        assert_eq!(header(&first, "X-Goog-Upload-Protocol"), Some("resumable"));
        assert_eq!(header(&first, "X-Goog-Upload-Command"), Some("start"));
        assert_eq!(
            header(&first, "X-Goog-Upload-Header-Content-Type"),
            Some("application/octet-stream")
        );
        assert_eq!(
            header(&second, "X-Goog-Upload-Command"),
            Some("upload, finalize")
        );
        assert_eq!(header(&second, "X-Goog-Upload-Offset"), Some("0"));
        assert_eq!(
            header(&second, "X-Goog-Upload-Header-Content-Type"),
            Some("application/octet-stream")
        );
        assert!(!first_text.to_ascii_lowercase().contains("cookie:"));
        assert!(!first_text.to_ascii_lowercase().contains("authorization:"));

        let first_body = &first[first
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4..];
        let decoded = STANDARD.decode(first_body).unwrap();
        assert_eq!(decoded, metadata);
        assert!(!first_body.windows(5).any(|window| window == b"plain"));
        let second_body = &second[second
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4..];
        assert!(
            !second_body
                .windows(b"plain fixture body".len())
                .any(|window| window == b"plain fixture body")
        );
        assert_eq!(
            header(&second, "X-Goog-Upload-Header-Content-Length")
                .unwrap()
                .parse::<usize>()
                .unwrap(),
            second_body.len()
        );
        let plaintext = crate::media::decrypt(second_body, &[9; 32]).unwrap();
        assert_eq!(&*plaintext, b"plain fixture body");
    }

    #[tokio::test]
    async fn upload_http_failure_does_not_retry() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/upload", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            response(&mut socket, "503 Service Unavailable", "", b"").await;
            drop(socket);
            let second =
                tokio::time::timeout(std::time::Duration::from_millis(200), listener.accept())
                    .await;
            (request, second.is_ok())
        });
        let result = upload_at(
            &crate::client(false).unwrap(),
            &endpoint,
            prepared(b"fixture"),
            b"metadata",
        )
        .await;
        assert_eq!(result.err().unwrap(), MediaError::Http(503));
        let (_, retried) = server.await.unwrap();
        assert!(!retried, "upload unexpectedly retried after HTTP failure");
    }

    #[tokio::test]
    async fn upload_rejects_missing_or_zero_granularity_before_second_post() {
        for granularity in [None, Some("0")] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}/upload", listener.local_addr().unwrap());
            let server_endpoint = endpoint.clone();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let _first = read_request(&mut socket).await;
                let granularity_header = granularity
                    .map(|value| format!("X-Goog-Upload-Chunk-Granularity: {value}\r\n"))
                    .unwrap_or_default();
                response(
                    &mut socket,
                    "200 OK",
                    &format!(
                        "X-Goog-Upload-Status: active\r\nX-Goog-Upload-URL: {server_endpoint}\r\n{granularity_header}"
                    ),
                    b"",
                )
                .await;
                drop(socket);
                tokio::time::timeout(std::time::Duration::from_millis(200), listener.accept())
                    .await
                    .is_ok()
            });
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                upload_at(
                    &crate::client(false).unwrap(),
                    &endpoint,
                    prepared(b"fixture"),
                    b"metadata",
                ),
            )
            .await
            .expect("upload request timed out");
            assert_eq!(result.err().unwrap(), MediaError::UploadProtocol);
            assert!(
                !server.await.unwrap(),
                "invalid granularity caused a second POST"
            );
        }
    }

    #[tokio::test]
    async fn upload_rejects_foreign_resumable_url_without_contacting_it() {
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let foreign = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/upload", origin.local_addr().unwrap());
        let foreign_url = format!("http://{}/upload", foreign.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = origin.accept().await.unwrap();
            let _first = read_request(&mut socket).await;
            response(
                &mut socket,
                "200 OK",
                &format!(
                    "X-Goog-Upload-Status: active\r\nX-Goog-Upload-URL: {foreign_url}\r\nX-Goog-Upload-Chunk-Granularity: 4096\r\n"
                ),
                b"",
            )
            .await;
            drop(socket);
            tokio::time::timeout(std::time::Duration::from_millis(250), foreign.accept())
                .await
                .is_ok()
        });
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            upload_at(
                &crate::client(false).unwrap(),
                &endpoint,
                prepared(b"fixture"),
                b"metadata",
            ),
        )
        .await
        .expect("upload request timed out");
        assert_eq!(result.err().unwrap(), MediaError::UploadEndpoint);
        assert!(
            !server.await.unwrap(),
            "foreign resumable URL received a connection"
        );
    }

    #[tokio::test]
    async fn upload_rejects_malformed_and_oversized_final_responses() {
        let oversized = vec![b'x'; RESPONSE_LIMIT + 1];
        for body in [b"\xff".to_vec(), oversized] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}/upload", listener.local_addr().unwrap());
            let server_endpoint = endpoint.clone();
            let server = tokio::spawn(async move {
                let (mut start, _) = listener.accept().await.unwrap();
                let _first = read_request(&mut start).await;
                response(
                    &mut start,
                    "200 OK",
                    &format!(
                        "X-Goog-Upload-Status: active\r\nX-Goog-Upload-URL: {server_endpoint}\r\nX-Goog-Upload-Chunk-Granularity: 4096\r\n"
                    ),
                    b"",
                )
                .await;
                drop(start);
                let (mut finish, _) =
                    tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept())
                        .await
                        .expect("final POST did not arrive")
                        .unwrap();
                let _second = read_request(&mut finish).await;
                response(
                    &mut finish,
                    "200 OK",
                    "X-Goog-Upload-Status: final\r\n",
                    &body,
                )
                .await;
            });
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                upload_at(
                    &crate::client(false).unwrap(),
                    &endpoint,
                    prepared(b"fixture"),
                    b"metadata",
                ),
            )
            .await
            .expect("upload request timed out");
            assert_eq!(result.err().unwrap(), MediaError::UploadProtocol);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn final_http_failure_does_not_retry() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/upload", listener.local_addr().unwrap());
        let server_endpoint = endpoint.clone();
        let server = tokio::spawn(async move {
            let (mut start, _) = listener.accept().await.unwrap();
            let _first = read_request(&mut start).await;
            response(
                &mut start,
                "200 OK",
                &format!(
                    "X-Goog-Upload-Status: active\r\nX-Goog-Upload-URL: {server_endpoint}\r\nX-Goog-Upload-Chunk-Granularity: 4096\r\n"
                ),
                b"",
            )
            .await;
            drop(start);
            let (mut finish, _) =
                tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept())
                    .await
                    .expect("final POST did not arrive")
                    .unwrap();
            let _second = read_request(&mut finish).await;
            response(&mut finish, "503 Service Unavailable", "", b"").await;
            drop(finish);
            tokio::time::timeout(std::time::Duration::from_millis(200), listener.accept())
                .await
                .is_ok()
        });
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            upload_at(
                &crate::client(false).unwrap(),
                &endpoint,
                prepared(b"fixture"),
                b"metadata",
            ),
        )
        .await
        .expect("upload request timed out");
        assert_eq!(result.err().unwrap(), MediaError::Http(503));
        assert!(!server.await.unwrap(), "final POST unexpectedly retried");
    }
}
