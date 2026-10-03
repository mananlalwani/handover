//! Independently authored, read-only Google Messages authentication probe.
//!
//! This does not register a device, pair a phone, or provide a messaging backend.

pub mod native;

use reqwest::{
    Client,
    header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, ORIGIN},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fmt, time::Duration};
use uuid::Uuid;

const ORIGIN_VALUE: &str = "https://messages.google.com";
const SIGN_IN_PATH: &str =
    "/$rpc/google.internal.communications.instantmessaging.v1.Registration/SignInGaia";
const ENDPOINTS: [&str; 3] = [
    "https://instantmessaging-pa.googleapis.com",
    "https://instantmessaging-pa.clients6.google.com",
    "https://instantmessaging-pa-jms-us.clients6.google.com",
];
const RESPONSE_LIMIT: usize = 512 * 1024;
const SOURCE_LIMIT: usize = 128;
// Public MW_CONFIG build label comms-messages.web-server_20261001.02_p0,
// interpreted by OPa and GH in the independently captured Google source.
const OBSERVED_WIRE_VERSION: [u32; 3] = [20261001, 2, 0];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RpcStatus {
    Cancelled,
    Unknown,
    InvalidArgument,
    DeadlineExceeded,
    NotFound,
    AlreadyExists,
    PermissionDenied,
    Unauthenticated,
    ResourceExhausted,
    FailedPrecondition,
    Aborted,
    OutOfRange,
    Unimplemented,
    Internal,
    Unavailable,
    DataLoss,
}

/// A one-use browser proof. Debug deliberately excludes all supplied values.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserProof {
    #[serde(rename = "type")]
    pub kind: String,
    pub endpoint: String,
    pub origin: String,
    pub authorization: String,
    pub api_key: String,
    pub auth_user: Option<String>,
    pub service_cookie: Option<String>,
}

impl fmt::Debug for BrowserProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BrowserProof { redacted }")
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ProbeResult {
    pub sources: usize,
}

/// Fixed errors contain neither credentials nor server payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    InvalidOrigin,
    InvalidFrame,
    InvalidBootstrap,
    InvalidEndpoint,
    InvalidCredentials,
    Network,
    HttpError(u16),
    HttpErrorWithStatus(u16, RpcStatus),
    UnexpectedResponse,
    ResponseTooLarge,
    Timeout,
    RpcError,
    NativeError,
}

impl ProbeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidOrigin => "invalid_origin",
            Self::InvalidFrame => "invalid_frame",
            Self::InvalidBootstrap => "invalid_bootstrap",
            Self::InvalidEndpoint => "invalid_endpoint",
            Self::InvalidCredentials => "invalid_credentials",
            Self::Network => "network",
            Self::HttpError(_) | Self::HttpErrorWithStatus(_, _) => "http_error",
            Self::UnexpectedResponse => "unexpected_response",
            Self::ResponseTooLarge => "response_too_large",
            Self::Timeout => "timeout",
            Self::RpcError => "rpc_error",
            Self::NativeError => "native_error",
        }
    }

    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::HttpError(status) | Self::HttpErrorWithStatus(status, _)
                if (100..=599).contains(status) =>
            {
                Some(*status)
            }
            _ => None,
        }
    }

    pub fn rpc_status(&self) -> Option<RpcStatus> {
        match self {
            Self::HttpErrorWithStatus(_, status) => Some(*status),
            _ => None,
        }
    }
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for ProbeError {}

fn sensitive_header(value: &str, limit: usize) -> Result<HeaderValue, ProbeError> {
    if value.is_empty() || value.len() > limit || !value.bytes().all(|b| (0x20..=0x7e).contains(&b))
    {
        return Err(ProbeError::InvalidCredentials);
    }
    let mut header = HeaderValue::from_str(value).map_err(|_| ProbeError::InvalidCredentials)?;
    header.set_sensitive(true);
    Ok(header)
}

impl BrowserProof {
    fn validate(&self) -> Result<HeaderMap, ProbeError> {
        if !matches!(
            self.kind.as_str(),
            "gaia_lookup" | "gaia_lookup_with_cookies"
        ) {
            return Err(ProbeError::InvalidBootstrap);
        }
        if self.origin != ORIGIN_VALUE {
            return Err(ProbeError::InvalidOrigin);
        }
        if !ENDPOINTS.contains(&self.endpoint.as_str()) {
            return Err(ProbeError::InvalidEndpoint);
        }
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, sensitive_header(&self.authorization, 8192)?);
        headers.insert("x-goog-api-key", sensitive_header(&self.api_key, 4096)?);
        match (self.kind.as_str(), self.service_cookie.as_deref()) {
            ("gaia_lookup", None) => {}
            ("gaia_lookup_with_cookies", Some(cookie)) => {
                headers.insert(COOKIE, sensitive_header(cookie, 16384)?);
            }
            _ => return Err(ProbeError::InvalidCredentials),
        }
        if let Some(user) = &self.auth_user {
            if user.is_empty() || user.len() > 2 || !user.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ProbeError::InvalidCredentials);
            }
            headers.insert("x-goog-authuser", sensitive_header(user, 2)?);
        }
        headers.insert(ORIGIN, HeaderValue::from_static(ORIGIN_VALUE));
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json+protobuf"),
        );
        headers.insert(
            "x-user-agent",
            HeaderValue::from_static("grpc-web-javascript/0.1"),
        );
        Ok(headers)
    }
}

