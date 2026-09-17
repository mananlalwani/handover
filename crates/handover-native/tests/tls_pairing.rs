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

use handover_core::{DeviceEvent, StateEvent};
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
        if let Some(candidate) = harness.backend.pending().into_iter().find(|p| p.id == id) {
            return candidate.code;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("peer {id} never appeared in pending");
}

/// v1 ceremony code: SHA-256 over the domain label plus the two sorted
/// certificate fingerprints, truncated to eight decimal digits. The Android
/// `pairingCode` implementation must produce the identical value for the same
/// inputs; the shared cross-language vector lives in the crate unit tests.
fn comparison_code(a: &str, b: &str) -> String {
    let mut ids = [a, b];
    ids.sort_unstable();
    let digest = Sha256::digest(format!("handover-pair-v1:{}:{}", ids[0], ids[1]).as_bytes());
    let number = u32::from_be_bytes(digest[..4].try_into().unwrap()) % 100_000_000;
    format!("{number:08}")
}

#[test]
fn pairing_succeeds_over_real_tls() {
    let harness = harness();
    let client = test_identity();
    let mut peer = connect(harness.port, &client);

    send(
        &mut peer,
        serde_json::json!({"type":"hello","protocol":1,"id":client.fingerprint,"name":"Test Phone"}),
    );
    let hello = recv(&mut peer);
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["protocol"], 1);
    // The server must advertise the fingerprint of the certificate it
    // presented during the TLS handshake, not an unverified string.
    assert_eq!(hello["id"].as_str().unwrap(), peer.server_fingerprint);
    assert_eq!(hello["id"].as_str().unwrap(), harness.backend.identity());

    let code = wait_pending_code(&harness, &client.fingerprint);
    assert_eq!(
        code,
        comparison_code(&client.fingerprint, &peer.server_fingerprint)
    );

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

    send(
        &mut peer,
        serde_json::json!({"type":"hello","protocol":1,"id":client.fingerprint,"name":"Test Phone"}),
    );
    let _ = recv(&mut peer);
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
    send(
        &mut peer,
        serde_json::json!({"type":"hello","protocol":1,"id":client.fingerprint,"name":"Test Phone"}),
    );
    let hello = recv(&mut peer);
    assert_eq!(hello["type"], "hello");
    let code = wait_pending_code(&harness, &client.fingerprint);
    harness.backend.approve(&client.fingerprint, &code).unwrap();
    send(
        &mut peer,
        serde_json::json!({"type":"pair_confirm","protocol":1,"code":code}),
    );
    assert_eq!(recv(&mut peer)["type"], "paired");
    assert_eq!(harness.backend.peers().len(), 1);
}
