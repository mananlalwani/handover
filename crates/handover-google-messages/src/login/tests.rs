use super::*;
use base64::engine::general_purpose::STANDARD;
use crypto_provider_rustcrypto::RustCryptoImpl;
use prost::Message;
use rand::{SeedableRng, rngs::StdRng};
use serde_json::{Value, json};
use std::collections::HashSet;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use ukey2_rs::{HandshakeImplementation, StateMachine, Ukey2ServerStage1};

fn registration() -> UnpairedRegistration {
    crate::registration::RegistrationAttempt::prepare()
        .unwrap()
        .accept_response(
            br#"[[],"c3ludGhldGljLWlk",null,["c3ludGhldGljLXRva2Vu","3600000000"]]"#,
            Duration::from_secs(3600),
        )
        .unwrap()
}

fn proof(kind: &str) -> BrowserProof {
    BrowserProof {
        kind: kind.into(),
        endpoint: crate::ENDPOINTS[0].into(),
        origin: crate::ORIGIN_VALUE.into(),
        authorization: "Bearer synthetic".into(),
        api_key: "synthetic-key".into(),
        auth_user: Some("0".into()),
        service_cookie: Some("SID=synthetic".into()),
        account_email: Some("person@example.test".into()),
        browser_request: None,
    }
}

fn bundle(proof: &BrowserProof) -> String {
    let mut data = LOGIN_MAGIC.to_vec();
    data.extend(serde_json::to_vec(proof).unwrap());
    general_purpose::STANDARD.encode(data)
}

#[test]
fn bundle_binds_the_alias_and_rejects_invalid_credentials_before_network() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().join("sessions"));
    let registration = registration();
    registration.persist_pending(&store).unwrap();
    let account = registration.handover_account_id();
    let valid = bundle(&proof("gaia_login"));
    let login = LoginBootstrap::from_bundle(account, &valid, &store).unwrap();
    assert!(!login.start_pairing);
    assert_eq!(format!("{login:?}"), "LoginBootstrap { redacted }");
    assert!(matches!(
        LoginBootstrap::from_bundle("other", &valid, &store),
        Err(ProbeError::NoPendingRegistration)
    ));
    for invalid in [
        "".to_owned(),
        general_purpose::STANDARD.encode(b"opaque bytes"),
        "A".repeat(MAX_BUNDLE_BYTES + 1),
    ] {
        assert!(LoginBootstrap::from_bundle(account, &invalid, &store).is_err());
    }
    let mut invalid = proof("gaia_pairing_start");
    invalid.service_cookie = None;
    assert!(LoginBootstrap::from_bundle(account, &bundle(&invalid), &store).is_err());
    assert!(
        LoginBootstrap::from_bundle(account, &bundle(&proof("gaia_pairing_start")), &store)
            .unwrap()
            .start_pairing
    );
    // Nothing in proof validation rewrites a saved credential.
    assert_eq!(store.load_all().unwrap().len(), 1);
}

#[derive(Clone, Copy)]
enum Scenario {
    Success,
    ReadOnly,
    WrongAccount,
    RejectFinal,
    RejectSend,
    RejectReceive,
    EndReceive,
    RejectFinalAck,
    CancelOnPrompt,
    InactiveBootstrapReply,
    MatchedInvalidPayload,
}

fn source_body(wrong_account: bool) -> Vec<u8> {
    let web_id = if wrong_account {
        "another-web"
    } else {
        "synthetic-id"
    };
    serde_json::to_vec(&json!([
        [],
        null,
        [
            null,
            null,
            [
                [general_purpose::STANDARD.encode(web_id), null, 3],
                [
                    general_purpose::STANDARD.encode("synthetic-phone"),
                    null,
                    1,
                    null,
                    null,
                    null,
                    null,
                    general_purpose::STANDARD.encode([8, 1, 16, 1])
                ],
            ]
        ]
    ]))
    .unwrap()
}

// Independent test projections of only the fields needed by the mock phone.
#[derive(Message)]
struct Wrapper {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(int32, tag = "2")]
    kind: i32,
    #[prost(bytes = "vec", tag = "3")]
    body: Vec<u8>,
    #[prost(string, tag = "6")]
    session_id: String,
}
#[derive(Message)]
struct Handshake {
    #[prost(string, tag = "1")]
    pairing_id: String,
    #[prost(bytes = "vec", tag = "4")]
    bytes: Vec<u8>,
}
#[derive(Message)]
struct InitialResponse {
    #[prost(bool, tag = "3")]
    confirmation: bool,
    #[prost(string, tag = "4")]
    pairing_id: String,
    #[prost(bytes = "vec", tag = "5")]
    bytes: Vec<u8>,
    #[prost(int32, tag = "6")]
    auth_revision: i32,
    #[prost(int32, tag = "7")]
    key_revision: i32,
}
#[derive(Message)]
struct FinalResponse {
    #[prost(int32, tag = "1")]
    status: i32,
    #[prost(string, tag = "4")]
    pairing_id: String,
}
#[derive(Message)]
struct Response {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(int32, tag = "4")]
    kind: i32,
    #[prost(bytes = "vec", tag = "5")]
    body: Vec<u8>,
    #[prost(bool, tag = "9")]
    inactive: bool,
    #[prost(bool, tag = "6")]
    counted_response: bool,
    #[prost(int32, tag = "7")]
    response_count: i32,
}