fn lookup_request() -> Value {
    // Google's public web source: Z4a mode 1, BC field 1, RKa device type 3.
    // All identifiers are fresh. Mode 0 device registration is never requested.
    json!([
        [
            Uuid::new_v4().to_string(),
            null,
            "GDitto",
            null,
            null,
            null,
            [
                null,
                null,
                OBSERVED_WIRE_VERSION[0],
                OBSERVED_WIRE_VERSION[1],
                OBSERVED_WIRE_VERSION[2],
                null,
                4,
                null,
                6
            ]
        ],
        [[3, format!("messages-web-{}", Uuid::new_v4())]],
        1,
        "GDitto"
    ])
}

fn client(https_only: bool) -> Result<Client, ProbeError> {
    Client::builder()
        .https_only(https_only)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| ProbeError::Network)
}

fn transport_error(error: reqwest::Error) -> ProbeError {
    if error.is_timeout() {
        ProbeError::Timeout
    } else {
        ProbeError::Network
    }
}

/// Query registered source count with SignInGaia mode 1, once.
/// Cookies require the separate explicit comparison request type.
/// This does not attest pairing or expose any returned source identifiers.
pub async fn probe(proof: BrowserProof) -> Result<ProbeResult, ProbeError> {
    let headers = proof.validate()?;
    let endpoint = format!("{}{SIGN_IN_PATH}", proof.endpoint);
    let http = client(true)?;
    query(&http, &endpoint, headers, proof.auth_user.as_deref()).await
}

