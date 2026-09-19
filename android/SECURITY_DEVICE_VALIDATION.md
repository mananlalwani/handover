# Android security validation

These checks need a physical device or an emulator with a real Wi-Fi network. Run them against a debug build with a paired Linux desktop. Record the Android version, OEM, build fingerprint, and whether the device is on Wi-Fi or cellular.

## Permissions and exported components

1. In Settings, deny overlay access. Leave clipboard sync enabled but background overlay assist disabled. Confirm a background clipboard read does not open `ClipboardReadActivity`.
2. Grant overlay access, but leave the Handover overlay-assist switch off. Confirm the same behavior. Turn on both clipboard sync and overlay assist, then confirm a clipboard change is read only after the normal clipboard path returns empty.
3. Revoke the ADB `READ_LOGS` grant (`adb shell pm revoke org.handover.android android.permission.READ_LOGS`). Confirm the logcat clipboard monitor does not start. Grant it again only on a test device and confirm the monitor filters `ClipboardService` lines and the app package.
4. Send explicit broadcasts to the test receiver without `org.handover.android.permission.TEST_CONTROL`. Android must reject them. A caller holding the signature permission may exercise the harmless test actions.
5. Confirm `ClipboardReadActivity`, `HandoverForegroundService`, and `UpdateInstalledReceiver` cannot be launched by an external package.

## Discovery and pairing

1. Put the phone and desktop on the same Wi-Fi network. Confirm multicast discovery finds the desktop only while the app has local-network access and the foreground service is running.
2. Block multicast or move the devices to different networks. Confirm discovery stops and no peer is shown as paired.
3. Pair using the displayed comparison code. Repeat with a changed code and reject it. Confirm no device state, clipboard, URL, or file arrives before approval.
4. Unpair, then reconnect from the old desktop. Confirm the stored certificate pin is rejected and pairing is required again.

## URLs, files, and updates

1. Send `https://` and `http://` URLs. Confirm Android opens the browser chooser or browser without a content-URI grant.
2. Attempt `content:`, `file:`, `javascript:`, and `data:` URL values from the desktop. Confirm each transfer is rejected and no notification opens an app.
3. Start a file transfer, then make the desktop send one small chunk every few seconds. Confirm the phone closes the transfer at the total deadline, removes the partial file or pending MediaStore row, and reports `timed_out`.
4. Send an APK with the wrong package, an older version, or a different signing certificate. Confirm it is not offered for installation.
5. Send a valid newer APK. Confirm Android shows its own install confirmation UI. Cancel it and confirm Handover does not install anything silently.

Keep the device logs and transfer IDs for failed cases. Do not include clipboard contents, message bodies, tokens, or private keys in a bug report.
