//! Real TLS pairing interop tests for the native backend.
//!
//! These tests speak the exact wire protocol (4-byte big-endian length +
//! JSON, TLS 1.3 with mutual certificates) against [`NativeBackend::serve`]
//! on an ephemeral port, using an OpenSSL client that mirrors the Android
//! transport's ceremony: hello, code comparison, pair confirmation, battery.
//! DNS-SD has a separate ignored live-network test in the crate unit tests.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use handover_core::{
    CallAction, CallEvent, CallPhase, DeviceCommandAction, DeviceEvent, MediaEvent,
    NotificationEvent, ShareFailure, ShareStatus, StateEvent,
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
    stream.set_nodelay(true).unwrap();
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

#[test]
#[ignore = "requires a live TCP listener and waits for the pre-auth timeout"]
fn hostile_pre_auth_connections_are_bounded() {
    let harness = harness();
    let mut sockets = Vec::new();
    for _ in 0..9 {
        let socket = TcpStream::connect(("127.0.0.1", harness.port)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        sockets.push(socket);
    }

    // Eight sockets are admitted and remain in the TLS pre-auth phase. The
    // ninth is rejected by the per-source active-session limit.
    let mut byte = [0u8; 1];
    assert_eq!(sockets[8].read(&mut byte).unwrap(), 0);
    drop(sockets);

    // A socket that is admitted but sends no TLS bytes is closed by the
    // explicit pre-auth read timeout rather than held indefinitely.
    let mut idle = TcpStream::connect(("127.0.0.1", harness.port)).unwrap();
    idle.set_read_timeout(Some(Duration::from_secs(7))).unwrap();
    std::thread::sleep(Duration::from_secs(6));
    assert_eq!(idle.read(&mut byte).unwrap(), 0);
}

#[test]
fn generated_malformed_frames_are_rejected_over_tls() {
    let harness = harness();
    let client = test_identity();
    for length in 0..=8usize {
        let mut peer = connect(harness.port, &client);
        let mut frame = (length as u32).to_be_bytes().to_vec();
        frame.extend(std::iter::repeat_n(0xff, length));
        peer.tls.write_all(&frame).unwrap();
        peer.tls.flush().unwrap();
        recv_err(&mut peer);
    }
}

struct Harness {
    directory: std::path::PathBuf,
    backend: NativeBackend,
    port: u16,
    events: mpsc::Receiver<StateEvent>,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let directory = dir.path().to_path_buf();
    let backend = NativeBackend::open(directory.clone()).unwrap();
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
        directory,
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
    exchange_openings_with_downloads(harness, peer, client, true)
}

fn exchange_openings_with_downloads(
    harness: &Harness,
    peer: &mut TlsPeer,
    client: &ClientIdentity,
    downloads: bool,
) -> String {
    exchange_openings_with_features(harness, peer, client, downloads, false, false)
}

fn exchange_openings_with_features(
    harness: &Harness,
    peer: &mut TlsPeer,
    client: &ClientIdentity,
    downloads: bool,
    files_v2: bool,
    volume_control: bool,
) -> String {
    let nonce = fresh_nonce();
    let commit = hex::encode(Sha256::digest(nonce));
    send(
        peer,
        serde_json::json!({"type":"hello","protocol":1,"id":client.fingerprint,"name":"Test Phone","pair_commit":commit,"file_downloads":downloads,"files_v2":files_v2,"volume_control":volume_control}),
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
    assert_eq!(recv(peer)["type"], "call_request");
    assert_eq!(recv(peer)["type"], "media_request");
    let contacts = recv(peer);
    assert_eq!(contacts["type"], "contacts_request");
    assert_eq!(contacts["protocol"], 1);
    assert_eq!(contacts["chunked"], true);
}

#[test]
fn local_device_name_is_preserved_by_live_updates_disconnect_and_reconnect() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);
    let next_device = || loop {
        if let StateEvent::Device(DeviceEvent::Added(device) | DeviceEvent::Updated(device)) =
            harness.events.recv_timeout(Duration::from_secs(5)).unwrap()
        {
            break device;
        }
    };
    assert_eq!(next_device().name, "Test Phone");
    assert_eq!(
        harness
            .backend
            .rename_peer(&client.fingerprint, "Pocket phone")
            .unwrap(),
        "Pocket phone"
    );
    send(
        &mut peer,
        serde_json::json!({"type":"battery","protocol":1,"percentage":77,"charging":false}),
    );
    let updated = next_device();
    assert_eq!(updated.name, "Pocket phone");
    assert_eq!(updated.battery.unwrap().percentage(), 77);
    drop(peer);
    let offline = next_device();
    assert!(!offline.connected);
    assert_eq!(offline.name, "Pocket phone");
    let mut peer = connect(harness.port, &client);
    send(
        &mut peer,
        serde_json::json!({"type":"hello","protocol":1,"id":client.fingerprint,"name":"New reported name","trusted_server_id":harness.backend.identity()}),
    );
    assert_eq!(recv(&mut peer)["type"], "hello");
    assert_eq!(recv(&mut peer)["type"], "paired");
    for expected in [
        "notifications_request",
        "call_request",
        "media_request",
        "contacts_request",
    ] {
        assert_eq!(recv(&mut peer)["type"], expected);
    }
    let online = next_device();
    assert!(online.connected);
    assert_eq!(online.name, "Pocket phone");
    assert_eq!(
        harness
            .backend
            .rename_peer(&client.fingerprint, "")
            .unwrap(),
        "New reported name"
    );
    send(
        &mut peer,
        serde_json::json!({"type":"battery","protocol":1,"percentage":79,"charging":false}),
    );
    assert_eq!(next_device().name, "New reported name");
}

#[test]
fn audio_offers_require_readiness_and_the_paired_certificate() {
    use handover_core::AudioStreamPhase;
    let harness = harness();
    let identity = test_identity();
    let mut control = connect(harness.port, &identity);
    pair_client(&harness, &identity, &mut control);
    let event: Arc<dyn Fn(StateEvent) + Send + Sync> = Arc::new(|_| {});
    assert!(
        harness
            .backend
            .start_audio(&identity.fingerprint, event.clone())
            .is_err()
    );
    send(
        &mut control,
        serde_json::json!({"type":"audio_ready","protocol":1}),
    );
    loop {
        if matches!(
            harness.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            StateEvent::AudioStream(_)
        ) {
            break;
        }
    }
    harness
        .backend
        .start_audio(&identity.fingerprint, event.clone())
        .unwrap();
    // Wake the control reader so it drains the queued offer.
    send(
        &mut control,
        serde_json::json!({"type":"pong","protocol":1}),
    );
    let offer = recv(&mut control);
    assert_eq!(offer["type"], "audio_offer");
    let old_id = offer["stream_id"].as_str().unwrap().to_owned();
    assert_eq!(
        harness.backend.audio_streams()[0].state,
        AudioStreamPhase::Starting
    );

    let attacker = test_identity();
    let mut unauthorized = connect(harness.port, &attacker);
    send(
        &mut unauthorized,
        serde_json::json!({"type":"audio_receive","protocol":1,"stream_id":old_id}),
    );
    recv_err(&mut unauthorized);
    assert_eq!(
        harness.backend.audio_streams()[0].state,
        AudioStreamPhase::Starting
    );

    harness
        .backend
        .stop_audio(&identity.fingerprint, event.clone())
        .unwrap();
    assert_eq!(
        harness.backend.audio_streams()[0].state,
        AudioStreamPhase::Idle
    );
    harness
        .backend
        .start_audio(&identity.fingerprint, event.clone())
        .unwrap();
    send(
        &mut control,
        serde_json::json!({"type":"audio_stop","protocol":1,"stream_id":old_id}),
    );
    send(
        &mut control,
        serde_json::json!({"type":"ping","protocol":1}),
    );
    // The pong proves that the stale stop was processed first.
    while recv(&mut control)["type"] != "pong" {}
    assert_eq!(
        harness.backend.audio_streams()[0].state,
        AudioStreamPhase::Starting
    );
    harness
        .backend
        .stop_audio(&identity.fingerprint, event)
        .unwrap();
}

