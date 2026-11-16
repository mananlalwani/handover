# Native control center

Launch **Handover**, run `handover-control`, or run `handover-gui` without
arguments. The launcher prefers the native control center installed beside it.
`handover-gui --qml` opens the Quickshell reference client, and
`handover-gui --send` keeps the existing desktop send picker.

The control center uses Rust and GTK4. It needs GTK 4.10 and libadwaita 1.4 or
newer. Install matching binaries with `make install-user` or the release tarball
installer.

## Phone and task views

Select a phone in the sidebar, then move between Phone, Notifications, Calls,
Files, Clipboard, Contacts, and Media. Device settings contains trust and
pairing. Messages opens the separate native Messages app.

Phone groups device controls, including ringing, screen lock, awake-mode
requests, and hotspot settings. Controls submit requests; they do not show an
invented lock, awake, or hotspot state. Offline battery values are labeled as
last known. File transfers and phone folder browsing share the Files view.

When one paired phone is connected, it is selected automatically. Use the
phone-name menu to switch devices or view all devices. A disconnect disables
controls until the daemon reconnects.

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

The transfer list shows progress and reported results. If the reply is lost, the
outcome is unknown and Handover will not retry automatically. Check the phone
before sending that file again.

Selections belong to the current window. Closing with unsent, submitting, or
unconfirmed files asks before discarding the selection. Closing or removing a
row does not cancel a submitted transfer. Remove old rows to make room for more
files.

Send link and Send clipboard text open a review dialog. Nothing is sent until
Send is chosen. File and link controls require the device's reported sharing
capability, and the daemon validates each resource before accepting it.

Recent transfers remain available after closing the control center, but are
cleared when the daemon restarts. Completed incoming files have an **Open
containing folder** action. Received paths and URLs do not open automatically.

## Phone folders

Files has Transfers and On phone tabs. On phone requests a directory listing
under the selected phone's shared root. Open a folder or enter a relative path.
Changing phones resets the folder location. Opening the tab does not request a
listing; choose a folder or enter a path to browse.

In the Android app's Remote desktop page, select Share all phone storage and
allow all-files access in Android settings once. On Android 11 or newer, the
Linux root listing then shows Internal storage and any mounted removable
volumes. Browse Downloads, DCIM, documents, and other shared-storage folders
without approving each file on the phone. The transfer service only lists and
reads files; it does not expose remote writes or deletes.

Android still blocks other apps' private data and protected system files.
[Android's all-files access documentation](https://developer.android.com/training/data-storage/manage-all-files)
explains these limits. This does not grant root access.

Share one folder instead keeps the narrower system folder picker available,
including on older Android versions. Its persisted read grant survives restarts.
Stop sharing disables both modes immediately, even if the system all-files
permission remains enabled. Revoking that permission also stops broad access.
Switching modes invalidates pending requests against the previous storage root.

Press Download beside a file or folder, or select several entries and download
up to 32 together. Load more reads the next directory page. With matching builds,
folders preserve their directory structure, empty folders, filenames, and bytes.
Downloads accept files up to 8 GiB and folders up to 10,000 entries. Handover limits
reserved partial data to 16 GiB and runs at most two downloads at once.

The list shows preparation, transfer, and verification progress. A download is
marked saved when the requested file or folder is ready in Downloads/Handover.

Cancel removes partial data. Disconnects and timeouts pause downloads. Reconnect
and press Resume to continue; requests do not resume automatically. Closing the
window leaves downloads running. Older phones support single-file downloads up
to 100 MiB.

## Calls

Calls shows the phone's reported call phase and only offers Answer, Decline, and
Hang up when reported as supported. Call requires a supported Place control and
a valid ordinary phone number. Dial text survives view updates. The request
carries the call generation the user saw, so the daemon rejects it if that call
has changed before submission. Queued or phone-accepted controls do not confirm
that a call connected. Use matching daemon and client binaries for this guard.

To use the laptop's microphone and speakers, pair the phone with Linux Bluetooth
and select its connected endpoint, a local microphone, and local speakers in
Calls. Use laptop for calls arms that explicit selection. Handover starts the two
PipeWire routes only while that phone reports an active call. Waiting for call
leaves the microphone off. Stop using laptop removes Handover's routes. Call end,
disconnection, or daemon exit stops them too. Global audio defaults do not change.
The previous Bluetooth profile is restored when safe; an interrupted restoration
is retried from the daemon's private recovery journal. Restart never arms a mic.

The routing status reports whether the selected PipeWire links are active. It
does not confirm that audio is audible on the phone.

Media and Calls also show reported phone volume streams. Adjust a slider and
press Apply. The phone's actual report updates the displayed level. Acceptance
alone does not prove a change. Refresh volume requests another phone report.

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
button to request a refresh; Android contact permission is required. Search by
name, phone number, or email and copy an address. **Call…** opens the selected
number in Calls for review; it does not place the call.

Media groups laptop audio and phone playback. Phone playback offers the
reported Previous, Play, Pause, Play/Pause, and Next controls without optimistic
state changes.

Laptop audio shows receiver state, Start/Stop, and available quality profiles.
Routing lets you choose audio sources, output (Phone, Laptop, or Both), and
playback players. Press Apply to submit source changes. Audio diagnostics show
reported sender and receiver data.

## Remote input and desktop tools

Phone contains Remote input for pointer moves, clicks, scrolling, and text.
Text is limited to 512 UTF-8 bytes. A lost acknowledgement keeps the draft and
does not retry automatically.

Device settings contains a phone connection test, Desktop behavior, and
Configured commands. Desktop behavior offers Automatic, Keep desktop awake, and Allow desktop sleep policy
requests. It does not claim to observe actual sleep inhibition. Commands lists
the desktop allowlist; Run submits only a reported command name. The app shows
the returned result and exit code, not an invented execution success.

Calls also shows local Bluetooth call-audio status.

## Device names

In Device settings, edit Name in Handover under a trusted device and press Save
name. This changes the name used on this computer, including the device selector
and alerts. It remains saved across reconnects and daemon restarts. The phone can
be offline. Use phone name removes the nickname and restores its reported name.
