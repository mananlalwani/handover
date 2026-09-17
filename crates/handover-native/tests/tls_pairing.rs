//! Real TLS pairing interop tests for the native backend.
//!
//! These tests speak the exact wire protocol (4-byte big-endian length +
//! JSON, TLS 1.3 with mutual certificates) against [`NativeBackend::serve`]
//! on an ephemeral port, using an OpenSSL client that mirrors the Android
//! transport's ceremony: hello, code comparison, pair confirmation, battery.
//! DNS-SD advertisement is intentionally out of scope here; it has code-level
//! coverage in the crate's unit tests but no live-network verification.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use handover_core::{
    DeviceEvent, MediaEvent, NotificationEvent, ShareFailure, ShareStatus, StateEvent,
};
use handover_native::NativeBackend;
use openssl::asn1::Asn1Time;
use openssl::bn::{BigNum, MsbOption};
use openssl::ec::{EcGroup, EcKey};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Private};
use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
use openssl::x509::{X509, X509NameBuilder};
use sha2::{Digest, Sha256};

struct ClientIdentity {
    key: PKey<Private>,
    cert: X509,
    fingerprint: String,
}

fn test_identity() -> ClientIdentity {
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
    let key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
    let mut subject = X509NameBuilder::new().unwrap();
    subject
        .append_entry_by_text("CN", "Handover Test Client")
        .unwrap();
    let subject = subject.build();
    let mut cert = X509::builder().unwrap();
    cert.set_version(2).unwrap();
    let mut serial = BigNum::new().unwrap();
    serial.rand(128, MsbOption::MAYBE_ZERO, false).unwrap();
    cert.set_serial_number(serial.to_asn1_integer().unwrap().as_ref())
        .unwrap();
    cert.set_subject_name(&subject).unwrap();
    cert.set_issuer_name(&subject).unwrap();
    cert.set_pubkey(&key).unwrap();
    cert.set_not_before(Asn1Time::days_from_now(0).unwrap().as_ref())
        .unwrap();
    cert.set_not_after(Asn1Time::days_from_now(1).unwrap().as_ref())
        .unwrap();
    cert.sign(&key, MessageDigest::sha256()).unwrap();
    let cert = cert.build();
    let fingerprint = hex::encode(Sha256::digest(cert.to_der().unwrap()));
    ClientIdentity {
        key,
        cert,
        fingerprint,
    }
}

struct TlsPeer {
    tls: openssl::ssl::SslStream<TcpStream>,
    server_fingerprint: String,
}

fn connect(port: u16, identity: &ClientIdentity) -> TlsPeer {
    let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
    builder.set_certificate(&identity.cert).unwrap();
    builder.set_private_key(&identity.key).unwrap();
    builder.set_verify_callback(SslVerifyMode::PEER, |_preverified, _context| true);
    let connector = builder.build();
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let tls = connector.connect("handover-interop", stream).unwrap();
    let server_fingerprint = hex::encode(Sha256::digest(
        tls.ssl().peer_certificate().unwrap().to_der().unwrap(),
    ));
    TlsPeer {
        tls,
        server_fingerprint,
    }
}

fn send(peer: &mut TlsPeer, message: serde_json::Value) {
    let data = serde_json::to_vec(&message).unwrap();
    peer.tls
        .write_all(&(data.len() as u32).to_be_bytes())
        .unwrap();
    peer.tls.write_all(&data).unwrap();
    peer.tls.flush().unwrap();
}

fn recv(peer: &mut TlsPeer) -> serde_json::Value {
    let mut len = [0u8; 4];
    peer.tls.read_exact(&mut len).unwrap();
    let len = u32::from_be_bytes(len) as usize;
    assert!((1..=64 * 1024).contains(&len));
    let mut data = vec![0u8; len];
    peer.tls.read_exact(&mut data).unwrap();
    serde_json::from_slice(&data).unwrap()
}

fn recv_err(peer: &mut TlsPeer) {
    let mut len = [0u8; 4];
    assert!(peer.tls.read_exact(&mut len).is_err());
}

