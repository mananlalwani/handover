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

impl RpcStatus {
    fn from_code(code: u64) -> Option<Self> {
        match code {
            1 => Some(Self::Cancelled),
            2 => Some(Self::Unknown),
            3 => Some(Self::InvalidArgument),
            4 => Some(Self::DeadlineExceeded),
            5 => Some(Self::NotFound),
            6 => Some(Self::AlreadyExists),
            7 => Some(Self::PermissionDenied),
            8 => Some(Self::ResourceExhausted),
            9 => Some(Self::FailedPrecondition),
            10 => Some(Self::Aborted),
            11 => Some(Self::OutOfRange),
            12 => Some(Self::Unimplemented),
            13 => Some(Self::Internal),
            14 => Some(Self::Unavailable),
            15 => Some(Self::DataLoss),
            16 => Some(Self::Unauthenticated),
            _ => None,
        }
    }
}

/// Recognized Google API infrastructure reasons, without server metadata.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RpcReason {
    ApiKeyInvalid,
    ApiKeyServiceBlocked,
    ApiKeyHttpReferrerBlocked,
    ApiKeyIpAddressBlocked,
    ApiKeyAndroidAppBlocked,
    ApiKeyIosAppBlocked,
    ConsumerInvalid,
    ServiceDisabled,
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
    pub browser_request: Option<Value>,
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
    HttpErrorWithReason(u16, RpcStatus, RpcReason),
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
            Self::HttpError(_)
            | Self::HttpErrorWithStatus(_, _)
            | Self::HttpErrorWithReason(_, _, _) => "http_error",
            Self::UnexpectedResponse => "unexpected_response",
            Self::ResponseTooLarge => "response_too_large",
            Self::Timeout => "timeout",
            Self::RpcError => "rpc_error",
            Self::NativeError => "native_error",
        }
    }

    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::HttpError(status)
            | Self::HttpErrorWithStatus(status, _)
            | Self::HttpErrorWithReason(status, _, _)
                if (100..=599).contains(status) =>
            {
                Some(*status)
            }
            _ => None,
        }
    }

    pub fn rpc_status(&self) -> Option<RpcStatus> {
        match self {
            Self::HttpErrorWithStatus(_, status) | Self::HttpErrorWithReason(_, status, _) => {
                Some(*status)
            }
            _ => None,
        }
    }

    pub fn rpc_reason(&self) -> Option<RpcReason> {
        match self {
            Self::HttpErrorWithReason(_, _, reason) => Some(*reason),
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
            "gaia_lookup" | "gaia_lookup_with_cookies" | "gaia_lookup_browser_request"
        ) {
            return Err(ProbeError::InvalidBootstrap);
        }
        match (self.kind.as_str(), self.browser_request.as_ref()) {
            ("gaia_lookup_browser_request", Some(body)) => validate_browser_lookup(body)?,
            ("gaia_lookup_browser_request", None) | (_, Some(_)) => {
                return Err(ProbeError::InvalidBootstrap);
            }
            (_, None) => {}
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
            ("gaia_lookup_with_cookies" | "gaia_lookup_browser_request", Some(cookie)) => {
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
        if self.kind == "gaia_lookup_browser_request" {
            headers.insert(
                reqwest::header::REFERER,
                HeaderValue::from_static("https://messages.google.com/"),
            );
        }
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

fn validate_browser_lookup(body: &Value) -> Result<(), ProbeError> {
    let valid = (|| {
        let fields = body.as_array()?;
        if fields.len() != 4 || fields[2] != 1 || fields[3] != "GDitto" {
            return None;
        }
        let header = fields[0].as_array()?;
        if header.len() != 7
            || header[2] != "GDitto"
            || [1, 3, 4, 5].iter().any(|i| !header[*i].is_null())
        {
            return None;
        }
        let request_id = header[0].as_str()?;
        if request_id.len() != 36 || Uuid::parse_str(request_id).is_err() {
            return None;
        }
        let info = header[6].as_array()?;
        if info.len() != 9
            || [0, 1, 5, 7].iter().any(|i| !info[*i].is_null())
            || info[6] != 4
            || info[8] != 6
            || [2, 3, 4]
                .iter()
                .any(|i| info[*i].as_u64().is_none_or(|n| n > u32::MAX as u64))
        {
            return None;
        }
        let device = fields[1].as_array()?;
        if device.len() != 1 {
            return None;
        }
        let id = device[0].as_array()?;
        if id.len() != 2 || id[0] != 3 {
            return None;
        }
        let suffix = id[1].as_str()?.strip_prefix("messages-web-")?;
        if !(suffix.len() == 32 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
            || suffix.len() == 36 && Uuid::parse_str(suffix).is_ok())
        {
            return None;
        }
        Some(())
    })();
    valid.ok_or(ProbeError::InvalidBootstrap)
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
        // Match the working browser's 32-character opaque suffix while keeping
        // the identifier fresh and independent of its stored device identity.
        [[3, format!("messages-web-{}", Uuid::new_v4().simple())]],
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
    if let Some(body) = proof.browser_request {
        query_with_body(&http, &endpoint, headers, proof.auth_user.as_deref(), &body).await
    } else {
        query(&http, &endpoint, headers, proof.auth_user.as_deref()).await
    }
}

async fn query(
    http: &Client,
    endpoint: &str,
    headers: HeaderMap,
    auth_user: Option<&str>,
) -> Result<ProbeResult, ProbeError> {
    query_with_body(http, endpoint, headers, auth_user, &lookup_request()).await
}

async fn query_with_body(
    http: &Client,
    endpoint: &str,
    headers: HeaderMap,
    auth_user: Option<&str>,
    body: &Value,
) -> Result<ProbeResult, ProbeError> {
    let body = serde_json::to_vec(body).map_err(|_| ProbeError::NativeError)?;
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
    const ERROR_LIMIT: usize = 16 * 1024;
    // The first-party error decoder accepts JSON-protobuf independently of
    // the media type. Only the bounded error schema below can yield a category.
    if response
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
        error_category(&body)
    })
    .await
    .ok()
    .flatten();
    match category {
        Some((category, Some(reason))) => ProbeError::HttpErrorWithReason(status, category, reason),
        Some((category, None)) => ProbeError::HttpErrorWithStatus(status, category),
        None => ProbeError::HttpError(status),
    }
}

fn error_category(body: &[u8]) -> Option<(RpcStatus, Option<RpcReason>)> {
    let value: Value = serde_json::from_slice(body).ok()?;
    if let Some(fields) = value.as_array() {
        // Google's public RPC transport decodes a JSPB RpcStatus: code field 1,
        // message field 2, repeated details field 3. Never return the latter two.
        if fields.is_empty()
            || fields.len() > 3
            || fields
                .get(1)
                .is_some_and(|v| !v.is_null() && !v.is_string())
            || fields.get(2).is_some_and(|v| !v.is_null() && !v.is_array())
        {
            return None;
        }
        let code = match &fields[0] {
            Value::Number(number) => number.as_u64()?,
            Value::String(text)
                if !text.is_empty()
                    && text.len() <= 2
                    && text.bytes().all(|b| b.is_ascii_digit()) =>
            {
                text.parse().ok()?
            }
            _ => return None,
        };
        return Some((RpcStatus::from_code(code)?, error_reason(fields.get(2))));
    }
    // The public AIP-193 JSON object representation is also supported.
    let error = value.get("error")?;
    Some((
        serde_json::from_value::<RpcStatus>(error.get("status")?.clone()).ok()?,
        error_reason(error.get("details")),
    ))
}

fn error_reason(details: Option<&Value>) -> Option<RpcReason> {
    let details = details?.as_array()?;
    if details.len() > 16 {
        return None;
    }
    let mut recognized = None;
    for detail in details {
        let Some(reason) = detail_reason(detail) else {
            continue;
        };
        if recognized.is_some_and(|previous| previous != reason) {
            return None;
        }
        recognized = Some(reason);
    }
    recognized
}

// Independently described from Google's public google.rpc.ErrorInfo schema.
// Metadata field 3 is intentionally absent so prost skips it without decoding.
#[derive(prost::Message)]
#[prost(skip_debug)]
struct ErrorInfoProjection {
    #[prost(string, tag = "1")]
    reason: String,
    #[prost(string, tag = "2")]
    domain: String,
}

impl fmt::Debug for ErrorInfoProjection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ErrorInfoProjection { redacted }")
    }
}