async fn request(socket: &mut TcpStream) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut byte = [0];
        assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
        bytes.push(byte[0]);
        assert!(bytes.len() <= 64 * 1024);
        if bytes.ends_with(b"\r\n\r\n") {
            break bytes.len();
        }
    };
    let header = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let length: usize = header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .unwrap();
    assert!(length <= 64 * 1024);
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await.unwrap();
    (
        header
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_owned(),
        body,
    )
}

async fn response(socket: &mut TcpStream, content_type: &str, body: &[u8]) {
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
    socket.write_all(body).await.unwrap();
}

async fn chunk(socket: &mut TcpStream, body: &[u8]) {
    socket
        .write_all(format!("{:x}\r\n", body.len()).as_bytes())
        .await
        .unwrap();
    socket.write_all(body).await.unwrap();
    socket.write_all(b"\r\n").await.unwrap();
}

fn record(id: &str, response: Response) -> Vec<u8> {
    let mut message = vec![Value::Null; 17];
    message[0] = json!(id);
    message[1] = json!(19);
    message[11] = json!(general_purpose::STANDARD.encode(response.encode_to_vec()));
    message[16] = json!(general_purpose::STANDARD.encode("synthetic-phone"));
    serde_json::to_vec(&json!([[], message])).unwrap()
}

async fn mock(
    scenario: Scenario,
    expected_device: Value,
) -> (PairingHttp, tokio::task::JoinHandle<Option<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let (path, body) = request(&mut socket).await;
        assert_eq!(path, crate::SIGN_IN_PATH);
        let lookup: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(lookup[1], expected_device);
        assert_eq!(lookup[2], 1);
        response(
            &mut socket,
            "application/json+protobuf",
            &source_body(matches!(scenario, Scenario::WrongAccount)),
        )
        .await;
        if matches!(scenario, Scenario::ReadOnly | Scenario::WrongAccount) {
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err()
            );
            return None;
        }
        let (mut receive_socket, _) = listener.accept().await.unwrap();
        let (path, receive_body) = request(&mut receive_socket).await;
        let receive_request: Value = serde_json::from_slice(&receive_body).unwrap();
        assert_eq!(
            receive_request[0][5],
            general_purpose::STANDARD.encode("synthetic-token")
        );
        assert_eq!(
            path,
            crate::RECEIVE_MESSAGES_PATH,
            "receive must open before sending"
        );
        if matches!(scenario, Scenario::RejectReceive) {
            receive_socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json+protobuf\r\nContent-Length: 26\r\nConnection: close\r\n\r\n[16,\"PRIVATE_SERVER_TEXT\"]").await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err(),
                "a rejected receive channel must prevent the initial send"
            );
            return None;
        }
        receive_socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        if matches!(scenario, Scenario::EndReceive) {
            chunk(&mut receive_socket, b"[[],[16]]").await;
            receive_socket.write_all(b"0\r\n\r\n").await.unwrap();
            return None;
        }
        // A receive record with no selected event is a no-op, not failure.
        chunk(&mut receive_socket, b"[[[],").await;
        let (mut socket, _) = listener.accept().await.unwrap();
        let (path, body) = request(&mut socket).await;
        assert_eq!(path, crate::SEND_MESSAGE_PATH);
        if matches!(scenario, Scenario::RejectSend) {
            socket.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err(),
                "failed sends are never retried"
            );
            return None;
        }
        let sent: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            sent[2][5],
            general_purpose::STANDARD.encode("synthetic-token")
        );
        assert_eq!(sent[0], json!([16, "person@example.test", "GDitto"]));
        assert_eq!(
            sent[8],
            json!([general_purpose::STANDARD.encode("synthetic-phone")])
        );
        let wrapper = Wrapper::decode(
            general_purpose::STANDARD
                .decode(
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap()[1][11]
                        .as_str()
                        .unwrap(),
                )
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        assert_eq!(wrapper.kind, 44);
        assert!(wrapper.session_id.is_empty());
        let init = Handshake::decode(wrapper.body.as_slice()).unwrap();
        let mut rng = StdRng::from_entropy();
        let peer = Ukey2ServerStage1::<RustCryptoImpl<StdRng>>::from(
            HashSet::from(["AES_256_CBC-HMAC_SHA256".to_owned()]),
            HandshakeImplementation::PublicKeyInProtobuf,
        )
        .advance_state(&mut rng, &init.bytes)
        .unwrap();
        response(&mut socket, "application/json+protobuf", b"[]").await;
        // A stale reply must not consume this attempt or become an ACK.
        let stale = record(
            "stale-inbox",
            Response {
                id: "unrelated-request".into(),
                kind: 44,
                inactive: true,
                counted_response: true,
                response_count: 2,
                body: vec![1],
            },
        );
        chunk(&mut receive_socket, &stale).await;
        chunk(&mut receive_socket, b",").await;
        chunk(
            &mut receive_socket,
            &record(
                "initial-inbox",
                Response {
                    id: wrapper.id,
                    kind: 44,
                    inactive: matches!(scenario, Scenario::InactiveBootstrapReply),
                    counted_response: true,
                    response_count: 2,
                    body: if matches!(scenario, Scenario::MatchedInvalidPayload) {
                        Vec::new()
                    } else {
                        InitialResponse {
                            confirmation: true,
                            pairing_id: init.pairing_id.clone(),
                            bytes: peer.server_init_msg().to_vec(),
                            auth_revision: 1,
                            key_revision: 1,
                        }
                        .encode_to_vec()
                    },
                },
            ),
        )
        .await;
        if matches!(scenario, Scenario::MatchedInvalidPayload) {
            let mut byte = [0];
            assert_eq!(receive_socket.read(&mut byte).await.unwrap(), 0);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err(),
                "preemption must not be acknowledged or trigger confirmation"
            );
            return None;
        }
        if matches!(scenario, Scenario::CancelOnPrompt) {
            let mut byte = [0];
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), receive_socket.read(&mut byte))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err()
            );
            return None;
        }
        let (mut socket, _) = listener.accept().await.unwrap();
        let (path, body) = request(&mut socket).await;
        assert_eq!(path, crate::ACK_MESSAGES_PATH);
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()[1],
            json!(["initial-inbox"])
        );
        response(&mut socket, "application/json+protobuf", b"[]").await;
        let (mut socket, _) = listener.accept().await.unwrap();
        let (path, body) = request(&mut socket).await;
        assert_eq!(path, crate::SEND_MESSAGE_PATH);
        let sent: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            sent[2][5],
            general_purpose::STANDARD.encode("synthetic-token")
        );
        assert_eq!(sent[0], json!([16, "person@example.test", "GDitto"]));
        assert_eq!(
            sent[8],
            json!([general_purpose::STANDARD.encode("synthetic-phone")])
        );
        let wrapper = Wrapper::decode(
            general_purpose::STANDARD
                .decode(
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap()[1][11]
                        .as_str()
                        .unwrap(),
                )
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        assert_eq!(wrapper.kind, 45);
        assert!(wrapper.session_id.is_empty());
        let finish = Handshake::decode(wrapper.body.as_slice()).unwrap();
        assert_eq!(finish.pairing_id, init.pairing_id);
        let peer = peer.advance_state(&mut rng, &finish.bytes).unwrap();
        let auth = peer
            .completed_handshake()
            .auth_string::<RustCryptoImpl<StdRng>>()
            .derive_array::<32>()
            .unwrap();
        let expected_symbol = crate::pairing::verification::revision_one(&auth)
            .as_str()
            .to_owned();
        response(&mut socket, "application/json+protobuf", b"[]").await;
        chunk(&mut receive_socket, b",").await;
        chunk(
            &mut receive_socket,
            &record(
                "final-inbox",
                Response {
                    id: wrapper.id,
                    kind: 45,
                    inactive: false,
                    counted_response: false,
                    response_count: 1,
                    body: FinalResponse {
                        status: i32::from(matches!(scenario, Scenario::RejectFinal)),
                        pairing_id: finish.pairing_id,
                    }
                    .encode_to_vec(),
                },
            ),
        )
        .await;
        if matches!(scenario, Scenario::RejectFinal) {
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err(),
                "rejected phone replies are not acknowledged"
            );
        } else {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (path, body) = request(&mut socket).await;
            assert_eq!(path, crate::ACK_MESSAGES_PATH);
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap()[1],
                json!(["final-inbox"])
            );
            if matches!(scenario, Scenario::RejectFinalAck) {
                socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                assert!(
                    tokio::time::timeout(Duration::from_millis(50), listener.accept())
                        .await
                        .is_err(),
                    "failed ACKs are never retried automatically"
                );
            } else {
                response(&mut socket, "application/json+protobuf", b"[]").await;
            }
        }
        // Ending a ceremony cancels receive instead of leaving an orphan.
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), receive_socket.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        Some(expected_symbol)
    });
    let http = PairingHttp {
        short: crate::client(false).unwrap(),
        stream: crate::client(false).unwrap(),
        endpoint,
    };
    (http, server)
}