struct Harness {
    backend: NativeBackend,
    port: u16,
    events: mpsc::Receiver<StateEvent>,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let backend = NativeBackend::open(dir.path().to_path_buf()).unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    let server = backend.clone();
    std::thread::spawn(move || {
        let event = Arc::new(move |ev: StateEvent| {
            let _ = tx.send(ev);
        });
        let _ = server.serve(listener, event);
    });
    // Keep the state directory alive for the duration of the test.
    std::mem::forget(dir);
    Harness {
        backend,
        port,
        events: rx,
    }
}

fn wait_pending_code(harness: &Harness, id: &str) -> String {
    for _ in 0..100 {
        if let Some(candidate) = harness
            .backend
            .pending()
            .into_iter()
            .find(|p| p.id == id && !p.code.is_empty())
        {
            return candidate.code;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("peer {id} never reached the code stage of pairing");
}

/// v2 ceremony code: SHA-256 over the domain label plus the two certificate
/// fingerprints with each side's fresh nonce bound to its fingerprint owner,
/// truncated to eight decimal digits. The Android `pairingCode`
/// implementation must produce the identical value for the same inputs; the
/// shared cross-language vector lives in the crate unit tests.
fn comparison_code(own_fp: &str, own_nonce: &str, peer_fp: &str, peer_nonce: &str) -> String {
    let ((low_fp, low_nonce), (high_fp, high_nonce)) = if own_fp <= peer_fp {
        ((own_fp, own_nonce), (peer_fp, peer_nonce))
    } else {
        ((peer_fp, peer_nonce), (own_fp, own_nonce))
    };
    let digest = Sha256::digest(
        format!("handover-pair-v2:{low_fp}:{high_fp}:{low_nonce}:{high_nonce}").as_bytes(),
    );
    let number = u32::from_be_bytes(digest[..4].try_into().unwrap()) % 100_000_000;
    format!("{number:08}")
}

fn fresh_nonce() -> [u8; 16] {
    let mut nonce = [0u8; 16];
    openssl::rand::rand_bytes(&mut nonce).unwrap();
    nonce
}

/// Runs the opening phase of the ceremony: both sides commit in hello, then
/// reveal. Returns the ceremony code as the test client derives it.
fn exchange_openings(harness: &Harness, peer: &mut TlsPeer, client: &ClientIdentity) -> String {
    let nonce = fresh_nonce();
    let commit = hex::encode(Sha256::digest(nonce));
    send(
        peer,
        serde_json::json!({"type":"hello","protocol":1,"id":client.fingerprint,"name":"Test Phone","pair_commit":commit}),
    );
    let hello = recv(peer);
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["protocol"], 1);
    // The server must advertise the fingerprint of the certificate it
    // presented during the TLS handshake, not an unverified string.
    assert_eq!(hello["id"].as_str().unwrap(), peer.server_fingerprint);
    assert_eq!(hello["id"].as_str().unwrap(), harness.backend.identity());

    let opening = recv(peer);
    assert_eq!(opening["type"], "pair_open");
    let server_nonce = opening["nonce"].as_str().unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(hex::decode(server_nonce).unwrap())),
        hello["pair_commit"].as_str().unwrap()
    );
    send(
        peer,
        serde_json::json!({"type":"pair_open","protocol":1,"nonce":hex::encode(nonce)}),
    );
    comparison_code(
        &client.fingerprint,
        &hex::encode(nonce),
        &peer.server_fingerprint,
        server_nonce,
    )
}

#[test]
fn pairing_succeeds_over_real_tls() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);

    let derived = exchange_openings(&harness, &mut peer, &client);
    let code = wait_pending_code(&harness, &client.fingerprint);
    assert_eq!(code, derived);

    harness.backend.approve(&client.fingerprint, &code).unwrap();
    send(
        &mut peer,
        serde_json::json!({"type":"pair_confirm","protocol":1,"code":code}),
    );
    let paired = recv(&mut peer);
    assert_eq!(paired["type"], "paired");
    assert_eq!(harness.backend.peers().len(), 1);

    send(
        &mut peer,
        serde_json::json!({"type":"battery","protocol":1,"percentage":87,"charging":true}),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let battery = loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let event = harness.events.recv_timeout(remaining).unwrap();
        if let StateEvent::Device(DeviceEvent::Updated(device)) = event
            && let Some(battery) = device.battery
        {
            break battery;
        }
    };
    assert_eq!(battery.percentage(), 87);
    assert!(battery.charging);
}