#[test]
fn phone_can_start_and_pause_its_stream_without_disconnect_or_restart() {
    use handover_core::AudioStreamPhase;
    fn receive_control(peer: &mut TlsPeer) -> serde_json::Value {
        loop {
            let message = recv(peer);
            match message["type"].as_str() {
                Some("notifications_request" | "media_request" | "contacts_request") => {}
                Some("ping") => send(peer, serde_json::json!({"type":"pong","protocol":1})),
                _ => return message,
            }
        }
    }
    let harness = harness();
    let identity = test_identity();

    let mut unpaired = connect(harness.port, &identity);
    send(
        &mut unpaired,
        serde_json::json!({"type":"audio_stream_set","protocol":1,"enabled":true}),
    );
    recv_err(&mut unpaired);
    assert!(harness.backend.audio_streams().is_empty());

    let mut control = connect(harness.port, &identity);
    pair_client(&harness, &identity, &mut control);
    // A paired phone still has to advertise audio readiness.
    send(
        &mut control,
        serde_json::json!({"type":"audio_stream_set","protocol":1,"enabled":true}),
    );
    send(
        &mut control,
        serde_json::json!({"type":"ping","protocol":1}),
    );
    while receive_control(&mut control)["type"] != "pong" {}
    assert!(harness.backend.audio_streams().is_empty());

    send(
        &mut control,
        serde_json::json!({"type":"audio_ready","protocol":1}),
    );
    send(
        &mut control,
        serde_json::json!({"type":"audio_stream_set","protocol":1,"enabled":true}),
    );
    let offer = receive_control(&mut control);
    assert_eq!(offer["type"], "audio_offer");
    let first_id = offer["stream_id"].as_str().unwrap().to_owned();
    assert_eq!(
        harness.backend.audio_streams()[0].state,
        AudioStreamPhase::Starting
    );

    // Repeated Start must not replace an offer already being negotiated.
    send(
        &mut control,
        serde_json::json!({"type":"audio_stream_set","protocol":1,"enabled":true}),
    );
    send(
        &mut control,
        serde_json::json!({"type":"ping","protocol":1}),
    );
    assert_eq!(receive_control(&mut control)["type"], "pong");

    send(
        &mut control,
        serde_json::json!({"type":"audio_stream_set","protocol":1,"enabled":false}),
    );
    let stop = receive_control(&mut control);
    assert_eq!(stop["type"], "audio_stop");
    assert_eq!(stop["stream_id"], first_id);
    assert_eq!(
        harness.backend.audio_streams()[0].state,
        AudioStreamPhase::Idle
    );

    // The same authenticated connection can start a fresh stream.
    send(
        &mut control,
        serde_json::json!({"type":"audio_stream_set","protocol":1,"enabled":true}),
    );
    let offer = receive_control(&mut control);
    assert_eq!(offer["type"], "audio_offer");
    assert_ne!(offer["stream_id"], first_id);
    send(
        &mut control,
        serde_json::json!({"type":"audio_stream_set","protocol":1,"enabled":false}),
    );
    assert_eq!(receive_control(&mut control)["type"], "audio_stop");
    send(
        &mut control,
        serde_json::json!({"type":"ping","protocol":1}),
    );
    assert_eq!(receive_control(&mut control)["type"], "pong");
    assert_eq!(
        harness.backend.audio_streams()[0].state,
        AudioStreamPhase::Idle
    );
}

#[test]
fn managed_audio_reports_confirmed_state_and_cancels_recovery_after_manual_pause() {
    fn receive(peer: &mut TlsPeer, kind: &str) -> serde_json::Value {
        loop {
            let message = recv(peer);
            if message["type"] == kind {
                return message;
            }
            if message["type"] == "ping" {
                send(peer, serde_json::json!({"type":"pong","protocol":1}));
            }
        }
    }
    let harness = harness();
    let identity = test_identity();
    let mut peer = connect(harness.port, &identity);
    pair_client(&harness, &identity, &mut peer);
    send(
        &mut peer,
        serde_json::json!({"type":"audio_ready","protocol":1,"managed":true}),
    );
    let status = receive(&mut peer, "audio_status");
    assert_eq!(status["status"]["state"], "idle");
    assert_eq!(status["status"]["enabled"], false);
    let request = "11111111111111111111111111111111";
    send(
        &mut peer,
        serde_json::json!({"type":"audio_stream_set","protocol":1,"enabled":true,"request_id":request}),
    );
    let offer = receive(&mut peer, "audio_offer");
    let stream = offer["stream_id"].as_str().unwrap();
    let result = receive(&mut peer, "audio_stream_result");
    assert_eq!(result["request_id"], request);
    assert_eq!(result["accepted"], true);
    // An accepted start is not a claim that audio is playing.
    assert_eq!(
        harness.backend.audio_streams()[0].state,
        handover_core::AudioStreamPhase::Starting
    );
    send(
        &mut peer,
        serde_json::json!({"type":"audio_receiver_state","protocol":1,"stream_id":stream,"state":"suspended"}),
    );
    assert_eq!(
        receive(&mut peer, "audio_status")["status"]["state"],
        "suspended"
    );
    send(
        &mut peer,
        serde_json::json!({"type":"audio_diagnostics","protocol":1,"stream_id":"stale","diagnostics":{"route":"unknown","round_trip_ms":45}}),
    );
    send(&mut peer, serde_json::json!({"type":"ping","protocol":1}));
    receive(&mut peer, "pong");
    assert!(harness.backend.audio_streams()[0].diagnostics.is_none());
    send(
        &mut peer,
        serde_json::json!({"type":"audio_diagnostics","protocol":1,"stream_id":stream,"diagnostics":{"route":"vpn","transport":"udp","round_trip_ms":45,"packets_received":10}}),
    );
    let status = receive(&mut peer, "audio_status");
    assert_eq!(
        status["status"]["diagnostics"]["receiver"]["round_trip_ms"],
        45
    );
    assert!(
        status["status"]["diagnostics"]["receiver"]["updated_at_ms"]
            .as_i64()
            .unwrap()
            > 0
    );
    send(
        &mut peer,
        serde_json::json!({"type":"audio_receiver_state","protocol":1,"stream_id":stream,"state":"failed"}),
    );
    let status = receive(&mut peer, "audio_status");
    assert_eq!(status["status"]["state"], "reconnecting");
    assert_eq!(status["status"]["enabled"], true);
    assert_eq!(status["status"]["retry_attempt"], 1);
    let stop = "22222222222222222222222222222222";
    send(
        &mut peer,
        serde_json::json!({"type":"audio_stream_set","protocol":1,"enabled":false,"request_id":stop}),
    );
    let status = receive(&mut peer, "audio_status");
    assert_eq!(status["status"]["state"], "idle");
    assert_eq!(status["status"]["enabled"], false);
    let result = receive(&mut peer, "audio_stream_result");
    assert_eq!(result["request_id"], stop);
    assert_eq!(result["accepted"], true);
    std::thread::sleep(Duration::from_millis(1100));
    send(&mut peer, serde_json::json!({"type":"ping","protocol":1}));
    loop {
        let message = recv(&mut peer);
        assert_ne!(
            message["type"], "audio_offer",
            "a manual pause must cancel the pending recovery"
        );
        if message["type"] == "pong" {
            break;
        }
    }
}

