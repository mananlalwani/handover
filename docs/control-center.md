# Native control center

Launch **Handover**, run `handover-control`, or run `handover-gui` without
arguments. The launcher prefers the native control center installed beside it.
`handover-gui --qml` opens the Quickshell reference client, and
`handover-gui --send` keeps the existing desktop send picker.

The control center uses Rust and GTK4. It needs GTK 4.10 and libadwaita 1.4 or
newer. Install matching binaries with `make install-user` or the release tarball
installer.

## Phone and task views

The window uses the native Messages app's neutral colors, sidebar selection,
header sizing, and spacing. Select a phone once in the sidebar. Its scope stays
visible as you move between Phone, Notifications, Calls, Files, Clipboard,
Contacts, and Media. Device settings contains trust and pairing. Messages opens
the separate native Messages app.

Phone groups device controls, including ringing, screen lock, awake-mode
requests, and hotspot settings. Controls submit requests; they do not show an
invented lock, awake, or hotspot state. Offline battery values are labeled as
last known. File transfers and phone folder browsing share the Files view.

The app selects the only connected paired device when there is one. Use the
phone-name menu to switch devices or view all devices. Selection changes preserve
files staged for review. Submitted files keep their original recipient.
A daemon disconnect disables controls and retains last reported data until the
subscription recovers. Slow request replies run separately from the live
subscription. Work still queued when that session is lost is rejected; in-flight
commands may finish, but are never resubmitted. Unrelated battery or media events preserve the current
notification or contact view and keyboard focus.

## Devices and pairing

Open Device settings to review trusted devices and pending requests. Open
Handover on Android and connect to the computer, then press Refresh. Compare
the eight-digit code with the phone, check that the codes match, and approve.
Reading a request never approves it. Forget device requires confirmation and
removes saved trust; pairing will be required again. Fingerprints and manual
connection help are available in expandable details.

## Notifications

Notifications shows one searchable inbox of current phone notifications, with
app identity and actions on each row. In All devices, rows also name the phone.
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

Choose Send file on Phone, use Choose files in Files, or drop local files
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

## Phone folders

Files has Transfers and On phone tabs. On phone requests a directory listing
under the selected phone's shared root. Open a folder or enter a relative path.
Responses from another device or a different requested path do not replace the
current listing. Changing phones resets the folder location. Listing is an
explicit read; opening the tab does not submit a request.

