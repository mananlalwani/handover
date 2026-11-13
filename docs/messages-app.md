# Native Messages app

`handover-messages` is a Rust GTK4/libadwaita Linux app. It connects to
`handoverd` through the public Unix-socket IPC. The daemon owns accounts,
history, drafts, and outgoing operations. The app does not open a separate phone
session or handle Google credentials.

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
capabilities. Search matches conversation names and saved message text. It does
not search messages that the daemon has never fetched. Selecting a message
result opens its saved context. Scrolling upward loads older messages before
you reach the top and keeps the message you were reading in place. A small
indicator appears during loading; failed requests offer Retry. Latest messages
returns to the conversation's newest messages. Restored positions and search
results can continue into older history when the daemon has a saved continuation.

The layout follows the system light or dark appearance and accent color.
The conversation list loads previews from locally saved messages in bounded
batches. Phone-only conversations use a person icon and groups use a group
icon. Accounts filters the list; Details shows the selected conversation's
participants and account.
Hover a message, or tab to its reply button, to reply. Quoted replies show the
original sender and saved text; click the quote to jump to the original message.
If it is outside the current view, the app requests its saved context.
Right-click a message or open its actions menu to copy text, reply, or add and
remove reactions when the conversation supports them. Reaction counts change
only when the daemon reports updated messages. Hover a bubble for its timestamp. Messages from the same sender group together; date markers separate
longer gaps. The composer grows for multiline drafts.

The composer supports text, replies, and attachments where the conversation
advertises those capabilities. Choose files with the attachment button, drag files
into the conversation, or paste copied images and files. Repeated image paste adds
to the same draft, up to ten files. Removing attachments keeps the caption. Send
submits one message per file, with the caption on the first. Each file can be up
to 50 MiB; attachments cannot be combined with quoted replies. A rejected or
uncertain submission stops the batch, leaving remaining files in the draft.
Files are not retried automatically.
The daemon saves a private copy of each selected file alongside the draft.
Saved selections and captions survive app and daemon restarts, including after
the original file moves or is deleted. Send waits for the attachment-save reply.
Closing waits for pending saves and offers keep/discard if saving fails. A draft
changed in another window requires reloading the saved attachment before replacing
it. With persistence disabled, attachments last only for the daemon session. New-conversation creation is unavailable until
the backend has verified support. The Quickshell reference client remains
available for workflows the native app does not yet expose.

| Shortcut | Action |
| --- | --- |
| Ctrl+K | Search all saved messages |
| Ctrl+F | Find within the current conversation |
| Enter / Shift+Enter in Find | Next / previous match |
| Enter or Ctrl+Enter | Send the current draft |
| Shift+Enter in the composer | Insert a new line |
| Alt+Up or Alt+Down | Move between conversations |

Drafts autosave through the daemon. The app restores the selected conversation
and message position. A conflicting draft revision requires reloading the saved
draft before overwriting it. Closing with unsaved edits offers a choice to keep
the window open or discard those edits. See [message storage](message-storage.md)
for file locations, limits, and how to disable persistence.

Incoming typing appears above the message box with the participant name, and as
a blue **Typing…** label beneath the contact name in
the conversation header, matching QML's placement near the title. The chat-list
preview also shows **Typing…**, including for conversations that are not open.
Tooltips and screen-reader labels identify the reported participants. Typing
updates preserve selection and scroll position; normal transport labels and
message previews return when typing stops. Disconnects and authoritative
snapshots clear or replace typing state. These indicators require an incoming
typing event from the daemon.

Chat-list unread dots follow reported read-state events and conversation unread
counts. A later explicit count supersedes an earlier read-state event; an unknown
count preserves reported read evidence. Initial subscriptions and recovery
snapshots restore read state. A missing dot can mean unread state is unknown.
A focused conversation at its newest messages requests mark-read when the backend
advertises support. The native helper sends the observed phone command and then
refreshes the conversation inventory. The dot clears only when reported state
confirms no unread messages. Reading older history or leaving the app unfocused
does not mark the newest messages read. Returning focus or reconnecting allows a
new request; command acceptance alone does not change the dot.

Reaction removal appears only when reported reaction participants include a
participant identified as self in that conversation. Other people's reactions
and reactions with unknown ownership remain visible without a removal action.
The app rechecks current ownership when a removal is clicked, so an outdated
menu cannot submit it after the reported reaction changes. Acceptance alone
does not change the displayed reaction count.

Outgoing bubbles show status only when the daemon identifies that exact message.
Accepted means command acceptance; Sent, Delivered, and Read require reported
evidence. Historical messages without evidence have no status label. Local read
state never stands in for the recipient's read receipt.

While you read older history, a floating indicator counts newly observed incoming
messages newer than the saved preview. Fetching history and updating existing
messages do not increment it. Selecting the indicator returns to the newest
messages and clears the count, including when another update arrives during the
jump.

A send acknowledgement means the daemon accepted the command. Delivery labels
use later provider evidence. If the connection drops before the acknowledgement,
the app retains the draft and warns that the outcome is unknown. It does not
resend automatically. Check the conversation before sending again.

## Search, links, and saving files