#[test]
#[ignore = "requires parec and a live local PulseAudio or PipeWire output"]
fn audio_stream_captures_pcm_over_a_separate_authenticated_socket() {
    use handover_core::AudioStreamPhase;
    let harness = harness();
    let identity = test_identity();
    let mut control = connect(harness.port, &identity);
    pair_client(&harness, &identity, &mut control);
    send(
        &mut control,
        serde_json::json!({"type":"audio_ready","protocol":1}),
    );
    loop {
        if matches!(
            harness.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            StateEvent::AudioStream(_)
        ) {
            break;
        }
    }
    let (tx, rx) = mpsc::channel();
    let event: Arc<dyn Fn(StateEvent) + Send + Sync> = Arc::new(move |ev| {
        let _ = tx.send(ev);
    });
    harness
        .backend
        .start_audio(&identity.fingerprint, event.clone())
        .unwrap();
    send(
        &mut control,
        serde_json::json!({"type":"pong","protocol":1}),
    );
    let offer = recv(&mut control);
    assert_eq!(offer["type"], "audio_offer");
    let id = offer["stream_id"].as_str().unwrap();
    let mut audio = connect(harness.port, &identity);
    send(
        &mut audio,
        serde_json::json!({"type":"audio_receive","protocol":1,"stream_id":id}),
    );
    assert_eq!(recv(&mut audio)["type"], "audio_format");
    let mut previous = 0;
    for _ in 0..10 {
        let mut timestamp = [0; 8];
        audio.tls.read_exact(&mut timestamp).unwrap();
        let timestamp = u64::from_be_bytes(timestamp);
        assert!(timestamp >= previous);
        previous = timestamp;
        audio.tls.read_exact(&mut [0; 1920]).unwrap();
    }
    send(
        &mut control,
        serde_json::json!({"type":"audio_playing","protocol":1,"stream_id":id}),
    );
    send(
        &mut control,
        serde_json::json!({"type":"ping","protocol":1}),
    );
    assert_eq!(recv(&mut control)["type"], "pong");
    assert_eq!(
        harness.backend.audio_streams()[0].state,
        AudioStreamPhase::Streaming
    );
    harness
        .backend
        .stop_audio(&identity.fingerprint, event)
        .unwrap();
    assert_eq!(
        harness.backend.audio_streams()[0].state,
        AudioStreamPhase::Idle
    );
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        StateEvent::AudioStream(_)
    ));
    harness.backend.stop_all_audio();
}

#[test]
#[ignore = "requires a live PulseAudio or PipeWire output and parec"]
fn realtime_audio_negotiates_and_decodes_opus_over_the_paired_control_channel() {
    use handover_core::AudioStreamPhase;
    use rtc::{
        interceptor::Registry, peer_connection::configuration::media_engine::MediaEngine,
        rtp_transceiver::rtp_sender::RtpCodecKind,
    };
    use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
    use webrtc::peer_connection::register_default_interceptors;
    use webrtc::peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
        RTCIceGatheringState, RTCSessionDescription,
    };
    use webrtc::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};

    struct AudioReceiver {
        track_tx: tokio::sync::mpsc::Sender<()>,
        gather_complete_tx: tokio::sync::mpsc::Sender<()>,
    }

    #[async_trait::async_trait]
    impl PeerConnectionEventHandler for AudioReceiver {
        async fn on_track(&self, track: Arc<dyn TrackRemote>) {
            let track_tx = self.track_tx.clone();
            drop(tokio::spawn(async move {
                let Ok(mut decoder) = opus::Decoder::new(48_000, opus::Channels::Stereo) else {
                    return;
                };
                let mut pcm = vec![0_i16; 5760 * 2];
                while let Some(event) = track.poll().await {
                    if let TrackRemoteEvent::OnRtpPacket(packet) = event
                        && matches!(
                            decoder.decode(&packet.payload, &mut pcm, false),
                            Ok(samples) if samples > 0
                        )
                    {
                        let _ = track_tx.try_send(());
                        break;
                    }
                }
            }));
        }

        async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
            if state == RTCIceGatheringState::Complete {
                let _ = self.gather_complete_tx.try_send(());
            }
        }
    }

    struct StopAudioOnDrop {
        backend: NativeBackend,
        peer: String,
    }
    impl Drop for StopAudioOnDrop {
        fn drop(&mut self) {
            let event: Arc<dyn Fn(StateEvent) + Send + Sync> = Arc::new(|_| {});
            let _ = self.backend.stop_audio(&self.peer, event);
            self.backend.stop_all_audio();
        }
    }

    struct ClosePeerOnDrop {
        handle: tokio::runtime::Handle,
        peer: Arc<dyn PeerConnection>,
        closed: bool,
    }
    impl ClosePeerOnDrop {
        fn close(&mut self) {
            if !self.closed {
                let _ = self.handle.block_on(self.peer.close());
                self.closed = true;
            }
        }
    }
    impl Drop for ClosePeerOnDrop {
        fn drop(&mut self) {
            self.close();
        }
    }

    fn recv_audio_offer(peer: &mut TlsPeer) -> serde_json::Value {
        loop {
            let message = recv(peer);
            match message["type"].as_str() {
                Some("audio_rtc_offer") => return message,
                Some("pong" | "audio_stop") => continue,
                other => {
                    panic!("unexpected control frame while waiting for audio offer: {other:?}")
                }
            }
        }
    }

    let harness = harness();
    let identity = test_identity();
    let cleanup = StopAudioOnDrop {
        backend: harness.backend.clone(),
        peer: identity.fingerprint.clone(),
    };
    let mut control = connect(harness.port, &identity);
    pair_client(&harness, &identity, &mut control);
    send(
        &mut control,
        serde_json::json!({"type":"audio_ready","protocol":1,"realtime":true}),
    );
    loop {
        match harness.events.recv_timeout(Duration::from_secs(10)) {
            Ok(StateEvent::AudioStream(_)) => break,
            Ok(_) => continue,
            Err(error) => panic!("timed out waiting for realtime audio readiness: {error}"),
        }
    }

    let event: Arc<dyn Fn(StateEvent) + Send + Sync> = Arc::new(|_| {});
    harness
        .backend
        .start_audio(&identity.fingerprint, event.clone())
        .unwrap();
    send(
        &mut control,
        serde_json::json!({"type":"ping","protocol":1}),
    );
    let old_offer = recv_audio_offer(&mut control);
    let old_id = old_offer["stream_id"].as_str().unwrap().to_owned();

    // Replace a still-starting stream. A delayed answer for its old ID must
    // not be delivered to the new peer connection's answer channel.
    harness
        .backend
        .stop_audio(&identity.fingerprint, event.clone())
        .unwrap();
    harness
        .backend
        .start_audio(&identity.fingerprint, event.clone())
        .unwrap();
    send(
        &mut control,
        serde_json::json!({
            "type":"audio_rtc_answer",
            "protocol":1,
            "stream_id":old_id,
            "sdp":"not a valid SDP answer"
        }),
    );
    send(
        &mut control,
        serde_json::json!({"type":"ping","protocol":1}),
    );
    let mut pending_offer = None;
    loop {
        let message = recv(&mut control);
        match message["type"].as_str() {
            Some("pong") => break,
            Some("audio_rtc_offer") => pending_offer = Some(message),
            Some("audio_stop") => {}
            other => panic!("unexpected control frame after stale answer: {other:?}"),
        }
    }
    let offer = pending_offer.unwrap_or_else(|| recv_audio_offer(&mut control));
    let id = offer["stream_id"].as_str().unwrap().to_owned();
    assert_ne!(id, old_id);
    assert_eq!(
        harness.backend.audio_streams()[0].state,
        AudioStreamPhase::Starting
    );
    let offer_sdp = offer["sdp"].as_str().unwrap().to_owned();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (track_tx, mut track_rx) = tokio::sync::mpsc::channel(1);
    let (gather_complete_tx, mut gather_complete_rx) = tokio::sync::mpsc::channel(1);
    let (peer, answer_result) = runtime.block_on(async {
        let mut media = MediaEngine::default();
        if let Err(error) = media.register_default_codecs() {
            return (None, Err(error.to_string()));
        }
        let registry = match register_default_interceptors(Registry::new(), &mut media) {
            Ok(registry) => registry,
            Err(error) => return (None, Err(error.to_string())),
        };
        let handler = Arc::new(AudioReceiver {
            track_tx,
            gather_complete_tx,
        });
        let peer = match PeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::new().build())
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .with_handler(handler)
            .with_udp_addrs(vec!["0.0.0.0:0"])
            .build()
            .await
        {
            Ok(peer) => Arc::new(peer),
            Err(error) => return (None, Err(error.to_string())),
        };
        let answer_result = async {
            peer.add_transceiver_from_kind(
                RtpCodecKind::Audio,
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Recvonly,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|error| error.to_string())?;
            let offer =
                RTCSessionDescription::offer(offer_sdp).map_err(|error| error.to_string())?;
            peer.set_remote_description(offer)
                .await
                .map_err(|error| error.to_string())?;
            let answer = peer
                .create_answer(None)
                .await
                .map_err(|error| error.to_string())?;
            peer.set_local_description(answer)
                .await
                .map_err(|error| error.to_string())?;
            let gathered = tokio::time::timeout(Duration::from_secs(10), gather_complete_rx.recv())
                .await
                .map_err(|_| "ICE gathering timed out".to_owned())?;
            if gathered.is_none() {
                return Err("ICE gathering event channel closed".to_owned());
            }
            peer.local_description()
                .await
                .map(|description| description.sdp)
                .ok_or_else(|| "missing local answer".to_owned())
        }
        .await;
        (Some(peer as Arc<dyn PeerConnection>), answer_result)
    });

    let mut close_peer = peer.map(|peer| ClosePeerOnDrop {
        handle: runtime.handle().clone(),
        peer,
        closed: false,
    });
    let mut result = match answer_result {
        Ok(answer) => {
            send(
                &mut control,
                serde_json::json!({
                    "type":"audio_rtc_answer",
                    "protocol":1,
                    "stream_id":id,
                    "sdp":answer
                }),
            );
            match runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(10), track_rx.recv()).await
            }) {
                Ok(Some(())) => Ok(()),
                Ok(None) => Err("WebRTC track ended before Opus decode".to_owned()),
                Err(_) => Err("timed out waiting for a decodable Opus RTP packet".to_owned()),
            }
        }
        Err(error) => Err(error),
    };
    if result.is_ok() {
        send(
            &mut control,
            serde_json::json!({"type":"audio_playing","protocol":1,"stream_id":id}),
        );
        send(
            &mut control,
            serde_json::json!({"type":"ping","protocol":1}),
        );
        loop {
            if recv(&mut control)["type"] == "pong" {
                break;
            }
        }
        result = if harness.backend.audio_streams()[0].state == AudioStreamPhase::Streaming {
            Ok(())
        } else {
            Err("audio_playing was not accepted for the current stream".to_owned())
        };
    }
    if let Some(peer) = close_peer.as_mut() {
        peer.close();
    }
    let _ = harness
        .backend
        .stop_audio(&identity.fingerprint, event.clone());
    harness.backend.stop_all_audio();
    drop(cleanup);
    assert!(
        result.is_ok(),
        "realtime audio integration failed: {result:?}"
    );
    assert_eq!(
        harness.backend.audio_streams()[0].state,
        AudioStreamPhase::Idle
    );
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
    assert!(
        !harness
            .backend
            .cancel_share(&client.fingerprint, &transfer_id),
        "a share already sent to the phone cannot be recalled"
    );
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

