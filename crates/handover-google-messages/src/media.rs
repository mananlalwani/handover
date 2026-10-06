//! Bounded first-party media framing. Network and staging are separate.
//!
//! Independently observed in the public web client's SVb/TVb and PVb/QVb:
//! header [0, 15], 32 KiB encrypted chunks, nonce prefix, 128-bit GCM tag,
//! and five-byte authenticated data containing the final flag and index.

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use prost::Message;
use rand::{RngCore, rngs::OsRng};
use zeroize::{Zeroize, Zeroizing};
mod upload;
pub(crate) use upload::{Uploaded, metadata as upload_metadata, upload_staged};

/// A native-only full-size blob reference. It cannot expose secrets through
/// Debug or serialization, and never crosses the normalized helper contract.
pub struct Download {
    message: Zeroizing<String>,
    part: Zeroizing<String>,
    blob: Zeroizing<String>,
    key: Option<Zeroizing<[u8; 32]>>,
}

impl Download {
    pub(crate) fn apply_path(
        &self,
        messages: &mut [handover_core::messaging::Message],
        path: String,
    ) {
        if let Some(message) = messages.iter_mut().find(|message| self.matches(message)) {
            if let Some(part) = message
                .attachments
                .iter_mut()
                .find(|part| part.local_id == *self.part)
            {
                part.staged_path = Some(path);
            }
        }
    }
    pub(crate) async fn fetch(
        &self,
        registration: &crate::registration::UnpairedRegistration,
    ) -> Result<Zeroizing<Vec<u8>>, MediaError> {
        let metadata = registration
            .media_download_metadata(&self.blob)
            .map_err(|_| MediaError::CredentialUnavailable)?;
        let http = crate::client(true).map_err(|_| MediaError::Network)?;
        download(&http, self, &metadata).await
    }
    pub(crate) fn from_part(message: &str, part: &str, blob: &str, key: &[u8]) -> Option<Self> {
        if message.is_empty()
            || part.is_empty()
            || blob.is_empty()
            || blob.len() > 1024
            || blob.chars().any(char::is_control)
        {
            return None;
        }
        Some(Self {
            message: Zeroizing::new(message.into()),
            part: Zeroizing::new(part.into()),
            blob: Zeroizing::new(blob.into()),
            key: if key.is_empty() {
                None
            } else {
                Some(Zeroizing::new(key.try_into().ok()?))
            },
        })
    }

    pub(crate) fn matches(&self, message: &handover_core::messaging::Message) -> bool {
        message.id.local_id == *self.message
            && message
                .attachments
                .iter()
                .any(|part| part.local_id == *self.part)
    }

    pub(crate) fn matches_attachment(&self, message: &str, part: &str) -> bool {
        *self.message == message && *self.part == part
    }

    pub(crate) fn encrypted(&self) -> bool {
        self.key.is_some()
    }
}

/// Reuse the normalized helper's bounded, content-addressed staging policy.
/// Only this helper's private namespace is touched; the daemon imports the
/// result into its own cache before publishing the path to clients.
fn stage_bytes(bytes: &[u8], directory: &std::path::Path) -> Result<String, MediaError> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    if bytes.is_empty() {
        return Err(MediaError::Empty);
    }
    if bytes.len() > MAX_MEDIA_BYTES {
        return Err(MediaError::TooLarge);
    }
    std::fs::create_dir_all(directory).map_err(|_| MediaError::Staging)?;
    let metadata = std::fs::symlink_metadata(directory).map_err(|_| MediaError::Staging)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(MediaError::Staging);
    }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| MediaError::Staging)?;
    let mut file = tempfile::Builder::new()
        .prefix("native-download-")
        .tempfile_in(directory)
        .map_err(|_| MediaError::Staging)?;
    file.write_all(bytes).map_err(|_| MediaError::Staging)?;
    file.as_file().sync_all().map_err(|_| MediaError::Staging)?;
    let path = file.path().to_str().ok_or(MediaError::Staging)?;
    handover_gmessages::staging::import_staged_path(path, &[directory.to_owned()], directory)
        .map_err(|_| MediaError::Staging)?
        .into_os_string()
        .into_string()
        .map_err(|_| MediaError::Staging)
}

