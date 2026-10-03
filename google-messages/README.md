# Handover Google Messages client

An independent Google Messages protocol client for Handover, developed without
mautrix-gmessages. The target is Google Messages companion access, including RCS;
it still requires Google Messages on the phone and Google's service.

The Rust client lives in [`crates/handover-google-messages`](../crates/handover-google-messages).
It has independently authored source lookup, registration preparation,
correlated UKEY2 pairing, receive framing, pairing-reply validation, payload
encryption, and acknowledgement payload construction. A live read-only
authentication test returned four registered sources using matched browser
service cookies and a request constructed by Rust. Registration, pairing,
authenticated receiving, and acknowledgement transport have only local mock
coverage and remain unverified against Google. It does not yet read conversations
or send user messages. The existing adapter remains in use during development.

The native crate now includes a restricted local session-record store. It is
not yet connected to a restorable confirmed account session. An explicit,
opt-in mode-0 registration action uses the observer's matched browser
credentials and saves its unpaired credential in that store. The user ran that
registration once by hand on 2026-10-03, so the unpaired credential is real and
saved locally; no phone pairing, receive session, or message send has followed
from it. Type-44/45 pairing envelopes include the registration token and
observed Tachyon header. An explicit authenticated HTTP send path is covered by a
local mock only. It reports HTTP acceptance and has not sent a request to Google.

The crate also builds `handover-google-messages-helper`, the daemon-supervised
process for this client. It speaks the existing normalized helper contract,
restores locally saved pending registrations, and rejects every capability it
cannot yet serve. Explicit login operations run account-binding lookup or a
bounded phone-pairing ceremony. Confirmed keys are saved privately; messaging
startup and browser-free token refresh remain unimplemented. See
[`docs/gmessages-sidecar.md`](../docs/gmessages-sidecar.md) for how it relates to
the loopback helper and the production adapter.

Handover's client code is covered by the root MIT license. The included
[UKEY2 dependency](../third_party/ukey2/README.md) retains Apache-2.0 licensing.
It builds inside this repository and requires no separate checkout.

## Collect first-party evidence

```sh
python3 google-messages/tools/capture_bootstrap.py --output /tmp/handover-google-bootstrap
python3 -m unittest discover -s google-messages/tests
```

The output directory must not already exist. Captures contain public Google
HTML/JavaScript and a manifest of source URLs, hashes, sizes, and candidate RPC
method names. The collector does not execute JavaScript, use cookies, or import
an existing account session. Raw captures are research inputs, not repository
sources, and must stay outside Git.

To index observed RPC descriptor literals from a capture, run:

```sh
python3 google-messages/tools/index_rpc_descriptors.py /path/to/capture
```

The indexer checks each script against the manifest size and SHA-256 before it
reads descriptors. Its JSON records contain only literal RPC paths, request and
response JavaScript symbols, and source locations. `candidate_count` includes
RPC path strings that do not match the descriptor form; `indexed_count` counts
matches. Descriptor recognition is limited to the observed
`google.internal.communications.instantmessaging.v1` namespace. The index is
partial evidence and says nothing about wire fields or schemas.

## Observe a read-only pairing test

Use a separate browser profile with a loopback-only DevTools endpoint. Open the
browser manually, then start the observer before signing in to Google Messages.
Pause the existing relay before pairing and restore it after the test.

```sh
node google-messages/tools/observe_rpc.mjs \
  --devtools-file /path/to/private-profile/DevToolsActivePort \
  --output /tmp/handover-pairing-shapes.jsonl --duration 60
node --test google-messages/tools/observe_rpc.test.mjs
```

The output file must not exist. The observer attaches to Google Messages pages
and records bounded RPC metadata and body structure. It omits body values,
object keys, cookies, credentials, and message contents. It does not navigate,
click, sign in, or pair on the user's behalf. Successful pairing in Google's web
client supplies evidence for implementation; it does not validate independent
Handover pairing.

For an already signed-in normal Chrome profile, the local
[Chrome observer](tools/chrome-observer/README.md) observes the selected Google
Messages tab without relaunching Chrome. Its installation requires a manual
Chrome extension-management step.

Validate a saved Chrome JSON export and print aggregate facts with:

```sh
python3 google-messages/tools/summarize_observation.py /path/to/google-messages-rpc-observation.json
```

The summarizer rejects unexpected fields and emits RPC counts, HTTP facts, and
top-level shape counts. It does not print complete nested shapes or values.

The [source-selection and session contract](docs/pairing-session.md) records
protocol evidence and separates offline checks from live verification.

## Integration target

The client implements the existing Handover helper IPC v1, documented in
[`docs/gmessages-sidecar.md`](../docs/gmessages-sidecar.md). The process boundary
and contract handling exist; the protocol work behind it does not.
Handoverd remains authoritative for normalized runtime state; credentials and
Google protocol details remain inside this client.
Do not copy or import the AGPL adapter, its upstream code, or generated protocol
definitions. Derive protocol behavior from first-party observations and verified
specifications, using established cryptographic libraries.

The first acceptance checkpoint is fresh pairing and a read-only conversation
and history fetch without loading mautrix. Later checkpoints cover incoming
updates, text and media sends, delivery evidence, logout, and recovery. No send
capability is advertised before it is implemented and verified.