fn wait_call(harness: &Harness) -> CallEvent {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if let StateEvent::Call(event) = harness.events.recv_timeout(remaining).unwrap() {
            return event;
        }
    }
}

fn wait_call_result(harness: &Harness) -> handover_core::CallCommandResult {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if let StateEvent::CallCommandResult(result) =
            harness.events.recv_timeout(remaining).unwrap()
        {
            return result;
        }
    }
}

#[test]
fn native_call_result_is_correlated_without_claiming_effect() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);
    send(
        &mut peer,
        serde_json::json!({"type":"call_state","protocol":1,
        "phase":"ringing","controls":["answer"],"generation":9}),
    );
    let _ = wait_call(&harness);

    let stale_request_id = harness
        .backend
        .call_control(&client.fingerprint, "answer", None, Some(8))
        .unwrap();
    let request_id = harness
        .backend
        .call_control(&client.fingerprint, "answer", None, Some(9))
        .unwrap();
    let command = recv(&mut peer);
    assert_eq!(command["request_id"], request_id);
    assert_ne!(request_id, stale_request_id);
    assert_eq!(command["generation"], 9);
    send(
        &mut peer,
        serde_json::json!({"type":"call_result","protocol":1,
        "request_id":request_id,"action":"answer","accepted":false,
        "failure":"permission_denied"}),
    );
    let result = wait_call_result(&harness);
    assert_eq!(result.request_id, request_id);
    assert_eq!(result.action, CallAction::Answer);
    assert!(!result.accepted);
    assert_eq!(
        result.failure,
        Some(handover_core::CallCommandFailure::PermissionDenied)
    );
}

#[test]
fn native_call_result_accepts_platform_unsupported_verdict() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);
    send(
        &mut peer,
        serde_json::json!({"type":"call_state","protocol":1,
        "phase":"off_hook","controls":["hangup"],"generation":3}),
    );
    let _ = wait_call(&harness);

    let request_id = harness
        .backend
        .call_control(&client.fingerprint, "hangup", None, Some(3))
        .unwrap();
    let _ = recv(&mut peer);
    // Older Android releases report this instead of attempting the platform
    // call; the session must surface it, not drop the frame as invalid.
    send(
        &mut peer,
        serde_json::json!({"type":"call_result","protocol":1,
        "request_id":request_id,"action":"hangup","accepted":false,
        "failure":"unsupported"}),
    );
    let result = wait_call_result(&harness);
    assert_eq!(result.request_id, request_id);
    assert!(!result.accepted);
    assert_eq!(
        result.failure,
        Some(handover_core::CallCommandFailure::Unsupported)
    );
}

#[test]
fn phone_screensaver_request_is_tracked_and_released() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);
    let request_id = "0123456789abcdef0123456789abcdef";
    send(
        &mut peer,
        serde_json::json!({"type":"screensaver_control","protocol":1,
        "request_id":request_id,"inhibit":true}),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let result = loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if let StateEvent::DeviceCommandResult(result) =
            harness.events.recv_timeout(remaining).unwrap()
        {
            break result;
        }
    };
    assert_eq!(result.request_id, request_id);
    assert_eq!(result.action, DeviceCommandAction::Screensaver);
    assert!(result.accepted);
    assert_eq!(
        harness.backend.phone_screensaver_requests(),
        [client.fingerprint]
    );
    // A keep-awake frame from the phone is a wrong-direction command: the
    // session ends instead of changing wake state.
    send(
        &mut peer,
        serde_json::json!({"type":"keep_awake","protocol":1,
        "request_id":request_id,"inhibit":true}),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if let StateEvent::Device(_) = harness.events.recv_timeout(remaining).unwrap() {
            break;
        }
    }
}