pub(crate) async fn hydrate_page(
    registration: &crate::registration::UnpairedRegistration,
    page: &mut crate::history::HistoryPage,
    budget: std::time::Duration,
) {
    let Ok(directory) = handover_gmessages::staging::adapter_staging_directory() else {
        return;
    };
    let directory = directory.join("native-media");
    // Enrichment is optional. Keep the validated history page even if a
    // download fails, and bound work independently of the history page size.
    let enrich = async {
        for reference in page.downloads.iter().take(8) {
            let Ok(bytes) = reference.fetch(registration).await else {
                continue;
            };
            let directory = directory.clone();
            let result = tokio::task::spawn_blocking(move || stage_bytes(&bytes, &directory)).await;
            if let Ok(Ok(path)) = result {
                reference.apply_path(&mut page.messages, path);
            }
        }
    };
    let _ = tokio::time::timeout(budget.min(std::time::Duration::from_secs(8)), enrich).await;
}

#[derive(Message)]
#[prost(skip_debug)]
struct DownloadMetadata {
    #[prost(message, optional, tag = "1")]
    media: Option<MediaIdentity>,
    #[prost(message, optional, tag = "2")]
    header: Option<RequestHeader>,
}
#[derive(Message)]
#[prost(skip_debug)]
struct MediaIdentity {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(int32, tag = "2")]
    kind: i32,
}
impl Drop for MediaIdentity {
    fn drop(&mut self) {
        self.id.zeroize();
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct RequestHeader {
    #[prost(string, tag = "1")]
    request: String,
    #[prost(string, tag = "3")]
    application: String,
    #[prost(bytes = "vec", tag = "6")]
    token: Vec<u8>,
    #[prost(message, optional, tag = "7")]
    client: Option<ClientInfo>,
}
impl Drop for RequestHeader {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}
#[derive(Message)]
struct ClientInfo {
    #[prost(uint32, tag = "3")]
    major: u32,
    #[prost(uint32, tag = "4")]
    minor: u32,
    #[prost(uint32, tag = "5")]
    patch: u32,
    #[prost(int32, tag = "7")]
    platform: i32,
    #[prost(int32, tag = "9")]
    variant: i32,
}

pub(crate) fn download_metadata(blob: &str, token: &[u8]) -> Zeroizing<Vec<u8>> {
    // iWb: MediaId field 1, kind 1; request header field 2.
    let metadata = DownloadMetadata {
        media: Some(MediaIdentity {
            id: blob.into(),
            kind: 1,
        }),
        header: Some(request_header(token)),
    };
    Zeroizing::new(metadata.encode_to_vec())
}

fn request_header(token: &[u8]) -> RequestHeader {
    RequestHeader {
        request: uuid::Uuid::new_v4().to_string(),
        application: "GDitto".into(),
        token: token.to_vec(),
        client: Some(ClientInfo {
            major: crate::OBSERVED_WIRE_VERSION[0],
            minor: crate::OBSERVED_WIRE_VERSION[1],
            patch: crate::OBSERVED_WIRE_VERSION[2],
            platform: 4,
            variant: 6,
        }),
    }
}

pub(crate) async fn download(
    http: &reqwest::Client,
    reference: &Download,
    metadata: &[u8],
) -> Result<Zeroizing<Vec<u8>>, MediaError> {
    download_at(
        http,
        "https://instantmessaging-pa.googleapis.com/upload",
        reference,
        metadata,
    )
    .await
}

async fn download_at(
    http: &reqwest::Client,
    url: &str,
    reference: &Download,
    metadata: &[u8],
) -> Result<Zeroizing<Vec<u8>>, MediaError> {
    let encoded = Zeroizing::new(STANDARD.encode(metadata));
    let mut header =
        reqwest::header::HeaderValue::from_str(&encoded).map_err(|_| MediaError::InvalidFraming)?;
    header.set_sensitive(true);
    let mut response = http
        .get(url)
        .header("X-Goog-Download-Metadata", header)
        .send()
        .await
        .map_err(|_| MediaError::Network)?;
    if !response.status().is_success() {
        return Err(MediaError::Http(response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_ENCODED_BYTES as u64)
    {
        return Err(MediaError::TooLarge);
    }
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| MediaError::Network)? {
        if chunk.len() > MAX_ENCODED_BYTES - body.len() {
            return Err(MediaError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    match &reference.key {
        Some(key) => decrypt(&body, key),
        None if body.is_empty() => Err(MediaError::Empty),
        None if body.len() > MAX_MEDIA_BYTES => Err(MediaError::TooLarge),
        // An encrypted-looking payload without a key must not be staged as
        // if it were usable plaintext.
        None if body.starts_with(&HEADER) => Err(MediaError::EncryptionKeyUnavailable),
        None => Ok(body),
    }
}

const HEADER: [u8; 2] = [0, 15];
const CHUNK_BYTES: usize = 32 * 1024;
const NONCE_BYTES: usize = 12;
const OVERHEAD: usize = NONCE_BYTES + 16;
const PLAIN_CHUNK_BYTES: usize = CHUNK_BYTES - OVERHEAD;
pub const MAX_MEDIA_BYTES: usize = handover_gmessages::staging::MAX_STAGED_BYTES as usize;
const MAX_ENCODED_BYTES: usize =
    2 + MAX_MEDIA_BYTES + MAX_MEDIA_BYTES.div_ceil(PLAIN_CHUNK_BYTES) * OVERHEAD;

/// Fixed categories contain no file contents, keys, or server data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaError {
    Empty,
    TooLarge,
    UnsupportedFraming,
    InvalidFraming,
    Authentication,
    Randomness,
    Network,
    Http(u16),
    CredentialUnavailable,
    ReferenceUnavailable,
    EncryptionKeyUnavailable,
    Staging,
    InvalidFile,
    UploadProtocol,
    UploadEndpoint,
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "media {:?}", self)
    }
}

impl std::error::Error for MediaError {}

/// Encrypt one nonempty file. Each chunk gets a fresh OS-generated nonce.
/// The caller owns the key and must keep it below the normalized IPC model.
pub fn encrypt(bytes: &[u8], key: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>, MediaError> {
    encrypt_with_nonce(bytes, key, |nonce| {
        OsRng
            .try_fill_bytes(nonce)
            .map_err(|_| MediaError::Randomness)
    })
}

fn encrypt_with_nonce(
    bytes: &[u8],
    key: &[u8; 32],
    mut nonce_source: impl FnMut(&mut [u8; NONCE_BYTES]) -> Result<(), MediaError>,
) -> Result<Zeroizing<Vec<u8>>, MediaError> {
    if bytes.is_empty() {
        return Err(MediaError::Empty);
    }
    if bytes.len() > MAX_MEDIA_BYTES {
        return Err(MediaError::TooLarge);
    }
    let cipher = Aes256Gcm::new(key.into());
    let count = bytes.len().div_ceil(PLAIN_CHUNK_BYTES);
    let mut output = Zeroizing::new(Vec::with_capacity(2 + bytes.len() + count * OVERHEAD));
    output.extend_from_slice(&HEADER);
    for (index, chunk) in bytes.chunks(PLAIN_CHUNK_BYTES).enumerate() {
        let mut nonce = [0; NONCE_BYTES];
        nonce_source(&mut nonce)?;
        let aad = chunk_aad(index, index + 1 == count);
        let encoded = Zeroizing::new(
            cipher
                .encrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: chunk,
                        aad: &aad,
                    },
                )
                .map_err(|_| MediaError::Authentication)?,
        );
        output.extend_from_slice(&nonce);
        output.extend_from_slice(&encoded);
    }
    Ok(output)
}

/// Authenticate the entire file before returning any plaintext. Reordered,
/// appended, or truncated chunks fail because index and finality are bound.
pub fn decrypt(bytes: &[u8], key: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>, MediaError> {
    if bytes.len() > MAX_ENCODED_BYTES {
        return Err(MediaError::TooLarge);
    }
    if bytes.len() < 2 || bytes[..2] != HEADER {
        return Err(MediaError::UnsupportedFraming);
    }
    let body = &bytes[2..];
    if body.is_empty() || body.len() % CHUNK_BYTES <= OVERHEAD && body.len() % CHUNK_BYTES != 0 {
        return Err(MediaError::InvalidFraming);
    }
    let cipher = Aes256Gcm::new(key.into());
    let count = body.len().div_ceil(CHUNK_BYTES);
    let mut output = Zeroizing::new(Vec::with_capacity(body.len() - count * OVERHEAD));
    for (index, chunk) in body.chunks(CHUNK_BYTES).enumerate() {
        let aad = chunk_aad(index, index + 1 == count);
        let plain = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&chunk[..NONCE_BYTES]),
                    Payload {
                        msg: &chunk[NONCE_BYTES..],
                        aad: &aad,
                    },
                )
                .map_err(|_| MediaError::Authentication)?,
        );
        if output.len() + plain.len() > MAX_MEDIA_BYTES {
            return Err(MediaError::TooLarge);
        }
        output.extend_from_slice(&plain);
    }
    Ok(output)
}

