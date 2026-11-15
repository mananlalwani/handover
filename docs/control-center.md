# Native control center

Launch **Handover**, run `handover-control`, or run `handover-gui` without
arguments. The launcher prefers the native control center installed beside it.
`handover-gui --qml` opens the Quickshell reference client, and
`handover-gui --send` keeps the existing desktop send picker.

The control center uses Rust and GTK4. It needs GTK 4.10 and libadwaita 1.4 or
newer. Install matching binaries with `make install-user` or the release tarball
installer.

## Phone and task views

The phone pane stays visible while you switch between Notifications, Files,
Contacts, Clipboard, and Media. It shows reported connection, battery, and
network state, with actions for files, links, and clipboard text. Offline battery values are
labeled as last known. Messages opens the separate native Messages app.

The app selects the only connected paired device when there is one. Use the
phone-name menu to switch devices or view all devices. Selection changes preserve
files staged for review. Submitted files keep their original recipient.
A daemon disconnect disables controls and retains last reported data until the
subscription recovers. Slow request replies run separately from the live
subscription. Work still queued when that session is lost is rejected; in-flight
commands may finish, but are never resubmitted. Unrelated battery or media events preserve the current
notification or contact view and keyboard focus.

## Devices and pairing

Open Devices & pairing to review trusted devices and pending requests. Open
Handover on Android and connect to the computer, then refresh Devices. Compare
the eight-digit code with the phone, check that the codes match, and approve.
Reading a request never approves it. Forget device requires confirmation and
removes saved trust; pairing will be required again. Fingerprints and manual
connection help are available in expandable details.

## Notifications

Notifications groups current forwarded notifications by app and supports search.
The close button requests dismissal on the phone and appears only for clearable
notifications. Other buttons come from reported phone actions. Commands are
disabled for disconnected devices. Acceptance alone does not change phone state.
Reply opens a text editor only when the notification reports reply support.
Enter adds a new line; Send reply submits. Drafts stay in this window when a
notification updates, the phone disconnects, or the editor closes. Use Reply
drafts to recover or discard them, including drafts for removed notifications.
Removed notifications cannot receive replies. Closing Handover with drafts asks
before discarding them.

A lost acknowledgement keeps the text and displays the unknown outcome. The app
never resends automatically. An accepted reply clears its draft but does not
claim delivery or dismiss the notification. The control center itself does not
create desktop popups.

Choose which apps forward notifications in the Android app's Notifications page.
The control center does not change those phone preferences.

## Files, links, and clipboard

Choose Send file in the phone pane, use Choose files in Files, or drop local files
anywhere on the control center. Selection opens Files for review and sends
nothing. Keep up to eight files in the selection, choose a connected phone, then
press Send. Each file is a separate tracked transfer. Duplicate selections are
ignored, and an oversized selection is rejected without partially adding it.
Original filenames and bytes go through the existing file-sharing API; the app
does not convert images or rename source-code files.

Each selected file distinguishes submission, acceptance, confirmed receipt,
reported failure, and an unknown outcome. Byte progress does not establish
receipt. Lost replies leave files unconfirmed and are never retried automatically.
Check the phone before submitting an unconfirmed file again. Progress-triggered
activity refreshes are coalesced; terminal results refresh immediately.

Selections belong to the current window. Closing with unsent, submitting, or
unconfirmed files asks before discarding the selection. Closing or removing a
row does not cancel a submitted transfer. Remove old rows to make room for more
files.

Send link and Send clipboard text open a review dialog. Nothing is sent until
Send is chosen. File and link controls require the device's reported sharing
capability, and the daemon validates each resource before accepting it.

Recent transfers is owned by the daemon and survives closing the control center.
It retains up to 128 observations for the current daemon session and resets on
daemon restart. It is not a durable archive. Accepted requests without later
observations may be absent. Completed incoming files have an explicit Open
containing folder action. The control center never opens received paths or URLs
automatically. Forgetting a device clears its activity metadata, not received files.

## Clipboard history

Clipboard shows the daemon's saved text history for this computer, across all
devices. The selected phone does not filter it. Opening the page or pressing
Refresh reads the current history. Search filters text, Copy puts the complete
entry on the desktop clipboard, and Pin keeps an entry in history. Pinning and
clearing refresh from the daemon after acceptance.

Clear history asks first and keeps pinned entries by default. Include pinned
entries only with the checkbox. A disconnected daemon disables history changes;
already loaded text remains available to copy. Opening this page does not enable
clipboard mirroring or change Android permissions.

## Contacts and media

Contacts reads synced address books. Select a connected phone and use the sync
button to request a refresh, which still needs Android contact permission.
Search by name, phone number, or email and copy an address. The view shows up to
100 matches. Contact photos and calling controls are not yet part of this view.

Media shows reported phone players and supported Previous, Play, Pause,
Play/Pause, and Next controls. The central control follows the reported playback
state. Commands do not optimistically change that state.

## Validation

Run `python3 scripts/test-handover-control.py` for the private-display GTK and IPC
fixture. `--dark`, `--width`, `--page`, and `--screenshot` support layout checks.
The runner requires Xvfb and dbus-daemon; screenshots also need ImageMagick.
Fictional devices and a private session bus exercise device selection,
notification actions, reply drafts and lost acknowledgements, clipboard search,
copy, pinning and clearing, pairing comparison, drag-and-drop review, independent
file outcomes, lost replies, contact copy, media controls, and reconnect without command
replay. Physical-phone effects need separate live validation.