#[test]
fn session_teardown_with_reentrant_callback_completes() {
    // The daemon's event callback calls back into the backend (its
    // screensaver refresh reads the phone request set). Teardown must emit
    // without holding the runtime lock, or the session thread deadlocks
    // itself and wedges every later backend and IPC operation.
    let dir = tempfile::tempdir().unwrap();
    let backend = NativeBackend::open(dir.path().to_path_buf()).unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    let server = backend.clone();
    let reader = backend.clone();
    std::thread::spawn(move || {
        let event = Arc::new(move |ev: StateEvent| {
            let _ = reader.phone_screensaver_requests();
            let _ = tx.send(ev);
        });
        let _ = server.serve(listener, event);
    });
    let directory = dir.path().to_path_buf();
    std::mem::forget(dir);
    let harness = Harness {
        directory,
        backend,
        port,
        events: rx,
    };
    let client = test_identity();
    let peer = connect(harness.port, &client);
    let mut peer = peer;
    pair_client(&harness, &client, &mut peer);
    drop(peer);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if let StateEvent::Device(DeviceEvent::Updated(device)) =
            harness.events.recv_timeout(remaining).unwrap()
            && device.id.as_str() == format!("native:{}", client.fingerprint)
        {
            assert!(!device.connected);
            break;
        }
    }
}

#[test]
fn native_call_state_updates_and_disconnect_removes_state() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);
    let device_id = format!("native:{}", client.fingerprint);

    // Each phone-attested state replaces the previous controls; in particular,
    // unknown must not retain actions from the preceding off-hook state.
    for (wire_phase, phase, wire_controls, controls) in [
        (
            "idle",
            CallPhase::Idle,
            vec!["place"],
            vec![CallAction::Place],
        ),
        (
            "ringing",
            CallPhase::Ringing,
            vec!["answer", "decline"],
            vec![CallAction::Answer, CallAction::Decline],
        ),
        (
            "off_hook",
            CallPhase::OffHook,
            vec!["hangup"],
            vec![CallAction::Hangup],
        ),
        ("unknown", CallPhase::Unknown, vec![], vec![]),
    ] {
        send(
            &mut peer,
            serde_json::json!({"type":"call_state","protocol":1,
                "phase":wire_phase,"controls":wire_controls}),
        );
        match wait_call(&harness) {
            CallEvent::Updated(state) => {
                assert_eq!(state.device_id.as_str(), device_id);
                assert_eq!(state.phase, phase);
                assert_eq!(state.controls, controls.into_iter().collect());
            }
            event => panic!("expected call state update, got {event:?}"),
        }
    }

    drop(peer);
    match wait_call(&harness) {
        CallEvent::Removed(id) => assert_eq!(id.as_str(), device_id),
        event => panic!("expected call state removal on disconnect, got {event:?}"),
    }
}

#[test]
fn phone_side_call_commands_are_rejected() {
    for message in [
        serde_json::json!({"type":"call_control","protocol":1,"action":"answer"}),
        serde_json::json!({"type":"call_request","protocol":1}),
    ] {
        let harness = harness();
        let client = test_identity();
        let mut peer = connect(harness.port, &client);
        pair_client(&harness, &client, &mut peer);

        // Control and resync requests flow Linux-to-phone only. Even valid
        // messages in the wrong direction must cause the session to close.
        send(&mut peer, message);
        recv_err(&mut peer);
    }
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

fn wait_contacts(harness: &Harness) -> handover_core::ContactsEvent {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if let StateEvent::Contacts(event) = harness.events.recv_timeout(remaining).unwrap() {
            return event;
        }
    }
}

#[test]
fn native_contacts_publish_complete_generations_and_report_failures() {
    use handover_core::{ContactsEvent, ContactsSyncFailure};
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);
    assert!(harness.backend.request_contacts_sync(&client.fingerprint));
    let request = recv(&mut peer);
    assert_eq!(request["type"], "contacts_request");
    assert_eq!(request["chunked"], true);

    // Two frames carry more photo data than a single native frame can hold.
    // The first observed contact event must contain both records, never a prefix.
    for (index, id) in ["a", "b"].into_iter().enumerate() {
        send(
            &mut peer,
            serde_json::json!({
                "type": "contacts_chunk", "protocol": 1, "generation": "complete",
                "index": index, "done": index == 1,
                "contacts": [{"local_id": id, "display_name": "Test", "photo": "a".repeat(45 * 1024)}]
            }),
        );
    }
    match wait_contacts(&harness) {
        ContactsEvent::Synced {
            device_id,
            contacts,
        } => {
            assert_eq!(device_id.as_str(), format!("native:{}", client.fingerprint));
            assert_eq!(contacts.len(), 2);
            assert_eq!(contacts[0].local_id, "a");
            assert_eq!(contacts[1].local_id, "b");
        }
        event => panic!("expected complete snapshot, got {event:?}"),
    }

    send(
        &mut peer,
        serde_json::json!({"type":"contacts_chunk","protocol":1,
        "generation":"incomplete","index":0,"done":false,"contacts":[{"local_id":"c","display_name":"Test"}]}),
    );
    send(
        &mut peer,
        serde_json::json!({"type":"contacts_error","protocol":1,
        "generation":"incomplete","failure":"permission_denied"}),
    );
    assert!(matches!(
        wait_contacts(&harness),
        ContactsEvent::SyncFailed {
            failure: ContactsSyncFailure::PermissionDenied,
            ..
        }
    ));
    send(
        &mut peer,
        serde_json::json!({"type":"contacts_chunk","protocol":1,
        "generation":"incomplete","index":1,"done":true,"contacts":[]}),
    );
    assert!(matches!(
        wait_contacts(&harness),
        ContactsEvent::SyncFailed {
            failure: ContactsSyncFailure::Rejected,
            ..
        }
    ));

    // Legacy truncation markers must not masquerade as successful empty syncs.
    send(
        &mut peer,
        serde_json::json!({"type":"contacts_sync","protocol":1,"complete":false,"contacts":[]}),
    );
    assert!(matches!(
        wait_contacts(&harness),
        ContactsEvent::SyncFailed {
            failure: ContactsSyncFailure::TooLarge,
            ..
        }
    ));
    send(
        &mut peer,
        serde_json::json!({"type":"contacts_sync","protocol":1,"contacts":[]}),
    );
    assert!(
        matches!(wait_contacts(&harness), ContactsEvent::Synced { contacts, .. } if contacts.is_empty())
    );
}

fn wait_download(harness: &Harness, id: &str) -> handover_core::FileDownload {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match harness
            .events
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap()
        {
            StateEvent::FileDownload(download) if download.transfer_id == id => return download,
            _ => {}
        }
    }
}