#[test]
fn wrong_confirmation_code_aborts_pairing() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);

    exchange_openings(&harness, &mut peer, &client);
    let code = wait_pending_code(&harness, &client.fingerprint);
    harness.backend.approve(&client.fingerprint, &code).unwrap();

    // A confirmation that does not repeat the displayed code must not pair.
    // The server drops the session instead of allowing unlimited guesses.
    send(
        &mut peer,
        serde_json::json!({"type":"pair_confirm","protocol":1,"code":"00000000"}),
    );
    recv_err(&mut peer);
    assert!(harness.backend.peers().is_empty());

    // The aborted ceremony must not poison future attempts: the same client
    // identity can immediately start a fresh ceremony.
    let mut peer = connect(harness.port, &client);
    let derived = exchange_openings(&harness, &mut peer, &client);
    let code = wait_pending_code(&harness, &client.fingerprint);
    assert_eq!(code, derived);
    harness.backend.approve(&client.fingerprint, &code).unwrap();
    send(
        &mut peer,
        serde_json::json!({"type":"pair_confirm","protocol":1,"code":code}),
    );
    assert_eq!(recv(&mut peer)["type"], "paired");
    assert_eq!(harness.backend.peers().len(), 1);
}

#[test]
fn tampered_opening_aborts_pairing() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);

    let nonce = fresh_nonce();
    send(
        &mut peer,
        serde_json::json!({"type":"hello","protocol":1,"id":client.fingerprint,"name":"Test Phone","pair_commit":hex::encode(Sha256::digest(nonce))}),
    );
    let _ = recv(&mut peer);
    let _ = recv(&mut peer);
    // Reveal a different nonce than the hello committed to.
    let mut tampered = nonce;
    tampered[0] ^= 0xff;
    send(
        &mut peer,
        serde_json::json!({"type":"pair_open","protocol":1,"nonce":hex::encode(tampered)}),
    );
    recv_err(&mut peer);
    assert!(harness.backend.peers().is_empty());
}

fn pair_client(harness: &Harness, client: &ClientIdentity, peer: &mut TlsPeer) {
    let derived = exchange_openings(harness, peer, client);
    let code = wait_pending_code(harness, &client.fingerprint);
    assert_eq!(code, derived);
    harness.backend.approve(&client.fingerprint, &code).unwrap();
    send(
        peer,
        serde_json::json!({"type":"pair_confirm","protocol":1,"code":code}),
    );
    assert_eq!(recv(peer)["type"], "paired");
    // The server requests a notification sync on every fresh session; the
    // phone answers with its current list (possibly empty when the listener
    // permission is off).
    assert_eq!(recv(peer)["type"], "notifications_request");
    // Same recovery for media sessions: a daemon restart must not wait for
    // the next playback change to learn the current players.
    assert_eq!(recv(peer)["type"], "media_request");
}

fn wait_share_result(harness: &Harness, id: &str) -> handover_core::ShareResult {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if let StateEvent::ShareResult(result) = harness.events.recv_timeout(remaining).unwrap()
            && result.transfer_id == id
        {
            return result;
        }
    }
}

#[test]
fn native_share_result_requires_matching_receiver_ack_and_disconnect_fails_pending() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);
    let transfer_id = harness
        .backend
        .share_url(&client.fingerprint, "https://example.org/ok".into())
        .unwrap();
    let outbound = recv(&mut peer);
    assert_eq!(outbound["type"], "share_url");
    assert_eq!(outbound["transfer_id"], transfer_id);
    send(
        &mut peer,
        serde_json::json!({"type":"share_result","protocol":1,
        "transfer_id":transfer_id,"status":"completed"}),
    );
    let result = wait_share_result(&harness, &transfer_id);
    assert_eq!(result.status, ShareStatus::Completed);
    assert_eq!(result.reason, None);

    let pending = harness
        .backend
        .share_url(&client.fingerprint, "https://example.org/pending".into())
        .unwrap();
    assert_eq!(recv(&mut peer)["transfer_id"], pending);
    drop(peer);
    let result = wait_share_result(&harness, &pending);
    assert_eq!(result.status, ShareStatus::Failed);
    assert_eq!(result.reason, Some(ShareFailure::Disconnected));
}

