# Multiple attachments in one message

On 2026-11-14, the public Google Messages web bootstrap was captured with
`google-messages/tools/capture_bootstrap.py`. No account, cookies, authentication,
or messages were accessed. The capture stays outside the repository under
`~/.cache/handover-multi-media-first-party-20261114`.

The [first-party JavaScript bundle](https://www.gstatic.com/_/messagesweb/_/js/k=messagesweb.mw.en_US.uwar1xJBL1A.O/am=AAAAwAhAAAAQ/d=1/rs=AIp04d_ysQS4pWaZDwonnvH6HPe7ZhhAvA/m=mw_b?wli=mw.t1WtSMB7wt8.loadPhotosC2paSummaryTranslatorWasm.O%3A%3B)
had SHA-256 `8ad7b249e55fb95324b08afec35881c15180be1a409333e12ec627f9440b368b`.

The current message-part selector, `oA.XP`, enumerates all entries in the logical
message's part collection and orders them by their order field. The content
builder, `G$a`, maps that collection to repeated content field 10 through `Kv`.
Every media entry retains its own filename, MIME type, upload identity, size,
and key. Text remains a separate content part. The Bugle backend wraps the
result once in `Vv` and submits it with phone action 3.

A local synthetic harness evaluated the extracted content builder with two image
entries and one caption, replacing protocol primitives with inert field setters.
It retained three parts in one content object, including both original filenames
and upload identities. The harness made no network calls and performed no upload
or send. This confirms the builder's collection behavior, not phone acceptance
or delivery. No source code from the bundle or generated definitions were added
to this repository.

Handover independently extends its existing repeated-part encoder. It keeps the
existing upload and encryption implementations, uploads files sequentially, and
constructs one send request only after all uploads succeed. The daemon records
one outgoing operation. The application limit is ten attachments, rather than a
claim about the provider's maximum.

Validation covers one encoded native content object, distinct media keys and
names, a single caption, one normalized IPC request, one daemon operation, and
one simulated helper message with multiple original files. Invalid inputs and
unknown command replies retain the draft and never trigger automatic splitting or
replay. Command acceptance clears the draft before eventual provider status is
known; later upload or send failures are reported through the outgoing operation.
Real-account multi-file acceptance and recipient delivery were not tested.