#[test]
fn requested_download_requires_local_save_and_reports_permission_and_disconnect_failures() {
    use handover_core::{Capability, FileDownloadFailure, FileDownloadState};
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);
    send(
        &mut peer,
        serde_json::json!({"type":"notifications_sync","protocol":1,"enabled":false,"notifications":[]}),
    );
    let device = loop {
        if let StateEvent::Device(DeviceEvent::Added(device)) =
            harness.events.recv_timeout(Duration::from_secs(5)).unwrap()
        {
            break device;
        }
    };
    assert!(device.capabilities.contains(&Capability::FileDownload));
    for path in [
        ".",
        "../secret",
        "/absolute",
        "folder//file",
        "folder/../file",
        "file\n.cc",
        "folder\\file",
    ] {
        assert!(matches!(
            harness
                .backend
                .request_file(&client.fingerprint, path.into()),
            Err(handover_native::NativeCommandError::InvalidResource)
        ));
    }
    let queued = harness
        .backend
        .request_file(&client.fingerprint, "Code/original ✓.cc".into())
        .unwrap();
    assert_eq!(queued.state, FileDownloadState::Queued);
    assert!(
        harness
            .backend
            .request_file(&client.fingerprint, queued.source_path.clone())
            .is_err()
    );
    let request = recv(&mut peer);
    assert_eq!(request["type"], "file_request");
    assert_eq!(request["transfer_id"], queued.transfer_id);
    assert_eq!(request["path"], queued.source_path);
    // A remote completion receipt cannot prove that the desktop saved anything.
    send(
        &mut peer,
        serde_json::json!({"type":"share_result","protocol":1,"transfer_id":queued.transfer_id,"status":"completed"}),
    );
    let bytes = b"// original source\nint main() { return 0; }\n\0\xff";
    send(
        &mut peer,
        serde_json::json!({"type":"share_file","protocol":1,"transfer_id":queued.transfer_id,"name":"original ✓.cc","size":bytes.len()}),
    );
    assert_eq!(
        wait_download(&harness, &queued.transfer_id).state,
        FileDownloadState::Receiving
    );
    peer.tls.write_all(bytes).unwrap();
    peer.tls.flush().unwrap();
    let completed = wait_download(&harness, &queued.transfer_id);
    let FileDownloadState::Completed { path } = completed.state else {
        panic!("not saved: {:?}", completed.state);
    };
    assert!(path.ends_with("original ✓.cc"));
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert_eq!(recv(&mut peer)["status"], "completed");

    let denied = harness
        .backend
        .request_file(&client.fingerprint, "permission.txt".into())
        .unwrap();
    assert_eq!(recv(&mut peer)["transfer_id"], denied.transfer_id);
    send(
        &mut peer,
        serde_json::json!({"type":"file_request_failure","protocol":1,"transfer_id":denied.transfer_id,"reason":"permission_required"}),
    );
    assert_eq!(
        wait_download(&harness, &denied.transfer_id).state,
        FileDownloadState::Failed {
            reason: FileDownloadFailure::PermissionRequired
        }
    );
    let abandoned = harness
        .backend
        .request_file(&client.fingerprint, "queued.bin".into())
        .unwrap();
    assert_eq!(recv(&mut peer)["transfer_id"], abandoned.transfer_id);
    drop(peer);
    assert_eq!(
        wait_download(&harness, &abandoned.transfer_id).state,
        FileDownloadState::Failed {
            reason: FileDownloadFailure::Disconnected
        }
    );
}

#[test]
fn older_phone_does_not_advertise_or_accept_requested_downloads() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    let code = exchange_openings_with_downloads(&harness, &mut peer, &client, false);
    assert_eq!(wait_pending_code(&harness, &client.fingerprint), code);
    harness.backend.approve(&client.fingerprint, &code).unwrap();
    send(
        &mut peer,
        serde_json::json!({"type":"pair_confirm","protocol":1,"code":code}),
    );
    assert_eq!(recv(&mut peer)["type"], "paired");
    let device = loop {
        if let StateEvent::Device(DeviceEvent::Added(device)) =
            harness.events.recv_timeout(Duration::from_secs(5)).unwrap()
        {
            break device;
        }
    };
    assert!(
        !device
            .capabilities
            .contains(&handover_core::Capability::FileDownload)
    );
    assert!(matches!(
        harness
            .backend
            .request_file(&client.fingerprint, "file.cc".into()),
        Err(handover_native::NativeCommandError::Unsupported)
    ));
}

#[test]
fn requested_download_rejects_a_different_basename_before_reading_payload() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_client(&harness, &client, &mut peer);
    let queued = harness
        .backend
        .request_file(&client.fingerprint, "Code/source.cc".into())
        .unwrap();
    assert_eq!(recv(&mut peer)["transfer_id"], queued.transfer_id);
    send(
        &mut peer,
        serde_json::json!({"type":"share_file","protocol":1,"transfer_id":queued.transfer_id,"name":"different.bin","size":100}),
    );
    let failed = wait_download(&harness, &queued.transfer_id);
    assert_eq!(
        failed.state,
        handover_core::FileDownloadState::Failed {
            reason: handover_core::FileDownloadFailure::InvalidResource
        }
    );
    recv_err(&mut peer);
}