#[test]
fn receiver_ack_follows_completed_file_and_rejects_invalid_url() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);
    let id = "00112233445566778899aabbccddeeff";
    send(
        &mut peer,
        serde_json::json!({"type":"share_file","protocol":1,
        "transfer_id":id,"name":"space ✓.txt","size":4}),
    );
    peer.tls.write_all(b"test").unwrap();
    peer.tls.flush().unwrap();
    let receipt = recv(&mut peer);
    assert_eq!(receipt["type"], "share_result");
    assert_eq!(receipt["transfer_id"], id);
    assert_eq!(receipt["status"], "completed");
    let bad = "ffeeddccbbaa99887766554433221100";
    send(
        &mut peer,
        serde_json::json!({"type":"share_url","protocol":1,
        "transfer_id":bad,"url":"file:///etc/passwd"}),
    );
    let rejected = recv(&mut peer);
    assert_eq!(rejected["status"], "failed");
    assert_eq!(rejected["reason"], "invalid_resource");
}

fn wait_notification(
    harness: &Harness,
    mut matches: impl FnMut(&NotificationEvent) -> bool,
) -> NotificationEvent {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match harness.events.recv_timeout(remaining).unwrap() {
            StateEvent::Notification(event) if matches(&event) => return event,
            _ => {}
        }
    }
}

fn post(key: &str, title: &str) -> serde_json::Value {
    serde_json::json!({"type":"notification_post","protocol":1,"key":key,
        "app":"Example","title":title,"body":"World","clearable":true,
        "actions":[{"id":"0","label":"Reply"}],"reply_supported":true})
}

#[test]
fn native_notifications_add_update_remove_and_sync() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);

    send(&mut peer, post("key-1", "Hello"));
    let added = wait_notification(
        &harness,
        |event| matches!(event, NotificationEvent::Added(n) if n.id.local_id == "key-1"),
    );
    match added {
        NotificationEvent::Added(notification) => {
            assert_eq!(
                notification.id.device_id.as_str(),
                format!("native:{}", client.fingerprint)
            );
            assert_eq!(notification.title, "Hello");
            assert_eq!(notification.actions.len(), 1);
            assert!(notification.reply_supported);
        }
        _ => unreachable!(),
    }

    send(&mut peer, post("key-1", "Hello again"));
    let updated = wait_notification(
        &harness,
        |event| matches!(event, NotificationEvent::Updated(n) if n.id.local_id == "key-1"),
    );
    match updated {
        NotificationEvent::Updated(notification) => assert_eq!(notification.title, "Hello again"),
        _ => unreachable!(),
    }

    send(
        &mut peer,
        serde_json::json!({"type":"notification_removed","protocol":1,"key":"key-1"}),
    );
    wait_notification(
        &harness,
        |event| matches!(event, NotificationEvent::Removed(id) if id.local_id == "key-1"),
    );

    // A full sync reconciles stale entries: key-2 is new, key-3 is new, and
    // the resync drops anything the phone no longer holds.
    send(
        &mut peer,
        serde_json::json!({"type":"notifications_sync","protocol":1,"enabled":true,
        "notifications":[
            {"key":"key-2","app":"Example","title":"Two","body":"b","clearable":true,"actions":[],"reply_supported":false},
            {"key":"key-3","app":"Example","title":"Three","body":"b","clearable":false,"actions":[],"reply_supported":false}
        ]}),
    );
    wait_notification(
        &harness,
        |event| matches!(event, NotificationEvent::Added(n) if n.id.local_id == "key-2"),
    );
    wait_notification(
        &harness,
        |event| matches!(event, NotificationEvent::Added(n) if n.id.local_id == "key-3"),
    );

    send(
        &mut peer,
        serde_json::json!({"type":"notifications_sync","protocol":1,"enabled":true,
        "notifications":[
            {"key":"key-3","app":"Example","title":"Three","body":"b","clearable":false,"actions":[],"reply_supported":false}
        ]}),
    );
    wait_notification(
        &harness,
        |event| matches!(event, NotificationEvent::Removed(id) if id.local_id == "key-2"),
    );

    // Disabling the listener clears the capability and the remaining state.
    // The device update is emitted before the removals, so accept both in
    // either order.
    send(
        &mut peer,
        serde_json::json!({"type":"notifications_sync","protocol":1,"enabled":false,"notifications":[]}),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut removed = false;
    let mut capability_cleared = false;
    while !removed || !capability_cleared {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match harness.events.recv_timeout(remaining).unwrap() {
            StateEvent::Notification(NotificationEvent::Removed(id)) if id.local_id == "key-3" => {
                removed = true;
            }
            StateEvent::Device(DeviceEvent::Updated(device))
                if device.id.as_str() == format!("native:{}", client.fingerprint)
                    && !device
                        .capabilities
                        .contains(&handover_core::Capability::Notifications) =>
            {
                capability_cleared = true;
            }
            _ => {}
        }
    }
    assert!(removed && capability_cleared);
}

