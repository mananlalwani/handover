//! Authoritative audio sessions. Legacy PCM uses a separate pinned TLS connection.
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use handover_core::{AudioStreamPhase, AudioStreamStatus, DeviceId, StateEvent};
use openssl::ssl::SslStream;

use crate::protocol::{Message, write_frame};
use crate::{NativeBackend, NativeCommandError, NativeError, WIRE_VERSION};

pub(crate) type Event = Arc<dyn Fn(StateEvent) + Send + Sync>;
const BLOCK_BYTES: usize = 1920; // 480 frames, stereo signed 16-bit LE, 48 kHz.
type StartupSignal = Arc<(Mutex<bool>, std::sync::Condvar)>;

pub(crate) struct AudioSession {
    id: String,
    phase: AudioStreamPhase,
    socket: Option<TcpStream>,
    startup: Option<StartupSignal>,
    realtime: bool,
    rtc: Option<crate::audio_rtc::RtcControl>,
}

impl AudioSession {
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

    pub(crate) fn status(&self, peer: &str, state: AudioStreamPhase) -> AudioStreamStatus {
        AudioStreamStatus {
            device_id: DeviceId::new(format!("native:{peer}")),
            state,
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
            .map(|(peer, session)| session.status(peer, session.phase))
            .collect()
    }

    pub(crate) fn audio_ready(&self, peer: &str, realtime: bool, event: &Event) {
        let status = {
            let mut inner = self.inner.lock().unwrap();
            let audio = inner.audio.entry(peer.to_owned()).or_insert(AudioSession {
                id: String::new(),
                phase: AudioStreamPhase::Idle,
                socket: None,
                startup: None,
                realtime,
                rtc: None,
            });
            audio.realtime = realtime;
            audio.status(peer, audio.phase)
        };
        event(StateEvent::AudioStream(status));
    }

    pub fn start_audio(&self, peer: &str, event: Event) -> Result<(), NativeCommandError> {
        let mut nonce = [0u8; 16];
        openssl::rand::rand_bytes(&mut nonce).map_err(|_| NativeCommandError::QueueFull)?;
        let id = hex::encode(nonce);
        let startup: StartupSignal = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let (rtc_control, rtc_input) = crate::audio_rtc::RtcControl::new();
        let realtime;
        let status = {
            let mut inner = self.inner.lock().unwrap();
            if !inner.active.contains_key(peer) || !inner.audio.contains_key(peer) {
                return Err(NativeCommandError::Offline);
            }
            realtime = inner.audio[peer].realtime;
            let queue = inner.outbox.entry(peer.to_owned()).or_default();
            if queue.len() >= crate::limits::MAX_OUTBOX_PER_PEER {
                return Err(NativeCommandError::QueueFull);
            }
            if !realtime {
                queue.push(Message::AudioOffer {
                    protocol: WIRE_VERSION,
                    stream_id: id.clone(),
                });
            }
            let audio = inner.audio.get_mut(peer).unwrap();
            audio.cancel();
            audio.id.clone_from(&id);
            audio.phase = AudioStreamPhase::Starting;
            audio.socket = None;
            audio.rtc = realtime.then_some(rtc_control);
            audio.startup = Some(startup.clone());
            audio.status(peer, audio.phase)
        };
        event(StateEvent::AudioStream(status));
        if realtime {
            crate::audio_rtc::start(
                self.clone(),
                peer.to_owned(),
                id.clone(),
                event.clone(),
                rtc_input,
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
            let timed_out = backend
                .inner
                .lock()
                .unwrap()
                .audio
                .get(&peer)
                .is_some_and(|a| a.id == id && a.phase == AudioStreamPhase::Starting);
            if timed_out {
                backend.finish_audio(&peer, &id, AudioStreamPhase::Failed, &event);
            }
        });
        Ok(())
    }

    pub fn stop_audio(&self, peer: &str, event: Event) -> Result<(), NativeCommandError> {
        let id = self
            .inner
            .lock()
            .unwrap()
            .audio
            .get(peer)
            .map(|a| a.id.clone())
            .ok_or(NativeCommandError::Offline)?;
        self.finish_audio(peer, &id, AudioStreamPhase::Idle, &event);
        Ok(())
    }

    pub(crate) fn audio_playing(&self, peer: &str, id: &str, event: &Event) {
        let status = {
            let mut inner = self.inner.lock().unwrap();
            let Some(audio) = inner.audio.get_mut(peer) else {
                return;
            };
            if audio.id != id
                || audio.phase != AudioStreamPhase::Starting
                || (audio.socket.is_none() && audio.rtc.is_none())
            {
                return;
            }
            audio.phase = AudioStreamPhase::Streaming;
            audio.finish_startup();
            audio.status(peer, audio.phase)
        };
        event(StateEvent::AudioStream(status));
    }

    pub(crate) fn finish_audio(
        &self,
        peer: &str,
        id: &str,
        phase: AudioStreamPhase,
        event: &Event,
    ) {
        let status = {
            let mut inner = self.inner.lock().unwrap();
            let Some(audio) = inner.audio.get_mut(peer) else {
                return;
            };
            if audio.id != id {
                return;
            }
            audio.cancel();
            audio.socket = None;
            audio.rtc = None;
            audio.id.clear();
            audio.phase = phase;
            // Also stops a phone waiting for data or an offer still in flight.
            if let Some(queue) = inner.outbox.get_mut(peer) {
                queue.retain(|message| !matches!(message, Message::AudioOffer { stream_id, .. } | Message::AudioRtcOffer { stream_id, .. } if stream_id == id));
                if queue.len() < crate::limits::MAX_OUTBOX_PER_PEER {
                    queue.push(Message::AudioStop {
                        protocol: WIRE_VERSION,
                        stream_id: id.to_owned(),
                    });
                }
            }
            inner.audio[peer].status(peer, phase)
        };
        event(StateEvent::AudioStream(status));
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
            && audio.phase == AudioStreamPhase::Starting
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
        queue.push(Message::AudioRtcOffer {
            protocol: WIRE_VERSION,
            stream_id: id.to_owned(),
            sdp,
        });
        true
    }

    pub fn stop_all_audio(&self) {
        for session in self.inner.lock().unwrap().audio.values() {
            session.cancel();
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