fn pair_advanced(harness: &Harness, client: &ClientIdentity, peer: &mut TlsPeer) {
    let code = exchange_openings_with_features(harness, peer, client, true, true, false);
    assert_eq!(wait_pending_code(harness, &client.fingerprint), code);
    harness.backend.approve(&client.fingerprint, &code).unwrap();
    send(
        peer,
        serde_json::json!({"type":"pair_confirm","protocol":1,"code":code}),
    );
    assert_eq!(recv(peer)["type"], "paired");
    for expected in [
        "notifications_request",
        "call_request",
        "media_request",
        "contacts_request",
    ] {
        assert_eq!(recv(peer)["type"], expected);
    }
    send(
        peer,
        serde_json::json!({"type":"notifications_sync","protocol":1,"enabled":false,"notifications":[]}),
    );
}
fn terminal_download(harness: &Harness, id: &str) -> handover_core::FileDownload {
    loop {
        let download = wait_download(harness, id);
        if download.state.is_terminal()
            || matches!(
                download.state,
                handover_core::FileDownloadState::Paused { .. }
            )
        {
            return download;
        }
    }
}
fn serve_v2_file(peer: &mut TlsPeer, open: serde_json::Value, bytes: &[u8], out_of_order: bool) {
    serve_v2_file_from(peer, open, bytes, out_of_order, 0)
}
fn serve_v2_file_from(
    peer: &mut TlsPeer,
    open: serde_json::Value,
    bytes: &[u8],
    out_of_order: bool,
    start: usize,
) {
    use base64::Engine;
    assert_eq!(open["type"], "file_open_v2");
    let id = open["transfer_id"].as_str().unwrap();
    let name = open["path"].as_str().unwrap().rsplit('/').next().unwrap();
    send(
        peer,
        serde_json::json!({"type":"file_info_v2","protocol":1,"transfer_id":id,"name":name,"size":bytes.len(),"version":hex::encode(Sha256::digest(bytes))}),
    );
    let mut sent = start;
    while sent < bytes.len() {
        let mut reads = Vec::new();
        let count = if out_of_order {
            4.min((bytes.len() - sent).div_ceil(32768))
        } else {
            1
        };
        for _ in 0..count {
            let request = recv(peer);
            assert_eq!(request["type"], "file_read_v2");
            reads.push(request);
        }
        if out_of_order {
            reads.reverse();
        }
        for request in reads {
            let offset = request["offset"].as_u64().unwrap() as usize;
            let size = request["size"].as_u64().unwrap() as usize;
            send(
                peer,
                serde_json::json!({"type":"file_chunk_v2","protocol":1,"transfer_id":id,"offset":offset,"data":base64::engine::general_purpose::STANDARD.encode(&bytes[offset..offset+size])}),
            );
            sent += size;
        }
    }
    let closed = recv(peer);
    assert_eq!(closed["type"], "file_close_v2");
    assert_eq!(closed["transfer_id"], id);
}
#[test]
fn chunked_download_verifies_original_bytes_with_out_of_order_chunks() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_advanced(&harness, &client, &mut peer);
    let download = harness
        .backend
        .request_file(
            &client.fingerprint,
            "Internal storage/Code/original ✓.cc".into(),
        )
        .unwrap();
    let bytes: Vec<_> = (0..(4 * 32768 + 333)).map(|i| (i % 251) as u8).collect();
    let open = recv(&mut peer);
    serve_v2_file(&mut peer, open, &bytes, true);
    let completed = terminal_download(&harness, &download.transfer_id);
    let handover_core::FileDownloadState::Completed { path } = completed.state else {
        panic!("download did not finish: {:?}", completed.state);
    };
    assert!(path.ends_with("original ✓.cc"));
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert_eq!(completed.progress.received_bytes, bytes.len() as u64);
    assert_eq!(completed.progress.completed_files, 1);
}
#[test]
fn chunked_download_saves_a_file_above_the_legacy_limit_with_its_original_hash() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_advanced(&harness, &client, &mut peer);
    let download = harness
        .backend
        .request_file(&client.fingerprint, "Internal storage/large.cc".into())
        .unwrap();
    let bytes: Vec<u8> = (0..(101 * 1024 * 1024 + 17))
        .map(|i| (i % 251) as u8)
        .collect();
    let expected = Sha256::digest(&bytes);
    let open = recv(&mut peer);
    serve_v2_file(&mut peer, open, &bytes, true);
    let completed = terminal_download(&harness, &download.transfer_id);
    let handover_core::FileDownloadState::Completed { path } = completed.state else {
        panic!("large download did not finish: {:?}", completed.state);
    };
    assert!(path.ends_with("large.cc"));
    let mut file = std::fs::File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    assert_eq!(hash.finalize(), expected);
    assert_eq!(completed.progress.received_bytes, bytes.len() as u64);
}
#[test]
fn directory_download_walks_pages_and_keeps_empty_directories_and_original_files() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_advanced(&harness, &client, &mut peer);
    let download = harness
        .backend
        .request_download(
            &client.fingerprint,
            "Internal storage/Photos".into(),
            handover_core::FileDownloadKind::Directory,
        )
        .unwrap();
    let first = recv(&mut peer);
    assert_eq!(first["type"], "filesystem_list");
    assert_eq!(first["paged"], true);
    send(
        &mut peer,
        serde_json::json!({"type":"filesystem_entries","protocol":1,"request_id":first["request_id"],"path":first["path"],"cursor_next":"page2","entries":[{"name":"empty","directory":true},{"name":"one.cc","directory":false,"size":3}]}),
    );
    let second = recv(&mut peer);
    assert_eq!(second["cursor"], "page2");
    send(
        &mut peer,
        serde_json::json!({"type":"filesystem_entries","protocol":1,"request_id":second["request_id"],"path":second["path"],"entries":[{"name":"two.heic","directory":false,"size":4}]}),
    );
    let nested = recv(&mut peer);
    assert!(nested["path"].as_str().unwrap().ends_with("/empty"));
    send(
        &mut peer,
        serde_json::json!({"type":"filesystem_entries","protocol":1,"request_id":nested["request_id"],"path":nested["path"],"entries":[]}),
    );
    let open = recv(&mut peer);
    serve_v2_file(&mut peer, open, b"one", false);
    let open = recv(&mut peer);
    serve_v2_file(&mut peer, open, b"two!", false);
    let completed = terminal_download(&harness, &download.transfer_id);
    let handover_core::FileDownloadState::Completed { path } = completed.state else {
        panic!("folder did not finish: {:?}", completed.state);
    };
    let root = std::path::Path::new(&path);
    assert!(root.join("empty").is_dir());
    assert_eq!(std::fs::read(root.join("one.cc")).unwrap(), b"one");
    assert_eq!(std::fs::read(root.join("two.heic")).unwrap(), b"two!");
    assert_eq!(completed.progress.total_files, Some(2));
    assert_eq!(completed.progress.completed_files, 2);
}
#[test]
fn resumed_folder_listing_discards_directories_from_the_abandoned_manifest() {
    use handover_core::{DownloadControl, FileDownloadKind, FileDownloadState};
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_advanced(&harness, &client, &mut peer);
    let download = harness
        .backend
        .request_download(
            &client.fingerprint,
            "Internal storage/Folder".into(),
            FileDownloadKind::Directory,
        )
        .unwrap();
    let first = recv(&mut peer);
    send(
        &mut peer,
        serde_json::json!({"type":"filesystem_entries","protocol":1,"request_id":first["request_id"],"path":first["path"],"cursor_next":"next-page","entries":[{"name":"removed-later","directory":true}]}),
    );
    let next = recv(&mut peer);
    send(
        &mut peer,
        serde_json::json!({"type":"filesystem_failure","protocol":1,"request_id":next["request_id"],"path":next["path"],"reason":"unavailable"}),
    );
    assert!(matches!(
        terminal_download(&harness, &download.transfer_id).state,
        FileDownloadState::Paused { .. }
    ));
    harness
        .backend
        .control_download(
            &client.fingerprint,
            &download.transfer_id,
            DownloadControl::Resume,
        )
        .unwrap();
    let resumed = loop {
        let frame = recv(&mut peer);
        if frame["type"] == "filesystem_list" {
            break frame;
        }
    };
    assert!(resumed.get("cursor").is_none());
    send(
        &mut peer,
        serde_json::json!({"type":"filesystem_entries","protocol":1,"request_id":resumed["request_id"],"path":resumed["path"],"entries":[{"name":"current.cc","directory":false,"size":3}]}),
    );
    let open = recv(&mut peer);
    serve_v2_file(&mut peer, open, b"now", false);
    let FileDownloadState::Completed { path } =
        terminal_download(&harness, &download.transfer_id).state
    else {
        panic!("resumed folder did not complete");
    };
    assert!(!std::path::Path::new(&path).join("removed-later").exists());
    assert_eq!(
        std::fs::read(std::path::Path::new(&path).join("current.cc")).unwrap(),
        b"now"
    );
}
#[test]
fn large_download_can_be_cancelled_and_late_chunks_cannot_recreate_it() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_advanced(&harness, &client, &mut peer);
    let download = harness
        .backend
        .request_file(&client.fingerprint, "Internal storage/large.bin".into())
        .unwrap();
    let open = recv(&mut peer);
    send(
        &mut peer,
        serde_json::json!({"type":"file_info_v2","protocol":1,"transfer_id":open["transfer_id"],"name":"large.bin","size":101*1024*1024,"version":"a".repeat(64)}),
    );
    let first = recv(&mut peer);
    assert_eq!(first["type"], "file_read_v2");
    let cancelled = harness
        .backend
        .control_download(
            &client.fingerprint,
            &download.transfer_id,
            handover_core::DownloadControl::Cancel,
        )
        .unwrap();
    assert_eq!(cancelled.state, handover_core::FileDownloadState::Cancelled);
    send(
        &mut peer,
        serde_json::json!({"type":"file_chunk_v2","protocol":1,"transfer_id":open["transfer_id"],"offset":0,"data":"aWdub3JlZA=="}),
    );
    send(&mut peer, serde_json::json!({"type":"ping","protocol":1}));
    loop {
        let frame = recv(&mut peer);
        if frame["type"] == "pong" {
            break;
        }
    }
    assert_eq!(
        harness
            .backend
            .restored_downloads()
            .into_iter()
            .find(|d| d.transfer_id == download.transfer_id)
            .unwrap()
            .state,
        handover_core::FileDownloadState::Cancelled
    );
}