In the Android app's Remote desktop page, choose a shared folder and grant read access.
The desktop can browse and download files only inside that folder. Stop sharing
revokes Handover's saved grant. Android restricts selecting the entire internal
storage or Downloads root; choose a subfolder instead. See
[Android's folder access rules](https://developer.android.com/training/data-storage/shared/documents-files).

Press Download beside a file. The request first shows as queued, then receiving.
Saved means the desktop finished an atomic save in Downloads/Handover, preserving
the original filename and bytes. Open containing folder reveals the result.
The per-file limit is 100 MiB. Failed requests can be retried explicitly; Handover
never retries automatically. Downloads continue if the control window closes.
Their status stays in Recent transfers until the daemon restarts or its bounded
history expires. Older Android apps need an update before Download is available.

## Calls

Calls shows the phone's reported call phase and only offers Answer, Decline, and
Hang up when reported as supported. Call requires a supported Place control and
a valid ordinary phone number. Dial text survives view updates. The request
carries the call generation the user saw, so the daemon rejects it if that call
has changed before submission. Queued or phone-accepted controls do not confirm
that a call connected. Use matching daemon and client binaries for this guard.

## Clipboard history

Clipboard shows the daemon's saved text history for this computer, across all
devices. The selected phone does not filter it. Opening the page or pressing
Refresh reads the current history. Search filters text, Copy puts the complete
entry on the desktop clipboard, and Pin keeps an entry in history. Pinning and
clearing refresh from the daemon after acceptance.

Clear history asks first and keeps pinned entries by default. Include pinned
entries only with the checkbox. A disconnected daemon disables history changes;
already loaded text remains available to copy. Mirroring has an explicit Turn on
or Turn off button based on the daemon's reported setting. Opening the page does
not change it. A successful change refreshes its reported state.

## Contacts and media

Contacts reads synced address books. Select a connected phone and use the sync
button to request a refresh, which still needs Android contact permission.
Search by name, phone number, or email and copy an address. The view shows up to
100 matches. Reported contact pictures decode and resize off the GTK thread;
rebuilt rows reuse cached images. Invalid, oversized, or unsupported pictures
fall back to initials. Call… opens the selected contact's number in Calls for
review. It does not place a call automatically and requires a reported Place
control.

Media groups laptop audio and phone playback. Phone playback offers the
reported Previous, Play, Pause, Play/Pause, and Next controls without optimistic
state changes.

Laptop audio shows reported receiver state, Start/Stop, and available quality
profiles. Streaming means receiver output started, not confirmation that a
listener heard sound. Audio diagnostics show reported sender and receiver data.
Laptop audio routing contains shared source, output, and playback-player
choices. Output can be Phone, Laptop, or Both. Source can be all applications or
an explicit application selection. Source edits remain drafts until Apply and
survive unrelated state updates. Failed or unknown replies keep the draft.
Missing application/player observations remain unavailable rather than being
replaced with an invented default. Older snapshots cannot overwrite newer audio
revisions. Diagnostic values update in place without rebuilding the routing choices.

## Remote input and desktop tools

Phone contains Remote input for pointer moves, clicks, scrolling, and text.
Text is limited to 512 UTF-8 bytes and belongs to the selected phone. A lost
acknowledgement keeps the draft and never retries automatically. Closing with
remote text or an unapplied audio-source draft asks before discarding it.

Device settings contains a phone connection test, Desktop behavior, and
Configured commands. Desktop behavior offers Automatic, Keep desktop awake, and Allow desktop sleep policy
requests. It does not claim to observe actual sleep inhibition. Commands lists
the desktop allowlist; Run submits only a reported command name. The app shows
the returned result and exit code, not an invented execution success.

Calls contains Call audio on this computer, a read-only inspection of local HFP
gateway and duplex observations. Refresh and phone call-state events update it. These observations do not
confirm the associated phone or microphone/speaker routing.

Connection status and operation outcomes are separate. Reconnect retries do not
erase rejected or unconfirmed command results. No mutation replays after the
subscription session changes.

## Capability coverage

| Capability | Location |
| --- | --- |
| Device ring, lock, awake requests, hotspot settings | Phone |
| Calls and local HFP audio observations | Calls |
| File/link sharing and phone directory listings | Files |
| Clipboard text, mirroring, history and pins | Clipboard |
| Contact pictures, search, copy and call shortcuts | Contacts |
| Laptop audio, shared routing and phone playback | Media |
| Pairing, desktop sleep policy and configured commands | Device settings |
| Pointer, click, scrolling and text input | Phone, under Remote input |
| Messaging | Separate native Messages app |

Phone folder grants and provider behavior, audio output, input effects, and call
routing still require live device validation. The private TLS fixture checks
download requests, exact saved bytes, permission failures, and disconnects.

## Validation

Run `python3 scripts/test-handover-control.py` for the private-display GTK and IPC
fixture. `--dark`, `--width`, `--page`, and `--screenshot` support layout checks.
The runner requires Xvfb and dbus-daemon; screenshots also need ImageMagick.
Fictional devices and a private session bus exercise device selection,
phone controls, call-generation guards, phone folder navigation and isolation,
notification actions, reply drafts and lost acknowledgements, clipboard search,
copy, mirroring, pinning and clearing, pairing comparison, drag-and-drop review,
independent file outcomes, lost replies, contact pictures and call shortcuts,
remote-input drafts, allowlisted commands, desktop sleep policy requests, local
call-audio observations, laptop-audio controls, stale-snapshot ordering, and
reconnect without command replay. Physical-phone effects need separate live validation.
