# Native Messages app

`handover-messages` is the native Linux app for Handover conversations. Google
Messages account setup remains in the separate helper.

## Start and use

Install the matching Handover binaries, then launch `handover-messages` or open
Handover Messages from your application launcher. Source builds can use
`cargo run -p handover-messages`. GTK 4.10 and libadwaita 1.4 or newer are required.

The Quickshell Messages page recommends this app and offers **Open Handover
Messages** when it finds an executable in `~/.local/bin`, beside the `handover-gui` launcher, or in `PATH`. If it is not
installed, the page keeps QML messaging available with a fallback warning.
**Use QML fallback** also remains available when the standalone app is installed.

**Install Handover Messages** opens a user-only installer. Download and extract
the matching Linux release, then choose its `bin/handover-messages` executable,
or choose an executable from your own build. The installer copies it to
`~/.local/bin` and adds an application launcher under your user data directory.
It does not install dependencies, restart the daemon, or change phone pairing.
GTK and libadwaita must already be installed. These Quickshell install and launch
controls require Python 3. Launch failures leave the QML fallback available.

Choose an account and conversation. Offline accounts retain their saved history
and drafts; send controls require the account's reported connection and
capabilities. Search matches contacts, conversation names, participant addresses,
and saved message text. It does not search messages that the daemon has never fetched. Selecting a message
result opens its saved context. Scrolling upward loads older messages before
you reach the top and keeps the message you were reading in place. A small
indicator appears during loading; failed requests offer Retry. Latest messages
returns to the conversation's newest messages. Restored positions and search
results can continue into older history when the daemon has a saved continuation.

The app follows the system light or dark appearance. Use Accounts to filter the
conversation list and Details to view participants and account information.
Hover a message, or tab to its reply button, to reply. Quoted replies show the
original sender and saved text; click the quote to jump to the original message.
If it is outside the current view, the app requests its saved context.
Right-click a message or open its actions menu to copy text, reply, or add and
remove reactions when the conversation supports them. Reaction counts change
only when the daemon reports updated messages. Hover a bubble for its timestamp. Messages from the same
sender group together; date markers separate longer gaps. The composer grows for multiline drafts.

The composer supports text, replies, and attachments when available. Add files
with the attachment button, drag them into the conversation, or paste images and
files. You can add up to ten files, each up to 50 MiB. Attachments cannot be sent
with a quoted reply. Remove files to change the selection; removing them keeps
the caption. Sending uploads all files before submitting one message, so a failed
upload does not send a partial group.

The app saves drafts and attachment selections across restarts. A pending or
uncertain send is not retried automatically. After checking the conversation,
retry a confirmed failure or choose Keep editing. New-conversation creation is
not available yet. The Quickshell client remains available as an alternative.

| Shortcut | Action |
| --- | --- |
| Ctrl+K | Search contacts, conversations, and saved messages |
| Enter / Down in global search | Open / focus the first result |
| Ctrl+Shift+M | Select messages |
| Ctrl+A in selection mode | Select up to 100 loaded messages |
| Escape in selection mode | Finish selection |
| Ctrl+F | Find within the current conversation |
| Enter / Shift+Enter in Find | Next / previous match |
| Enter or Ctrl+Enter | Send the current draft |
| Shift+Enter in the composer | Insert a new line |
| Alt+Up or Alt+Down | Move between conversations |
| Alt+Left | Return from message context |

Drafts autosave through the daemon. The app restores the selected conversation
and message position. A conflicting draft revision requires reloading the saved
draft before overwriting it. Closing with unsaved edits offers a choice to keep
the window open or discard those edits. See [message storage](message-storage.md)
for file locations, limits, and how to disable persistence.

Typing indicators appear in the conversation header, above the composer, and in
the conversation list when the daemon reports incoming typing activity.

Unread dots and delivery labels follow state reported by the service. A missing
unread dot can mean the state is unknown. Reading older history does not mark the
newest messages as read. When read support is available, the app requests a
read update while the conversation is focused at its newest messages. The dot
changes only after the service reports the result.

While viewing older messages, a floating indicator counts newly received
messages. Select it to return to the newest messages. An unread divider and
**First unread** action appear when the saved read position is available.

Send acknowledgements mean the service accepted the request. Sent, Delivered,
and Read labels require later provider evidence. If a send's outcome is unknown,
check the conversation before trying again. Failed text messages may offer
**Edit and resend**, which restores the text for review without sending it.

## Organizing conversations

