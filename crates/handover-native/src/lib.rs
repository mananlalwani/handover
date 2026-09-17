//! Native Android transport. TLS authenticates a persistent certificate; DNS-SD only locates us.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use handover_core::{BatteryState, Capability, Device, DeviceEvent, DeviceId, StateEvent};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use openssl::asn1::Asn1Time;
use openssl::bn::{BigNum, MsbOption};
use openssl::ec::{EcGroup, EcKey};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Private};
use openssl::ssl::{SslAcceptor, SslMethod, SslVerifyMode, SslVersion};
use openssl::x509::{X509, X509NameBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::warn;

pub const WIRE_VERSION: u32 = 1;
pub const MAX_FRAME: usize = 64 * 1024;
const MAX_SESSIONS: usize = 16;
const LISTEN_PORT: u16 = 24837;

#[derive(Debug, Error)]
pub enum NativeError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("TLS: {0}")]
    Tls(#[from] openssl::error::ErrorStack),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid peer frame")]
    InvalidFrame,
    #[error("unknown pending peer or comparison code")]
    UnknownPending,
    #[error("discovery: {0}")]
    Discovery(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PendingPeer {
    pub id: String,
    pub name: String,
    pub code: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct PeerFile {
    peers: BTreeMap<String, Peer>,
}

struct Candidate {
    peer: Peer,
    code: String,
    commit: String,
    approved: bool,
    created: Instant,
}
struct Runtime {
    peers: PeerFile,
    pending: BTreeMap<String, Candidate>,
    active: BTreeMap<String, TcpStream>,
    sessions: usize,
    connecting: BTreeSet<String>,
    batteries: BTreeMap<String, BatteryState>,
}

#[derive(Clone)]
pub struct NativeBackend {
    inner: Arc<Mutex<Runtime>>,
    directory: PathBuf,
    certificate: X509,
    key: PKey<Private>,
    id: String,
}

// Serializes session teardown with peer admission so an old disconnect cannot
// overwrite a replacement connection's presence.
struct Session<'a> {
    backend: &'a NativeBackend,
    peer: Peer,
    event: &'a Arc<dyn Fn(StateEvent) + Send + Sync>,
    published: bool,
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        let mut inner = self.backend.inner.lock().unwrap();
        inner.connecting.remove(&self.peer.id);
        inner.pending.remove(&self.peer.id);
        if self.published {
            inner.active.remove(&self.peer.id);
            let event = if inner.peers.peers.contains_key(&self.peer.id) {
                DeviceEvent::Updated(device(
                    &self.peer,
                    false,
                    inner.batteries.get(&self.peer.id).cloned(),
                ))
            } else {
                DeviceEvent::Removed(device(&self.peer, false, None).id)
            };
            (self.event)(StateEvent::Device(event));
        }
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message {
    Hello {
        protocol: u32,
        id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trusted_server_id: Option<String>,
        // Hex SHA-256 commitment to the sender's fresh pairing nonce. Present
        // on every hello; required from unknown peers so the comparison code
        // binds this ceremony instead of only the long-lived certificates.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pair_commit: Option<String>,
    },
    PairOpen {
        protocol: u32,
        // Hex 16-byte nonce revealing the hello's commitment.
        nonce: String,
    },
    PairConfirm {
        protocol: u32,
        // The ceremony code the phone user approved. The server verifies it
        // against the pending candidate; a confirmation that does not repeat
        // the displayed code aborts pairing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    Paired {
        protocol: u32,
    },
    Battery {
        protocol: u32,
        percentage: u8,
        charging: bool,
    },
    Revoke {
        protocol: u32,
    },
    Ping {
        protocol: u32,
    },
    Pong {
        protocol: u32,
    },
}

impl Message {
    fn version(&self) -> u32 {
        match self {
            Self::Hello { protocol, .. }
            | Self::PairOpen { protocol, .. }
            | Self::PairConfirm { protocol, .. }
            | Self::Paired { protocol }
            | Self::Battery { protocol, .. }
            | Self::Revoke { protocol }
            | Self::Ping { protocol }
            | Self::Pong { protocol } => *protocol,
        }
    }
}

impl NativeBackend {
    pub fn open(directory: PathBuf) -> Result<Self, NativeError> {
        fs::create_dir_all(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        let key_path = directory.join("identity.key");
        let cert_path = directory.join("identity.crt");
        let (key, certificate) = if key_path.exists() && cert_path.exists() {
            (
                PKey::private_key_from_pem(&fs::read(&key_path)?)?,
                X509::from_pem(&fs::read(&cert_path)?)?,
            )
        } else {
            let (key, cert) = generate_identity()?;
            write_private(&key_path, &key.private_key_to_pem_pkcs8()?)?;
            write_private(&cert_path, &cert.to_pem()?)?;
            (key, cert)
        };
        let id = fingerprint(&certificate)?;
        let peers_path = directory.join("peers.json");
        let peers = if peers_path.exists() {
            serde_json::from_slice(&fs::read(peers_path)?)?
        } else {
            PeerFile::default()
        };
        Ok(Self {
            inner: Arc::new(Mutex::new(Runtime {
                peers,
                pending: BTreeMap::new(),
                active: BTreeMap::new(),
                sessions: 0,
                connecting: BTreeSet::new(),
                batteries: BTreeMap::new(),
            })),
            directory,
            certificate,
            key,
            id,
        })
    }

    pub fn default_directory() -> Result<PathBuf, NativeError> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "HOME or XDG_STATE_HOME is required",
                )
            })?;
        Ok(base.join("handover/native"))
    }

    pub fn identity(&self) -> &str {
        &self.id
    }
    pub fn peers(&self) -> Vec<Peer> {
        self.inner
            .lock()
            .unwrap()
            .peers
            .peers
            .values()
            .cloned()
            .collect()
    }
    pub fn remembered_devices(&self) -> Vec<Device> {
        self.peers()
            .iter()
            .map(|peer| device(peer, false, None))
            .collect()
    }
    pub fn pending(&self) -> Vec<PendingPeer> {
        let mut inner = self.inner.lock().unwrap();
        inner
            .pending
            .retain(|_, candidate| candidate.created.elapsed() < Duration::from_secs(120));
        inner
            .pending
            .values()
            .map(|c| PendingPeer {
                id: c.peer.id.clone(),
                name: c.peer.name.clone(),
                code: c.code.clone(),
            })
            .collect()
    }
    pub fn approve(&self, id: &str, code: &str) -> Result<(), NativeError> {
        let mut inner = self.inner.lock().unwrap();
        inner
            .pending
            .retain(|_, candidate| candidate.created.elapsed() < Duration::from_secs(120));
        let candidate = inner
            .pending
            .get_mut(id)
            .ok_or(NativeError::UnknownPending)?;
        if candidate.code.is_empty() || candidate.code != code {
            // The code only exists after both sides reveal their pairing
            // nonces; there is nothing to approve before that.
            return Err(NativeError::UnknownPending);
        }
        candidate.approved = true;
        Ok(())
    }
    pub fn unpair(&self, id: &str) -> Result<bool, NativeError> {
        let mut inner = self.inner.lock().unwrap();
        inner.pending.remove(id);
        let mut peers = inner.peers.clone();
        let removed = peers.peers.remove(id).is_some();
        if removed {
            self.save_peers(&peers)?;
            inner.peers = peers;
            inner.batteries.remove(id);
        }
        if let Some(stream) = inner.active.remove(id) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        Ok(removed)
    }
    fn save_peers(&self, peers: &PeerFile) -> Result<(), NativeError> {
        let path = self.directory.join("peers.json");
        let tmp = self.directory.join("peers.json.tmp");
        write_private(&tmp, &serde_json::to_vec(peers)?)?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn run(self, event: Arc<dyn Fn(StateEvent) + Send + Sync>) -> Result<(), NativeError> {
        let listener = TcpListener::bind(("0.0.0.0", LISTEN_PORT))?;
        let port = listener.local_addr()?.port();
        // Keep the advertisement alive for the lifetime of the listener.
        let _discovery = advertise(port, &self.id)?;
        self.serve(listener, event)
    }

    /// Serves the native protocol on an already-bound listener without DNS-SD
    /// advertisement. The pairing/TLS interop harness and integration tests
    /// use this with an ephemeral port; production callers use [`Self::run`].
    pub fn serve(
        self,
        listener: TcpListener,
        event: Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        for incoming in listener.incoming() {
            let stream = incoming?;
            let mut inner = self.inner.lock().unwrap();
            if inner.sessions >= MAX_SESSIONS {
                continue;
            }
            inner.sessions += 1;
            drop(inner);
            let backend = self.clone();
            let event = event.clone();
            std::thread::spawn(move || {
                if let Err(error) = backend.handle(stream, &event) {
                    tracing::debug!(%error, "native connection ended");
                }
                backend.inner.lock().unwrap().sessions -= 1;
            });
        }
        Ok(())
    }

    fn handle(
        &self,
        stream: TcpStream,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut builder = SslAcceptor::mozilla_modern_v5(SslMethod::tls())?;
        builder.set_min_proto_version(Some(SslVersion::TLS1_3))?;
        builder.set_certificate(&self.certificate)?;
        builder.set_private_key(&self.key)?;
        builder.set_verify_callback(
            SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT,
            |_preverified, _context| true,
        );
        let acceptor = builder.build();
        let mut tls = match acceptor.accept(stream) {
            Ok(tls) => tls,
            Err(error) => {
                warn!(%error, "native TLS handshake failed");
                return Err(NativeError::InvalidFrame);
            }
        };
        let cert = tls
            .ssl()
            .peer_certificate()
            .ok_or(NativeError::InvalidFrame)?;
        let peer_fp = fingerprint(&cert)?;
        let hello = read_frame(&mut tls)?;
        let Message::Hello {
            protocol: WIRE_VERSION,
            id,
            name,
            trusted_server_id,
            pair_commit,
        } = hello
        else {
            return Err(NativeError::InvalidFrame);
        };
        if id != peer_fp || name.is_empty() || name.len() > 128 {
            return Err(NativeError::InvalidFrame);
        }
        // Every ceremony gets a fresh nonce. The hello carries only its
        // commitment; the opening follows, so a middlebox that committed to
        // its certificates at the TLS handshake cannot grind the displayed
        // code offline before the users compare it.
        let mut nonce = [0u8; 16];
        openssl::rand::rand_bytes(&mut nonce)?;
        let nonce_hex = hex::encode(nonce);
        write_frame(
            &mut tls,
            &Message::Hello {
                protocol: WIRE_VERSION,
                id: self.id.clone(),
                name: "Linux desktop".into(),
                trusted_server_id: None,
                pair_commit: Some(hex::encode(Sha256::digest(nonce))),
            },
        )?;
        let peer = Peer {
            id: id.clone(),
            name: name.clone(),
            fingerprint: peer_fp.clone(),
        };
        if trusted_server_id.as_deref() == Some(self.id.as_str())
            && !self.inner.lock().unwrap().peers.peers.contains_key(&id)
        {
            write_frame(
                &mut tls,
                &Message::Revoke {
                    protocol: WIRE_VERSION,
                },
            )?;
            return Ok(());
        }
        let known = {
            let mut inner = self.inner.lock().unwrap();
            if inner.active.contains_key(&id) || !inner.connecting.insert(id.clone()) {
                // A live session for this peer must not be overwritten by a
                // second concurrent connection with the same identity.
                return Err(NativeError::InvalidFrame);
            }
            let known = inner
                .peers
                .peers
                .get(&id)
                .is_some_and(|p| p.fingerprint == peer_fp);
            if !known {
                inner
                    .pending
                    .retain(|_, candidate| candidate.created.elapsed() < Duration::from_secs(120));
                if inner.pending.len() >= MAX_SESSIONS {
                    inner.connecting.remove(&id);
                    return Err(NativeError::InvalidFrame);
                }
                let Some(commit) = pair_commit else {
                    // Unknown peers must commit to a fresh nonce; without it
                    // the ceremony code would be a static function of the
                    // certificates and grindable offline by a middlebox.
                    inner.connecting.remove(&id);
                    return Err(NativeError::InvalidFrame);
                };
                if !hex::decode(&commit).is_ok_and(|bytes| bytes.len() == 32) {
                    inner.connecting.remove(&id);
                    return Err(NativeError::InvalidFrame);
                }
                inner.pending.insert(
                    id.clone(),
                    Candidate {
                        peer: peer.clone(),
                        // Computed once the phone reveals its nonce.
                        code: String::new(),
                        commit,
                        approved: false,
                        created: Instant::now(),
                    },
                );
            }
            known
        };
        let mut session = Session {
            backend: self,
            peer: peer.clone(),
            event,
            published: false,
        };
        if !known {
            // Created after the session guard so a failed opening write
            // still releases the identity slot for a fresh ceremony.
            write_frame(
                &mut tls,
                &Message::PairOpen {
                    protocol: WIRE_VERSION,
                    nonce: nonce_hex.clone(),
                },
            )?;
        }
        if !known {
            // Both sides committed to a fresh nonce in their hellos and have
            // now revealed the openings. Each side verifies the peer's
            // opening against its commitment, then both users compare the
            // resulting code out of band and approve on their own side: Linux
            // through local IPC, the phone by repeating the code it
            // displayed. Pairing completes only when the local approval and
            // the matching phone confirmation meet within one ceremony.
            let started = Instant::now();
            let mut opened = false;
            let mut phone_confirmed = false;
            loop {
                if started.elapsed() > Duration::from_secs(120) {
                    return Err(NativeError::InvalidFrame);
                }
                match read_frame(&mut tls) {
                    Ok(Message::PairOpen {
                        protocol: WIRE_VERSION,
                        nonce: peer_nonce,
                    }) => {
                        if opened {
                            return Err(NativeError::InvalidFrame);
                        }
                        let Some(bytes) = parse_nonce(&peer_nonce) else {
                            return Err(NativeError::InvalidFrame);
                        };
                        let mut inner = self.inner.lock().unwrap();
                        let Some(candidate) = inner.pending.get_mut(&id) else {
                            return Err(NativeError::InvalidFrame);
                        };
                        if hex::encode(Sha256::digest(bytes)) != candidate.commit {
                            // The opening does not match the hello's
                            // commitment: drop the ceremony instead of
                            // displaying a code the peer did not commit to.
                            return Err(NativeError::InvalidFrame);
                        }
                        candidate.code =
                            comparison_code(&self.id, &nonce_hex, &peer_fp, &peer_nonce);
                        opened = true;
                    }
                    Ok(Message::PairConfirm {
                        protocol: WIRE_VERSION,
                        code,
                    }) => {
                        let expected = self
                            .inner
                            .lock()
                            .unwrap()
                            .pending
                            .get(&id)
                            .map_or(String::new(), |candidate| candidate.code.clone());
                        if expected.is_empty() || code.as_deref() != Some(expected.as_str()) {
                            // A confirmation that does not repeat the displayed
                            // ceremony code is a malfunction or an attack. Drop
                            // the session so any retry starts a fresh,
                            // user-visible ceremony instead of allowing
                            // unlimited guesses against this one.
                            return Err(NativeError::InvalidFrame);
                        }
                        phone_confirmed = true;
                    }
                    Ok(Message::Ping {
                        protocol: WIRE_VERSION,
                    }) => write_frame(
                        &mut tls,
                        &Message::Pong {
                            protocol: WIRE_VERSION,
                        },
                    )?,
                    Err(NativeError::Io(e))
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut => {}
                    _ => return Err(NativeError::InvalidFrame),
                }
                let mut inner = self.inner.lock().unwrap();
                if phone_confirmed && inner.pending.get(&id).is_some_and(|p| p.approved) {
                    let mut peers = inner.peers.clone();
                    peers.peers.insert(id.clone(), peer.clone());
                    self.save_peers(&peers)?;
                    inner.peers = peers;
                    inner.pending.remove(&id);
                    break;
                }
            }
        }
        write_frame(
            &mut tls,
            &Message::Paired {
                protocol: WIRE_VERSION,
            },
        )?;
        {
            let mut inner = self.inner.lock().unwrap();
            if !inner.peers.peers.contains_key(&id) {
                return Err(NativeError::InvalidFrame);
            }
            inner.active.insert(id.clone(), tls.get_ref().try_clone()?);
            session.published = true;
            event(StateEvent::Device(DeviceEvent::Added(device(
                &peer,
                true,
                inner.batteries.get(&id).cloned(),
            ))));
        }
        let mut last_received = Instant::now();
        loop {
            if last_received.elapsed() > Duration::from_secs(90) {
                break;
            }
            if !self.inner.lock().unwrap().peers.peers.contains_key(&id) {
                break;
            }
            match read_frame(&mut tls) {
                Ok(Message::Battery {
                    protocol: WIRE_VERSION,
                    percentage,
                    charging,
                }) => {
                    last_received = Instant::now();
                    let battery = BatteryState::new(percentage, charging)
                        .map_err(|_| NativeError::InvalidFrame)?;
                    let mut inner = self.inner.lock().unwrap();
                    if !inner.peers.peers.contains_key(&id) {
                        break;
                    }
                    inner.batteries.insert(id.clone(), battery);
                    event(StateEvent::Device(DeviceEvent::Updated(device(
                        &peer,
                        true,
                        Some(battery),
                    ))));
                }
                Ok(Message::Ping {
                    protocol: WIRE_VERSION,
                }) => {
                    last_received = Instant::now();
                    write_frame(
                        &mut tls,
                        &Message::Pong {
                            protocol: WIRE_VERSION,
                        },
                    )?;
                }
                Ok(Message::Revoke {
                    protocol: WIRE_VERSION,
                }) => {
                    self.unpair(&id)?;
                    break;
                }
                Err(NativeError::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                _ => break,
            }
        }
        Ok(())
    }
}

fn device(peer: &Peer, connected: bool, battery: Option<BatteryState>) -> Device {
    Device {
        id: DeviceId::new(format!("native:{}", peer.id)),
        name: peer.name.clone(),
        connected,
        paired: true,
        battery,
        capabilities: [Capability::Battery].into(),
    }
}

/// Registers the `_handover._tcp.local.` advertisement for a bound port and
/// returns the daemon handle, which keeps the record alive while it is held.
fn advertise(port: u16, id: &str) -> Result<ServiceDaemon, NativeError> {
    let mdns = ServiceDaemon::new().map_err(|e| NativeError::Discovery(e.to_string()))?;
    mdns.register(discovery_service(port, id)?)
        .map_err(|e| NativeError::Discovery(e.to_string()))?;
    Ok(mdns)
}

/// Builds the DNS-SD record advertised by [`NativeBackend::run`]. Discovery
/// addresses and TXT values are untrusted hints; trust comes from the TLS
/// certificate fingerprint pinned at pairing time.
fn discovery_service(port: u16, id: &str) -> Result<ServiceInfo, NativeError> {
    let name = format!("Handover-{}", &id[..12]);
    Ok(ServiceInfo::new(
        "_handover._tcp.local.",
        &name,
        &format!("{}.local.", name.to_lowercase()),
        "",
        port,
        &[("v", "1")][..],
    )
    .map_err(|e| NativeError::Discovery(e.to_string()))?
    .enable_addr_auto())
}

fn generate_identity() -> Result<(PKey<Private>, X509), NativeError> {
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?;
    let key = PKey::from_ec_key(EcKey::generate(&group)?)?;
    let mut subject = X509NameBuilder::new()?;
    subject.append_entry_by_text("CN", "Handover Linux")?;
    let subject = subject.build();
    let mut cert = X509::builder()?;
    cert.set_version(2)?;
    let mut serial = BigNum::new()?;
    serial.rand(128, MsbOption::MAYBE_ZERO, false)?;
    let serial = serial.to_asn1_integer()?;
    cert.set_serial_number(serial.as_ref())?;
    cert.set_subject_name(&subject)?;
    cert.set_issuer_name(&subject)?;
    cert.set_pubkey(&key)?;
    let not_before = Asn1Time::days_from_now(0)?;
    let not_after = Asn1Time::days_from_now(3650)?;
    cert.set_not_before(not_before.as_ref())?;
    cert.set_not_after(not_after.as_ref())?;
    cert.sign(&key, MessageDigest::sha256())?;
    Ok((key, cert.build()))
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), NativeError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn fingerprint(cert: &X509) -> Result<String, NativeError> {
    Ok(hex::encode(Sha256::digest(cert.to_der()?)))
}
fn comparison_code(own_fp: &str, own_nonce: &str, peer_fp: &str, peer_nonce: &str) -> String {
    // Each nonce stays bound to its fingerprint owner, so both sides derive
    // the same code without roles while a middlebox cannot swap openings.
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
fn parse_nonce(hex_nonce: &str) -> Option<[u8; 16]> {
    hex::decode(hex_nonce).ok()?.try_into().ok()
}
fn read_frame<R: Read>(reader: &mut R) -> Result<Message, NativeError> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len[..1])?;
    reader
        .read_exact(&mut len[1..])
        .map_err(|_| NativeError::InvalidFrame)?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(NativeError::InvalidFrame);
    }
    let mut data = vec![0; len];
    reader
        .read_exact(&mut data)
        .map_err(|_| NativeError::InvalidFrame)?;
    let msg: Message = serde_json::from_slice(&data)?;
    if msg.version() != WIRE_VERSION {
        return Err(NativeError::InvalidFrame);
    }
    Ok(msg)
}
fn write_frame<W: Write>(writer: &mut W, message: &Message) -> Result<(), NativeError> {
    let data = serde_json::to_vec(message)?;
    if data.is_empty() || data.len() > MAX_FRAME {
        return Err(NativeError::InvalidFrame);
    }
    writer.write_all(&(data.len() as u32).to_be_bytes())?;
    writer.write_all(&data)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_record_matches_advertised_service() {
        // Code-level coverage for the DNS-SD advertisement. Multicast
        // discovery has no live-network verification (see DESIGN.md).
        let id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let service = discovery_service(24837, id).unwrap();
        assert_eq!(service.get_type(), "_handover._tcp.local.");
        assert_eq!(
            service.get_fullname(),
            "Handover-0123456789ab._handover._tcp.local."
        );
        assert_eq!(service.get_port(), 24837);
        assert_eq!(
            service.get_properties().get_property_val_str("v"),
            Some("1")
        );
    }
    #[test]
    fn framing_rejects_oversize() {
        let bytes = (MAX_FRAME as u32 + 1).to_be_bytes();
        assert!(matches!(
            read_frame(&mut &bytes[..]),
            Err(NativeError::InvalidFrame)
        ));
    }
    #[test]
    fn framing_rejects_truncated_and_wrong_version_messages() {
        let mut truncated = (6_u32).to_be_bytes().to_vec();
        truncated.extend_from_slice(br#"{"x":"#);
        assert!(matches!(
            read_frame(&mut &truncated[..]),
            Err(NativeError::InvalidFrame)
        ));

        let message = br#"{"type":"ping","protocol":2}"#;
        let mut wrong_version = (message.len() as u32).to_be_bytes().to_vec();
        wrong_version.extend_from_slice(message);
        assert!(matches!(
            read_frame(&mut &wrong_version[..]),
            Err(NativeError::InvalidFrame)
        ));
    }

    #[test]
    fn framing_round_trips_every_fixture_message() {
        let messages: Vec<Message> =
            serde_json::from_str(include_str!("../../../tests/fixtures/native-protocol.json"))
                .unwrap();
        for message in messages {
            let mut bytes = Vec::new();
            write_frame(&mut bytes, &message).unwrap();
            assert_eq!(read_frame(&mut &bytes[..]).unwrap(), message);
        }
    }
    #[test]
    fn code_matches_cross_language_vector_and_is_order_independent() {
        // Shared with NativeTransportTest on the Kotlin side: the same
        // fingerprints and nonces must produce this code in both languages.
        let fp_a = "aa".repeat(32);
        let fp_b = "bb".repeat(32);
        let nonce_a = "00112233445566778899aabbccddeeff";
        let nonce_b = "ffeeddccbbaa99887766554433221100";
        assert_eq!(comparison_code(&fp_a, nonce_a, &fp_b, nonce_b), "18954386");
        assert_eq!(
            comparison_code(&fp_a, nonce_a, &fp_b, nonce_b),
            comparison_code(&fp_b, nonce_b, &fp_a, nonce_a)
        );
    }
    #[test]
    fn identity_persists() {
        let dir = tempfile::tempdir().unwrap();
        let first = NativeBackend::open(dir.path().to_path_buf()).unwrap();
        let second = NativeBackend::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(first.identity(), second.identity());
    }

    #[test]
    fn unpair_revokes_persisted_trust_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let peer = Peer {
            id: "android-device-01".into(),
            name: "Pixel Test".into(),
            fingerprint: "fingerprint".into(),
        };
        fs::write(
            dir.path().join("peers.json"),
            serde_json::to_vec(&serde_json::json!({"peers": {peer.id.clone(): peer}})).unwrap(),
        )
        .unwrap();
        let backend = NativeBackend::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(backend.peers().len(), 1);
        assert!(backend.unpair("android-device-01").unwrap());
        assert!(!backend.unpair("android-device-01").unwrap());
        assert!(
            NativeBackend::open(dir.path().to_path_buf())
                .unwrap()
                .peers()
                .is_empty()
        );
    }
}
