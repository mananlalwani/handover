# Native control center

Launch **Handover** from your application launcher, run `handover-control`, or
run `handover-gui` without arguments. The launcher prefers the native control
center installed beside it. `handover-gui --qml` opens the Quickshell reference
client; `handover-gui --send` keeps the existing desktop send picker.

The control center uses Rust and GTK4 with a custom interface matching Messages.
It needs GTK 4.10 and libadwaita 1.4 or newer. Install matching binaries with
`make install-user`, or use the release tarball installer.

## Devices and connection

Choose All devices or a device in the left rail. Overview shows reported pairing,
connection, battery, network, notification, contact, and call state. Battery values
on offline devices are labeled as last reported. A daemon disconnect disables
commands and retains the last view until a replacement snapshot arrives.

Open Connection to review trusted devices and pending pairing requests. Open
Handover on the phone and connect to the computer. Refresh Connection when a
request arrives. Compare the eight-digit code with the phone, check that the
codes match, then approve. Reading a request never approves it. Forget device
requires confirmation and removes saved trust; pairing will be required again.
Pairing refresh uses the existing explicit read API, not a polling loop.

## Notifications

Notifications shows current forwarded phone notifications, filtered by device
and search. It does not create desktop popups. Dismiss on phone appears only for
clearable notifications; other buttons come from reported phone actions. Actions
are disabled when the device or daemon is disconnected. Acceptance does not prove
that the phone performed the action. Phone state updates reconcile the view.

Choose which apps forward notifications in the Android app's Notifications page.
The control center does not change phone forwarding preferences. Inline reply is
not yet exposed in this view.

## Sharing and recent activity

Select a connected device, then choose Send file, Send link, or Send clipboard
text. Clipboard text opens for review and is sent only after choosing Send.
File and link controls require the device's reported file-sharing capability.
The daemon validates the selected resource before accepting a share.

Recent activity is owned by the daemon, so closing and reopening the control
center does not lose it. It retains up to 128 observed transfers for the current
daemon session. A daemon restart resets the list. It is not a durable transfer
archive, and accepted requests without later progress or a result may have no
activity entry. Progress reports bytes written to the authenticated connection;
it does not confirm phone storage. Receipt is shown only after a reported result.
An interrupted connection does not make an unfinished transfer successful.

Completed incoming files have an explicit Open containing folder action. No
received path or URL is opened by the control center automatically. Forgetting a
device clears its retained transfer activity. It does not delete received files.

## Contacts and media

Contacts reads the daemon's synced address books. Select a connected phone to
request a sync, which still needs contact permission on Android. Search by name,
phone number, or email and copy an address. The view shows up to 100 matches;
search to narrow larger lists. Contact photos and calling controls are not yet
part of this view.

Media shows reported phone players and their available Previous, Play, Pause,
Play/Pause, and Next controls. Unsupported controls are omitted. Playback state
changes only with device reports. Open Messages launches the separate native
Messages app without opening another phone connection.

## Recovery and validation

The control center subscribes to device continuity events without loading
messaging history. Contact and transfer lists refresh after relevant events and
recovery snapshots. Queues are bounded, requests have timeouts, and commands are
never retried automatically. If a reply is lost, the UI reports an unknown outcome
and asks the user to check the device before trying again.

Run `python3 scripts/test-handover-control.py` for the private-display GTK and IPC
fixture test. `--dark`, `--width`, and `--screenshot` support layout checks. The
runner requires Xvfb and dbus-daemon; screenshots also need ImageMagick. Fixtures
use fictional devices and a private session bus. Physical-phone effects and
desktop-specific window behavior need separate live validation.
