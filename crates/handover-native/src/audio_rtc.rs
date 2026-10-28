//! Opus over standard WebRTC. SDP travels only on the paired TLS control channel.
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use handover_core::AudioStreamPhase;
use rtc::media::Sample;
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
    RtpCodecKind,
};
use rtc::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
use tokio::sync::{mpsc, watch};
use webrtc::media_stream::track_local::{TrackLocal, static_sample::TrackLocalStaticSample};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
    RTCPeerConnectionState, RTCSessionDescription, Registry, register_default_interceptors,
};

use crate::NativeBackend;
use crate::audio::Event;

pub(crate) struct RtcControl {
    stop: watch::Sender<bool>,
    answer: mpsc::Sender<String>,
}
pub(crate) struct RtcInput {
    stop: watch::Receiver<bool>,
    answer: mpsc::Receiver<String>,
}
impl RtcControl {
    pub(crate) fn new() -> (Self, RtcInput) {
        let (stop, stopped) = watch::channel(false);
        let (answer, answers) = mpsc::channel(1);
        (
            Self { stop, answer },
            RtcInput {
                stop: stopped,
                answer: answers,
            },
        )
    }
    pub(crate) fn cancel(&self) {
        let _ = self.stop.send(true);
    }
    pub(crate) fn answer(&self, sdp: String) {
        let _ = self.answer.try_send(sdp);
    }
}

struct Events {
    state: watch::Sender<RTCPeerConnectionState>,
    gathered: watch::Sender<bool>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Events {
    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        let _ = self.state.send(state);
    }

    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.gathered.send(true);
        }
    }
}

fn opus_codec() -> RTCRtpCodecParameters {
    RTCRtpCodecParameters {
        rtp_codec: RTCRtpCodec {
            mime_type: "audio/opus".to_owned(),
            clock_rate: 48000,
            channels: 2,
            sdp_fmtp_line: "minptime=10;useinbandfec=1;stereo=1".to_owned(),
            rtcp_feedback: vec![],
        },
        payload_type: 111,
    }
}

pub(crate) fn start(
    backend: NativeBackend,
    peer: String,
    id: String,
    event: Event,
    mut input: RtcInput,
) {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build();
        if let Ok(runtime) = runtime {
            runtime.block_on(async {
                let mut media = MediaEngine::default();
                if media.register_codec(opus_codec(), RtpCodecKind::Audio).is_err() {
                    return;
                }
                let Ok(registry) = register_default_interceptors(Registry::new(), &mut media) else {
                    return;
                };
                let (state_tx, mut state_rx) = watch::channel(RTCPeerConnectionState::New);
                let (gathered_tx, mut gathered_rx) = watch::channel(false);
                let pc = PeerConnectionBuilder::new()
                    .with_media_engine(media)
                    .with_interceptor_registry(registry)
                    .with_handler(Arc::new(Events {
                        state: state_tx,
                        gathered: gathered_tx,
                    }))
                    .with_udp_addrs(vec!["0.0.0.0:0", "[::]:0"])
                    .build()
                    .await;
                let Ok(pc) = pc else { return; };
                // Cancellation covers ICE gathering, negotiation and blocked capture.
                tokio::select! {
                    _ = input.stop.changed() => {},
                    result = stream(&backend, &peer, &id, &pc, &mut input.answer, &mut state_rx, &mut gathered_rx) => {
                        if result.is_err() { tracing::debug!("WebRTC audio session stopped"); }
                    }
                }
                let _ = pc.close().await;
            });
        }
        backend.finish_audio(&peer, &id, AudioStreamPhase::Failed, &event);
    });
}