#[test]
fn native_notification_commands_reach_the_phone() {
    use handover_core::{DeviceId, NotificationCommand, NotificationId};
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);

    send(&mut peer, post("key-9", "Hello"));
    wait_notification(
        &harness,
        |event| matches!(event, NotificationEvent::Added(n) if n.id.local_id == "key-9"),
    );

    let id = NotificationId::new(
        DeviceId::new(format!("native:{}", client.fingerprint)),
        "key-9",
    );
    harness
        .backend
        .execute_notification(
            &client.fingerprint,
            &NotificationCommand::Dismiss {
                notification_id: id.clone(),
            },
        )
        .unwrap();
    let dismiss = recv(&mut peer);
    assert_eq!(dismiss["type"], "notification_dismiss");
    assert_eq!(dismiss["key"], "key-9");

    harness
        .backend
        .execute_notification(
            &client.fingerprint,
            &NotificationCommand::Reply {
                notification_id: id.clone(),
                text: "Thanks".into(),
            },
        )
        .unwrap();
    let reply = recv(&mut peer);
    assert_eq!(reply["type"], "notification_reply");
    assert_eq!(reply["text"], "Thanks");

    harness
        .backend
        .execute_notification(
            &client.fingerprint,
            &NotificationCommand::InvokeAction {
                notification_id: id,
                action_id: "0".into(),
            },
        )
        .unwrap();
    let action = recv(&mut peer);
    assert_eq!(action["type"], "notification_action");
    assert_eq!(action["action_id"], "0");
}

#[test]
fn phone_side_notification_commands_are_rejected() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);

    // The dismiss/reply/action/request direction is Linux-to-phone only. A
    // phone sending them violates the protocol and loses the session without
    // any notification state entering Handover.
    send(
        &mut peer,
        serde_json::json!({"type":"notification_dismiss","protocol":1,"key":"key-1"}),
    );
    recv_err(&mut peer);
}

fn wait_media(harness: &Harness, mut matches: impl FnMut(&MediaEvent) -> bool) -> MediaEvent {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match harness.events.recv_timeout(remaining).unwrap() {
            StateEvent::Media(event) if matches(&event) => return event,
            _ => {}
        }
    }
}

fn media_post(player: &str, playback: &str) -> serde_json::Value {
    serde_json::json!({"type":"media_post","protocol":1,"player":player,
        "application":"Example Music","title":"Test track","artist":"Test artist",
        "playback":playback,"position_ms":1000,"duration_ms":180000,
        "controls":["play","pause","play_pause","next","previous","set_position"]})
}

