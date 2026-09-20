# Messaging commands

Commands under `handoverctl messages` inspect and use messaging accounts
and conversations. Sends are accepted with a request id. Delivery,
display, and failure arrive later as attested status events visible in
`handoverctl monitor`. This CLI never invents delivery state.

Selectors:

- Conversations: `ACCOUNT:THREAD`, or a bare `THREAD` when it is
  unambiguous across accounts. Account ids contain colons, so the CLI
  matches the longest known account prefix.
- Messages: `ACCOUNT:THREAD:MESSAGE`, `THREAD:MESSAGE`, or a bare message
  id when it is unambiguous.

```
handoverctl messages accounts
handoverctl messages conversations ACCOUNT
handoverctl messages history CONVERSATION [--limit N] [--cursor CURSOR]
handoverctl messages send CONVERSATION TEXT
handoverctl messages send-file CONVERSATION PATH [--caption TEXT]
handoverctl messages reply MESSAGE TEXT
handoverctl messages react MESSAGE EMOJI
handoverctl messages unreact MESSAGE EMOJI
handoverctl messages read CONVERSATION [--message MESSAGE]
handoverctl messages typing CONVERSATION
handoverctl messages delete MESSAGE
handoverctl messages open ACCOUNT ADDRESS...
handoverctl messages login ACCOUNT [--from-file PATH]
handoverctl messages logout ACCOUNT
handoverctl messages sync ACCOUNT
```

## accounts

Lists messaging accounts: id, label, and state. State is `online`,
`pairing`, or `offline`, derived from the connection and authentication
flags attested by the helper.

## conversations

Lists conversations for one account: conversation id, kind, transport,
title or participants, and unread count.

## history

Shows message history for one conversation. Prints a `--cursor CURSOR`
line when older messages remain in the stored window, and a start marker
when the window is exhausted. `--limit N` caps the page. `--cursor`
continues from a previous page.

## send

Sends a text message. Prints a request id. Accepted, not delivered.

## send-file

Sends a file attachment with an optional caption. The path is
canonicalized locally. Accepted, not delivered.

## reply

Replies to a message. The daemon validates that the target message is
known before accepting. Accepted, not delivered.

## react and unreact

Adds or removes an emoji reaction on a message. Prints the accepted
request id.

## read

Marks a conversation read. With `--message`, marks read up to that
message. Queued, not confirmed.

## typing

Sends a typing-start ping for a conversation. No typing-stop command
exists because the upstream relay cannot send typing-stop.

## delete

Deletes one own message. Accepted, not confirmed. Only own messages can
be deleted.

## open

Opens or creates a conversation with one or more addresses, such as
phone numbers or emails. Accepted with a request id. Watch for the
conversation event. The conversation is not confirmed by this command.

## login

```
handoverctl messages login ACCOUNT [--from-file PATH]
cat bundle.json | handoverctl messages login ACCOUNT
```

Reads a credential bundle from PATH or stdin and passes it to the
helper. Bundles never travel through argv, shell history, logs, or
crash reports. There is no password, token, or bundle argument. The
bundle is bounded at 256 KiB. Accepted means the helper took the
bundle. Confirm pairing on the phone. See
`docs/gmessages-sidecar.md` for the adapter runbook and storage rules.

## logout

Logs out and revokes helper access. Queued. Access ends when the helper
revokes the session.

## sync

Asks the helper to re-emit authoritative state for one account.
Requested, not confirmed.