async fn exercise(
    scenario: Scenario,
    start_pairing: bool,
) -> (
    Result<LoginOutcome, ProbeError>,
    Vec<LoginProgress>,
    Option<String>,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().join("sessions"));
    let registration = registration();
    registration.persist_pending(&store).unwrap();
    let (transport, server) = mock(scenario, registration.lookup_request()[1].clone()).await;
    let login = LoginBootstrap {
        proof: proof("gaia_pairing"),
        registration,
        start_pairing,
        store: store.clone(),
    };
    let mut progress = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        login.run_with_transport(transport, |event| {
            let verification = matches!(event, LoginProgress::Verification(_));
            progress.push(event);
            if matches!(scenario, Scenario::CancelOnPrompt) && verification {
                return Err(ProbeError::NativeError);
            }
            Ok(())
        }),
    )
    .await
    .expect("local ceremony must finish");
    let expected = tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    if let Ok(LoginOutcome::PhoneConfirmed { pairing, .. }) = &result {
        let restored = ConfirmedPairing::restore_all(&store).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].account_id(), pairing.account_id());
        let encrypted = pairing.encrypt(b"synthetic restart test").unwrap();
        assert_eq!(
            restored[0]
                .pairing
                .decrypt_payload(encrypted.as_bytes())
                .unwrap()
                .as_bytes(),
            b"synthetic restart test"
        );
        let session = crate::session::RecoveredSession::from_credentials(
            ConfirmedPairing::restore_all(&store).unwrap().remove(0),
            proof("gaia_pairing"),
        )
        .unwrap();
        assert_eq!(format!("{session:?}"), "RecoveredSession { redacted }");
        let activation = session.prepare_activation().unwrap();
        assert_eq!(format!("{activation:?}"), "SessionRequest { redacted }");
        let body: Value = serde_json::from_slice(activation.body().unwrap()).unwrap();
        assert!(body[4].is_null(), "activation must omit delivery TTL");
        let wrapper = STANDARD.decode(body[1][11].as_str().unwrap()).unwrap();
        let wrapper = ActivationWrapper::decode(wrapper.as_slice()).unwrap();
        assert_eq!(wrapper.action, 16);
        assert_eq!(wrapper.request_id, wrapper.session_id);
        assert!(uuid::Uuid::parse_str(&wrapper.session_id).is_ok());
        let payload = pairing.pairing.decrypt_payload(&wrapper.encrypted).unwrap();
        let payload = ActivationTimestamp::decode(payload.as_bytes()).unwrap();
        assert!(payload.millis > 0);
        let next = session.prepare_activation().unwrap();
        assert_ne!(
            activation.body().unwrap(),
            next.body().unwrap(),
            "encryption must have fresh counters"
        );
        for (_, record) in store.confirmed_store().unwrap().load_all().unwrap() {
            for secret in [
                b"person@example.test".as_slice(),
                b"SID=synthetic",
                b"Bearer synthetic",
            ] {
                assert!(
                    !record
                        .as_bytes()
                        .windows(secret.len())
                        .any(|window| window == secret)
                );
            }
        }
        assert!(
            LoginBootstrap::from_bundle(
                pairing.account_id(),
                &bundle(&proof("gaia_pairing_start")),
                &store
            )
            .is_err()
        );
        restored[0].forget(&store).unwrap();
        assert!(ConfirmedPairing::restore_all(&store).unwrap().is_empty());
        assert!(
            UnpairedRegistration::restore_all_pending(&store)
                .unwrap()
                .is_empty()
        );
    } else {
        assert!(ConfirmedPairing::restore_all(&store).unwrap().is_empty());
    }
    (result, progress, expected)
}

