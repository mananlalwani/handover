# Laptop audio on the phone

Install the updated Android app and Linux binaries, then pair the devices.
Building the Linux sender requires Rust 1.88 or newer.
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
notification has a Stop audio action. Playback stops on audio focus loss,
disconnect, unpairing, or a stream error. Start again after reconnecting.

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

Current apps use WebRTC with 48 kHz stereo Opus, 20 ms packets, and a
96 kbit/s encoder target. WebRTC handles encrypted RTP, packet loss and adaptive
jitter buffering. Capture requests 10 ms latency and keeps at most three encoded
packets queued. Capture packets older than 60 ms are discarded before sending.

The phone requests Android's low latency Wi-Fi mode during the session. Android
decides whether that lock is effective for the current screen and foreground
state. The lock is released when the stream ends.

These settings are requests, not measured end-to-end latency. Bluetooth adds
codec, radio, and headphone buffering. For the lowest total delay, use the phone
speaker or wired headphones. Wi-Fi congestion can still cause audible gaps.

`audio start` reports command acceptance. `audio status` reports `starting`
until the phone confirms that its audio output started and, for WebRTC, the
peer connection is connected. `streaming` means that
the phone media output is running for the session; it does not confirm audible sound or
measure Bluetooth delay. `failed` means startup or streaming ended with an
error; `idle` means the receiver is available or was stopped from the desktop.

## Transport and compatibility

Public IPC uses `audio.start`, `audio.stop`, and `audio.list` with normalized
device IDs. Clients opt in with `audio.subscribe` and receive `audio_stream_updated` events and a separate
`audio_streams` snapshot when subscribing and after lag recovery.

The phone advertises support with `audio_ready` on each authenticated control
session. Current apps include `realtime: true`. The desktop sends an
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

Implementation checks cover certificate rejection and stale stream commands.
A 60-second WebRTC test through Buds4 Pro kept the laptop outputs muted,
and the listener reported smooth beeps. Total Bluetooth
latency has not been measured.

Platform references: [PulseAudio latency control](https://www.freedesktop.org/wiki/Software/PulseAudio/Documentation/Developer/Clients/LatencyControl/),
[PulseAudio null sink](https://www.freedesktop.org/wiki/Software/PulseAudio/Documentation/User/Modules/),
[Android AudioTrack](https://developer.android.com/reference/android/media/AudioTrack),
and [Android audio focus](https://developer.android.com/media/optimize/audio-focus),
and [Android Wi-Fi locks](https://developer.android.com/reference/android/net/wifi/WifiManager#WIFI_MODE_FULL_LOW_LATENCY).