Use Find in conversation or Ctrl+F to search that chat's saved history.
Matches highlight in the message text. Previous/Next and Shift+Enter/Enter move
between matches, fetching the next saved-result page when needed. The counter
shows the known result count, with + when more remain. Escape closes Find.
Ctrl+K keeps the global saved-message search available.

HTTP and HTTPS links open in your browser when clicked. Message actions include
Copy link. Save attachment as opens a destination chooser for a locally available
file. Images also have Save as in the viewer, and Photos and files includes save
controls. Saving writes a new local copy and never downloads missing media.

## Images, audio, video, and contact pictures

Image attachments stored locally appear directly in the conversation. Click an
image to open the in-app viewer. Zoom in and out, pan with the scrollbars, or
choose Fit to see the whole image. Escape closes the viewer. Open externally
uses your system viewer when needed. Previous and Next, or the left and right
arrow keys, move between images in the opened gallery. The app uses installed
raster-image loaders, including JPEG, PNG, GIF, WebP, BMP, TIFF, ICO, HEIC/HEIF,
AVIF, and JPEG XL. It detects the format from file contents before using the
provider's MIME hint. Contact photos use the same decoder. Original attachment
files retain their format when saved or sent. If MIME metadata is absent or
generic, a recognized image filename selects the preview UI; the decoder still
validates the file contents.

HEIC/HEIF and AVIF need a HEIF-capable image loader. The Arch development package
depends on `libheif`. On Ubuntu, install `heif-gdk-pixbuf` and
`webp-pixbuf-loader`; JPEG XL needs `libjxl-gdk-pixbuf` where available.
See Ubuntu's [HEIF loader](https://packages.ubuntu.com/noble/heif-gdk-pixbuf)
and [JPEG XL loader](https://packages.ubuntu.com/resolute/libjxl-gdk-pixbuf).
Newer
GdkPixbuf installations can provide these formats through Glycin instead.
Restart the app after installing image loaders.

Support depends on the installed decoder and the encoding variant. For example,
the current Glycin TIFF decoder rejects some low-bit-depth palette TIFFs.
Animated images and HEIF sequences currently show a still preview. SVG and other
vector documents remain outside the raster viewer. Input files are limited to
25 MiB and 100 megapixels, with decoded textures scaled to the requested size.
Unsupported files retain the Open externally and Save as actions. Attachments
without a local file show that they are unavailable locally.

Open Photos and files in the conversation header to browse saved attachments,
including those outside the current chat view. Newer media and Older media page
through saved messages with attachments. This lists the daemon's local archive;
it does not fetch a complete provider media library. Missing local files remain
unavailable. Deleted messages are omitted.

Audio and video attachments have inline Play controls. Media loads only after
Play, and uses the system GTK media backend and installed codecs. Duration and
seek controls use decoder reports. At most four media files stay loaded;
opening another unloads the oldest player. Removing or hiding a player's row
pauses and unloads it. Delivery-status changes preserve the player. Unsupported
formats show an external-player action. Playback does not start automatically.

Contact pictures come from the daemon's synced address book. Direct
conversations use a photo when a participant's phone number or email has an
normalized match. Formatting punctuation is ignored for phone numbers. Valid
North American 10-digit numbers match their country-code 1 form, preserving the
QML client's behavior; other country codes are retained. Email matching ignores
ASCII case. Conflicting photos remain ambiguous, while a duplicate contact
without a photo does not hide the sole available photo. Direct conversations
use one matched photo; groups combine up to four distinct available member
photos. Missing photos use initials or a person/group icon. Display names alone
never select a photo.
Group sender labels, participant details, quotes, and unnamed conversation titles
use synced contact names matched by the same normalized addresses. Contacts need
no photo to supply a name. Conflicting names fall back to the reported identity.
Unchanged address-book refreshes preserve decoded pictures instead of clearing
them. Photo cache entries use image contents as their keys. Hidden avatars retain
their small decoded textures; offscreen message images still release theirs.

Pictures refresh when the daemon reports an address-book change. The app reads
existing contacts and does not start a phone sync or fetch remote profile images.

The interface uses custom GTK title bars, neutral fallback avatars, and Inter
when available, with a system sans-serif fallback. GTK supplies native window
dragging, keyboard input, menus, and accessibility behavior.

Recent conversation switches can display the last loaded messages immediately.
The app keeps up to eight recent threads in memory and still requests fresh
history. Message changes, removals, account disconnects, and recovery snapshots
invalidate affected cached views. Fresh history replaces cached rows while
preserving updates received during the request. Cached history does not confirm
connection, delivery, or read status.

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
native app; they do not send text. For direct routing, pass both scoped values:

```sh
handover-messages --account ACCOUNT_ID --conversation CONVERSATION_ID
```

Repeated launches route to the running application. The app rebuilds its view
from daemon snapshots after reconnecting. Neither it nor the notification bridge
replays queued sends across a disconnect.

## Validation limits

Automated tests use a private display, session bus, and daemon fixture. They
cover real widgets, socket framing, persistence, and reconnect handling. These
checks do not establish live phone delivery, Google authentication, or the
behavior of a particular desktop notification server.