#[tokio::test]
async fn pairing_runs_over_http_with_a_mock_phone_and_only_validated_acknowledgements() {
    let (result, progress, expected) = exercise(Scenario::Success, true).await;
    let LoginOutcome::PhoneConfirmed {
        pairing: confirmed,
        acknowledgement_accepted: true,
        ..
    } = result.unwrap()
    else {
        panic!("phone did not confirm");
    };
    assert_eq!(
        progress,
        [
            LoginProgress::RegistrationVerified,
            LoginProgress::InitialSendAccepted,
            LoginProgress::Verification(expected.unwrap()),
            LoginProgress::InitialAcknowledgementAccepted,
            LoginProgress::FinalSendAccepted
        ]
    );
    assert!(
        !confirmed
            .encrypt(b"synthetic offline payload")
            .unwrap()
            .as_bytes()
            .is_empty()
    );
    assert_eq!(format!("{confirmed:?}"), "ConfirmedPairing { redacted }");
}

#[tokio::test]
async fn readiness_only_reads_and_another_account_never_opens_receive() {
    let (result, progress, _) = exercise(Scenario::ReadOnly, false).await;
    assert!(matches!(result, Ok(LoginOutcome::Ready)));
    assert_eq!(progress, [LoginProgress::Ready]);
    let (result, progress, _) = exercise(Scenario::WrongAccount, true).await;
    assert!(matches!(
        result,
        Err(ProbeError::RegistrationAccountMismatch)
    ));
    assert!(progress.is_empty());
}

#[tokio::test]
async fn rejection_and_stream_failure_never_create_confirmed_state_or_retry() {
    for (scenario, expected) in [
        (Scenario::RejectFinal, "pairing_failed"),
        (Scenario::RejectSend, "http_error"),
        (Scenario::EndReceive, "receive_failed"),
    ] {
        let (result, _, _) = exercise(scenario, true).await;
        let code = result.unwrap_err().code();
        if matches!(scenario, Scenario::EndReceive) {
            // This peer closes the receive stream and its listener together.
            // Either receive EOF or the concurrent send connection can fail first.
            assert!(matches!(code, "receive_failed" | "rpc_error" | "network"));
        } else {
            assert_eq!(code, expected);
        }
    }
}
#[tokio::test]
async fn final_ack_failure_keeps_confirmed_keys_and_prevents_repairing() {
    let (result, progress, _) = exercise(Scenario::RejectFinalAck, true).await;
    assert!(matches!(
        result,
        Ok(LoginOutcome::PhoneConfirmed {
            acknowledgement_accepted: false,
            ..
        })
    ));
    assert_eq!(progress.len(), 5);
}

#[tokio::test]
async fn canceled_prompt_closes_receive_before_ack_or_final_send() {
    let (result, progress, _) = exercise(Scenario::CancelOnPrompt, true).await;
    assert!(matches!(result, Err(ProbeError::NativeError)));
    assert_eq!(progress.len(), 3);
}

