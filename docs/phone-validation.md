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

Try **Cancel queued transfer**. A confirmed cancellation removes a share still in
the local send queue. If the backend has already taken it for delivery, the UI
must report that cancellation was unavailable and wait for the transfer's result.
Closing the send window must not cancel an accepted transfer.

## Android sharing

In another Android app, select Share, then Send to PC. Send one photo, several
files, and an HTTPS link. Confirm the paired desktop in the dialog. Check the
received files' names and contents in `Downloads/Handover/YYYY-MM-DD_HH-MM-SS/`
on Linux and compare transfer history with
the actual results. Cancel a share and verify nothing arrives. Try sharing while
offline and confirm the app asks you to connect rather than claiming success.

The action accepts up to 32 content-provider files per share, with the existing
100 MiB limit per file, or one HTTP(S) link. Plain text without a web URL is not
supported. URI permissions and provider behavior still need checking with the
phone's gallery and file manager.

## Requested phone downloads

Install the matching Android build. In Remote desktop, select Share all phone
storage and allow Android's all-files permission. On Linux, verify Internal
storage and mounted SD or USB volumes appear. Browse Downloads and DCIM. Stop
sharing, and separately revoke the system permission, then confirm new requests
fail. Test reconnect and app restart without another per-file phone approval.
Protected app data must remain inaccessible. The emulator fixture covers the
permission gate, exact file bytes, read-only path confinement, and stop sharing;
physical removable-storage mounts still need a phone check.

Also test Share one folder instead with an eligible subfolder.
On Linux, open Files > On phone, list that folder, and download a source file,
an image, an empty file, and a file with a Unicode filename. Compare names and
byte hashes with the originals. Confirm Saved appears only after the file exists
in Downloads/Handover. Try a provider with unknown file sizes as well.

Remove the folder grant and try browsing and downloading again. Both must fail
with an access explanation. Change the shared folder during a large transfer
and confirm the old request stops. Disconnect during receiving and confirm there
is no completed entry or visible partial file. Reconnect without replaying the
request. Restart the desktop window during a download and confirm its status
rebuilds from daemon activity. Test with the previous Android build and confirm
Download is unavailable until that app is updated.

With matching builds, download a file larger than 100 MiB, select several files,
and download a folder containing nested and empty folders. Browse more than 512
entries with Load more. Compare the final bytes and filenames. Disconnect during
a large download, restart the daemon, and confirm it remains paused until Resume.
Change the source before resuming and confirm it fails rather than splicing bytes.
Cancel during preparation, receiving, and verification. No cancelled or paused
request may appear as Saved or expose a completed partial file.

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

## Phone volume and Bluetooth calls

Refresh phone volume from Linux. Change media, alarm, ringtone, and call streams
where Android permits it and compare the reported level with Android. Exercise
Do Not Disturb and fixed-volume restrictions. Disconnect before a result and
confirm the outcome is unconfirmed rather than accepted or changed.

Pair a test phone with Linux Bluetooth. In Calls choose its endpoint and explicit
microphone and speakers. Arm it while idle and confirm no microphone capture.
During an active call, check sound in both directions and verify changing local
audio defaults does not redirect the chosen route. Hang up, disconnect, stop
routing, restart the daemon, and terminate it abruptly. Confirm its microphone
capture ends in each case. Never auto-arm after restart. Check profile recovery
without changing a profile that another app has selected or is using.

The private process fixture verifies ownership, abrupt-parent cleanup, and actual
PipeWire link evidence. It does not establish physical Bluetooth audibility.

## Recovery

Exercise laptop suspend/resume and a network change while the devices are paired.
Check that the daemon reconnects, clients rebuild, and previous sends or actions
are not repeated. Note whether pairing, notification access, and media permissions
remain valid. Capture failure IDs and fixed diagnostics; omit message bodies,
clipboard contents, tokens, and keys.
