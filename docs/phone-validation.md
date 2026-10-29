# Phone validation handoff

The desktop send picker, notification bridge, MPRIS adapter, and IPC recovery
have local fixture coverage. These checks need a phone and, for audible playback,
a person listening. They have not been completed by those fixtures.

Record the Linux commit, Android app version, Android version, phone model, and
network used. The Android security checks remain in
[security device validation](../android/SECURITY_DEVICE_VALIDATION.md).

## Desktop sharing

After installing the Linux build, run `handover-gui --send` or open a file with
Send to phone. Send a small file whose name contains spaces and Unicode, and an
HTTPS link. Check the file's name and contents on the phone and confirm the link
opens. Compare the desktop's accepted, progress, and completed states with the
actual result. Disconnect during a transfer and confirm it does not claim success
or send again automatically.

## Notifications and media

Enable the optional client with
`systemctl --user enable --now handover-desktop.service`.

Post a phone notification. Confirm the desktop alert appears and updates in
place. Close the desktop popup and check that the phone notification remains.
Use Dismiss on phone on a clearable notification and observe its actual removal.
Exercise an action supplied by a phone app and check that the app acts.

Play media on the phone. Confirm its title and reported playback state appear in
Linux media controls. Exercise advertised Play, Pause, Play/Pause, Next, and
Previous controls and watch the phone's actual response. Disconnect and reconnect;
players should disappear and rebuild without replaying commands. Unknown playback
must not become an invented paused or stopped player.

During a phone call, check that the existing local-player pause behavior still
works and that Handover's MPRIS player does not send an extra pause to the phone.

## Google Messages and audio

Fresh Google Messages pairing needs the phone's confirmation. Use a separate
test session when available. Check that the paired account receives live updates
and that text and media outcomes follow provider evidence. A successful send
request alone does not prove delivery.

Start laptop audio with `handoverctl audio start "Phone"`. Listen on the phone
speaker or connected headphones. Confirm audible playback, then stop it and check
that desktop output returns. The existing TLS/Opus fixture proves decoding, not
audibility or Bluetooth latency.

## Recovery

Exercise laptop suspend/resume and a network change while the devices are paired.
Check that the daemon reconnects, clients rebuild, and previous sends or actions
are not repeated. Note whether pairing, notification access, and media permissions
remain valid. Capture failure IDs and fixed diagnostics; omit message bodies,
clipboard contents, tokens, and keys.