#[tokio::test]
async fn rejected_receive_preserves_http_status_and_prevents_send() {
    let (result, _, _) = exercise(Scenario::RejectReceive, true).await;
    assert_eq!(
        result.unwrap_err(),
        ProbeError::HttpErrorWithStatus(401, crate::RpcStatus::Unauthenticated)
    );
}

#[tokio::test]
async fn unpaired_inactive_reply_still_completes_authenticated_pairing() {
    let (result, _, _) = exercise(Scenario::InactiveBootstrapReply, true).await;
    assert!(result.is_ok(), "bootstrap has no active session to preempt");
}

#[tokio::test]
async fn correlated_invalid_payload_aborts_without_acknowledgement() {
    let (result, _, _) = exercise(Scenario::MatchedInvalidPayload, true).await;
    assert_eq!(
        result.unwrap_err(),
        ProbeError::ReceiveProtocol(ReceiveError::MissingPairingBody)
    );
}

#[derive(Message)]
struct ActivationWrapper {
    #[prost(string, tag = "1")]
    request_id: String,
    #[prost(int32, tag = "2")]
    action: i32,
    #[prost(bytes = "vec", tag = "5")]
    encrypted: Vec<u8>,
    #[prost(string, tag = "6")]
    session_id: String,
}
#[derive(Message)]
struct ActivationTimestamp {
    #[prost(int64, tag = "2")]
    millis: i64,
}

#[tokio::test]
async fn restored_startup_opens_receive_before_activation_and_rejects_wrong_account() {
    for (wrong, conversations, corrupt, paged, cycle, history) in [
        (false, false, false, false, false, false),
        (true, false, false, false, false, false),
        (false, true, false, false, false, false),
        (false, true, true, false, false, false),
        (false, true, false, true, false, false),
        (false, true, false, true, true, false),
        (false, true, false, false, false, true),
        (false, true, true, false, false, true),
    ] {
        let (outcome, _, _) = exercise(Scenario::Success, true).await;
        let LoginOutcome::PhoneConfirmed { pairing, .. } = outcome.unwrap() else {
            panic!("mock phone must confirm")
        };
        let mut pages = Vec::new();
        for index in 0..if paged { 2 } else { 1 } {
            let mut bytes = if history {
                synthetic_history_page()
            } else {
                synthetic_conversation_page(26 + index)
            };
            if paged && (index == 0 || cycle) {
                // First-party cursor: conversation ID field 1, timestamp field 2.
                bytes.extend_from_slice(&[0x2a, 8, 0x0a, 4, b'n', b'e', b'x', b't', 0x10, 1]);
            }
            let mut encrypted = pairing.encrypt(&bytes).unwrap().as_bytes().to_vec();
            if corrupt {
                encrypted[0] ^= 1;
            }
            pages.push(encrypted);
        }
        let session =
            crate::session::RecoveredSession::from_credentials(*pairing, proof("gaia_pairing"))
                .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (path, _) = request(&mut socket).await;
            assert_eq!(path, crate::SIGN_IN_PATH);
            response(
                &mut socket,
                "application/json+protobuf",
                &source_body(wrong),
            )
            .await;
            if wrong {
                assert!(
                    tokio::time::timeout(Duration::from_millis(100), listener.accept())
                        .await
                        .is_err()
                );
                return;
            }
            let (mut receive, _) = listener.accept().await.unwrap();
            let (path, _) = request(&mut receive).await;
            assert_eq!(path, crate::RECEIVE_MESSAGES_PATH);
            receive.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
            chunk(&mut receive, b"[[[],").await;
            let (mut send, _) = listener.accept().await.unwrap();
            let (path, body) = request(&mut send).await;
            assert_eq!(path, crate::SEND_MESSAGE_PATH);
            let body: Value = serde_json::from_slice(&body).unwrap();
            let wrapper = ActivationWrapper::decode(
                STANDARD
                    .decode(body[1][11].as_str().unwrap())
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
            assert_eq!(wrapper.action, 16);
            assert!(body[4].is_null());
            response(&mut send, "application/json+protobuf", b"[]").await;
            if conversations {
                for (index, encrypted_page) in pages.into_iter().enumerate() {
                    let (mut list, _) = listener.accept().await.unwrap();
                    let (path, body) = request(&mut list).await;
                    assert_eq!(path, crate::SEND_MESSAGE_PATH);
                    let body: Value = serde_json::from_slice(&body).unwrap();
                    assert_eq!(body[1][22][1], if index == 0 && !history { 16 } else { 2 });
                    let wrapper = ActivationWrapper::decode(
                        STANDARD
                            .decode(body[1][11].as_str().unwrap())
                            .unwrap()
                            .as_slice(),
                    )
                    .unwrap();
                    assert_eq!(wrapper.action, if history { 2 } else { 1 });
                    response(&mut list, "application/json+protobuf", b"[]").await;
                    let rpc_reply = SessionResponse {
                        request_id: wrapper.request_id,
                        action: if history { 2 } else { 1 },
                        encrypted: encrypted_page,
                    };
                    let mut message = vec![Value::Null; 17];
                    message[0] = json!("session-reply");
                    message[1] = json!(19);
                    message[11] = json!(STANDARD.encode(rpc_reply.encode_to_vec()));
                    message[16] = json!(STANDARD.encode("synthetic-phone"));
                    if index > 0 {
                        chunk(&mut receive, b",").await;
                    }
                    chunk(
                        &mut receive,
                        &serde_json::to_vec(&json!([[], message])).unwrap(),
                    )
                    .await;
                    if corrupt || (cycle && index == 1) {
                        assert!(
                            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                                .await
                                .is_err(),
                            "invalid MAC must not be acknowledged"
                        );
                    } else {
                        let (mut ack, _) = listener.accept().await.unwrap();
                        let (path, body) = request(&mut ack).await;
                        assert_eq!(path, crate::ACK_MESSAGES_PATH);
                        let body: Value = serde_json::from_slice(&body).unwrap();
                        assert_eq!(body[1], json!(["session-reply"]));
                        response(&mut ack, "application/json+protobuf", b"[]").await;
                    }
                }
            }
            let mut byte = [0];
            assert_eq!(
                receive.read(&mut byte).await.unwrap(),
                0,
                "probe must cancel receive before returning"
            );
        });
        let fixture = synthetic_conversation_page(1);
        let model = crate::conversation::decode(session.account_id(), &fixture[2..]).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            if history {
                session
                    .history_at(&endpoint, crate::client(false).unwrap(), &model)
                    .await
                    .map(|page| Some(page.messages.len()))
            } else {
                session
                    .probe_startup_at(
                        &endpoint,
                        crate::client(false).unwrap(),
                        crate::client(false).unwrap(),
                        conversations,
                    )
                    .await
                    .map(|page| page.map(|page| page.len()))
            }
        })
        .await
        .unwrap();
        if wrong {
            assert!(matches!(
                result,
                Err(ProbeError::RegistrationAccountMismatch)
            ));
        } else if corrupt {
            assert!(matches!(result, Err(ProbeError::ReceiveFailed)));
        } else if cycle {
            assert!(matches!(
                result,
                Err(ProbeError::SessionProtocol(
                    crate::session::SessionError::ConversationCursor
                ))
            ));
        } else {
            assert_eq!(
                result.unwrap(),
                conversations.then_some(if history {
                    1
                } else if paged {
                    27
                } else {
                    26
                })
            );
        }
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }
}