fn known_reason(reason: &str, domain: &str) -> Option<RpcReason> {
    if domain != "googleapis.com" || reason.len() > 63 {
        return None;
    }
    serde_json::from_value(Value::String(reason.to_owned())).ok()
}

fn detail_reason(detail: &Value) -> Option<RpcReason> {
    const TYPE: &str = "type.googleapis.com/google.rpc.ErrorInfo";
    if let Some(fields) = detail.as_array() {
        if fields.len() != 2 || fields[0].as_str()? != TYPE {
            return None;
        }
        if let Some(info) = fields[1].as_array() {
            if !(2..=3).contains(&info.len()) {
                return None;
            }
            return known_reason(info[0].as_str()?, info[1].as_str()?);
        }
        use base64::{Engine, engine::general_purpose};
        use prost::Message;
        let encoded = fields[1].as_str()?;
        if encoded.len() > 16 * 1024 {
            return None;
        }
        let bytes = [
            general_purpose::STANDARD,
            general_purpose::STANDARD_NO_PAD,
            general_purpose::URL_SAFE,
            general_purpose::URL_SAFE_NO_PAD,
        ]
        .iter()
        .find_map(|engine| engine.decode(encoded).ok())?;
        let info = ErrorInfoProjection::decode(bytes.as_slice()).ok()?;
        return known_reason(&info.reason, &info.domain);
    }
    if detail.get("@type").and_then(Value::as_str)? != TYPE {
        return None;
    }
    known_reason(
        detail.get("reason")?.as_str()?,
        detail.get("domain")?.as_str()?,
    )
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
        Some(Value::Array(list)) if list.len() <= 4 => match list.get(2) {
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
            browser_request: None,
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
        let device_id = first[1][0][1].as_str().unwrap();
        let suffix = device_id.strip_prefix("messages-web-").unwrap();
        assert_eq!(suffix.len(), 32);
        assert!(suffix.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(first[0].as_array().unwrap().len(), 7);
        assert_eq!(
            first[0][6],
            json!([null, null, 20261001, 2, 0, null, 4, null, 6])
        );
    }

    #[test]
    fn browser_comparison_requires_explicit_mode_and_rejects_effectful_or_token_requests() {
        let body = lookup_request();
        let mut p = proof();
        p.browser_request = Some(body.clone());
        assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidBootstrap);
        p.kind = "gaia_lookup_browser_request".into();
        assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidCredentials);
        p.service_cookie = Some("SID=synthetic".into());
        assert_eq!(
            p.validate().unwrap()[reqwest::header::REFERER],
            "https://messages.google.com/"
        );
        assert!(
            !proof()
                .validate()
                .unwrap()
                .contains_key(reqwest::header::REFERER)
        );
        for (pointer, value) in [
            ("/2", json!(0)),
            ("/0/5", json!("PRIVATE_TOKEN")),
            ("/0/1", json!("PRIVATE_ACCOUNT")),
            ("/1/0/1", json!("PRIVATE_DEVICE_ID")),
            ("/0/6/0", json!("PRIVATE_FIELD")),
            ("/3", json!("other-client")),
        ] {
            let mut invalid = body.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            p.browser_request = Some(invalid);
            assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidBootstrap);
        }
        let mut extra = body;
        extra.as_array_mut().unwrap().push(json!("PRIVATE_EXTRA"));
        p.browser_request = Some(extra);
        assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidBootstrap);
        p.browser_request = None;
        assert_eq!(p.validate().unwrap_err(), ProbeError::InvalidBootstrap);
    }

    #[tokio::test]
    async fn browser_comparison_preserves_the_validated_lookup_body() {
        let body = lookup_request();
        let (endpoint, server) = mock_server(b"HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf\r\nContent-Length: 4\r\nConnection: close\r\n\r\n[[]]".to_vec()).await;
        let mut p = proof();
        p.kind = "gaia_lookup_browser_request".into();
        p.service_cookie = Some("SID=synthetic".into());
        p.browser_request = Some(body.clone());
        query_with_body(
            &client(false).unwrap(),
            &endpoint,
            p.validate().unwrap(),
            None,
            &body,
        )
        .await
        .unwrap();
        let request = server.await.unwrap();
        let offset = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert!(
            std::str::from_utf8(&request)
                .unwrap()
                .contains("referer: https://messages.google.com/\r\n")
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&request[offset..]).unwrap(),
            body
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
        // The browser capture includes YC field 4 alongside its source list.
        // Neither the adjacent identity records nor that extra field is exposed.
        assert_eq!(
            parse_response(
                br#"[[],null,[[[3,"identity","GDitto"]],[["account"]],[["source"]],[null,[[],[]]]]]"#
            )
            .unwrap(),
            ProbeResult { sources: 1 }
        );
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
            b"[[],null,[null,null,[],null,null]]",
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
                json!([3,"PRIVATE_SERVER_TEXT", [{"@type":"type.googleapis.com/google.rpc.ErrorInfo","domain":"googleapis.com","reason":"API_KEY_HTTP_REFERRER_BLOCKED","metadata":{"key":"PRIVATE_KEY"}}]]),
                ProbeError::HttpErrorWithReason(
                    400,
                    RpcStatus::InvalidArgument,
                    RpcReason::ApiKeyHttpReferrerBlocked,
                ),
            ),
            (
                json!([3,"PRIVATE_SERVER_TEXT", [{"payload":"PRIVATE_COOKIE"}]]),
                ProbeError::HttpErrorWithStatus(400, RpcStatus::InvalidArgument),
            ),
            (
                json!(["16", "PRIVATE_SERVER_TEXT"]),
                ProbeError::HttpErrorWithStatus(400, RpcStatus::Unauthenticated),
            ),
            (json!([3, {}, []]), ProbeError::HttpError(400)),
            (
                json!([99, "PRIVATE_SERVER_TEXT"]),
                ProbeError::HttpError(400),
            ),
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
            let media = if body.is_array() {
                "text/plain"
            } else {
                "application/json"
            };
            let body = serde_json::to_vec(&body).unwrap();
            let header = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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

    #[test]
    fn structured_reasons_discard_unknown_values_and_private_metadata() {
        let info = json!({
            "@type": "type.googleapis.com/google.rpc.ErrorInfo",
            "domain": "googleapis.com",
            "reason": "API_KEY_INVALID",
            "metadata": {"key": "PRIVATE_KEY", "account": "PRIVATE_ACCOUNT"}
        });
        for body in [
            json!([3, "PRIVATE_MESSAGE", [info.clone()]]),
            json!({"error":{"status":"INVALID_ARGUMENT","details":[info.clone()]}}),
        ] {
            let projected = error_category(&serde_json::to_vec(&body).unwrap()).unwrap();
            assert_eq!(
                projected,
                (RpcStatus::InvalidArgument, Some(RpcReason::ApiKeyInvalid))
            );
            assert!(!format!("{projected:?}").contains("PRIVATE_"));
        }
        let mut invalid = info.clone();
        invalid["reason"] = json!("PRIVATE_MESSAGE");
        assert_eq!(error_reason(Some(&json!([invalid]))), None);
        let mut wrong_domain = info.clone();
        wrong_domain["domain"] = json!("untrusted.test");
        assert_eq!(error_reason(Some(&json!([wrong_domain]))), None);
        let mut wrong_type = info.clone();
        wrong_type["@type"] = json!("type.googleapis.com/unrecognized.ErrorInfo");
        assert_eq!(error_reason(Some(&json!([wrong_type]))), None);
        assert_eq!(error_reason(Some(&json!(vec![info.clone(); 17]))), None);
        let mut conflict = info.clone();
        conflict["reason"] = json!("SERVICE_DISABLED");
        assert_eq!(error_reason(Some(&json!([info, conflict]))), None);
    }

    #[test]
    fn jspb_any_reasons_project_only_known_infrastructure_errors() {
        use base64::Engine;
        // Standard ErrorInfo wire fields 1 and 2. Field 3 remains opaque.
        let mut bytes = b"\x0a\x0fAPI_KEY_INVALID\x12\x0egoogleapis.com".to_vec();
        bytes.extend_from_slice(b"\x1a\x0bPRIVATE_KEY");
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        for value in [
            json!(encoded),
            json!(base64::engine::general_purpose::STANDARD_NO_PAD.encode(&bytes)),
            json!([
                "API_KEY_INVALID",
                "googleapis.com",
                [["key", "PRIVATE_KEY"]]
            ]),
        ] {
            let body = json!([
                3,
                "PRIVATE_MESSAGE",
                [["type.googleapis.com/google.rpc.ErrorInfo", value]]
            ]);
            assert_eq!(
                error_category(&serde_json::to_vec(&body).unwrap()),
                Some((RpcStatus::InvalidArgument, Some(RpcReason::ApiKeyInvalid)))
            );
        }
        for detail in [
            json!(["unrecognized.Type", "PRIVATE_MESSAGE"]),
            json!(["type.googleapis.com/google.rpc.ErrorInfo", "%%%"]),
            json!([
                "type.googleapis.com/google.rpc.ErrorInfo",
                "A".repeat(16385)
            ]),
            json!([
                "type.googleapis.com/google.rpc.ErrorInfo",
                ["PRIVATE_REASON", "googleapis.com"]
            ]),
            json!([
                "type.googleapis.com/google.rpc.ErrorInfo",
                ["API_KEY_INVALID", "other.domain"]
            ]),
            json!([
                "type.googleapis.com/google.rpc.ErrorInfo",
                ["API_KEY_INVALID", "googleapis.com", null, "PRIVATE_EXTRA"]
            ]),
            json!(["type.googleapis.com/google.rpc.ErrorInfo", "CgVh"]),
        ] {
            assert_eq!(error_reason(Some(&json!([detail]))), None);
        }
        assert_eq!(
            format!(
                "{:?}",
                ErrorInfoProjection {
                    reason: "PRIVATE_REASON".into(),
                    domain: "PRIVATE_DOMAIN".into()
                }
            ),
            "ErrorInfoProjection { redacted }"
        );
    }
}
