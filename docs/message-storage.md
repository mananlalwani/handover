# Message storage and recovery

The daemon keeps a bounded live message window and stores normalized state and
fetched history locally. These files contain private message content. They do
not constitute a complete copy of the provider's history or a backup service.

## Files and limits

Files live under `${XDG_STATE_HOME:-$HOME/.local/state}/handover/`:

- `messaging-cache.json` restores account labels, conversations, message windows,
  read state, loaded-history markers, and contact names. The restore limit is
  32 MiB. Before writing, the daemon prunes older cached messages to fit; if the
  other state alone exceeds the bound, it writes an empty cache. Invalid,
  oversized, or unsupported-version caches are not partially restored.
- `messaging-workspace.sqlite` stores drafts and the last selected conversation
  and message position. It holds up to 128 nonempty drafts within a 16 MiB
  database limit. Drafts are limited to 8,000 characters and 64 KiB.
- `messaging-history.sqlite` stores normalized messages and fetched history-page
  boundaries beyond the live window. Its default database limit is 1,024 MiB.
  The configurable range is 16 to 1,048,576 MiB. The limit covers the SQLite
  database, not every Handover state file, SQLite temporary file, or attachment.

When writing the cache or opening the archive, the daemon sets the containing
directory to mode `0700` and the file to mode `0600`. This restricts other local
users; it is not encryption at rest. Desktop credential-store data remains separate.

Restored accounts begin offline and unauthenticated. Only the helper's live
session evidence can restore connectivity. A cached message is not proof that
the phone is connected or that a new send was delivered.

## Archive size and full storage

The reference Quickshell Messages page has a **Private archive size** dialog.
Changing the limit preserves history. A limit below the database's current size
is rejected; lowering it does not delete older messages.

IPC clients can query or change the limit through protocol 1:

```json
{"protocol":1,"method":"messages.archive_settings"}
{"protocol":1,"method":"messages.archive_settings","limit_mib":2048}
```

The `archive_settings` response contains `limit_mib` and `used_bytes`. Used bytes
measure the database's logical pages, including reusable space. They are not the
size of all Handover storage. This method is not currently a named CLI command.

The archive does not automatically evict old rows to make room. When a page write
exceeds the limit, its transaction fails and the previous complete page remains.
The storage-exhaustion regression verifies that behavior across restart. Failed
archive writes are logged; in-memory updates can still arrive. A successful
history read or message update does not acknowledge durable storage. Older or
full-history requests can report `history_unavailable` when an exact page cannot
be stored or read.

If the archive cannot open at startup, the daemon logs the failure and continues
without that archive. In-memory state and helper history remain subject to their
normal limits and availability. Fix the storage problem and restart the daemon
to try opening the archive again. Handover does not silently recreate a corrupt
archive by deleting it.

## Search and drafts

Saved-message search builds a local FTS5 index on first use. Insertions, updates,
and removals update that index. Deleted messages are excluded. The index counts
towards the archive size limit. Search does not contact a provider or fetch
unseen history. An index write failure leaves the prior archive intact.

Draft updates compare the revision the client loaded with the stored revision.
A stale revision returns `draft_conflict` instead of overwriting another client's
edits. Clearing a draft removes its row. Confirmed account removal also removes
its drafts and saved selection. Storage failures return errors rather than
claiming a durable save. If the workspace database cannot open, the daemon uses
session-only storage and reports `persistent: false` to clients.

## Disable persistence

Set `HANDOVER_MESSAGING_CACHE=0` in the daemon's environment before starting it.
This disables both normalized cache loading/writing and opening the history
archive. Drafts and selection then live only for the daemon session. A systemd user-service override can set it:

```ini
[Service]
Environment=HANDOVER_MESSAGING_CACHE=0
```

Use `systemctl --user edit handoverd.service` to add the override and restart the
service. Restarts then rebuild message views from the helper. Disabling this
storage does not delete existing files, disable the durable outgoing-operation
journal, remove helper credentials, or control attachment staging.

## Removal and recovery limits

Confirmed account and conversation removals request removal of their archived
records. Message removals invalidate affected stored pages; confirmed tombstones
carry the deleted state. Storage failures can prevent cleanup and are logged.
Removal is not a secure erase of free database pages or backups.

There is no automatic age-based archive retention or dedicated purge control.
Those require a separate retention decision. Do not delete state while the daemon
is writing it, and do not include these files or their contents in bug reports.
For diagnostics, record versions, operation identifiers, fixed errors, and
whether a result came from cache, a fixture, or a live account.

Source owners are [`messaging_cache.rs`](../handoverd/src/messaging_cache.rs),
[`messaging_archive.rs`](../handoverd/src/messaging_archive.rs), and the history
request path in [`messaging.rs`](../handoverd/src/ipc_server/messaging.rs).
