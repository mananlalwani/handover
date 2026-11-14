# Laptop audio on the phone

Install the updated Android app and Linux binaries, then pair the devices.
Building the Linux sender requires Rust 1.91 or newer.
The laptop needs `parec` and a running PulseAudio server or `pipewire-pulse`.
On Arch, `parec` comes from `libpulse`; on Ubuntu, from `pulseaudio-utils`.

Connect your Bluetooth headphones to the phone. Start audio from Handover's
desktop Media page or use a device name from `handoverctl devices`:

```sh
handoverctl audio start "Phone"
handoverctl audio status
handoverctl audio stop "Phone"
```

The phone uses Android's current media output and media volume. Its Handover
notification has a Stop audio action. In the phone app, open **Laptop audio**
and use **Start stream** or **Pause stream** to start or stop delivery to this phone.
Pausing the stream leaves the laptop player running. Laptop audio outputs are
restored after the last stream stops. Starting again creates a new stream. Repeated start requests leave an
already starting or running stream intact. These controls require the updated
desktop daemon and keep the paired connection active.

Tap **Pause / resume** to toggle the laptop player without ending the stream.
This button requires the updated desktop daemon and `playerctl`, and works with
players that expose MPRIS controls. With `playerctld` running, it targets the most
recently active player; otherwise it uses the first available local player.
Handover's phone-player proxy is excluded. The app reports that a request was
queued, not that playback changed. No Android media session or notification
playback buttons are added.

The daemon remembers stream intent while it runs. A temporary control disconnect
retains that intent and resumes after the paired phone reconnects and advertises
readiness. A failed audio session gets at most five automatic retries, delayed
by 1, 2, 4, 8, and 16 seconds. Thirty seconds of uninterrupted confirmed receiver
output resets the retry budget. Brief starts do not create an unlimited retry
loop. After exhaustion, use Start stream or Retry stream. Unpairing,
manual stop, and daemon shutdown cancel recovery. Intent is not restored after
a daemon restart.

On a temporary Android audio-focus interruption, playback pauses without
tearing down the receiver. It resumes when Android returns focus. With phone
call-state permission, an observed call ending also retries the pending focus
request. A permanent focus loss can require an explicit Start stream. Manual
pause removes that receiver, so late focus callbacks cannot resume it. A pause
requested while disconnected is remembered on the phone and reconciled before
the daemon resumes. Stop acknowledgments are correlated to the request; an old
status update cannot clear a newer pending pause.

The existing policy that pauses laptop players when a call starts is separate
from stream recovery. Resuming the stream does not issue a Play command to a
laptop player.

Handover creates a silent virtual output and moves app audio into it while
streaming. The phone receives that output's audio. Laptop outputs stay muted,
and laptop mute and volume controls do not change the phone stream. Use the
phone's media volume to adjust headphone loudness.

Apps that start playing during the stream are routed automatically. Stopping,
disconnecting, or shutting down restores their previous outputs and the laptop
outputs' previous mute settings. Multiple receivers share the virtual output;
restoration happens after the last stream stops. An audio routing journal lets
the next daemon startup restore routing after an unexpected daemon exit.

## Latency and status

Current apps use encrypted WebRTC with 48 kHz stereo Opus. Choose a preset in
the phone app, desktop Media page, or CLI:

```sh
handoverctl audio profile "Phone" balanced
handoverctl audio profile "Phone" music
handoverctl audio profile "Phone" low-delay
```

| Preset | Encoder target | Packet duration | Capture latency request | Discard capture packets older than |
| --- | --- | --- | --- | --- |
| Balanced, the default | 96 kbit/s | 20 ms | 10 ms | 60 ms |
| Music | 160 kbit/s | 20 ms | 10 ms | 100 ms |
| Low delay | 64 kbit/s | 10 ms | 5 ms | 30 ms |

Low delay uses Opus's restricted low-delay mode and requests faster adaptive
jitter-buffer acceleration on Android. These are configuration choices, not
measured audible-delay guarantees. All presets bound the capture queue to three
packets. A profile change restarts an active stream with the new configuration;
a rejected restart keeps the old profile. Legacy PCM receivers only support the
default profile. WebRTC still handles packet loss and adaptive jitter buffering.

The phone requests Android's low latency Wi-Fi mode during the session. Android
decides whether that lock is effective for the current screen and foreground
state. The lock is released when the stream ends.

These settings are requests, not measured end-to-end latency. Bluetooth adds
codec, radio, and headphone buffering. For the lowest total delay, use the phone
speaker or wired headphones. Wi-Fi congestion can still cause audible gaps.

