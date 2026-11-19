//! Authoritative audio intent, sessions, observations, and bounded recovery.
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use handover_core::{
    AudioProfile, AudioReceiverDiagnostics, AudioSenderDiagnostics, AudioStreamDiagnostics,
    AudioStreamFailure, AudioStreamPhase, AudioStreamStatus, DeviceId, StateEvent,
};
use openssl::ssl::SslStream;

use crate::protocol::{AudioReceiverState, Message, write_frame};
use crate::{NativeBackend, NativeCommandError, NativeError, WIRE_VERSION};

pub(crate) type Event = Arc<dyn Fn(StateEvent) + Send + Sync>;
const BLOCK_BYTES: usize = 1920;
const MAX_RETRIES: u32 = 5;
type StartupSignal = Arc<(Mutex<bool>, std::sync::Condvar)>;

pub(crate) struct AudioSession {
    id: String,
    phase: AudioStreamPhase,
    socket: Option<TcpStream>,
    startup: Option<StartupSignal>,
    realtime: bool,
    rtc: Option<crate::audio_rtc::RtcControl>,
    enabled: bool,
    ready: bool,
    pub(crate) managed: bool,
    pub(crate) silence_standby: bool,
    profile: AudioProfile,
    retry_attempt: u32,
    generation: u64,
    revision: u64,
    failure: Option<AudioStreamFailure>,
    diagnostics: Option<AudioStreamDiagnostics>,
    transport_connected: bool,
    playing_since: Option<Instant>,
}
impl AudioSession {
    fn new(realtime: bool, managed: bool) -> Self {
        Self {
            id: String::new(),
            phase: AudioStreamPhase::Idle,
            socket: None,
            startup: None,
            realtime,
            rtc: None,
            enabled: false,
            ready: true,
            managed,
            silence_standby: false,
            profile: AudioProfile::Balanced,
            retry_attempt: 0,
            generation: 0,
            revision: 0,
            failure: None,
            diagnostics: None,
            transport_connected: false,
            playing_since: None,
        }
    }
    pub(crate) fn cancel(&self) {
        self.finish_startup();
        if let Some(rtc) = &self.rtc {
            rtc.cancel();
        }
        if let Some(socket) = &self.socket {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
    fn finish_startup(&self) {
        if let Some(signal) = &self.startup {
            *signal.0.lock().unwrap() = true;
            signal.1.notify_one();
        }
    }
    fn reset_stable_retries(&mut self) {
        if self
            .playing_since
            .is_some_and(|since| since.elapsed() >= Duration::from_secs(30))
        {
            self.retry_attempt = 0;
        }
    }
    pub(crate) fn disconnected(&mut self) {
        self.reset_stable_retries();
        self.playing_since = None;
        self.cancel();
        self.id.clear();
        self.socket = None;
        self.rtc = None;
        self.startup = None;
        self.transport_connected = false;
        self.ready = false;
        self.generation += 1;
        self.revision += 1;
        self.diagnostics = None;
        self.phase = if self.enabled {
            if self.retry_attempt < MAX_RETRIES {
                AudioStreamPhase::Reconnecting
            } else {
                AudioStreamPhase::Failed
            }
        } else {
            AudioStreamPhase::Idle
        };
        if self.enabled && self.retry_attempt < MAX_RETRIES {
            self.failure = Some(AudioStreamFailure::Disconnected);
        }
        if !self.enabled {
            self.failure = None;
        }
    }
    pub(crate) fn current_status(&self, peer: &str) -> AudioStreamStatus {
        self.status(peer, self.phase)
    }
    pub(crate) fn status(&self, peer: &str, state: AudioStreamPhase) -> AudioStreamStatus {
        AudioStreamStatus {
            device_id: DeviceId::new(format!("native:{peer}")),
            state,
            enabled: Some(self.enabled),
            profile: Some(self.profile),
            retry_attempt: self.retry_attempt,
            revision: self.revision,
            failure: self.failure,
            diagnostics: self.diagnostics.clone(),
        }
    }
}

impl NativeBackend {
    pub fn audio_streams(&self) -> Vec<AudioStreamStatus> {
        self.inner
            .lock()
            .unwrap()
            .audio
            .iter()
            .map(|(peer, a)| a.status(peer, a.phase))
            .collect()
    }
    fn publish_audio(&self, peer: &str, event: &Event) {
        let status = {
            let mut inner = self.inner.lock().unwrap();
            let Some(audio) = inner.audio.get(peer) else {
                return;
            };
            let status = audio.status(peer, audio.phase);
            if audio.managed && inner.active.contains_key(peer) {
                let queue = inner.outbox.entry(peer.to_owned()).or_default();
                queue.retain(|m| !matches!(m, Message::AudioStatus { .. }));
                if queue.len() < crate::limits::MAX_OUTBOX_PER_PEER {
                    queue.push(Message::AudioStatus {
                        protocol: WIRE_VERSION,
                        controls: true,
                        status: status.clone(),
                    });
                } else if let Some(socket) = inner.active.get(peer) {
                    let _ = socket.shutdown(Shutdown::Both);
                }
            }
            status
        };
        event(StateEvent::AudioStream(status));
    }
    pub(crate) fn audio_stream_result(&self, peer: &str, request_id: String, accepted: bool) {
        let mut inner = self.inner.lock().unwrap();
        let queue = inner.outbox.entry(peer.to_owned()).or_default();
        if queue.len() < crate::limits::MAX_OUTBOX_PER_PEER {
            queue.push(Message::AudioStreamResult {
                protocol: WIRE_VERSION,
                request_id,
                accepted,
            });
        } else if let Some(socket) = inner.active.get(peer) {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }

    pub(crate) fn audio_ready(
        &self,
        peer: &str,
        realtime: bool,
        managed: bool,
        pause_pending: bool,
        event: &Event,
    ) {
        let resume = {
            let mut inner = self.inner.lock().unwrap();
            let a = inner
                .audio
                .entry(peer.to_owned())
                .or_insert_with(|| AudioSession::new(realtime, managed));
            if pause_pending {
                a.enabled = false;
                a.generation += 1;
                a.retry_attempt = 0;
                Self::clear_audio(a, AudioStreamPhase::Idle);
            }
            a.realtime = realtime;
            a.managed = managed;
            a.ready = true;
            a.revision += 1;
            (a.enabled && a.retry_attempt < MAX_RETRIES).then_some(a.generation)
        };
        self.publish_audio(peer, event);
        if let Some(generation) = resume {
            let _ = self.start_audio_session(peer, event.clone(), Some(generation), false, None);
        }
    }
    pub fn start_audio(&self, peer: &str, event: Event) -> Result<(), NativeCommandError> {
        self.start_audio_session(peer, event, None, true, None)
    }
    pub(crate) fn set_audio_from_phone(
        &self,
        peer: &str,
        enabled: bool,
        event: Event,
    ) -> Result<(), NativeCommandError> {
        if enabled {
            self.start_audio_session(peer, event, None, false, None)
        } else {
            self.stop_audio(peer, event)
        }
    }
    fn start_audio_session(
        &self,
        peer: &str,
        event: Event,
        recovery: Option<u64>,
        replace: bool,
        profile_override: Option<AudioProfile>,
    ) -> Result<(), NativeCommandError> {
        let mut nonce = [0u8; 16];
        openssl::rand::rand_bytes(&mut nonce).map_err(|_| NativeCommandError::QueueFull)?;
        let id = hex::encode(nonce);
        let startup: StartupSignal = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let (rtc_control, rtc_input) = crate::audio_rtc::RtcControl::new();
        let (realtime, profile) = {
            let mut inner = self.inner.lock().unwrap();
            let a = inner.audio.get(peer).ok_or(NativeCommandError::Offline)?;
            if let Some(generation) = recovery
                && (!a.enabled || a.generation != generation)
            {
                return if profile_override.is_some() {
                    Err(NativeCommandError::Offline)
                } else {
                    Ok(())
                };
            }
            if !inner.active.contains_key(peer) || !a.ready {
                return Err(NativeCommandError::Offline);
            }
            if !replace
                && matches!(
                    a.phase,
                    AudioStreamPhase::Starting
                        | AudioStreamPhase::Streaming
                        | AudioStreamPhase::Standby
                )
            {
                return Ok(());
            }
            let (realtime, profile) = (a.realtime, profile_override.unwrap_or(a.profile));
            let queue = inner.outbox.entry(peer.to_owned()).or_default();
            if queue.len() >= crate::limits::MAX_OUTBOX_PER_PEER {
                if recovery.is_some()
                    && profile_override.is_none()
                    && let Some(socket) = inner.active.get(peer)
                {
                    let _ = socket.shutdown(Shutdown::Both);
                }
                return Err(NativeCommandError::QueueFull);
            }
            if !realtime {
                queue.push(Message::AudioOffer {
                    protocol: WIRE_VERSION,
                    stream_id: id.clone(),
                });
            }
            let a = inner.audio.get_mut(peer).unwrap();
            a.cancel();
            if recovery.is_none() || profile_override.is_some() {
                a.generation += 1;
                a.retry_attempt = 0;
            }
            a.enabled = true;
            a.profile = profile;
            a.failure = None;
            a.diagnostics = None;
            a.transport_connected = false;
            a.playing_since = None;
            a.id.clone_from(&id);
            a.phase = AudioStreamPhase::Starting;
            a.revision += 1;
            a.socket = None;
            a.rtc = realtime.then_some(rtc_control);
            a.startup = Some(startup.clone());
            (realtime, profile)
        };
        self.publish_audio(peer, &event);
        if realtime {
            crate::audio_rtc::start(
                self.clone(),
                peer.to_owned(),
                id.clone(),
                event.clone(),
                rtc_input,
                profile,
            );
        }
        let backend = self.clone();
        let peer = peer.to_owned();
        std::thread::spawn(move || {
            let (done, _) = startup
                .1
                .wait_timeout_while(startup.0.lock().unwrap(), Duration::from_secs(10), |done| {
                    !*done
                })
                .unwrap();
            if *done {
                return;
            }
            drop(done);
            backend.fail_audio(&peer, &id, AudioStreamFailure::StartupTimeout, &event);
        });
        Ok(())
    }
    pub fn set_audio_profile(
        &self,
        peer: &str,
        profile: AudioProfile,
        event: Event,
    ) -> Result<(), NativeCommandError> {
        let restart = {
            let mut inner = self.inner.lock().unwrap();
            let a = inner
                .audio
                .get_mut(peer)
                .ok_or(NativeCommandError::Offline)?;
            if !a.realtime && profile != AudioProfile::Balanced {
                return Err(NativeCommandError::Offline);
            }
            if a.profile == profile {
                drop(inner);
                self.publish_audio(peer, &event);
                return Ok(());
            }
            if a.enabled && a.ready {
                Some(a.generation)
            } else {
                a.profile = profile;
                a.revision += 1;
                None
            }
        };
        if let Some(generation) = restart {
            self.start_audio_session(peer, event, Some(generation), true, Some(profile))?;
        } else {
            self.publish_audio(peer, &event);
        }
        Ok(())
    }
    pub fn stop_audio(&self, peer: &str, event: Event) -> Result<(), NativeCommandError> {
        let id = {
            let mut inner = self.inner.lock().unwrap();
            let a = inner
                .audio
                .get_mut(peer)
                .ok_or(NativeCommandError::Offline)?;
            a.enabled = false;
            a.generation += 1;
            a.retry_attempt = 0;
            a.id.clone()
        };
        self.finish_audio(peer, &id, AudioStreamPhase::Idle, &event);
        Ok(())
    }
    pub fn audio_stream_is_playing(&self, peer: &str, id: &str) -> bool {
        self.inner.lock().unwrap().audio.get(peer).is_some_and(|a| {
            !id.is_empty()
                && a.id == id
                && matches!(
                    a.phase,
                    AudioStreamPhase::Streaming | AudioStreamPhase::Standby
                )
        })
    }
    #[cfg(test)]
    pub(crate) fn audio_playback_toggle(&self, peer: &str, id: &str, event: &Event) {
        self.audio_playback_request(peer, id, None, event);
    }
    pub(crate) fn audio_playback_request(
        &self,
        peer: &str,
        id: &str,
        request_id: Option<String>,
        event: &Event,
    ) {
        if self.audio_stream_is_playing(peer, id) {
            event(StateEvent::DesktopPlaybackToggle {
                device_id: DeviceId::new(format!("native:{peer}")),
                stream_id: id.to_owned(),
                playback: self.audio_output.lock().unwrap().settings.playback.clone(),
                request_id,
            });
        }
    }
    pub(crate) fn audio_focus_pending(&self, peer: &str, id: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .audio
            .get(peer)
            .is_some_and(|audio| {
                audio.id == id
                    && audio.enabled
                    && audio.phase == AudioStreamPhase::Suspended
                    && audio.failure == Some(AudioStreamFailure::AudioFocus)
            })
    }
    pub(crate) fn set_silence_standby_support(&self, peer: &str, supported: bool) {
        if let Some(audio) = self.inner.lock().unwrap().audio.get_mut(peer) {
            audio.silence_standby = supported;
        }
    }
    pub(crate) fn request_audio_standby(&self, peer: &str, id: &str, standby: bool) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(audio) = inner.audio.get(peer) else {
            return false;
        };
        if audio.id != id || !audio.enabled || !audio.silence_standby {
            return false;
        }
        if standby && let Some(rtc) = &audio.rtc {
            rtc.receiver_wait();
        }
        let queue = inner.outbox.entry(peer.to_owned()).or_default();
        if queue.len() >= crate::limits::MAX_OUTBOX_PER_PEER {
            return false;
        }
        queue.push(Message::AudioStandby {
            protocol: WIRE_VERSION,
            stream_id: id.to_owned(),
            standby,
        });
        true
    }
    pub(crate) fn audio_playing(&self, peer: &str, id: &str, event: &Event) {
        {
            let mut inner = self.inner.lock().unwrap();
            let Some(a) = inner.audio.get_mut(peer) else {
                return;
            };
            if a.id != id
                || !a.enabled
                || !matches!(
                    a.phase,
                    AudioStreamPhase::Starting
                        | AudioStreamPhase::Streaming
                        | AudioStreamPhase::Standby
                        | AudioStreamPhase::Suspended
                        | AudioStreamPhase::Reconnecting
                )
                || (a.socket.is_none() && a.rtc.is_none())
            {
                return;
            }
            a.reset_stable_retries();
            if let Some(rtc) = &a.rtc {
                rtc.receiver_ready();
            }
            a.phase = AudioStreamPhase::Streaming;
            a.playing_since = Some(Instant::now());
            a.failure = None;
            a.revision += 1;
            a.finish_startup();
        }
        self.publish_audio(peer, event);
    }
    pub(crate) fn audio_transport_connected(&self, peer: &str, id: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(a) = inner.audio.get_mut(peer)
            && a.id == id
            && a.enabled
        {
            a.transport_connected = true;
            if a.phase == AudioStreamPhase::Suspended {
                a.finish_startup();
            }
        }
    }

    pub(crate) fn audio_receiver_state(
        &self,
        peer: &str,
        id: &str,
        state: AudioReceiverState,
        event: &Event,
    ) {
        if matches!(
            state,
            AudioReceiverState::Failed
                | AudioReceiverState::Disconnected
                | AudioReceiverState::FocusUnavailable
        ) {
            self.fail_audio(
                peer,
                id,
                match state {
                    AudioReceiverState::Disconnected => AudioStreamFailure::Disconnected,
                    AudioReceiverState::FocusUnavailable => AudioStreamFailure::AudioFocus,
                    _ => AudioStreamFailure::ReceiverFailed,
                },
                event,
            );
            return;
        }
        if state == AudioReceiverState::Standby {
            {
                let mut inner = self.inner.lock().unwrap();
                let Some(audio) = inner.audio.get_mut(peer) else {
                    return;
                };
                if audio.id != id || !audio.enabled || !audio.silence_standby {
                    return;
                }
                audio.reset_stable_retries();
                audio.phase = AudioStreamPhase::Standby;
                audio.failure = None;
                audio.playing_since = None;
                audio.revision += 1;
            }
            self.publish_audio(peer, event);
            return;
        }
        if state == AudioReceiverState::Playing {
            self.audio_playing(peer, id, event);
            return;
        }
        {
            let mut inner = self.inner.lock().unwrap();
            let Some(a) = inner.audio.get_mut(peer) else {
                return;
            };
            if a.id != id || id.is_empty() || !a.enabled {
                return;
            }
            a.reset_stable_retries();
            a.playing_since = None;
            a.phase = if matches!(
                state,
                AudioReceiverState::Reconnecting | AudioReceiverState::OutputSuspended
            ) {
                AudioStreamPhase::Reconnecting
            } else {
                AudioStreamPhase::Suspended
            };
            a.failure = Some(match state {
                AudioReceiverState::Reconnecting => AudioStreamFailure::Disconnected,
                AudioReceiverState::OutputSuspended => AudioStreamFailure::ReceiverFailed,
                _ => AudioStreamFailure::AudioFocus,
            });
            a.revision += 1;
            if let Some(diagnostics) = &mut a.diagnostics {
                diagnostics.receiver = None;
            }
            if a.transport_connected {
                a.finish_startup();
            }
        }
        self.publish_audio(peer, event);
    }
    pub(crate) fn finish_audio(
        &self,
        peer: &str,
        id: &str,
        phase: AudioStreamPhase,
        event: &Event,
    ) {
        if phase == AudioStreamPhase::Failed {
            self.fail_audio(peer, id, AudioStreamFailure::ConnectionFailed, event);
            return;
        }
        {
            let mut inner = self.inner.lock().unwrap();
            let Some(a) = inner.audio.get_mut(peer) else {
                return;
            };
            if a.id != id {
                return;
            }
            a.enabled = false;
            a.generation += 1;
            a.failure = None;
            Self::clear_audio(a, phase);
            Self::queue_audio_stop(&mut inner, peer, id);
        }
        self.publish_audio(peer, event);
    }
    fn clear_audio(a: &mut AudioSession, phase: AudioStreamPhase) {
        a.cancel();
        a.socket = None;
        a.rtc = None;
        a.startup = None;
        a.id.clear();
        a.phase = phase;
        a.diagnostics = None;
        a.transport_connected = false;
        a.playing_since = None;
        a.revision += 1;
    }
    fn queue_audio_stop(inner: &mut crate::Runtime, peer: &str, id: &str) {
        if !inner.active.contains_key(peer) {
            return;
        }
        let queue = inner.outbox.entry(peer.to_owned()).or_default();
        queue.retain(|message| !matches!(message, Message::AudioOffer { stream_id, .. } | Message::AudioRtcOffer { stream_id, .. } if stream_id == id));
        if !id.is_empty() && queue.len() < crate::limits::MAX_OUTBOX_PER_PEER {
            queue.push(Message::AudioStop {
                protocol: WIRE_VERSION,
                stream_id: id.to_owned(),
            });
        } else if queue.len() >= crate::limits::MAX_OUTBOX_PER_PEER
            && let Some(socket) = inner.active.get(peer)
        {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
    pub(crate) fn fail_audio(
        &self,
        peer: &str,
        id: &str,
        failure: AudioStreamFailure,
        event: &Event,
    ) {
        let retry = {
            let mut inner = self.inner.lock().unwrap();
            let active = inner.active.contains_key(peer);
            let Some(a) = inner.audio.get_mut(peer) else {
                return;
            };
            if id.is_empty() || a.id != id {
                return;
            }
            a.reset_stable_retries();
            let retry = a.enabled && active && a.ready && a.retry_attempt < MAX_RETRIES;
            let phase = if a.enabled && (!active || retry) {
                AudioStreamPhase::Reconnecting
            } else {
                AudioStreamPhase::Failed
            };
            Self::clear_audio(a, phase);
            a.failure = Some(failure);
            let result = if retry {
                a.retry_attempt += 1;
                Some((
                    a.generation,
                    Duration::from_secs(1 << (a.retry_attempt - 1)),
                ))
            } else {
                None
            };
            Self::queue_audio_stop(&mut inner, peer, id);
            result
        };
        self.publish_audio(peer, event);
        if let Some((generation, delay)) = retry {
            let backend = self.clone();
            let peer = peer.to_owned();
            let event = event.clone();
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                let _ = backend.start_audio_session(&peer, event, Some(generation), false, None);
            });
        }
    }
    pub(crate) fn audio_sender_diagnostics(
        &self,
        peer: &str,
        id: &str,
        diagnostics: AudioSenderDiagnostics,
        event: &Event,
    ) {
        {
            let mut inner = self.inner.lock().unwrap();
            let Some(a) = inner.audio.get_mut(peer) else {
                return;
            };
            if a.id != id || !a.enabled {
                return;
            }
            a.reset_stable_retries();
            a.diagnostics.get_or_insert_with(Default::default).sender = Some(diagnostics);
            a.revision += 1;
        }
        self.publish_audio(peer, event);
    }
    pub(crate) fn audio_receiver_diagnostics(
        &self,
        peer: &str,
        id: &str,
        mut diagnostics: AudioReceiverDiagnostics,
        event: &Event,
    ) {
        {
            let mut inner = self.inner.lock().unwrap();
            let Some(a) = inner.audio.get_mut(peer) else {
                return;
            };
            if a.id != id || id.is_empty() || !a.enabled || !a.managed {
                return;
            }
            if diagnostics.round_trip_ms.is_some_and(|v| v > 60_000)
                || diagnostics.jitter_ms.is_some_and(|v| v > 60_000)
                || diagnostics.jitter_buffer_ms.is_some_and(|v| v > 60_000)
                || diagnostics.bitrate_bps.is_some_and(|v| v > 10_000_000)
            {
                return;
            }
            a.reset_stable_retries();
            diagnostics.updated_at_ms = jiff::Timestamp::now().as_millisecond();
            a.diagnostics.get_or_insert_with(Default::default).receiver = Some(diagnostics);
            a.revision += 1;
        }
        self.publish_audio(peer, event);
    }
    pub(crate) fn handle_audio(
        &self,
        mut tls: SslStream<TcpStream>,
        peer: &str,
        id: &str,
        event: &Event,
    ) -> Result<(), NativeError> {
        {
            let mut inner = self.inner.lock().unwrap();
            if !inner.active.contains_key(peer)
                || !inner
                    .peers
                    .peers
                    .get(peer)
                    .is_some_and(|p| p.fingerprint == peer)
            {
                return Err(NativeError::InvalidFrame);
            }
            let audio = inner.audio.get_mut(peer).ok_or(NativeError::InvalidFrame)?;
            if audio.id != id
                || audio.phase != AudioStreamPhase::Starting
                || audio.socket.is_some()
                || audio.rtc.is_some()
            {
                return Err(NativeError::InvalidFrame);
            }
            audio.socket = Some(tls.get_ref().try_clone()?);
        }
        let result = self.stream_pcm(&mut tls);
        if let Err(error) = &result {
            tracing::debug!(%error, "audio stream stopped");
        }
        self.finish_audio(peer, id, AudioStreamPhase::Failed, event);
        result
    }

    pub(crate) fn audio_rtc_answer(&self, peer: &str, id: &str, sdp: String) {
        let inner = self.inner.lock().unwrap();
        if let Some(audio) = inner.audio.get(peer)
            && audio.id == id
            && matches!(
                audio.phase,
                AudioStreamPhase::Starting
                    | AudioStreamPhase::Suspended
                    | AudioStreamPhase::Reconnecting
            )
            && sdp.len() <= 32 * 1024
            && let Some(rtc) = &audio.rtc
        {
            rtc.answer(sdp);
        }
    }

    pub(crate) fn audio_rtc_offer(&self, peer: &str, id: &str, sdp: String) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if sdp.len() > 32 * 1024
            || !inner
                .audio
                .get(peer)
                .is_some_and(|a| a.id == id && a.rtc.is_some())
        {
            return false;
        }
        let queue = inner.outbox.entry(peer.to_owned()).or_default();
        if queue.len() >= crate::limits::MAX_OUTBOX_PER_PEER {
            return false;
        }
        let profile = inner.audio[peer].profile;
        let queue = inner.outbox.entry(peer.to_owned()).or_default();
        queue.push(Message::AudioRtcOffer {
            protocol: WIRE_VERSION,
            stream_id: id.to_owned(),
            sdp,
            profile,
        });
        true
    }

    pub fn stop_all_audio(&self) {
        for session in self.inner.lock().unwrap().audio.values_mut() {
            session.enabled = false;
            session.generation += 1;
            Self::clear_audio(session, AudioStreamPhase::Idle);
        }
        self.audio_output.lock().unwrap().shutdown();
    }

    fn stream_pcm(&self, tls: &mut SslStream<TcpStream>) -> Result<(), NativeError> {
        tls.get_ref().set_nodelay(true)?;
        tls.get_ref()
            .set_write_timeout(Some(Duration::from_secs(1)))?;
        let output = crate::audio_output::OutputRouter::acquire(&self.audio_output)?;
        // Capture the owned phone output, independently of hardware mute.
        let mut child = Command::new("parec")
            .args([
                &format!("--device={}", output.monitor),
                "--raw",
                "--format=s16le",
                "--rate=48000",
                "--channels=2",
                "--latency-msec=10",
                "--process-time-msec=10",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut capture = child.stdout.take().ok_or(NativeError::InvalidFrame)?;
        // The producer always drains capture. Keep only the most recent 60 ms.
        let blocks = Arc::new((
            Mutex::new((std::collections::VecDeque::new(), false)),
            std::sync::Condvar::new(),
        ));
        let producer_blocks = blocks.clone();
        let started = Instant::now();
        let producer = std::thread::spawn(move || {
            loop {
                let mut pcm = [0u8; BLOCK_BYTES];
                if capture.read_exact(&mut pcm).is_err() {
                    break;
                }
                let (lock, ready) = &*producer_blocks;
                let mut queue = lock.lock().unwrap();
                if queue.0.len() == 6 {
                    queue.0.pop_front();
                }
                queue
                    .0
                    .push_back((started.elapsed().as_micros() as u64, pcm));
                ready.notify_one();
            }
            let (lock, ready) = &*producer_blocks;
            lock.lock().unwrap().1 = true;
            ready.notify_one();
        });
        let result = (|| {
            write_frame(
                tls,
                &Message::AudioFormat {
                    protocol: WIRE_VERSION,
                },
            )?;
            loop {
                if !output.healthy() {
                    return Err(NativeError::InvalidFrame);
                }
                let (lock, ready) = &*blocks;
                let mut queue = lock.lock().unwrap();
                while queue.0.is_empty() && !queue.1 {
                    let (next, wait) = ready.wait_timeout(queue, Duration::from_secs(2)).unwrap();
                    queue = next;
                    if wait.timed_out() {
                        return Err(NativeError::InvalidFrame);
                    }
                }
                let Some((timestamp, pcm)) = queue.0.pop_front() else {
                    return Err(NativeError::InvalidFrame);
                };
                drop(queue);
                if started.elapsed().as_micros() as u64 - timestamp > 60_000 {
                    continue;
                }
                let mut record = [0u8; BLOCK_BYTES + 8];
                record[..8].copy_from_slice(&timestamp.to_be_bytes());
                record[8..].copy_from_slice(&pcm);
                tls.write_all(&record)?;
                tls.flush()?;
            }
        })();
        let _ = child.kill();
        let _ = child.wait();
        let _ = producer.join();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_toggle_requires_the_current_stream_on_the_requesting_peer() {
        let directory = tempfile::tempdir().unwrap();
        let backend = NativeBackend::open(directory.path().to_path_buf()).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let event: Event = Arc::new(move |event| captured.lock().unwrap().push(event));
        backend.audio_ready("phone", true, true, false, &event);
        events.lock().unwrap().clear();
        backend.audio_playback_toggle("phone", "", &event);
        {
            let mut inner = backend.inner.lock().unwrap();
            let audio = inner.audio.get_mut("phone").unwrap();
            audio.id = "current".into();
            audio.phase = AudioStreamPhase::Starting;
        }
        backend.audio_playback_toggle("phone", "current", &event);
        backend
            .inner
            .lock()
            .unwrap()
            .audio
            .get_mut("phone")
            .unwrap()
            .phase = AudioStreamPhase::Streaming;
        backend.audio_playback_toggle("another-phone", "current", &event);
        backend.audio_playback_toggle("phone", "stale", &event);
        assert!(events.lock().unwrap().is_empty());
        backend.audio_playback_toggle("phone", "current", &event);
        assert_eq!(
            *events.lock().unwrap(),
            vec![StateEvent::DesktopPlaybackToggle {
                device_id: DeviceId::new("native:phone"),
                stream_id: "current".into(),
                playback: handover_core::AudioPlaybackTarget::Automatic,
                request_id: None,
            }]
        );
        backend
            .inner
            .lock()
            .unwrap()
            .audio
            .get_mut("phone")
            .unwrap()
            .phase = AudioStreamPhase::Idle;
        backend.audio_playback_toggle("phone", "current", &event);
        assert_eq!(events.lock().unwrap().len(), 1);
    }
    fn audio_fixture() -> (tempfile::TempDir, NativeBackend, Event) {
        let directory = tempfile::tempdir().unwrap();
        let backend = NativeBackend::open(directory.path().to_path_buf()).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_accepted, _) = listener.accept().unwrap();
        backend
            .inner
            .lock()
            .unwrap()
            .active
            .insert("phone".into(), client);
        let event: Event = Arc::new(|_| {});
        // Legacy offers avoid touching a real audio server in recovery tests.
        backend.audio_ready("phone", false, true, false, &event);
        (directory, backend, event)
    }

    #[test]
    fn standby_is_confirmed_and_manual_stop_cancels_automatic_wake() {
        let (_directory, backend, event) = audio_fixture();
        backend.start_audio("phone", event.clone()).unwrap();
        let id = backend.inner.lock().unwrap().audio["phone"].id.clone();
        let (rtc, _input) = crate::audio_rtc::RtcControl::new();
        {
            let mut inner = backend.inner.lock().unwrap();
            let audio = inner.audio.get_mut("phone").unwrap();
            audio.rtc = Some(rtc);
            audio.phase = AudioStreamPhase::Streaming;
        }
        assert!(!backend.request_audio_standby("phone", &id, true));
        backend.set_silence_standby_support("phone", true);
        assert!(!backend.request_audio_standby("phone", "stale", true));
        assert!(backend.request_audio_standby("phone", &id, true));
        assert_eq!(
            backend.audio_streams()[0].state,
            AudioStreamPhase::Streaming
        );
        backend.audio_receiver_state("phone", &id, AudioReceiverState::Standby, &event);
        assert_eq!(backend.audio_streams()[0].state, AudioStreamPhase::Standby);
        assert!(backend.audio_stream_is_playing("phone", &id));
        backend
            .set_audio_from_phone("phone", true, event.clone())
            .unwrap();
        assert_eq!(backend.audio_streams()[0].state, AudioStreamPhase::Standby);
        assert!(backend.request_audio_standby("phone", &id, false));
        backend.audio_playing("phone", &id, &event);
        assert_eq!(
            backend.audio_streams()[0].state,
            AudioStreamPhase::Streaming
        );
        backend.stop_audio("phone", event.clone()).unwrap();
        assert!(!backend.request_audio_standby("phone", &id, false));
        backend.audio_receiver_state("phone", &id, AudioReceiverState::Standby, &event);
        backend.audio_playing("phone", &id, &event);
        assert_eq!(backend.audio_streams()[0].state, AudioStreamPhase::Idle);
    }

    #[test]
    fn manual_stop_invalidates_a_scheduled_recovery_and_late_receiver_events() {
        let (_directory, backend, event) = audio_fixture();
        backend.start_audio("phone", event.clone()).unwrap();
        let id = backend.inner.lock().unwrap().audio["phone"].id.clone();
        backend.fail_audio("phone", &id, AudioStreamFailure::ConnectionFailed, &event);
        let generation = backend.inner.lock().unwrap().audio["phone"].generation;
        assert_eq!(
            backend.audio_streams()[0].state,
            AudioStreamPhase::Reconnecting
        );
        backend.stop_audio("phone", event.clone()).unwrap();
        backend
            .start_audio_session("phone", event.clone(), Some(generation), false, None)
            .unwrap();
        backend.audio_receiver_state("phone", &id, AudioReceiverState::Playing, &event);
        backend.fail_audio("phone", &id, AudioStreamFailure::ConnectionFailed, &event);
        let status = &backend.audio_streams()[0];
        assert_eq!(status.state, AudioStreamPhase::Idle);
        assert_eq!(status.enabled, Some(false));
        assert!(backend.inner.lock().unwrap().audio["phone"].id.is_empty());
    }

    #[test]
    fn reconnect_resumes_only_wanted_audio_and_honors_offline_phone_pause() {
        let (_directory, backend, event) = audio_fixture();
        backend.start_audio("phone", event.clone()).unwrap();
        let first_id = backend.inner.lock().unwrap().audio["phone"].id.clone();
        backend
            .inner
            .lock()
            .unwrap()
            .audio
            .get_mut("phone")
            .unwrap()
            .disconnected();
        backend.audio_ready("phone", false, true, false, &event);
        assert_eq!(backend.audio_streams()[0].state, AudioStreamPhase::Starting);
        assert_ne!(backend.inner.lock().unwrap().audio["phone"].id, first_id);
        backend
            .inner
            .lock()
            .unwrap()
            .audio
            .get_mut("phone")
            .unwrap()
            .disconnected();
        backend.audio_ready("phone", false, true, true, &event);
        assert_eq!(backend.audio_streams()[0].state, AudioStreamPhase::Idle);
        assert_eq!(backend.audio_streams()[0].enabled, Some(false));
        backend.audio_ready("phone", false, true, false, &event);
        assert_eq!(backend.audio_streams()[0].state, AudioStreamPhase::Idle);
        backend.stop_all_audio();
    }

    #[test]
    fn retries_are_bounded_and_keep_the_observed_failure_cause() {
        let (_directory, backend, event) = audio_fixture();
        backend.start_audio("phone", event.clone()).unwrap();
        for index in 0..=MAX_RETRIES {
            let id = format!("attempt-{index}");
            backend
                .inner
                .lock()
                .unwrap()
                .audio
                .get_mut("phone")
                .unwrap()
                .id = id.clone();
            backend.fail_audio("phone", &id, AudioStreamFailure::CaptureFailed, &event);
        }
        let status = &backend.audio_streams()[0];
        assert_eq!(status.state, AudioStreamPhase::Failed);
        assert_eq!(status.retry_attempt, MAX_RETRIES);
        assert_eq!(status.failure, Some(AudioStreamFailure::CaptureFailed));
        backend.stop_all_audio();
    }

    #[test]
    fn focus_wait_keeps_negotiation_timeout_until_transport_is_connected() {
        let (_directory, backend, event) = audio_fixture();
        backend.start_audio("phone", event.clone()).unwrap();
        let id = backend.inner.lock().unwrap().audio["phone"].id.clone();
        backend.audio_receiver_state("phone", &id, AudioReceiverState::Suspended, &event);
        let startup = backend.inner.lock().unwrap().audio["phone"]
            .startup
            .clone()
            .unwrap();
        assert!(!*startup.0.lock().unwrap());
        backend.audio_transport_connected("phone", &id);
        assert!(*startup.0.lock().unwrap());
        assert_eq!(
            backend.audio_streams()[0].state,
            AudioStreamPhase::Suspended
        );
        backend.stop_audio("phone", event.clone()).unwrap();
        backend.audio_receiver_state("phone", &id, AudioReceiverState::Playing, &event);
        assert_eq!(backend.audio_streams()[0].state, AudioStreamPhase::Idle);
    }
    #[test]
    fn rejected_profile_change_keeps_the_running_profile_unchanged() {
        let (_directory, backend, event) = audio_fixture();
        {
            let mut inner = backend.inner.lock().unwrap();
            let a = inner.audio.get_mut("phone").unwrap();
            a.realtime = true;
            a.enabled = true;
            a.phase = AudioStreamPhase::Streaming;
            inner.outbox.insert(
                "phone".into(),
                (0..crate::limits::MAX_OUTBOX_PER_PEER)
                    .map(|_| Message::Ping {
                        protocol: WIRE_VERSION,
                    })
                    .collect(),
            );
        }
        assert!(matches!(
            backend.set_audio_profile("phone", AudioProfile::Music, event),
            Err(NativeCommandError::QueueFull)
        ));
        assert_eq!(
            backend.audio_streams()[0].profile,
            Some(AudioProfile::Balanced)
        );
        assert_eq!(
            backend.audio_streams()[0].state,
            AudioStreamPhase::Streaming
        );
        backend.stop_all_audio();
    }
    #[test]
    fn brief_playback_does_not_give_an_unstable_stream_an_unlimited_retry_budget() {
        let (_directory, backend, event) = audio_fixture();
        backend.start_audio("phone", event.clone()).unwrap();
        let id = backend.inner.lock().unwrap().audio["phone"].id.clone();
        {
            let mut inner = backend.inner.lock().unwrap();
            let a = inner.audio.get_mut("phone").unwrap();
            a.rtc = Some(crate::audio_rtc::RtcControl::new().0);
            a.retry_attempt = 3;
        }
        backend.audio_playing("phone", &id, &event);
        assert_eq!(backend.audio_streams()[0].retry_attempt, 3);
        backend
            .inner
            .lock()
            .unwrap()
            .audio
            .get_mut("phone")
            .unwrap()
            .playing_since = Some(Instant::now() - Duration::from_secs(31));
        backend.fail_audio("phone", &id, AudioStreamFailure::ConnectionFailed, &event);
        assert_eq!(backend.audio_streams()[0].retry_attempt, 1);
        backend.stop_all_audio();
    }
}
