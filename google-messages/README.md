# Handover Google Messages client

An independent Google Messages protocol client for Handover, developed without
mautrix-gmessages. The target is Google Messages companion access, including RCS;
it still requires Google Messages on the phone and Google's service.

The Rust client lives in [`crates/handover-google-messages`](../crates/handover-google-messages).
It has independently authored source lookup, registration, correlated UKEY2
pairing, encrypted receive, and normalized conversation/history projection.
On 2026-10-05, live tests verified phone confirmation, credential recovery after
restart, all 201 conversations, older message pages, and automatic incoming
message display in Handover. The production adapter remains the default during
development.

`handover-google-messages-helper` runs this client through the existing normalized
helper contract. It keeps one receive stream per active account for updates,
reads, and bounded text sends. A phone capability response gates sending; an
accepted send never claims delivery, and interrupted sends remain unknown
without automatic retries. Native text sending has mock coverage and still
needs a recipient test. Attachments, replies, reactions, token renewal, and
remote logout remain unfinished. See
[`docs/gmessages-sidecar.md`](../docs/gmessages-sidecar.md) for the helper contract.

Confirmed pairing records stay in the restricted local session store. Required
Google authentication is saved in desktop Secret Service with the user's
approval. Protocol envelopes use the independently observed JSON-protobuf outer
format and binary encrypted payloads. HTTP acceptance alone does not prove
pairing or message delivery.

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