Details has an **Only on this computer** section for pinning, archiving, muting
Handover desktop message alerts, and marking a conversation unread here. These
settings belong to the daemon workspace and survive app restarts. They do not
change the phone's conversation state. With persistence disabled, they last only
for the daemon session.

Pinned conversations appear first. The sidebar filters Chats, Unread, Archived,
and All conversations. Search includes archived chats. Mark unread adds a local
reminder that clears when you leave and reopen the conversation. Muting affects
Handover desktop message alerts, not phone alerts or other app notifications.

Choose Select messages from the main menu or Select message from a message's
actions. Select up to 100 loaded messages, then Copy or Save files. Copy preserves
conversation order and includes sender names, text, and attachment filenames.
Changing conversations ends selection. Save files opens a folder chooser. It
exports up to 64 local attachments per operation. Missing files are skipped; the
completion dialog lists saved, skipped, and failed files.

## Search, links, and saving files

Use Find in conversation or Ctrl+F to search that chat's saved history. Use
Enter and Shift+Enter to move between matches; Escape closes Find. Ctrl+K searches
contacts, conversations, participant addresses, and saved message text. Select a
result to open its saved context. Contact results can copy an address or open an
existing conversation, but do not create one. Search cannot find messages the
service has not fetched. Alt+Left returns to the conversation and its previous
position.

HTTP and HTTPS links open in your browser when clicked. Message actions include
Copy link. Save attachment as opens a destination chooser for a locally available
file. Images also have Save as in the viewer, and Photos and files includes save
controls. Saving writes a new local copy and never downloads missing media.

## Images, audio, video, and contact pictures

Images stored locally appear in the conversation. Click one to open the viewer.
Use Ctrl+scroll or pinch to zoom, drag to pan, and arrow keys to move between
images. Press F or Ctrl+0 to fit, Ctrl+S to save a copy, Ctrl+O to open it
externally, or Escape to close the viewer. Available image formats depend on
installed system image loaders. Unsupported or unavailable files can be opened
externally or saved when local.

HEIC/HEIF and AVIF require a system image loader. On Arch, install `libheif`.
On Ubuntu, install `heif-gdk-pixbuf`, `libheif-plugin-libde265`, and
`libheif-plugin-aomdec`. WebP uses `webp-pixbuf-loader`; JPEG XL uses
`libjxl-gdk-pixbuf` where available. Restart the app after installing loaders.
Animations show a still preview; SVG is not supported by the raster viewer.

Open Photos and files in the conversation header to browse saved attachments.
This view searches Handover's local archive; it does not fetch a complete media
library. Audio and video play only after you select Play and depend on installed
codecs. Playback does not start automatically. Contact pictures come from the
synced address book; the app does not fetch profile photos from the phone.

## Account setup

The account menu can run the optional `handover-google-messages-setup` component.
Install it with `make install-user GOOGLE_MESSAGES_SETUP=1`. The native dialog
lists installed Chromium browsers and displays the setup component's progress.
Authentication stays in the browser and existing helper. The phone verification
step still needs the phone; saving credentials does not prove that the helper
has connected. Existing accounts need no new setup for this app.

After opening Google Messages in a browser, close that browser tab, select a
conversation in Handover, and choose **Reconnect** from the main menu at the top
left. This requests a sync for that conversation's account using its saved
pairing. With no conversation selected, it uses the account filter, or all known
accounts if the filter is All accounts. Acceptance does not confirm a connection;
the account indicator updates when the daemon reports its state. Reconnect does
not unpair the phone or retry outgoing messages.

## Desktop alerts

Enable the optional bridge with:

```sh
systemctl --user enable --now handover-desktop.service
```

New incoming messages can create desktop alerts. Startup and recovery snapshots
establish a silent baseline. Open conversation and Reply actions both open the
native app; Reply focuses the matching conversation's composer after its saved
draft loads. Neither action sends text. For direct routing, pass both scoped values:

```sh
handover-messages --account ACCOUNT_ID --conversation CONVERSATION_ID
# Add --reply to focus its composer without sending.
```

The window title counts unread conversations, including local unread reminders.
The desktop bridge publishes the same count through LauncherEntry for desktops
that support launcher badges. This is a conversation count, not a message count.
The bridge clears the badge when disconnected and rebuilds it from current
state after reconnecting. Muted conversations still count as unread.

Repeated launches route to the running application. The app rebuilds its view
from daemon snapshots after reconnecting. Neither it nor the notification bridge
replays queued sends across a disconnect.
