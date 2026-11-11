# Native Messages app

`handover-messages` is a Rust GTK4/libadwaita Linux app. It connects to
`handoverd` through the public Unix-socket IPC. The daemon owns accounts,
history, drafts, and outgoing operations. The app does not open a separate phone
session or handle Google credentials.

## Start and use

Install the matching Handover binaries, then launch `handover-messages` or open
Handover Messages from your application launcher. Source builds can use
`cargo run -p handover-messages`. GTK 4.10 and libadwaita 1.4 or newer are required.

Choose an account and conversation. Offline accounts retain their saved history
and drafts; send controls require the account's reported connection and
capabilities. Search matches conversation names and saved message text. It does
not search messages that the daemon has never fetched. Selecting a message
result opens its saved context. Load older messages to request more history.

The layout follows the system light or dark appearance and accent color.
The conversation list loads previews from locally saved messages in bounded
batches. Phone-only conversations use a person icon and groups use a group
icon. Accounts filters the list; Details shows the selected conversation's
participants and account.
Hover a message, or tab to its reply button, to reply. Hover a bubble for its
timestamp. Messages from the same sender group together; date markers separate
longer gaps. The composer grows for multiline drafts.

The composer supports text, replies, and attachments where the conversation
advertises those capabilities. New-conversation creation is unavailable until
the backend has verified support. The Quickshell reference client remains
available for workflows the native app does not yet expose.

| Shortcut | Action |
| --- | --- |
| Ctrl+K or Ctrl+F | Focus search |
| Ctrl+Enter | Send the current draft |
| Alt+Up or Alt+Down | Move between conversations |

Drafts autosave through the daemon. The app restores the selected conversation
and message position. A conflicting draft revision requires reloading the saved
draft before overwriting it. Closing with unsaved edits offers a choice to keep
the window open or discard those edits. See [message storage](message-storage.md)
for file locations, limits, and how to disable persistence.

A send acknowledgement means the daemon accepted the command. Delivery labels
use later provider evidence. If the connection drops before the acknowledgement,
the app retains the draft and warns that the outcome is unknown. It does not
resend automatically. Check the conversation before sending again.

## Images and contact pictures

Image attachments stored locally appear directly in the conversation. Click an
image to open the in-app viewer. Zoom in and out, pan with the scrollbars, or
choose Fit to see the whole image. Escape closes the viewer. Open externally
uses your system viewer when needed. JPEG and PNG are supported; GIF, WebP, and
BMP depend on installed image loaders. Animated images currently show a still
preview. Attachments without a local file show that they are unavailable locally.

Contact pictures come from the daemon's synced address book. Direct
conversations use a photo when a participant's phone number or email has an
exact normalized match. Formatting punctuation is ignored for phone numbers;
country codes remain part of the match. Ambiguous matches and missing photos
use initials or a person/group icon. Display names alone never select a photo.
Pictures refresh when the daemon reports an address-book change. The app reads
existing contacts and does not start a phone sync or fetch remote profile images.

The interface uses custom GTK title bars, neutral fallback avatars, and Inter
when available, with a system sans-serif fallback. GTK supplies native window
dragging, keyboard input, menus, and accessibility behavior.

## Account setup

The account menu can run the optional `handover-google-messages-setup` component.
Install it with `make install-user GOOGLE_MESSAGES_SETUP=1`. The native dialog
lists installed Chromium browsers and displays the setup component's progress.
Authentication stays in the browser and existing helper. The phone verification
step still needs the phone; saving credentials does not prove that the helper
has connected. Existing accounts need no new setup for this app.

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
