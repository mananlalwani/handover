# Laptop audio on the phone

Install the updated Android app and Linux binaries, then pair the devices.
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

Handover captures the default laptop output's monitor when the stream starts.
It mirrors that output and leaves local routing and volume alone. Restart the
stream after changing the laptop's default output. This version does not
create a selectable phone sink or capture one chosen app.

## Latency and status

Audio is uncompressed 48 kHz stereo signed 16-bit PCM, about 1.54 Mbit/s before
network overhead. Capture requests 10 ms latency and sends 10 ms blocks.
Both capture and reception keep at most three queued blocks. Android requests
a 20 ms output buffer and low latency playback, which the device may adjust.
Old queued audio is dropped when playback falls behind.

These buffer settings are requests, not measured end-to-end latency. Bluetooth
adds codec, radio, and headphone buffering. For the lowest total delay, use
the phone speaker or wired headphones. Wi-Fi congestion can cause gaps or
stop the stream. TLS over TCP can wait for lost packets before newer audio
arrives.

`audio start` reports command acceptance. `audio status` reports `starting`
until the phone confirms that its AudioTrack started. `streaming` means that
Android accepted audio for playback; it does not confirm audible sound or
measure Bluetooth delay. `failed` means startup or streaming ended with an
error; `idle` means the receiver is available or was stopped from the desktop.

## Transport and compatibility

Public IPC uses `audio.start`, `audio.stop`, and `audio.list` with normalized
device IDs. Clients opt in with `audio.subscribe` and receive `audio_stream_updated` events and a separate
`audio_streams` snapshot when subscribing and after lag recovery.

The phone advertises support with `audio_ready` on each authenticated control
session. The desktop offers a random stream ID. The phone opens a second
TLS 1.3 connection to the same desktop address and port and sends
`audio_receive`. The desktop requires the active paired certificate and the
current offered stream ID. The phone pins the desktop certificate using its
existing pairing identity. The stream ID alone cannot authorize playback.

After an `audio_format` JSON frame, the connection carries repeated binary
records: an eight-byte big-endian sender timestamp in monotonic microseconds,
then exactly 1,920 bytes of PCM. The timestamp helps discard delayed batches;
it is not a synchronized latency measurement. Control and file traffic use
the original connection. Old apps do not advertise audio support.

Implementation checks cover certificate rejection and stale stream commands.
Phone playback and Bluetooth latency still need a live device check.

Platform references: [PulseAudio latency control](https://www.freedesktop.org/wiki/Software/PulseAudio/Documentation/Developer/Clients/LatencyControl/),
[Android AudioTrack](https://developer.android.com/reference/android/media/AudioTrack),
and [Android audio focus](https://developer.android.com/media/optimize/audio-focus).