type StreamError = Box<dyn std::error::Error + Send + Sync>;
async fn stream(
    backend: &NativeBackend,
    peer: &str,
    id: &str,
    pc: &dyn PeerConnection,
    answers: &mut mpsc::Receiver<String>,
    states: &mut watch::Receiver<RTCPeerConnectionState>,
    gathered: &mut watch::Receiver<bool>,
) -> Result<(), StreamError> {
    let output = crate::audio_output::OutputRouter::acquire(&backend.audio_output)?;
    let codec = opus_codec();
    let mut nonce = [0u8; 4];
    let ssrc = loop {
        openssl::rand::rand_bytes(&mut nonce)?;
        let value = u32::from_be_bytes(nonce);
        if value != 0 {
            break value;
        }
    };
    let track = Arc::new(TrackLocalStaticSample::new(
        Instant::now(),
        MediaStreamTrack::new(
            "handover".to_owned(),
            "audio".to_owned(),
            "audio".to_owned(),
            RtpCodecKind::Audio,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(ssrc),
                    ..Default::default()
                },
                codec: codec.rtp_codec,
                ..Default::default()
            }],
        ),
    )?);
    let transceiver = pc
        .add_transceiver_from_track(
            track.clone() as Arc<dyn TrackLocal>,
            Some(RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Sendonly,
                ..Default::default()
            }),
        )
        .await?;
    let sender = transceiver.sender().await?.ok_or("missing sender")?;
    pc.set_local_description(pc.create_offer(None).await?)
        .await?;
    gathered.wait_for(|complete| *complete).await?;
    let offer = pc.local_description().await.ok_or("missing offer")?;
    if !backend.audio_rtc_offer(peer, id, offer.sdp) {
        return Err("session ended".into());
    }
    let answer = answers.recv().await.ok_or("missing answer")?;
    pc.set_remote_description(RTCSessionDescription::answer(answer)?)
        .await?;
    loop {
        let state = *states.borrow_and_update();
        match state {
            RTCPeerConnectionState::Connected => break,
            RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => {
                return Err("connection failed".into());
            }
            _ => states.changed().await?,
        }
    }
    let payload_type = sender
        .get_parameters()
        .await?
        .rtp_parameters
        .codecs
        .iter()
        .find(|codec| codec.rtp_codec.mime_type.eq_ignore_ascii_case("audio/opus"))
        .ok_or("Opus was not negotiated")?
        .payload_type;
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
    let capture = child.stdout.take().ok_or("missing capture")?;
    let (tx, mut packets) = mpsc::channel(3);
    let worker = std::thread::spawn(move || {
        let _ = encode(capture, tx);
    });
    let _capture = Capture {
        child,
        worker: Some(worker),
    };
    let mut previous = None;
    loop {
        tokio::select! {
            packet = packets.recv() => {
                let (sequence, captured, data) = packet.ok_or("capture ended")?;
                if !output.healthy() { return Err("audio output ended".into()); }
                if captured.elapsed() > Duration::from_millis(60) { continue; }
                let skipped = previous.map_or(0, |last| sequence - last - 1);
                previous = Some(sequence);
                track.sample_writer(ssrc, payload_type).write_sample(&Sample {
                    data: data.into(),
                    duration: Duration::from_millis(20),
                    prev_dropped_packets: skipped.min(u16::MAX as u64) as u16,
                    ..Sample::new(captured)
                }).await?;
            }
            state = states.changed() => {
                state?;
                if matches!(*states.borrow(), RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed) { return Err("connection ended".into()); }
            }
        }
    }
}

struct Capture {
    child: Child,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn encode(
    mut capture: impl Read,
    packets: mpsc::Sender<(u64, Instant, Vec<u8>)>,
) -> Result<(), StreamError> {
    let mut encoder = opus::Encoder::new(48000, opus::Channels::Stereo, opus::Application::Audio)?;
    encoder.set_bitrate(opus::Bitrate::Bits(96000))?;
    encoder.set_inband_fec(true)?;
    encoder.set_packet_loss_perc(10)?;
    let mut raw = [0u8; 3840];
    let mut pcm = [0i16; 1920];
    let mut sequence = 0;
    while !packets.is_closed() {
        capture.read_exact(&mut raw)?;
        for (value, bytes) in pcm.iter_mut().zip(raw.as_chunks::<2>().0) {
            *value = i16::from_le_bytes([bytes[0], bytes[1]]);
        }
        let data = encoder.encode_vec(&pcm, 1200)?;
        let _ = packets.try_send((sequence, Instant::now(), data));
        sequence += 1;
    }
    Ok(())
}