#[derive(Message)]
struct SessionResponse {
    #[prost(string, tag = "1")]
    request_id: String,
    #[prost(int32, tag = "4")]
    action: i32,
    #[prost(bytes = "vec", tag = "8")]
    encrypted: Vec<u8>,
}

fn synthetic_conversation_page(count: usize) -> Vec<u8> {
    let mut page = Vec::new();
    for index in 0..count {
        let id = format!("thread-{index}");
        let mut record = vec![0x0a, id.len() as u8];
        record.extend(id.as_bytes());
        record.extend([0xa2, 0x01, 7, 0x0a, 5, 0x08, 1, 0x12, 1, b'p']);
        page.extend([0x12, record.len() as u8]);
        page.extend(record);
    }
    page
}

fn synthetic_history_page() -> Vec<u8> {
    #[derive(prost::Message)]
    struct Text {
        #[prost(string, tag = "1")]
        text: String,
    }
    #[derive(prost::Message)]
    struct Part {
        #[prost(message, optional, tag = "2")]
        text: Option<Text>,
    }
    #[derive(prost::Message)]
    struct Status {
        #[prost(int32, tag = "2")]
        code: i32,
    }
    #[derive(prost::Message)]
    struct Record {
        #[prost(message, optional, tag = "4")]
        status: Option<Status>,
        #[prost(string, tag = "1")]
        id: String,
        #[prost(string, tag = "7")]
        conversation: String,
        #[prost(string, tag = "9")]
        sender: String,
        #[prost(message, repeated, tag = "10")]
        parts: Vec<Part>,
    }
    #[derive(prost::Message)]
    struct Page {
        #[prost(message, repeated, tag = "2")]
        records: Vec<Record>,
    }
    Page {
        records: vec![Record {
            status: Some(Status { code: 100 }),
            id: "message".into(),
            conversation: "thread-0".into(),
            sender: "fixture-peer".into(),
            parts: vec![Part {
                text: Some(Text {
                    text: "Synthetic content".into(),
                }),
            }],
        }],
    }
    .encode_to_vec()
}

