//! Opus over standard WebRTC. SDP travels only on the paired TLS control channel.
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use handover_core::AudioStreamPhase;
use tokio::sync::{mpsc, watch};
use webrtc::api::{
    APIBuilder, interceptor_registry::register_default_interceptors, media_engine::MediaEngine,
};
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::{
    configuration::RTCConfiguration, peer_connection_state::RTCPeerConnectionState,
    sdp::session_description::RTCSessionDescription,
};
use webrtc::rtp_transceiver::{
    RTCRtpTransceiverInit, rtp_codec::RTCRtpCodecCapability,
    rtp_transceiver_direction::RTCRtpTransceiverDirection,
};
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

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
                if media.register_default_codecs().is_err() { return; }
                let Ok(registry) = register_default_interceptors(Registry::new(), &mut media) else { return; };
                let api = APIBuilder::new().with_media_engine(media).with_interceptor_registry(registry).build();
                let Ok(pc) = api.new_peer_connection(RTCConfiguration::default()).await else { return; };
                let pc = Arc::new(pc);
                let (state_tx, mut state_rx) = watch::channel(RTCPeerConnectionState::New);
                pc.on_peer_connection_state_change(Box::new(move |state| {
                    let tx = state_tx.clone();
                    Box::pin(async move { let _ = tx.send(state); })
                }));
                // Cancellation also covers ICE gathering, negotiation and blocked capture.
                tokio::select! {
                    _ = input.stop.changed() => {},
                    result = stream(&backend, &peer, &id, &pc, &mut input.answer, &mut state_rx) => {
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
    pc: &Arc<webrtc::peer_connection::RTCPeerConnection>,
    answers: &mut mpsc::Receiver<String>,
    states: &mut watch::Receiver<RTCPeerConnectionState>,
) -> Result<(), StreamError> {
    let output = crate::audio_output::OutputRouter::acquire(&backend.audio_output)?;
    let track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: "audio/opus".to_owned(),
            clock_rate: 48000,
            channels: 2,
            sdp_fmtp_line: "minptime=10;useinbandfec=1;stereo=1".to_owned(),
            ..Default::default()
        },
        "audio".to_owned(),
        "handover".to_owned(),
    ));
    let transceiver = pc
        .add_transceiver_from_track(
            track.clone(),
            Some(RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Sendonly,
                send_encodings: vec![],
            }),
        )
        .await?;
    let sender = transceiver.sender().await;
    // Drain RTCP so sender reports and negotiated feedback do not accumulate.
    let feedback = AbortTask(tokio::spawn(async move {
        let mut buffer = [0u8; 1500];
        while sender.read(&mut buffer).await.is_ok() {}
    }));
    let result = async {
        let mut gathered = pc.gathering_complete_promise().await;
        let offer = pc.create_offer(None).await?;
        pc.set_local_description(offer).await?;
        gathered.recv().await;
        let offer = pc.local_description().await.ok_or("missing offer")?;
        if !backend.audio_rtc_offer(peer, id, offer.sdp) { return Err("session ended".into()); }
        let answer = answers.recv().await.ok_or("missing answer")?;
        pc.set_remote_description(RTCSessionDescription::answer(answer)?).await?;
        loop {
            let state = *states.borrow_and_update();
            match state {
                RTCPeerConnectionState::Connected => break,
                RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => return Err("connection failed".into()),
                _ => states.changed().await?,
            }
        }
        let mut child = Command::new("parec").args([
            &format!("--device={}", output.monitor), "--raw", "--format=s16le", "--rate=48000", "--channels=2", "--latency-msec=10", "--process-time-msec=10",
        ]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let capture = child.stdout.take().ok_or("missing capture")?;
        let (tx, mut packets) = mpsc::channel(3);
        let worker = std::thread::spawn(move || { let _ = encode(capture, tx); });
        let _capture = Capture { child, worker: Some(worker) };
        let mut previous = None;
        loop {
            tokio::select! {
                packet = packets.recv() => {
                    let (sequence, captured, data) = packet.ok_or("capture ended")?;
                    if !output.healthy() { return Err("audio output ended".into()); }
                    if captured.elapsed() > Duration::from_millis(60) { continue; }
                    let skipped = previous.map_or(0, |last| sequence - last - 1);
                    previous = Some(sequence);
                    track.write_sample(&Sample { data: data.into(), duration: Duration::from_millis(20), prev_dropped_packets: skipped.min(u16::MAX as u64) as u16, ..Default::default() }).await?;
                }
                state = states.changed() => {
                    state?;
                    if matches!(*states.borrow(), RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed) { return Err("connection ended".into()); }
                }
            }
        }
    }.await;
    drop(feedback);
    result
}

struct AbortTask(tokio::task::JoinHandle<()>);
impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
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
