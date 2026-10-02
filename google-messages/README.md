# Handover Google Messages client

An independent Google Messages protocol client for Handover, developed without
mautrix-gmessages. The target is Google Messages companion access, including RCS;
it still requires Google Messages on the phone and Google's service.

This directory currently contains bootstrap evidence tools and a read-only
network observer.
It does not yet authenticate, pair, read conversations, or send messages. The
existing adapter remains in use while this implementation is developed.

This code is part of Handover and covered by the root MIT license.

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

## Integration target

Implement the existing Handover helper IPC v1, documented in
[`docs/gmessages-sidecar.md`](../docs/gmessages-sidecar.md). Handoverd remains authoritative for normalized
runtime state; credentials and Google protocol details remain inside this client.
Do not copy or import the AGPL adapter, its upstream code, or generated protocol
definitions. Derive protocol behavior from first-party observations and verified
specifications, using established cryptographic libraries.

The first acceptance checkpoint is fresh pairing and a read-only conversation
and history fetch without loading mautrix. Later checkpoints cover incoming
updates, text and media sends, delivery evidence, logout, and recovery. No send
capability is advertised before it is implemented and verified.