#[test]
fn partial_download_resumes_from_a_durable_checkpoint_after_daemon_restart() {
    use base64::Engine;
    use handover_core::{DownloadControl, FileDownloadState};
    let first = harness();
    let client = test_identity();
    let mut peer = connect(first.port, &client);
    pair_advanced(&first, &client, &mut peer);
    let download = first
        .backend
        .request_file(&client.fingerprint, "Internal storage/resume.bin".into())
        .unwrap();
    let open = recv(&mut peer);
    let bytes: Vec<_> = (0..100_123).map(|i| (i % 239) as u8).collect();
    let version = hex::encode(Sha256::digest(&bytes));
    send(
        &mut peer,
        serde_json::json!({"type":"file_info_v2","protocol":1,"transfer_id":open["transfer_id"],"name":"resume.bin","size":bytes.len(),"version":version}),
    );
    let chunk = recv(&mut peer);
    assert_eq!(chunk["offset"], 0);
    send(
        &mut peer,
        serde_json::json!({"type":"file_chunk_v2","protocol":1,"transfer_id":open["transfer_id"],"offset":0,"data":base64::engine::general_purpose::STANDARD.encode(&bytes[..32768])}),
    );
    loop {
        let state = wait_download(&first, &download.transfer_id);
        if state.progress.received_bytes >= 32768 {
            break;
        }
    }
    drop(peer);
    let paused = terminal_download(&first, &download.transfer_id);
    assert!(matches!(paused.state, FileDownloadState::Paused { .. }));
    assert_eq!(paused.progress.received_bytes, 32768);
    let backend = NativeBackend::open(first.directory.clone()).unwrap();
    let restored = backend.restored_downloads();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].transfer_id, download.transfer_id);
    assert_eq!(restored[0].progress.received_bytes, 32768);
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    let server = backend.clone();
    std::thread::spawn(move || {
        let _ = server.serve(
            listener,
            Arc::new(move |event| {
                let _ = tx.send(event);
            }),
        );
    });
    let second = Harness {
        directory: first.directory.clone(),
        backend,
        port,
        events: rx,
    };
    let mut peer = connect(port, &client);
    send(
        &mut peer,
        serde_json::json!({"type":"hello","protocol":1,"id":client.fingerprint,"name":"Test Phone","trusted_server_id":second.backend.identity(),"file_downloads":true,"files_v2":true}),
    );
    assert_eq!(recv(&mut peer)["type"], "hello");
    assert_eq!(recv(&mut peer)["type"], "paired");
    for kind in [
        "notifications_request",
        "call_request",
        "media_request",
        "contacts_request",
    ] {
        assert_eq!(recv(&mut peer)["type"], kind);
    }
    send(
        &mut peer,
        serde_json::json!({"type":"notifications_sync","protocol":1,"enabled":false,"notifications":[]}),
    );
    second
        .backend
        .control_download(
            &client.fingerprint,
            &download.transfer_id,
            DownloadControl::Resume,
        )
        .unwrap();
    let resumed = recv(&mut peer);
    assert_eq!(resumed["expected_version"], version);
    assert_ne!(resumed["transfer_id"], open["transfer_id"]);
    serve_v2_file_from(&mut peer, resumed, &bytes, false, 32768);
    let complete = terminal_download(&second, &download.transfer_id);
    let FileDownloadState::Completed { path } = complete.state else {
        panic!("resume did not complete: {:?}", complete.state);
    };
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
#[test]
fn changed_source_cannot_be_spliced_onto_partial_bytes() {
    use base64::Engine;
    use handover_core::{DownloadControl, FileDownloadFailure, FileDownloadState};
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    pair_advanced(&harness, &client, &mut peer);
    let download = harness
        .backend
        .request_file(&client.fingerprint, "Internal storage/changed.bin".into())
        .unwrap();
    let open = recv(&mut peer);
    let bytes = vec![7u8; 65536];
    send(
        &mut peer,
        serde_json::json!({"type":"file_info_v2","protocol":1,"transfer_id":open["transfer_id"],"name":"changed.bin","size":bytes.len(),"version":hex::encode(Sha256::digest(&bytes))}),
    );
    assert_eq!(recv(&mut peer)["type"], "file_read_v2");
    send(
        &mut peer,
        serde_json::json!({"type":"file_chunk_v2","protocol":1,"transfer_id":open["transfer_id"],"offset":0,"data":base64::engine::general_purpose::STANDARD.encode(&bytes[..32768])}),
    );
    loop {
        let state = wait_download(&harness, &download.transfer_id);
        if state.progress.received_bytes >= 32768 {
            break;
        }
    }
    send(
        &mut peer,
        serde_json::json!({"type":"file_error_v2","protocol":1,"transfer_id":open["transfer_id"],"reason":"interrupted"}),
    );
    let paused = terminal_download(&harness, &download.transfer_id);
    assert!(matches!(paused.state, FileDownloadState::Paused { .. }));
    harness
        .backend
        .control_download(
            &client.fingerprint,
            &download.transfer_id,
            DownloadControl::Resume,
        )
        .unwrap();
    let resumed = loop {
        let next = recv(&mut peer);
        if next["type"] == "file_open_v2" {
            break next;
        }
    };
    send(
        &mut peer,
        serde_json::json!({"type":"file_info_v2","protocol":1,"transfer_id":resumed["transfer_id"],"name":"changed.bin","size":bytes.len(),"version":"0".repeat(64)}),
    );
    assert_eq!(
        terminal_download(&harness, &download.transfer_id).state,
        FileDownloadState::Failed {
            reason: FileDownloadFailure::SourceChanged
        }
    );
}

#[test]
fn volume_commands_use_real_reports_and_match_results_without_optimistic_state() {
    use handover_core::{DeviceVolumeFailure, VolumeKind};
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);
    let code = exchange_openings_with_features(&harness, &mut peer, &client, true, false, true);
    assert_eq!(wait_pending_code(&harness, &client.fingerprint), code);
    harness.backend.approve(&client.fingerprint, &code).unwrap();
    send(
        &mut peer,
        serde_json::json!({"type":"pair_confirm","protocol":1,"code":code}),
    );
    assert_eq!(recv(&mut peer)["type"], "paired");
    assert_eq!(recv(&mut peer)["type"], "volume_request");
    for expected in [
        "notifications_request",
        "call_request",
        "media_request",
        "contacts_request",
    ] {
        assert_eq!(recv(&mut peer)["type"], expected);
    }
    send(
        &mut peer,
        serde_json::json!({"type":"notifications_sync","protocol":1,"enabled":false,"notifications":[]}),
    );
    let state = |level| serde_json::json!({"type":"volume_state","protocol":1,"streams":[{"kind":"media","min_level":0,"max_level":15,"level":level,"muted":false,"adjustable":true}]});
    send(&mut peer, state(4));
    loop {
        if let StateEvent::DeviceVolume(state) =
            harness.events.recv_timeout(Duration::from_secs(5)).unwrap()
        {
            assert_eq!(state.streams[0].level, 4);
            break;
        }
    }
    let request = harness
        .backend
        .set_volume(&client.fingerprint, VolumeKind::Media, 7)
        .unwrap();
    let command = recv(&mut peer);
    assert_eq!(command["type"], "volume_set");
    assert_eq!(command["request_id"], request);
    assert_eq!(command["kind"], "media");
    assert_eq!(command["level"], 7);
    assert_eq!(
        harness.backend.volume_states()[0].streams[0].level,
        4,
        "queue acceptance must not invent a new volume"
    );
    send(
        &mut peer,
        serde_json::json!({"type":"volume_result","protocol":1,"request_id":request,"accepted":true}),
    );
    send(&mut peer, state(7));
    loop {
        if let StateEvent::DeviceVolume(state) =
            harness.events.recv_timeout(Duration::from_secs(5)).unwrap()
        {
            assert_eq!(state.streams[0].level, 7);
            break;
        }
    }
    assert!(
        harness
            .backend
            .set_volume(&client.fingerprint, VolumeKind::Media, 16)
            .is_err()
    );
    let lost = harness
        .backend
        .set_volume(&client.fingerprint, VolumeKind::Media, 6)
        .unwrap();
    assert_eq!(recv(&mut peer)["request_id"], lost);
    drop(peer);
    loop {
        if let StateEvent::DeviceVolumeResult(result) =
            harness.events.recv_timeout(Duration::from_secs(5)).unwrap()
            && result.request_id == lost
        {
            assert!(!result.accepted);
            assert_eq!(result.failure, Some(DeviceVolumeFailure::Unconfirmed));
            break;
        }
    }
}