#[tokio::test]
async fn update_observer_validates_pushes_and_closes_without_acknowledging() {
    for (corrupt, foreign, inactive, reject_callback, foreign_session) in [
        (false, false, false, false, false),
        (true, false, false, false, false),
        (false, true, false, false, false),
        (false, false, true, false, false),
        (false, false, false, true, false),
        (false, false, false, false, true),
    ] {
        let (outcome, _, _) = exercise(Scenario::Success, true).await;
        let LoginOutcome::PhoneConfirmed { pairing, .. } = outcome.unwrap() else {
            panic!("pairing");
        };
        let mut payload = vec![0x32, 2, 0x10, if inactive { 1 } else { 2 }];
        if !inactive {
            payload = vec![0x1a];
            let messages = synthetic_history_page();
            prost::encoding::encode_varint(messages.len() as u64, &mut payload);
            payload.extend_from_slice(&messages);
        }
        let mut encrypted = pairing.encrypt(&payload).unwrap().as_bytes().to_vec();
        if corrupt {
            encrypted[0] ^= 1;
        }
        let session =
            crate::session::RecoveredSession::from_credentials(*pairing, proof("gaia_pairing"))
                .unwrap();
        let fixture = synthetic_conversation_page(1);
        let model = crate::conversation::decode(session.account_id(), &fixture[2..]).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut lookup, _) = listener.accept().await.unwrap();
            assert_eq!(request(&mut lookup).await.0, crate::SIGN_IN_PATH);
            response(
                &mut lookup,
                "application/json+protobuf",
                &source_body(false),
            )
            .await;
            let (mut receive, _) = listener.accept().await.unwrap();
            assert_eq!(request(&mut receive).await.0, crate::RECEIVE_MESSAGES_PATH);
            receive.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
            chunk(&mut receive, b"[[[],").await;
            let (mut activation, _) = listener.accept().await.unwrap();
            let (path, body) = request(&mut activation).await;
            assert_eq!(path, crate::SEND_MESSAGE_PATH);
            let body: Value = serde_json::from_slice(&body).unwrap();
            let wrapper = ActivationWrapper::decode(
                STANDARD
                    .decode(body[1][11].as_str().unwrap())
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
            assert_eq!(wrapper.action, 16);
            response(&mut activation, "application/json+protobuf", b"[]").await;
            let rpc = SessionResponse {
                request_id: if foreign_session {
                    "foreign-session".into()
                } else {
                    wrapper.request_id
                },
                action: 16,
                encrypted,
            };
            let mut message = vec![Value::Null; 17];
            message[0] = json!("push-id");
            message[1] = json!(19);
            message[11] = json!(STANDARD.encode(rpc.encode_to_vec()));
            message[16] = json!(STANDARD.encode(if foreign {
                "foreign-phone"
            } else {
                "synthetic-phone"
            }));
            chunk(
                &mut receive,
                &serde_json::to_vec(&json!([[], message])).unwrap(),
            )
            .await;
            let mut byte = [0];
            assert_eq!(
                receive.read(&mut byte).await.unwrap(),
                0,
                "observer must close receive"
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err(),
                "observer must not ACK or retry"
            );
        });
        let mut count = 0;
        let result = session
            .observe_updates_at(
                &endpoint,
                crate::client(false).unwrap(),
                crate::client(false).unwrap(),
                vec![model],
                Duration::from_millis(100),
                |update| {
                    count += 1;
                    if inactive {
                        assert!(matches!(update, crate::updates::Update::Inactive));
                    } else {
                        let crate::updates::Update::Messages(messages) = update else {
                            panic!("messages");
                        };
                        assert_eq!(messages.len(), 1);
                        assert_eq!(messages[0].text.as_deref(), Some("Synthetic content"));
                    }
                    if reject_callback {
                        Err(ProbeError::NativeError)
                    } else {
                        Ok(())
                    }
                },
            )
            .await;
        assert_eq!(count, usize::from(!corrupt && !foreign && !foreign_session));
        if corrupt || inactive || reject_callback {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn live_session_shares_receive_with_history_and_requires_push_publication_before_ack() {
    use crate::session::{LiveCommand, LiveEvent};
    for reject_push in [false, true] {
        let (outcome, _, _) = exercise(Scenario::Success, true).await;
        let LoginOutcome::PhoneConfirmed { pairing, .. } = outcome.unwrap() else {
            panic!("pairing");
        };
        let inventory = pairing
            .encrypt(&synthetic_conversation_page(1))
            .unwrap()
            .as_bytes()
            .to_vec();
        let history = pairing
            .encrypt(&synthetic_history_page())
            .unwrap()
            .as_bytes()
            .to_vec();
        let presence = pairing
            .encrypt(&[0x3a, 3, 0x0a, 1, b'x'])
            .unwrap()
            .as_bytes()
            .to_vec();
        let active = pairing
            .encrypt(&[0x32, 2, 0x10, 2])
            .unwrap()
            .as_bytes()
            .to_vec();
        let mut payload = vec![0x1a];
        let records = synthetic_history_page();
        prost::encoding::encode_varint(records.len() as u64, &mut payload);
        payload.extend_from_slice(&records);
        let pushed = pairing.encrypt(&payload).unwrap().as_bytes().to_vec();
        let session =
            crate::session::RecoveredSession::from_credentials(*pairing, proof("gaia_pairing"))
                .unwrap();
        let fixture = synthetic_conversation_page(1);
        let model = crate::conversation::decode(session.account_id(), &fixture[2..]).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (checked_tx, checked_rx) = tokio::sync::oneshot::channel();
        let (presence_tx, presence_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut lookup, _) = listener.accept().await.unwrap();
            assert_eq!(request(&mut lookup).await.0, crate::SIGN_IN_PATH);
            response(
                &mut lookup,
                "application/json+protobuf",
                &source_body(false),
            )
            .await;
            let (mut receive, _) = listener.accept().await.unwrap();
            assert_eq!(request(&mut receive).await.0, crate::RECEIVE_MESSAGES_PATH);
            receive.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
            chunk(&mut receive, b"[[[],").await;
            let (mut activation, _) = listener.accept().await.unwrap();
            let (path, body) = request(&mut activation).await;
            assert_eq!(path, crate::SEND_MESSAGE_PATH);
            let wrapper = rpc_request(&body);
            assert_eq!(wrapper.action, 16);
            let session_id = wrapper.request_id;
            response(&mut activation, "application/json+protobuf", b"[]").await;
            let (mut list, _) = listener.accept().await.unwrap();
            let (path, body) = request(&mut list).await;
            assert_eq!(path, crate::SEND_MESSAGE_PATH);
            let wrapper = rpc_request(&body);
            assert_eq!(wrapper.action, 1);
            response(&mut list, "application/json+protobuf", b"[]").await;
            rpc_push(
                &mut receive,
                &wrapper.request_id,
                1,
                inventory,
                "inventory",
                false,
            )
            .await;
            expect_ack(&listener, "inventory").await;
            rpc_push(&mut receive, &session_id, 16, active, "active", true).await;
            expect_ack(&listener, "active").await;
            rpc_push(&mut receive, &session_id, 16, pushed, "message-push", true).await;
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err(),
                "a decoded push must wait for publication before ACK"
            );
            checked_tx.send(()).unwrap();
            if !reject_push {
                expect_ack(&listener, "message-push").await;
                let (mut read, _) = listener.accept().await.unwrap();
                let (path, body) = request(&mut read).await;
                assert_eq!(
                    path,
                    crate::SEND_MESSAGE_PATH,
                    "history must reuse receive, not open a second stream"
                );
                let wrapper = rpc_request(&body);
                assert_eq!(wrapper.action, 2);
                response(&mut read, "application/json+protobuf", b"[]").await;
                rpc_push(
                    &mut receive,
                    &wrapper.request_id,
                    2,
                    history,
                    "history-page",
                    true,
                )
                .await;
                expect_ack(&listener, "history-page").await;
                rpc_push(
                    &mut receive,
                    &session_id,
                    16,
                    presence,
                    "presence-check",
                    true,
                )
                .await;
                let (mut response_socket, _) = listener.accept().await.unwrap();
                let (path, body) = request(&mut response_socket).await;
                assert_eq!(path, crate::SEND_MESSAGE_PATH);
                assert_eq!(rpc_request(&body).action, 17);
                response(&mut response_socket, "application/json+protobuf", b"[]").await;
                expect_ack(&listener, "presence-check").await;
                presence_tx.send(()).unwrap();
            }
            let mut byte = [0];
            assert_eq!(
                receive.read(&mut byte).await.unwrap(),
                0,
                "session must close receive when its owner leaves"
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err(),
                "failed publication must not ACK or retry"
            );
        });
        let (commands, incoming) = tokio::sync::mpsc::channel(1);
        let (events, mut outgoing) = tokio::sync::mpsc::channel(1);
        let network = tokio::spawn(async move {
            session
                .run_live_at(
                    &endpoint,
                    crate::client(false).unwrap(),
                    crate::client(false).unwrap(),
                    incoming,
                    events,
                )
                .await
        });
        let inventory = outgoing.recv().await.unwrap();
        assert!(
            matches!(inventory.event, LiveEvent::Conversations(ref records) if records.len() == 1)
        );
        inventory.accepted.send(()).unwrap();
        let active = outgoing.recv().await.unwrap();
        assert!(matches!(active.event, LiveEvent::Online));
        active.accepted.send(()).unwrap();
        let message = outgoing.recv().await.unwrap();
        assert!(matches!(message.event, LiveEvent::Messages(ref records) if records.len() == 1));
        checked_rx.await.unwrap();
        if reject_push {
            drop(message.accepted);
        } else {
            message.accepted.send(()).unwrap();
            commands
                .send(LiveCommand::History {
                    conversation: Box::new(model),
                    cursor: None,
                    limit: 20,
                    fetch_id: Some(77),
                })
                .await
                .unwrap();
            let page = outgoing.recv().await.unwrap();
            assert!(
                matches!(page.event, LiveEvent::History { fetch_id: Some(77), ref page, .. } if page.messages.len() == 1)
            );
            page.accepted.send(()).unwrap();
            presence_rx.await.unwrap();
        }
        drop(commands);
        let result = tokio::time::timeout(Duration::from_secs(2), network)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.is_err(), reject_push);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}

