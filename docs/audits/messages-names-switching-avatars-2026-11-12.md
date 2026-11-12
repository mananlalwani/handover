# Group names, switching, and avatar stability

Group sender labels used only the provider's name or address. They now resolve
synced contact names through the same normalized address matching as photos.
Names work without photos. Conflicting address-book names fall back to the
reported identity. Participant details, quotes, and unnamed conversation titles
use the same lookup. Explicit conversation titles remain intact.

The GUI regression caught an unchanged contact refresh clearing the header photo.
A second regression caught the image loader clearing a temporarily hidden avatar.
Both now pass. Address-book refreshes retain content-keyed photo cache entries;
identical contact data causes no repaint. Hidden avatars keep decoded textures,
bounded by 1024 bindings and 128-pixel decoding. The existing decode concurrency,
96-entry/64 MiB shared texture cache, and offscreen message-image release remain.
Removing or changing a contact updates the lookup and rebinds affected widgets.

Switching back to a loaded conversation previously cleared its messages before
fetching history. A GUI regression failed with an empty thread immediately after
the switch. The app now keeps at most eight recent thread windows in memory,
using the model's existing 300-message window bound. It displays cached messages
while refreshing history. Cached rows are replaced by fresh history; incoming
message events exclude their rows from that replacement. Message changes,
removals, account disconnection, and recovery invalidate cached views. Anchored
search/context views do not populate this cache. Connection and delivery state
still come from the daemon.

Unchanged selection, draft, and empty attachment-draft replies no longer rebuild
the whole chat list. These changes preserve the existing draft conflict and
history epoch handling. GUI tests also cover returning from quoted context so
cached rows do not outlive the refreshed history window.

Formatting, strict workspace Clippy, 628 workspace tests with 7 ignored, Android
assembly/lint/unit tests, QML lint, and 49 JavaScript tests passed. Private GUI
scenarios passed in regular light and narrow dark layouts. The names, immediate
cached switch, unchanged-contact avatar, and hidden-avatar checks use synthetic
contacts and messages. These tests do not establish that every intermittent
flicker or delay in the live account has been eliminated.

Installed and reopened the final release through normal window closure. The
running executable matched the release build. Drafts were preserved, the daemon
and helper were left running, and no messages were sent.
