# Google UKEY2 dependency

This directory contains the UKEY2 handshake and RustCrypto provider from
[Google beto-core](https://beto-core.googlesource.com/beto-core/), pinned to revision
`479289ef072b0880c0347d36937265e44f00f4ee`. It is included directly in Handover so
building the independent Messages client needs no sibling checkout, Git
submodule, or additional protocol repository.

The code retains its [Apache-2.0 license](LICENSE) and Google copyright notices.
Handover's own code remains MIT. This subtree contains no mautrix or AGPL adapter
code. The protobuf definitions here belong to the upstream UKEY2 library;
Messages service messages are independently authored elsewhere.

## Packaging changes

The original workspace lists unrelated crates absent from the public checkout.
This standalone workspace includes only the handshake, UKEY2 protobuf types,
crypto-provider interface, RustCrypto provider, and provider selector used by
upstream tests. Unrelated provider tests and benches are omitted from manifests.
The unused BoringSSL dependency and feature switches are omitted. The test
provider selector exposes only RustCrypto. Production uses RustCrypto directly.

The protobuf crate uses upstream's checked-in generated definitions with the
matching `protobuf = 3.7.2` runtime. Build-time generation and its Cargo/Soong
feature switches are removed. No external `protoc` installation is required.
The original proto definitions and build script remain as reference inputs.

Manifests are adapted to this minimal workspace. Imported Rust files have been
formatted by Handover's normal `cargo fmt --all`. Formatting changes carry a
modified-file notice.

## Runtime changes

`ukey2_handshake.rs` has three documented changes:

- `from_p256` offers only P-256 and retains that exact offer in the authenticated
  transcript. The original constructor still supports its original ciphers.
  This matches the observed Messages client without reconstructing its transcript
  outside the library.
- Completed-handshake shared-secret buffers are erased on drop with `zeroize`.
  This does not promise erasure of every temporary copy or RNG state.
- Two return types explicitly name their inferred lifetimes to satisfy current
  Rust diagnostics. Behavior is unchanged.

`proto_adapter.rs` rejects explicitly unknown selected next-protocol labels.
The default remains available when the label is omitted by older peers. A
Handover regression test failed before this validation change and passes after
it. Invalid P-256 points are also covered at the wrapper boundary.

Handover's wrapper bounds peer messages, consumes failed handshakes, redacts
state, and stores derived material in buffers erased on drop. It exposes no
method to turn pending confirmation into an authenticated messaging account.

`UPSTREAM.json` records original and included SHA-256 hashes for copied files.
Local changes are identified separately from upstream provenance. Updates must
review the upstream diff, reapply the documented changes, and refresh both lock
files and this manifest.

## Verification

From the Handover root:

```sh
cargo test --locked --manifest-path third_party/ukey2/Cargo.toml -p ukey2_rs --features test_rustcrypto
cargo test --locked -p handover-google-messages
```

CI runs both the Handover workspace suite and the upstream UKEY2 tests. Local
peer tests establish handshake agreement; they do not establish live Messages
pairing or a usable messaging session.