async fn query(
    http: &Client,
    endpoint: &str,
    headers: HeaderMap,
    auth_user: Option<&str>,
) -> Result<ProbeResult, ProbeError> {
    let body = serde_json::to_vec(&lookup_request()).map_err(|_| ProbeError::NativeError)?;
    let mut request = http.post(endpoint).headers(headers).body(body);
    if let Some(user) = auth_user {
        request = request.query(&[("authuser", user)]);
    }
    let mut response = request.send().await.map_err(transport_error)?;
    if !response.status().is_success() {
        return Err(http_error_details(response).await);
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.split(';').next().map(str::trim) != Some("application/json+protobuf") {
        return Err(ProbeError::UnexpectedResponse);
    }
    if response
        .content_length()
        .is_some_and(|len| len > RESPONSE_LIMIT as u64)
    {
        return Err(ProbeError::ResponseTooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if chunk.len() > RESPONSE_LIMIT - body.len() {
            return Err(ProbeError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    parse_response(&body)
}

async fn http_error_details(mut response: reqwest::Response) -> ProbeError {
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    const ERROR_LIMIT: usize = 16 * 1024;
    if !matches!(
        content_type,
        "application/json" | "application/json+protobuf"
    ) || response
        .content_length()
        .is_some_and(|n| n > ERROR_LIMIT as u64)
    {
        return ProbeError::HttpError(status);
    }
    let category = tokio::time::timeout(Duration::from_secs(1), async {
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if chunk.len() > ERROR_LIMIT - body.len() {
                return None;
            }
            body.extend_from_slice(&chunk);
        }
        // AIP-193 status is a fixed enum. Never retain or return message/details.
        let value: Value = serde_json::from_slice(&body).ok()?;
        serde_json::from_value::<RpcStatus>(value.get("error")?.get("status")?.clone()).ok()
    })
    .await
    .ok()
    .flatten();
    match category {
        Some(category) => ProbeError::HttpErrorWithStatus(status, category),
        None => ProbeError::HttpError(status),
    }
}

fn parse_response(body: &[u8]) -> Result<ProbeResult, ProbeError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| ProbeError::UnexpectedResponse)?;
    if value.is_object() {
        return Err(ProbeError::RpcError);
    }
    let fields = value.as_array().ok_or(ProbeError::UnexpectedResponse)?;
    if fields.is_empty() || fields.len() > 3 || !fields[0].is_array() {
        return Err(ProbeError::UnexpectedResponse);
    }
    // mPa field 3 is YC; YC repeated field 3 contains registered sources.
    let sources = match fields.get(2) {
        None | Some(Value::Null) => 0,
        Some(Value::Array(list)) if list.len() <= 3 => match list.get(2) {
            None | Some(Value::Null) => 0,
            Some(Value::Array(sources))
                if sources.len() <= SOURCE_LIMIT && sources.iter().all(Value::is_array) =>
            {
                sources.len()
            }
            _ => return Err(ProbeError::UnexpectedResponse),
        },
        _ => return Err(ProbeError::UnexpectedResponse),
    };
    Ok(ProbeResult { sources })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proof() -> BrowserProof {
        BrowserProof {
            kind: "gaia_lookup".into(),
            endpoint: ENDPOINTS[0].into(),
            origin: ORIGIN_VALUE.into(),
            authorization: "sensitive-auth".into(),
            api_key: "sensitive-key".into(),
            auth_user: Some("0".into()),
            service_cookie: None,
        }
    }

    #[test]
    fn credentials_are_redacted_and_headers_are_sensitive() {
        let proof = proof();
        let headers = proof.validate().unwrap();
        assert!(headers[AUTHORIZATION].is_sensitive());
        assert!(headers["x-goog-api-key"].is_sensitive());
        assert!(!format!("{proof:?} {headers:?}").contains("sensitive-auth"));
        assert!(!headers.contains_key("cookie"));
        assert!(serde_json::from_str::<BrowserProof>(r#"{"type":"gaia_lookup","endpoint":"x","origin":"x","authorization":"x","api_key":"x","cookie":"secret"}"#).is_err());
    }

    #[test]
    fn validation_rejects_credential_routing_and_header_injection() {
        for endpoint in [
            "http://instantmessaging-pa.googleapis.com",
            "https://instantmessaging-pa.googleapis.com/",
            "https://instantmessaging-pa.googleapis.com:443",
            "https://evil.test",
        ] {
            let mut p = proof();
            p.endpoint = endpoint.into();
            assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidEndpoint);
        }
        let mut p = proof();
        p.origin = "https://evil.test".into();
        assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidOrigin);
        let mut p = proof();
        p.authorization = "ok\r\nCookie: secret".into();
        assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidCredentials);
        let mut p = proof();
        p.auth_user = Some("100".into());
        assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidCredentials);
    }

    #[test]
    fn request_only_lists_with_fresh_identifiers() {
        let first = lookup_request();
        let second = lookup_request();
        assert_eq!(first[2], 1);
        assert_eq!(first[1][0][0], 3);
        assert_eq!(first[3], "GDitto");
        assert_ne!(first[0][0], second[0][0]);
        assert_ne!(first[1][0][1], second[1][0][1]);
        assert_eq!(first[0].as_array().unwrap().len(), 7);
        assert_eq!(
            first[0][6],
            json!([null, null, 20261001, 2, 0, null, 4, null, 6])
        );
    }

    #[test]
    fn service_cookies_require_explicit_mode_and_remain_redacted() {
        let mut p = proof();
        p.service_cookie = Some("SID=synthetic-private-cookie".into());
        assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidCredentials);
        p.kind = "gaia_lookup_with_cookies".into();
        let headers = p.validate().unwrap();
        assert!(headers[COOKIE].is_sensitive());
        assert!(!format!("{p:?} {headers:?}").contains("synthetic-private-cookie"));
        p.service_cookie = None;
        assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidCredentials);
        for invalid in [
            "".to_owned(),
            "SID=x\r\nX-Injected: y".into(),
            "x".repeat(16385),
        ] {
            p.service_cookie = Some(invalid);
            assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidCredentials);
        }
        p.service_cookie = Some("SID=synthetic".into());
        p.endpoint = "https://unapproved.test".into();
        assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidEndpoint);
    }

    #[test]
    fn response_returns_only_count_and_rejects_unknown_shapes() {
        assert_eq!(
            parse_response(br#"[[],null,[null,null,[["private-id"],["another-id"]]]]"#).unwrap(),
            ProbeResult { sources: 2 }
        );
        assert_eq!(parse_response(b"[[]]").unwrap(), ProbeResult { sources: 0 });
        for input in [
            b"[]".as_slice(),
            b"[null]",
            b"[[],null,{}]",
            b"[[],null,[null,null,[42]]]",
            b"secret",
        ] {
            assert_eq!(
                parse_response(input).unwrap_err(),
                ProbeError::UnexpectedResponse
            );
        }
        assert_eq!(
            parse_response(br#"{"error":{"message":"private-server-error"}}"#).unwrap_err(),
            ProbeError::RpcError
        );
        let many = json!([[], null, [null, null, vec![json!([]); SOURCE_LIMIT + 1]]]);
        assert_eq!(
            parse_response(&serde_json::to_vec(&many).unwrap()).unwrap_err(),
            ProbeError::UnexpectedResponse
        );
    }

    async fn mock_server(response: Vec<u8>) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read != 0 && request.len() + read < 32768);
                request.extend_from_slice(&buffer[..read]);
                if let Some(offset) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&request[..offset]).unwrap();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= offset + 4 + length {
                        break;
                    }
                }
            }
            // Oversized/redirect replies can cause the client to close early.
            let _ = stream.write_all(&response).await;
            request
        });
        (endpoint, task)
    }

    #[tokio::test]
    async fn real_transport_sends_only_owned_lookup_request() {
        let body = b"[[],null,[null,null,[[]]]]";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf; charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let (endpoint, server) = mock_server([response.as_bytes(), body].concat()).await;
        assert_eq!(
            query(
                &client(false).unwrap(),
                &format!("{endpoint}{SIGN_IN_PATH}"),
                proof().validate().unwrap(),
                Some("0")
            )
            .await
            .unwrap(),
            ProbeResult { sources: 1 }
        );
        let request = server.await.unwrap();
        let offset = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let headers = std::str::from_utf8(&request[..offset])
            .unwrap()
            .to_ascii_lowercase();
        assert!(
            headers.starts_with(
                &format!("post {SIGN_IN_PATH}?authuser=0 http/1.1").to_ascii_lowercase()
            )
        );
        assert!(headers.contains("origin: https://messages.google.com"));
        assert!(!headers.contains("cookie:"));
        let body: Value = serde_json::from_slice(&request[offset + 4..]).unwrap();
        assert_eq!(body[2], 1);
    }

    #[tokio::test]
    async fn real_transport_rejects_redirects_and_bounded_bad_responses() {
        let cases: Vec<(Vec<u8>, ProbeError)> = vec![
            (b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/secret\r\nContent-Length: 0\r\n\r\n".to_vec(), ProbeError::HttpError(302)),
            (b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 18\r\n\r\nprivate-error-body".to_vec(), ProbeError::HttpError(401)),
            (b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\n\r\n".to_vec(), ProbeError::UnexpectedResponse),
            (format!("HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf\r\nContent-Length: {}\r\n\r\n", RESPONSE_LIMIT + 1).into_bytes(), ProbeError::ResponseTooLarge),
            (format!("HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n", RESPONSE_LIMIT + 1, "x".repeat(RESPONSE_LIMIT + 1)).into_bytes(), ProbeError::ResponseTooLarge),
        ];
        for (response, expected) in cases {
            let (endpoint, server) = mock_server(response).await;
            assert_eq!(
                query(
                    &client(false).unwrap(),
                    &endpoint,
                    proof().validate().unwrap(),
                    None
                )
                .await
                .unwrap_err(),
                expected
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn cookie_comparison_forwards_only_the_explicit_sensitive_header() {
        let (endpoint, server) = mock_server(b"HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf\r\nContent-Length: 4\r\nConnection: close\r\n\r\n[[]]".to_vec()).await;
        let mut p = proof();
        p.kind = "gaia_lookup_with_cookies".into();
        p.service_cookie = Some("SID=synthetic-comparison".into());
        assert_eq!(
            query(
                &client(false).unwrap(),
                &endpoint,
                p.validate().unwrap(),
                None
            )
            .await
            .unwrap()
            .sources,
            0
        );
        let request = server.await.unwrap();
        let request = std::str::from_utf8(&request).unwrap();
        assert!(request.contains("cookie: SID=synthetic-comparison\r\n"));
        let offset = request.find("\r\n\r\n").unwrap();
        let body: Value = serde_json::from_str(&request[offset + 4..]).unwrap();
        assert_eq!(body[2], 1);
        assert!(!request[offset + 4..].contains("synthetic-comparison"));
    }

    #[tokio::test]
    async fn http_error_diagnostics_accept_only_fixed_rpc_categories() {
        for (body, expected) in [
            (
                json!({"error":{"status":"INVALID_ARGUMENT","message":"PRIVATE_SERVER_TEXT","details":{"cookie":"PRIVATE_COOKIE"}}}),
                ProbeError::HttpErrorWithStatus(400, RpcStatus::InvalidArgument),
            ),
            (
                json!({"error":{"status":"PRIVATE_SERVER_TEXT"}}),
                ProbeError::HttpError(400),
            ),
            (json!({"error":{"status":123}}), ProbeError::HttpError(400)),
            (
                json!({"error":{"status":"INVALID_ARGUMENT","message":"x".repeat(16384)}}),
                ProbeError::HttpError(400),
            ),
        ] {
            let body = serde_json::to_vec(&body).unwrap();
            let header = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let (endpoint, server) = mock_server([header.as_bytes(), &body].concat()).await;
            let result = query(
                &client(false).unwrap(),
                &endpoint,
                proof().validate().unwrap(),
                None,
            )
            .await
            .unwrap_err();
            assert_eq!(result, expected);
            assert!(!format!("{result:?}").contains("PRIVATE_"));
            server.await.unwrap();
        }
    }
}