fn rpc_request(body: &[u8]) -> ActivationWrapper {
    let body: Value = serde_json::from_slice(body).unwrap();
    ActivationWrapper::decode(
        STANDARD
            .decode(body[1][11].as_str().unwrap())
            .unwrap()
            .as_slice(),
    )
    .unwrap()
}
async fn rpc_push(
    stream: &mut TcpStream,
    request_id: &str,
    action: i32,
    encrypted: Vec<u8>,
    id: &str,
    comma: bool,
) {
    if comma {
        chunk(stream, b",").await;
    }
    let rpc = SessionResponse {
        request_id: request_id.into(),
        action,
        encrypted,
    };
    let mut message = vec![Value::Null; 17];
    message[0] = json!(id);
    message[1] = json!(19);
    message[11] = json!(STANDARD.encode(rpc.encode_to_vec()));
    message[16] = json!(STANDARD.encode("synthetic-phone"));
    chunk(stream, &serde_json::to_vec(&json!([[], message])).unwrap()).await;
}
async fn expect_ack(listener: &TcpListener, id: &str) {
    let (mut ack, _) = listener.accept().await.unwrap();
    let (path, body) = request(&mut ack).await;
    assert_eq!(path, crate::ACK_MESSAGES_PATH);
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body[1], json!([id]));
    response(&mut ack, "application/json+protobuf", b"[]").await;
}
