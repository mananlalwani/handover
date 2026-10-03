use super::*;
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
    EndReceive,
    RejectFinalAck,
    CancelOnPrompt,
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
struct Send {
    #[prost(message, optional, tag = "2")]
    message: Option<Outer>,
}
#[derive(Message)]
struct Outer {
    #[prost(bytes = "vec", tag = "12")]
    payload: Vec<u8>,
}
#[derive(Message)]
struct Wrapper {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(int32, tag = "2")]
    kind: i32,
    #[prost(bytes = "vec", tag = "3")]
    body: Vec<u8>,
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
}
#[derive(Message)]
struct Ack {
    #[prost(string, repeated, tag = "2")]
    ids: Vec<String>,
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

async fn mock(scenario: Scenario) -> (PairingHttp, tokio::task::JoinHandle<Option<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let (path, _) = request(&mut socket).await;
        assert_eq!(path, crate::SIGN_IN_PATH);
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
        let (path, _) = request(&mut receive_socket).await;
        assert_eq!(
            path,
            crate::RECEIVE_MESSAGES_PATH,
            "receive must open before sending"
        );
        receive_socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        if matches!(scenario, Scenario::EndReceive) {
            chunk(&mut receive_socket, b"[[],[16]]").await;
            receive_socket.write_all(b"0\r\n\r\n").await.unwrap();
            return None;
        }
        chunk(&mut receive_socket, b"[[").await;
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
        let wrapper = Wrapper::decode(
            Send::decode(body.as_slice())
                .unwrap()
                .message
                .unwrap()
                .payload
                .as_slice(),
        )
        .unwrap();
        assert_eq!(wrapper.kind, 44);
        let init = Handshake::decode(wrapper.body.as_slice()).unwrap();
        let mut rng = StdRng::from_entropy();
        let peer = Ukey2ServerStage1::<RustCryptoImpl<StdRng>>::from(
            HashSet::from(["AES_256_CBC-HMAC_SHA256".to_owned()]),
            HandshakeImplementation::PublicKeyInProtobuf,
        )
        .advance_state(&mut rng, &init.bytes)
        .unwrap();
        response(&mut socket, "application/x-protobuf", b"").await;
        // A stale reply must not consume this attempt or become an ACK.
        let stale = record(
            "stale-inbox",
            Response {
                id: "unrelated-request".into(),
                kind: 44,
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
                    body: InitialResponse {
                        confirmation: true,
                        pairing_id: init.pairing_id.clone(),
                        bytes: peer.server_init_msg().to_vec(),
                        auth_revision: 1,
                        key_revision: 1,
                    }
                    .encode_to_vec(),
                },
            ),
        )
        .await;
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
        assert_eq!(Ack::decode(body.as_slice()).unwrap().ids, ["initial-inbox"]);
        response(&mut socket, "application/x-protobuf", b"").await;
        let (mut socket, _) = listener.accept().await.unwrap();
        let (path, body) = request(&mut socket).await;
        assert_eq!(path, crate::SEND_MESSAGE_PATH);
        let wrapper = Wrapper::decode(
            Send::decode(body.as_slice())
                .unwrap()
                .message
                .unwrap()
                .payload
                .as_slice(),
        )
        .unwrap();
        assert_eq!(wrapper.kind, 45);
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
        response(&mut socket, "application/x-protobuf", b"").await;
        chunk(&mut receive_socket, b",").await;
        chunk(
            &mut receive_socket,
            &record(
                "final-inbox",
                Response {
                    id: wrapper.id,
                    kind: 45,
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
            assert_eq!(Ack::decode(body.as_slice()).unwrap().ids, ["final-inbox"]);
            if matches!(scenario, Scenario::RejectFinalAck) {
                socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                assert!(
                    tokio::time::timeout(Duration::from_millis(50), listener.accept())
                        .await
                        .is_err(),
                    "failed ACKs are never retried automatically"
                );
            } else {
                response(&mut socket, "application/x-protobuf", b"").await;
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
    let (transport, server) = mock(scenario).await;
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().join("sessions"));
    let registration = registration();
    registration.persist_pending(&store).unwrap();
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
            progress.push(event);
            if matches!(scenario, Scenario::CancelOnPrompt) {
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
    } = result.unwrap()
    else {
        panic!("phone did not confirm");
    };
    assert_eq!(progress, [LoginProgress::Verification(expected.unwrap())]);
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
        assert_eq!(result.unwrap_err().code(), expected);
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
    assert_eq!(progress.len(), 1);
}

#[tokio::test]
async fn canceled_prompt_closes_receive_before_ack_or_final_send() {
    let (result, progress, _) = exercise(Scenario::CancelOnPrompt, true).await;
    assert!(matches!(result, Err(ProbeError::NativeError)));
    assert_eq!(progress.len(), 1);
}