`audio start` reports command acceptance. Status and profile selection in both
apps follow daemon reports rather than queue success. `starting` waits for the
phone to confirm its output started and its WebRTC connection is connected.
`streaming` confirms the receiver output is running, not that sound reached the
listener. `reconnecting` means recovery is pending, `suspended` means the receiver
is waiting for audio focus, `failed` means recovery stopped, and `idle` means the
stream was stopped or has not started. Status includes intent, retry count,
profile, failure cause, and a monotonically increasing session-status revision.
Older daemon snapshots leave unknown intent and profile fields absent.

Diagnostics are sampled at most once every two seconds per active session and
sent as current observations, not retained history. Missing values stay
unavailable. They include:

- The selected media path, classified as direct, VPN, relay, or unknown. VPN
  classification requires the selected candidate address to match an observed
  Android VPN interface; IP addresses are not included in diagnostics.
- Network round-trip time, RTP jitter, received/lost packet counters, and recent
  received payload bitrate. The first sample has no bitrate estimate.
- Lifetime-average receiver jitter-buffer delay, calculated from accumulated
  delay and emitted samples. This is receiver buffering, not network latency.
- Configured encoder bitrate target, packet duration, capture drops, and sample
  timestamps. Captured packets dropped by the sender are distinct from network
  packet loss.

Samples are cleared at stream replacement and receiver observations are cleared
on interruption. Slow control clients are disconnected when critical state
cannot fit the bounded queue; reconnecting rebuilds their view from current
state. Neither network RTT nor jitter-buffer measurements include Bluetooth,
DAC, headphone buffering, or total audible delay. Those remain unmeasured.

## Transport and compatibility

Public IPC uses `audio.start`, `audio.stop`, `audio.profile`, and `audio.list` with normalized
device IDs. Clients opt in with `audio.subscribe` and receive `audio_stream_updated` events and a separate
`audio_streams` snapshot when subscribing and after lag recovery.

The phone advertises support with `audio_ready` on each authenticated control
session. Current apps include `realtime: true` and `managed: true`, plus a pending
stop request ID when necessary. Managed clients receive normalized `audio_status`
updates and correlated `audio_stream_result` acknowledgments, and send receiver
state and diagnostic reports tied to the current stream ID. Legacy clients do
not receive these additional status frames. Focus recovery, stop acknowledgments,
and receiver diagnostics require the updated Android app. A legacy client's
unreasoned stop frame is treated as a manual stop, so it cannot trigger an
unwanted retry. The desktop sends an
`audio_rtc_offer` containing a fresh stream ID and WebRTC SDP; the phone returns
`audio_rtc_answer` on that same paired TLS connection. The SDP binds WebRTC's
DTLS certificate fingerprints to the existing authenticated peer. SDP, ICE
credentials and audio payloads are never logged. Stale stream IDs are ignored.

WebRTC gathers host ICE candidates for reachable local and VPN interfaces. No
external STUN or TURN service is configured. The devices must have a reachable
UDP path, for example on their LAN or Tailscale network. Audio uses DTLS-SRTP;
control and file traffic keep using the original TLS connection. The Android
peer receives audio only and creates no microphone source.

Apps advertising audio without `realtime` retain the separate TLS 1.3 PCM path.
The desktop requires the active paired certificate and current offered stream
ID on that connection. After `audio_format`, records contain an eight-byte
big-endian monotonic timestamp followed by 1,920 bytes of stereo signed 16-bit
PCM at 48 kHz. This legacy TCP path can stall behind lost packets. Apps predating
audio support do not advertise the capability.

Automated checks cover certificate rejection, stale stream commands, intent
recovery and stop cancellation, suspended negotiation, bounded retries, rejected
profile changes, actual Opus encode/decode for all presets, missing statistics,
route selection, and counter resets.
A 60-second WebRTC test through Buds4 Pro kept the laptop outputs muted,
and the listener reported smooth beeps. Total Bluetooth
latency has not been measured.

Platform references: [WebRTC statistics](https://www.w3.org/TR/webrtc-stats/),
[PulseAudio latency control](https://www.freedesktop.org/wiki/Software/PulseAudio/Documentation/Developer/Clients/LatencyControl/),
[PulseAudio null sink](https://www.freedesktop.org/wiki/Software/PulseAudio/Documentation/User/Modules/),
[Android AudioTrack](https://developer.android.com/reference/android/media/AudioTrack),
and [Android audio focus](https://developer.android.com/media/optimize/audio-focus),
and [Android Wi-Fi locks](https://developer.android.com/reference/android/net/wifi/WifiManager#WIFI_MODE_FULL_LOW_LATENCY).

The recovery and preset changes have not been live-tested through a phone call,
screen-off playback, Wi-Fi/VPN handoff, or extended Bluetooth listening. Their
unit and loopback checks do not establish audible latency or battery savings.