#[test]
fn native_media_add_update_remove_and_sync() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);

    send(&mut peer, media_post("com.example.music", "playing"));
    let added = wait_media(
        &harness,
        |event| matches!(event, MediaEvent::Added(s) if s.id.player_id == "com.example.music"),
    );
    match added {
        MediaEvent::Added(session) => {
            assert_eq!(
                session.id.device_id.as_str(),
                format!("native:{}", client.fingerprint)
            );
            assert_eq!(session.application, "Example Music");
            assert_eq!(session.playback, handover_core::PlaybackState::Playing);
            assert_eq!(session.position_ms, Some(1_000));
            assert_eq!(session.duration_ms, Some(180_000));
            // Volume is never transported.
            assert_eq!(session.volume_percent, None);
            assert!(
                session
                    .controls
                    .contains(&handover_core::MediaControl::Next)
            );
            assert!(
                !session
                    .controls
                    .contains(&handover_core::MediaControl::Seek)
            );
        }
        _ => unreachable!(),
    }

    send(&mut peer, media_post("com.example.music", "paused"));
    let updated = wait_media(
        &harness,
        |event| matches!(event, MediaEvent::Updated(s) if s.id.player_id == "com.example.music"),
    );
    match updated {
        MediaEvent::Updated(session) => {
            assert_eq!(session.playback, handover_core::PlaybackState::Paused)
        }
        _ => unreachable!(),
    }

    send(
        &mut peer,
        serde_json::json!({"type":"media_removed","protocol":1,"player":"com.example.music"}),
    );
    wait_media(
        &harness,
        |event| matches!(event, MediaEvent::Removed(id) if id.player_id == "com.example.music"),
    );

    // A full sync reconciles stale players.
    send(
        &mut peer,
        serde_json::json!({"type":"media_sync","protocol":1,"sessions":[
            {"player":"com.example.a","application":"A","playback":"playing","controls":["pause"]},
            {"player":"com.example.b","application":"B","playback":"stopped","controls":[]}
        ]}),
    );
    wait_media(
        &harness,
        |event| matches!(event, MediaEvent::Added(s) if s.id.player_id == "com.example.a"),
    );
    wait_media(
        &harness,
        |event| matches!(event, MediaEvent::Added(s) if s.id.player_id == "com.example.b"),
    );

    send(
        &mut peer,
        serde_json::json!({"type":"media_sync","protocol":1,"sessions":[
            {"player":"com.example.b","application":"B","playback":"stopped","controls":[]}
        ]}),
    );
    wait_media(
        &harness,
        |event| matches!(event, MediaEvent::Removed(id) if id.player_id == "com.example.a"),
    );

    // A phone advertising relative seeks has no genuine platform API behind
    // the claim and must not enter Handover state.
    send(
        &mut peer,
        serde_json::json!({"type":"media_post","protocol":1,"player":"com.example.c",
            "application":"C","playback":"playing","controls":["seek"]}),
    );
    recv_err(&mut peer);
}

#[test]
fn native_media_commands_reach_the_phone() {
    use handover_core::{DeviceId, MediaCommand, MediaSessionId};
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);

    send(&mut peer, media_post("com.example.music", "playing"));
    wait_media(
        &harness,
        |event| matches!(event, MediaEvent::Added(s) if s.id.player_id == "com.example.music"),
    );

    let id = MediaSessionId::new(
        DeviceId::new(format!("native:{}", client.fingerprint)),
        "com.example.music",
    );
    harness
        .backend
        .execute_media(&client.fingerprint, &MediaCommand::Pause { id: id.clone() })
        .unwrap();
    let pause = recv(&mut peer);
    assert_eq!(pause["type"], "media_control");
    assert_eq!(pause["player"], "com.example.music");
    assert_eq!(pause["action"], "pause");

    harness
        .backend
        .execute_media(
            &client.fingerprint,
            &MediaCommand::SetPosition {
                id,
                position_ms: 60_000,
            },
        )
        .unwrap();
    let seek = recv(&mut peer);
    assert_eq!(seek["type"], "media_control");
    assert_eq!(seek["action"], "set_position");
    assert_eq!(seek["position_ms"], 60_000);
}

#[test]
fn phone_side_media_commands_are_rejected() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);

    // Media commands flow Linux-to-phone only. A phone sending them violates
    // the protocol and loses the session without state entering Handover.
    send(
        &mut peer,
        serde_json::json!({"type":"media_control","protocol":1,"player":"x","action":"pause"}),
    );
    recv_err(&mut peer);
}