fn chunk_aad(index: usize, final_chunk: bool) -> [u8; 5] {
    // The 50 MiB bound keeps the chunk index far below u32::MAX.
    let mut aad = [0; 5];
    aad[0] = u8::from(final_chunk);
    aad[1..].copy_from_slice(&(index as u32).to_be_bytes());
    aad
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_privately_reuses_content_and_rejects_symlink_directory() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("native-media");
        let path = stage_bytes(b"file fixture", &directory).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"file fixture");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(stage_bytes(b"file fixture", &directory).unwrap(), path);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        let link = root.path().join("symlink-cache");
        symlink(&directory, &link).unwrap();
        assert_eq!(
            stage_bytes(b"other fixture", &link),
            Err(MediaError::Staging)
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        assert_eq!(stage_bytes(&[], &directory), Err(MediaError::Empty));
    }

    async fn serve_once(
        status: &str,
        headers: &str,
        body: Vec<u8>,
    ) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/upload", listener.local_addr().unwrap());
        let prefix = format!("HTTP/1.1 {status}\r\n{headers}Connection: close\r\n\r\n");
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0; 1];
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                assert!(request.len() <= 16 * 1024);
            }
            socket.write_all(prefix.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
            request
        });
        (url, task)
    }

    #[tokio::test]
    async fn bounded_download_authenticates_without_browser_headers() {
        let reference = Download::from_part("message", "part", "blob", &[7; 32]).unwrap();
        let metadata = download_metadata("blob", b"private-fixture-token");
        let encoded = encrypt(b"file fixture", &[7; 32]).unwrap();
        let (url, task) = serve_once(
            "200 OK",
            &format!("Content-Length: {}\r\n", encoded.len()),
            encoded.to_vec(),
        )
        .await;
        assert_eq!(
            &*download_at(&crate::client(false).unwrap(), &url, &reference, &metadata)
                .await
                .unwrap(),
            b"file fixture"
        );
        let request = String::from_utf8(task.await.unwrap()).unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(!lower.contains("cookie:"));
        assert!(!lower.contains("authorization:"));
        assert!(!request.contains("private-fixture-token"));
        let header = request
            .lines()
            .find(|line| {
                line.to_ascii_lowercase()
                    .starts_with("x-goog-download-metadata:")
            })
            .unwrap()
            .split_once(':')
            .unwrap()
            .1
            .trim();
        let bytes = STANDARD.decode(header).unwrap();
        let decoded = DownloadMetadata::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.media.as_ref().unwrap().id, "blob");
        assert_eq!(decoded.media.as_ref().unwrap().kind, 1);
        assert_eq!(
            decoded.header.as_ref().unwrap().token,
            b"private-fixture-token"
        );
    }

    #[tokio::test]
    async fn download_rejects_http_redirect_size_and_authentication_failures() {
        let reference = Download::from_part("message", "part", "blob", &[7; 32]).unwrap();
        let http = crate::client(false).unwrap();
        for (status, headers, body, expected) in [
            (
                "302 Found",
                "Location: http://127.0.0.1:1/secret\r\nContent-Length: 0\r\n".into(),
                Vec::new(),
                MediaError::Http(302),
            ),
            (
                "401 Unauthorized",
                "Content-Length: 0\r\n".into(),
                Vec::new(),
                MediaError::Http(401),
            ),
            (
                "200 OK",
                format!("Content-Length: {}\r\n", MAX_ENCODED_BYTES + 1),
                Vec::new(),
                MediaError::TooLarge,
            ),
            (
                "200 OK",
                String::new(),
                encrypt(b"fixture", &[8; 32]).unwrap().to_vec(),
                MediaError::Authentication,
            ),
        ] {
            let (url, task) = serve_once(status, &headers, body).await;
            assert_eq!(
                download_at(&http, &url, &reference, b"metadata").await,
                Err(expected)
            );
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn explicit_unencrypted_reference_does_not_bypass_gcm() {
        let reference = Download::from_part("message", "part", "blob", &[]).unwrap();
        assert!(!reference.encrypted());
        assert!(Download::from_part("message", "part", "blob", &[7; 31]).is_none());
        let http = crate::client(false).unwrap();
        let (url, task) = serve_once("200 OK", "", b"clear file fixture".to_vec()).await;
        assert_eq!(
            &*download_at(&http, &url, &reference, b"metadata")
                .await
                .unwrap(),
            b"clear file fixture"
        );
        task.await.unwrap();
        let (url, task) = serve_once(
            "200 OK",
            "",
            encrypt(b"encrypted", &[7; 32]).unwrap().to_vec(),
        )
        .await;
        assert_eq!(
            download_at(&http, &url, &reference, b"metadata").await,
            Err(MediaError::EncryptionKeyUnavailable)
        );
        task.await.unwrap();
    }

    #[test]
    fn matches_independent_webcrypto_fixture() {
        // Node WebCrypto AES-GCM, key [7;32], nonce [3;12], AAD [1,0,0,0,0].
        let hex = "000f0303030303030303030303036d9fcd67355e3b305a2d26388236df6a8444949ac6f0bd66856acbb3b477974d54cd21d0e4c8";
        let fixture: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let plain = b"Handover media fixture";
        assert_eq!(&*decrypt(&fixture, &[7; 32]).unwrap(), plain);
        let encoded = encrypt_with_nonce(plain, &[7; 32], |nonce| {
            nonce.fill(3);
            Ok(())
        })
        .unwrap();
        assert_eq!(*encoded, fixture);
    }

    #[test]
    fn handles_full_and_partial_chunk_boundaries() {
        for size in [
            1,
            PLAIN_CHUNK_BYTES - 1,
            PLAIN_CHUNK_BYTES,
            PLAIN_CHUNK_BYTES + 1,
            PLAIN_CHUNK_BYTES * 2,
        ] {
            let plain = vec![91; size];
            let encoded = encrypt(&plain, &[5; 32]).unwrap();
            assert_eq!(*decrypt(&encoded, &[5; 32]).unwrap(), plain);
        }
    }

    #[test]
    fn multi_chunk_encoding_matches_webcrypto_digest() {
        use sha2::{Digest, Sha256};
        let encoded = encrypt_with_nonce(&vec![91; PLAIN_CHUNK_BYTES * 2 + 1], &[7; 32], |nonce| {
            nonce.fill(3);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(encoded.as_slice())),
            "15cd0dd4f52f82b1c820bb5c30aad1cfeba83db90baba72412a5d54bd470bc50"
        );
    }

    #[test]
    fn rejects_wrong_key_tampering_reordering_and_truncation() {
        let encoded = encrypt(&vec![17; PLAIN_CHUNK_BYTES * 3], &[5; 32]).unwrap();
        assert_eq!(decrypt(&encoded, &[6; 32]), Err(MediaError::Authentication));
        let mut corrupted = encoded.to_vec();
        corrupted[20] ^= 1;
        assert_eq!(
            decrypt(&corrupted, &[5; 32]),
            Err(MediaError::Authentication)
        );
        let mut reordered = encoded.to_vec();
        reordered[2..2 + CHUNK_BYTES]
            .copy_from_slice(&encoded[2 + CHUNK_BYTES..2 + 2 * CHUNK_BYTES]);
        reordered[2 + CHUNK_BYTES..2 + 2 * CHUNK_BYTES]
            .copy_from_slice(&encoded[2..2 + CHUNK_BYTES]);
        assert_eq!(
            decrypt(&reordered, &[5; 32]),
            Err(MediaError::Authentication)
        );
        assert_eq!(
            decrypt(&encoded[..2 + 2 * CHUNK_BYTES], &[5; 32]),
            Err(MediaError::Authentication)
        );
        assert!(decrypt(&encoded[..encoded.len() - 1], &[5; 32]).is_err());
        let mut appended = encoded.to_vec();
        appended.extend_from_slice(&encoded[2..2 + CHUNK_BYTES]);
        assert_eq!(
            decrypt(&appended, &[5; 32]),
            Err(MediaError::Authentication)
        );
    }

    #[test]
    fn rejects_empty_unsupported_and_oversized_files() {
        assert_eq!(encrypt(&[], &[0; 32]), Err(MediaError::Empty));
        assert_eq!(decrypt(&HEADER, &[0; 32]), Err(MediaError::InvalidFraming));
        assert_eq!(
            decrypt(&[0, 14, 1], &[0; 32]),
            Err(MediaError::UnsupportedFraming)
        );
        assert_eq!(
            encrypt(&vec![0; MAX_MEDIA_BYTES + 1], &[0; 32]),
            Err(MediaError::TooLarge)
        );
        assert_eq!(
            decrypt(&vec![0; MAX_ENCODED_BYTES + 1], &[0; 32]),
            Err(MediaError::TooLarge)
        );
    }

    #[test]
    fn randomness_failure_returns_no_partial_file() {
        assert_eq!(
            encrypt_with_nonce(&[1], &[0; 32], |_| Err(MediaError::Randomness)),
            Err(MediaError::Randomness)
        );
    }
}
