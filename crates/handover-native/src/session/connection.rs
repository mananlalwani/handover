use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use handover_core::{
    BatteryState, CallAction, CallCommandResult, CallEvent, CallState, Capability, ClipboardFile,
    ClipboardText, ConnectivityState, Contact, ContactsEvent, DeviceCommandAction,
    DeviceCommandResult, DeviceEvent, DeviceId, FilesystemEntry, FilesystemResult,
    PresentationCommand, ReceivedShare, RemoteInputAction, RemoteInputCommand, ShareFailure,
    ShareProgress, ShareResult, ShareStatus, SharedResource, StateEvent, VolumeCommand,
};
use openssl::ssl::{SslAcceptor, SslMethod, SslVerifyMode, SslVersion};
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::identity::{fingerprint, validate_peer_certificate};
use crate::limits::*;
use crate::pairing::{comparison_code, parse_nonce};
use crate::protocol::*;
use crate::{
    Candidate, NativeBackend, NativeError, Peer, Session, clipboard_temp_path,
    delete_clipboard_temp, device, list_local_directory, native_custom_commands,
    run_native_custom_command, safe_command_name, stream_file_until, take_expired_shares,
};

impl NativeBackend {
    pub(crate) fn handle(
        &self,
        stream: TcpStream,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        // TLS and the first hello are unauthenticated. Keep each socket read
        // bounded while the admission controls cap concurrent attempts.
        stream.set_read_timeout(Some(PREAUTH_READ_TIMEOUT))?;
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
        tls.get_ref()
            .set_read_timeout(Some(Duration::from_secs(2)))?;
        let cert = tls
            .ssl()
            .peer_certificate()
            .ok_or(NativeError::InvalidFrame)?;
        validate_peer_certificate(&cert)?;
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
        let added = {
            let mut inner = self.inner.lock().unwrap();
            if !inner.peers.peers.contains_key(&id) {
                return Err(NativeError::InvalidFrame);
            }
            inner.active.insert(id.clone(), tls.get_ref().try_clone()?);
            session.published = true;
            device(
                &peer,
                true,
                inner.batteries.get(&id).cloned(),
                inner.notif_enabled.get(&id).copied().unwrap_or(false),
                inner.media_enabled.get(&id).copied().unwrap_or(false),
            )
        };
        // The device event emits without the lock held: callbacks may call
        // back into the backend.
        event(StateEvent::Device(DeviceEvent::Added(added)));
        // Ask a freshly paired phone for its current notification list. The
        // phone also syncs proactively after `paired`; the request covers a
        // daemon restart where the phone never saw the pairing transition.
        let _ = write_frame(
            &mut tls,
            &Message::NotificationsRequest {
                protocol: WIRE_VERSION,
            },
        );
        let _ = write_frame(
            &mut tls,
            &Message::CallRequest {
                protocol: WIRE_VERSION,
            },
        );
        // Same recovery for media sessions: a daemon restart must not wait
        // for the next playback change to learn the current players.
        let _ = write_frame(
            &mut tls,
            &Message::MediaRequest {
                protocol: WIRE_VERSION,
            },
        );
        let mut last_received = Instant::now();
        let mut last_ping = Instant::now();
        let session_started = Instant::now();
        let mut snapshot_retry_sent = false;
        loop {
            if last_received.elapsed() > SESSION_IDLE_TIMEOUT {
                break;
            }
            if !self.inner.lock().unwrap().peers.peers.contains_key(&id) {
                break;
            }
            if !snapshot_retry_sent && session_started.elapsed() >= Duration::from_secs(5) {
                let notification_answered = {
                    let inner = self.inner.lock().unwrap();
                    inner.notif_enabled.contains_key(&id)
                };
                if !notification_answered {
                    let _ = write_frame(
                        &mut tls,
                        &Message::NotificationsRequest {
                            protocol: WIRE_VERSION,
                        },
                    );
                }
                snapshot_retry_sent = true;
            }
            let expired = {
                let mut inner = self.inner.lock().unwrap();
                let pending = inner.pending_shares.entry(id.clone()).or_default();
                let expired = take_expired_shares(pending, Instant::now());
                if let Some(queue) = inner.outbox.get_mut(&id) {
                    let mut temps = Vec::new();
                    queue.retain(|message| {
                        let drop_it = match message {
                            Message::ShareUrl { transfer_id, .. }
                            | Message::ShareFile { transfer_id, .. } => {
                                expired.contains(transfer_id)
                            }
                            _ => false,
                        };
                        if drop_it {
                            temps.extend(clipboard_temp_path(message).cloned());
                        }
                        !drop_it
                    });
                    drop(inner);
                    for temp in temps {
                        let _ = fs::remove_file(temp);
                    }
                }
                expired
            };
            for transfer_id in expired {
                event(StateEvent::ShareResult(ShareResult {
                    device_id: DeviceId::new(format!("native:{id}")),
                    transfer_id,
                    status: ShareStatus::Failed,
                    reason: Some(ShareFailure::TimedOut),
                }));
            }
            // Drain queued Linux-to-phone notification commands before
            // blocking on the next inbound frame. Acceptance was already
            // reported over IPC; a write failure ends the session and the
            // phone resyncs on reconnect.
            let outbound = self
                .inner
                .lock()
                .unwrap()
                .outbox
                .remove(&id)
                .unwrap_or_default();
            // A call command queued before the phone's latest report is stale:
            // state re-observation proves a call may have started or ended
            // since the user acted. Dropping it here keeps a delayed command
            // from answering, declining or hanging up a *later* call.
            let latest_generation = self
                .inner
                .lock()
                .unwrap()
                .call_generations
                .get(&id)
                .copied();
            let outbound: Vec<_> = outbound
                .into_iter()
                .filter(|message| match message {
                    Message::CallControl {
                        generation: Some(stamped),
                        ..
                    } => latest_generation == Some(*stamped),
                    _ => true,
                })
                .collect();
            for (index, message) in outbound.iter().enumerate() {
                let share_id = match message {
                    Message::ShareUrl { transfer_id, .. }
                    | Message::ShareFile { transfer_id, .. } => Some(transfer_id),
                    _ => None,
                };
                let started = share_id.and_then(|transfer_id| {
                    self.inner
                        .lock()
                        .unwrap()
                        .pending_shares
                        .get(&id)
                        .and_then(|pending| pending.get(transfer_id))
                        .copied()
                });
                if share_id.is_some() && started.is_none() {
                    continue;
                }
                if let (Some(transfer_id), Some(started)) = (share_id, started)
                    && started.elapsed() >= SHARE_RESULT_TIMEOUT
                {
                    self.inner
                        .lock()
                        .unwrap()
                        .pending_shares
                        .entry(id.clone())
                        .or_default()
                        .remove(transfer_id);
                    delete_clipboard_temp(message);
                    event(StateEvent::ShareResult(ShareResult {
                        device_id: DeviceId::new(format!("native:{id}")),
                        transfer_id: transfer_id.clone(),
                        status: ShareStatus::Failed,
                        reason: Some(ShareFailure::TimedOut),
                    }));
                    continue;
                }
                if let Err(error) = write_frame(&mut tls, message) {
                    tracing::debug!(peer = %id, %error, "native notification command write failed");
                    // The session is over; every unsent queued entry
                    // ends here. Delete their clipboard temps.
                    for remaining in outbound.iter().skip(index) {
                        delete_clipboard_temp(remaining);
                    }
                    return Err(error);
                }
                if let Message::CallControl {
                    request_id, action, ..
                } = message
                    && let Some(action) = match action.as_str() {
                        "place" => Some(CallAction::Place),
                        "answer" => Some(CallAction::Answer),
                        "decline" => Some(CallAction::Decline),
                        "hangup" => Some(CallAction::Hangup),
                        _ => None,
                    }
                {
                    self.inner
                        .lock()
                        .unwrap()
                        .pending_call_results
                        .entry(id.clone())
                        .or_default()
                        .insert(request_id.clone(), action);
                }
                if let Message::ShareFile {
                    path,
                    size,
                    transfer_id,
                    clipboard,
                    ..
                } = message
                {
                    // A send that never completes must not leave its
                    // clipboard temp behind. User files are never
                    // touched; only clipboard-owned temps are cleaned.
                    let temp = clipboard.then(|| path.clone());
                    let cleanup = |temp: &Option<PathBuf>| {
                        if let Some(path) = temp {
                            let _ = fs::remove_file(path);
                        }
                    };
                    let file = File::open(path).inspect_err(|_| cleanup(&temp))?;
                    let metadata = file.metadata().inspect_err(|_| cleanup(&temp))?;
                    if !metadata.is_file() || metadata.len() != *size {
                        cleanup(&temp);
                        return Err(NativeError::InvalidFrame);
                    }
                    let started = started.ok_or(NativeError::InvalidFrame).inspect_err(|_| {
                        cleanup(&temp);
                    })?;
                    let device_id = DeviceId::new(format!("native:{id}"));
                    let mut report_progress = |bytes_sent| {
                        event(StateEvent::ShareProgress(ShareProgress {
                            device_id: device_id.clone(),
                            transfer_id: transfer_id.clone(),
                            bytes_sent,
                            total_bytes: *size,
                        }));
                    };
                    if let Err(error) = stream_file_until(
                        &mut file.take(*size),
                        &mut tls,
                        *size,
                        started,
                        &mut report_progress,
                    ) {
                        if error.kind() == std::io::ErrorKind::TimedOut {
                            self.inner
                                .lock()
                                .unwrap()
                                .pending_shares
                                .entry(id.clone())
                                .or_default()
                                .remove(transfer_id);
                            event(StateEvent::ShareResult(ShareResult {
                                device_id: DeviceId::new(format!("native:{id}")),
                                transfer_id: transfer_id.clone(),
                                status: ShareStatus::Failed,
                                reason: Some(ShareFailure::TimedOut),
                            }));
                        }
                        cleanup(&temp);
                        return Err(NativeError::Io(error));
                    }
                    if let Err(error) = tls.flush() {
                        cleanup(&temp);
                        return Err(NativeError::Io(error));
                    }
                    if *clipboard {
                        let _ = fs::remove_file(path);
                    }
                }
            }
            match read_frame(&mut tls) {
                Ok(Message::ShareUrl {
                    protocol: WIRE_VERSION,
                    transfer_id,
                    url,
                }) => {
                    last_received = Instant::now();
                    if !valid_transfer_id(&transfer_id) || !valid_share_url(&url) {
                        write_frame(
                            &mut tls,
                            &Message::ShareResult {
                                protocol: WIRE_VERSION,
                                transfer_id,
                                status: ShareStatus::Failed,
                                reason: Some(ShareFailure::InvalidResource),
                            },
                        )?;
                        continue;
                    }
                    event(StateEvent::ShareReceived(ReceivedShare {
                        device_id: DeviceId::new(format!("native:{id}")),
                        resource: SharedResource::Url { url },
                    }));
                    write_frame(
                        &mut tls,
                        &Message::ShareResult {
                            protocol: WIRE_VERSION,
                            transfer_id,
                            status: ShareStatus::Completed,
                            reason: None,
                        },
                    )?;
                }
                Ok(Message::ShareFile {
                    protocol: WIRE_VERSION,
                    transfer_id,
                    name,
                    size,
                    ..
                }) => {
                    let rejected = if !valid_transfer_id(&transfer_id)
                        || !safe_share_name(&name)
                        || name.len() > 255
                    {
                        Some(ShareFailure::InvalidResource)
                    } else if size > MAX_SHARE_SIZE {
                        Some(ShareFailure::SizeLimit)
                    } else {
                        None
                    };
                    if let Some(reason) = rejected {
                        let _ = write_frame(
                            &mut tls,
                            &Message::ShareResult {
                                protocol: WIRE_VERSION,
                                transfer_id,
                                status: ShareStatus::Failed,
                                reason: Some(reason),
                            },
                        );
                        return Err(NativeError::InvalidFrame);
                    }
                    let path = match self.receive_share_file(&mut tls, &name, size) {
                        Ok(path) => path,
                        Err(error) => {
                            let reason = match &error {
                                NativeError::Io(io)
                                    if io.kind() == std::io::ErrorKind::UnexpectedEof =>
                                {
                                    ShareFailure::Interrupted
                                }
                                _ => ShareFailure::Storage,
                            };
                            let _ = write_frame(
                                &mut tls,
                                &Message::ShareResult {
                                    protocol: WIRE_VERSION,
                                    transfer_id,
                                    status: ShareStatus::Failed,
                                    reason: Some(reason),
                                },
                            );
                            return Err(error);
                        }
                    };
                    last_received = Instant::now();
                    event(StateEvent::ShareReceived(ReceivedShare {
                        device_id: DeviceId::new(format!("native:{id}")),
                        resource: SharedResource::File {
                            path: path.to_string_lossy().into_owned(),
                        },
                    }));
                    write_frame(
                        &mut tls,
                        &Message::ShareResult {
                            protocol: WIRE_VERSION,
                            transfer_id,
                            status: ShareStatus::Completed,
                            reason: None,
                        },
                    )?;
                }
                Ok(Message::ShareResult {
                    protocol: WIRE_VERSION,
                    transfer_id,
                    status,
                    reason,
                }) => {
                    last_received = Instant::now();
                    if !valid_transfer_id(&transfer_id)
                        || (status == ShareStatus::Completed && reason.is_some())
                        || (status == ShareStatus::Failed && reason.is_none())
                    {
                        return Err(NativeError::InvalidFrame);
                    }
                    let was_pending = self
                        .inner
                        .lock()
                        .unwrap()
                        .pending_shares
                        .entry(id.clone())
                        .or_default()
                        .remove(&transfer_id)
                        .is_some();
                    if was_pending {
                        event(StateEvent::ShareResult(ShareResult {
                            device_id: DeviceId::new(format!("native:{id}")),
                            transfer_id,
                            status,
                            reason,
                        }));
                    }
                }
                Ok(Message::FilesystemList {
                    protocol: WIRE_VERSION,
                    request_id,
                    path,
                }) => {
                    if !valid_transfer_id(&request_id) {
                        return Err(NativeError::InvalidFrame);
                    }
                    let response = match list_local_directory(&path) {
                        Ok(entries) => Message::FilesystemEntries {
                            protocol: WIRE_VERSION,
                            request_id,
                            path,
                            entries,
                        },
                        Err(_) => Message::FilesystemFailure {
                            protocol: WIRE_VERSION,
                            request_id,
                            reason: "unavailable".into(),
                        },
                    };
                    write_frame(&mut tls, &response)?;
                    last_received = Instant::now();
                }
                Ok(Message::CustomCommandListRequest {
                    protocol: WIRE_VERSION,
                    request_id,
                }) => {
                    if !valid_transfer_id(&request_id) {
                        return Err(NativeError::InvalidFrame);
                    }
                    let names = native_custom_commands()
                        .into_iter()
                        .map(|(name, _)| name)
                        .collect();
                    write_frame(
                        &mut tls,
                        &Message::CustomCommandList {
                            protocol: WIRE_VERSION,
                            request_id,
                            names,
                        },
                    )?;
                    last_received = Instant::now();
                }
                Ok(Message::CustomCommandRequest {
                    protocol: WIRE_VERSION,
                    request_id,
                    name,
                }) => {
                    if !valid_transfer_id(&request_id) {
                        return Err(NativeError::InvalidFrame);
                    }
                    let result = if safe_command_name(&name) {
                        run_native_custom_command(&name)
                    } else {
                        (false, None, Some("unknown_command".into()))
                    };
                    write_frame(
                        &mut tls,
                        &Message::CustomCommandResult {
                            protocol: WIRE_VERSION,
                            request_id,
                            name,
                            accepted: result.0,
                            exit_code: result.1,
                            failure: result.2,
                        },
                    )?;
                    last_received = Instant::now();
                }
                Ok(Message::FilesystemEntries {
                    protocol: WIRE_VERSION,
                    request_id,
                    path,
                    entries,
                }) => {
                    if !valid_transfer_id(&request_id)
                        || !safe_browse_path(&path)
                        || entries.len() > 512
                    {
                        return Err(NativeError::InvalidFrame);
                    }
                    let entries = entries
                        .into_iter()
                        .filter(|entry| safe_share_name(&entry.name))
                        .map(|entry| FilesystemEntry {
                            name: entry.name,
                            directory: entry.directory,
                            size: entry.size,
                        })
                        .collect();
                    event(StateEvent::Filesystem(FilesystemResult {
                        device_id: DeviceId::new(format!("native:{id}")),
                        request_id,
                        path,
                        entries,
                        failure: None,
                    }));
                    last_received = Instant::now();
                }
                Ok(Message::FilesystemFailure {
                    protocol: WIRE_VERSION,
                    request_id,
                    reason,
                }) => {
                    if !valid_transfer_id(&request_id) || reason.len() > 128 {
                        return Err(NativeError::InvalidFrame);
                    }
                    event(StateEvent::Filesystem(FilesystemResult {
                        device_id: DeviceId::new(format!("native:{id}")),
                        request_id,
                        path: ".".into(),
                        entries: Vec::new(),
                        failure: Some(reason),
                    }));
                    last_received = Instant::now();
                }
                Ok(Message::DeviceCommandResult {
                    protocol: WIRE_VERSION,
                    request_id,
                    action,
                    accepted,
                    failure,
                }) => {
                    last_received = Instant::now();
                    if !valid_transfer_id(&request_id)
                        || (accepted && failure.is_some())
                        || (!accepted && failure.is_none())
                    {
                        return Err(NativeError::InvalidFrame);
                    }
                    event(StateEvent::DeviceCommandResult(DeviceCommandResult {
                        device_id: DeviceId::new(format!("native:{id}")),
                        request_id,
                        action,
                        accepted,
                        failure,
                    }));
                }
                Ok(Message::ScreensaverControl {
                    protocol: WIRE_VERSION,
                    request_id,
                    inhibit,
                }) => {
                    last_received = Instant::now();
                    if !valid_transfer_id(&request_id) {
                        return Err(NativeError::InvalidFrame);
                    }
                    // The phone only requests; the daemon owns the inhibitor.
                    // A release from a peer that never requested is still
                    // accepted so both sides converge on awake policy.
                    let mut inner = self.inner.lock().unwrap();
                    if inhibit {
                        inner.screensaver_requests.insert(id.clone());
                    } else {
                        inner.screensaver_requests.remove(&id);
                    }
                    drop(inner);
                    event(StateEvent::DeviceCommandResult(DeviceCommandResult {
                        device_id: DeviceId::new(format!("native:{id}")),
                        request_id,
                        action: DeviceCommandAction::Screensaver,
                        accepted: true,
                        failure: None,
                    }));
                }
                Ok(Message::Battery {
                    protocol: WIRE_VERSION,
                    percentage,
                    charging,
                }) => {
                    last_received = Instant::now();
                    let battery = BatteryState::new(percentage, charging)
                        .map_err(|_| NativeError::InvalidFrame)?;
                    let updated = {
                        let mut inner = self.inner.lock().unwrap();
                        if !inner.peers.peers.contains_key(&id) {
                            break;
                        }
                        inner.batteries.insert(id.clone(), battery);
                        let notifications_supported =
                            inner.notif_enabled.get(&id).copied().unwrap_or(false);
                        let media_supported =
                            inner.media_enabled.get(&id).copied().unwrap_or(false);
                        let mut updated = device(
                            &peer,
                            true,
                            Some(battery),
                            notifications_supported,
                            media_supported,
                        );
                        updated.connectivity = inner.connectivity.get(&id).copied();
                        if updated.connectivity.is_some() {
                            updated.capabilities.insert(Capability::Connectivity);
                        }
                        updated
                    };
                    // Emit without the lock held: callbacks may call back
                    // into the backend.
                    event(StateEvent::Device(DeviceEvent::Updated(updated)));
                }
                Ok(Message::Connectivity {
                    protocol: WIRE_VERSION,
                    transport,
                    validated,
                    metered,
                }) => {
                    last_received = Instant::now();
                    let connectivity = ConnectivityState {
                        transport,
                        validated,
                        metered,
                    };
                    let updated = {
                        let mut inner = self.inner.lock().unwrap();
                        if !inner.peers.peers.contains_key(&id) {
                            break;
                        }
                        inner.connectivity.insert(id.clone(), connectivity);
                        let mut updated = device(
                            &peer,
                            true,
                            inner.batteries.get(&id).cloned(),
                            inner.notif_enabled.get(&id).copied().unwrap_or(false),
                            inner.media_enabled.get(&id).copied().unwrap_or(false),
                        );
                        updated.connectivity = Some(connectivity);
                        updated.capabilities.insert(Capability::Connectivity);
                        updated
                    };
                    // Emit without the lock held: callbacks may call back
                    // into the backend.
                    event(StateEvent::Device(DeviceEvent::Updated(updated)));
                }
                Ok(Message::CallState {
                    protocol: WIRE_VERSION,
                    phase,
                    controls,
                    generation,
                }) => {
                    last_received = Instant::now();
                    // The generation is the phone's own counter, not a daemon
                    // guess: storing the received value keeps stamps and the
                    // phone's execution check in one sequence.
                    self.inner
                        .lock()
                        .unwrap()
                        .call_generations
                        .insert(id.clone(), generation);
                    event(StateEvent::Call(CallEvent::Updated(CallState {
                        device_id: DeviceId::new(format!("native:{id}")),
                        phase,
                        controls,
                        generation,
                    })));
                }
                Ok(Message::CallResult {
                    protocol: WIRE_VERSION,
                    request_id,
                    action,
                    accepted,
                    failure,
                }) => {
                    last_received = Instant::now();
                    let pending_action = self
                        .inner
                        .lock()
                        .unwrap()
                        .pending_call_results
                        .entry(id.clone())
                        .or_default()
                        .remove(&request_id);
                    if !valid_transfer_id(&request_id)
                        || pending_action != Some(action)
                        || (accepted && failure.is_some())
                        || (!accepted && failure.is_none())
                    {
                        return Err(NativeError::InvalidFrame);
                    }
                    event(StateEvent::CallCommandResult(CallCommandResult {
                        device_id: DeviceId::new(format!("native:{id}")),
                        request_id,
                        action,
                        accepted,
                        failure,
                    }));
                }
                Ok(Message::PresentationControl {
                    protocol: WIRE_VERSION,
                    action,
                    delta_x,
                    delta_y,
                }) => {
                    last_received = Instant::now();
                    event(StateEvent::Presentation(PresentationCommand {
                        device_id: DeviceId::new(format!("native:{id}")),
                        action,
                        delta_x,
                        delta_y,
                    }));
                }
                Ok(Message::VolumeControl {
                    protocol: WIRE_VERSION,
                    action,
                }) => {
                    last_received = Instant::now();
                    event(StateEvent::Volume(VolumeCommand {
                        device_id: DeviceId::new(format!("native:{id}")),
                        action,
                    }));
                }
                Ok(Message::RemoteInputControl {
                    protocol: WIRE_VERSION,
                    action,
                    delta_x,
                    delta_y,
                    button,
                    text,
                }) => {
                    let valid = match action {
                        RemoteInputAction::Move => delta_x.abs() <= 2000 && delta_y.abs() <= 2000,
                        RemoteInputAction::Click => (1..=5).contains(&button),
                        RemoteInputAction::Scroll => delta_y.unsigned_abs() <= 20,
                        RemoteInputAction::Type => {
                            text.as_ref().is_some_and(|value| value.len() <= 512)
                        }
                    };
                    if !valid {
                        return Err(NativeError::InvalidFrame);
                    }
                    last_received = Instant::now();
                    event(StateEvent::RemoteInput(RemoteInputCommand {
                        device_id: DeviceId::new(format!("native:{id}")),
                        action,
                        delta_x,
                        delta_y,
                        button,
                        text,
                    }));
                }
                Ok(Message::ContactsSync {
                    protocol: WIRE_VERSION,
                    contacts,
                }) => {
                    last_received = Instant::now();
                    let device_id = DeviceId::new(format!("native:{id}"));
                    let contacts = contacts
                        .into_iter()
                        .map(|contact| Contact {
                            device_id: device_id.clone(),
                            local_id: contact.local_id,
                            display_name: contact.display_name,
                            phones: contact.phones,
                            emails: contact.emails,
                            photo: contact.photo,
                        })
                        .collect();
                    event(StateEvent::Contacts(ContactsEvent::Synced {
                        device_id,
                        contacts,
                    }));
                }
                Ok(Message::ClipboardPost {
                    protocol: WIRE_VERSION,
                    text,
                    html,
                    uri,
                }) => {
                    last_received = Instant::now();
                    let rich_size = text.len()
                        + html.as_ref().map_or(0, String::len)
                        + uri.as_ref().map_or(0, String::len);
                    if text.len() <= 32 * 1024
                        && html.as_ref().is_none_or(|value| value.len() <= 32 * 1024)
                        && uri.as_ref().is_none_or(|value| value.len() <= 32 * 1024)
                        && rich_size <= 48 * 1024
                    {
                        event(StateEvent::Clipboard(ClipboardText {
                            device_id: DeviceId::new(format!("native:{id}")),
                            text,
                            html,
                            uri,
                        }));
                    }
                }
                Ok(Message::ClipboardFile {
                    protocol: WIRE_VERSION,
                    transfer_id,
                    name,
                    size,
                    mime,
                }) => {
                    if !valid_transfer_id(&transfer_id)
                        || size > 10 * 1024 * 1024
                        || !safe_share_name(&name)
                        || name.len() > 255
                        || mime.is_empty()
                        || mime.len() > 128
                    {
                        let _ = write_frame(
                            &mut tls,
                            &Message::ClipboardResult {
                                protocol: WIRE_VERSION,
                                transfer_id,
                                status: ShareStatus::Failed,
                                reason: Some(ShareFailure::InvalidResource),
                            },
                        );
                        return Err(NativeError::InvalidFrame);
                    }
                    let path = match self.receive_share_file(&mut tls, &name, size) {
                        Ok(path) => path,
                        Err(error) => {
                            let reason = match &error {
                                NativeError::Io(io)
                                    if io.kind() == std::io::ErrorKind::UnexpectedEof =>
                                {
                                    ShareFailure::Interrupted
                                }
                                _ => ShareFailure::Storage,
                            };
                            let _ = write_frame(
                                &mut tls,
                                &Message::ClipboardResult {
                                    protocol: WIRE_VERSION,
                                    transfer_id,
                                    status: ShareStatus::Failed,
                                    reason: Some(reason),
                                },
                            );
                            return Err(error);
                        }
                    };
                    event(StateEvent::ClipboardFile(ClipboardFile {
                        device_id: DeviceId::new(format!("native:{id}")),
                        path: path.to_string_lossy().into_owned(),
                        mime,
                    }));
                    last_received = Instant::now();
                    write_frame(
                        &mut tls,
                        &Message::ClipboardResult {
                            protocol: WIRE_VERSION,
                            transfer_id,
                            status: ShareStatus::Completed,
                            reason: None,
                        },
                    )?;
                }
                Ok(Message::NotificationPost {
                    protocol: WIRE_VERSION,
                    key,
                    app,
                    title,
                    body,
                    clearable,
                    actions,
                    reply_supported,
                }) => {
                    last_received = Instant::now();
                    self.handle_notification_post(
                        &peer,
                        WireNotification {
                            key,
                            app,
                            title,
                            body,
                            clearable,
                            actions,
                            reply_supported,
                        },
                        event,
                    )?;
                }
                Ok(Message::NotificationRemoved {
                    protocol: WIRE_VERSION,
                    key,
                }) => {
                    last_received = Instant::now();
                    self.handle_notification_removed(&peer, &key, event)?;
                }
                Ok(Message::NotificationsSync {
                    protocol: WIRE_VERSION,
                    enabled,
                    notifications,
                }) => {
                    last_received = Instant::now();
                    self.handle_notifications_sync(&peer, enabled, notifications, event)?;
                }
                Ok(Message::MediaPost {
                    protocol: WIRE_VERSION,
                    player,
                    application,
                    title,
                    artist,
                    album,
                    playback,
                    position_ms,
                    duration_ms,
                    controls,
                }) => {
                    last_received = Instant::now();
                    self.handle_media_post(
                        &peer,
                        WireMediaSession {
                            player,
                            application,
                            title,
                            artist,
                            album,
                            playback,
                            position_ms,
                            duration_ms,
                            controls,
                        },
                        event,
                    )?;
                }
                Ok(Message::MediaRemoved {
                    protocol: WIRE_VERSION,
                    player,
                }) => {
                    last_received = Instant::now();
                    self.handle_media_removed(&peer, &player, event)?;
                }
                Ok(Message::MediaSync {
                    protocol: WIRE_VERSION,
                    sessions,
                }) => {
                    last_received = Instant::now();
                    self.handle_media_sync(&peer, sessions, event)?;
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
                Ok(Message::Pong {
                    protocol: WIRE_VERSION,
                }) => {
                    last_received = Instant::now();
                }
                Ok(Message::Revoke {
                    protocol: WIRE_VERSION,
                }) => {
                    self.unpair(&id)?;
                    break;
                }
                Err(NativeError::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    if last_ping.elapsed() >= SESSION_PING_INTERVAL {
                        write_frame(
                            &mut tls,
                            &Message::Ping {
                                protocol: WIRE_VERSION,
                            },
                        )?;
                        last_ping = Instant::now();
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }
}
