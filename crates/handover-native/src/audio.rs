//! A separate, pinned TLS connection carries fixed-size PCM blocks.
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use handover_core::{AudioStreamPhase, AudioStreamStatus, DeviceId, StateEvent};
use openssl::ssl::SslStream;

use crate::protocol::{Message, write_frame};
use crate::{NativeBackend, NativeCommandError, NativeError, WIRE_VERSION};

type Event = Arc<dyn Fn(StateEvent) + Send + Sync>;
const BLOCK_BYTES: usize = 1920; // 480 frames, stereo signed 16-bit LE, 48 kHz.
type StartupSignal = Arc<(Mutex<bool>, std::sync::Condvar)>;

pub(crate) struct AudioSession {
    id: String,
    phase: AudioStreamPhase,
    socket: Option<TcpStream>,
    startup: Option<StartupSignal>,
}

impl AudioSession {
    pub(crate) fn cancel(&self) {
        self.finish_startup();
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

    pub(crate) fn audio_ready(&self, peer: &str, event: &Event) {
        let status = {
            let mut inner = self.inner.lock().unwrap();
            let audio = inner.audio.entry(peer.to_owned()).or_insert(AudioSession {
                id: String::new(),
                phase: AudioStreamPhase::Idle,
                socket: None,
                startup: None,
            });
            audio.status(peer, audio.phase)
        };
        event(StateEvent::AudioStream(status));
    }

    pub fn start_audio(&self, peer: &str, event: Event) -> Result<(), NativeCommandError> {
        let mut nonce = [0u8; 16];
        openssl::rand::rand_bytes(&mut nonce).map_err(|_| NativeCommandError::QueueFull)?;
        let id = hex::encode(nonce);
        let startup: StartupSignal = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let status = {
            let mut inner = self.inner.lock().unwrap();
            if !inner.active.contains_key(peer) || !inner.audio.contains_key(peer) {
                return Err(NativeCommandError::Offline);
            }
            let queue = inner.outbox.entry(peer.to_owned()).or_default();
            if queue.len() >= crate::limits::MAX_OUTBOX_PER_PEER {
                return Err(NativeCommandError::QueueFull);
            }
            queue.push(Message::AudioOffer {
                protocol: WIRE_VERSION,
                stream_id: id.clone(),
            });
            let audio = inner.audio.get_mut(peer).unwrap();
            audio.cancel();
            audio.id.clone_from(&id);
            audio.phase = AudioStreamPhase::Starting;
            audio.socket = None;
            audio.startup = Some(startup.clone());
            audio.status(peer, audio.phase)
        };
        event(StateEvent::AudioStream(status));
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
            if audio.id != id || audio.phase != AudioStreamPhase::Starting || audio.socket.is_none()
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
            audio.id.clear();
            audio.phase = phase;
            // Also stops a phone waiting for data or an offer still in flight.
            if let Some(queue) = inner.outbox.get_mut(peer) {
                queue.retain(|message| !matches!(message, Message::AudioOffer { stream_id, .. } if stream_id == id));
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
            if audio.id != id || audio.phase != AudioStreamPhase::Starting || audio.socket.is_some()
            {
                return Err(NativeError::InvalidFrame);
            }
            audio.socket = Some(tls.get_ref().try_clone()?);
        }
        let result = self.stream_pcm(&mut tls);
        self.finish_audio(peer, id, AudioStreamPhase::Failed, event);
        result
    }

    fn stream_pcm(&self, tls: &mut SslStream<TcpStream>) -> Result<(), NativeError> {
        tls.get_ref().set_nodelay(true)?;
        tls.get_ref()
            .set_write_timeout(Some(Duration::from_millis(100)))?;
        // PulseAudio compatibility is also provided by pipewire-pulse.
        // Capture the default output's monitor without changing local routing.
        let mut child = Command::new("parec")
            .args([
                "--device=@DEFAULT_MONITOR@",
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
        // The producer always drains capture. Keep only the most recent 30 ms.
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
                if queue.0.len() == 3 {
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
                if started.elapsed().as_micros() as u64 - timestamp > 30_000 {
                    continue;
                }
                tls.write_all(&timestamp.to_be_bytes())?;
                tls.write_all(&pcm)?;
                tls.flush()?;
            }
        })();
        let _ = child.kill();
        let _ = child.wait();
        let _ = producer.join();
        result
    }
}
